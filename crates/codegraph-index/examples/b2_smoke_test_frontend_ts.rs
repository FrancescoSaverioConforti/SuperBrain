//! B2-EVAL-004: multi-codebase validation, Frontend/UI TypeScript/TSX domain.
//!
//! Drives the SAME real production pipeline as `b2_smoke_test.rs`/
//! `b2_smoke_test_backend_api.rs` (`RepositorySource` -> parse -> extract
//! -> promote -> lineage -> edges -> `CodeGraph` -> incremental reindex),
//! against a real, locally-cloned React/TypeScript frontend repository
//! instead of the ML/DL or Backend/API corpora the prior two harnesses
//! targeted. The generic measurement sections (A-G) are structurally
//! identical by design, so the three reports are directly comparable;
//! only the domain-specific Q&A section (H) differs, grounded in THIS
//! repository's own real component/hook/store architecture.
//!
//! Usage: `cargo run --example b2_smoke_test_frontend_ts -- <repo_path> <commit_a> <commit_b>`

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use codegraph_domain::{CommitHash, EdgeKind, RepositorySource, ResolutionMethod, SymbolKind};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::edges::{resolve_calls, resolve_extends_implements, resolve_tests_for};
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::imports::ImportResolution;
use codegraph_index::incremental::CodeGraphIndexer;
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: b2_smoke_test_frontend_ts <repo_path> <commit_a> <commit_b>");
        std::process::exit(2);
    }
    let repo_path = std::fs::canonicalize(&args[1]).expect("repo_path must exist");
    let commit_a_str = &args[2];
    let commit_b_str = &args[3];

    let mut allowlist = RepositoryAllowlist::new();
    let allowlisted_root = allowlist.authorize(&repo_path).expect("authorize repo root");
    let mut registry = codegraph_domain::RepositoryRegistry::new();
    let repo_root = codegraph_domain::RepositoryRoot::parse(&repo_path).unwrap();
    let repo_id = registry.register(repo_root);

    let root_for_id = move |id: codegraph_domain::RepositoryId| -> Option<codegraph_index::allowlist::AllowlistedRoot> {
        if id == repo_id {
            Some(allowlisted_root.clone())
        } else {
            None
        }
    };
    let source = LocalGitRepositorySource::new(&allowlist, &root_for_id);

    let commit_b = CommitHash::parse(commit_b_str).expect("commit_b must be a valid SHA-1");

    println!("=== B2-EVAL-004: Frontend/UI TypeScript domain smoke test ===");
    println!("repo: {}", repo_path.display());
    println!("commit_b: {commit_b_str}");

    // --- Full pipeline at commit B ---
    let t0 = Instant::now();
    let files = source.list_files_at(repo_id, &commit_b).expect("list_files_at commit_b");
    let total_files = files.len();

    let mut files_supported = 0usize;
    let mut files_parsed = 0usize;
    let mut files_with_error_nodes = 0usize;
    let mut parse_failures = 0usize;
    let mut raw_candidates_total = 0usize;
    let mut rejections_total = 0usize;
    let mut snapshots: Vec<codegraph_index::snapshot::SymbolSnapshot> = Vec::new();
    let mut promotion_failures: Vec<String> = Vec::new();
    let mut kind_counts: HashMap<SymbolKind, usize> = HashMap::new();
    let mut ts_tsx_files_with_import_statement_count = 0usize;

    // Potential-new-gap scan: raw tree-sitter node-kind counts for
    // constructs this pipeline does NOT currently promote as a symbol
    // at all (interface_declaration, type_alias_declaration,
    // internal_module/namespace, decorator, jsx element references by
    // tag name) -- measured directly from the parse tree, independent
    // of extraction, so "zero promoted" can be told apart from "zero
    // present in the corpus."
    let mut unpromoted_construct_counts: HashMap<&'static str, usize> = HashMap::new();

    let mut top_level_counts_by_file: HashMap<String, usize> = HashMap::new();
    let mut spans_by_file: HashMap<String, Vec<(codegraph_index::extract::ByteSpan, String)>> = HashMap::new();

    for relative_path in &files {
        let file = match codegraph_domain::FileIdentity::parse(repo_id, commit_b.clone(), relative_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let Some(parser) = parser_for(&file) else {
            continue;
        };
        files_supported += 1;
        let bytes = match source.read_file(&file) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("read_file failed for {relative_path}: {e}");
                continue;
            }
        };
        let is_ts_family = relative_path.ends_with(".ts") || relative_path.ends_with(".tsx");
        if is_ts_family {
            let text = String::from_utf8_lossy(&bytes);
            if text.lines().any(|l| l.trim_start().starts_with("import ")) {
                ts_tsx_files_with_import_statement_count += 1;
            }
        }
        let tree = match parser.parse(&bytes) {
            Ok(t) => t,
            Err(_) => {
                parse_failures += 1;
                continue;
            }
        };
        files_parsed += 1;
        if tree.tree().root_node().has_error() {
            files_with_error_nodes += 1;
        }

        if is_ts_family {
            count_unpromoted_constructs(tree.tree().root_node(), &bytes, &mut unpromoted_construct_counts);
        }

        let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
        raw_candidates_total += candidates.len();
        rejections_total += rejections.len();
        for candidate in candidates {
            spans_by_file.entry(relative_path.clone()).or_default().push((candidate.span, candidate.name.clone()));
            match promote(&candidate, file.clone()) {
                Ok(snapshot) => {
                    *kind_counts.entry(snapshot.kind()).or_insert(0) += 1;
                    snapshots.push(snapshot);
                }
                Err(e) => promotion_failures.push(format!("{relative_path}: {e}")),
            }
        }
    }

    for (file, spans) in &spans_by_file {
        let top_level = spans
            .iter()
            .filter(|(span, _)| !spans.iter().any(|(other, _)| other != span && other.start <= span.start && span.end <= other.end))
            .count();
        top_level_counts_by_file.insert(file.clone(), top_level);
    }

    {
        let mut seen: HashMap<codegraph_domain::SymbolSnapshotId, Vec<String>> = HashMap::new();
        for s in &snapshots {
            seen.entry(s.id()).or_default().push(format!("{}::{} [{:?}]", s.file().relative_path(), s.name().as_str(), s.kind()));
        }
        let duplicates: Vec<_> = seen.into_iter().filter(|(_, v)| v.len() > 1).collect();
        if !duplicates.is_empty() {
            println!("\n!!! DUPLICATE SymbolSnapshotId found in real data: {} group(s) !!!", duplicates.len());
            for (_, members) in &duplicates {
                println!("  colliding group: {members:?}");
            }
        }
    }

    let no_known_lineage = |_: codegraph_domain::SymbolSnapshotId| None;
    let lineage_edges = codegraph_index::lineage::resolve_lineage(&[], &snapshots, &no_known_lineage).expect("resolve_lineage at B (no prev)");

    let calls_edges = resolve_calls(&snapshots).expect("resolve_calls");
    let extends_edges = resolve_extends_implements(&snapshots).expect("resolve_extends_implements");
    let tests_for_edges = resolve_tests_for(&snapshots).expect("resolve_tests_for");
    let contains_edges = codegraph_index::edges::resolve_contains(&snapshots).expect("resolve_contains");
    let instantiates_edges = codegraph_index::edges::resolve_instantiates(&snapshots).expect("resolve_instantiates");

    {
        let mut seen: HashMap<(codegraph_domain::SymbolSnapshotId, codegraph_domain::SymbolSnapshotId, EdgeKind), usize> = HashMap::new();
        for edge in calls_edges.iter().chain(extends_edges.iter()).chain(tests_for_edges.iter()).chain(contains_edges.iter()).chain(instantiates_edges.iter()) {
            *seen.entry((edge.from, edge.to, edge.kind)).or_insert(0) += 1;
        }
        let dups: Vec<_> = seen.into_iter().filter(|(_, count)| *count > 1).collect();
        if !dups.is_empty() {
            println!("\n!!! DUPLICATE (from,to,kind) typed edge triples found: {} !!!", dups.len());
            for ((from, to, kind), count) in &dups {
                let from_name = snapshots.iter().find(|s| s.id() == *from).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str()));
                let to_name = snapshots.iter().find(|s| s.id() == *to).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str()));
                println!("  duplicate x{count}: {from_name:?} -> {to_name:?} [{kind:?}]");
            }
        }
    }

    let mut edge_kind_counts: HashMap<EdgeKind, usize> = HashMap::new();
    let mut resolution_method_counts: HashMap<ResolutionMethod, usize> = HashMap::new();
    for edge in calls_edges.iter().chain(extends_edges.iter()).chain(tests_for_edges.iter()).chain(contains_edges.iter()).chain(instantiates_edges.iter()) {
        *edge_kind_counts.entry(edge.kind).or_insert(0) += 1;
        *resolution_method_counts.entry(edge.evidence.method()).or_insert(0) += 1;
    }

    let full_index_elapsed = t0.elapsed();

    println!("\n--- B. Pipeline results (full index at commit B) ---");
    println!("total_files_in_tree: {total_files}");
    println!("files_supported_by_parser: {files_supported}");
    println!("files_parsed: {files_parsed}");
    println!("parse_failures: {parse_failures}");
    println!("files_with_tree_has_error: {files_with_error_nodes}");
    println!("raw_candidates_extracted: {raw_candidates_total}");
    println!("extraction_rejections (oversized name/signature): {rejections_total}");
    println!("snapshots_promoted: {}", snapshots.len());
    println!("promotion_failures: {}", promotion_failures.len());
    for failure in &promotion_failures {
        println!("  promotion_failure: {failure}");
    }
    println!("ts_tsx_files_with_at_least_one_import_statement: {ts_tsx_files_with_import_statement_count}");

    println!("\n--- C. Symbol coverage (SymbolKind counts) ---");
    let mut kind_counts_vec: Vec<_> = kind_counts.into_iter().collect();
    kind_counts_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &kind_counts_vec {
        println!("  {kind:?}: {count}");
    }

    println!("\n--- C1. Potential-new-gap scan: constructs this pipeline does NOT promote as a symbol today ---");
    let mut gap_vec: Vec<_> = unpromoted_construct_counts.into_iter().collect();
    gap_vec.sort_by_key(|(k, _)| k.to_string());
    for (construct, count) in &gap_vec {
        println!("  {construct}: {count} (present in corpus, NOT promoted to any SymbolKind by the current extractor)");
    }

    println!("\n--- C2. Module/File -> top-level-symbol gap (Frontend/UI domain stress case) ---");
    let total_top_level: usize = top_level_counts_by_file.values().sum();
    let files_with_multiple_top_level = top_level_counts_by_file.values().filter(|&&n| n > 1).count();
    println!("files_with_at_least_one_extracted_symbol: {}", top_level_counts_by_file.len());
    println!("total_top_level_symbols_across_all_files (MEASURED, span-nesting definition): {total_top_level}");
    println!("files_with_more_than_one_top_level_symbol (MEASURED): {files_with_multiple_top_level}");
    let mut top_files: Vec<(&String, &usize)> = top_level_counts_by_file.iter().collect();
    top_files.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("top 10 files by top-level-symbol count (file: count):");
    for (file, count) in top_files.iter().take(10) {
        println!("  {file}: {count}");
    }

    println!("\n--- D. Edge quality ---");
    println!("lineage_edges (B alone, no prev): {}", lineage_edges.len());
    println!("calls_edges: {}", calls_edges.len());
    println!("extends_implements_edges: {}", extends_edges.len());
    println!("tests_for_edges: {}", tests_for_edges.len());
    let mut edge_kind_counts_vec: Vec<_> = edge_kind_counts.into_iter().collect();
    edge_kind_counts_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &edge_kind_counts_vec {
        println!("  by_kind {kind:?}: {count}");
    }
    let mut method_counts_vec: Vec<_> = resolution_method_counts.into_iter().collect();
    method_counts_vec.sort_by_key(|(m, _)| format!("{m:?}"));
    for (method, count) in &method_counts_vec {
        println!("  by_resolution_method {method:?}: {count}");
    }
    println!("full_index_elapsed: {full_index_elapsed:?}");

    {
        let mut sample: Vec<&codegraph_index::edges::Edge> = calls_edges.iter().collect();
        sample.sort_by(|a, b| a.evidence.note().cmp(b.evidence.note()));
        println!("\n--- Manual call-quality sample (first 30 by evidence note, deterministic) ---");
        for edge in sample.iter().take(30) {
            let from = snapshots.iter().find(|s| s.id() == edge.from);
            let to = snapshots.iter().find(|s| s.id() == edge.to);
            println!(
                "{:?} [{:?}/{:?}] -> {:?} [{:?}] :: {}",
                from.map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())),
                edge.kind,
                edge.evidence.method(),
                to.map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())),
                edge.evidence.method(),
                edge.evidence.note()
            );
        }
    }

    let mut total_import_statements_line_based = 0usize;
    for relative_path in &files {
        if !(relative_path.ends_with(".ts") || relative_path.ends_with(".tsx")) {
            continue;
        }
        let file = codegraph_domain::FileIdentity::parse(repo_id, commit_b.clone(), relative_path).unwrap();
        if let Ok(bytes) = source.read_file(&file) {
            let text = String::from_utf8_lossy(&bytes);
            total_import_statements_line_based += text.lines().filter(|l| l.trim_start().starts_with("import ")).count();
        }
    }
    print_approx_memory();

    println!("\n--- F. Incremental A -> B (CodeGraphIndexer) ---");
    let commit_a = CommitHash::parse(commit_a_str).expect("commit_a must be a valid SHA-1");
    let indexer = CodeGraphIndexer::new();

    let t_full_a = Instant::now();
    let outcome_a = indexer.observe_commit(&source, repo_id, &commit_a, None, &[]).expect("observe_commit A");
    let full_a_elapsed = t_full_a.elapsed();
    println!("commit_a: {commit_a_str}");
    println!("observe_commit(A) generation: {}", outcome_a.generation);
    println!("observe_commit(A) snapshot_count: {}", outcome_a.graph.snapshot_count());
    println!("observe_commit(A) edge_count: {}", outcome_a.graph.edge_count());
    println!("observe_commit(A) elapsed (full reindex): {full_a_elapsed:?}");

    let changed_files = git_diff_name_only(&repo_path, commit_a_str, commit_b_str);
    println!("changed_files (git diff A..B): {changed_files:?}");

    let t_incremental = Instant::now();
    let outcome_b = indexer.observe_commit(&source, repo_id, &commit_b, Some(&commit_a), &changed_files).expect("observe_commit B");
    let incremental_elapsed = t_incremental.elapsed();
    println!("observe_commit(B) generation: {}", outcome_b.generation);
    println!("observe_commit(B) metrics: {:?}", outcome_b.metrics);
    println!("observe_commit(B) elapsed (incremental): {incremental_elapsed:?}");
    println!("observe_commit(B) final snapshot_count: {}", outcome_b.graph.snapshot_count());
    println!("observe_commit(B) final edge_count: {}", outcome_b.graph.edge_count());
    println!("observe_commit(B) final lineage_edge_count: {}", outcome_b.graph.lineage_edge_count());

    {
        let ts_tsx_import_facts = outcome_b.import_facts.iter().filter(|f| {
            let p = f.importing_file.relative_path();
            p.ends_with(".ts") || p.ends_with(".tsx")
        }).count();
        let internal_count = outcome_b.import_facts.iter().filter(|f| matches!(f.resolution, ImportResolution::Internal(_))).count();
        let external_count = outcome_b.import_facts.iter().filter(|f| f.resolution == ImportResolution::External).count();
        let ambiguous_count = outcome_b.import_facts.iter().filter(|f| f.resolution == ImportResolution::Ambiguous).count();
        let alias_count = outcome_b.import_facts.iter().filter(|f| f.alias.is_some()).count();

        println!("\n--- Imports measurement (via the real CodeGraphIndexer, extraction coverage vs. resolution accuracy kept separate) ---");
        println!("total_import_statements_in_ts_tsx_files_at_commit_b (line-based count, MEASURED): {total_import_statements_line_based}");
        println!("ts_tsx_import_facts_extracted (MEASURED, ImportFact, file-level): {ts_tsx_import_facts}");
        println!(
            "extraction_coverage: {ts_tsx_import_facts}/{total_import_statements_line_based} ({:.1}%)",
            if total_import_statements_line_based > 0 {
                100.0 * ts_tsx_import_facts as f64 / total_import_statements_line_based as f64
            } else {
                0.0
            }
        );
        println!("total_import_facts_all_languages (MEASURED): {}", outcome_b.import_facts.len());
        println!("  resolution_internal (MEASURED): {internal_count}");
        println!("  resolution_external (MEASURED): {external_count}");
        println!("  resolution_ambiguous (MEASURED): {ambiguous_count}");
        println!("  aliases_captured (MEASURED): {alias_count}");
        println!(
            "  raw_import_statements_extracted_this_call (MEASURED, IndexMetrics): {}",
            outcome_b.metrics.raw_import_statements_extracted
        );
        println!("  raw_import_statements_reused_this_call (MEASURED, IndexMetrics): {}", outcome_b.metrics.raw_import_statements_reused);
    }

    // --- H. Frontend/UI coding-agent usefulness Q&A, answered using
    // ONLY the real CodeGraph built above (observe_commit(B)'s final
    // graph). Every question/expected-target pair below was derived
    // from direct reading of this repository's real source at commit B
    // (see B2-EVAL-004.md for the exact file/line citations), not
    // invented or adjusted to make the pipeline look good.
    {
        let graph = &outcome_b.graph;
        println!("\n--- H. Frontend/UI usefulness Q&A (conduit-react, commit B) ---");

        let find_by_name = |name: &str| -> Vec<&codegraph_index::snapshot::SymbolSnapshot> { graph.all_snapshots().into_iter().filter(|s| s.name().as_str() == name).collect() };
        let find_current_by_name_in_file = |name: &str, file_suffix: &str| -> Option<&codegraph_index::snapshot::SymbolSnapshot> {
            find_by_name(name)
                .into_iter()
                .filter(|s| graph.lineage_from(s.id()).is_empty())
                .find(|s| s.file().relative_path().ends_with(file_suffix))
        };

        // Q1: which component renders/contains ListView as a child?
        // Ground truth (src/components/ListPage/index.tsx): ListPage's
        // own JSX body constructs <ListView .../> directly, which this
        // pipeline can only see as a CALL to the `ListView` function
        // component (JSX element construction lowers to a function
        // call in the grammar's own call-expression-shaped sense for
        // this pipeline's resolver, not a dedicated JSX-aware edge
        // kind).
        if let Some(list_page) = find_current_by_name_in_file("ListPage", "ListPage/index.tsx") {
            let callees: Vec<String> = graph.callees_of(list_page.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q1 'Which component renders/contains ListView as a child?' -> ListPage calls/references: {callees:?}");
        } else {
            println!("Q1 'ListPage' component NOT FOUND in src/components/ListPage/index.tsx (unexpected)");
        }

        // Q2: which component uses the useTopicListStore hook?
        // Ground truth: both ListPage and ListView call
        // useTopicListStore (src/stores/topic.ts).
        if let Some(use_topic_list_store) = find_current_by_name_in_file("useTopicListStore", "stores/topic.ts") {
            let callers: Vec<String> = graph.callers_of(use_topic_list_store.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q2 'Which component(s) use the useTopicListStore hook?' -> callers: {callers:?}");
        } else {
            println!("Q2 'useTopicListStore' NOT FOUND in src/stores/topic.ts (unexpected)");
        }

        // Q3: from where does the Header component's active-link state
        // come? Ground truth: Header calls useAcountStore
        // (src/stores/auth.ts) and react-router-dom's useLocation —
        // the latter is an external import, so only the internal
        // useAcountStore call is checkable against this graph.
        if let Some(header) = find_current_by_name_in_file("Header", "Header/index.tsx") {
            let callees: Vec<String> = graph.callees_of(header.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q3 'Where does Header get its active-link/auth state from?' -> Header calls: {callees:?}");
        } else {
            println!("Q3 'Header' component NOT FOUND (unexpected)");
        }

        // Q4: if I modify fetchTopicList (the store's own data-fetch
        // function), which components are impacted? Ground truth:
        // ListPage calls fetchTopicList (destructured from
        // useTopicListStore()) inside its own useEffect.
        // fetchTopicList is a closure returned from createStore's own
        // factory, not a standalone top-level symbol -- check and
        // report plainly whether this pipeline's `Calls` resolver can
        // even represent that relationship, rather than assuming it
        // can.
        let fetch_topic_list_callers: Vec<String> = find_by_name("fetchTopicList")
            .into_iter()
            .flat_map(|s| graph.callers_of(s.id()))
            .filter_map(|id| graph.snapshot(id))
            .map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str()))
            .collect();
        println!("Q4 'If I modify fetchTopicList, which components are impacted?' -> callers: {fetch_topic_list_callers:?} (fetchTopicList itself found as a promoted symbol: {})", !find_by_name("fetchTopicList").is_empty());

        // Q5: what does ListView's handleClick event handler call?
        // Ground truth (src/components/ListView/index.tsx): handleClick
        // calls e.preventDefault(), then either navigate('/') or
        // handleTabSelect('all') -- navigate is a hook return value
        // (external useNavigate), handleTabSelect is a received prop
        // (a function value, not a resolvable call target by name in
        // this pipeline's own symbol pool unless a same-named real
        // function exists elsewhere, which it does: Homepage's own
        // handleTabSelect).
        if let Some(handle_click) = find_current_by_name_in_file("handleClick", "ListView/index.tsx") {
            let callees: Vec<String> = graph.callees_of(handle_click.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q5 'What does ListView's handleClick event handler call?' -> calls: {callees:?}");
        } else {
            println!("Q5 'handleClick' NOT FOUND in src/components/ListView/index.tsx (unexpected -- arrow function naming may differ)");
        }

        // Q6: which tests cover the ListItem component? Ground truth:
        // this repository has NO tests/ directory at all (confirmed by
        // direct file-tree inspection) -- report honestly, not assumed.
        println!("Q6 'Which tests cover the ListItem component?' -> total TestsFor edges in corpus: {}", tests_for_edges.len());

        // Q7: what does ListPage extend/implement (class-based
        // component question, included to test whether this pipeline
        // correctly reports ZERO rather than inventing something for a
        // functional-component-only codebase)? Ground truth: ListPage
        // is a functional component (React.FC), not a class -- it has
        // NO Extends/Implements edge, and this is the CORRECT answer,
        // not a gap.
        if let Some(list_page) = find_current_by_name_in_file("ListPage", "ListPage/index.tsx") {
            let extends: Vec<String> = graph.outgoing_edges(list_page.id(), Some(EdgeKind::Extends)).into_iter().filter_map(|e| graph.snapshot(e.to)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q7 'What does ListPage extend/implement?' -> {extends:?} (expected: empty -- ListPage is a functional component, not a class)");
        }
    }

    println!("\n=== done ===");
}

fn git_diff_name_only(repo_path: &Path, commit_a: &str, commit_b: &str) -> Vec<String> {
    let output = std::process::Command::new("git")
        .arg("diff")
        .arg("--name-only")
        .arg(commit_a)
        .arg(commit_b)
        .current_dir(repo_path)
        .output()
        .expect("git diff must run");
    String::from_utf8_lossy(&output.stdout).lines().map(|s| s.to_string()).filter(|s| !s.is_empty()).collect()
}

/// Counts raw tree-sitter nodes for constructs this pipeline's own
/// extractor (`extract/javascript.rs`, used for both JS and TS/Tsx)
/// does NOT walk into at all: `interface_declaration`,
/// `type_alias_declaration` (TypeScript-only node kinds), `internal_module`
/// (TS namespace/module), `decorator`, and JSX opening-element tag names
/// (`jsx_opening_element`/`jsx_self_closing_element`'s own identifier
/// child) as a proxy for "component references via JSX," which this
/// pipeline currently only sees indirectly through `resolve_calls`
/// treating a JSX element construction as a call expression, never as
/// its own typed relationship.
fn count_unpromoted_constructs(node: tree_sitter::Node, source: &[u8], out: &mut HashMap<&'static str, usize>) {
    match node.kind() {
        "interface_declaration" => *out.entry("interface_declaration").or_insert(0) += 1,
        "type_alias_declaration" => *out.entry("type_alias_declaration").or_insert(0) += 1,
        "internal_module" => *out.entry("internal_module (TS namespace)").or_insert(0) += 1,
        "decorator" => *out.entry("decorator").or_insert(0) += 1,
        "jsx_opening_element" | "jsx_self_closing_element" => *out.entry("jsx_element_reference (opening/self-closing tag)").or_insert(0) += 1,
        "export_statement" => {
            let text = node.utf8_text(source).unwrap_or("");
            if text.trim_start().starts_with("export default") {
                *out.entry("default_export").or_insert(0) += 1;
            } else if text.trim_start().starts_with("export *") {
                *out.entry("re_export (export *)").or_insert(0) += 1;
            } else {
                *out.entry("named_export").or_insert(0) += 1;
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_unpromoted_constructs(child, source, out);
    }
}

#[cfg(windows)]
fn print_approx_memory() {
    let pid = std::process::id();
    if let Ok(output) = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output() {
        let text = String::from_utf8_lossy(&output.stdout);
        println!("\n--- G. Performance observation (approx, Windows tasklist) ---");
        println!("{}", text.trim());
    }
}

#[cfg(not(windows))]
fn print_approx_memory() {}
