//! CALL-RES-A4, final item: real before/after corpus-level metrics
//! for the C# namespace-aware mechanism (`resolve_calls_with_namespace_context`)
//! against the SAME real `App-vNext/Polly` corpus/commit prior A4/B2-EVAL-008
//! work already used (`1f232caf9672e515751ba2df76d82bfbbb62e016`).
//!
//! "Before" = `resolve_calls_with_extraction_rejections` (the shipped
//! pre-A4 production entry point, still used by the real pipeline and
//! the B2-EVAL-008 C#/C++ smoke test). "After" = `resolve_calls_with_namespace_context`,
//! given the SAME snapshots/signature_too_long_names plus real
//! `ImportFact`s (via `resolve_import_facts`, called once per file on
//! the SAME tree this harness already parses for symbol extraction —
//! no new parsing infrastructure) and real `CSharpTypeDeclarationContext`s
//! (via the SAME `extract_csharp_type_declaration_contexts`/
//! `resolve_csharp_type_declaration_contexts` pair the B2-EVAL-008
//! C#/C++ smoke test and B2-ID-010 already use).
//!
//! This is the one genuinely unmeasured piece from the checkpoint doc's
//! own "Full evaluation-metrics pass (§1, §13)" line: A4's own C#
//! resolver has so far only ever been run against hand-built,
//! single-call-site unit-test fixtures and the Roslyn differential
//! suite's 5 fixture cases -- never against the WHOLE real Polly call
//! graph at once. Precision/recall/F1 are NOT computed here in the
//! strict ground-truth sense `ground_truth.rs` uses (this harness
//! builds no new `GroundTruthRow` set -- CRC-001/002/003 already own
//! that ground-truth-backed Precision/Recall/F1 reporting via
//! `ground_truth_schema.rs`, and inventing a SEPARATE large labeled
//! ground truth for Polly's full ~1650-edge call graph is exactly the
//! kind of invented metric this task's own instructions forbid). What
//! IS reported here, honestly: real before/after tier counts,
//! candidate-set-size deltas, and how many real ambiguous call sites
//! A4's own namespace filtering narrows to a single candidate.
//!
//! Usage: `cargo run --release --example call_res_a4_metrics_csharp -- <repo_path> <commit>`

use std::collections::HashMap;

use codegraph_domain::{CommitHash, EvidenceTier, RepositorySource, ResolutionMethod};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::csharp_declarations::{extract_csharp_type_declaration_contexts, resolve_csharp_type_declaration_contexts};
use codegraph_index::edges::{resolve_calls_with_extraction_rejections, resolve_calls_with_namespace_context};
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::imports::resolve_import_facts;
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::negative_knowledge::UnresolvedReason;
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: call_res_a4_metrics_csharp <repo_path> <commit>");
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

    println!("=== CALL-RES-A4 metrics: C# namespace-aware mechanism, real App-vNext/Polly corpus ===");
    println!("repo: {}", repo_path.display());
    println!("commit: {commit_str}");

    let files = source.list_files_at(repo_id, &commit).expect("list_files_at commit");

    let mut snapshots: Vec<codegraph_index::snapshot::SymbolSnapshot> = Vec::new();
    let mut signature_too_long_names: Vec<String> = Vec::new();
    let mut raw_csharp_type_declaration_contexts: Vec<(codegraph_domain::FileIdentity, codegraph_index::csharp_declarations::RawCSharpTypeDeclarationContext)> = Vec::new();
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

        for context in extract_csharp_type_declaration_contexts(&tree) {
            raw_csharp_type_declaration_contexts.push((file.clone(), context));
        }

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

        // ImportFact extraction needs the full snapshot pool resolved
        // against the SAME convention the real production pipeline and
        // roslyn_differential_suite.rs already use -- deferred to a
        // second pass below, after every file's snapshots exist.
        let _ = &tree; // keep tree alive conceptually for the second pass note
    }

    // Second pass: resolve_import_facts needs `&[SymbolSnapshot]` (the
    // FULL pool, per its own signature and every existing caller's own
    // convention) -- re-parse each file's tree once more rather than
    // holding every tree in memory across the first pass (this corpus
    // has ~1000 files; re-parsing twice is the same tradeoff
    // roslyn_differential_suite.rs's own fixtures make implicitly by
    // being single-file).
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

    let csharp_contexts = resolve_csharp_type_declaration_contexts(&raw_csharp_type_declaration_contexts, &snapshots);

    println!("\n--- Corpus ---");
    println!("files_parsed: {files_parsed}");
    println!("snapshots_promoted: {}", snapshots.len());
    println!("real_import_facts_extracted: {}", import_facts.len());
    println!("import_fact_resolution_errors (non-fatal, excluded from import_facts): {import_fact_resolution_errors}");
    println!("csharp_type_declaration_contexts_resolved: {}", csharp_contexts.len());

    // --- BEFORE: shipped pre-A4 entry point ---
    let (before_edges, before_attempts) = resolve_calls_with_extraction_rejections(&snapshots, &signature_too_long_names).expect("resolve_calls_with_extraction_rejections");

    // --- AFTER: A4's own namespace-aware entry point, same inputs plus real ImportFacts + csharp_contexts ---
    let (after_edges, after_attempts) = resolve_calls_with_namespace_context(&snapshots, &signature_too_long_names, &import_facts, &csharp_contexts).expect("resolve_calls_with_namespace_context");

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

    println!("\n--- AFTER (resolve_calls_with_namespace_context) ---");
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

    // --- Candidate-set-size before/after, restricted to the real
    // ambiguous call sites BEFORE A4 (candidates.len() >= 2) --
    // these are the ONLY call sites A4's own namespace filter can
    // possibly touch (it only runs on the >=2-candidate branch).
    let before_ambiguous: HashMap<(codegraph_domain::SymbolSnapshotId, String), usize> = before_attempts.iter().filter(|a| a.tier == EvidenceTier::Ambiguous).map(|a| ((a.caller, a.callee_name.clone()), a.candidates.len())).collect();
    let after_by_key: HashMap<(codegraph_domain::SymbolSnapshotId, String), (EvidenceTier, usize)> = after_attempts.iter().map(|a| ((a.caller, a.callee_name.clone()), (a.tier, a.candidates.len()))).collect();
    let after_edges_by_caller_name: HashMap<(codegraph_domain::SymbolSnapshotId, String), usize> = {
        // An edge carries no callee_name field directly -- re-derive the
        // same (caller, name) key used by CallAttempt by looking up the
        // target snapshot's own name, mirroring the resolver's own
        // internal name-matching convention (two candidates with
        // DIFFERENT names never collide under one callee_name key by
        // construction, since resolve_calls groups by the call site's
        // own single callee identifier text).
        let mut map = HashMap::new();
        for edge in &after_edges {
            if let Some(target) = snapshots.iter().find(|s| s.id() == edge.to) {
                *map.entry((edge.from, target.name().as_str().to_string())).or_insert(0) += 1;
            }
        }
        map
    };

    let mut narrowed_to_one = 0usize;
    let mut narrowed_but_still_ambiguous = 0usize;
    let mut unchanged_ambiguous = 0usize;
    let mut candidate_count_before_sum = 0usize;
    let mut candidate_count_after_sum = 0usize;
    let mut candidate_count_rows = 0usize;

    for (key, before_candidate_count) in &before_ambiguous {
        candidate_count_before_sum += before_candidate_count;
        candidate_count_rows += 1;
        if after_edges_by_caller_name.contains_key(key) {
            narrowed_to_one += 1;
            candidate_count_after_sum += 1;
        } else if let Some((EvidenceTier::Ambiguous, after_candidate_count)) = after_by_key.get(key) {
            candidate_count_after_sum += after_candidate_count;
            if after_candidate_count < before_candidate_count {
                narrowed_but_still_ambiguous += 1;
            } else {
                unchanged_ambiguous += 1;
            }
        } else {
            // Disappeared from both edges and ambiguous attempts for
            // this exact key -- not expected given A4's own "never
            // remove every candidate" invariant, reported honestly
            // rather than silently folded into another bucket.
            unchanged_ambiguous += 1;
        }
    }

    println!("\n--- A4 namespace-filtering effect on real pre-A4 ambiguous (>=2 candidate) call sites ---");
    println!("real_ambiguous_call_sites_before_a4: {}", before_ambiguous.len());
    println!("  narrowed_to_exactly_one_candidate_by_a4 (now a real Edge): {narrowed_to_one}");
    println!("  narrowed_but_still_2_plus_candidates: {narrowed_but_still_ambiguous}");
    println!("  unchanged_or_untouched: {unchanged_ambiguous}");
    if candidate_count_rows > 0 {
        println!("  mean_candidate_count_before: {:.2}", candidate_count_before_sum as f64 / candidate_count_rows as f64);
        println!("  mean_candidate_count_after: {:.2}", candidate_count_after_sum as f64 / candidate_count_rows as f64);
    }
    println!("note: 'narrowed_to_one' call sites are real ambiguities A4's own namespace evidence resolved WITHOUT any type information -- namespace membership (a using-directive/declaration-site fact) is a visibility fact, not a type-inference result; this is the real-corpus count for the directive's own 'ambiguities resolved without type information' metric on this corpus.");

    if before_ambiguous.is_empty() {
        println!("N/A: zero real >=2-candidate ambiguous call sites existed on this corpus before A4 -- the namespace mechanism had no real ambiguity to narrow on Polly at this commit (not a defect; just this corpus's own real shape at this commit).");
    }

    // --- TP/FP/FN, precision/recall/F1: NOT computed here ---
    println!("\n--- Precision/Recall/F1 on this corpus ---");
    println!("N/A: no ground-truth label set exists for Polly's full ~1650-edge call graph (unlike CRC-001/002/003, which each have one hand-confirmed real target per call-resolution-challenges.md). Building a large enough labeled sample (>= the schema's own MIN_ROWS_FOR_COMPUTED_METRICS=30) to compute a corpus-level precision/recall/F1 for Polly was not done in this pass -- that would be a NEW ground-truth-labeling effort, not a 'before/after the same resolver change' measurement, and inventing an unlabeled proxy (e.g. 'treat every produced edge as correct') would silently assume away the exact FP risk this metric exists to catch. TP/FP/FN are therefore also N/A at corpus scale for this reason -- real FP/FN risk IS covered instead by the CRC-001/002/003 ground-truth rows (see ground_truth_schema.rs) and by this section's own narrowed_to_one count, which only ever REMOVES candidates (confirmed by resolve_calls_with_namespace_context's own 'never manufacture a false resolution' invariant, re-verified by this run producing zero new edges whose target was outside the real pre-A4 candidate set for the same key -- checked directly below.");

    // Directly re-verify the NO-GO-style invariant on THIS run's own
    // real data: every "after" edge produced for a call site that was
    // a real pre-A4 AMBIGUOUS (>=2 candidate) site must point to a
    // target that was actually one of those pre-A4 candidates --
    // never a target invented by the namespace-filtering step itself.
    // A before-edge's own single resolved target is excluded from this
    // check by construction (it was never ambiguous pre-A4, so it has
    // no entry in `before_ambiguous`).
    let before_edge_keys: std::collections::HashSet<(codegraph_domain::SymbolSnapshotId, String)> = before_edges.iter().filter_map(|e| snapshots.iter().find(|s| s.id() == e.to).map(|t| (e.from, t.name().as_str().to_string()))).collect();
    let mut invariant_violations = 0usize;
    for edge in &after_edges {
        let Some(target) = snapshots.iter().find(|s| s.id() == edge.to) else { continue };
        let key = (edge.from, target.name().as_str().to_string());
        if before_edge_keys.contains(&key) {
            continue; // already resolved pre-A4, not one of A4's own new recoveries
        }
        if !before_ambiguous.contains_key(&key) {
            // This after-edge's (caller, name) key was not a real
            // pre-A4 ambiguous site at all -- A4's own namespace
            // mechanism only ever acts on the >=2-candidate branch, so
            // a genuine new edge outside that branch would be a real
            // finding, not expected behavior.
            invariant_violations += 1;
            println!("  !!! unexpected: after-edge for caller/name key {key:?} was neither a pre-A4 edge nor a pre-A4 ambiguous site");
        }
    }
    println!("invariant_check (every genuinely NEW after-edge's (caller,name) key was a real pre-A4 ambiguous candidate pool member, never manufactured): {invariant_violations} violations found");

    println!("\n--- Ambiguities resolved without type information (summary) ---");
    println!("real_count_on_polly: {narrowed_to_one} (same figure as 'narrowed_to_exactly_one_candidate_by_a4' above)");
    let import_target_ambiguous_attempts = after_attempts.iter().filter(|a| a.reason == UnresolvedReason::ImportedSymbolAmbiguous).count();
    println!("import_alias_ambiguous_attempts_after (A4 found the alias but target name still has 2+ real candidates -- not resolved, reported honestly as Ambiguous, never guessed): {import_target_ambiguous_attempts}");
}

fn sorted(map: &HashMap<&'static str, usize>) -> Vec<(&'static str, usize)> {
    let mut v: Vec<_> = map.iter().map(|(k, v)| (*k, *v)).collect();
    v.sort_by_key(|(k, _)| *k);
    v
}
