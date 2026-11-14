//! B2-EVAL-011 full/incremental multiset-equivalence check for the
//! HTML/CSS corpus (tbreuss/html-boilerplate), reusing the existing,
//! already-tested `conformance::compare_full_incremental_state` gate
//! directly (never a custom ad hoc comparison) against two real
//! `CodeGraphIndexer::observe_commit` outcomes on a real commit pair
//! that changes BOTH an HTML file and a CSS file
//! (`index.html` + `static/css/styles.css`).
//!
//! Usage: `cargo run --release --example b2_eval_011_html_css_parity -- <repo_path> <commit_a> <commit_b>`

use codegraph_domain::CommitHash;
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::conformance::compare_full_incremental_state;
use codegraph_index::incremental::CodeGraphIndexer;
use codegraph_index::local_git_source::LocalGitRepositorySource;

fn git_diff_name_only(repo_path: &std::path::Path, commit_a: &str, commit_b: &str) -> Vec<String> {
    let output = std::process::Command::new("git").arg("diff").arg("--name-only").arg(commit_a).arg(commit_b).current_dir(repo_path).output().expect("git diff must run");
    String::from_utf8_lossy(&output.stdout).lines().map(|s| s.to_string()).filter(|s| !s.is_empty()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: b2_eval_011_html_css_parity <repo_path> <commit_a> <commit_b>");
        std::process::exit(2);
    }
    let repo_path = std::fs::canonicalize(&args[1]).expect("repo_path must exist");
    let commit_a_str = &args[2];
    let commit_b_str = &args[3];

    let commit_a = CommitHash::parse(commit_a_str).expect("commit_a must be a valid SHA-1");
    let commit_b = CommitHash::parse(commit_b_str).expect("commit_b must be a valid SHA-1");
    let changed_files = git_diff_name_only(&repo_path, commit_a_str, commit_b_str);
    println!("changed_files (git diff A..B): {} -> {:?}", changed_files.len(), changed_files);

    let mut allowlist = RepositoryAllowlist::new();
    let allowlisted_root = allowlist.authorize(&repo_path).expect("authorize repo root");
    let mut registry = codegraph_domain::RepositoryRegistry::new();
    let repo_root = codegraph_domain::RepositoryRoot::parse(&repo_path).unwrap();
    let repo_id = registry.register(repo_root);
    let root_for_id = move |id: codegraph_domain::RepositoryId| -> Option<codegraph_index::allowlist::AllowlistedRoot> {
        if id == repo_id { Some(allowlisted_root.clone()) } else { None }
    };
    let source = LocalGitRepositorySource::new(&allowlist, &root_for_id);

    // Run 1 ("incremental"): full reindex of commit_a, then INCREMENTAL
    // advance to commit_b.
    let indexer_incremental_path = CodeGraphIndexer::new();
    let _ = indexer_incremental_path.observe_commit(&source, repo_id, &commit_a, None, &[]).expect("full reindex of commit_a");
    let incremental_outcome = indexer_incremental_path.observe_commit(&source, repo_id, &commit_b, Some(&commit_a), &changed_files).expect("incremental advance to commit_b");

    // Run 2 ("full"): a wholly FRESH indexer, full reindex DIRECTLY of commit_b.
    let indexer_full_path = CodeGraphIndexer::new();
    let full_outcome = indexer_full_path.observe_commit(&source, repo_id, &commit_b, None, &[]).expect("full reindex of commit_b");

    println!("full_outcome: snapshots={} edges={} import_facts={} impl_facts={} call_attempts={}", full_outcome.graph.snapshot_count(), full_outcome.graph.edge_count(), full_outcome.import_facts.len(), full_outcome.impl_facts.len(), full_outcome.call_attempts.len());
    println!("incremental_outcome: snapshots={} edges={} import_facts={} impl_facts={} call_attempts={}", incremental_outcome.graph.snapshot_count(), incremental_outcome.graph.edge_count(), incremental_outcome.import_facts.len(), incremental_outcome.impl_facts.len(), incremental_outcome.call_attempts.len());

    let divergences = compare_full_incremental_state(&full_outcome, &incremental_outcome);
    if divergences.is_empty() {
        println!("\nRESULT: PARITY CONFIRMED -- zero divergences across snapshots/edges/import_facts/impl_facts/call_attempts multisets (compare_full_incremental_state, the same gate the lib test suite's own gate_3_full_equals_incremental_exact_state uses).");
    } else {
        println!("\nRESULT: {} DIVERGENCE(S) FOUND:", divergences.len());
        for d in &divergences {
            println!("  {d:?}");
        }
    }
}
