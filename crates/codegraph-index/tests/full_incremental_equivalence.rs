//! Quality & Semantic Reliability Hardening, Phase 1 — full/incremental
//! exact-equivalence gate.
//!
//! # Invariant under test
//!
//! For the SAME repository state at commit X:
//!
//! `FullIndex(X) == IncrementalIndex(prior -> X)`
//!
//! on the EXACT snapshot set, the EXACT typed-edge set, and the EXACT
//! `ImportFact`/`ImplFact` sets — never merely their counts. Two
//! indexes could have the same snapshot COUNT while disagreeing on
//! which symbols those snapshots actually are (a real risk this
//! project's own history has already hit once: B2-ID-010's final-
//! review fix wave found a Critical bug where ordinary incremental
//! edits silently dropped `Implements` edges that a full reindex of
//! the identical commit still produced — a count-only comparison would
//! not have caught that if the dropped edge happened to be replaced by
//! an unrelated extra one).
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
//! it can be shown to actually FAIL when the two really do diverge —
//! `the_equivalence_helper_itself_detects_an_injected_divergence` below
//! deliberately builds two `ObserveOutcome`s that are NOT equivalent
//! (one extra unrelated snapshot inserted into one side's own commit)
//! and asserts the helper's own comparison reports a real mismatch,
//! not a false "equivalent."

use std::collections::{HashMap, HashSet};

use codegraph_domain::{CommitHash, EdgeKind, FileIdentity, RepositoryId, RepositoryRegistry, RepositoryRoot, RepositorySource, SourceError, SymbolSnapshotId};
use codegraph_index::incremental::{CodeGraphIndexer, ObserveOutcome};

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

/// A semantic (not structural) snapshot comparison key: name + kind +
/// file + span. Deliberately NOT `SymbolSnapshotId` alone — two
/// snapshots with the same id are trivially "the same," but this gate
/// exists to catch the SAME semantic symbol ending up with two
/// different representations (or going missing) between the two
/// paths, which comparing raw ids could mask if a bug happened to
/// change the id-derivation inputs identically on both paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SnapshotKey {
    name: String,
    kind: String,
    file: String,
    span: (usize, usize),
}

fn snapshot_keys(outcome: &ObserveOutcome) -> HashSet<SnapshotKey> {
    outcome
        .graph
        .all_snapshots()
        .into_iter()
        .filter(|s| outcome.graph.lineage_from(s.id()).is_empty()) // current, non-superseded only
        .map(|s| SnapshotKey {
            name: s.name().as_str().to_string(),
            kind: format!("{:?}", s.kind()),
            file: s.file().relative_path().to_string(),
            span: (s.span().start, s.span().end),
        })
        .collect()
}

/// A semantic edge comparison key: (from-symbol-key, to-symbol-key,
/// kind). Deliberately resolved through each endpoint's OWN semantic
/// key (name/kind/file/span), not raw `SymbolSnapshotId` — for the
/// identical reason `SnapshotKey` above is semantic, not structural.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EdgeKeyTuple {
    from: SnapshotKey,
    to: SnapshotKey,
    kind: EdgeKind,
}

fn edge_keys(outcome: &ObserveOutcome) -> HashSet<EdgeKeyTuple> {
    let key_of = |id: SymbolSnapshotId| -> Option<SnapshotKey> {
        outcome.graph.snapshot(id).map(|s| SnapshotKey {
            name: s.name().as_str().to_string(),
            kind: format!("{:?}", s.kind()),
            file: s.file().relative_path().to_string(),
            span: (s.span().start, s.span().end),
        })
    };
    outcome
        .graph
        .all_edges()
        .into_iter()
        .filter_map(|e| Some(EdgeKeyTuple { from: key_of(e.from)?, to: key_of(e.to)?, kind: e.kind }))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ImportFactKey {
    importing_file: String,
    module_path: String,
    imported_symbol: Option<String>,
    alias: Option<String>,
}

fn import_fact_keys(outcome: &ObserveOutcome) -> HashSet<ImportFactKey> {
    outcome
        .import_facts
        .iter()
        .map(|f| ImportFactKey {
            importing_file: f.importing_file.relative_path().to_string(),
            module_path: f.module_path.clone(),
            imported_symbol: f.imported_symbol.clone(),
            alias: f.alias.clone(),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ImplFactKey {
    defining_file: String,
    self_type_text: String,
    trait_type_text: Option<String>,
}

fn impl_fact_keys(outcome: &ObserveOutcome) -> HashSet<ImplFactKey> {
    outcome
        .impl_facts
        .iter()
        .map(|f| ImplFactKey {
            defining_file: f.defining_file.relative_path().to_string(),
            self_type_text: f.self_type_text.clone(),
            trait_type_text: f.trait_type_text.clone(),
        })
        .collect()
}

/// The reusable equivalence assertion: compares a "full reindex at
/// commit B" outcome against an "incremental advance A->B" outcome on
/// snapshot set, typed-edge set, `ImportFact` set, and `ImplFact` set
/// — all EXACT set equality, never counts. Panics with a readable
/// diff (not just "not equal") on the first mismatched dimension.
fn assert_full_equals_incremental(full: &ObserveOutcome, incremental: &ObserveOutcome, context: &str) {
    let full_snapshots = snapshot_keys(full);
    let incremental_snapshots = snapshot_keys(incremental);
    if full_snapshots != incremental_snapshots {
        let only_in_full: Vec<_> = full_snapshots.difference(&incremental_snapshots).collect();
        let only_in_incremental: Vec<_> = incremental_snapshots.difference(&full_snapshots).collect();
        panic!("{context}: snapshot sets diverge.\n  only in full: {only_in_full:#?}\n  only in incremental: {only_in_incremental:#?}");
    }

    let full_edges = edge_keys(full);
    let incremental_edges = edge_keys(incremental);
    if full_edges != incremental_edges {
        let only_in_full: Vec<_> = full_edges.difference(&incremental_edges).collect();
        let only_in_incremental: Vec<_> = incremental_edges.difference(&full_edges).collect();
        panic!("{context}: typed edge sets diverge.\n  only in full: {only_in_full:#?}\n  only in incremental: {only_in_incremental:#?}");
    }

    let full_imports = import_fact_keys(full);
    let incremental_imports = import_fact_keys(incremental);
    assert_eq!(full_imports, incremental_imports, "{context}: ImportFact sets diverge");

    let full_impls = impl_fact_keys(full);
    let incremental_impls = impl_fact_keys(incremental);
    assert_eq!(full_impls, incremental_impls, "{context}: ImplFact sets diverge");
}

// ===========================================================================
// Self-test: the helper above must actually DETECT a real divergence,
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
