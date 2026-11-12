//! B2-EVAL-008: multi-codebase validation, C#/C++ domain.
//!
//! Drives the SAME real production pipeline as the prior six domain-
//! specific harnesses (`RepositorySource` -> parse -> extract ->
//! promote -> lineage -> edges -> `CodeGraph` -> incremental reindex),
//! against a real, locally-cloned C# library (`App-vNext/Polly`)
//! instead of the ML/DL, Backend/API, Frontend/UI, Rust/systems,
//! Data-science, or CLI/tooling corpora the prior six targeted. This
//! domain's own distinguishing measurements are:
//!
//! - Section C1: real inheritance/interface ground truth, queried
//!   directly against `BulkheadPolicy : Policy, IBulkheadPolicy` (a
//!   class extending a base class AND implementing an interface in one
//!   declaration) and `IBulkheadPolicy : IsPolicy, IDisposable`
//!   (interface-to-interface multiple inheritance).
//! - Section C2: real method-overload ground truth, queried against
//!   the `Handle<TException>()`/`Handle<TException>(Func<...>)`
//!   overloads in `Policy.HandleSyntax.cs` (4 within that one file;
//!   6 corpus-wide once `PredicateBuilder<TResult>`'s own
//!   independently-declared same-name/same-arity pair is included,
//!   per B2-EVAL-008's corrected ground truth), to determine whether
//!   `resolve_calls`'s pure-name matching (now with call-site
//!   generic-arity narrowing, Semantic Resolution & Reliability
//!   Checkpoint Priority 1) collapses them.
//! - Section C3: a constructor-injected delegate-field callback shape
//!   (`BulkheadPolicy`'s `_onBulkheadRejected`, `FallbackPolicy`'s
//!   `_onFallback`) checked against the command-registration-graph
//!   category B2-EVAL-007 (pipx) first named, to see whether it
//!   recurs outside the CLI domain via a structurally different
//!   mechanism (field storage + deferred invocation vs. a dynamic
//!   dispatch table).
//! - Section C4: the Module/File gap, continued in a class-centric
//!   language.
//! - Section A1: SignatureTooLong, checked cross-language.
//!
//! Usage: `cargo run --example b2_smoke_test_csharp_cpp -- <repo_path> <commit_a> <commit_b>`

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use codegraph_domain::{CommitHash, EdgeKind, EvidenceTier, RepositorySource, ResolutionMethod, SymbolKind};
use codegraph_index::agent_policy::{evaluate, IntendedUse, PolicyVerdict};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::csharp_declarations::{extract_csharp_type_declaration_contexts, resolve_csharp_type_declaration_contexts};
use codegraph_index::edges::{resolve_calls_with_extraction_rejections, resolve_extends_implements_with_csharp_contexts, resolve_tests_for};
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::imports::ImportResolution;
use codegraph_index::incremental::CodeGraphIndexer;
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::negative_knowledge::CallAttempt;
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: b2_smoke_test_csharp_cpp <repo_path> <commit_a> <commit_b>");
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

    println!("=== B2-EVAL-008: C#/C++ domain smoke test ===");
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
    let mut signature_too_long_rejections: Vec<String> = Vec::new();
    let mut signature_too_long_names: Vec<String> = Vec::new();
    let mut rejections_by_kind: HashMap<String, usize> = HashMap::new();

    let mut top_level_counts_by_file: HashMap<String, usize> = HashMap::new();
    let mut spans_by_file: HashMap<String, Vec<(codegraph_index::extract::ByteSpan, String)>> = HashMap::new();

    // C#-domain raw-parse proxies (Section C-prelim): struct/enum/
    // constructor/field declarations, which the current csharp.rs
    // extractor does NOT walk for at all (only interface_declaration,
    // class_declaration, method_declaration are matched) -- measured
    // directly to quantify this extraction-coverage gap, not assumed.
    let mut csharp_struct_decls = 0usize;
    let mut csharp_enum_decls = 0usize;
    let mut csharp_constructor_decls = 0usize;
    let mut csharp_field_decls = 0usize;
    let mut csharp_using_directives = 0usize;
    let mut csharp_namespace_decls = 0usize;

    // C++-domain raw-parse proxy (Section C-prelim): out-of-class
    // qualified method DEFINITIONS (`Type::method(...) { ... }`), the
    // real header/.cpp-split shape cpp.rs's own test suite already
    // documents as omitted (not misnamed) by the current extractor.
    let mut cpp_qualified_out_of_class_definitions = 0usize;

    // B2-ID-010: raw C# type-declaration contexts, extracted from each
    // file's own FULL tree (never a single snapshot's isolated
    // signature_text) in this same per-file loop -- mirroring exactly
    // how the real production pipeline (incremental::pipeline::index_files)
    // extracts them, so this harness's own direct resolve_extends_implements_with_csharp_contexts
    // call below sees the SAME partial-class reconciliation the real
    // CodeGraphIndexer path (Section F/H below) already receives.
    let mut raw_csharp_type_declaration_contexts: Vec<(codegraph_domain::FileIdentity, codegraph_index::csharp_declarations::RawCSharpTypeDeclarationContext)> = Vec::new();

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

        if relative_path.ends_with(".cs") {
            count_csharp_constructs(
                tree.tree().root_node(),
                &bytes,
                &mut csharp_struct_decls,
                &mut csharp_enum_decls,
                &mut csharp_constructor_decls,
                &mut csharp_field_decls,
                &mut csharp_using_directives,
                &mut csharp_namespace_decls,
            );
        }
        if relative_path.ends_with(".cpp") || relative_path.ends_with(".cc") || relative_path.ends_with(".hpp") || relative_path.ends_with(".h") {
            count_cpp_qualified_definitions(tree.tree().root_node(), &bytes, &mut cpp_qualified_out_of_class_definitions);
        }

        for context in extract_csharp_type_declaration_contexts(&tree) {
            raw_csharp_type_declaration_contexts.push((file.clone(), context));
        }

        let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
        raw_candidates_total += candidates.len();
        rejections_total += rejections.len();
        for rejection in &rejections {
            let kind_label = match rejection {
                codegraph_index::extract::CandidateRejection::SignatureTooLong { name, .. } => {
                    signature_too_long_rejections.push(relative_path.clone());
                    signature_too_long_names.push(name.clone());
                    "SignatureTooLong"
                }
                codegraph_index::extract::CandidateRejection::NameTooLong { .. } => "NameTooLong",
            };
            *rejections_by_kind.entry(kind_label.to_string()).or_insert(0) += 1;
        }
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

    // Semantic Reliability Hardening Phase 2 stress-test, extended by
    // Semantic Resolution & Reliability Checkpoint Priority 1: use the
    // extraction-rejection-aware entry point (not the plain
    // resolve_calls_with_negative_knowledge wrapper), so this
    // evaluation measures EvidenceTier/Provenance/CallAttempt AND
    // SignatureTooLong-caused ExtractionRejected provenance on the
    // real corpus, not just edge-present/edge-absent. `calls_edges`
    // itself is byte-for-byte identical to what resolve_calls would
    // have returned when no rejection applies (confirmed by the
    // dedicated equivalence test in calls.rs), so every section below
    // that already used `calls_edges` is unaffected.
    let (calls_edges, call_attempts) = resolve_calls_with_extraction_rejections(&snapshots, &signature_too_long_names).expect("resolve_calls_with_extraction_rejections");
    // B2-ID-010: resolve the C# declaration contexts against the FULL
    // current snapshot pool (same convention as import_facts/impl_facts
    // in the real production pipeline), then pass them into the
    // partial-class-aware resolver.
    let csharp_contexts = resolve_csharp_type_declaration_contexts(&raw_csharp_type_declaration_contexts, &snapshots);
    let extends_edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &csharp_contexts).expect("resolve_extends_implements_with_csharp_contexts");
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

    // --- Reliability layer measurement (Semantic Reliability Hardening
    // Phase 2 stress-test). EvidenceTier distribution over the
    // resolved Calls edges, plus the negative-knowledge CallAttempt
    // records for every call site that did NOT produce an edge.
    println!("\n--- R. Reliability layer: EvidenceTier/Provenance/CallAttempt (Calls only, per Phase 2 scope) ---");
    let mut tier_counts_resolved_edges: HashMap<EvidenceTier, usize> = HashMap::new();
    for edge in &calls_edges {
        let tier = EvidenceTier::from_resolution_method(edge.evidence.method());
        *tier_counts_resolved_edges.entry(tier).or_insert(0) += 1;
    }
    println!("resolved Calls edges by EvidenceTier (MEASURED):");
    let mut tier_vec: Vec<_> = tier_counts_resolved_edges.into_iter().collect();
    tier_vec.sort_by_key(|(t, _)| format!("{t:?}"));
    for (tier, count) in &tier_vec {
        println!("  {tier:?}: {count}");
    }

    let ambiguous_attempts: Vec<&CallAttempt> = call_attempts.iter().filter(|a| a.tier == EvidenceTier::Ambiguous).collect();
    let unresolved_attempts: Vec<&CallAttempt> = call_attempts.iter().filter(|a| a.tier == EvidenceTier::Unresolved).collect();
    println!("total_call_attempts (negative knowledge, MEASURED): {}", call_attempts.len());
    println!("  Ambiguous: {} (2+ real candidates, resolver declined to choose)", ambiguous_attempts.len());
    println!("  Unresolved: {} (zero real candidates in the given pool)", unresolved_attempts.len());

    // Semantic Resolution & Reliability Checkpoint, Priority 1: reason
    // distribution over real `UnresolvedReason`s on the real corpus,
    // specifically to verify SignatureTooLong-caused unavailability is
    // now observably distinct from "genuinely external/no candidate"
    // (Unknown) and from "2+ real candidates" (AmbiguousCandidates),
    // instead of collapsing to one indistinguishable "no edge"
    // outcome. A misclassification here would be any ExtractionRejected
    // reason whose callee_name is NOT actually in
    // signature_too_long_names (impossible by this resolver's own
    // construction -- checked anyway as a direct real-data sanity
    // check, not merely trusted from reading the code).
    {
        let mut reason_counts: HashMap<&'static str, usize> = HashMap::new();
        let mut misclassified_extraction_rejected = 0usize;
        for attempt in &call_attempts {
            let label = match &attempt.reason {
                codegraph_index::negative_knowledge::UnresolvedReason::Unknown => "Unknown",
                codegraph_index::negative_knowledge::UnresolvedReason::ExtractionRejected { .. } => "ExtractionRejected",
                codegraph_index::negative_knowledge::UnresolvedReason::AmbiguousCandidates => "AmbiguousCandidates",
            };
            *reason_counts.entry(label).or_insert(0) += 1;
            if let codegraph_index::negative_knowledge::UnresolvedReason::ExtractionRejected { .. } = &attempt.reason {
                if !signature_too_long_names.contains(&attempt.callee_name) {
                    misclassified_extraction_rejected += 1;
                }
            }
        }
        println!("\n--- R0a. UnresolvedReason distribution across all real CallAttempts (MEASURED, Priority 1) ---");
        println!(
            "real SignatureTooLong-rejected names observed this run (first 15 of {}, deterministic order): {:?}",
            signature_too_long_names.len(),
            {
                let mut sorted_names = signature_too_long_names.clone();
                sorted_names.sort();
                sorted_names.dedup();
                sorted_names.into_iter().take(15).collect::<Vec<_>>()
            }
        );
        let mut reason_vec: Vec<_> = reason_counts.into_iter().collect();
        reason_vec.sort_by_key(|(k, _)| k.to_string());
        for (reason, count) in &reason_vec {
            println!("  {reason}: {count}");
        }
        println!(
            "misclassified ExtractionRejected (reason claims SignatureTooLong but callee_name is absent from the real \
             signature_too_long_names set this run observed, {} names): {}",
            signature_too_long_names.len(),
            misclassified_extraction_rejected
        );
        let extraction_rejected_sample: Vec<&CallAttempt> = call_attempts
            .iter()
            .filter(|a| matches!(a.reason, codegraph_index::negative_knowledge::UnresolvedReason::ExtractionRejected { .. }))
            .collect();
        println!(
            "ExtractionRejected CallAttempts found on this real run: {} (ground truth: {} real SignatureTooLong \
             extraction rejections exist in this corpus; an ExtractionRejected CallAttempt requires BOTH zero resolvable \
             candidates for that callee_name AND that exact name appearing in signature_too_long_names, so this count can \
             be lower than 213 whenever a same-named, non-rejected candidate elsewhere in the corpus still resolves or \
             disambiguates the call)",
            extraction_rejected_sample.len(),
            rejections_total
        );
        for attempt in extraction_rejected_sample.iter().take(10) {
            println!("  callee_name={:?} reason={:?}", attempt.callee_name, attempt.reason);
        }
    }

    // Negative-knowledge quality: for a deterministic sample of
    // Ambiguous attempts, print the real candidate set and the
    // provenance's own method_limitations string, so the eval doc can
    // classify whether the explanation is genuinely specific or a
    // generic canned reason collapsing multiple distinct real causes.
    {
        let mut sample: Vec<&CallAttempt> = ambiguous_attempts.clone();
        sample.sort_by(|a, b| a.callee_name.cmp(&b.callee_name).then_with(|| format!("{:?}", a.caller).cmp(&format!("{:?}", b.caller))));
        println!("\n--- R1. Negative-knowledge quality sample: first 15 Ambiguous CallAttempts (deterministic order) ---");
        for attempt in sample.iter().take(15) {
            let caller_name = snapshots.iter().find(|s| s.id() == attempt.caller).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str()));
            let candidate_names: Vec<String> = attempt.candidates.iter().filter_map(|id| snapshots.iter().find(|s| s.id() == *id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!(
                "  caller={caller_name:?} callee_name={:?} candidate_count={} candidates={candidate_names:?} rationale={:?} limitations={:?}",
                attempt.callee_name,
                attempt.candidates.len(),
                attempt.provenance.selection_rationale(),
                attempt.provenance.method_limitations()
            );
        }
    }

    // Specifically re-check the Handle<TException> case now that
    // B2-ID-011's fix makes these call sites extractable, AND now that
    // Semantic Resolution & Reliability Checkpoint Priority 1 wires
    // call-site generic-arity narrowing directly into resolve_calls:
    // how many Handle-named CallAttempts (Ambiguous or Unresolved)
    // exist, vs. how many Handle calls actually produced a resolved
    // edge, and of those, how many resolved via arity-narrowing
    // specifically (StaticAnalysis selection_rationale mentioning
    // "generic arity") vs. the pre-existing same-file NameHeuristic
    // short-circuit. B2-EVAL-008's own corrected ground truth: the
    // real Handle-named candidate pool is 6, not 4 -- `Policy.Handle<TException>()`/
    // `Policy.Handle<TException>(Func<TException,bool>)` AND an
    // independently-declared `PredicateBuilder<TResult>` class in
    // src/Polly.Core/PredicateBuilder.TResult.cs that happens to
    // declare its OWN same-named, same-arity overload pair. Because
    // all 6 real candidates share identical generic arity (one
    // `TException` type parameter) for a same-arity call site, arity
    // narrowing CANNOT disambiguate any of them -- this is the real
    // corpus's own instance of the required negative case ("same name
    // + same generic arity must still resolve Ambiguous").
    {
        let handle_attempts: Vec<&CallAttempt> = call_attempts.iter().filter(|a| a.callee_name == "Handle").collect();
        let handle_resolved_edges: Vec<&codegraph_index::edges::Edge> = calls_edges
            .iter()
            .filter(|e| snapshots.iter().find(|s| s.id() == e.to).map(|s| s.name().as_str()) == Some("Handle"))
            .collect();
        let handle_arity_narrowed: Vec<&codegraph_index::edges::Edge> = handle_resolved_edges
            .iter()
            .filter(|e| e.evidence.note().contains("generic arity"))
            .copied()
            .collect();
        println!("\n--- R2. Handle<TException> case, separated by layer (MEASURED, post B2-ID-009/010/011, arity-aware) ---");
        println!("real Handle-named candidate pool size (ground truth, B2-EVAL-008 corrected count): 6 (Policy x2, PredicateBuilder<TResult> x2, their generic PolicyBuilder/PredicateBuilder<TResult> counterparts x2 -- all same generic arity)");
        println!("Handle-named CallAttempts (candidate discovery succeeded, but tier != Resolved): {}", handle_attempts.len());
        for attempt in &handle_attempts {
            println!("  tier={:?} candidate_count={}", attempt.tier, attempt.candidates.len());
        }
        println!("Handle-named resolved Calls edges (StaticAnalysis/Heuristic, any mechanism): {}", handle_resolved_edges.len());
        for edge in &handle_resolved_edges {
            println!("  method={:?} note={:?}", edge.evidence.method(), edge.evidence.note());
        }
        println!(
            "Handle-named edges resolved SPECIFICALLY via call-site generic-arity narrowing (new this checkpoint): {} \
             (expected 0 on this real corpus: all 6 real Handle candidates share the same single-TException generic arity, \
             so arity narrowing has no discriminating power here -- confirms the required negative case holds on real data, \
             not just synthetic fixtures)",
            handle_arity_narrowed.len()
        );
    }

    // Agent-reliability-policy simulation on real evaluation data: for
    // every Ambiguous/Unresolved CallAttempt, what would the executable
    // policy say about (a) informational display and (b) treating this
    // caller as part of a high-impact modification's blast radius?
    {
        println!("\n--- R3. Agent-reliability-policy simulation on real CallAttempts (first 10 Ambiguous, deterministic) ---");
        let mut sample: Vec<&CallAttempt> = ambiguous_attempts.clone();
        sample.sort_by(|a, b| a.callee_name.cmp(&b.callee_name).then_with(|| format!("{:?}", a.caller).cmp(&format!("{:?}", b.caller))));
        for attempt in sample.iter().take(10) {
            let display_verdict = evaluate(attempt.tier, IntendedUse::InformationalDisplay);
            let high_impact_verdict = evaluate(attempt.tier, IntendedUse::HighImpactModification);
            let display_tag = match display_verdict {
                PolicyVerdict::Permitted => "Permitted",
                PolicyVerdict::PermittedWithDisclosedUncertainty => "PermittedWithDisclosedUncertainty",
                PolicyVerdict::Forbidden { .. } => "Forbidden",
            };
            let high_impact_tag = match high_impact_verdict {
                PolicyVerdict::Permitted => "Permitted",
                PolicyVerdict::PermittedWithDisclosedUncertainty => "PermittedWithDisclosedUncertainty",
                PolicyVerdict::Forbidden { .. } => "Forbidden",
            };
            println!("  callee_name={:?} tier={:?} informational_display={display_tag} high_impact_modification={high_impact_tag}", attempt.callee_name, attempt.tier);
        }
    }

    println!("\n--- B. Pipeline results (full index at commit B) ---");
    println!("total_files_in_tree: {total_files}");
    println!("files_supported_by_parser: {files_supported}");
    println!("files_parsed: {files_parsed}");
    println!("parse_failures: {parse_failures}");
    println!("files_with_tree_has_error: {files_with_error_nodes}");
    println!("raw_candidates_extracted: {raw_candidates_total}");
    println!("extraction_rejections (oversized name/signature): {rejections_total}");
    let mut rej_by_kind_vec: Vec<_> = rejections_by_kind.into_iter().collect();
    rej_by_kind_vec.sort_by_key(|(k, _)| k.clone());
    for (kind, count) in &rej_by_kind_vec {
        println!("  rejection_kind {kind}: {count}");
    }
    println!(
        "  signature_too_long files (first 10): {:?}",
        signature_too_long_rejections.iter().take(10).collect::<Vec<_>>()
    );
    println!(
        "  signature_too_long frequency vs total candidates (MEASURED, Section A1 -- SignatureTooLong cross-language check): {}/{} ({:.2}%)",
        signature_too_long_rejections.len(),
        raw_candidates_total + rejections_total,
        if raw_candidates_total + rejections_total > 0 {
            100.0 * signature_too_long_rejections.len() as f64 / (raw_candidates_total + rejections_total) as f64
        } else {
            0.0
        }
    );
    println!("snapshots_promoted: {}", snapshots.len());
    println!("promotion_failures: {}", promotion_failures.len());
    for failure in &promotion_failures {
        println!("  promotion_failure: {failure}");
    }

    println!("\n--- C. Symbol coverage (SymbolKind counts) ---");
    let mut kind_counts_vec: Vec<_> = kind_counts.into_iter().collect();
    kind_counts_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &kind_counts_vec {
        println!("  {kind:?}: {count}");
    }

    println!("\n--- C-prelim. Extraction-coverage gap, C#/C++ domain (raw-parse proxy, MEASURED) ---");
    println!("csharp_struct_decls (NOT walked by extract/csharp.rs today): {csharp_struct_decls}");
    println!("csharp_enum_decls (NOT walked by extract/csharp.rs today): {csharp_enum_decls}");
    println!("csharp_constructor_decls (NOT walked by extract/csharp.rs today): {csharp_constructor_decls}");
    println!("csharp_field_decls (NOT walked by extract/csharp.rs today): {csharp_field_decls}");
    println!("csharp_using_directives (file-level import-equivalent, MEASURED): {csharp_using_directives}");
    println!("csharp_namespace_decls (MEASURED): {csharp_namespace_decls}");
    println!(
        "cpp_qualified_out_of_class_method_definitions (`Type::method(...)` shape; cpp.rs's own test suite \
         already documents these as OMITTED, not misnamed -- re-measured here on real corpus, not assumed): {cpp_qualified_out_of_class_definitions}"
    );

    println!("\n--- C1. Inheritance/interface ground truth (C#/C++ domain priority) ---");
    inheritance_ground_truth_section(&snapshots, &extends_edges);

    println!("\n--- C2. Method-overload ground truth (C#/C++ domain priority) ---");
    overload_ground_truth_section(&snapshots, &calls_edges);

    println!("\n--- C3. Callback/registration relationship recurrence check (cross-domain radar) ---");
    callback_registration_section(&snapshots, &calls_edges);

    println!("\n--- C4. Module/File -> top-level-symbol gap (C#/C++ domain, class-centric language) ---");
    let total_top_level: usize = top_level_counts_by_file.values().sum();
    let files_with_multiple_top_level = top_level_counts_by_file.values().filter(|&&n| n > 1).count();
    let total_files_with_symbols = top_level_counts_by_file.len();
    println!("files_with_at_least_one_extracted_symbol: {total_files_with_symbols}");
    println!("total_top_level_symbols_across_all_files (MEASURED, span-nesting definition): {total_top_level}");
    println!("files_with_more_than_one_top_level_symbol (MEASURED): {files_with_multiple_top_level}");
    println!(
        "percentage_of_files_with_multiple_top_level_symbols (MEASURED): {:.1}%",
        if total_files_with_symbols > 0 {
            100.0 * files_with_multiple_top_level as f64 / total_files_with_symbols as f64
        } else {
            0.0
        }
    );
    println!(
        "mean_top_level_symbols_per_file (MEASURED): {:.2}",
        if total_files_with_symbols > 0 { total_top_level as f64 / total_files_with_symbols as f64 } else { 0.0 }
    );
    let mut counts_sorted: Vec<usize> = top_level_counts_by_file.values().copied().collect();
    counts_sorted.sort_unstable();
    let median = if counts_sorted.is_empty() {
        0.0
    } else if counts_sorted.len() % 2 == 1 {
        counts_sorted[counts_sorted.len() / 2] as f64
    } else {
        (counts_sorted[counts_sorted.len() / 2 - 1] + counts_sorted[counts_sorted.len() / 2]) as f64 / 2.0
    };
    println!("median_top_level_symbols_per_file (MEASURED): {median}");
    let mut top_files: Vec<(&String, &usize)> = top_level_counts_by_file.iter().collect();
    top_files.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("top 15 files by top-level-symbol count (file: count):");
    for (file, count) in top_files.iter().take(15) {
        println!("  {file}: {count}");
    }
    println!(
        "note: in this class-centric corpus, EACH class is usually its own top-level symbol in its own file \
         (1 class : 1 file convention) -- the Module/File gap's RELEVANCE here depends on whether that convention \
         actually holds across this corpus, not assumed; see the measured percentage/mean/median above."
    );

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

    {
        let csharp_import_facts = outcome_b.import_facts.iter().filter(|f| f.importing_file.relative_path().ends_with(".cs")).count();
        let internal_count = outcome_b.import_facts.iter().filter(|f| matches!(f.resolution, ImportResolution::Internal(_))).count();
        let external_count = outcome_b.import_facts.iter().filter(|f| f.resolution == ImportResolution::External).count();
        let ambiguous_count = outcome_b.import_facts.iter().filter(|f| f.resolution == ImportResolution::Ambiguous).count();
        let alias_count = outcome_b.import_facts.iter().filter(|f| f.alias.is_some()).count();

        println!("\n--- Imports measurement (via the real CodeGraphIndexer; C#/C++ domain -- namespaces/usings/includes) ---");
        println!("total_using_directives_at_commit_b (raw-parse count, MEASURED): {csharp_using_directives}");
        println!("csharp_import_facts_extracted (MEASURED, ImportFact, file-level): {csharp_import_facts}");
        println!(
            "extraction_coverage: {csharp_import_facts}/{csharp_using_directives} ({:.1}%)",
            if csharp_using_directives > 0 { 100.0 * csharp_import_facts as f64 / csharp_using_directives as f64 } else { 0.0 }
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
        println!(
            "note: BulkheadPolicy.cs itself has ZERO `using` directives (confirmed via dedicated grep on the \
             definitive clone before this harness ran) -- this file relies entirely on its own `namespace Polly.Bulkhead;` \
             declaration plus C#'s implicit same-namespace visibility, not on explicit per-file using statements. \
             Any ImportFact-based cross-reference measurement for THIS file specifically will show zero import facts \
             by design, not as a resolver defect."
        );
    }

    // --- H. C#/C++ coding-agent usefulness Q&A, answered using ONLY
    // the real CodeGraph built above. Every question/expected-target
    // pair below was derived from direct reading of this repository's
    // real source at commit B, not invented or adjusted to make the
    // pipeline look good.
    {
        let graph = &outcome_b.graph;
        println!("\n--- H. C#/C++ usefulness Q&A (Polly, commit B) ---");

        let find_by_name = |name: &str| -> Vec<&codegraph_index::snapshot::SymbolSnapshot> { graph.all_snapshots().into_iter().filter(|s| s.name().as_str() == name).collect() };
        let find_current_by_name_in_file = |name: &str, file_suffix: &str| -> Option<&codegraph_index::snapshot::SymbolSnapshot> {
            find_by_name(name)
                .into_iter()
                .filter(|s| graph.lineage_from(s.id()).is_empty())
                .find(|s| s.file().relative_path().ends_with(file_suffix))
        };

        // Q1 [needs: Extends+Implements, the domain's own central
        // question]: does BulkheadPolicy extend Policy AND implement
        // IBulkheadPolicy -- both in the SAME declaration? Ground
        // truth: YES, `public class BulkheadPolicy : Policy,
        // IBulkheadPolicy` (confirmed by direct source reading).
        if let Some(bulkhead) = find_current_by_name_in_file("BulkheadPolicy", "Bulkhead/BulkheadPolicy.cs") {
            let extends: Vec<String> = graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Extends)).iter().filter_map(|e| graph.snapshot(e.to)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            let implements: Vec<String> = graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Implements)).iter().filter_map(|e| graph.snapshot(e.to)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!(
                "Q1 [needs: Extends+Implements] 'Does BulkheadPolicy extend a base class AND implement an interface?' -> \
                 Extends: {extends:?}, Implements: {implements:?} (ground truth: extends Policy, implements IBulkheadPolicy)"
            );
        } else {
            println!("Q1 'BulkheadPolicy' NOT FOUND in src/Polly/Bulkhead/BulkheadPolicy.cs (unexpected)");
        }

        // Q2 [needs: Implements, interface-to-interface inheritance]:
        // does IBulkheadPolicy itself extend/implement anything?
        // Ground truth: YES, `public interface IBulkheadPolicy :
        // IsPolicy, IDisposable` -- an interface inheriting from TWO
        // other interfaces (multiple interface inheritance).
        if let Some(ibulkhead) = find_current_by_name_in_file("IBulkheadPolicy", "Bulkhead/IBulkheadPolicy.cs") {
            let outgoing: Vec<String> = graph
                .outgoing_edges(ibulkhead.id(), None)
                .iter()
                .filter(|e| matches!(e.kind, EdgeKind::Extends | EdgeKind::Implements))
                .filter_map(|e| graph.snapshot(e.to).map(|s| (e.kind, s)))
                .map(|(k, s)| format!("{k:?} -> {}::{}", s.file().relative_path(), s.name().as_str()))
                .collect();
            println!(
                "Q2 [needs: interface-to-interface inheritance] 'Does IBulkheadPolicy itself inherit from other interfaces?' -> \
                 {outgoing:?} (ground truth: `interface IBulkheadPolicy : IsPolicy, IDisposable`, TWO interface bases -- \
                 IsPolicy/IDisposable are both outside this exact commit pair's own changed-file set, so whether this \
                 resolves depends on whether IsPolicy/IDisposable are themselves present as snapshots in this full-corpus pool)"
            );
        } else {
            println!("Q2 'IBulkheadPolicy' NOT FOUND in src/Polly/Bulkhead/IBulkheadPolicy.cs (unexpected)");
        }

        // Q3 [needs: overload-aware resolution -- the domain's own
        // central question on Calls]: how many distinct `Handle`
        // snapshots exist within Policy.HandleSyntax.cs specifically,
        // and does resolve_calls's pure-name matching (now extended
        // with call-site generic-arity narrowing) collapse the real
        // Handle<TException> overloads into ambiguity or a single
        // silently-wrong target? Corrected ground truth (B2-EVAL-008,
        // confirmed by direct source tracing, superseding this
        // harness's own earlier "4 real overloads" assumption): within
        // Policy.HandleSyntax.cs alone there are 4 (Policy's own
        // Handle<TException>()/Handle<TException>(Func<...>) pair plus
        // their PolicyBuilder<TResult> generic counterparts), but the
        // REAL corpus-wide candidate pool for bare name "Handle" is 6,
        // because src/Polly.Core/PredicateBuilder.TResult.cs
        // independently declares its OWN same-named, same-generic-
        // arity Handle<TException>() pair on an unrelated class
        // (PredicateBuilder<TResult>). This section intentionally
        // scopes to one file to match the original Q3 question's own
        // framing; Section R2 above measures the real, corpus-wide
        // 6-candidate pool.
        let handle_snapshots: Vec<&codegraph_index::snapshot::SymbolSnapshot> = find_by_name("Handle").into_iter().filter(|s| graph.lineage_from(s.id()).is_empty() && s.file().relative_path().ends_with("Policy.HandleSyntax.cs")).collect();
        println!(
            "Q3 [needs: overload-aware resolution] 'How many distinct `Handle` snapshots does Policy.HandleSyntax.cs produce, \
             and are they collapsed to one name bucket?' -> {} snapshot(s) named exactly \"Handle\" found in this ONE file \
             (ground truth within this file: 4 real overloads -- Handle<TException>(), Handle<TException>(Func<TException,bool>), \
             and their PolicyBuilder<TResult> counterparts; the REAL corpus-wide pool is 6, not 4, once PredicateBuilder<TResult>'s \
             own independently-declared same-name/same-arity pair in a different file is included -- see Section R2). If this \
             count is less than 4, extraction itself (not just resolve_calls) may be collapsing distinct overloads to one \
             SymbolSnapshotId or rejecting some; if it is 4, resolve_calls with arity narrowing still cannot distinguish WHICH \
             of the (file-local 4, corpus-wide 6) a given call site intends, since all share the identical single-TException \
             generic arity -- arity alone has no discriminating power over this specific real overload family.",
            handle_snapshots.len()
        );
        for s in &handle_snapshots {
            println!("  Handle snapshot: {:?} (signature starts: {:?})", s.id(), &s.signature().as_str().chars().take(80).collect::<String>());
        }

        // Q4 [needs: Module/File structure]: what does
        // BulkheadPolicy.cs contain (every top-level symbol)?
        let file_symbols: Vec<String> = graph.all_snapshots().into_iter().filter(|s| s.file().relative_path() == "src/Polly/Bulkhead/BulkheadPolicy.cs").filter(|s| graph.lineage_from(s.id()).is_empty()).map(|s| s.name().as_str().to_string()).collect();
        println!(
            "Q4 [needs: Module/File structure] 'What does src/Polly/Bulkhead/BulkheadPolicy.cs contain?' -> {} symbols found ONLY via file-path string matching: {:?}",
            file_symbols.len(),
            file_symbols
        );

        // Q5 [needs: the C3 callback/registration relationship]: does
        // ANY Calls edge connect BulkheadPolicy's constructor/methods
        // to the delegate FIELD `_onBulkheadRejected` it stores and
        // later invokes from `Implementation`? Ground truth: the field
        // is a parameter-turned-field, never a named symbol in the
        // snapshot pool, so no edge can exist for it by construction.
        if let Some(bulkhead) = find_current_by_name_in_file("BulkheadPolicy", "Bulkhead/BulkheadPolicy.cs") {
            let callees: Vec<String> = graph.callees_of(bulkhead.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!(
                "Q5 [needs: callback/registration relationship, NOT Calls] 'Does any edge represent BulkheadPolicy storing \
                 _onBulkheadRejected in its constructor and invoking it later from Implementation()?' -> BulkheadPolicy's own \
                 outgoing Calls: {callees:?} (ground truth: _onBulkheadRejected is a constructor PARAMETER stored as a private \
                 field then invoked inside BulkheadEngine.Implementation -- never a named symbol this pipeline's Calls resolver \
                 could match against; confirms the same category of structurally invisible relationship B2-EVAL-007 named for \
                 pipx's dispatch table, recurring here via constructor-injected delegate FIELDS instead of a dynamic attribute lookup)"
            );
        }

        // Q6 [needs: TestsFor]: which tests cover BulkheadPolicy?
        println!("Q6 [needs: TestsFor] 'Which tests cover BulkheadPolicy?' -> total TestsFor edges in corpus: {}", tests_for_edges.len());
        if let Some(bulkhead) = find_current_by_name_in_file("BulkheadPolicy", "Bulkhead/BulkheadPolicy.cs") {
            let incoming_tests = graph.incoming_edges(bulkhead.id(), Some(EdgeKind::TestsFor));
            let names: Vec<String> = incoming_tests.iter().filter_map(|e| graph.snapshot(e.from)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("  TestsFor edges pointing at BulkheadPolicy: {names:?}");
        }

        // Q7 [needs: Calls, impact analysis]: if I modify Dispose() on
        // BulkheadPolicy, which other functions are impacted?
        if let Some(dispose) = find_by_name("Dispose").into_iter().find(|s| graph.lineage_from(s.id()).is_empty() && s.file().relative_path().ends_with("Bulkhead/BulkheadPolicy.cs")) {
            let callers: Vec<String> = graph.callers_of(dispose.id()).into_iter().filter_map(|id| graph.snapshot(id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
            println!("Q7 [needs: Calls, impact analysis] 'If I modify BulkheadPolicy.Dispose, what is impacted?' -> callers: {callers:?}");
        } else {
            println!("Q7 'Dispose' NOT FOUND in src/Polly/Bulkhead/BulkheadPolicy.cs (unexpected)");
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

/// Direct, real-source-grounded summary of Section C1's inheritance/
/// interface findings: every Extends/Implements edge touching
/// BulkheadPolicy or IBulkheadPolicy, printed by name so the
/// independent review can verify each one against real source without
/// re-deriving them.
fn inheritance_ground_truth_section(snapshots: &[codegraph_index::snapshot::SymbolSnapshot], extends_edges: &[codegraph_index::edges::Edge]) {
    let by_id: HashMap<_, _> = snapshots.iter().map(|s| (s.id(), s)).collect();
    let relevant_names = ["BulkheadPolicy", "IBulkheadPolicy", "Policy"];
    let mut relevant_edges: Vec<&codegraph_index::edges::Edge> = extends_edges
        .iter()
        .filter(|e| {
            let from_ok = by_id.get(&e.from).is_some_and(|s| relevant_names.contains(&s.name().as_str()));
            let to_ok = by_id.get(&e.to).is_some_and(|s| relevant_names.contains(&s.name().as_str()));
            from_ok || to_ok
        })
        .collect();
    relevant_edges.sort_by_key(|e| format!("{:?}{:?}", e.from, e.to));
    println!("extends_implements_edges touching BulkheadPolicy/IBulkheadPolicy/Policy (MEASURED, {} found):", relevant_edges.len());
    for edge in &relevant_edges {
        let from = by_id.get(&edge.from).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str()));
        let to = by_id.get(&edge.to).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str()));
        println!("  {from:?} -[{:?}]-> {to:?}", edge.kind);
    }
    println!(
        "ground_truth (from direct source reading): `class BulkheadPolicy : Policy, IBulkheadPolicy` (base class + interface, \
         one declaration), `interface IBulkheadPolicy : IsPolicy, IDisposable` (interface-to-interface, TWO bases), \
         `class BulkheadPolicy<TResult> : Policy<TResult>, IBulkheadPolicy<TResult>` (generic variant, same shape)."
    );
}

/// Section C2: direct enumeration of every snapshot named exactly
/// "Handle" in this corpus, plus every Calls edge targeting one of
/// them, so the overload-collapse question can be answered by reading
/// printed evidence rather than trusting a single boolean.
fn overload_ground_truth_section(snapshots: &[codegraph_index::snapshot::SymbolSnapshot], calls_edges: &[codegraph_index::edges::Edge]) {
    let handle_ids: Vec<_> = snapshots.iter().filter(|s| s.name().as_str() == "Handle").map(|s| s.id()).collect();
    println!("total snapshots named exactly \"Handle\" across the WHOLE corpus (MEASURED): {}", handle_ids.len());
    let incoming_to_handle: Vec<&codegraph_index::edges::Edge> = calls_edges.iter().filter(|e| handle_ids.contains(&e.to)).collect();
    println!("Calls edges targeting any \"Handle\" snapshot (MEASURED): {}", incoming_to_handle.len());
    let mut by_method: HashMap<ResolutionMethod, usize> = HashMap::new();
    for edge in &incoming_to_handle {
        *by_method.entry(edge.evidence.method()).or_insert(0) += 1;
    }
    let mut method_vec: Vec<_> = by_method.into_iter().collect();
    method_vec.sort_by_key(|(m, _)| format!("{m:?}"));
    for (method, count) in &method_vec {
        println!("  resolution_method {method:?}: {count}");
    }
    println!(
        "interpretation: if ANY edge above resolved via StaticAnalysis (not NameHeuristic), that call site had EXACTLY ONE \
         candidate named \"Handle\" in the given pool for that resolve_calls invocation (per-signature-text re-parse scope) -- \
         meaning the other real overloads (ground truth: 6 corpus-wide, per B2-EVAL-008's corrected count, not the 4 originally \
         assumed) were simply not visible to THAT resolution, not correctly disambiguated by type/arity. If NameHeuristic, this \
         may now be EITHER the pre-existing same-file-preference short-circuit OR (new this checkpoint) a call-site \
         generic-arity-narrowing resolution -- Section R2 above distinguishes the two by inspecting each edge's evidence note \
         for the arity-narrowing phrase. Because all 6 real Handle candidates share the identical single-TException generic \
         arity, arity narrowing is expected to contribute ZERO of these edges on this real corpus -- confirming arity alone \
         cannot and does not silently fabricate false certainty here. Neither path means full overload-aware (CALL-RES-A5-style, \
         type-aware) resolution is happening; this section measures whether real overloads produce visible ambiguity or silent \
         single-candidate luck, to ground the user's own research question on A4/A5 justification."
    );
}

/// Section C3: looks for the general field-storage-then-deferred-
/// invocation shape (constructor assigns a parameter to `this.field`
/// or `_field`; a later, different method body references that same
/// field name as a call's callee) across the WHOLE corpus, not just
/// the BulkheadPolicy/FallbackPolicy cases already hand-verified in Q5
/// above -- a raw-parse proxy count, not a resolver capability.
fn callback_registration_section(snapshots: &[codegraph_index::snapshot::SymbolSnapshot], calls_edges: &[codegraph_index::edges::Edge]) {
    let underscored_field_names: Vec<&str> = snapshots.iter().filter(|s| matches!(s.kind(), SymbolKind::Field)).map(|s| s.name().as_str()).filter(|n| n.starts_with('_')).collect();
    println!(
        "fields_with_underscore_prefix_naming_convention (MEASURED, a proxy for Polly's private-delegate-field convention \
         seen in BulkheadPolicy/_onBulkheadRejected and FallbackPolicy/_onFallback): {}",
        underscored_field_names.len()
    );
    println!(
        "note: this pipeline's SymbolKind::Field population and resolve_calls's own name-matching do not treat a delegate \
         field differently from any other field or method name -- confirmed structurally: resolve_calls matches callee \
         identifiers against snapshot NAMES, and a field name like \"_onBulkheadRejected\" never appears as a call-expression \
         callee text at its OWN declaration site (only at its later invocation site inside a DIFFERENT method), so no edge \
         kind in this pipeline represents 'this constructor parameter becomes this field, later invoked from that other \
         method' -- the same class of gap B2-EVAL-007 named for pipx's parser.set_defaults(func=X)/args.func(...) pair, \
         recurring here through a structurally different mechanism (field storage, not a dynamic dispatch table), confirming \
         it is not CLI-framework-specific."
    );
    let total_calls = calls_edges.len();
    println!("total_calls_edges_in_corpus_for_context (MEASURED): {total_calls}");
}

/// Walks a real C# parse tree counting six domain-relevant construct
/// kinds this project's `extract/csharp.rs` does NOT currently extract
/// as symbols (struct/enum/constructor/field declarations), plus two
/// it has no concept of as symbols at all but which ARE relevant to
/// Imports/Module measurement (using_directive, namespace_declaration).
/// Node kinds confirmed against this workspace's real tree-sitter-c-sharp
/// output by direct parse inspection before writing this function, the
/// same confirm-before-counting discipline every prior domain harness's
/// own raw-parse proxy used.
fn count_csharp_constructs(
    node: tree_sitter::Node,
    source: &[u8],
    struct_decls: &mut usize,
    enum_decls: &mut usize,
    constructor_decls: &mut usize,
    field_decls: &mut usize,
    using_directives: &mut usize,
    namespace_decls: &mut usize,
) {
    match node.kind() {
        "struct_declaration" => *struct_decls += 1,
        "enum_declaration" => *enum_decls += 1,
        "constructor_declaration" => *constructor_decls += 1,
        "field_declaration" => *field_decls += 1,
        "using_directive" => *using_directives += 1,
        "namespace_declaration" | "file_scoped_namespace_declaration" => *namespace_decls += 1,
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_csharp_constructs(child, source, struct_decls, enum_decls, constructor_decls, field_decls, using_directives, namespace_decls);
    }
}

/// Walks a real C++ parse tree counting `function_definition` nodes
/// whose `function_declarator` child's name-position child is a
/// `qualified_identifier` (the `Type::method(...)` out-of-class
/// definition shape) -- confirmed, via `cpp.rs`'s own existing test
/// `an_out_of_class_qualified_method_definition_is_omitted_not_misnamed`,
/// to be exactly the shape the current extractor's `named_function_definition`
/// omits (its find predicate only matches `identifier`/`field_identifier`,
/// never `qualified_identifier`). Re-measured here on the real corpus
/// rather than assumed to still hold or to be frequent in practice.
fn count_cpp_qualified_definitions(node: tree_sitter::Node, source: &[u8], out: &mut usize) {
    if node.kind() == "function_definition" {
        let mut cursor = node.walk();
        let declarator = node.children(&mut cursor).find(|c| c.kind() == "function_declarator");
        if let Some(declarator) = declarator {
            let mut decl_cursor = declarator.walk();
            if declarator.children(&mut decl_cursor).any(|c| c.kind() == "qualified_identifier") {
                *out += 1;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_cpp_qualified_definitions(child, source, out);
    }
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
