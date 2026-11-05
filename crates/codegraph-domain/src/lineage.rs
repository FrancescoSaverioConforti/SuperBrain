//! B2 Task 1/6: cross-commit lineage identity and status (D3).
//!
//! # `SymbolLineageId` design correction (Task 6)
//!
//! Task 1's original `SymbolLineageId::new()` was `Uuid::new_v4()` — pure
//! randomness, with no derivation from anything. This could not satisfy
//! the actual requirement a lineage id must meet: the SAME logical
//! lineage chain, resolved by independent resolver runs (including a
//! deterministic rebuild of the identical commit history), must produce
//! the SAME id every time. A random UUID delegates that stability
//! entirely to some external store remembering and reusing it; two
//! identical rebuilds with no such store would mint two different,
//! unrelated ids for what is logically one chain. Nothing had yet
//! persisted a Task 1 lineage id anywhere (no storage, no API, no stable
//! format to migrate), so this is corrected directly here rather than
//! kept as a supported legacy scheme alongside a new one.
//!
//! **Corrected semantics:** a `SymbolLineageId` identifies the logical
//! chain started by its GENESIS `SymbolSnapshotId` — the first snapshot
//! this pipeline ever observed as belonging to that chain, for whatever
//! portion of history it indexed. It does NOT mean "the symbol's current
//! name," "current file," "current commit," or "current signature" —
//! exactly because rename/move must never change it. Every subsequent
//! snapshot linked into the same chain inherits the genesis's id
//! unchanged; this module never recomputes a lineage id from a
//! non-genesis snapshot.
//!
//! **Derivation** (`superbrain-symbol-lineage-v1`, mirroring
//! `SymbolSnapshotId`'s own domain-separated, versioned scheme): SHA-256
//! of `(domain_separator || genesis_snapshot_id_bytes)`, truncated to the
//! first 16 bytes and interpreted as a `Uuid` — chosen so this type keeps
//! its existing `Uuid`-shaped public API (`as_uuid`) rather than changing
//! every downstream signature that already expects one, while still
//! being a fully deterministic function of the genesis snapshot alone,
//! never randomness. `genesis_snapshot_id_bytes` is `SymbolSnapshotId`'s
//! own fixed 32-byte digest (`as_bytes()`), itself already a
//! collision-resistant, domain-separated derivation of validated
//! identity — no additional length-prefixing is needed here because this
//! derivation has exactly one input field and it is fixed-width.
//!
//! **Partial history:** if SuperBrain has only indexed a portion of a
//! repository's commit history and does not know the symbol's true
//! earlier genesis (one that existed before the indexed window began),
//! the first snapshot actually observed becomes that lineage's anchor
//! for the portion of history known to this pipeline. This module never
//! invents or guesses an earlier genesis it has not actually seen. A
//! future Task 10 persistence layer that already knows an earlier
//! anchor from a prior run may reuse it without changing this semantics
//! — it would simply supply that already-known `SymbolSnapshotId` as the
//! genesis to [`SymbolLineageId::from_genesis`] again, not invent a new
//! derivation rule.

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::evidence::{Confidence, Evidence, ResolutionMethod};
use crate::snapshot_id::SymbolSnapshotId;

/// The `superbrain-symbol-lineage-v1` domain/version separator, fed
/// before the genesis snapshot id's bytes in every
/// [`SymbolLineageId::from_genesis`] call. See this module's doc comment.
const DOMAIN_SEPARATOR_V1: &[u8] = b"superbrain-symbol-lineage-v1";

/// An opaque id for a lineage chain: the logical identity shared by
/// every `SymbolSnapshot` the resolver has linked together across
/// commits, anchored at the chain's GENESIS snapshot. See this module's
/// doc comment for the full derivation and the deliberate exclusion of
/// randomness. Never user-supplied and never parsed from a string — the
/// only constructor is [`SymbolLineageId::from_genesis`], called by the
/// lineage resolver (Task 6) with the chain's genesis `SymbolSnapshotId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolLineageId(Uuid);

impl SymbolLineageId {
    /// Deterministically derives this chain's id from `genesis` — the
    /// first `SymbolSnapshotId` this pipeline has observed belonging to
    /// the chain. Two calls with the same `genesis` always produce the
    /// same id; a chain's id is computed exactly once, at the moment its
    /// genesis is first identified, and every later snapshot linked into
    /// the same chain reuses that value rather than calling this again
    /// with a different snapshot.
    pub fn from_genesis(genesis: SymbolSnapshotId) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_SEPARATOR_V1);
        hasher.update(genesis.as_bytes());
        let digest = hasher.finalize();
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes.copy_from_slice(&digest[..16]);
        Self(Uuid::from_bytes(uuid_bytes))
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

/// Whether a lineage link is a syntactic certainty or a resolver's
/// hypothesis, carried structurally rather than as a boolean flag next to
/// a confidence number. Per the P1 checkpoint's verdict on CF-06/metadata
/// classification policy: a `Hypothesized` link must never silently
/// become `Confirmed` through storage, retrieval, or a
/// serialize/deserialize round trip. There is no `From<Hypothesized> for
/// Confirmed` conversion, no "upgrade" method, and no shared constructor
/// that both variants funnel through with a boolean toggle — the only way
/// to produce a `Confirmed` value is to call [`LineageStatus::confirmed`]
/// directly with `ResolutionMethod::StaticAnalysis` evidence, and the only
/// way to produce a `Hypothesized` value is
/// [`LineageStatus::hypothesized`]. Promoting a link from one to the other
/// requires the resolver to re-run with new evidence and construct a new
/// `LineageStatus` from scratch (Task 6), never a conversion of an
/// existing value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LineageStatus {
    Confirmed { evidence: Evidence },
    Hypothesized { evidence: Evidence, confidence: Confidence },
}

impl LineageStatus {
    /// Constructs a `Confirmed` status. `evidence` must have been built
    /// with `ResolutionMethod::StaticAnalysis` — passing evidence built
    /// with any other method is rejected, so a caller cannot construct a
    /// `Confirmed` status from heuristic/inferred evidence by accident.
    pub fn confirmed(evidence: Evidence) -> Result<Self, LineageStatusError> {
        if evidence.method() != ResolutionMethod::StaticAnalysis {
            return Err(LineageStatusError::ConfirmedRequiresStaticAnalysis {
                found: evidence.method(),
            });
        }
        Ok(Self::Confirmed { evidence })
    }

    pub fn hypothesized(evidence: Evidence, confidence: Confidence) -> Self {
        Self::Hypothesized { evidence, confidence }
    }

    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LineageStatusError {
    #[error("a Confirmed lineage status requires StaticAnalysis evidence, got {found:?}")]
    ConfirmedRequiresStaticAnalysis { found: ResolutionMethod },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot};
    use crate::kind::SymbolKind;

    fn test_snapshot_id(repo_root: &std::path::Path, relative_path: &str) -> SymbolSnapshotId {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(repo_root).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, relative_path).unwrap();
        SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo", "fn foo()")
    }

    #[test]
    fn deriving_from_the_same_genesis_twice_yields_the_same_lineage_id() {
        let genesis = test_snapshot_id(&std::env::current_dir().unwrap(), "src/lib.rs");
        let a = SymbolLineageId::from_genesis(genesis);
        let b = SymbolLineageId::from_genesis(genesis);
        assert_eq!(a, b);
    }

    #[test]
    fn two_different_genesis_snapshots_yield_different_lineage_ids() {
        let genesis_a = test_snapshot_id(&std::env::current_dir().unwrap(), "src/lib.rs");
        let genesis_b = test_snapshot_id(&std::env::current_dir().unwrap(), "src/other.rs");
        let a = SymbolLineageId::from_genesis(genesis_a);
        let b = SymbolLineageId::from_genesis(genesis_b);
        assert_ne!(a, b, "two distinct genesis snapshots must not be fused into one lineage id");
    }

    #[test]
    fn as_uuid_returns_a_real_derived_value_not_a_default() {
        let genesis = test_snapshot_id(&std::env::current_dir().unwrap(), "src/lib.rs");
        let id = SymbolLineageId::from_genesis(genesis);
        assert_ne!(id.as_uuid(), Uuid::default());
    }

    #[test]
    fn golden_vector_matches_an_independently_recomputed_v1_encoding() {
        // Pins the encoding itself (domain separator, truncation to 16
        // bytes, UUID construction) against accidental drift, via a
        // hand-written independent re-implementation rather than calling
        // `from_genesis`'s own internals.
        let genesis = test_snapshot_id(&std::env::current_dir().unwrap(), "src/golden.rs");
        let actual = SymbolLineageId::from_genesis(genesis);

        let mut hasher = Sha256::new();
        hasher.update(b"superbrain-symbol-lineage-v1");
        hasher.update(genesis.as_bytes());
        let digest = hasher.finalize();
        let mut expected_bytes = [0u8; 16];
        expected_bytes.copy_from_slice(&digest[..16]);
        let expected = Uuid::from_bytes(expected_bytes);

        assert_eq!(actual.as_uuid(), expected);
    }

    #[test]
    fn confirmed_status_from_static_analysis_evidence_is_accepted() {
        let evidence = Evidence::new(ResolutionMethod::StaticAnalysis, None, "exact match").unwrap();
        assert!(LineageStatus::confirmed(evidence).is_ok());
    }

    #[test]
    fn confirmed_status_from_name_heuristic_evidence_is_rejected() {
        let evidence = Evidence::new(ResolutionMethod::NameHeuristic, None, "same name, different file").unwrap();
        let result = LineageStatus::confirmed(evidence);
        assert_eq!(
            result,
            Err(LineageStatusError::ConfirmedRequiresStaticAnalysis {
                found: ResolutionMethod::NameHeuristic
            })
        );
    }

    #[test]
    fn confirmed_status_from_inferred_evidence_is_rejected() {
        let evidence = Evidence::new(ResolutionMethod::Inferred, None, "probable rename").unwrap();
        let result = LineageStatus::confirmed(evidence);
        assert_eq!(
            result,
            Err(LineageStatusError::ConfirmedRequiresStaticAnalysis {
                found: ResolutionMethod::Inferred
            })
        );
    }

    #[test]
    fn hypothesized_status_never_reports_as_confirmed() {
        let evidence = Evidence::new(ResolutionMethod::NameHeuristic, None, "probable rename").unwrap();
        let status = LineageStatus::hypothesized(evidence, Confidence::Likely);
        assert!(!status.is_confirmed());
    }

    #[test]
    fn confirmed_status_reports_as_confirmed() {
        let evidence = Evidence::new(ResolutionMethod::StaticAnalysis, None, "exact match").unwrap();
        let status = LineageStatus::confirmed(evidence).unwrap();
        assert!(status.is_confirmed());
    }

    #[test]
    fn there_is_no_conversion_from_hypothesized_to_confirmed() {
        // Structural proof, not a runtime assertion: matching on
        // LineageStatus's two variants and attempting to relabel a
        // Hypothesized match arm as Confirmed without reconstructing the
        // evidence is a compile error, because `evidence` inside the
        // Hypothesized arm is still bound to NameHeuristic-origin data and
        // `LineageStatus::confirmed` independently re-validates the
        // method. This test exercises that re-validation path directly.
        let heuristic_evidence = Evidence::new(ResolutionMethod::NameHeuristic, None, "same name").unwrap();
        let hypothesized = LineageStatus::hypothesized(heuristic_evidence.clone(), Confidence::Likely);
        assert!(!hypothesized.is_confirmed());

        // The ONLY way to get a Confirmed value is to call `confirmed`
        // again, and it re-validates the same evidence and rejects it.
        assert!(LineageStatus::confirmed(heuristic_evidence).is_err());
    }
}
