//! B2 Task 1: evidence and confidence for edges/lineage whose resolution is
//! not a syntactic certainty.

use thiserror::Error;

use crate::identity::FileIdentity;

/// How an edge or lineage link was resolved. `StaticAnalysis` is the only
/// method that may accompany `LineageStatus::Confirmed`/a resolved
/// (non-heuristic) edge; `NameHeuristic` and `Inferred` may only accompany
/// a `Hypothesized` lineage status or a heuristic edge classification —
/// enforced at the call site that constructs those (Task 6/Task 7), not
/// here (this type only records which method was used, it does not itself
/// gate what status that method is allowed to justify).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolutionMethod {
    StaticAnalysis,
    NameHeuristic,
    Inferred,
}

/// A closed confidence bucket, never a raw float exposed without one. See
/// [`Confidence::from_score`] for the only way to derive a bucket from a
/// continuous similarity score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    Certain,
    Likely,
    Speculative,
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ConfidenceError {
    #[error("confidence score must be finite, got {0}")]
    NotFinite(f64),
    #[error("confidence score must be within [0.0, 1.0], got {0}")]
    OutOfRange(f64),
}

impl Confidence {
    /// Buckets a continuous `[0.0, 1.0]` similarity score into a closed
    /// `Confidence` variant. Rejects NaN and out-of-range scores outright
    /// rather than clamping them into the nearest valid bucket — a caller
    /// that computed a broken score (e.g. a similarity metric that
    /// produced NaN from empty input) must see a typed error, not a
    /// silently "Certain" or "Speculative" result that looks like a real
    /// measurement.
    pub fn from_score(score: f64) -> Result<Self, ConfidenceError> {
        if score.is_nan() {
            return Err(ConfidenceError::NotFinite(score));
        }
        if !score.is_finite() {
            return Err(ConfidenceError::NotFinite(score));
        }
        if !(0.0..=1.0).contains(&score) {
            return Err(ConfidenceError::OutOfRange(score));
        }
        Ok(if score >= 0.85 {
            Self::Certain
        } else if score >= 0.5 {
            Self::Likely
        } else {
            Self::Speculative
        })
    }
}

/// Maximum accepted length, in bytes, of [`Evidence::note`]. Bounds the one
/// free-text field this type carries — the same IX-05/unbounded-text
/// concern the B2 plan calls out for `RawSymbolCandidate.signature_text`,
/// applied here since `note` is the other place free text could otherwise
/// grow without limit.
const MAX_NOTE_LEN: usize = 512;

/// Evidence for an edge or lineage link: how it was resolved, optionally
/// where in the source that resolution came from, and a short, bounded
/// human-readable note. Never carries raw, unbounded user/source content —
/// `note` is capped at construction, mirroring `ApiError`'s sanitized
/// `Display` discipline (never echo arbitrary underlying content verbatim
/// and unbounded).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Evidence {
    method: ResolutionMethod,
    source_span: Option<(FileIdentity, LineRange)>,
    note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EvidenceError {
    #[error("evidence note is {actual} bytes long, maximum is {max}")]
    NoteTooLong { max: usize, actual: usize },
    #[error("line range end ({end}) must not be before start ({start})")]
    InvertedLineRange { start: u32, end: u32 },
}

impl Evidence {
    pub fn new(
        method: ResolutionMethod,
        source_span: Option<(FileIdentity, LineRange)>,
        note: &str,
    ) -> Result<Self, EvidenceError> {
        if note.len() > MAX_NOTE_LEN {
            return Err(EvidenceError::NoteTooLong {
                max: MAX_NOTE_LEN,
                actual: note.len(),
            });
        }
        if let Some((_, range)) = &source_span {
            if range.end < range.start {
                return Err(EvidenceError::InvertedLineRange {
                    start: range.start,
                    end: range.end,
                });
            }
        }
        Ok(Self {
            method,
            source_span,
            note: note.to_string(),
        })
    }

    pub fn method(&self) -> ResolutionMethod {
        self.method
    }

    pub fn source_span(&self) -> Option<&(FileIdentity, LineRange)> {
        self.source_span.as_ref()
    }

    pub fn note(&self) -> &str {
        &self.note
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{CommitHash, RepositoryRegistry, RepositoryRoot};

    fn test_file_identity() -> FileIdentity {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        FileIdentity::parse(repo, commit, "src/lib.rs").unwrap()
    }

    // --- Confidence ---

    #[test]
    fn high_score_buckets_to_certain() {
        assert_eq!(Confidence::from_score(0.9), Ok(Confidence::Certain));
        assert_eq!(Confidence::from_score(1.0), Ok(Confidence::Certain));
    }

    #[test]
    fn mid_score_buckets_to_likely() {
        assert_eq!(Confidence::from_score(0.6), Ok(Confidence::Likely));
    }

    #[test]
    fn low_score_buckets_to_speculative() {
        assert_eq!(Confidence::from_score(0.1), Ok(Confidence::Speculative));
        assert_eq!(Confidence::from_score(0.0), Ok(Confidence::Speculative));
    }

    #[test]
    fn nan_score_is_rejected_not_bucketed() {
        // NaN != NaN by IEEE 754, so assert_eq! against
        // Err(ConfidenceError::NotFinite(f64::NAN)) can never pass even on
        // a correct implementation — match the variant and separately
        // assert the inner value is NaN via `is_nan()`.
        match Confidence::from_score(f64::NAN) {
            Err(ConfidenceError::NotFinite(value)) => assert!(value.is_nan()),
            other => panic!("expected Err(ConfidenceError::NotFinite(NaN)), got {other:?}"),
        }
    }

    #[test]
    fn infinite_score_is_rejected_not_bucketed() {
        assert_eq!(
            Confidence::from_score(f64::INFINITY),
            Err(ConfidenceError::NotFinite(f64::INFINITY))
        );
    }

    #[test]
    fn negative_score_is_rejected_not_clamped_to_speculative() {
        assert_eq!(Confidence::from_score(-0.1), Err(ConfidenceError::OutOfRange(-0.1)));
    }

    #[test]
    fn above_one_score_is_rejected_not_clamped_to_certain() {
        assert_eq!(Confidence::from_score(1.1), Err(ConfidenceError::OutOfRange(1.1)));
    }

    // --- Evidence ---

    #[test]
    fn evidence_without_source_span_is_accepted_and_accessor_reports_none() {
        let evidence = Evidence::new(ResolutionMethod::Inferred, None, "heuristic match").unwrap();
        assert_eq!(evidence.source_span(), None);
    }

    #[test]
    fn evidence_with_valid_source_span_is_accepted_and_accessor_returns_it() {
        let span = (test_file_identity(), LineRange { start: 10, end: 12 });
        let evidence = Evidence::new(ResolutionMethod::StaticAnalysis, Some(span.clone()), "resolved call").unwrap();
        assert_eq!(evidence.source_span(), Some(&span));
    }

    #[test]
    fn note_accessor_returns_the_exact_note_text() {
        let evidence = Evidence::new(ResolutionMethod::Inferred, None, "a specific note").unwrap();
        assert_eq!(evidence.note(), "a specific note");
    }

    #[test]
    fn oversized_note_is_rejected_not_truncated() {
        let huge_note = "a".repeat(MAX_NOTE_LEN + 1);
        let result = Evidence::new(ResolutionMethod::Inferred, None, &huge_note);
        assert_eq!(
            result,
            Err(EvidenceError::NoteTooLong {
                max: MAX_NOTE_LEN,
                actual: MAX_NOTE_LEN + 1
            })
        );
    }

    #[test]
    fn inverted_line_range_is_rejected() {
        let span = (test_file_identity(), LineRange { start: 20, end: 10 });
        let result = Evidence::new(ResolutionMethod::StaticAnalysis, Some(span), "note");
        assert_eq!(result, Err(EvidenceError::InvertedLineRange { start: 20, end: 10 }));
    }
}
