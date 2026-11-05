//! B2 real-repository smoke test checkpoint (not part of the crate's own
//! test suite, not a benchmark, not a persistent feature). Drives the
//! REAL production pipeline (`RepositorySource` -> parse -> extract ->
//! promote -> lineage -> edges -> `CodeGraph` -> incremental reindex)
//! against a real, locally-cloned repository, and prints a quantitative
//! report this checkpoint's human-written report is built from.
//!
//! Usage: `cargo run --example b2_smoke_test -- <repo_path> <commit_a> <commit_b>`
//!
//! No fixture-specific hacks: this uses exactly the public API
//! (`LocalGitRepositorySource`, `extract_symbols_reporting_rejections`,
//! `promote`, `resolve_lineage`, `resolve_calls`/`resolve_imports`/
//! `resolve_extends_implements`/`resolve_tests_for`, `CodeGraph`,
//! `CodeGraphIndexer`) that any other caller of this crate would use.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use codegraph_domain::{CommitHash, EdgeKind, RepositorySource, ResolutionMethod, SymbolKind};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::edges::{resolve_calls, resolve_extends_implements, resolve_imports, resolve_tests_for};
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::incremental::CodeGraphIndexer;
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: b2_smoke_test <repo_path> <commit_a> <commit_b>");
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

    println!("=== B2 real-repository smoke test ===");
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
            match promote(&candidate, file.clone()) {
                Ok(snapshot) => {
                    *kind_counts.entry(snapshot.kind()).or_insert(0) += 1;
                    snapshots.push(snapshot);
                }
                Err(e) => promotion_failures.push(format!("{relative_path}: {e}")),
            }
        }
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
    let imports_edges = resolve_imports(&snapshots).expect("resolve_imports");
    let extends_edges = resolve_extends_implements(&snapshots).expect("resolve_extends_implements");
    let tests_for_edges = resolve_tests_for(&snapshots).expect("resolve_tests_for");

    // Detect duplicate (from, to, kind) typed-edge triples within one
    // resolver's own output — the same shape CodeGraph::insert_edge
    // rejects, surfaced here as a report finding instead of a panic.
    {
        let mut seen: HashMap<(codegraph_domain::SymbolSnapshotId, codegraph_domain::SymbolSnapshotId, EdgeKind), usize> = HashMap::new();
        for edge in calls_edges.iter().chain(imports_edges.iter()).chain(extends_edges.iter()).chain(tests_for_edges.iter()) {
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
    for edge in calls_edges.iter().chain(imports_edges.iter()).chain(extends_edges.iter()).chain(tests_for_edges.iter()) {
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

    println!("\n--- D. Edge quality ---");
    println!("lineage_edges (B alone, no prev): {}", lineage_edges.len());
    println!("calls_edges: {}", calls_edges.len());
    println!("imports_edges: {}", imports_edges.len());
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
        println!("\n--- Manual call-quality sample (first 20 by evidence note, deterministic) ---");
        for edge in sample.iter().take(20) {
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

    // --- Imports gap measurement: count real import statements across
    // the Python files that actually contributed a promoted snapshot,
    // vs. how many Imports edges were produced — a direct empirical
    // check of the Task 7 finding.
    {
        let mut total_import_statements = 0usize;
        for relative_path in &files {
            if !relative_path.ends_with(".py") {
                continue;
            }
            let file = codegraph_domain::FileIdentity::parse(repo_id, commit_b.clone(), relative_path).unwrap();
            if let Ok(bytes) = source.read_file(&file) {
                let text = String::from_utf8_lossy(&bytes);
                total_import_statements += text.lines().filter(|l| l.trim_start().starts_with("import ") || l.trim_start().starts_with("from ")).count();
            }
        }
        println!("\n--- Imports gap measurement ---");
        println!("total_import_statements_in_python_files (line-based count, MEASURED): {total_import_statements}");
        println!("imports_edges_produced (MEASURED): {}", imports_edges.len());
    }

    // --- G. Performance observation: approximate RSS (best-effort, Windows) ---
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

    // Determine the exact changed-files list between A and B via git2
    // directly (the SAME information a real caller diffing two commits
    // would supply to observe_commit — this harness does not hardcode
    // which files changed, it discovers them).
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

    // --- Deep-learning usefulness Q&A, answered using ONLY the real
    // CodeGraph built above (observe_commit(B)'s final graph) ---
    {
        let graph = &outcome_b.graph;
        println!("\n--- Deep-learning usefulness Q&A (word_language_model, commit B) ---");

        let find_by_name = |name: &str| -> Vec<&codegraph_index::snapshot::SymbolSnapshot> { graph.all_snapshots().into_iter().filter(|s| s.name().as_str() == name).collect() };

        let transformer_model = find_by_name("TransformerModel");
        let current_transformer_model = transformer_model.iter().find(|s| graph.lineage_from(s.id()).is_empty());
        println!(
            "Q1 'What is the main model class?' -> TransformerModel snapshots found: {} (current+historical; disambiguated below)",
            transformer_model.len()
        );

        if let Some(model) = current_transformer_model {
            let extends = graph.outgoing_edges(model.id(), Some(EdgeKind::Extends));
            println!(
                "Q2 'What does the CURRENT TransformerModel extend?' -> {} Extends edge(s): {:?}",
                extends.len(),
                extends.iter().map(|e| e.to).collect::<Vec<_>>()
            );
        }

        let forward_candidates = find_by_name("forward");
        println!("Q3 'What composes the forward path?' -> {} snapshot(s) named 'forward' in the whole pool (ambiguous by name alone across 97 files)", forward_candidates.len());
        // Narrow to the CURRENT one inside word_language_model/model.py
        // specifically — never the historical (pre-edit) one. The
        // current snapshot is never the `from` of an outgoing lineage
        // edge (nothing supersedes it); the historical one always is.
        // `all_snapshots()`'s deterministic byte-order sort gives no
        // guarantee about which of the two same-named, same-path
        // snapshots a naive `.find()` would hit first — this disambig-
        // uation step exists specifically because of that, and is
        // itself a concrete illustration of the documented residual
        // limitation from Task 9/B2-ID-002 (CodeGraph has no current-
        // vs-historical distinction of its own).
        let model_py_forward_candidates: Vec<_> = forward_candidates.iter().filter(|s| s.file().relative_path() == "word_language_model/model.py").collect();
        let current_forward = model_py_forward_candidates.iter().find(|s| graph.lineage_from(s.id()).is_empty());
        println!(
            "  (word_language_model/model.py has {} 'forward' snapshot(s) in the final graph: current+historical combined)",
            model_py_forward_candidates.len()
        );
        if let Some(forward) = current_forward {
            let callees = graph.callees_of(forward.id());
            let callee_names: Vec<String> = callees.iter().filter_map(|id| graph.snapshot(*id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("  CURRENT word_language_model/model.py::forward calls: {callee_names:?}");
        } else {
            println!("  CURRENT word_language_model/model.py::forward NOT FOUND (unexpected)");
        }

        println!("Q4 'Who calls the model?' -> searching train.py/main.py callers of TransformerModel/RNNModel constructors:");
        for model_name in ["TransformerModel", "RNNModel"] {
            if let Some(model) = find_by_name(model_name).first() {
                let callers = graph.callers_of(model.id());
                let caller_names: Vec<String> = callers.iter().filter_map(|id| graph.snapshot(*id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
                println!("  callers of {model_name}: {caller_names:?}");
            }
        }

        println!("Q5 'Which tests are linked?' -> word_language_model has no test file in this repo; TestsFor edges in the whole corpus: {}", tests_for_edges.len());

        // Diagnostic: is `_generate_square_subsequent_mask` (a REAL
        // same-class method `forward` calls) promoted as its own
        // snapshot at all? If not found, that is itself a pipeline
        // observation worth recording, not an edge-resolution bug.
        let gssm = find_by_name("_generate_square_subsequent_mask");
        println!(
            "Diagnostic: '_generate_square_subsequent_mask' snapshots found: {} (in files: {:?})",
            gssm.len(),
            gssm.iter().map(|s| s.file().relative_path()).collect::<Vec<_>>()
        );

        if let Some(forward) = current_forward {
            println!("Q6 'If I modify forward, what is impacted?' -> incoming edges to the CURRENT forward (who is affected if its signature/behavior changes):");
            let incoming = graph.incoming_edges(forward.id(), None);
            for edge in &incoming {
                let from_name = graph.snapshot(edge.from).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str()));
                println!("    {:?} [{:?}]", from_name, edge.kind);
            }
            if incoming.is_empty() {
                println!("    (none found — forward is only called externally from outside this pipeline's extracted pool, e.g. PyTorch's own nn.Module.__call__ machinery, which this resolver cannot see)");
            }
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
    // Best-effort only, per the task's "non e' un benchmark finale"
    // instruction — uses `tasklist` rather than a new dependency, since
    // this harness is throwaway investigative tooling, not a permanent
    // feature.
    let pid = std::process::id();
    if let Ok(output) = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output() {
        let text = String::from_utf8_lossy(&output.stdout);
        println!("\n--- G. Performance observation (approx, Windows tasklist) ---");
        println!("{}", text.trim());
    }
}

#[cfg(not(windows))]
fn print_approx_memory() {}
