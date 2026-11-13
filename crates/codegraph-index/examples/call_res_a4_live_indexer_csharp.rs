//! CALL-RES-A4, live-indexer re-measurement (C#): re-measures A4's
//! real-world effect by driving the ACTUAL, now-wired `CodeGraphIndexer`
//! end to end (`observe_commit`), not by calling A4's resolver function
//! directly in isolation the way `call_res_a4_metrics_csharp.rs` (commit
//! `ab0591d`) did. That prior harness measured A4's own resolvers
//! correctly, but at the time, the real indexer never called them at
//! all (`resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`
//! called the pre-A4 `resolve_calls_with_extraction_rejections`
//! unconditionally) — a gap closed by commit `ffd0295`. This harness
//! answers: now that the real indexer DOES call A4's own
//! `resolve_calls_with_namespace_context`, does the real end-to-end
//! pipeline reproduce the same ab0591d numbers (most importantly the
//! 366-case narrowing figure), or does something about real
//! `observe_commit` assembly (incremental reuse, publication, file
//! ordering) change the real count?
//!
//! "AFTER (live)" = the real `CodeGraphIndexer::observe_commit` at HEAD
//! (`ffd0295`+), read directly off `ObserveOutcome.graph`/`call_attempts`
//! — the actual wired path, no resolver called directly by this harness.
//!
//! "BEFORE (real-indexer-equivalent)" = `resolve_calls_with_extraction_rejections`
//! called on the EXACT SAME snapshot/signature_too_long_names pool this
//! harness itself extracts the same way the real pipeline does (same
//! convention `ab0591d`'s own harness used). This is mechanically
//! equivalent to what the real indexer did before `ffd0295`, since the
//! ONLY change `ffd0295` made at that call site was swapping which
//! resolver function is called on an otherwise-identical pool — confirmed
//! by reading `edges/mod.rs`'s pre/post-`ffd0295` diff directly. No
//! separate pre-`ffd0295` checkout is needed to reproduce this baseline
//! exactly, and none was performed.
//!
//! Usage: `cargo run --release --example call_res_a4_live_indexer_csharp -- <repo_path> <commit>`

use std::collections::HashMap;

use codegraph_domain::{CommitHash, EvidenceTier, ResolutionMethod, RepositorySource};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::edges::resolve_calls_with_extraction_rejections;
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::incremental::CodeGraphIndexer;
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::negative_knowledge::UnresolvedReason;
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: call_res_a4_live_indexer_csharp <repo_path> <commit>");
        std::process::exit(2);
    }
    let repo_path = std::fs::canonicalize(&args[1]).expect("repo_path must exist");
    let commit_str = &args[2];

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
    let commit = CommitHash::parse(commit_str).expect("commit must be a valid SHA-1");

    println!("=== CALL-RES-A4 live-indexer re-measurement: C# namespace-aware mechanism, real App-vNext/Polly corpus ===");
    println!("repo: {}", repo_path.display());
    println!("commit: {commit_str}");

    // --- AFTER (live): the real, now-wired CodeGraphIndexer ---
    let indexer = CodeGraphIndexer::new();
    let outcome = indexer.observe_commit(&source, repo_id, &commit, None, &[]).expect("observe_commit (full, live)");

    let live_edges: Vec<&codegraph_index::edges::Edge> = outcome.graph.all_edges().into_iter().filter(|e| e.kind == codegraph_domain::EdgeKind::Calls).collect();
    let live_attempts = &*outcome.call_attempts;

    println!("\n--- AFTER (live, real CodeGraphIndexer::observe_commit, HEAD) ---");
    println!("total_snapshots: {}", outcome.graph.snapshot_count());
    println!("total_edges_all_kinds: {}", outcome.graph.edge_count());
    println!("calls_edges: {}", live_edges.len());
    let live_static = live_edges.iter().filter(|e| e.evidence.method() == ResolutionMethod::StaticAnalysis).count();
    let live_heuristic = live_edges.iter().filter(|e| e.evidence.method() == ResolutionMethod::NameHeuristic).count();
    println!("  StaticAnalysis: {live_static}, NameHeuristic: {live_heuristic}");
    println!("call_attempts_total: {}", live_attempts.len());
    let live_ambiguous = live_attempts.iter().filter(|a| a.tier == EvidenceTier::Ambiguous).count();
    let live_unresolved = live_attempts.iter().filter(|a| a.tier == EvidenceTier::Unresolved).count();
    println!("  Ambiguous: {live_ambiguous}, Unresolved: {live_unresolved}");
    println!("real_import_facts_published: {}", outcome.import_facts.len());

    // --- BEFORE (real-indexer-equivalent): same pool, pre-A4 resolver ---
    // Re-extract the identical snapshot/signature_too_long_names pool
    // the SAME way ab0591d's own harness and the real pipeline both do,
    // so the "before" comparison is apples-to-apples against the SAME
    // real corpus content the live run above just indexed.
    let files = source.list_files_at(repo_id, &commit).expect("list_files_at commit");
    let mut snapshots: Vec<codegraph_index::snapshot::SymbolSnapshot> = Vec::new();
    let mut signature_too_long_names: Vec<String> = Vec::new();
    for relative_path in &files {
        let file = match codegraph_domain::FileIdentity::parse(repo_id, commit.clone(), relative_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let Some(parser) = parser_for(&file) else { continue };
        let bytes = match source.read_file(&file) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let tree = match parser.parse(&bytes) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
        for rejection in &rejections {
            if let codegraph_index::extract::CandidateRejection::SignatureTooLong { name, .. } = rejection {
                signature_too_long_names.push(name.clone());
            }
        }
        for candidate in candidates {
            if let Ok(snapshot) = promote(&candidate, file.clone()) {
                snapshots.push(snapshot);
            }
        }
    }

    let (before_edges, before_attempts) = resolve_calls_with_extraction_rejections(&snapshots, &signature_too_long_names).expect("resolve_calls_with_extraction_rejections (real-indexer-equivalent baseline)");

    println!("\n--- BEFORE (real-indexer-equivalent: resolve_calls_with_extraction_rejections on the SAME real pool) ---");
    println!("snapshots_for_baseline: {} (live indexer reports {} -- any difference is a real finding, checked below)", snapshots.len(), outcome.graph.snapshot_count());
    println!("calls_edges: {}", before_edges.len());
    let before_static = before_edges.iter().filter(|e| e.evidence.method() == ResolutionMethod::StaticAnalysis).count();
    let before_heuristic = before_edges.iter().filter(|e| e.evidence.method() == ResolutionMethod::NameHeuristic).count();
    println!("  StaticAnalysis: {before_static}, NameHeuristic: {before_heuristic}");
    println!("call_attempts_total: {}", before_attempts.len());
    let before_ambiguous_count = before_attempts.iter().filter(|a| a.tier == EvidenceTier::Ambiguous).count();
    let before_unresolved_count = before_attempts.iter().filter(|a| a.tier == EvidenceTier::Unresolved).count();
    println!("  Ambiguous: {before_ambiguous_count}, Unresolved: {before_unresolved_count}");

    if snapshots.len() != outcome.graph.snapshot_count() {
        println!("\n!!! FINDING: baseline snapshot count ({}) differs from live indexer snapshot count ({}) -- this harness's own re-extraction is NOT byte-identical to the real pipeline's own promotion/publication path. Delta reported honestly, not silently reconciled.", snapshots.len(), outcome.graph.snapshot_count());
    } else {
        println!("\nsnapshot_count_parity: baseline and live indexer agree exactly ({} snapshots) -- before/after comparison below is apples-to-apples.", snapshots.len());
    }

    println!("\n--- Delta (live after - real-indexer-equivalent before) ---");
    println!("calls_edges: {:+}", live_edges.len() as i64 - before_edges.len() as i64);
    println!("NameHeuristic: {:+}", live_heuristic as i64 - before_heuristic as i64);
    println!("call_attempts_total: {:+}", live_attempts.len() as i64 - before_attempts.len() as i64);
    println!("Ambiguous: {:+}", live_ambiguous as i64 - before_ambiguous_count as i64);

    // --- Narrowed-to-one count through the LIVE path, same (caller,name)
    // key convention ab0591d's own harness used, so the 366 figure is
    // directly comparable. ---
    let before_ambiguous: HashMap<(codegraph_domain::SymbolSnapshotId, String), usize> = before_attempts.iter().filter(|a| a.tier == EvidenceTier::Ambiguous).map(|a| ((a.caller, a.callee_name.clone()), a.candidates.len())).collect();
    let live_by_key: HashMap<(codegraph_domain::SymbolSnapshotId, String), (EvidenceTier, usize)> = live_attempts.iter().map(|a| ((a.caller, a.callee_name.clone()), (a.tier, a.candidates.len()))).collect();
    let live_edges_by_caller_name: HashMap<(codegraph_domain::SymbolSnapshotId, String), usize> = {
        let mut map = HashMap::new();
        for edge in &live_edges {
            if let Some(target) = outcome.graph.snapshot(edge.to) {
                *map.entry((edge.from, target.name().as_str().to_string())).or_insert(0) += 1;
            }
        }
        map
    };

    let mut narrowed_to_one = 0usize;
    let mut narrowed_but_still_ambiguous = 0usize;
    let mut unchanged_or_untouched = 0usize;
    for (key, _before_count) in &before_ambiguous {
        if live_edges_by_caller_name.contains_key(key) {
            narrowed_to_one += 1;
        } else if let Some((EvidenceTier::Ambiguous, after_count)) = live_by_key.get(key) {
            if *after_count < *before_ambiguous.get(key).unwrap() {
                narrowed_but_still_ambiguous += 1;
            } else {
                unchanged_or_untouched += 1;
            }
        } else {
            unchanged_or_untouched += 1;
        }
    }

    println!("\n--- A4 namespace-filtering effect, measured through the LIVE wired indexer ---");
    println!("real_ambiguous_call_sites_before_a4 (baseline): {}", before_ambiguous.len());
    println!("  narrowed_to_exactly_one_candidate_by_a4 (LIVE, now a real Edge via observe_commit): {narrowed_to_one}");
    println!("  narrowed_but_still_2_plus_candidates: {narrowed_but_still_ambiguous}");
    println!("  unchanged_or_untouched: {unchanged_or_untouched}");
    println!("comparison_to_ab0591d_resolver_level_figure: ab0591d reported 366 narrowed-to-one on this same corpus/commit via the DIRECT resolver call; this run reports {narrowed_to_one} via the LIVE wired indexer -- {}", if narrowed_to_one == 366 { "EXACT MATCH, confirms resolver-level and live-indexer figures agree for this corpus.".to_string() } else { format!("DIFFERS by {} -- see finding below, not silently reconciled.", narrowed_to_one as i64 - 366) });

    // --- Invariant re-check on live data: never a manufactured target ---
    let before_edge_keys: std::collections::HashSet<(codegraph_domain::SymbolSnapshotId, String)> = before_edges.iter().filter_map(|e| snapshots.iter().find(|s| s.id() == e.to).map(|t| (e.from, t.name().as_str().to_string()))).collect();
    let mut invariant_violations = 0usize;
    for edge in &live_edges {
        let Some(target) = outcome.graph.snapshot(edge.to) else { continue };
        let key = (edge.from, target.name().as_str().to_string());
        if before_edge_keys.contains(&key) {
            continue;
        }
        if !before_ambiguous.contains_key(&key) {
            invariant_violations += 1;
        }
    }
    println!("invariant_check (every genuinely new live edge's (caller,name) key was a real pre-A4 ambiguous candidate-pool member, never manufactured): {invariant_violations} violations found");

    println!("\n--- Precision/Recall/F1/TP/FP/FN on this corpus, live-indexer ---");
    println!("N/A, same reasoning as ab0591d: no ground-truth label set exists for Polly's full call graph beyond CRC-001/002/003's own rows. FP risk is covered by the invariant check above (0 violations means 0 manufactured targets); FN is not separately quantifiable without a labeled sample.");

    let import_target_ambiguous_attempts = live_attempts.iter().filter(|a| a.reason == UnresolvedReason::ImportedSymbolAmbiguous).count();
    println!("\nimport_alias_ambiguous_attempts_live (A4 found evidence but target still has 2+ real candidates, honestly Ambiguous): {import_target_ambiguous_attempts}");
}
