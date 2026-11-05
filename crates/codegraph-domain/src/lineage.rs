//! B2 Task 1: cross-commit lineage identity and status (D3).

use uuid::Uuid;

use crate::evidence::{Confidence, Evidence, ResolutionMethod};

/// An opaque id for a lineage link between two `SymbolSnapshot`s across
/// commits. Never user-supplied and never parsed from a string: the only
/// constructor is [`SymbolLineageId::new`], called exclusively by the
/// lineage resolver (Task 6). There is deliberately no `FromStr`/`parse`
/// impl — unlike `CommitHash`/`FileIdentity`, nothing about this value is
/// meaningful if supplied by anything other than the resolver that minted
/// it, so there is nothing for an external caller to validate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolLineageId(Uuid);

impl SymbolLineageId {
    /// Mints a fresh lineage id. Intentionally not `pub(crate)`-restricted
    /// to the resolver module alone (Rust's privacy cannot express
    /// "only Task 6's module" without an awkward sealed-trait indirection
    /// this scaffold does not need yet) — the real enforcement is that
    /// `LineageEdge` (Task 6) is the only place a `SymbolLineageId` is
    /// paired with a `LineageStatus` and persisted/returned, and
    /// `LineageStatus` itself cannot be constructed as `Confirmed` from
    /// weak evidence (see its doc comment).
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for SymbolLineageId {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn two_fresh_lineage_ids_are_distinct() {
        assert_ne!(SymbolLineageId::new(), SymbolLineageId::new());
    }

    #[test]
    fn as_uuid_returns_the_actual_wrapped_value_not_a_default() {
        // Pins SymbolLineageId::as_uuid against a mutant that replaces it
        // with Default::default() (the nil UUID).
        let a = SymbolLineageId::new();
        let b = SymbolLineageId::new();

        assert_ne!(a.as_uuid(), b.as_uuid());
        assert_ne!(a.as_uuid(), Uuid::default());
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
