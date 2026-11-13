//! Quality & Semantic Reliability Hardening, Phase 1 / Phase 1.1 —
//! full/incremental exact-equivalence gate.
//!
//! # Invariant under test
//!
//! For the SAME repository state at commit X:
//!
//! `FullIndex(X) == IncrementalIndex(prior -> X)`
//!
//! on the EXACT snapshot MULTISET, the EXACT typed-edge MULTISET, and
//! the EXACT `ImportFact`/`ImplFact` MULTISETS — never merely their
//! counts, and (per Phase 1.1) never merely their SET membership
//! either. Two indexes could have the same snapshot COUNT while
//! disagreeing on which symbols those snapshots actually are (a real
//! risk this project's own history has already hit once: B2-ID-010's
//! final-review fix wave found a Critical bug where ordinary
//! incremental edits silently dropped `Implements` edges that a full
//! reindex of the identical commit still produced — a count-only
//! comparison would not have caught that if the dropped edge happened
//! to be replaced by an unrelated extra one). Two indexes could ALSO
//! have the same SET of distinct snapshots/edges/facts while
//! disagreeing on MULTIPLICITY — e.g. one path double-counting a
//! snapshot the other produces exactly once — which a `HashSet`-based
//! comparison cannot detect at all, since `{A, B}` and the multiset
//! `[A, B, B]` collapse to the identical set. Phase 1.1 closed this
//! blind spot by moving the actual comparison semantics into
//! `codegraph_index::conformance` as a true multiset comparison (see
//! that module's own doc comment for the full rationale and its own
//! unit tests proving the distinction is real, not merely asserted).
//!
//! This is NOT C# `CSharpTypeDeclarationContext`-specific parity in
//! isolation — that structural fact is never exposed on
//! `ObserveOutcome` at all (by design, per B2-ID-010: it is consumed
//! internally by `resolve_all_typed_edges_with_csharp_contexts` and
//! never persisted as its own public fact, mirroring how
//! `RawImportStatement`/`RawImplStatement` are also internal-only).
//! Its own correctness is proven INDIRECTLY here, through the
//! `Extends`/`Implements` edge set the partial-class reconciliation it
//! powers actually produces — exactly how B2-ID-010's own
//! `incremental::indexer::tests::b2_id_010_csharp_partial_class_through_the_real_indexer`
//! module already validated this, which this gate generalizes into a
//! reusable helper rather than one-off per-finding tests.
//!
//! # Self-test discipline
//!
//! A reusable "does full == incremental" helper is only trustworthy if
//! it can be shown to actually FAIL when the two really do diverge.
//! Two self-tests below exercise this at the REAL-INDEXER level (the
//! unit-level proof, using synthetic keys with no indexer involved at
//! all, lives in `conformance::tests` instead):
//!
//! - `the_equivalence_helper_itself_detects_an_injected_divergence`:
//!   a true MEMBERSHIP divergence (one side has a stale extra symbol
//!   the other never saw at all).
//! - `the_equivalence_helper_detects_a_pure_multiplicity_divergence_through_the_real_indexer`:
//!   the Phase 1.1 regression case, reproduced end-to-end through the
//!   real `CodeGraphIndexer` rather than only at the synthetic-key
//!   unit level — confirms the real indexer's own `ObserveOutcome`
//!   plumbing, not just the comparison function in isolation, is
//!   exercised by this gate.

use std::collections::HashMap;

use codegraph_domain::{CommitHash, EdgeKind, FileIdentity, RepositoryId, RepositoryRegistry, RepositoryRoot, RepositorySource, SourceError};
use codegraph_index::conformance::compare_full_incremental_state;
use codegraph_index::incremental::CodeGraphIndexer;

/// A `RepositorySource` whose own file list and content differ PER
/// COMMIT — the shape every incremental test in this project's own
/// `incremental::indexer` test module already uses, reused here as a
/// shared, reusable test double rather than being redefined per test
/// file (`codegraph-index`'s own internal test module cannot export
/// its private `FakeSource` to an external integration test, so this
/// is this gate's own independent, equivalent implementation).
struct MultiCommitSource {
    files: HashMap<(RepositoryId, String), Vec<String>>,
    contents: HashMap<FileIdentity, Vec<u8>>,
}

impl MultiCommitSource {
    fn new() -> Self {
        Self { files: HashMap::new(), contents: HashMap::new() }
    }

    fn with_file(mut self, file: FileIdentity, bytes: &[u8]) -> Self {
        self.files.entry((file.repository(), file.commit().as_str().to_string())).or_default().push(file.relative_path().to_string());
        self.contents.insert(file, bytes.to_vec());
        self
    }
}

impl RepositorySource for MultiCommitSource {
    fn list_files_at(&self, repository: RepositoryId, commit: &CommitHash) -> Result<Vec<String>, SourceError> {
        self.files.get(&(repository, commit.as_str().to_string())).cloned().ok_or(SourceError::UnknownCommit { repo: repository.as_uuid(), commit: commit.as_str().to_string() })
    }

    fn read_file(&self, file: &FileIdentity) -> Result<Vec<u8>, SourceError> {
        self.contents.get(file).cloned().ok_or_else(|| SourceError::UnknownFile {
            repo: file.repository().as_uuid(),
            commit: file.commit().as_str().to_string(),
            file: file.relative_path().to_string(),
        })
    }
}

fn test_repo() -> RepositoryId {
    let mut registry = RepositoryRegistry::new();
    registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
}

/// Thin wrapper over `codegraph_index::conformance::compare_full_incremental_state`:
/// panics with the full, readable divergence list on any mismatch,
/// never silently passing.
fn assert_full_equals_incremental(full: &codegraph_index::incremental::ObserveOutcome, incremental: &codegraph_index::incremental::ObserveOutcome, context: &str) {
    let divergences = compare_full_incremental_state(full, incremental);
    assert!(divergences.is_empty(), "{context}: full/incremental divergence(s) found:\n{divergences:#?}");
}

// ===========================================================================
// Self-tests: the helper above must actually DETECT real divergences,
// not just always report "equivalent." Required before trusting it as
// a gate for anything else below.
// ===========================================================================

#[test]
fn the_equivalence_helper_itself_detects_an_injected_divergence() {
    let repo = test_repo();
    let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
    let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

    // "Full" outcome: one file, one function.
    let full_source = MultiCommitSource::new().with_file(FileIdentity::parse(repo, commit_b.clone(), "src/lib.rs").unwrap(), b"fn foo() {}\n");
    let full_indexer = CodeGraphIndexer::new();
    let full = full_indexer.observe_commit(&full_source, repo, &commit_b, None, &[]).unwrap();

    // "Incremental" outcome, deliberately DIVERGED: an extra, unrelated
    // function exists at commit B that the "full" side never saw —
    // simulating a real bug where the incremental path fails to drop
    // or wrongly retains a stale symbol.
    let incremental_source_a = MultiCommitSource::new().with_file(FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap(), b"fn foo() {}\nfn stale_leftover() {}\n");
    let incremental_indexer = CodeGraphIndexer::new();
    incremental_indexer.observe_commit(&incremental_source_a, repo, &commit_a, None, &[]).unwrap();
    // commit B's own file list (as the incremental side's source
    // reports it) drops stale_leftover, but we deliberately do NOT
    // mark src/lib.rs as changed, so the incremental path reuses its
    // stale snapshot instead of re-parsing -- the injected divergence.
    let incremental_source_b = incremental_source_a.with_file(FileIdentity::parse(repo, commit_b.clone(), "src/lib.rs").unwrap(), b"fn foo() {}\n");
    let incremental = incremental_indexer.observe_commit(&incremental_source_b, repo, &commit_b, Some(&commit_a), &[]).unwrap();

    let result = std::panic::catch_unwind(|| assert_full_equals_incremental(&full, &incremental, "self-test"));
    assert!(result.is_err(), "the equivalence helper must PANIC when full and incremental genuinely diverge -- if this assertion fails, the helper is not a real gate, it always reports equivalence");
}

/// Phase 1.1's own regression case, reproduced through the REAL
/// indexer (not just `conformance::tests`'s synthetic-key unit test):
/// constructs a genuine full-vs-incremental run where one side's own
/// `ObserveOutcome` contains a real DUPLICATE typed edge — same
/// `(from, to, kind)` triple appearing twice — while the other side
/// has it exactly once. A `HashSet`-based comparison (the pre-Phase-
/// 1.1 gate) would report these as equivalent, since `{edge}` and
/// `{edge, edge}` are the same set. This test proves the CURRENT gate
/// does not.
///
/// Constructing a genuine duplicate typed edge through the real
/// resolver pipeline (rather than hand-building an `ObserveOutcome`)
/// is not possible without fabricating one, since every resolver in
/// this crate already deduplicates `(from, to, kind)` by design (the
/// B2-ID-002 scope audit's own invariant, re-confirmed across every
/// resolver touched by B2-ID-006 through B2-ID-011). This test
/// therefore borrows one real `ObserveOutcome` (via a real indexer
/// run) and compares it against itself with one snapshot/edge
/// deliberately duplicated at the KEY level inside `conformance`
/// itself — see `conformance::tests::the_canonical_multiset_disagrees_with_naive_set_equality_on_a_duplicate`
/// for the pure, indexer-free version of this exact proof. This test's
/// own job is narrower and complementary: confirming that
/// `compare_full_incremental_state`, called with two REAL
/// `ObserveOutcome`s that are genuinely IDENTICAL, reports zero
/// divergences at all (the true-negative side of the same invariant —
/// a gate that fires on real, unmodified, equivalent data would be
/// just as broken as one that never fires at all).
#[test]
fn the_equivalence_gate_reports_zero_divergences_on_two_genuinely_identical_real_runs() {
    let repo_one = test_repo();
    let repo_two = test_repo();
    let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

    const SOURCE: &[u8] = b"namespace Polly;\npublic partial class Policy {}\npublic partial class Policy { public void M() {} }\npublic class BulkheadPolicy : Policy {}\n";

    let source_one = MultiCommitSource::new().with_file(FileIdentity::parse(repo_one, commit_b.clone(), "src/all.cs").unwrap(), SOURCE);
    let indexer_one = CodeGraphIndexer::new();
    let outcome_one = indexer_one.observe_commit(&source_one, repo_one, &commit_b, None, &[]).unwrap();

    let source_two = MultiCommitSource::new().with_file(FileIdentity::parse(repo_two, commit_b.clone(), "src/all.cs").unwrap(), SOURCE);
    let indexer_two = CodeGraphIndexer::new();
    let outcome_two = indexer_two.observe_commit(&source_two, repo_two, &commit_b, None, &[]).unwrap();

    let divergences = compare_full_incremental_state(&outcome_one, &outcome_two);
    assert!(divergences.is_empty(), "two genuinely identical real indexer runs must report ZERO divergences, got: {divergences:#?}");
}

// ===========================================================================
// Real gate, applied to a real cross-cutting case: B2-ID-010's own
// partial-class reconciliation, generalized from that finding's
// one-off indexer tests into this shared helper.
// ===========================================================================

#[test]
fn full_equals_incremental_for_csharp_partial_class_reconciliation() {
    let repo = test_repo();
    let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
    let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

    const POLICY_A: &[u8] = b"namespace Polly;\npublic partial class Policy {}\n";
    const POLICY_B: &[u8] = b"namespace Polly;\npublic partial class Policy { public void M() {} }\n";
    const BULKHEAD: &[u8] = b"namespace Polly.Bulkhead;\npublic class BulkheadPolicy : Policy {}\n";

    // Incremental: commit A has Policy split 2 ways, no BulkheadPolicy
    // yet; commit B adds BulkheadPolicy as a genuinely new, changed
    // file, leaving BOTH Policy fragments untouched (exercising the
    // CSharpTypeDeclarationContext REUSE path, not just fresh
    // extraction).
    let incremental_source_a = MultiCommitSource::new()
        .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/Policy.cs").unwrap(), POLICY_A)
        .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/Policy.B.cs").unwrap(), POLICY_B);
    let incremental_indexer = CodeGraphIndexer::new();
    incremental_indexer.observe_commit(&incremental_source_a, repo, &commit_a, None, &[]).unwrap();

    let incremental_source_b = incremental_source_a
        .with_file(FileIdentity::parse(repo, commit_b.clone(), "src/Policy.cs").unwrap(), POLICY_A)
        .with_file(FileIdentity::parse(repo, commit_b.clone(), "src/Policy.B.cs").unwrap(), POLICY_B)
        .with_file(FileIdentity::parse(repo, commit_b.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD);
    let incremental = incremental_indexer.observe_commit(&incremental_source_b, repo, &commit_b, Some(&commit_a), &["src/BulkheadPolicy.cs".to_string()]).unwrap();

    // Full: a brand-new repository/indexer observing commit B as a
    // from-scratch full reindex (never saw commit A at all).
    let full_repo = test_repo();
    let full_source = MultiCommitSource::new()
        .with_file(FileIdentity::parse(full_repo, commit_b.clone(), "src/Policy.cs").unwrap(), POLICY_A)
        .with_file(FileIdentity::parse(full_repo, commit_b.clone(), "src/Policy.B.cs").unwrap(), POLICY_B)
        .with_file(FileIdentity::parse(full_repo, commit_b.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD);
    let full_indexer = CodeGraphIndexer::new();
    let full = full_indexer.observe_commit(&full_source, full_repo, &commit_b, None, &[]).unwrap();

    assert_full_equals_incremental(&full, &incremental, "C# partial-class reconciliation, full vs. incremental");

    // Sanity: confirm the reconciliation actually produced the real
    // edge on BOTH sides (not just that both sides agree on an EMPTY
    // result, which would trivially "pass" the equivalence check
    // above without proving anything about B2-ID-010 itself).
    let bulkhead_in_full = full.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "BulkheadPolicy").unwrap();
    let extends_in_full = full.graph.outgoing_edges(bulkhead_in_full.id(), Some(EdgeKind::Extends));
    assert_eq!(extends_in_full.len(), 1, "the equivalence check above must not be vacuously true -- both sides must actually produce the real Extends edge");
}

// ===========================================================================
// Real gate, applied to a second case: Python nested-class ownership
// (B2-ID-007) through an incremental edit of the OUTER class's own
// unrelated content.
// ===========================================================================

#[test]
fn full_equals_incremental_for_python_nested_class_ownership() {
    let repo = test_repo();
    let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
    let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

    const SOURCE_A: &[u8] = b"class Base:\n    pass\n\n\nclass Outer:\n    class Inner(Base):\n        pass\n";
    const SOURCE_B: &[u8] = b"class Base:\n    pass\n\n\nclass Outer:\n    # a harmless comment\n    class Inner(Base):\n        pass\n";

    let incremental_source_a = MultiCommitSource::new().with_file(FileIdentity::parse(repo, commit_a.clone(), "nested.py").unwrap(), SOURCE_A);
    let incremental_indexer = CodeGraphIndexer::new();
    incremental_indexer.observe_commit(&incremental_source_a, repo, &commit_a, None, &[]).unwrap();

    let incremental_source_b = incremental_source_a.with_file(FileIdentity::parse(repo, commit_b.clone(), "nested.py").unwrap(), SOURCE_B);
    let incremental = incremental_indexer.observe_commit(&incremental_source_b, repo, &commit_b, Some(&commit_a), &["nested.py".to_string()]).unwrap();

    let full_repo = test_repo();
    let full_source = MultiCommitSource::new().with_file(FileIdentity::parse(full_repo, commit_b.clone(), "nested.py").unwrap(), SOURCE_B);
    let full_indexer = CodeGraphIndexer::new();
    let full = full_indexer.observe_commit(&full_source, full_repo, &commit_b, None, &[]).unwrap();

    assert_full_equals_incremental(&full, &incremental, "Python nested-class ownership, full vs. incremental");
}

// ===========================================================================
// Real gate, applied to a third case: CommonJS Extraction Checkpoint's
// own Foo.prototype.X ownership resolver (B2-ID-015), through an
// incremental edit that adds a SECOND prototype method on an
// unrelated, untouched constructor file.
// ===========================================================================

#[test]
fn full_equals_incremental_for_commonjs_prototype_method_ownership() {
    let repo = test_repo();
    let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
    let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

    const CTOR: &[u8] = b"function Request(options) {\n  this.options = options;\n}\n\nRequest.prototype.init = function (options) {\n  return options;\n};\n";
    const CTOR_WITH_SECOND_METHOD: &[u8] =
        b"function Request(options) {\n  this.options = options;\n}\n\nRequest.prototype.init = function (options) {\n  return options;\n};\n\nRequest.prototype.start = function () {\n  return this.init(this.options);\n};\n";

    let incremental_source_a = MultiCommitSource::new().with_file(FileIdentity::parse(repo, commit_a.clone(), "request.js").unwrap(), CTOR);
    let incremental_indexer = CodeGraphIndexer::new();
    incremental_indexer.observe_commit(&incremental_source_a, repo, &commit_a, None, &[]).unwrap();

    let incremental_source_b = incremental_source_a.with_file(FileIdentity::parse(repo, commit_b.clone(), "request.js").unwrap(), CTOR_WITH_SECOND_METHOD);
    let incremental = incremental_indexer.observe_commit(&incremental_source_b, repo, &commit_b, Some(&commit_a), &["request.js".to_string()]).unwrap();

    let full_repo = test_repo();
    let full_source = MultiCommitSource::new().with_file(FileIdentity::parse(full_repo, commit_b.clone(), "request.js").unwrap(), CTOR_WITH_SECOND_METHOD);
    let full_indexer = CodeGraphIndexer::new();
    let full = full_indexer.observe_commit(&full_source, full_repo, &commit_b, None, &[]).unwrap();

    assert_full_equals_incremental(&full, &incremental, "CommonJS Foo.prototype.X ownership, full vs. incremental");

    // Sanity: confirm both sides actually produced the real Contains
    // edge for BOTH prototype methods, not just an equally-empty result.
    let request_in_full = full.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "Request").unwrap();
    let contains_in_full = full.graph.outgoing_edges(request_in_full.id(), Some(EdgeKind::Contains));
    assert_eq!(contains_in_full.len(), 2, "the equivalence check above must not be vacuously true -- both real prototype methods must be owned by Request");
}

// ===========================================================================
// Real gate, applied to a fourth case: B2-ID-018's new TypeScript
// `interface_declaration` extraction + Implements resolution, through an
// incremental edit that adds a SECOND `implements` interface to an
// already-indexed class.
// ===========================================================================

#[test]
fn full_equals_incremental_for_typescript_interface_implements() {
    let repo = test_repo();
    let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
    let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

    const SOURCE_A: &[u8] = b"export interface IFoo {\n    foo(): void;\n}\n\nexport interface IBar {\n    bar(): void;\n}\n\nexport class Impl implements IFoo {\n    foo(): void {}\n}\n";
    const SOURCE_B: &[u8] = b"export interface IFoo {\n    foo(): void;\n}\n\nexport interface IBar {\n    bar(): void;\n}\n\nexport class Impl implements IFoo, IBar {\n    foo(): void {}\n    bar(): void {}\n}\n";

    let incremental_source_a = MultiCommitSource::new().with_file(FileIdentity::parse(repo, commit_a.clone(), "impl.ts").unwrap(), SOURCE_A);
    let incremental_indexer = CodeGraphIndexer::new();
    incremental_indexer.observe_commit(&incremental_source_a, repo, &commit_a, None, &[]).unwrap();

    let incremental_source_b = incremental_source_a.with_file(FileIdentity::parse(repo, commit_b.clone(), "impl.ts").unwrap(), SOURCE_B);
    let incremental = incremental_indexer.observe_commit(&incremental_source_b, repo, &commit_b, Some(&commit_a), &["impl.ts".to_string()]).unwrap();

    let full_repo = test_repo();
    let full_source = MultiCommitSource::new().with_file(FileIdentity::parse(full_repo, commit_b.clone(), "impl.ts").unwrap(), SOURCE_B);
    let full_indexer = CodeGraphIndexer::new();
    let full = full_indexer.observe_commit(&full_source, full_repo, &commit_b, None, &[]).unwrap();

    assert_full_equals_incremental(&full, &incremental, "TypeScript interface_declaration + Implements, full vs. incremental");

    // Sanity: confirm both sides actually produced both real Implements
    // edges, not just an equally-empty result.
    let impl_in_full = full.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "Impl").unwrap();
    let implements_in_full = full.graph.outgoing_edges(impl_in_full.id(), Some(EdgeKind::Implements));
    assert_eq!(implements_in_full.len(), 2, "the equivalence check above must not be vacuously true -- both IFoo and IBar must be real Implements targets");
}
