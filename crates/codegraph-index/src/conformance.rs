//! Quality & Semantic Reliability Hardening, Phase 1.1 — the
//! correctness-critical CORE of the full/incremental exact-equivalence
//! gate, moved here (out of `tests/full_incremental_equivalence.rs`)
//! specifically so `cargo-mutants` can exercise it: this crate's own
//! mutation-testing policy (`docs/research/MUTATION_TESTING_POLICY.md`)
//! documents that `cargo-mutants` produces ZERO mutants for anything
//! under `tests/` by default (no `#[cfg(test)]`/integration-test
//! binary is instrumented), so a comparison helper that only ever
//! lived in an integration test file had no mutation coverage at all
//! — confirmed during Phase 1's own independent review. This module is
//! the minimal semantic core; the integration test
//! (`tests/full_incremental_equivalence.rs`) now calls it instead of
//! re-implementing the comparison inline.
//!
//! # Multiset, not set — the exact bug this module exists to fix
//!
//! The PREVIOUS version of this gate compared snapshot/edge/fact
//! collections via `HashSet` equality. A `HashSet` collapses
//! duplicates: `[A, B]` and `[A, B, B]` produce the SAME set
//! (`{A, B}`), so a real bug where the incremental path double-counts
//! or double-promotes one snapshot/edge/fact — while the full-reindex
//! path produces it exactly once — would be silently invisible to a
//! set-based comparison. This module compares via a SORTED `Vec` of
//! canonical keys instead: two collections are equal only if they have
//! the same elements in the same MULTIPLICITY, not merely the same
//! membership. `the_canonical_multiset_disagrees_with_naive_set_equality_on_a_duplicate`
//! (this module's own unit test, below) proves this distinction is
//! real on the exact `[A,B]` vs. `[A,B,B]` shape — not merely asserted.

use std::collections::HashSet;

use codegraph_domain::EdgeKind;

use crate::incremental::ObserveOutcome;

/// A semantic (not structural) snapshot comparison key: name + kind +
/// file + span. Deliberately NOT `SymbolSnapshotId` alone — two
/// snapshots with the same id are trivially "the same," but this gate
/// exists to catch the SAME semantic symbol ending up with a different
/// representation (or going missing, or being duplicated) between the
/// two paths, which comparing raw ids could mask if a bug happened to
/// change the id-derivation inputs identically on both paths.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotKey {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub span: (usize, usize),
}

/// A semantic edge comparison key: (from-symbol-key, to-symbol-key,
/// kind). Deliberately resolved through each endpoint's OWN semantic
/// key, not raw `SymbolSnapshotId`, for the identical reason
/// `SnapshotKey` itself is semantic rather than structural.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeKeyTuple {
    pub from: SnapshotKey,
    pub to: SnapshotKey,
    pub kind_rank: u8,
    pub kind_debug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImportFactKey {
    pub importing_file: String,
    pub module_path: String,
    pub imported_symbol: Option<String>,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImplFactKey {
    pub defining_file: String,
    pub self_type_text: String,
    pub trait_type_text: Option<String>,
}

/// Semantic Resolution & Reliability Checkpoint, Priority 2: a
/// semantic `CallAttempt` comparison key. Deliberately resolves
/// `caller` through its OWN `SnapshotKey` (identical reasoning to
/// `EdgeKeyTuple`'s own `from`/`to` fields: a raw `SymbolSnapshotId`
/// is structural, not semantic, and this gate exists to catch the
/// SAME semantic call site ending up with a different attempt
/// representation between the two paths). `candidate_count` (not the
/// candidates' own raw ids) is the comparison granularity — the exact
/// identity of which candidates were considered is real provenance
/// data, but two attempts that agree on caller/callee/tier/reason/
/// candidate-count are the SAME semantic attempt for parity purposes;
/// a real identity-level divergence (same count, different actual
/// candidates) would still be caught by a parity break at the
/// `edge_keys`/`snapshot_keys` level if it ever changed which edge
/// gets produced.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallAttemptKey {
    pub caller: SnapshotKey,
    pub callee_name: String,
    pub tier_debug: String,
    pub reason_debug: String,
    pub candidate_count: usize,
}

/// A deterministic numeric rank for `EdgeKind`, used only so
/// `EdgeKeyTuple` can derive `Ord` (the enum itself does not, and this
/// module does not need it to beyond providing a stable sort key here
/// — `kind_debug` is still carried separately for human-readable
/// diagnostics). Exhaustively matched, not a catch-all, so a future
/// new `EdgeKind` variant fails to COMPILE here rather than silently
/// sorting into an arbitrary bucket.
fn edge_kind_rank(kind: EdgeKind) -> u8 {
    match kind {
        EdgeKind::Calls => 0,
        EdgeKind::Contains => 1,
        EdgeKind::Extends => 2,
        EdgeKind::Implements => 3,
        EdgeKind::Instantiates => 4,
        EdgeKind::TestsFor => 5,
        EdgeKind::References => 6,
        EdgeKind::RenamedTo => 7,
    }
}

pub fn snapshot_keys(outcome: &ObserveOutcome) -> Vec<SnapshotKey> {
    let mut keys: Vec<SnapshotKey> = outcome
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
        .collect();
    keys.sort();
    keys
}

pub fn edge_keys(outcome: &ObserveOutcome) -> Vec<EdgeKeyTuple> {
    let key_of = |id: codegraph_domain::SymbolSnapshotId| -> Option<SnapshotKey> {
        outcome.graph.snapshot(id).map(|s| SnapshotKey {
            name: s.name().as_str().to_string(),
            kind: format!("{:?}", s.kind()),
            file: s.file().relative_path().to_string(),
            span: (s.span().start, s.span().end),
        })
    };
    let mut keys: Vec<EdgeKeyTuple> = outcome
        .graph
        .all_edges()
        .into_iter()
        .filter_map(|e| {
            Some(EdgeKeyTuple {
                from: key_of(e.from)?,
                to: key_of(e.to)?,
                kind_rank: edge_kind_rank(e.kind),
                kind_debug: format!("{:?}", e.kind),
            })
        })
        .collect();
    keys.sort();
    keys
}

pub fn import_fact_keys(outcome: &ObserveOutcome) -> Vec<ImportFactKey> {
    let mut keys: Vec<ImportFactKey> = outcome
        .import_facts
        .iter()
        .map(|f| ImportFactKey {
            importing_file: f.importing_file.relative_path().to_string(),
            module_path: f.module_path.clone(),
            imported_symbol: f.imported_symbol.clone(),
            alias: f.alias.clone(),
        })
        .collect();
    keys.sort();
    keys
}

pub fn impl_fact_keys(outcome: &ObserveOutcome) -> Vec<ImplFactKey> {
    let mut keys: Vec<ImplFactKey> = outcome
        .impl_facts
        .iter()
        .map(|f| ImplFactKey {
            defining_file: f.defining_file.relative_path().to_string(),
            self_type_text: f.self_type_text.clone(),
            trait_type_text: f.trait_type_text.clone(),
        })
        .collect();
    keys.sort();
    keys
}

/// Semantic Resolution & Reliability Checkpoint, Priority 2: resolves
/// each `CallAttempt`'s own `caller` id through the SAME real graph
/// `snapshot_keys`/`edge_keys` already use — a `CallAttempt` whose
/// caller cannot be found in `outcome.graph` at all (should never
/// happen in practice; the caller is always a real, currently-promoted
/// snapshot) is skipped rather than panicking, mirroring `edge_keys`'
/// own `filter_map` discipline.
pub fn call_attempt_keys(outcome: &ObserveOutcome) -> Vec<CallAttemptKey> {
    let mut keys: Vec<CallAttemptKey> = outcome
        .call_attempts
        .iter()
        .filter_map(|a| {
            let caller = outcome.graph.snapshot(a.caller)?;
            Some(CallAttemptKey {
                caller: SnapshotKey {
                    name: caller.name().as_str().to_string(),
                    kind: format!("{:?}", caller.kind()),
                    file: caller.file().relative_path().to_string(),
                    span: (caller.span().start, caller.span().end),
                },
                callee_name: a.callee_name.clone(),
                tier_debug: format!("{:?}", a.tier),
                reason_debug: format!("{:?}", a.reason),
                candidate_count: a.candidates.len(),
            })
        })
        .collect();
    keys.sort();
    keys
}

/// One dimension (snapshots / edges / imports / impls) on which two
/// states were compared and found to disagree — either by true
/// membership difference, or (the whole point of this module) by
/// MULTIPLICITY: the same multiset of distinct elements, but a
/// different count for at least one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Divergence {
    SnapshotsDiffer { only_in_full: Vec<String>, only_in_incremental: Vec<String> },
    SnapshotMultiplicityDiffers { key: String, full_count: usize, incremental_count: usize },
    EdgesDiffer { only_in_full: Vec<String>, only_in_incremental: Vec<String> },
    EdgeMultiplicityDiffers { key: String, full_count: usize, incremental_count: usize },
    ImportFactsDiffer { only_in_full: Vec<String>, only_in_incremental: Vec<String> },
    ImportFactMultiplicityDiffers { key: String, full_count: usize, incremental_count: usize },
    ImplFactsDiffer { only_in_full: Vec<String>, only_in_incremental: Vec<String> },
    ImplFactMultiplicityDiffers { key: String, full_count: usize, incremental_count: usize },
    /// Semantic Resolution & Reliability Checkpoint, Priority 2:
    /// mirrors every other `*Differ` variant exactly, for `CallAttempt`
    /// membership.
    CallAttemptsDiffer { only_in_full: Vec<String>, only_in_incremental: Vec<String> },
    /// Mirrors every other `*MultiplicityDiffers` variant exactly, for
    /// `CallAttempt` count.
    CallAttemptMultiplicityDiffers { key: String, full_count: usize, incremental_count: usize },
}

/// Compares two already-sorted `Vec<K>`s as true MULTISETS: returns
/// `Some(Divergence)` if either (a) the two sides have different
/// DISTINCT membership, or (b) they have identical distinct membership
/// but at least one element appears a different number of times on
/// each side. `label` is used only to build human-readable keys inside
/// the returned `Divergence`, via `describe`.
///
/// This is the one function this whole module exists to make
/// mutation-testable: every boundary here (`==`, the membership-diff
/// branch vs. the multiplicity branch, the counting logic itself) is a
/// genuine correctness-relevant decision a mutation could silently
/// break, and — unlike code that only ever lived under `tests/` —
/// `cargo-mutants` can actually instrument this because it lives under
/// `src/`.
fn compare_multiset<K: Ord + Clone + std::hash::Hash>(
    full_sorted: &[K],
    incremental_sorted: &[K],
    describe: impl Fn(&K) -> String,
    on_membership_diff: impl Fn(Vec<String>, Vec<String>) -> Divergence,
    on_multiplicity_diff: impl Fn(String, usize, usize) -> Divergence,
) -> Option<Divergence> {
    if full_sorted == incremental_sorted {
        return None;
    }

    let full_set: HashSet<&K> = full_sorted.iter().collect();
    let incremental_set: HashSet<&K> = incremental_sorted.iter().collect();

    if full_set != incremental_set {
        let mut only_in_full: Vec<String> = full_set.difference(&incremental_set).map(|k| describe(k)).collect();
        let mut only_in_incremental: Vec<String> = incremental_set.difference(&full_set).map(|k| describe(k)).collect();
        only_in_full.sort();
        only_in_incremental.sort();
        return Some(on_membership_diff(only_in_full, only_in_incremental));
    }

    // Same DISTINCT membership (confirmed above) but the sorted
    // sequences are unequal -- per the standard multiset-equality
    // argument, this is only possible if at least one element's own
    // COUNT differs between the two sides. Find the first such element
    // (sorted order makes this deterministic, not "whichever HashMap
    // iterates first").
    for key in full_set.iter() {
        let full_count = full_sorted.iter().filter(|k| *k == *key).count();
        let incremental_count = incremental_sorted.iter().filter(|k| *k == *key).count();
        if full_count != incremental_count {
            return Some(on_multiplicity_diff(describe(key), full_count, incremental_count));
        }
    }

    // Unreachable in practice (membership equal + sequences unequal
    // necessarily implies SOME count differs, per the loop above), but
    // never silently claim "equal" if we somehow get here -- that
    // would exactly reintroduce the false-negative this module exists
    // to close.
    Some(on_membership_diff(vec!["<multiset comparison invariant violated>".to_string()], vec![]))
}

/// The reusable equivalence check: compares a "full reindex" outcome
/// against an "incremental advance" outcome on the snapshot MULTISET,
/// typed-edge MULTISET, `ImportFact` MULTISET, and `ImplFact`
/// MULTISET — never mere membership, never mere counts. Returns every
/// divergence found (not just the first), so a caller gets a complete
/// diagnostic rather than having to re-run after fixing one dimension
/// to discover the next.
pub fn compare_full_incremental_state(full: &ObserveOutcome, incremental: &ObserveOutcome) -> Vec<Divergence> {
    let mut divergences = Vec::new();

    if let Some(d) = compare_multiset(
        &snapshot_keys(full),
        &snapshot_keys(incremental),
        |k| format!("{k:?}"),
        |only_in_full, only_in_incremental| Divergence::SnapshotsDiffer { only_in_full, only_in_incremental },
        |key, full_count, incremental_count| Divergence::SnapshotMultiplicityDiffers { key, full_count, incremental_count },
    ) {
        divergences.push(d);
    }

    if let Some(d) = compare_multiset(
        &edge_keys(full),
        &edge_keys(incremental),
        |k| format!("{k:?}"),
        |only_in_full, only_in_incremental| Divergence::EdgesDiffer { only_in_full, only_in_incremental },
        |key, full_count, incremental_count| Divergence::EdgeMultiplicityDiffers { key, full_count, incremental_count },
    ) {
        divergences.push(d);
    }

    if let Some(d) = compare_multiset(
        &import_fact_keys(full),
        &import_fact_keys(incremental),
        |k| format!("{k:?}"),
        |only_in_full, only_in_incremental| Divergence::ImportFactsDiffer { only_in_full, only_in_incremental },
        |key, full_count, incremental_count| Divergence::ImportFactMultiplicityDiffers { key, full_count, incremental_count },
    ) {
        divergences.push(d);
    }

    if let Some(d) = compare_multiset(
        &impl_fact_keys(full),
        &impl_fact_keys(incremental),
        |k| format!("{k:?}"),
        |only_in_full, only_in_incremental| Divergence::ImplFactsDiffer { only_in_full, only_in_incremental },
        |key, full_count, incremental_count| Divergence::ImplFactMultiplicityDiffers { key, full_count, incremental_count },
    ) {
        divergences.push(d);
    }

    // Semantic Resolution & Reliability Checkpoint, Priority 2: the
    // same exact-multiset discipline now extended to CallAttempt
    // membership/multiplicity/tier/reason/candidate-count — closes the
    // gap B2-EVAL-008 found (negative-knowledge was confirmed NOT
    // wired into the real incremental path at all).
    if let Some(d) = compare_multiset(
        &call_attempt_keys(full),
        &call_attempt_keys(incremental),
        |k| format!("{k:?}"),
        |only_in_full, only_in_incremental| Divergence::CallAttemptsDiffer { only_in_full, only_in_incremental },
        |key, full_count, incremental_count| Divergence::CallAttemptMultiplicityDiffers { key, full_count, incremental_count },
    ) {
        divergences.push(d);
    }

    divergences
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use codegraph_domain::{CommitHash, FileIdentity, RepositoryId, RepositoryRegistry, RepositoryRoot, RepositorySource, SourceError};

    use super::*;
    use crate::incremental::CodeGraphIndexer;

    /// A minimal `RepositorySource` test double, local to this
    /// module's own tests — the same shape every other full/incremental
    /// test in this crate uses, reused here (not imported from
    /// elsewhere) because this is a `src/`-internal unit test module,
    /// not the external integration test.
    struct FakeSource {
        files: HashMap<(RepositoryId, String), Vec<String>>,
        contents: HashMap<FileIdentity, Vec<u8>>,
    }
    impl FakeSource {
        fn new() -> Self {
            Self { files: HashMap::new(), contents: HashMap::new() }
        }
        fn with_file(mut self, file: FileIdentity, bytes: &[u8]) -> Self {
            self.files.entry((file.repository(), file.commit().as_str().to_string())).or_default().push(file.relative_path().to_string());
            self.contents.insert(file, bytes.to_vec());
            self
        }
    }
    impl RepositorySource for FakeSource {
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

    /// Mutation-testing-driven additions: `edge_keys`/`import_fact_keys`/
    /// `impl_fact_keys` each need a REAL, non-empty `ObserveOutcome`
    /// exercising their own dimension specifically — the self-tests
    /// above only ever call `compare_multiset` directly against
    /// synthetic `SnapshotKey` values, never these three extraction
    /// functions themselves, so a mutant collapsing any one of them to
    /// `vec![]` unconditionally went undetected (confirmed by
    /// `cargo-mutants`: `replace X -> Vec<...> with vec![]` was
    /// MISSED for all three before these tests existed).
    #[test]
    fn edge_keys_is_non_empty_for_a_real_run_with_a_real_edge() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let source = FakeSource::new().with_file(FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap(), b"fn a() { b(); }\nfn b() {}\n");
        let indexer = CodeGraphIndexer::new();
        let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        let keys = edge_keys(&outcome);
        assert!(!keys.is_empty(), "a real Calls edge exists (a() calls b()) -- edge_keys must not be empty");
        assert!(keys.iter().any(|k| k.from.name == "a" && k.to.name == "b" && k.kind_debug == "Calls"), "expected an a->b Calls edge among {keys:?}");
    }

    #[test]
    fn import_fact_keys_is_non_empty_for_a_real_run_with_a_real_import() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let source = FakeSource::new().with_file(FileIdentity::parse(repo, commit.clone(), "mod.py").unwrap(), b"import os\n");
        let indexer = CodeGraphIndexer::new();
        let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        let keys = import_fact_keys(&outcome);
        assert!(!keys.is_empty(), "a real `import os` statement exists -- import_fact_keys must not be empty");
        assert!(keys.iter().any(|k| k.module_path == "os"), "expected an import fact for module \"os\" among {keys:?}");
    }

    #[test]
    fn impl_fact_keys_is_non_empty_for_a_real_run_with_a_real_impl_block() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let source = FakeSource::new().with_file(FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap(), b"trait Shape { fn area(&self) -> f64; }\nstruct Circle;\nimpl Shape for Circle { fn area(&self) -> f64 { 1.0 } }\n");
        let indexer = CodeGraphIndexer::new();
        let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        let keys = impl_fact_keys(&outcome);
        assert!(!keys.is_empty(), "a real `impl Shape for Circle` block exists -- impl_fact_keys must not be empty");
        assert!(keys.iter().any(|k| k.self_type_text == "Circle" && k.trait_type_text.as_deref() == Some("Shape")), "expected an ImplFact for Circle/Shape among {keys:?}");
    }

    /// Semantic Resolution & Reliability Checkpoint, Priority 2: same
    /// mutation-driven discipline as the three tests immediately
    /// above, now for `call_attempt_keys`. A bare, unqualified
    /// `missing()` call with zero candidates in the pool produces
    /// exactly one real `Unresolved` `CallAttempt`.
    #[test]
    fn call_attempt_keys_is_non_empty_for_a_real_run_with_a_real_unresolved_attempt() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let source = FakeSource::new().with_file(FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap(), b"fn a() { missing(); }\n");
        let indexer = CodeGraphIndexer::new();
        let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        let keys = call_attempt_keys(&outcome);
        assert!(!keys.is_empty(), "a real unresolved call site exists (a() calls missing(), which has zero candidates) -- call_attempt_keys must not be empty");
        assert!(keys.iter().any(|k| k.caller.name == "a" && k.callee_name == "missing" && k.tier_debug == "Unresolved"), "expected an Unresolved CallAttempt for missing() among {keys:?}");
    }

    fn k(name: &str) -> SnapshotKey {
        SnapshotKey { name: name.to_string(), kind: "Function".to_string(), file: "a.rs".to_string(), span: (0, 1) }
    }

    #[test]
    fn identical_multisets_produce_no_divergence() {
        let a = vec![k("A"), k("B")];
        let b = vec![k("A"), k("B")];
        let result = compare_multiset(&a, &b, |x| format!("{x:?}"), |o1, o2| Divergence::SnapshotsDiffer { only_in_full: o1, only_in_incremental: o2 }, |key, f, i| {
            Divergence::SnapshotMultiplicityDiffers { key, full_count: f, incremental_count: i }
        });
        assert_eq!(result, None);
    }

    #[test]
    fn a_true_membership_difference_is_detected() {
        let full = vec![k("A"), k("B")];
        let incremental = vec![k("A"), k("C")];
        let result = compare_multiset(&full, &incremental, |x| format!("{x:?}"), |o1, o2| Divergence::SnapshotsDiffer { only_in_full: o1, only_in_incremental: o2 }, |key, f, i| {
            Divergence::SnapshotMultiplicityDiffers { key, full_count: f, incremental_count: i }
        });
        assert!(matches!(result, Some(Divergence::SnapshotsDiffer { .. })), "expected a membership divergence, got {result:?}");
    }

    /// THE central regression this module exists to fix: `[A, B]` vs.
    /// `[A, B, B]` — identical SET membership (`{A, B}` both sides),
    /// but B appears twice on the incremental side. A `HashSet`-based
    /// comparison (the OLD approach, before Phase 1.1) would report
    /// these as equal. This multiset-based comparison must not.
    #[test]
    fn the_canonical_multiset_disagrees_with_naive_set_equality_on_a_duplicate() {
        let full = vec![k("A"), k("B")];
        let mut incremental = vec![k("A"), k("B"), k("B")];
        incremental.sort();

        // Prove the OLD failure mode first: a naive HashSet-based
        // comparison of these exact two inputs says "equal" -- this is
        // not a strawman, it is literally what the pre-Phase-1.1 gate
        // did.
        let full_set: HashSet<&SnapshotKey> = full.iter().collect();
        let incremental_set: HashSet<&SnapshotKey> = incremental.iter().collect();
        assert_eq!(full_set, incremental_set, "the OLD HashSet-based comparison must find these equal -- that is exactly the blind spot Phase 1.1 closes");

        // Now prove the NEW multiset-based comparison correctly
        // disagrees.
        let result = compare_multiset(&full, &incremental, |x| format!("{x:?}"), |o1, o2| Divergence::SnapshotsDiffer { only_in_full: o1, only_in_incremental: o2 }, |key, f, i| {
            Divergence::SnapshotMultiplicityDiffers { key, full_count: f, incremental_count: i }
        });
        assert_eq!(
            result,
            Some(Divergence::SnapshotMultiplicityDiffers { key: format!("{:?}", k("B")), full_count: 1, incremental_count: 2 }),
            "the multiset comparison must report a MULTIPLICITY divergence on B (1 vs. 2), not silently agree"
        );
    }

    /// The symmetric case: full has the duplicate, incremental does
    /// not -- confirms the multiplicity check is not direction-biased.
    #[test]
    fn multiplicity_divergence_is_detected_regardless_of_which_side_has_the_duplicate() {
        let mut full = vec![k("A"), k("B"), k("B")];
        full.sort();
        let incremental = vec![k("A"), k("B")];

        let result = compare_multiset(&full, &incremental, |x| format!("{x:?}"), |o1, o2| Divergence::SnapshotsDiffer { only_in_full: o1, only_in_incremental: o2 }, |key, f, i| {
            Divergence::SnapshotMultiplicityDiffers { key, full_count: f, incremental_count: i }
        });
        assert_eq!(result, Some(Divergence::SnapshotMultiplicityDiffers { key: format!("{:?}", k("B")), full_count: 2, incremental_count: 1 }));
    }

    #[test]
    fn empty_vs_empty_is_not_a_divergence() {
        let a: Vec<SnapshotKey> = vec![];
        let b: Vec<SnapshotKey> = vec![];
        let result = compare_multiset(&a, &b, |x| format!("{x:?}"), |o1, o2| Divergence::SnapshotsDiffer { only_in_full: o1, only_in_incremental: o2 }, |key, f, i| {
            Divergence::SnapshotMultiplicityDiffers { key, full_count: f, incremental_count: i }
        });
        assert_eq!(result, None);
    }

    #[test]
    fn a_triple_vs_double_multiplicity_is_detected_not_just_duplicate_vs_single() {
        // Guards against an off-by-one in the counting logic itself
        // (e.g. a mutant that always reports count 1 vs 2 regardless
        // of the real counts).
        let mut full = vec![k("A"), k("A"), k("A")];
        full.sort();
        let mut incremental = vec![k("A"), k("A")];
        incremental.sort();
        let result = compare_multiset(&full, &incremental, |x| format!("{x:?}"), |o1, o2| Divergence::SnapshotsDiffer { only_in_full: o1, only_in_incremental: o2 }, |key, f, i| {
            Divergence::SnapshotMultiplicityDiffers { key, full_count: f, incremental_count: i }
        });
        assert_eq!(result, Some(Divergence::SnapshotMultiplicityDiffers { key: format!("{:?}", k("A")), full_count: 3, incremental_count: 2 }));
    }

    #[test]
    fn edge_kind_rank_is_exhaustive_and_stable() {
        // Pins the exact ranking so a future accidental reordering is
        // caught, and confirms every current EdgeKind variant has a
        // rank (the match itself is exhaustive at compile time; this
        // test additionally confirms no two variants collide).
        let kinds = [EdgeKind::Calls, EdgeKind::Contains, EdgeKind::Extends, EdgeKind::Implements, EdgeKind::Instantiates, EdgeKind::TestsFor, EdgeKind::References, EdgeKind::RenamedTo];
        let ranks: Vec<u8> = kinds.iter().map(|k| edge_kind_rank(*k)).collect();
        let mut sorted_ranks = ranks.clone();
        sorted_ranks.sort();
        sorted_ranks.dedup();
        assert_eq!(sorted_ranks.len(), ranks.len(), "every EdgeKind variant must have a DISTINCT rank, got {ranks:?}");
    }

    // Semantic Resolution & Reliability Checkpoint, Priority 2 (the
    // "CallAttempt incremental integration" requirement, flagged
    // PARTIAL in B2-EVAL-008 and NOT DONE in the checkpoint's own
    // first pass): the 7 required adversarial full/incremental
    // CallAttempt parity cases, each run through the REAL
    // `CodeGraphIndexer` (never a synthetic unit fixture) and checked
    // via `compare_full_incremental_state` itself -- the exact same
    // production entry point and comparison gate a real evaluation
    // run uses, mirroring exactly how B2-ID-010's own
    // `b2_id_010_csharp_partial_class_through_the_real_indexer` module
    // validated full/incremental parity for partial-class
    // reconciliation.
    mod call_attempt_adversarial_parity {
        use super::*;

        /// Confirms a full reindex of commit B (as a brand-new,
        /// separate repository) and an incremental advance A->B
        /// through the SAME indexer produce IDENTICAL `CallAttempt`
        /// multisets for the identical final commit -- the real-
        /// indexer equivalent of this module's own synthetic
        /// `compare_multiset` self-tests above. Each test builds its
        /// own commit-A and commit-B file contents as
        /// `(path, bytes)` pairs; this helper derives TWO independent
        /// `RepositoryId`s (one for the incremental indexer, one for
        /// the full-reindex indexer) and constructs each `FakeSource`
        /// with the correct `FileIdentity`s itself, so the SAME file
        /// content list can be reused for both paths without the
        /// caller juggling two different `RepositoryId`s by hand.
        pub(super) fn assert_full_and_incremental_agree(
            commit_a_files: &[(&str, &[u8])],
            commit_b_files: &[(&str, &[u8])],
            commit_a: &CommitHash,
            commit_b: &CommitHash,
            changed_files: &[String],
        ) -> (ObserveOutcome, ObserveOutcome) {
            let repo_incremental = test_repo();
            let repo_full = test_repo();

            let mut source_incremental = FakeSource::new();
            for (path, bytes) in commit_a_files {
                source_incremental = source_incremental.with_file(FileIdentity::parse(repo_incremental, commit_a.clone(), path).unwrap(), bytes);
            }
            for (path, bytes) in commit_b_files {
                source_incremental = source_incremental.with_file(FileIdentity::parse(repo_incremental, commit_b.clone(), path).unwrap(), bytes);
            }

            let mut source_full = FakeSource::new();
            for (path, bytes) in commit_b_files {
                source_full = source_full.with_file(FileIdentity::parse(repo_full, commit_b.clone(), path).unwrap(), bytes);
            }

            let indexer = CodeGraphIndexer::new();
            indexer.observe_commit(&source_incremental, repo_incremental, commit_a, None, &[]).unwrap();
            let incremental_outcome = indexer.observe_commit(&source_incremental, repo_incremental, commit_b, Some(commit_a), changed_files).unwrap();

            let full_indexer = CodeGraphIndexer::new();
            let full_outcome = full_indexer.observe_commit(&source_full, repo_full, commit_b, None, &[]).unwrap();

            let divergences = compare_full_incremental_state(&full_outcome, &incremental_outcome);
            assert!(divergences.is_empty(), "full vs. incremental CallAttempt state must be IDENTICAL for the same final commit, found: {divergences:?}");
            (full_outcome, incremental_outcome)
        }

        #[test]
        fn unresolved_becomes_resolved() {
            // Commit A: a() calls missing() -- zero candidates, Unresolved.
            // Commit B: missing() is DEFINED -- a() calling it now resolves.
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

            let (_, incremental_outcome) = assert_full_and_incremental_agree(
                &[("src/lib.rs", b"fn a() { missing(); }\n")],
                &[("src/lib.rs", b"fn a() { missing(); }\nfn missing() {}\n")],
                &commit_a,
                &commit_b,
                &["src/lib.rs".to_string()],
            );

            let keys = call_attempt_keys(&incremental_outcome);
            assert!(!keys.iter().any(|k| k.callee_name == "missing" && k.tier_debug == "Unresolved"), "missing() is now defined -- no Unresolved CallAttempt for it should remain among {keys:?}");
        }

        #[test]
        fn resolved_becomes_ambiguous() {
            // Commit A: caller.rs's a() calls shared() -- shared()
            // itself lives in a DIFFERENT file (one.rs), so this is a
            // genuine repo-wide resolution, not narrowed to
            // same-file/same-class first (see `narrow_by_scope`'s own
            // CALL-RES-A1 same-file preference -- if shared() lived in
            // the CALLER's own file, adding a second same-named
            // candidate elsewhere would never become ambiguous, since
            // the same-file one would still be preferred; this fixture
            // deliberately avoids that confound).
            // Commit B: a SECOND shared() is added in yet another file
            // -- now genuinely ambiguous repo-wide.
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

            let (_, incremental_outcome) = assert_full_and_incremental_agree(
                &[("src/caller.rs", b"fn a() { shared(); }\n"), ("src/one.rs", b"fn shared() {}\n")],
                &[("src/caller.rs", b"fn a() { shared(); }\n"), ("src/one.rs", b"fn shared() {}\n"), ("src/two.rs", b"fn shared() {}\n")],
                &commit_a,
                &commit_b,
                &["src/two.rs".to_string()],
            );

            let keys = call_attempt_keys(&incremental_outcome);
            assert!(keys.iter().any(|k| k.callee_name == "shared" && k.tier_debug == "Ambiguous"), "a second shared() was added elsewhere -- the a() call site must now be Ambiguous among {keys:?}");
        }

        #[test]
        fn target_deleted_resolved_becomes_unresolved() {
            // Commit A: a() calls target() -- Resolved.
            // Commit B: target()'s OWN file is deleted entirely (simply
            // absent from commit_b_files).
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

            let (_, incremental_outcome) = assert_full_and_incremental_agree(
                &[("src/caller.rs", b"fn a() { target(); }\n"), ("src/target.rs", b"fn target() {}\n")],
                &[("src/caller.rs", b"fn a() { target(); }\n")],
                &commit_a,
                &commit_b,
                &["src/target.rs".to_string()],
            );

            let keys = call_attempt_keys(&incremental_outcome);
            assert!(keys.iter().any(|k| k.callee_name == "target" && k.tier_debug == "Unresolved"), "target() was deleted -- the a() call site must now be Unresolved among {keys:?}");
        }

        #[test]
        fn target_added_unresolved_becomes_resolved() {
            // Symmetric to `unresolved_becomes_resolved` but via a
            // whole NEW file being added, not an edit to an existing one.
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

            let (_, incremental_outcome) = assert_full_and_incremental_agree(
                &[("src/caller.rs", b"fn a() { brand_new(); }\n")],
                &[("src/caller.rs", b"fn a() { brand_new(); }\n"), ("src/new.rs", b"fn brand_new() {}\n")],
                &commit_a,
                &commit_b,
                &["src/new.rs".to_string()],
            );

            let keys = call_attempt_keys(&incremental_outcome);
            assert!(!keys.iter().any(|k| k.callee_name == "brand_new" && k.tier_debug == "Unresolved"), "brand_new() was just added -- no Unresolved CallAttempt should remain among {keys:?}");
        }

        #[test]
        fn signature_too_long_introduced_changes_reason() {
            // Commit A: a() calls missing() -- zero candidates, Unresolved/Unknown.
            // Commit B: a REAL candidate named "missing" exists in source
            // but is rejected via SignatureTooLong (an absurdly long
            // signature) -- the SAME callee name's Unresolved reason
            // must become ExtractionRejected, never staying generic
            // Unknown, once the real rejection is observable.
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
            let long_params = (0..2000).map(|i| format!("p{i}: i32")).collect::<Vec<_>>().join(", ");
            let oversized_fn = format!("fn missing({long_params}) {{}}\n");

            let (_, incremental_outcome) = assert_full_and_incremental_agree(
                &[("src/caller.rs", b"fn a() { missing(); }\n")],
                &[("src/caller.rs", b"fn a() { missing(); }\n"), ("src/rejected.rs", oversized_fn.as_bytes())],
                &commit_a,
                &commit_b,
                &["src/rejected.rs".to_string()],
            );

            let keys = call_attempt_keys(&incremental_outcome);
            assert!(
                keys.iter().any(|k| k.callee_name == "missing" && k.tier_debug == "Unresolved" && k.reason_debug.contains("ExtractionRejected")),
                "a real SignatureTooLong rejection sharing the exact callee name now exists -- the reason must be ExtractionRejected, not generic Unknown, among {keys:?}"
            );
        }

        #[test]
        fn signature_too_long_removed_reverts_reason_to_unknown() {
            // The exact reverse of the previous case: start WITH the
            // rejection present, then remove the oversized file --
            // the reason must revert to Unknown, never staying stuck
            // at a stale ExtractionRejected classification.
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
            let long_params = (0..2000).map(|i| format!("p{i}: i32")).collect::<Vec<_>>().join(", ");
            let oversized_fn = format!("fn missing({long_params}) {{}}\n");

            let (_, incremental_outcome) = assert_full_and_incremental_agree(
                &[("src/caller.rs", b"fn a() { missing(); }\n"), ("src/rejected.rs", oversized_fn.as_bytes())],
                &[("src/caller.rs", b"fn a() { missing(); }\n")],
                &commit_a,
                &commit_b,
                &["src/rejected.rs".to_string()],
            );

            let keys = call_attempt_keys(&incremental_outcome);
            let missing_attempt = keys.iter().find(|k| k.callee_name == "missing").expect("an Unresolved attempt for missing() must still exist");
            assert_eq!(missing_attempt.tier_debug, "Unresolved");
            assert_eq!(missing_attempt.reason_debug, "Unknown", "once the rejected candidate's own file is deleted, the reason must revert to Unknown, not stay stuck at a stale ExtractionRejected");
        }

        #[test]
        fn generic_arity_changes_the_candidate_set() {
            // Commit A: Policy.Handle<T>() is the ONLY Handle -- a
            // generic Handle<T>() call resolves unambiguously.
            // Commit B: a SECOND, same-arity overload
            // Handle<T>(Func<T,bool>) is added elsewhere -- arity alone
            // can no longer distinguish them (this is the EXACT real
            // Polly shape B2-EVAL-008 found), so the call site must
            // become Ambiguous, not silently stay Resolved.
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
            let caller: &[u8] = b"class Caller { void Use() { Policy.Handle<TException>(); } }\n";
            let policy_a: &[u8] = b"class Policy { public static PolicyBuilder Handle<TException>() => null; }\n";
            let policy_b_extra: &[u8] = b"class Policy { public static PolicyBuilder Handle<TException>(System.Func<TException, bool> p) => null; }\n";

            let (_, incremental_outcome) = assert_full_and_incremental_agree(
                &[("src/Caller.cs", caller), ("src/Policy.cs", policy_a)],
                &[("src/Caller.cs", caller), ("src/Policy.cs", policy_a), ("src/Policy.Extra.cs", policy_b_extra)],
                &commit_a,
                &commit_b,
                &["src/Policy.Extra.cs".to_string()],
            );

            let keys = call_attempt_keys(&incremental_outcome);
            assert!(
                keys.iter().any(|k| k.callee_name == "Handle" && k.tier_debug == "Ambiguous" && k.candidate_count == 2),
                "a second same-arity Handle overload was added -- arity alone cannot disambiguate 2 real candidates, so this must be Ambiguous (candidate_count 2), among {keys:?}"
            );
        }
    }

    /// CALL-RES-A4 wiring task: the prior checkpoint's reliability-policy
    /// re-check found that the REAL production entry point this module's
    /// own `assert_full_and_incremental_agree` helper exercises
    /// (`CodeGraphIndexer::observe_commit`, via
    /// `resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`
    /// at `incremental/indexer.rs`'s own call-resolution step) called
    /// `resolve_calls_with_extraction_rejections` — the PRE-A4 entry
    /// point — for its call-resolution pass, never
    /// `resolve_calls_with_imports`/`resolve_calls_with_namespace_context`,
    /// even though the SAME real `csharp_contexts`/`import_facts` were
    /// already computed and available at that exact call site. This
    /// module now wires `import_facts` through to that call (both the
    /// full-reindex path, via `pipeline::index_files`, and the
    /// incremental path, via `indexer.rs`'s own `incremental_reindex`)
    /// and routes it through `resolve_calls_with_namespace_context`
    /// instead. This test, which used to PIN THE GAP (asserting the
    /// stale `Ambiguous(2)` outcome), now confirms the FIX with the
    /// exact same real data and real entry point.
    mod call_res_a4_wired_into_the_real_indexer {
        use super::*;
        use crate::incremental::CodeGraphIndexer;
        use codegraph_domain::ResolutionMethod;

        /// Builds the EXACT real-corpus shape `edges::calls`'s own
        /// `namespace_import_excludes_the_non_imported_namespace_candidate`
        /// test proves A4's namespace mechanism resolves (two
        /// same-named candidates in different namespaces, caller `using`s
        /// one of them) and runs it through the REAL
        /// `CodeGraphIndexer::observe_commit` entry point (this task's
        /// required live-indexer test 1). Confirms the real indexer now
        /// reports `Heuristic`/1-candidate (A4's own `NameHeuristic`
        /// evidence), not the pre-wiring `Ambiguous`/2-candidate outcome,
        /// and that a real `Calls` edge to the correct, single `Target`
        /// now exists.
        #[test]
        fn the_real_indexer_now_resolves_a_case_a4_itself_resolves() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/NsA/Foo.cs").unwrap(), b"namespace NsA;\nclass Foo { public void Target() {} }\n")
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/NsB/Foo.cs").unwrap(), b"namespace NsB;\nclass Foo { public void Target() {} }\n")
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/Caller.cs").unwrap(), b"using NsA;\nclass Caller { public void Run() { Target(); } }\n");

            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let keys = call_attempt_keys(&outcome);
            let run_attempt = keys.iter().find(|k| k.callee_name == "Target");
            assert!(
                run_attempt.is_none(),
                "A4 must resolve this call site to a real edge, not leave a CallAttempt behind -- found {run_attempt:?}"
            );
            let edges = edge_keys(&outcome);
            assert!(
                edges.iter().any(|e| e.kind_debug == "Calls" && e.to.name == "Target" && e.to.file.contains("NsA")),
                "the real wired indexer must now produce a Calls edge to NsA's Target (the `using NsA;` import-visible candidate), confirming A4's namespace mechanism is reachable from CodeGraphIndexer::observe_commit. Edges: {edges:?}"
            );
        }

        /// This task's required live-indexer test 2: the SAME real case
        /// and SAME expected outcome, reproduced through
        /// `CodeGraphIndexer::observe_commit`'s INCREMENTAL path
        /// (`NormalAdvance`, not `NewRepository`) — a first commit with
        /// only the two namespace-ambiguous candidates, then a second
        /// commit that adds the caller file as the only change.
        #[test]
        fn the_same_resolution_reproduces_through_observe_commit_incrementally() {
            let repo = test_repo();
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
            let source = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/NsA/Foo.cs").unwrap(), b"namespace NsA;\nclass Foo { public void Target() {} }\n")
                .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/NsB/Foo.cs").unwrap(), b"namespace NsB;\nclass Foo { public void Target() {} }\n")
                .with_file(FileIdentity::parse(repo, commit_b.clone(), "src/NsA/Foo.cs").unwrap(), b"namespace NsA;\nclass Foo { public void Target() {} }\n")
                .with_file(FileIdentity::parse(repo, commit_b.clone(), "src/NsB/Foo.cs").unwrap(), b"namespace NsB;\nclass Foo { public void Target() {} }\n")
                .with_file(FileIdentity::parse(repo, commit_b.clone(), "src/Caller.cs").unwrap(), b"using NsA;\nclass Caller { public void Run() { Target(); } }\n");

            let indexer = CodeGraphIndexer::new();
            let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
            let outcome = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/Caller.cs".to_string()]).unwrap();
            drop(first);

            let keys = call_attempt_keys(&outcome);
            assert!(keys.iter().find(|k| k.callee_name == "Target").is_none(), "incremental advance must also resolve this call site to a real edge, leaving no CallAttempt");
            let edges = edge_keys(&outcome);
            assert!(
                edges.iter().any(|e| e.kind_debug == "Calls" && e.to.name == "Target" && e.to.file.contains("NsA")),
                "the incremental path must reach the identical A4 resolution as the full path. Edges: {edges:?}"
            );
        }

        /// This task's required live-indexer test 3: full-index vs.
        /// incremental-index equivalence extended to edge, CallAttempt,
        /// tier (both already covered by `assert_full_and_incremental_agree`'s
        /// own `compare_full_incremental_state`, which this reuses
        /// directly rather than duplicating), AND provenance — the one
        /// dimension `compare_full_incremental_state` does not compare
        /// (its `EdgeKeyTuple` has no evidence-note field). Confirms
        /// both paths produce the real, raw `Edge`/`Evidence` for the
        /// SAME A4 namespace-resolved call site with byte-identical
        /// `ResolutionMethod` and evidence-note text, not just agreeing
        /// multiset keys.
        #[test]
        fn full_and_incremental_agree_on_edge_call_attempt_tier_and_provenance_for_the_newly_wired_case() {
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

            let (full_outcome, incremental_outcome) = super::call_attempt_adversarial_parity::assert_full_and_incremental_agree(
                &[
                    ("src/NsA/Foo.cs", b"namespace NsA;\nclass Foo { public void Target() {} }\n" as &[u8]),
                    ("src/NsB/Foo.cs", b"namespace NsB;\nclass Foo { public void Target() {} }\n" as &[u8]),
                ],
                &[
                    ("src/NsA/Foo.cs", b"namespace NsA;\nclass Foo { public void Target() {} }\n" as &[u8]),
                    ("src/NsB/Foo.cs", b"namespace NsB;\nclass Foo { public void Target() {} }\n" as &[u8]),
                    ("src/Caller.cs", b"using NsA;\nclass Caller { public void Run() { Target(); } }\n" as &[u8]),
                ],
                &commit_a,
                &commit_b,
                &["src/Caller.cs".to_string()],
            );
            // assert_full_and_incremental_agree already asserted
            // compare_full_incremental_state is empty -- edges,
            // CallAttempts (hence tier/reason/candidate_count), import
            // facts, impl facts, and snapshots all already agree as a
            // multiset. The real, raw Evidence (not just its EdgeKeyTuple
            // projection) for the specific A4-resolved edge is compared
            // directly here, against both outcomes' own real `Edge`s.
            let find_target_edge = |outcome: &ObserveOutcome| -> codegraph_domain::Evidence {
                outcome
                    .graph
                    .all_snapshots()
                    .into_iter()
                    .find(|s| s.name().as_str() == "Target" && s.file().relative_path().contains("NsA"))
                    .and_then(|target| outcome.graph.incoming_edges(target.id(), Some(EdgeKind::Calls)).into_iter().next().cloned())
                    .map(|e| e.evidence)
                    .expect("the A4-resolved Calls edge to NsA's Target must exist in both outcomes")
            };
            let full_evidence = find_target_edge(&full_outcome);
            let incremental_evidence = find_target_edge(&incremental_outcome);
            assert_eq!(full_evidence.method(), incremental_evidence.method(), "full and incremental must agree on ResolutionMethod for the A4-resolved edge");
            assert_eq!(full_evidence.note(), incremental_evidence.note(), "full and incremental must agree on the evidence note (provenance) for the A4-resolved edge, not just its structural key");
            assert_eq!(full_evidence.method(), ResolutionMethod::NameHeuristic, "A4's namespace mechanism must produce NameHeuristic evidence, never StaticAnalysis -- it is still name-based linking, not full semantic resolution");
        }

        /// This task's required live-indexer test 4 (CRC-001): the EXACT
        /// real-corpus shape `edges::calls::call_res_a4_import_alias::crc_001_ambiguous_constructor_call_regression_check`
        /// already proves non-regressed at the isolated-resolver level,
        /// now run through the REAL `CodeGraphIndexer::observe_commit`
        /// entry point for the first time. A4's zero-candidate-only
        /// scope never touches this `>=2` ambiguity -- must stay
        /// honestly Ambiguous with 2 candidates through the real wired
        /// indexer too, never resolved and never a new FalsePositive.
        #[test]
        fn crc_001_through_the_real_wired_indexer_stays_ambiguous() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit.clone(), "models/a.py").unwrap(), b"class RNNModel:\n    pass\n")
                .with_file(FileIdentity::parse(repo, commit.clone(), "models/b.py").unwrap(), b"class RNNModel:\n    pass\n")
                .with_file(FileIdentity::parse(repo, commit.clone(), "models/trainer.py").unwrap(), b"import os\n\n\ndef _run_trainer():\n    return RNNModel()\n");

            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let keys = call_attempt_keys(&outcome);
            let attempt = keys.iter().find(|k| k.callee_name == "RNNModel").expect("CRC-001's own ambiguous constructor call must still produce a CallAttempt through the real indexer");
            assert_eq!(attempt.tier_debug, "Ambiguous", "CRC-001 must stay Ambiguous through the real wired indexer -- A4 never touches a >=2-candidate ambiguity");
            assert_eq!(attempt.candidate_count, 2);
            let edges = edge_keys(&outcome);
            assert!(edges.iter().all(|e| e.to.name != "RNNModel"), "no Calls edge to RNNModel must exist -- confirms A4 introduced no new FalsePositive on CRC-001 through the real indexer");
        }

        /// This task's required live-indexer test 4 (CRC-002): the EXACT
        /// real-corpus shape `edges::calls::call_res_a4_import_alias::crc_002_local_binding_shadowing_regression_check`
        /// already proves byte-for-byte identical before/after at the
        /// isolated-resolver level -- CRC-002 is PRE-EXISTING,
        /// documented, undocumented-parameter-shadowing territory A4
        /// has no visibility into (see CALL-RES-A4-CHECKPOINT.md's own
        /// "No local-binding/parameter-shadowing awareness" residual
        /// limitation): the pre-A4 baseline ALREADY wires a `Calls`
        /// edge from `render` to the unrelated module-level `Button`,
        /// and A4 does not change that. This test confirms the real
        /// wired indexer reproduces that SAME pre-existing shape
        /// (neither fixing nor worsening it), not a byte-identical-to-
        /// isolated-resolver claim the real indexer's own full pipeline
        /// (lineage/metrics/graph assembly) was never asserted to give.
        #[test]
        fn crc_002_through_the_real_wired_indexer_does_not_regress() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit.clone(), "components/other.py").unwrap(), b"def Button():\n    pass\n")
                .with_file(FileIdentity::parse(repo, commit.clone(), "components/card.py").unwrap(), b"import os\n\n\ndef render(Button):\n    return Button()\n");

            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let edges = edge_keys(&outcome);
            assert!(
                edges.iter().any(|e| e.kind_debug == "Calls" && e.to.name == "Button" && e.to.file.contains("other.py")),
                "CRC-002's own pre-existing shape (render's local `Button` parameter is not recognized as shadowing the module-level `Button`) must reproduce IDENTICALLY through the real wired indexer -- A4 neither introduces nor fixes this known limitation. Edges: {edges:?}"
            );
            let keys = call_attempt_keys(&outcome);
            assert!(keys.iter().find(|k| k.callee_name == "Button").is_none(), "matches the isolated-resolver baseline exactly: this call site resolves directly (no CallAttempt), both before and after A4");
        }

        /// This task's required live-indexer test 4 (CRC-003): the EXACT
        /// real-corpus shape `edges::calls::call_res_a4_import_alias::crc_003_exact_shape_recovers_only_to_ambiguous_due_to_a_real_name_collision`
        /// already proves at the isolated-resolver level that A4's
        /// alias mechanism finds a real match whose own target remains
        /// ambiguous due to a genuine name collision (the wrapper
        /// function shares its own bare name with the real target it
        /// wraps). Run through the real indexer for the first time:
        /// confirms the SAME honest `ImportedSymbolAmbiguous` outcome,
        /// not a silently-picked wrong target.
        #[test]
        fn crc_003_through_the_real_wired_indexer_reports_the_same_honest_ambiguity() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit.clone(), "janitor/ml.py").unwrap(), b"def get_features_targets():\n    pass\n")
                .with_file(
                    FileIdentity::parse(repo, commit.clone(), "janitor/__init__.py").unwrap(),
                    b"from .ml import get_features_targets as _get_features_targets\n\n\ndef get_features_targets(*args, **kwargs):\n    return _get_features_targets(*args, **kwargs)\n",
                );

            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let keys = call_attempt_keys(&outcome);
            let attempt = keys
                .iter()
                .find(|k| k.callee_name == "_get_features_targets")
                .expect("CRC-003's own alias call must produce a CallAttempt through the real indexer -- A4's alias mechanism found a real match, but that match's own target is ambiguous");
            assert_eq!(attempt.reason_debug, "ImportedSymbolAmbiguous", "the real wired indexer must report the same honest, specific ambiguity A4's isolated resolver already proved, never a plain Unknown and never a silently-picked wrong target");
            assert_eq!(attempt.candidate_count, 2, "both the real target and the wrapper's own same-named self remain real candidates");
        }
    }
}
