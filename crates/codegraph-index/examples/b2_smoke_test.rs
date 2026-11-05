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
