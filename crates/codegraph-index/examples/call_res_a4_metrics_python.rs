//! CALL-RES-A4, final item: real before/after corpus-level metrics for
//! the Python import-alias zero-candidate mechanism
//! (`resolve_calls_with_imports`) against the SAME real
//! `pyjanitor-devs/pyjanitor` corpus/commit CRC-003 and B2-EVAL-006
//! already used (`1247923ff01dd9a74f40f55d55f108e503e4b218`).
//!
//! "Before" = `resolve_calls_with_extraction_rejections` (shipped
//! pre-A4 entry point). "After" = `resolve_calls_with_imports`, same
//! snapshots/signature_too_long_names plus real `ImportFact`s (via
//! `resolve_import_facts`, same convention as the C# metrics harness
//! and `roslyn_differential_suite.rs`/production pipeline use).
//!
//! A4's own Python mechanism only ever acts on the
//! `candidates.is_empty()` branch (zero direct name matches) --
//! structurally disjoint from the C# namespace mechanism's `>=2`
//! branch. This harness measures that branch specifically: how many
//! real zero-candidate call sites on pyjanitor get a real alias-based
//! resolution, and whether it is Internal (recovers an edge),
//! External (stays honestly Unresolved with a more specific reason),
//! or Ambiguous (the real CRC-003 second-collision shape, recurring
//! at corpus scale rather than just the one documented call site).
//!
//! Precision/recall/F1 at corpus scale: same N/A reasoning as the C#
//! harness -- no ground-truth label set exists for pyjanitor's full
//! call graph; CRC-003 already owns the one hand-confirmed real target
//! this corpus has a ground-truth row for.
//!
//! Usage: `cargo run --release --example call_res_a4_metrics_python -- <repo_path> <commit>`

use std::collections::HashMap;

use codegraph_domain::{CommitHash, EvidenceTier, RepositorySource, ResolutionMethod};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::edges::{resolve_calls_with_extraction_rejections, resolve_calls_with_imports};
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::imports::resolve_import_facts;
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::negative_knowledge::UnresolvedReason;
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: call_res_a4_metrics_python <repo_path> <commit>");
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

    println!("=== CALL-RES-A4 metrics: Python import-alias mechanism, real pyjanitor-devs/pyjanitor corpus ===");
    println!("repo: {}", repo_path.display());
    println!("commit: {commit_str}");

    let files = source.list_files_at(repo_id, &commit).expect("list_files_at commit");

    let mut snapshots: Vec<codegraph_index::snapshot::SymbolSnapshot> = Vec::new();
    let mut signature_too_long_names: Vec<String> = Vec::new();
    let mut import_facts: Vec<codegraph_index::imports::ImportFact> = Vec::new();
    let mut files_parsed = 0usize;
    let mut import_fact_resolution_errors = 0usize;

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
        files_parsed += 1;

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

    // Second pass for ImportFact extraction, same two-pass convention
    // as call_res_a4_metrics_csharp.rs (resolve_import_facts needs the
    // FULL snapshot pool already resolved, per its own signature).
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
        match resolve_import_facts(&file, &tree, &snapshots) {
            Ok(mut facts) => import_facts.append(&mut facts),
            Err(_) => import_fact_resolution_errors += 1,
        }
    }

    println!("\n--- Corpus ---");
    println!("files_parsed: {files_parsed}");
    println!("snapshots_promoted: {}", snapshots.len());
    println!("real_import_facts_extracted: {}", import_facts.len());
    println!("import_fact_resolution_errors (non-fatal, excluded from import_facts): {import_fact_resolution_errors}");

    // --- BEFORE: shipped pre-A4 entry point ---
    let (before_edges, before_attempts) = resolve_calls_with_extraction_rejections(&snapshots, &signature_too_long_names).expect("resolve_calls_with_extraction_rejections");

    // --- AFTER: A4's own import-alias entry point ---
    let (after_edges, after_attempts) = resolve_calls_with_imports(&snapshots, &signature_too_long_names, &import_facts).expect("resolve_calls_with_imports");

    fn tier_counts(edges: &[codegraph_index::edges::Edge], attempts: &[codegraph_index::negative_knowledge::CallAttempt]) -> HashMap<&'static str, usize> {
        let mut counts: HashMap<&'static str, usize> = HashMap::new();
        for edge in edges {
            let tier = EvidenceTier::from_resolution_method(edge.evidence.method());
            let label = match tier {
                EvidenceTier::Resolved => "Resolved",
                EvidenceTier::Heuristic => "Heuristic",
                EvidenceTier::Ambiguous => "Ambiguous (unexpected on an Edge)",
                EvidenceTier::Unresolved => "Unresolved (unexpected on an Edge)",
            };
            *counts.entry(label).or_insert(0) += 1;
        }
        for attempt in attempts {
            let label = match attempt.tier {
                EvidenceTier::Resolved => "Resolved (unexpected on a CallAttempt)",
                EvidenceTier::Heuristic => "Heuristic (unexpected on a CallAttempt)",
                EvidenceTier::Ambiguous => "Ambiguous",
                EvidenceTier::Unresolved => "Unresolved",
            };
            *counts.entry(label).or_insert(0) += 1;
        }
        counts
    }

    let before_counts = tier_counts(&before_edges, &before_attempts);
    let after_counts = tier_counts(&after_edges, &after_attempts);

    println!("\n--- BEFORE (resolve_calls_with_extraction_rejections) ---");
    println!("edges_total: {}", before_edges.len());
    println!("call_attempts_total (ambiguous+unresolved, no edge produced): {}", before_attempts.len());
    for (label, count) in sorted(&before_counts) {
        println!("  {label}: {count}");
    }
    let before_static = before_edges.iter().filter(|e| e.evidence.method() == ResolutionMethod::StaticAnalysis).count();
    let before_heuristic = before_edges.iter().filter(|e| e.evidence.method() == ResolutionMethod::NameHeuristic).count();
    println!("  (by ResolutionMethod) StaticAnalysis: {before_static}, NameHeuristic: {before_heuristic}");

    println!("\n--- AFTER (resolve_calls_with_imports) ---");
    println!("edges_total: {}", after_edges.len());
    println!("call_attempts_total (ambiguous+unresolved, no edge produced): {}", after_attempts.len());
    for (label, count) in sorted(&after_counts) {
        println!("  {label}: {count}");
    }
    let after_static = after_edges.iter().filter(|e| e.evidence.method() == ResolutionMethod::StaticAnalysis).count();
    let after_heuristic = after_edges.iter().filter(|e| e.evidence.method() == ResolutionMethod::NameHeuristic).count();
    println!("  (by ResolutionMethod) StaticAnalysis: {after_static}, NameHeuristic: {after_heuristic}");

    println!("\n--- Delta (after - before) ---");
    println!("edges_total: {:+}", after_edges.len() as i64 - before_edges.len() as i64);
    println!("call_attempts_total: {:+}", after_attempts.len() as i64 - before_attempts.len() as i64);

    // --- Zero-candidate branch: A4's own only entry gate for Python.
    // `before_attempts` with tier == Unresolved AND candidates.is_empty()
    // are the real zero-candidate call sites A4 can possibly touch.
    //
    // IMPORTANT: a caller can invoke the same unresolved callee name
    // more than once in its own body -- `call_sites_in_tree` walks
    // every call expression, not deduplicated by name, and neither
    // `CallAttempt` nor `CallSite` exposes a per-occurrence span in
    // this crate's public API. A plain `HashMap<(caller,name), _>`
    // lookup would silently collapse N identical repeated calls into
    // 1 key and misclassify the other N-1. Counted as a MULTISET
    // instead (group by key, keep a Vec of each occurrence) so every
    // real occurrence is accounted for exactly once on each side.
    let mut before_zero_candidate: HashMap<(codegraph_domain::SymbolSnapshotId, String), usize> = HashMap::new();
    for a in before_attempts.iter().filter(|a| a.tier == EvidenceTier::Unresolved && a.candidates.is_empty()) {
        *before_zero_candidate.entry((a.caller, a.callee_name.clone())).or_insert(0) += 1;
    }
    let total_before_zero_candidate_occurrences: usize = before_zero_candidate.values().sum();

    let mut after_edges_by_caller_name: HashMap<(codegraph_domain::SymbolSnapshotId, String), usize> = HashMap::new();
    for edge in &after_edges {
        if let Some(target) = snapshots.iter().find(|s| s.id() == edge.to) {
            *after_edges_by_caller_name.entry((edge.from, target.name().as_str().to_string())).or_insert(0) += 1;
        }
    }
    let mut after_attempts_by_key: HashMap<(codegraph_domain::SymbolSnapshotId, String), Vec<&codegraph_index::negative_knowledge::CallAttempt>> = HashMap::new();
    for a in &after_attempts {
        after_attempts_by_key.entry((a.caller, a.callee_name.clone())).or_default().push(a);
    }

    let mut recovered_internal = 0usize;
    let mut import_target_external = 0usize;
    let mut imported_symbol_ambiguous = 0usize;
    let mut no_matching_import_alias = 0usize;
    let mut extraction_rejected = 0usize;
    let mut still_unknown_or_other = 0usize;

    for (key, before_occurrence_count) in &before_zero_candidate {
        // Per-key accounting: this caller/name key produced
        // `before_occurrence_count` occurrences pre-A4. Post-A4, the
        // SAME key's occurrences are split across
        // `after_edges_by_caller_name` (recovered) and
        // `after_attempts_by_key` (still unresolved/ambiguous) --
        // both counted by real reason, up to the same total, with any
        // remainder (if resolve_calls_with_imports produces a
        // DIFFERENT total occurrence count for this key than the
        // before stage -- which it should never do, since neither
        // resolver changes how many call expressions exist in the
        // source) flagged rather than silently dropped.
        let recovered_here = after_edges_by_caller_name.get(key).copied().unwrap_or(0).min(*before_occurrence_count);
        recovered_internal += recovered_here;
        let remaining = before_occurrence_count - recovered_here;
        if remaining == 0 {
            continue;
        }
        let after_reasons = after_attempts_by_key.get(key).cloned().unwrap_or_default();
        let mut taken = 0usize;
        for attempt in after_reasons.iter().take(remaining) {
            taken += 1;
            match &attempt.reason {
                UnresolvedReason::ImportTargetExternal => import_target_external += 1,
                UnresolvedReason::ImportedSymbolAmbiguous => imported_symbol_ambiguous += 1,
                UnresolvedReason::NoMatchingImportAlias => no_matching_import_alias += 1,
                // The resolver's own branch ORDER (calls.rs ~line 488)
                // checks `rejected_names` (a real SignatureTooLong/
                // NameTooLong extraction rejection for this exact
                // callee name) BEFORE ever calling
                // `resolve_via_import_alias` -- so a callee name that
                // is itself a rejected candidate never reaches A4's
                // own alias-matching logic at all, by design. Real,
                // not a defect: confirmed directly against this
                // corpus's own real SignatureTooLong names (e.g.
                // `conditional_join`, `get_join_indices`).
                UnresolvedReason::ExtractionRejected { .. } => extraction_rejected += 1,
                _ => still_unknown_or_other += 1,
            }
        }
        if taken < remaining {
            // Fewer after-side occurrences recorded for this key than
            // the before-side total minus recoveries -- reported
            // honestly as a real accounting gap rather than hidden.
            still_unknown_or_other += remaining - taken;
        }
    }

    println!("\n--- A4 import-alias effect on real pre-A4 zero-candidate call sites ---");
    println!("real_zero_candidate_call_sites_before_a4 (distinct caller+name keys): {}", before_zero_candidate.len());
    println!("real_zero_candidate_call_occurrences_before_a4 (multiset total, accounts for repeated identical calls from the same caller): {total_before_zero_candidate_occurrences}");
    println!("  recovered_to_a_real_edge (Internal alias target, now StaticAnalysis/NameHeuristic): {recovered_internal}");
    println!("  import_target_external (alias found, confirmed external, honestly Unresolved): {import_target_external}");
    println!("  imported_symbol_ambiguous (alias found, target name has 2+ real candidates, honestly Ambiguous -- the real CRC-003 second-collision shape): {imported_symbol_ambiguous}");
    println!("  no_matching_import_alias (no candidate, no alias match either): {no_matching_import_alias}");
    println!("  extraction_rejected (callee name is itself a real SignatureTooLong/NameTooLong rejection -- the resolver's own branch order checks this BEFORE the alias path, so A4 never gets a chance to act on these; real, not a defect): {extraction_rejected}");
    println!("  still_unknown_or_other (truly unexpected bucket -- reported honestly rather than hidden): {still_unknown_or_other}");
    println!("note: 'recovered_to_a_real_edge' call sites are real zero-candidate cases A4's own import-alias evidence resolved WITHOUT any type information -- an import alias is a name-binding fact (from import X as Y), not a type-inference result; this is the real-corpus count for the directive's own 'ambiguities resolved without type information' metric (here, more precisely, 'unresolved-by-name-alone call sites recovered without type information') on this corpus.");

    if before_zero_candidate.is_empty() {
        println!("N/A: zero real zero-candidate call sites existed on this corpus before A4.");
    }

    println!("\n--- Precision/Recall/F1 on this corpus ---");
    println!("N/A: no ground-truth label set exists for pyjanitor's full call graph beyond the one real CRC-003 call site already covered by ground_truth_schema.rs's own call-res-a4-crc-regression.jsonl rows. Building a separate >= 30-row labeled sample for this corpus was not done in this pass for the same reason given in call_res_a4_metrics_csharp.rs -- it would be a new ground-truth-labeling effort, not a before/after measurement, and an unlabeled proxy would hide the exact FP risk this metric exists to catch. TP/FP/FN are therefore also N/A at corpus scale here; real FP/FN risk for this corpus is covered by CRC-003's own ground-truth row plus the invariant check below.");

    // Re-verify the same NO-GO-style invariant as the C# harness: every
    // genuinely NEW after-edge's (caller, name) key must have been a
    // real pre-A4 zero-candidate site this run itself confirmed.
    let before_edge_keys: std::collections::HashSet<(codegraph_domain::SymbolSnapshotId, String)> = before_edges.iter().filter_map(|e| snapshots.iter().find(|s| s.id() == e.to).map(|t| (e.from, t.name().as_str().to_string()))).collect();
    let mut invariant_violations = 0usize;
    for edge in &after_edges {
        let Some(target) = snapshots.iter().find(|s| s.id() == edge.to) else { continue };
        let key = (edge.from, target.name().as_str().to_string());
        if before_edge_keys.contains(&key) {
            continue;
        }
        if !before_zero_candidate.contains_key(&key) {
            invariant_violations += 1;
            println!("  !!! unexpected: after-edge for caller/name key {key:?} was neither a pre-A4 edge nor a pre-A4 zero-candidate site");
        }
    }
    println!("invariant_check (every genuinely NEW after-edge's (caller,name) key was a real pre-A4 zero-candidate site, never manufactured): {invariant_violations} violations found");

    println!("\n--- Ambiguities resolved without type information (summary) ---");
    println!("real_count_on_pyjanitor: {recovered_internal} (same figure as 'recovered_to_a_real_edge' above)");
}

fn sorted(map: &HashMap<&'static str, usize>) -> Vec<(&'static str, usize)> {
    let mut v: Vec<_> = map.iter().map(|(k, v)| (*k, *v)).collect();
    v.sort_by_key(|(k, _)| *k);
    v
}
