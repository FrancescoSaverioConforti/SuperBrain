//! CALL-RES-A4 performance measurement: real-indexer full and
//! incremental wall-clock cost, baseline vs A4-live, via repeated
//! timed runs of the SAME `CodeGraphIndexer::observe_commit` entry
//! point this whole evaluation series already uses (same pattern as
//! `b2_smoke_test_csharp_cpp.rs`'s own Section F and B2-ID-012's own
//! measurement protocol: multiple runs, one discarded warm-up, median
//! + dispersion reported, same machine, same commit pair).
//!
//! This binary is built and run TWICE, once per git ref, by the
//! driving script: once at a pre-wiring commit (where
//! `resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`
//! still calls the pre-A4 `resolve_calls_with_extraction_rejections`)
//! for the BASELINE numbers, and once at current HEAD (A4 wired in,
//! commit `ffd0295`+) for the A4-LIVE numbers. The binary's own code
//! is identical in both cases -- it only calls the real, public
//! `CodeGraphIndexer::observe_commit`, never a resolver function
//! directly -- so any difference between the two runs' own timings is
//! attributable to whatever changed inside the real indexer between
//! those two commits, which per `ffd0295`'s own diff is exactly the
//! import_facts-assembly-timing-and-resolver-call change this
//! measurement exists to isolate, and nothing else.
//!
//! Usage: `cargo run --release --example call_res_a4_perf_measurement -- <repo_path> <commit_a> <commit_b> <runs>`
//! `commit_a` is indexed as a full reindex every run (freshly
//! constructed `CodeGraphIndexer` each time, so there is no
//! cross-run incremental state leakage). `commit_b` is then indexed
//! incrementally against `commit_a`'s own state, every run, using the
//! real `git diff --name-only` changed-file list between them (same
//! convention `b2_smoke_test_csharp_cpp.rs` uses).

use std::path::Path;
use std::time::{Duration, Instant};

use codegraph_domain::CommitHash;
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::incremental::CodeGraphIndexer;
use codegraph_index::local_git_source::LocalGitRepositorySource;

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

fn median(durations: &mut [Duration]) -> Duration {
    durations.sort();
    let mid = durations.len() / 2;
    if durations.len() % 2 == 0 {
        (durations[mid - 1] + durations[mid]) / 2
    } else {
        durations[mid]
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        eprintln!("usage: call_res_a4_perf_measurement <repo_path> <commit_a> <commit_b> <runs>");
        std::process::exit(2);
    }
    let repo_path = std::fs::canonicalize(&args[1]).expect("repo_path must exist");
    let commit_a_str = &args[2];
    let commit_b_str = &args[3];
    let runs: usize = args[4].parse().expect("runs must be a positive integer");
    assert!(runs >= 1, "runs must be >= 1");

    println!("=== CALL-RES-A4 performance measurement ===");
    println!("repo: {}", repo_path.display());
    println!("commit_a (full-index target): {commit_a_str}");
    println!("commit_b (incremental target): {commit_b_str}");
    println!("timed_runs: {runs} (plus 1 discarded warm-up run)");

    let commit_a = CommitHash::parse(commit_a_str).expect("commit_a must be a valid SHA-1");
    let commit_b = CommitHash::parse(commit_b_str).expect("commit_b must be a valid SHA-1");
    let changed_files = git_diff_name_only(&repo_path, commit_a_str, commit_b_str);
    println!("changed_files (git diff A..B): {} files", changed_files.len());

    let run_once = |repo_path: &Path| -> (Duration, Duration, usize, usize) {
        let mut allowlist = RepositoryAllowlist::new();
        let allowlisted_root = allowlist.authorize(repo_path).expect("authorize repo root");
        let mut registry = codegraph_domain::RepositoryRegistry::new();
        let repo_root = codegraph_domain::RepositoryRoot::parse(repo_path).unwrap();
        let repo_id = registry.register(repo_root);
        let root_for_id = move |id: codegraph_domain::RepositoryId| -> Option<codegraph_index::allowlist::AllowlistedRoot> {
            if id == repo_id { Some(allowlisted_root.clone()) } else { None }
        };
        let source = LocalGitRepositorySource::new(&allowlist, &root_for_id);
        let indexer = CodeGraphIndexer::new();

        let t_full = Instant::now();
        let outcome_a = indexer.observe_commit(&source, repo_id, &commit_a, None, &[]).expect("observe_commit A (full)");
        let full_elapsed = t_full.elapsed();
        let edge_count_a = outcome_a.graph.edge_count();

        let t_incremental = Instant::now();
        let outcome_b = indexer.observe_commit(&source, repo_id, &commit_b, Some(&commit_a), &changed_files).expect("observe_commit B (incremental)");
        let incremental_elapsed = t_incremental.elapsed();
        let edge_count_b = outcome_b.graph.edge_count();

        (full_elapsed, incremental_elapsed, edge_count_a, edge_count_b)
    };

    println!("\n--- warm-up run (discarded) ---");
    let (warm_full, warm_incremental, warm_edges_a, warm_edges_b) = run_once(&repo_path);
    println!("warm-up full: {warm_full:?}, incremental: {warm_incremental:?}, edges_a: {warm_edges_a}, edges_b: {warm_edges_b}");

    let mut full_times: Vec<Duration> = Vec::with_capacity(runs);
    let mut incremental_times: Vec<Duration> = Vec::with_capacity(runs);
    let mut edge_counts_a: Vec<usize> = Vec::with_capacity(runs);
    let mut edge_counts_b: Vec<usize> = Vec::with_capacity(runs);

    println!("\n--- {runs} timed runs ---");
    for i in 0..runs {
        let (full, incremental, edges_a, edges_b) = run_once(&repo_path);
        println!("run {i}: full={full:?} incremental={incremental:?} edges_a={edges_a} edges_b={edges_b}");
        full_times.push(full);
        incremental_times.push(incremental);
        edge_counts_a.push(edges_a);
        edge_counts_b.push(edges_b);
    }

    let full_min = *full_times.iter().min().unwrap();
    let full_max = *full_times.iter().max().unwrap();
    let full_median = median(&mut full_times.clone());
    let incremental_min = *incremental_times.iter().min().unwrap();
    let incremental_max = *incremental_times.iter().max().unwrap();
    let incremental_median = median(&mut incremental_times.clone());

    println!("\n--- Summary ---");
    println!("full_index: median={full_median:?} min={full_min:?} max={full_max:?} (range={:?})", full_max.saturating_sub(full_min));
    println!("incremental_index: median={incremental_median:?} min={incremental_min:?} max={incremental_max:?} (range={:?})", incremental_max.saturating_sub(incremental_min));
    println!("edge_count_a (full, all runs): {edge_counts_a:?}");
    println!("edge_count_b (incremental, all runs): {edge_counts_b:?}");
    let all_edges_a_equal = edge_counts_a.iter().all(|c| *c == edge_counts_a[0]);
    let all_edges_b_equal = edge_counts_b.iter().all(|c| *c == edge_counts_b[0]);
    println!("edge_count_stability: full={all_edges_a_equal}, incremental={all_edges_b_equal} (true means every run produced identical output, confirming timing variance is not masking a correctness difference)");
}
