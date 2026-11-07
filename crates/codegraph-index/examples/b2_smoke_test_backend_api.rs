//! B2-EVAL-003: multi-codebase validation, Backend/API domain.
//!
//! Drives the SAME real production pipeline as
//! `b2_smoke_test.rs` (`RepositorySource` -> parse -> extract -> promote
//! -> lineage -> edges -> `CodeGraph` -> incremental reindex), against a
//! real, locally-cloned FastAPI backend repository
//! (`nsidnev/fastapi-realworld-example-app`, MIT) instead of the ML/DL
//! corpus `b2_smoke_test.rs` targets. The generic measurement sections
//! (A-G) are structurally identical to `b2_smoke_test.rs` by design, so
//! the two reports are directly comparable; only the domain-specific
//! Q&A section (H) differs, because it must be grounded in THIS
//! repository's own real route/service/repository call chains, not
//! PyTorch's model classes.
//!
//! Usage: `cargo run --example b2_smoke_test_backend_api -- <repo_path> <commit_a> <commit_b>`

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
        eprintln!("usage: b2_smoke_test_backend_api <repo_path> <commit_a> <commit_b>");
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

    println!("=== B2-EVAL-003: Backend/API domain smoke test ===");
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
    let mut python_files_with_import_statement_count = 0usize;

    // Per-file top-level-symbol counts, kept to measure the
    // Module/File -> top-level-symbol structural gap (Contains only
    // links Class -> member today; see
    // `codegraph_index::edges::contains`'s own module doc). A symbol is
    // counted as "top-level" here if it is NOT nested inside any other
    // snapshot's byte span in the same file (same test `Contains`
    // itself uses: strict span nesting), independent of SymbolKind.
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
        if relative_path.ends_with(".py") {
            let text = String::from_utf8_lossy(&bytes);
            if text.lines().any(|l| l.trim_start().starts_with("import ") || l.trim_start().starts_with("from ")) {
                python_files_with_import_statement_count += 1;
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

    // Module/File -> top-level symbol gap measurement: for each file,
    // a candidate span is "top-level" if no OTHER span in the same file
    // strictly contains it.
    for (file, spans) in &spans_by_file {
        let top_level = spans
            .iter()
            .filter(|(span, _)| !spans.iter().any(|(other, _)| other != span && other.start <= span.start && span.end <= other.end))
            .count();
        top_level_counts_by_file.insert(file.clone(), top_level);
    }

    // Detect (don't just crash on) duplicate SymbolSnapshotIds within
    // the real snapshot pool, so a real collision surfaces as a report
    // finding rather than an unexplained panic further down.
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

    // Detect duplicate (from, to, kind) typed-edge triples within one
    // resolver's own output — the same shape CodeGraph::insert_edge
    // rejects, surfaced here as a report finding instead of a panic.
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
    println!("python_files_with_at_least_one_import_statement: {python_files_with_import_statement_count}");

    println!("\n--- C. Symbol coverage (SymbolKind counts) ---");
    let mut kind_counts_vec: Vec<_> = kind_counts.into_iter().collect();
    kind_counts_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &kind_counts_vec {
        println!("  {kind:?}: {count}");
    }

    println!("\n--- C2. Module/File -> top-level-symbol gap (Backend/API domain stress case) ---");
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

    // --- Manual sampling aid: dump a deterministic sample of Calls
    // edges (resolved + heuristic) with enough context for a human to
    // judge correctness by reading the real source — not computed as
    // a metric itself, purely a fixed, reproducible sample selection.
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
        if !relative_path.ends_with(".py") {
            continue;
        }
        let file = codegraph_domain::FileIdentity::parse(repo_id, commit_b.clone(), relative_path).unwrap();
        if let Ok(bytes) = source.read_file(&file) {
            let text = String::from_utf8_lossy(&bytes);
            total_import_statements_line_based += text.lines().filter(|l| l.trim_start().starts_with("import ") || l.trim_start().starts_with("from ")).count();
        }
    }
    print_approx_memory();

    // --- F. Incremental A -> B via CodeGraphIndexer ---
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

    // --- Imports measurement, using the REAL indexer's own
    // `import_facts` output — extraction coverage and resolution
    // accuracy kept separate, per the Evidence-before-claims
    // Experimental Policy.
    {
        let python_import_facts = outcome_b.import_facts.iter().filter(|f| f.importing_file.relative_path().ends_with(".py")).count();
        let internal_count = outcome_b.import_facts.iter().filter(|f| matches!(f.resolution, ImportResolution::Internal(_))).count();
        let external_count = outcome_b.import_facts.iter().filter(|f| f.resolution == ImportResolution::External).count();
        let ambiguous_count = outcome_b.import_facts.iter().filter(|f| f.resolution == ImportResolution::Ambiguous).count();
        let alias_count = outcome_b.import_facts.iter().filter(|f| f.alias.is_some()).count();

        println!("\n--- Imports measurement (via the real CodeGraphIndexer, extraction coverage vs. resolution accuracy kept separate) ---");
        println!("total_import_statements_in_python_files_at_commit_b (line-based count, MEASURED): {total_import_statements_line_based}");
        println!("python_import_facts_extracted (MEASURED, ImportFact, file-level): {python_import_facts}");
        println!(
            "extraction_coverage: {python_import_facts}/{total_import_statements_line_based} ({:.1}%)",
            if total_import_statements_line_based > 0 {
                100.0 * python_import_facts as f64 / total_import_statements_line_based as f64
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

    // --- H. Backend/API coding-agent usefulness Q&A, answered using
    // ONLY the real CodeGraph built above (observe_commit(B)'s final
    // graph). Every question/expected-target pair below was derived
    // from direct reading of this repository's real source at commit B
    // (see B2-EVAL-003.md for the exact file/line citations), not
    // invented or adjusted to make the pipeline look good.
    {
        let graph = &outcome_b.graph;
        println!("\n--- H. Backend/API usefulness Q&A (fastapi-realworld-example-app, commit B) ---");

        let find_by_name = |name: &str| -> Vec<&codegraph_index::snapshot::SymbolSnapshot> { graph.all_snapshots().into_iter().filter(|s| s.name().as_str() == name).collect() };
        let find_current_by_name_in_file = |name: &str, file_suffix: &str| -> Option<&codegraph_index::snapshot::SymbolSnapshot> {
            find_by_name(name)
                .into_iter()
                .filter(|s| graph.lineage_from(s.id()).is_empty())
                .find(|s| s.file().relative_path().ends_with(file_suffix))
        };

        // Q1: from which route does registration start, and what does
        // it call? Ground truth (app/api/routes/authentication.py):
        // `register` calls `check_username_is_taken`,
        // `check_email_is_taken` (app/services/authentication.py, free
        // functions), `users_repo.create_user` (receiver-qualified,
        // app/db/repositories/users.py::UsersRepository.create_user),
        // and `jwt.create_access_token_for_user`
        // (app/services/jwt.py).
        if let Some(register) = find_current_by_name_in_file("register", "routes/authentication.py") {
            let callee_names: Vec<String> = graph.callees_of(register.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q1 'From which route does user registration start, and what does it call?' -> auth:register calls: {callee_names:?}");
        } else {
            println!("Q1 'register' route handler NOT FOUND in app/api/routes/authentication.py (unexpected)");
        }

        // Q2: what does the login route call, and does it reach the
        // database layer? Ground truth: `login` calls
        // `users_repo.get_user_by_email` (receiver-qualified,
        // UsersRepository) and `jwt.create_access_token_for_user`.
        if let Some(login) = find_current_by_name_in_file("login", "routes/authentication.py") {
            let callee_names: Vec<String> = graph.callees_of(login.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q2 'What does the login route call, and does it reach the data layer?' -> auth:login calls: {callee_names:?}");
        } else {
            println!("Q2 'login' route handler NOT FOUND (unexpected)");
        }

        // Q3: which concrete repository class does the data layer use
        // for users, and what does it extend? Ground truth:
        // app/db/repositories/users.py::UsersRepository extends
        // app/db/repositories/base.py::BaseRepository.
        if let Some(users_repo_class) = find_current_by_name_in_file("UsersRepository", "repositories/users.py") {
            let extends: Vec<String> = graph.outgoing_edges(users_repo_class.id(), Some(EdgeKind::Extends)).into_iter().filter_map(|e| graph.snapshot(e.to)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q3 'What does UsersRepository (the users data-layer component) extend?' -> {extends:?}");
        } else {
            println!("Q3 UsersRepository class NOT FOUND (unexpected)");
        }

        // Q4: if I modify check_username_is_taken (a service-layer
        // function), what route handlers are impacted? Ground truth
        // (manually confirmed by direct source reading): called from
        // BOTH app/api/routes/authentication.py::register AND
        // app/api/routes/users.py::update_current_user.
        if let Some(check_username) = find_current_by_name_in_file("check_username_is_taken", "services/authentication.py") {
            let callers: Vec<String> = graph.callers_of(check_username.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q4 'If I modify check_username_is_taken, which route handlers are impacted?' -> callers: {callers:?}");
        } else {
            println!("Q4 check_username_is_taken NOT FOUND (unexpected)");
        }

        // Q5: does the service-layer check_username_is_taken reach the
        // database layer, and through which repository method? Ground
        // truth: calls repo.get_user_by_username (receiver-qualified,
        // receiver is the `repo: UsersRepository` parameter, not
        // `self` — a distinct receiver-resolution shape from Q1-Q4's
        // `self.*`/instance-attribute receivers).
        if let Some(check_username) = find_current_by_name_in_file("check_username_is_taken", "services/authentication.py") {
            let callees: Vec<String> = graph.callees_of(check_username.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q5 'Does check_username_is_taken reach the database layer? Through which method?' -> calls: {callees:?}");
        }

        // Q6: which tests cover UsersRepository or the authentication
        // routes? Ground truth: this repository DOES have a tests/
        // directory (unlike the ML corpus) — report whatever TestsFor
        // edges actually resolve, honestly, rather than assuming none
        // exist.
        println!("Q6 'Which tests cover the users data layer or authentication routes?' -> total TestsFor edges in corpus: {}", tests_for_edges.len());
        if let Some(users_repo_class) = find_current_by_name_in_file("UsersRepository", "repositories/users.py") {
            let incoming_tests = graph.incoming_edges(users_repo_class.id(), Some(EdgeKind::TestsFor));
            let names: Vec<String> = incoming_tests.iter().filter_map(|e| graph.snapshot(e.from)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("  TestsFor edges pointing at UsersRepository: {names:?}");
        }

        // Q7: what does the current_user dependency inject, and who
        // depends on it (DI chain)? Ground truth:
        // app/api/dependencies/authentication.py::get_current_user_authorizer
        // is used via Depends(...) in route handlers — this is a
        // CALL-shaped relationship only if the pipeline's call
        // resolver treats `Depends(get_current_user_authorizer())` as
        // a real call site (it is a real Python call expression,
        // regardless of FastAPI's own runtime DI semantics).
        if let Some(authorizer) = find_current_by_name_in_file("get_current_user_authorizer", "dependencies/authentication.py") {
            let callers: Vec<String> = graph.callers_of(authorizer.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q7 'Who depends on (calls) get_current_user_authorizer (the auth DI dependency)?' -> callers: {callers:?}");
        } else {
            println!("Q7 get_current_user_authorizer NOT FOUND (unexpected)");
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
