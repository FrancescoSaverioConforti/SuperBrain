//! B2-EVAL-005: multi-codebase validation, Rust/systems domain.
//!
//! Drives the SAME real production pipeline as the prior three
//! domain-specific harnesses (`RepositorySource` -> parse -> extract ->
//! promote -> lineage -> edges -> `CodeGraph` -> incremental reindex),
//! against a real, locally-cloned Rust CLI tool (`sharkdp/fd`) instead
//! of the ML/DL, Backend/API, or Frontend/UI corpora the prior three
//! targeted. This domain's own distinguishing addition is Section C1
//! (quantified macro-opacity gap) and Section H's trait/impl-focused
//! Q&A, since this is the first domain where the pipeline analyzes its
//! OWN implementation language.
//!
//! Usage: `cargo run --example b2_smoke_test_rust_systems -- <repo_path> <commit_a> <commit_b>`

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
        eprintln!("usage: b2_smoke_test_rust_systems <repo_path> <commit_a> <commit_b>");
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

    println!("=== B2-EVAL-005: Rust/systems domain smoke test ===");
    println!("repo: {}", repo_path.display());
    println!("commit_b: {commit_b_str}");

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
    let mut rust_files_with_use_statement_count = 0usize;

    // Macro-opacity quantification (this domain's own distinguishing
    // measurement): count real `macro_invocation` nodes, their
    // `token_tree` argument nodes, and -- within each token_tree's own
    // raw TEXT -- a conservative, deliberately approximate scan for
    // "looks like a call expression" (`identifier(` or `identifier::identifier(`
    // immediately followed by a `(`), which `resolve_calls` cannot see
    // because tree-sitter never parses inside a token_tree at all. This
    // text-based proxy is NOT a claim of resolver-grade accuracy -- it
    // exists only to give an order-of-magnitude, honestly-labeled lower
    // bound on how many real call-shaped expressions are hidden from
    // the real pipeline by macro opacity, separate from whatever the
    // real `Calls` resolver itself reports.
    let mut macro_invocation_count = 0usize;
    let mut token_tree_count = 0usize;
    let mut call_shaped_substrings_inside_token_trees = 0usize;
    let mut macro_rules_definitions = 0usize;

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
        let is_rust = relative_path.ends_with(".rs");
        if is_rust {
            let text = String::from_utf8_lossy(&bytes);
            if text.lines().any(|l| l.trim_start().starts_with("use ")) {
                rust_files_with_use_statement_count += 1;
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

        if is_rust {
            scan_macro_opacity(
                tree.tree().root_node(),
                &bytes,
                &mut macro_invocation_count,
                &mut token_tree_count,
                &mut call_shaped_substrings_inside_token_trees,
                &mut macro_rules_definitions,
            );
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
    println!("rust_files_with_at_least_one_use_statement: {rust_files_with_use_statement_count}");

    println!("\n--- C. Symbol coverage (SymbolKind counts) ---");
    let mut kind_counts_vec: Vec<_> = kind_counts.into_iter().collect();
    kind_counts_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &kind_counts_vec {
        println!("  {kind:?}: {count}");
    }

    println!("\n--- C1. Macro-opacity quantification (Rust/systems domain stress case) ---");
    println!("macro_invocation_nodes (MEASURED, real parse): {macro_invocation_count}");
    println!("token_tree_nodes (MEASURED, the opaque argument bodies of those invocations): {token_tree_count}");
    println!("macro_rules_definitions (MEASURED): {macro_rules_definitions}");
    println!(
        "call_shaped_substrings_inside_token_trees (Inferred, conservative text-based proxy, NOT resolver-grade): {call_shaped_substrings_inside_token_trees}"
    );
    println!(
        "note: these call-shaped substrings are, BY CONSTRUCTION, never visible to resolve_calls's own tree-sitter walk \
         (a token_tree is never a call_expression node) -- this is the quantified form of the known gap documented in \
         codegraph-index::edges::calls's own module doc comment, not a new finding about the resolver's correctness."
    );

    println!("\n--- C2. Module/File -> top-level-symbol gap (Rust/systems domain stress case) ---");
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

    // --- D1. All Extends/Implements edges printed in full (not just a
    // sample) -- this corpus's own trait-implementation count is small
    // enough (2 traits, <=10 impls) that the full set is more useful
    // than a sample, and lets every impl be ground-truth-checked.
    {
        println!("\n--- D1. All Extends/Implements (trait-impl) edges, full set ---");
        let mut impls: Vec<&codegraph_index::edges::Edge> = extends_edges.iter().collect();
        impls.sort_by(|a, b| a.evidence.note().cmp(b.evidence.note()));
        for edge in &impls {
            let from = snapshots.iter().find(|s| s.id() == edge.from);
            let to = snapshots.iter().find(|s| s.id() == edge.to);
            println!(
                "{:?} [{:?}] -> {:?} [{:?}/{:?}] :: {}",
                from.map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())),
                from.map(|s| s.kind()),
                to.map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())),
                edge.kind,
                edge.evidence.method(),
                edge.evidence.note()
            );
        }
    }

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

    let mut total_use_statements_line_based = 0usize;
    for relative_path in &files {
        if !relative_path.ends_with(".rs") {
            continue;
        }
        let file = codegraph_domain::FileIdentity::parse(repo_id, commit_b.clone(), relative_path).unwrap();
        if let Ok(bytes) = source.read_file(&file) {
            let text = String::from_utf8_lossy(&bytes);
            total_use_statements_line_based += text.lines().filter(|l| l.trim_start().starts_with("use ")).count();
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

    // B2-ID-008 C1 fix verification: confirm incremental A->B and a FULL
    // reindex of commit B (treated as a brand-new repository, via a
    // fresh CodeGraphIndexer) produce the SAME ImplFact-derived
    // Implements edge count and the SAME trait_resolution Resolved
    // count. This is the production-evaluation counterpart to the
    // unit-level parity test the C1 fix wave already added -- this one
    // runs the real harness against the real external repository
    // rather than a synthetic fixture.
    {
        let fresh_indexer = CodeGraphIndexer::new();
        let full_b_outcome = fresh_indexer.observe_commit(&source, repo_id, &commit_b, None, &[]).expect("full reindex of commit B as a new repository");
        let incremental_implements = outcome_b.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count();
        let full_implements = full_b_outcome.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count();
        let incremental_trait_resolved = outcome_b
            .impl_facts
            .iter()
            .filter(|f| matches!(f.trait_resolution, Some(codegraph_index::impls::ImplTargetResolution::Resolved(_))))
            .count();
        let full_trait_resolved = full_b_outcome
            .impl_facts
            .iter()
            .filter(|f| matches!(f.trait_resolution, Some(codegraph_index::impls::ImplTargetResolution::Resolved(_))))
            .count();
        println!("\n--- B2-ID-008 C1 fix: incremental vs. full-reindex parity on commit B (production evaluation, not just unit tests) ---");
        println!("incremental_implements_edges: {incremental_implements}");
        println!("full_reindex_implements_edges: {full_implements}");
        println!("parity (incremental == full): {}", incremental_implements == full_implements);
        println!("incremental_trait_resolved: {incremental_trait_resolved}");
        println!("full_reindex_trait_resolved: {full_trait_resolved}");
        println!("trait_resolved_parity (incremental == full): {}", incremental_trait_resolved == full_trait_resolved);
    }

    {
        let rust_import_facts = outcome_b.import_facts.iter().filter(|f| f.importing_file.relative_path().ends_with(".rs")).count();
        let internal_count = outcome_b.import_facts.iter().filter(|f| matches!(f.resolution, ImportResolution::Internal(_))).count();
        let external_count = outcome_b.import_facts.iter().filter(|f| f.resolution == ImportResolution::External).count();
        let ambiguous_count = outcome_b.import_facts.iter().filter(|f| f.resolution == ImportResolution::Ambiguous).count();
        let alias_count = outcome_b.import_facts.iter().filter(|f| f.alias.is_some()).count();

        println!("\n--- Imports measurement (via the real CodeGraphIndexer, extraction coverage vs. resolution accuracy kept separate) ---");
        println!("total_use_statements_in_rust_files_at_commit_b (line-based count, MEASURED): {total_use_statements_line_based}");
        println!("rust_import_facts_extracted (MEASURED, ImportFact, file-level): {rust_import_facts}");
        println!(
            "extraction_coverage: {rust_import_facts}/{total_use_statements_line_based} ({:.1}%)",
            if total_use_statements_line_based > 0 {
                100.0 * rust_import_facts as f64 / total_use_statements_line_based as f64
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

    // --- B2-ID-008 re-measurement (Task 5): ImplFact/raw-impl-statement
    // metrics, surfaced explicitly because this harness predates the
    // ImplFact fix and its existing fields (edge counts via
    // extends_edges/outcome_b.graph) do not by themselves show the
    // raw-extraction / self-type-resolution / trait-resolution /
    // associated-item breakdown Task 5 needs. Every number here is
    // MEASURED directly from outcome_b.impl_facts (ObserveOutcome's own
    // Task-4-added field) and outcome_b.metrics -- the real, fixed
    // production pipeline, not a recomputation.
    {
        use codegraph_index::impls::{AssociatedItemKind, ImplTargetResolution};

        let impl_facts = &outcome_b.impl_facts;
        let total_impl_facts = impl_facts.len();
        let trait_impls: Vec<_> = impl_facts.iter().filter(|f| f.trait_type_text.is_some()).collect();
        let inherent_impls: Vec<_> = impl_facts.iter().filter(|f| f.trait_type_text.is_none()).collect();
        let generic_impls = impl_facts.iter().filter(|f| f.has_generics).count();
        let lifetime_impls = impl_facts.iter().filter(|f| f.has_lifetime_params).count();

        let trait_resolved = trait_impls.iter().filter(|f| matches!(f.trait_resolution, Some(ImplTargetResolution::Resolved(_)))).count();
        let trait_unresolved = trait_impls.iter().filter(|f| matches!(f.trait_resolution, Some(ImplTargetResolution::Unresolved))).count();
        let trait_ambiguous = trait_impls.iter().filter(|f| matches!(f.trait_resolution, Some(ImplTargetResolution::Ambiguous))).count();

        let self_resolved = impl_facts.iter().filter(|f| matches!(f.self_type_resolution, ImplTargetResolution::Resolved(_))).count();
        let self_unresolved = impl_facts.iter().filter(|f| matches!(f.self_type_resolution, ImplTargetResolution::Unresolved)).count();
        let self_ambiguous = impl_facts.iter().filter(|f| matches!(f.self_type_resolution, ImplTargetResolution::Ambiguous)).count();

        let implements_edge_count = outcome_b.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count();

        let all_associated_items: Vec<_> = impl_facts.iter().flat_map(|f| f.associated_items.iter()).collect();
        let method_count = all_associated_items.iter().filter(|i| i.kind == AssociatedItemKind::Method).count();
        let assoc_fn_count = all_associated_items.iter().filter(|i| i.kind == AssociatedItemKind::AssociatedFunction).count();

        println!("\n--- B2-ID-008: ImplFact / Implements-edge measurement (via ObserveOutcome.impl_facts, the real fixed pipeline) ---");
        println!("raw_impl_statements_extracted_this_call (MEASURED, IndexMetrics): {}", outcome_b.metrics.raw_impl_statements_extracted);
        println!("raw_impl_statements_reused_this_call (MEASURED, IndexMetrics): {}", outcome_b.metrics.raw_impl_statements_reused);
        println!("impl_facts_total (MEASURED, ObserveOutcome.impl_facts, final published state): {total_impl_facts}");
        println!("  trait_impls (trait_type_text: Some(_)): {}/{total_impl_facts}", trait_impls.len());
        println!("  inherent_impls (trait_type_text: None): {}/{total_impl_facts}", inherent_impls.len());
        println!("  generic_impls (has_generics): {generic_impls}/{total_impl_facts}");
        println!("  lifetime_impls (has_lifetime_params): {lifetime_impls}/{total_impl_facts}");
        println!(
            "  of the trait impls, trait_resolution breakdown: Resolved {}/{t}, Unresolved {}/{t}, Ambiguous {}/{t}",
            trait_resolved,
            trait_unresolved,
            trait_ambiguous,
            t = trait_impls.len()
        );
        println!(
            "  self_type_resolution breakdown (all impl_facts, inherent + trait): Resolved {}/{total_impl_facts}, Unresolved {}/{total_impl_facts}, Ambiguous {}/{total_impl_facts}",
            self_resolved, self_unresolved, self_ambiguous
        );
        println!("implements_edges_in_final_graph (MEASURED, EdgeKind::Implements count in outcome_b.graph): {implements_edge_count}");
        println!("associated_items_total (MEASURED, summed across all impl_facts): {}", all_associated_items.len());
        println!("  associated_items_method: {method_count}");
        println!("  associated_items_associated_function: {assoc_fn_count}");

        // Full impl_facts dump, mirroring Section D1's own "full set, not
        // a sample" convention (this corpus's own ImplFact count is small
        // enough that every fact is more useful printed than sampled).
        println!("\n--- B2-ID-008 (cont.): full impl_facts dump (self_type, trait_type_text, resolution) ---");
        for f in impl_facts.iter() {
            println!(
                "  self={} trait={:?} self_res={:?} trait_res={:?} file={}",
                f.self_type_text,
                f.trait_type_text,
                f.self_type_resolution,
                f.trait_resolution,
                f.defining_file.relative_path()
            );
        }
    }

    // --- H. Rust/systems coding-agent usefulness Q&A, answered using
    // ONLY the real CodeGraph built above (observe_commit(B)'s final
    // graph). Every question/expected-target pair below was derived
    // from direct reading of this repository's real source at commit B
    // (see B2-EVAL-005.md for the exact file/line citations), not
    // invented or adjusted to make the pipeline look good.
    {
        let graph = &outcome_b.graph;
        println!("\n--- H. Rust/systems usefulness Q&A (sharkdp/fd, commit B) ---");

        let find_by_name = |name: &str| -> Vec<&codegraph_index::snapshot::SymbolSnapshot> { graph.all_snapshots().into_iter().filter(|s| s.name().as_str() == name).collect() };
        let find_current_by_name_in_file = |name: &str, file_suffix: &str| -> Option<&codegraph_index::snapshot::SymbolSnapshot> {
            find_by_name(name)
                .into_iter()
                .filter(|s| graph.lineage_from(s.id()).is_empty())
                .find(|s| s.file().relative_path().ends_with(file_suffix))
        };

        // Q1: which traits does FixedStrings implement? Ground truth
        // (src/patterns.rs): `impl Matcher for FixedStrings`.
        if let Some(fixed_strings) = find_current_by_name_in_file("FixedStrings", "patterns.rs") {
            let implements: Vec<String> = graph.outgoing_edges(fixed_strings.id(), Some(EdgeKind::Implements)).into_iter().filter_map(|e| graph.snapshot(e.to)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q1 'Which trait(s) does FixedStrings implement?' -> {implements:?}");
        } else {
            println!("Q1 'FixedStrings' NOT FOUND in src/patterns.rs (unexpected)");
        }

        // Q2: which types implement the Matcher trait? Ground truth:
        // RegexSet, GlobSet, GlobMatcher, FixedStrings, ExactStrings,
        // MatchAll -- six real impls, confirmed by direct source
        // reading.
        if let Some(matcher_trait) = find_current_by_name_in_file("Matcher", "patterns.rs") {
            let implementors: Vec<String> = graph.incoming_edges(matcher_trait.id(), Some(EdgeKind::Implements)).into_iter().filter_map(|e| graph.snapshot(e.from)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q2 'Which types implement the Matcher trait?' -> {implementors:?}");
        } else {
            println!("Q2 'Matcher' trait NOT FOUND in src/patterns.rs (unexpected)");
        }

        // Q3: which method(s) belong to `impl Matcher for GlobMatcher`?
        // Ground truth: exactly one, matches_path (the trait's own sole
        // method). Checked via Contains, since Contains is this
        // pipeline's own "which methods belong to this type" edge.
        if let Some(glob_matcher) = find_current_by_name_in_file("GlobMatcher", "patterns.rs") {
            let members: Vec<String> = graph.outgoing_edges(glob_matcher.id(), Some(EdgeKind::Contains)).into_iter().filter_map(|e| graph.snapshot(e.to)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q3 'Which methods belong to impl Matcher for GlobMatcher?' -> Contains edges from GlobMatcher: {members:?} (expected: this is a Rust impl-on-an-external-type shape -- GlobMatcher itself is not a locally-defined struct, so check whether Contains even applies here)");
        } else {
            println!("Q3 'GlobMatcher' NOT FOUND as a local snapshot (expected -- GlobMatcher is defined in the external `globset` crate, not in this repository's own source; this pipeline has no symbol for it to attach a Contains edge to)");
        }

        // Q4: if I modify build_patterns (the factory function), which
        // call sites are impacted? Ground truth: build_patterns is
        // called from cli.rs/config.rs's own argument-processing code.
        if let Some(build_patterns) = find_current_by_name_in_file("build_patterns", "patterns.rs") {
            let callers: Vec<String> = graph.callers_of(build_patterns.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q4 'If I modify build_patterns, which call sites are impacted?' -> callers: {callers:?}");
        } else {
            println!("Q4 'build_patterns' NOT FOUND in src/patterns.rs (unexpected)");
        }

        // Q5: which types implement the SanitizeErr trait (the SECOND
        // real trait in this corpus, defined inside the print_error!
        // macro's own module, error.rs)? Ground truth: &str, io::Error,
        // ignore::Error -- three real impls.
        if let Some(sanitize_err_trait) = find_current_by_name_in_file("SanitizeErr", "error.rs") {
            let implementors: Vec<String> = graph.incoming_edges(sanitize_err_trait.id(), Some(EdgeKind::Implements)).into_iter().filter_map(|e| graph.snapshot(e.from)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q5 'Which types implement the SanitizeErr trait?' -> {implementors:?}");
        } else {
            println!("Q5 'SanitizeErr' trait NOT FOUND in src/error.rs (unexpected)");
        }

        // Q6: which tests cover the patterns module? Ground truth:
        // patterns.rs itself has NO #[cfg(test)] mod tests of its own
        // (confirmed by direct file inspection) -- report honestly.
        println!("Q6 'Which tests cover the patterns module?' -> total TestsFor edges in corpus: {}", tests_for_edges.len());

        // Q7: what is the call flow from main's entry point into the
        // walk subsystem? Ground truth: main.rs's own main() calls
        // walk::run (or equivalent) -- checked directly rather than
        // assumed, since this is exactly the kind of "entry point to
        // subsystem" question a coding agent would ask.
        if let Some(main_fn) = find_current_by_name_in_file("main", "main.rs") {
            let callees: Vec<String> = graph.callees_of(main_fn.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q7 'What is the call flow from main() into the walk subsystem?' -> main() calls: {callees:?}");
        } else {
            println!("Q7 'main' function NOT FOUND in src/main.rs (unexpected)");
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

/// Walks the real parse tree counting `macro_invocation` nodes, their
/// `token_tree` argument-body children, `macro_rules!` definitions
/// (`macro_definition` node kind in tree-sitter-rust), and -- inside
/// each `token_tree`'s own raw source text -- a conservative,
/// deliberately approximate regex-free scan for "looks like a call":
/// an identifier (or `a::b`-qualified path) immediately followed by
/// `(`. This is a TEXT heuristic over the opaque token_tree's own raw
/// bytes, not a real parse (tree-sitter does not parse inside a
/// token_tree, which is exactly the gap being measured) -- it exists
/// only to give an honest, labeled lower-bound order-of-magnitude
/// count, never conflated with the real `Calls` resolver's own
/// tree-sitter-backed results.
fn scan_macro_opacity(
    node: tree_sitter::Node,
    source: &[u8],
    macro_invocation_count: &mut usize,
    token_tree_count: &mut usize,
    call_shaped_substrings: &mut usize,
    macro_rules_definitions: &mut usize,
) {
    match node.kind() {
        "macro_invocation" => *macro_invocation_count += 1,
        "macro_definition" => *macro_rules_definitions += 1,
        "token_tree" => {
            *token_tree_count += 1;
            if let Ok(text) = node.utf8_text(source) {
                *call_shaped_substrings += count_call_shaped_substrings(text);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        scan_macro_opacity(child, source, macro_invocation_count, token_tree_count, call_shaped_substrings, macro_rules_definitions);
    }
}

/// Conservative, hand-rolled (no regex dependency) scan for
/// `identifier(` or `identifier::identifier(` immediately followed by
/// `(`, skipping Rust keywords that syntactically precede `(` but are
/// not calls (`if`, `for`, `while`, `match`, `return`, `fn`). Intended
/// as a lower bound, not a precise count: it will miss calls with
/// whitespace/comments between the identifier and `(` (rare in
/// practice) and will not attempt macro-inside-macro recursion beyond
/// what the outer token_tree's own flat text already contains.
fn count_call_shaped_substrings(text: &str) -> usize {
    const KEYWORDS: &[&str] = &["if", "for", "while", "match", "return", "fn", "let", "in", "else"];
    let bytes = text.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b':') {
                i += 1;
            }
            let ident = &text[start..i];
            let last_segment = ident.rsplit("::").next().unwrap_or(ident);
            if i < bytes.len() && bytes[i] == b'(' && !KEYWORDS.contains(&last_segment) && !last_segment.is_empty() {
                count += 1;
            }
        } else {
            i += 1;
        }
    }
    count
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
