//! Quality & Semantic Reliability Hardening, Phase 1 — permanent
//! golden corpus + end-to-end semantic conformance harness.
//!
//! # Why this exists
//!
//! Every fixture under `tests/corpus/<language>/` pins a REAL,
//! previously-discovered defect or documented gap from this project's
//! own evaluation series (B2-ID-001 through B2-ID-011, CRC-001/002/003)
//! — minimized to the smallest real construct that still exercises the
//! exact shape the finding/gap was about, never invented from
//! scratch. Each test below drives that fixture through the REAL
//! production entry point (`CodeGraphIndexer::observe_commit`), not a
//! shortcut that calls an extractor or resolver directly — this is
//! deliberate: this evaluation series already found real bugs that a
//! unit test calling one stage in isolation would have missed (e.g.
//! "extractor correct but resolver unreachable," "resolver correct but
//! the harness queried the wrong EdgeKind," "full-index correct but
//! incremental diverges"). Driving the WHOLE chain —
//! `source -> parse -> extraction/structural-facts -> SymbolSnapshot
//! -> resolution -> CodeGraph -> query` — for at least the
//! cross-cutting cases below is what actually proves the pipeline
//! behaves as documented, not just that one stage's own unit tests
//! pass in isolation.
//!
//! # What this file is NOT
//!
//! It does not re-implement every existing unit test in `calls.rs`,
//! `extends_implements.rs`, etc. — those stay where they are and keep
//! testing their own stage's own internal logic with fine-grained
//! synthetic fixtures. This file exists one level up: real-looking
//! source files, driven through the real indexer, asserting on the
//! final queryable `CodeGraph` — the level at which the pipeline
//! actually gets used.
//!
//! # Fixture provenance
//!
//! Every fixture file under `tests/corpus/` carries its own comment
//! citing which finding/CRC case it pins and what the real repository
//! shape it was minimized from. A fixture whose own finding documents
//! a STILL-OPEN gap (e.g. CRC-002's prop-shadowing false positive, the
//! Rust macro-call-opacity gap, the C++ namespace-qualified-call gap,
//! the generic-arity-collision gap) asserts the CURRENT (gap-having)
//! behavior, not a desired one — flipping that assertion is exactly
//! how a future fix for that gap would be confirmed here, without
//! reinventing the minimal repro from scratch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use codegraph_domain::{CommitHash, EdgeKind, FileIdentity, RepositoryId, RepositoryRegistry, RepositoryRoot, RepositorySource, SourceError};
use codegraph_index::incremental::CodeGraphIndexer;

/// A `RepositorySource` backed by real files on disk under
/// `tests/corpus/<language>/`, loaded once per test. All files are
/// registered at ONE synthetic commit (`"a".repeat(40)`) — these
/// fixtures exist to pin single-commit semantic behavior, not
/// incremental-update behavior (the full/incremental equivalence gate
/// in `full_incremental_equivalence.rs` covers that separately,
/// against these same fixtures where useful).
struct CorpusSource {
    repository: RepositoryId,
    commit: CommitHash,
    files: HashMap<String, Vec<u8>>,
}

impl CorpusSource {
    /// Loads every file passed in `relative_paths` from
    /// `tests/corpus/<language>/`, keyed by their own relative path
    /// (e.g. `"same_name_same_body_different_files.py"`), all at one
    /// synthetic commit.
    fn load(language_dir: &str, relative_paths: &[&str]) -> Self {
        let mut registry = RepositoryRegistry::new();
        let repository = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let mut files = HashMap::new();
        for relative_path in relative_paths {
            let full_path = corpus_dir().join(language_dir).join(relative_path);
            let bytes = std::fs::read(&full_path).unwrap_or_else(|e| panic!("failed to read golden corpus fixture {full_path:?}: {e}"));
            files.insert(relative_path.to_string(), bytes);
        }
        Self { repository, commit, files }
    }
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("corpus")
}

impl RepositorySource for CorpusSource {
    fn list_files_at(&self, repository: RepositoryId, commit: &CommitHash) -> Result<Vec<String>, SourceError> {
        if repository == self.repository && commit == &self.commit {
            Ok(self.files.keys().cloned().collect())
        } else {
            Err(SourceError::UnknownCommit { repo: repository.as_uuid(), commit: commit.as_str().to_string() })
        }
    }

    fn read_file(&self, file: &FileIdentity) -> Result<Vec<u8>, SourceError> {
        self.files.get(file.relative_path()).cloned().ok_or_else(|| SourceError::UnknownFile {
            repo: file.repository().as_uuid(),
            commit: file.commit().as_str().to_string(),
            file: file.relative_path().to_string(),
        })
    }
}

/// Runs a full index of `source` at its own commit through the REAL
/// `CodeGraphIndexer::observe_commit` entry point — the same code path
/// production use goes through, not a shortcut.
fn index(source: &CorpusSource) -> codegraph_index::incremental::ObserveOutcome {
    let indexer = CodeGraphIndexer::new();
    indexer.observe_commit(source, source.repository, &source.commit, None, &[]).expect("golden corpus fixtures must index cleanly")
}

fn find_snapshot_by_name<'a>(outcome: &'a codegraph_index::incremental::ObserveOutcome, name: &str) -> Vec<&'a codegraph_index::snapshot::SymbolSnapshot> {
    outcome.graph.all_snapshots().into_iter().filter(|s| s.name().as_str() == name).collect()
}

fn find_one_by_name_in_file<'a>(outcome: &'a codegraph_index::incremental::ObserveOutcome, name: &str, file_suffix: &str) -> &'a codegraph_index::snapshot::SymbolSnapshot {
    find_snapshot_by_name(outcome, name)
        .into_iter()
        .find(|s| s.file().relative_path().ends_with(file_suffix))
        .unwrap_or_else(|| panic!("expected a snapshot named {name:?} in a file ending with {file_suffix:?}"))
}

// ===========================================================================
// B2-ID-001: SymbolSnapshotId must never collide for two distinct symbols
// sharing name + signature body, in different files.
// ===========================================================================

#[test]
fn b2_id_001_same_name_same_body_different_files_do_not_collide() {
    let source = CorpusSource::load("python", &["same_name_same_body_different_files.py", "same_name_same_body_different_files_b.py"]);
    let outcome = index(&source);

    let init_weights_snapshots = find_snapshot_by_name(&outcome, "init_weights");
    assert_eq!(init_weights_snapshots.len(), 2, "both init_weights methods must be extracted as distinct symbols");

    let ids: std::collections::HashSet<_> = init_weights_snapshots.iter().map(|s| s.id()).collect();
    assert_eq!(ids.len(), 2, "two init_weights methods with byte-identical bodies in DIFFERENT files must never collide to the same SymbolSnapshotId (B2-ID-001)");
}

// ===========================================================================
// B2-ID-007: nested-declaration ownership — a nested class's own base
// clause must be attributed to it alone, never also to its outer container.
// ===========================================================================

#[test]
fn b2_id_007_nested_class_extends_is_owned_by_inner_not_outer() {
    let source = CorpusSource::load("python", &["nested_class_extends_ownership.py"]);
    let outcome = index(&source);

    let base = find_one_by_name_in_file(&outcome, "Base", "nested_class_extends_ownership.py");
    let incoming = outcome.graph.incoming_edges(base.id(), Some(EdgeKind::Extends));
    let sources: Vec<&str> = incoming.iter().filter_map(|e| outcome.graph.snapshot(e.from)).map(|s| s.name().as_str()).collect();

    assert_eq!(sources, vec!["Inner"], "only Inner must extend Base through the real indexer; Outer must never also appear (B2-ID-007)");
}

// ===========================================================================
// CRC-003: import-alias blindness — documents the CURRENT gap (no edge),
// not a desired outcome. A future fix flips this assertion.
// ===========================================================================

#[test]
fn crc_003_import_alias_blindness_is_the_current_documented_gap() {
    let source = CorpusSource::load("python", &["import_alias_crc_003.py", "helper.py"]);
    let outcome = index(&source);

    let caller = find_one_by_name_in_file(&outcome, "caller", "import_alias_crc_003.py");
    let callees: Vec<String> = outcome.graph.outgoing_edges(caller.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).map(|s| s.name().as_str().to_string()).collect();

    assert!(
        callees.is_empty(),
        "CRC-003's known gap: a call through an import alias produces NO edge today, \
         since resolve_calls never consults ImportFact.alias -- if this assertion ever fails, \
         CRC-003 has been fixed and this test (and the CRC doc) must be updated to assert \
         the real edge instead of its absence"
    );
}

// ===========================================================================
// B2-ID-008: Rust `impl Trait for Type` must produce an Implements edge
// via the ImplFact mechanism (impl_item is never extracted as a symbol).
// ===========================================================================

#[test]
fn b2_id_008_rust_impl_trait_for_type_produces_an_implements_edge() {
    let source = CorpusSource::load("rust", &["impl_fact_trait_implementation.rs"]);
    let outcome = index(&source);

    let circle = find_one_by_name_in_file(&outcome, "Circle", "impl_fact_trait_implementation.rs");
    let shape = find_one_by_name_in_file(&outcome, "Shape", "impl_fact_trait_implementation.rs");
    let implements = outcome.graph.outgoing_edges(circle.id(), Some(EdgeKind::Implements));

    assert_eq!(implements.len(), 1, "Circle must implement exactly Shape, via ImplFact (B2-ID-008)");
    assert_eq!(implements[0].to, shape.id());
}

// ===========================================================================
// Known gap: Rust macro call opacity — documents the CURRENT gap.
// ===========================================================================

#[test]
fn rust_macro_call_opacity_is_the_current_documented_gap() {
    let source = CorpusSource::load("rust", &["macro_call_opacity_gap.rs"]);
    let outcome = index(&source);

    let uses_macro = find_one_by_name_in_file(&outcome, "uses_macro", "macro_call_opacity_gap.rs");
    let callees: Vec<String> = outcome.graph.outgoing_edges(uses_macro.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).map(|s| s.name().as_str().to_string()).collect();

    assert!(
        callees.is_empty(),
        "known gap: a call wrapped inside a macro invocation's argument list (assert_eq!(add(1,2), 3)) \
         is NOT found by resolve_calls (tree-sitter sees an opaque token_tree) -- if this ever fails, \
         a macro-aware heuristic has been built and this test must assert the real edge instead"
    );
}

// ===========================================================================
// B2-ID-009: C# method_declaration must extract its own name via the
// grammar's named `name` field, never confuse it with its return type.
// ===========================================================================

#[test]
fn b2_id_009_method_name_is_never_confused_with_its_return_type() {
    let source = CorpusSource::load("csharp", &["method_name_vs_return_type.cs"]);
    let outcome = index(&source);

    let handle = find_snapshot_by_name(&outcome, "Handle");
    assert_eq!(handle.len(), 1, "exactly one method must be named \"Handle\"");
    let bogus = find_snapshot_by_name(&outcome, "PolicyBuilder");
    assert!(bogus.iter().all(|s| s.kind() != codegraph_domain::SymbolKind::Method), "no METHOD must ever be misnamed \"PolicyBuilder\" after its own return type (B2-ID-009) -- a class legitimately named PolicyBuilder is fine");
}

// ===========================================================================
// B2-ID-010: C# partial class fragmentation must be reconciled for
// Extends/Implements resolution — the real headline BulkheadPolicy:Policy
// case, split across files.
// ===========================================================================

#[test]
fn b2_id_010_partial_class_fragments_reconcile_to_one_extends_edge() {
    let source = CorpusSource::load("csharp", &["partial_class_policy_a.cs", "partial_class_policy_b.cs", "partial_class_bulkhead.cs"]);
    let outcome = index(&source);

    let bulkhead = find_one_by_name_in_file(&outcome, "BulkheadPolicy", "partial_class_bulkhead.cs");
    let extends = outcome.graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Extends));

    assert_eq!(extends.len(), 1, "BulkheadPolicy:Policy must resolve to EXACTLY ONE Extends edge despite Policy's 2 partial fragments (B2-ID-010) -- zero means the bug regressed, more than one means the reconciliation itself regressed");
    let policy_fragments = find_snapshot_by_name(&outcome, "Policy");
    assert_eq!(policy_fragments.len(), 2, "Policy's own 2 fragments must remain distinct snapshots -- reconciliation never merges them");
    assert!(policy_fragments.iter().any(|p| p.id() == extends[0].to), "the Extends edge's target must be one of Policy's real fragments");
}

// ===========================================================================
// Known gap: generic-arity collision — documents the CURRENT gap
// (Moderate, out of B2-ID-010's own scope).
// ===========================================================================

#[test]
fn generic_arity_collision_is_the_current_documented_gap() {
    let source = CorpusSource::load("csharp", &["generic_arity_collision_gap.cs"]);
    let outcome = index(&source);

    let bulkhead = find_one_by_name_in_file(&outcome, "BulkheadPolicy", "generic_arity_collision_gap.cs");
    let implements = outcome.graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Implements));

    assert!(
        implements.is_empty(),
        "known gap: BulkheadPolicy:IBulkheadPolicy must stay AMBIGUOUS (no edge) because IBulkheadPolicy \
         and IBulkheadPolicy<TResult> share one bare name with no generic-arity-aware disambiguation -- \
         if this ever fails, the generic-arity gap has been fixed and this test must assert the real edge"
    );
}

// ===========================================================================
// B2-ID-011: C# generic method calls (bare and receiver-qualified) must
// be extracted and reach resolution, never silently dropped.
// ===========================================================================

#[test]
fn b2_id_011_csharp_generic_method_calls_are_extracted_and_resolved() {
    let source = CorpusSource::load("csharp", &["generic_method_call_extraction.cs"]);
    let outcome = index(&source);

    let bare_caller = find_one_by_name_in_file(&outcome, "BareGenericCall", "generic_method_call_extraction.cs");
    let bare_callees: Vec<String> = outcome.graph.outgoing_edges(bare_caller.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
    assert!(!bare_callees.is_empty(), "a bare generic call (Handle<System.Exception>()) must be extracted and resolved to SOME Handle candidate (B2-ID-011) -- empty means the generic call is still invisible");

    let qualified_caller = find_one_by_name_in_file(&outcome, "ReceiverQualifiedGenericCall", "generic_method_call_extraction.cs");
    let qualified_callees: Vec<String> = outcome.graph.outgoing_edges(qualified_caller.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
    assert!(!qualified_callees.is_empty(), "a receiver-qualified generic call (policy.Handle<System.Exception>()) must be extracted and resolved (B2-ID-011)");
}

// ===========================================================================
// B2-ID-011 (C++ half): template method calls must also be extracted,
// per the cross-language audit this fix's own review required.
// ===========================================================================

#[test]
fn b2_id_011_cpp_template_method_calls_are_extracted_and_resolved() {
    let source = CorpusSource::load("cpp", &["template_method_call_extraction.cpp"]);
    let outcome = index(&source);

    let bare_caller = find_one_by_name_in_file(&outcome, "bare_template_call", "template_method_call_extraction.cpp");
    let bare_callees: Vec<String> = outcome.graph.outgoing_edges(bare_caller.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).map(|s| s.name().as_str().to_string()).collect();
    assert!(bare_callees.contains(&"handle".to_string()), "a bare template call (handle<int>()) must resolve to the real handle method (B2-ID-011, C++ half)");

    let qualified_caller = find_one_by_name_in_file(&outcome, "qualified_template_call", "template_method_call_extraction.cpp");
    let qualified_callees: Vec<String> = outcome.graph.outgoing_edges(qualified_caller.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).map(|s| s.name().as_str().to_string()).collect();
    assert!(qualified_callees.contains(&"handle".to_string()), "a receiver-qualified template call (policy.handle<int>()) must resolve (B2-ID-011, C++ half)");
}

// ===========================================================================
// Semantic Resolution & Reliability Checkpoint, Priority 1: call-site
// generic-arity narrowing resolves a real same-name ambiguity when
// arity actually discriminates, and the required negative case stays
// genuinely Ambiguous when it does not (same name + same generic
// arity + differing ORDINARY parameter type) -- the exact real shape
// of Polly's own Handle<TException>() overload family.
// ===========================================================================

#[test]
fn arity_narrowing_resolves_a_genuine_arity_based_ambiguity() {
    let source = CorpusSource::load("csharp", &["generic_arity_narrowing_call_resolution.cs"]);
    let outcome = index(&source);

    let caller = find_one_by_name_in_file(&outcome, "ArityResolvesAmbiguity", "generic_arity_narrowing_call_resolution.cs");
    let callees: Vec<&codegraph_index::snapshot::SymbolSnapshot> = outcome.graph.outgoing_edges(caller.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).collect();
    assert_eq!(callees.len(), 1, "picker.Pick<System.Exception>() must resolve to EXACTLY ONE edge once narrowed by call-site generic arity (1) among the two same-named Pick/Pick<T> candidates");
    assert_eq!(callees[0].name().as_str(), "Pick");

    let snapshots: Vec<codegraph_index::snapshot::SymbolSnapshot> = outcome.graph.all_snapshots().into_iter().cloned().collect();
    let (_edges, attempts) = codegraph_index::edges::resolve_calls_with_negative_knowledge(&snapshots).expect("resolve_calls_with_negative_knowledge");
    let call_attempts_for_pick: Vec<_> = attempts.into_iter().filter(|a| a.callee_name == "Pick" && a.caller == caller.id()).collect();
    assert!(call_attempts_for_pick.is_empty(), "a call resolved via arity-narrowing must produce a real Edge, never also an Ambiguous/Unresolved CallAttempt for the same call site");
}

// ===========================================================================
// Semantic Resolution & Reliability Checkpoint, Priority 2: a real
// SignatureTooLong extraction rejection must link to the matching
// zero-candidate call site as UnresolvedReason::ExtractionRejected,
// never collapsing to the same Unknown reason a genuinely external
// (never-indexed) target would get. Permanent regression fixture for
// the real-corpus finding this checkpoint measured against
// App-vNext/Polly (213 real SignatureTooLong rejections there).
// ===========================================================================

#[test]
fn signature_too_long_rejection_links_to_extraction_rejected_reason() {
    let source = CorpusSource::load("csharp", &["signature_too_long_extraction_rejection_linkage.cs"]);
    let outcome = index(&source);

    let caller = find_one_by_name_in_file(&outcome, "InvokesRejectedTarget", "signature_too_long_extraction_rejection_linkage.cs");
    let callees: Vec<&codegraph_index::snapshot::SymbolSnapshot> = outcome.graph.outgoing_edges(caller.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).collect();
    assert!(callees.is_empty(), "VeryLongSignatureMethod's own candidate must have been dropped at extraction time (real SignatureTooLong), so no edge can exist for this call");

    let attempt = outcome
        .call_attempts
        .iter()
        .find(|a| a.callee_name == "VeryLongSignatureMethod" && a.caller == caller.id())
        .expect("a CallAttempt must exist for this call site through the real production pipeline (pipeline.rs/indexer.rs wiring)");
    assert_eq!(attempt.tier, codegraph_domain::EvidenceTier::Unresolved);
    assert_eq!(
        attempt.reason,
        codegraph_index::negative_knowledge::UnresolvedReason::ExtractionRejected { rejection_kind: "SignatureTooLong" },
        "the real production pipeline must link this zero-candidate call site to the SAME real extraction rejection \
         that dropped VeryLongSignatureMethod, not the generic Unknown reason a genuinely external target would get"
    );
}

// ===========================================================================
// Semantic Resolution & Reliability Checkpoint, Priority 6: real-case
// agent-policy re-check. Drives the SAME two arity-aware golden-corpus
// fixtures above through the real production pipeline, then checks
// codegraph_index::agent_policy::evaluate against the REAL resulting
// EvidenceTier for each case -- confirming the policy layer's own
// transitions are correct for cases this checkpoint's new resolution
// path actually produces, not merely asserted abstractly over
// synthetic EvidenceTier values (agent_policy.rs's own existing unit
// tests already cover the abstract tier-by-tier rules; this is the
// real-case integration check Priority 6 explicitly asks for).
// ===========================================================================

#[test]
fn arity_resolved_edge_is_heuristic_tier_and_the_policy_permits_it_only_with_disclosed_uncertainty() {
    let source = CorpusSource::load("csharp", &["generic_arity_narrowing_call_resolution.cs"]);
    let outcome = index(&source);

    let caller = find_one_by_name_in_file(&outcome, "ArityResolvesAmbiguity", "generic_arity_narrowing_call_resolution.cs");
    let edges = outcome.graph.outgoing_edges(caller.id(), Some(EdgeKind::Calls));
    assert_eq!(edges.len(), 1, "arity narrowing must have produced exactly one real Calls edge");
    let tier = codegraph_domain::EvidenceTier::from_resolution_method(edges[0].evidence.method());

    // The real, load-bearing transition this checkpoint's own Priority
    // 1 made possible: BEFORE arity narrowing existed, this call site
    // would have had 2+ same-named candidates and stayed Ambiguous
    // (Forbidden for every intended use, including mere display). The
    // real resolver now produces a Heuristic-tier edge instead -- more
    // permissive than Ambiguous/Forbidden, but still deliberately NOT
    // Resolved/fully Permitted, since arity narrowing is a syntactic
    // fact weaker than full overload resolution (see calls.rs's own
    // doc comment on this exact classification choice).
    assert_eq!(tier, codegraph_domain::EvidenceTier::Heuristic, "an arity-narrowed single-survivor edge must classify as Heuristic, never Resolved -- arity narrowing is strictly more than bare-name matching but not full type-aware overload resolution");

    let display_verdict = codegraph_index::agent_policy::evaluate(tier, codegraph_index::agent_policy::IntendedUse::InformationalDisplay);
    assert_eq!(display_verdict, codegraph_index::agent_policy::PolicyVerdict::PermittedWithDisclosedUncertainty, "informational display of an arity-resolved edge must disclose the uncertainty, never present it as certain fact");

    let high_impact_verdict = codegraph_index::agent_policy::evaluate(tier, codegraph_index::agent_policy::IntendedUse::HighImpactModification);
    assert!(matches!(high_impact_verdict, codegraph_index::agent_policy::PolicyVerdict::Forbidden { .. }), "a high-impact modification must still be Forbidden on a Heuristic-tier (arity-narrowed) edge alone -- it requires independent confirmation first, exactly like any other Heuristic edge");
}

#[test]
fn same_arity_ambiguity_produces_a_real_callattempt_the_policy_forbids_for_every_use() {
    let source = CorpusSource::load("csharp", &["generic_arity_narrowing_call_resolution.cs"]);
    let outcome = index(&source);

    let caller = find_one_by_name_in_file(&outcome, "SameArityStillAmbiguous", "generic_arity_narrowing_call_resolution.cs");
    let edges = outcome.graph.outgoing_edges(caller.id(), Some(EdgeKind::Calls));
    assert!(edges.is_empty(), "the required negative case must still produce no edge");

    let attempt = outcome
        .call_attempts
        .iter()
        .find(|a| a.callee_name == "Choose" && a.caller == caller.id())
        .expect("a real CallAttempt must exist for this call site through the real production pipeline");
    assert_eq!(attempt.tier, codegraph_domain::EvidenceTier::Ambiguous, "same generic arity with differing ordinary parameter types must stay genuinely Ambiguous, never silently narrowed to Heuristic/Resolved just because SOME arity information was available");

    // Arity narrowing correctly did NOT resolve this to false certainty
    // -- the real downstream policy consequence must be the SAME
    // Forbidden-for-every-use verdict as any other genuine ambiguity,
    // confirmed here against this checkpoint's own real new case, not
    // only the pre-existing synthetic EvidenceTier::Ambiguous unit
    // tests in agent_policy.rs itself.
    for intended_use in [
        codegraph_index::agent_policy::IntendedUse::InformationalDisplay,
        codegraph_index::agent_policy::IntendedUse::AssertAsFactOrSoleJustification,
        codegraph_index::agent_policy::IntendedUse::HighImpactModification,
    ] {
        assert!(
            matches!(codegraph_index::agent_policy::evaluate(attempt.tier, intended_use), codegraph_index::agent_policy::PolicyVerdict::Forbidden { .. }),
            "a real same-arity-ambiguity CallAttempt must be Forbidden for intended use {intended_use:?}, including mere informational display"
        );
    }
}

#[test]
fn same_generic_arity_with_differing_ordinary_parameter_type_stays_ambiguous() {
    let source = CorpusSource::load("csharp", &["generic_arity_narrowing_call_resolution.cs"]);
    let outcome = index(&source);

    let caller = find_one_by_name_in_file(&outcome, "SameArityStillAmbiguous", "generic_arity_narrowing_call_resolution.cs");
    let callees: Vec<&codegraph_index::snapshot::SymbolSnapshot> = outcome.graph.outgoing_edges(caller.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).collect();
    assert!(
        callees.is_empty(),
        "required negative case: picker.Choose<System.Exception>(1) has TWO same-arity (both generic arity 1) candidates \
         differing only in ordinary parameter type (int vs string) -- arity narrowing has no discriminating power here, \
         so this must stay genuinely Ambiguous (no edge), exactly like the real Polly Handle<TException>() family \
         (B2-EVAL-006/008: 0 of 846 real Handle call sites resolve via arity narrowing, for this same structural reason)"
    );
}

// ===========================================================================
// Known gap: C++ namespace-qualified calls — the NEW residual limitation
// B2-ID-011's own independent review found. Documents the CURRENT gap.
// ===========================================================================

#[test]
fn cpp_namespace_qualified_call_is_the_current_documented_gap() {
    let source = CorpusSource::load("cpp", &["namespace_qualified_call_gap.cpp"]);
    let outcome = index(&source);

    let caller = find_one_by_name_in_file(&outcome, "caller", "namespace_qualified_call_gap.cpp");
    let callees: Vec<String> = outcome.graph.outgoing_edges(caller.id(), Some(EdgeKind::Calls)).iter().filter_map(|e| outcome.graph.snapshot(e.to)).map(|s| s.name().as_str().to_string()).collect();

    assert!(
        callees.is_empty(),
        "known gap (found during B2-ID-011's own independent review): a namespace-qualified call \
         (ns::foo()) is invisible to extraction, orthogonal to generic/template calls -- if this \
         ever fails, this gap has been fixed and this test must assert the real edge"
    );
}

// ===========================================================================
// CRC-002: prop-shadowing false positive — documents the CURRENT gap
// (a confident but WRONG edge is produced, not an absent one).
// ===========================================================================

#[test]
fn crc_002_prop_shadowing_false_positive_is_the_current_documented_gap() {
    let source = CorpusSource::load("typescript", &["crc_002_prop_shadowing_false_positive.tsx"]);
    let outcome = index(&source);

    let child = find_one_by_name_in_file(&outcome, "Child", "crc_002_prop_shadowing_false_positive.tsx");
    let top_level_handle_submit = find_one_by_name_in_file(&outcome, "handleSubmit", "crc_002_prop_shadowing_false_positive.tsx");
    let callees = outcome.graph.outgoing_edges(child.id(), Some(EdgeKind::Calls));

    assert!(
        callees.iter().any(|e| e.to == top_level_handle_submit.id()),
        "CRC-002's known gap: Child's own call to its DESTRUCTURED PARAMETER handleSubmit is \
         currently (wrongly) resolved to the unrelated top-level handleSubmit function, since this \
         pipeline has no local-binding/shadowing check -- if this assertion ever fails, CRC-002 has \
         been fixed (the edge should be ABSENT, not present) and this test must be updated"
    );
}

// ===========================================================================
// Phase 2, deliverable 9: golden corpus + ground truth schema +
// reliability model integration. This is the one place all three meet:
// the SAME real fixture (generic_arity_collision_gap.cs) that the gap
// test above drives through the real pipeline also has a real
// ground-truth row (tests/ground_truth_fixtures/b2-eval-008-extends-
// implements.jsonl, row 5) recording the EvidenceTier this case should
// resolve to. This test proves those two artifacts agree with each
// other and with the real resolver's own current output, rather than
// letting the ground-truth fixture silently drift from what the code
// actually does.
// ===========================================================================

#[test]
fn golden_corpus_ground_truth_and_the_real_resolver_agree_on_the_generic_arity_collision_case() {
    let source = CorpusSource::load("csharp", &["generic_arity_collision_gap.cs"]);
    let outcome = index(&source);
    let bulkhead = find_one_by_name_in_file(&outcome, "BulkheadPolicy", "generic_arity_collision_gap.cs");
    let real_implements_edges = outcome.graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Implements));
    assert!(real_implements_edges.is_empty(), "the real pipeline must still produce no Implements edge for this known-ambiguous case");

    let contents = std::fs::read_to_string("tests/ground_truth_fixtures/b2-eval-008-extends-implements.jsonl").expect("the retrofitted B2-EVAL-008 ground-truth fixture must exist");
    let rows = codegraph_index::ground_truth::parse_jsonl(&contents).expect("the fixture must parse cleanly");
    let ambiguous_row = rows
        .iter()
        .find(|r| r.caller == "src/Polly/Bulkhead/BulkheadPolicy.cs::BulkheadPolicy" && r.expected_targets.len() >= 2)
        .expect("a ground-truth row recording the generic-arity-collision ambiguity must exist in the real fixture");

    assert_eq!(
        ambiguous_row.resolution_method,
        Some(codegraph_index::ground_truth::EvidenceTierField::Ambiguous),
        "the ground-truth row's own recorded resolution_method must be Ambiguous, matching the real EvidenceTier this case resolves to"
    );
    assert!(ambiguous_row.predicted_targets.is_empty(), "the ground-truth row's own predicted_targets must mirror the real resolver producing no edge");
    assert_eq!(
        ambiguous_row.compute_status(),
        codegraph_index::ground_truth::Status::AmbiguousCorrect,
        "a genuinely ambiguous case with no predicted edge is AmbiguousCorrect per the schema's own status table -- never a silent FalseNegative for the resolver correctly declining to guess"
    );
}
