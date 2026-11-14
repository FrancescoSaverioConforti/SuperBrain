//! B2 Task 7: shared edge type, evidence helpers, and target lookup.

use codegraph_domain::{Confidence, Evidence, EdgeKind, ResolutionMethod, SymbolSnapshotId};
use thiserror::Error;

use crate::snapshot::SymbolSnapshot;

/// A typed, non-lineage relationship between two `SymbolSnapshot`s.
/// Unlike `codegraph_domain::LineageStatus` (Task 6), which carries its
/// evidence inline per status variant, every `Edge` ALWAYS carries
/// `Evidence` directly — there is no "confirmed, no explanation needed"
/// shortcut, since even a syntactically-certain edge (e.g. `impl Trait
/// for Type`) still benefits from recording exactly which source
/// construct justified it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub from: SymbolSnapshotId,
    pub to: SymbolSnapshotId,
    pub kind: EdgeKind,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum EdgeResolutionError {
    /// `prev`/`next`-style duplicate-id invariant, mirroring Task 6's
    /// `LineageResolutionError::DuplicateSnapshotId` for the same
    /// reason: a caller including the same snapshot twice in the
    /// candidate pool is its own bug, never a domain ambiguity for this
    /// module to resolve heuristically.
    #[error("duplicate SymbolSnapshotId in the candidate pool: a caller must never include the same snapshot twice")]
    DuplicateSnapshotId,
}

/// Builds `Evidence` for a resolved (syntactically unambiguous) edge —
/// the ONLY place in this crate's edge resolvers that constructs
/// `ResolutionMethod::StaticAnalysis` evidence. `codegraph_domain::Evidence::new`
/// still independently validates the note length; this helper exists so
/// every resolved-edge call site shares one, auditable path into
/// `StaticAnalysis` evidence rather than six near-identical call sites
/// that could drift.
pub(super) fn resolved_evidence(note: &str) -> Evidence {
    Evidence::new(ResolutionMethod::StaticAnalysis, None, note).expect("edge evidence notes are built short and bounded by this module")
}

/// Builds `Evidence` for a heuristic/ambiguous/unresolved edge, with an
/// explicit `Confidence` and a non-`StaticAnalysis` `method`. Panics if
/// `method` is `StaticAnalysis` — that combination would contradict this
/// function's own name and purpose, and every call site in this crate
/// is expected to pass `NameHeuristic` or `Inferred`, never this method
/// by mistake.
pub(super) fn heuristic_evidence(method: ResolutionMethod, confidence: Confidence, note: &str) -> (Evidence, Confidence) {
    assert_ne!(
        method, ResolutionMethod::StaticAnalysis,
        "heuristic_evidence must never be called with StaticAnalysis — use resolved_evidence for that"
    );
    let evidence = Evidence::new(method, None, note).expect("edge evidence notes are built short and bounded by this module");
    (evidence, confidence)
}

/// A read-only view over the snapshots an edge resolver is allowed to
/// link against — shared by every per-edge-kind resolver in this
/// module, so "find every snapshot whose name matches X" has exactly
/// one implementation. Deliberately does not expose anything beyond
/// name-based lookup: resolving a target is always "does a snapshot
/// with this name exist among what I was given," never a broader
/// graph traversal (which belongs to a later task).
pub(super) struct TargetLookup<'a> {
    snapshots: &'a [SymbolSnapshot],
}

impl<'a> TargetLookup<'a> {
    pub(super) fn new(snapshots: &'a [SymbolSnapshot]) -> Result<Self, EdgeResolutionError> {
        reject_duplicate_ids(snapshots)?;
        Ok(Self { snapshots })
    }

    pub(super) fn snapshots(&self) -> &'a [SymbolSnapshot] {
        self.snapshots
    }

    /// Every snapshot whose name matches `name` exactly, EXCLUDING
    /// `exclude_id` (a resolver's own source snapshot, so a symbol can
    /// never be resolved as referencing itself by name coincidence
    /// alone — e.g. a recursive call is handled explicitly by its own
    /// resolver, not accidentally produced here as a side effect of
    /// "the function's own name matches its own name").
    pub(super) fn find_by_name(&self, name: &str, exclude_id: SymbolSnapshotId) -> Vec<&'a SymbolSnapshot> {
        self.snapshots.iter().filter(|s| s.id() != exclude_id && s.name().as_str() == name).collect()
    }
}

fn reject_duplicate_ids(snapshots: &[SymbolSnapshot]) -> Result<(), EdgeResolutionError> {
    let mut seen = std::collections::HashSet::with_capacity(snapshots.len());
    for snapshot in snapshots {
        if !seen.insert(snapshot.id()) {
            return Err(EdgeResolutionError::DuplicateSnapshotId);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot};

    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::promote;

    fn test_snapshot(name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, "src/lib.rs").unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    #[test]
    #[should_panic(expected = "heuristic_evidence must never be called with StaticAnalysis")]
    fn heuristic_evidence_panics_if_given_static_analysis_method() {
        let _ = heuristic_evidence(ResolutionMethod::StaticAnalysis, Confidence::Certain, "note");
    }

    #[test]
    fn resolved_evidence_has_static_analysis_method() {
        let evidence = resolved_evidence("exact match");
        assert_eq!(evidence.method(), ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn target_lookup_excludes_the_sources_own_id() {
        let snapshot = test_snapshot("foo", "function_item", "fn foo() {}");
        let snapshots = vec![snapshot.clone()];
        let lookup = TargetLookup::new(&snapshots).unwrap();

        let found = lookup.find_by_name("foo", snapshot.id());

        assert!(found.is_empty(), "a snapshot must never resolve to itself as a by-name target");
    }

    #[test]
    fn target_lookup_finds_a_different_snapshot_by_name() {
        let a = test_snapshot("foo", "function_item", "fn foo() {}");
        let b = test_snapshot("helper", "function_item", "fn helper() {}");
        let snapshots = vec![a.clone(), b.clone()];
        let lookup = TargetLookup::new(&snapshots).unwrap();

        let found = lookup.find_by_name("helper", a.id());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), b.id());
    }

    #[test]
    fn target_lookup_rejects_a_duplicate_snapshot_id() {
        let snapshot = test_snapshot("foo", "function_item", "fn foo() {}");
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = TargetLookup::new(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }
}
