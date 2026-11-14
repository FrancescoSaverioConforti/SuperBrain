//! CALL-RES-A4, live-indexer re-measurement (Python): re-measures A4's
//! real-world effect by driving the ACTUAL, now-wired `CodeGraphIndexer`
//! end to end (`observe_commit`), not by calling A4's resolver function
//! directly in isolation the way `call_res_a4_metrics_python.rs`
//! (commit `ab0591d`) did. See `call_res_a4_live_indexer_csharp.rs`'s
//! own doc comment for the full rationale — identical here, applied to
//! pyjanitor and the Python import-alias (zero-candidate) mechanism.
//! `resolve_calls_with_namespace_context` (the function the real
//! indexer now calls per commit `ffd0295`) falls through to
//! `resolve_calls_with_imports` byte-for-byte whenever `csharp_contexts`
//! is empty (confirmed directly in `calls.rs`), which it always will be
//! for a pure-Python repo — so the live indexer's real behavior on this
//! corpus IS the Python alias mechanism, unchanged.
//!
//! Usage: `cargo run --release --example call_res_a4_live_indexer_python -- <repo_path> <commit>`

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
        eprintln!("usage: call_res_a4_live_indexer_python <repo_path> <commit>");
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

    println!("=== CALL-RES-A4 live-indexer re-measurement: Python import-alias mechanism, real pyjanitor-devs/pyjanitor corpus ===");
    println!("repo: {}", repo_path.display());
    println!("commit: {commit_str}");

    // --- AFTER (live): the real, now-wired CodeGraphIndexer ---
    let indexer = CodeGraphIndexer::new();
    let outcome = indexer.observe_commit(&source, repo_id, &commit, None, &[]).expect("observe_commit (full, live)");

    let live_edges: Vec<&codegraph_index::edges::Edge> = outcome.graph.all_edges().into_iter().filter(|e| e.kind == codegraph_domain::EdgeKind::Calls).collect();
    let live_attempts = &*outcome.call_attempts;

    println!("\n--- AFTER (live, real CodeGraphIndexer::observe_commit, HEAD) ---");
    println!("total_snapshots: {}", outcome.graph.snapshot_count());
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
    println!("call_attempts_total: {:+}", live_attempts.len() as i64 - before_attempts.len() as i64);
    println!("Ambiguous: {:+}", live_ambiguous as i64 - before_ambiguous_count as i64);
    println!("Unresolved: {:+}", live_unresolved as i64 - before_unresolved_count as i64);

    // --- Zero-candidate branch recovery, measured through the LIVE path,
    // same multiset-by-key convention ab0591d's own harness used, so the
    // "0 corpus-level recoveries" figure is directly comparable. ---
    let mut before_zero_candidate: HashMap<(codegraph_domain::SymbolSnapshotId, String), usize> = HashMap::new();
    for a in before_attempts.iter().filter(|a| a.tier == EvidenceTier::Unresolved && a.candidates.is_empty()) {
        *before_zero_candidate.entry((a.caller, a.callee_name.clone())).or_insert(0) += 1;
    }
    let total_before_zero_candidate_occurrences: usize = before_zero_candidate.values().sum();

    let mut live_edges_by_caller_name: HashMap<(codegraph_domain::SymbolSnapshotId, String), usize> = HashMap::new();
    for edge in &live_edges {
        if let Some(target) = outcome.graph.snapshot(edge.to) {
            *live_edges_by_caller_name.entry((edge.from, target.name().as_str().to_string())).or_insert(0) += 1;
        }
    }
    let mut live_attempts_by_key: HashMap<(codegraph_domain::SymbolSnapshotId, String), Vec<&codegraph_index::negative_knowledge::CallAttempt>> = HashMap::new();
    for a in live_attempts.iter() {
        live_attempts_by_key.entry((a.caller, a.callee_name.clone())).or_default().push(a);
    }

    let mut recovered_internal = 0usize;
    let mut import_target_external = 0usize;
    let mut imported_symbol_ambiguous = 0usize;
    let mut no_matching_import_alias = 0usize;
    let mut extraction_rejected = 0usize;
    let mut still_unknown_or_other = 0usize;

    for (key, before_occurrence_count) in &before_zero_candidate {
        let recovered_here = live_edges_by_caller_name.get(key).copied().unwrap_or(0).min(*before_occurrence_count);
        recovered_internal += recovered_here;
        let remaining = before_occurrence_count - recovered_here;
        if remaining == 0 {
            continue;
        }
        let live_reasons = live_attempts_by_key.get(key).cloned().unwrap_or_default();
        let mut taken = 0usize;
        for attempt in live_reasons.iter().take(remaining) {
            taken += 1;
            match &attempt.reason {
                UnresolvedReason::ImportTargetExternal => import_target_external += 1,
                UnresolvedReason::ImportedSymbolAmbiguous => imported_symbol_ambiguous += 1,
                UnresolvedReason::NoMatchingImportAlias => no_matching_import_alias += 1,
                UnresolvedReason::ExtractionRejected { .. } => extraction_rejected += 1,
                _ => still_unknown_or_other += 1,
            }
        }
        if taken < remaining {
            still_unknown_or_other += remaining - taken;
        }
    }

    println!("\n--- A4 import-alias effect on real pre-A4 zero-candidate call sites, measured through the LIVE wired indexer ---");
    println!("real_zero_candidate_call_sites_before_a4 (distinct caller+name keys): {}", before_zero_candidate.len());
    println!("real_zero_candidate_call_occurrences_before_a4 (multiset total): {total_before_zero_candidate_occurrences}");
    println!("  recovered_to_a_real_edge (LIVE, via observe_commit): {recovered_internal}");
    println!("  import_target_external: {import_target_external}");
    println!("  imported_symbol_ambiguous (real CRC-003 second-collision shape, recurring at corpus scale): {imported_symbol_ambiguous}");
    println!("  no_matching_import_alias: {no_matching_import_alias}");
    println!("  extraction_rejected (pre-existing SignatureTooLong, resolver branch order never routes these through the alias path): {extraction_rejected}");
    println!("  still_unknown_or_other: {still_unknown_or_other}");
    println!("comparison_to_ab0591d_resolver_level_figure: ab0591d reported 0 corpus-level recoveries on this same corpus/commit via the DIRECT resolver call; this run reports {recovered_internal} via the LIVE wired indexer -- {}", if recovered_internal == 0 { "EXACT MATCH, confirms resolver-level and live-indexer figures agree for this corpus.".to_string() } else { format!("DIFFERS by {recovered_internal} -- see finding below, not silently reconciled.") });

    if before_zero_candidate.is_empty() {
        println!("N/A: zero real zero-candidate call sites existed on this corpus before A4.");
    }

    println!("\n--- Precision/Recall/F1/TP/FP/FN on this corpus, live-indexer ---");
    println!("N/A, same reasoning as ab0591d: no ground-truth label set exists for pyjanitor's full call graph beyond CRC-003's own already-retrofitted row. FP risk is covered by the invariant check below; FN is not separately quantifiable without a labeled sample.");

    let before_edge_keys: std::collections::HashSet<(codegraph_domain::SymbolSnapshotId, String)> = before_edges.iter().filter_map(|e| snapshots.iter().find(|s| s.id() == e.to).map(|t| (e.from, t.name().as_str().to_string()))).collect();
    let mut invariant_violations = 0usize;
    for edge in &live_edges {
        let Some(target) = outcome.graph.snapshot(edge.to) else { continue };
        let key = (edge.from, target.name().as_str().to_string());
        if before_edge_keys.contains(&key) {
            continue;
        }
        if !before_zero_candidate.contains_key(&key) {
            invariant_violations += 1;
            println!("  !!! unexpected: live edge for caller/name key {key:?} was neither a pre-A4 edge nor a pre-A4 zero-candidate site");
        }
    }
    println!("invariant_check (every genuinely NEW live edge's (caller,name) key was a real pre-A4 zero-candidate site, never manufactured): {invariant_violations} violations found");

    println!("\n--- Ambiguities resolved without type information (summary) ---");
    println!("real_count_on_pyjanitor_live: {recovered_internal} (same figure as 'recovered_to_a_real_edge' above)");
}
