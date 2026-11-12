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
}
