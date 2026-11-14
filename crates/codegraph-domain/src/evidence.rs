//! B2 Task 1: evidence and confidence for edges/lineage whose resolution is
//! not a syntactic certainty.

use thiserror::Error;

use crate::identity::FileIdentity;
use crate::snapshot_id::SymbolSnapshotId;

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

/// Quality & Semantic Reliability Hardening, Phase 2 — a uniform
/// evidence tier for ANY resolved-or-not relationship this pipeline
/// produces: an `Edge`, a structural fact's own resolution status
/// (`ImportResolution`, `ImplTargetResolution`), or (new in Phase 2) a
/// `codegraph_index::negative_knowledge::CallAttempt` negative-
/// knowledge record (that type lives in the `codegraph-index` crate,
/// not here, so it cannot be an intra-doc link from this crate).
/// Generalizes a pattern that already existed
/// independently in `ImportResolution`/`ImplTargetResolution` — this
/// is substantially a relabeling of proven prior art, not a new
/// invention; see `docs/research/SEMANTIC_UNCERTAINTY_MODEL.md` for
/// the full audit this type is derived from.
///
/// Deliberately four flat variants, no numeric confidence score: see
/// that design document's own "What this model explicitly rejects"
/// section for why a fifth "Confirmed/strongly-grounded" tier above
/// `Resolved` is NOT included here (nothing in this codebase today
/// could ever produce it — a future differential-testing oracle
/// agreeing with this pipeline's own resolver is the design doc's own
/// named future home for that tier, not fabricated now with no real
/// producer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvidenceTier {
    /// Exactly one real candidate existed, and the relationship
    /// follows from syntax/name-matching alone with no ambiguity left
    /// among the candidates a resolver was given. Mirrors today's
    /// `ResolutionMethod::StaticAnalysis` / `ImportResolution::Internal`
    /// / `ImplTargetResolution::Resolved`.
    Resolved,
    /// A naming-based match was found, but the matching process
    /// itself involved a non-syntactic judgment call (e.g. scope
    /// narrowing picked the nearest enclosing class/file among
    /// several same-named candidates). Mirrors today's
    /// `ResolutionMethod::NameHeuristic`.
    Heuristic,
    /// Two or more real candidates existed and the resolver DECLINED
    /// to pick one — never silently collapsed to the first or an
    /// arbitrary candidate. Mirrors today's `ImportResolution::Ambiguous`
    /// / `ImplTargetResolution::Ambiguous`.
    Ambiguous,
    /// Zero real candidates existed in the given pool. The real target
    /// may be external (stdlib/third-party/not-yet-indexed) or may
    /// simply not exist. Mirrors today's `ImportResolution::External`
    /// / `ImplTargetResolution::Unresolved`.
    Unresolved,
}

impl EvidenceTier {
    /// The direct, lossless mapping from today's `ResolutionMethod` —
    /// the migration sketch `SEMANTIC_UNCERTAINTY_MODEL.md` already
    /// specified, implemented here as the one place that mapping
    /// actually lives in code. `Inferred` (lineage-only today) maps to
    /// `Heuristic`, per that document's own reasoning: lineage's own
    /// `Hypothesized` status already carries a separate `Confidence`
    /// this mapping deliberately leaves untouched.
    pub fn from_resolution_method(method: ResolutionMethod) -> Self {
        match method {
            ResolutionMethod::StaticAnalysis => Self::Resolved,
            ResolutionMethod::NameHeuristic => Self::Heuristic,
            ResolutionMethod::Inferred => Self::Heuristic,
        }
    }
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

/// Maximum number of candidate ids [`Provenance::candidates_considered`]
/// may carry, and the maximum byte length of
/// [`Provenance::selection_rationale`] — the same bounded-free-text
/// discipline [`Evidence::note`] already applies, extended to this
/// type's own two unbounded-by-construction fields. A resolver that
/// genuinely considered more candidates than this bound must still
/// report the bound-worth it keeps (truncated, never silently dropped
/// without a trace) — see [`Provenance::new`]'s own doc comment.
pub const MAX_CANDIDATES_RECORDED: usize = 64;
pub const MAX_RATIONALE_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProvenanceError {
    #[error("selection_rationale is {actual} bytes long, maximum is {max}")]
    RationaleTooLong { max: usize, actual: usize },
}

/// Quality & Semantic Reliability Hardening, Phase 2 — structured
/// provenance for a resolution outcome, generalizing [`Evidence::note`]'s
/// own free-text content into queryable fields, per
/// `docs/research/SEMANTIC_UNCERTAINTY_MODEL.md`'s own design.
/// Deliberately bounded and memory-conscious — this is NOT a full AST
/// trace per edge: `candidates_considered` is capped at
/// [`MAX_CANDIDATES_RECORDED`] ids (a resolver with more real
/// candidates than that is itself evidence of a pathological case
/// worth knowing about via `candidates_considered_total_count`, not a
/// reason to store every one of them), and `selection_rationale` is a
/// short bounded string, not a structured reasoning trace (the design
/// document's own "Residual limitations" section names fully
/// structuring this as real future work, not attempted here).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Provenance {
    /// Which resolver/extractor produced this determination (e.g.
    /// `"resolve_calls"`, `"resolve_extends_implements_with_csharp_contexts"`).
    /// A plain, caller-supplied string naming the real function, never
    /// a central enum — a new resolver must not require editing this
    /// type just to report its own provenance.
    resolver: String,
    /// Every real candidate this resolver actually considered, up to
    /// [`MAX_CANDIDATES_RECORDED`]. For an `EvidenceTier::Resolved`
    /// outcome, this is always exactly one id — confirming there never
    /// was a second real candidate, not merely that one was picked.
    /// For `Ambiguous`, this is what lets a consumer inspect the real
    /// options instead of only knowing "it was ambiguous."
    candidates_considered: Vec<SymbolSnapshotId>,
    /// The TRUE total number of real candidates considered, even when
    /// it exceeds [`MAX_CANDIDATES_RECORDED`] and `candidates_considered`
    /// itself had to be truncated — so a consumer can always tell
    /// "I am seeing all of them" from "I am seeing a bounded sample of
    /// a much larger set" without that distinction depending on
    /// counting the (possibly truncated) `Vec` itself.
    candidates_considered_total_count: usize,
    /// Why THIS candidate (if any) was chosen over the others, in the
    /// same structured terms this project's own real evidence notes
    /// already use (e.g. `"narrowed to caller's own enclosing class"`).
    /// `None` when there was no choice to make (zero or exactly one
    /// candidate).
    selection_rationale: Option<String>,
    /// Known limitations of the METHOD used (not the specific
    /// instance) — e.g. `"name-based matching cannot distinguish
    /// overloads by parameter list or generic arity"` attached to
    /// every `Heuristic`-tier `Calls` edge in a domain with real
    /// overloads, independent of which specific call site this is.
    method_limitations: Vec<String>,
}

impl Provenance {
    /// Builds a `Provenance`. `candidates_considered` is truncated to
    /// [`MAX_CANDIDATES_RECORDED`] if longer — the TRUE count is always
    /// preserved separately (see
    /// [`Provenance::candidates_considered_total_count`]'s own doc
    /// comment), so truncation never silently loses the information
    /// that truncation happened.
    pub fn new(resolver: impl Into<String>, candidates_considered: Vec<SymbolSnapshotId>, selection_rationale: Option<String>, method_limitations: Vec<String>) -> Result<Self, ProvenanceError> {
        if let Some(rationale) = &selection_rationale {
            if rationale.len() > MAX_RATIONALE_LEN {
                return Err(ProvenanceError::RationaleTooLong { max: MAX_RATIONALE_LEN, actual: rationale.len() });
            }
        }
        let candidates_considered_total_count = candidates_considered.len();
        let mut truncated = candidates_considered;
        truncated.truncate(MAX_CANDIDATES_RECORDED);
        Ok(Self { resolver: resolver.into(), candidates_considered: truncated, candidates_considered_total_count, selection_rationale, method_limitations })
    }

    pub fn resolver(&self) -> &str {
        &self.resolver
    }

    pub fn candidates_considered(&self) -> &[SymbolSnapshotId] {
        &self.candidates_considered
    }

    /// The true total, even if `candidates_considered()` itself was
    /// truncated to `MAX_CANDIDATES_RECORDED` — always
    /// `>= candidates_considered().len()`, and strictly greater exactly
    /// when truncation occurred.
    pub fn candidates_considered_total_count(&self) -> usize {
        self.candidates_considered_total_count
    }

    /// `true` when `candidates_considered()` does NOT reflect every
    /// real candidate this resolver saw — i.e. the caller must not
    /// assume the returned slice is exhaustive.
    pub fn candidates_were_truncated(&self) -> bool {
        self.candidates_considered.len() < self.candidates_considered_total_count
    }

    pub fn selection_rationale(&self) -> Option<&str> {
        self.selection_rationale.as_deref()
    }

    pub fn method_limitations(&self) -> &[String] {
        &self.method_limitations
    }
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

    // --- EvidenceTier ---

    #[test]
    fn static_analysis_maps_to_resolved() {
        assert_eq!(EvidenceTier::from_resolution_method(ResolutionMethod::StaticAnalysis), EvidenceTier::Resolved);
    }

    #[test]
    fn name_heuristic_maps_to_heuristic() {
        assert_eq!(EvidenceTier::from_resolution_method(ResolutionMethod::NameHeuristic), EvidenceTier::Heuristic);
    }

    #[test]
    fn inferred_maps_to_heuristic_not_a_separate_tier() {
        // Per the design doc's own reasoning: lineage's Hypothesized
        // status already carries a separate Confidence this mapping
        // deliberately leaves untouched; Inferred is not promoted to
        // its own EvidenceTier variant.
        assert_eq!(EvidenceTier::from_resolution_method(ResolutionMethod::Inferred), EvidenceTier::Heuristic);
    }

    // --- Provenance ---

    fn test_snapshot_id(name: &str) -> SymbolSnapshotId {
        use crate::kind::SymbolKind;
        SymbolSnapshotId::derive(&test_file_identity(), SymbolKind::Function, name, "fn x() {}", 0, 10)
    }

    #[test]
    fn provenance_with_few_candidates_is_not_truncated() {
        let ids = vec![test_snapshot_id("a"), test_snapshot_id("b")];
        let provenance = Provenance::new("resolve_calls", ids.clone(), Some("narrowed to caller's own enclosing class".to_string()), vec![]).unwrap();
        assert_eq!(provenance.candidates_considered(), ids.as_slice());
        assert_eq!(provenance.candidates_considered_total_count(), 2);
        assert!(!provenance.candidates_were_truncated());
        assert_eq!(provenance.resolver(), "resolve_calls");
        assert_eq!(provenance.selection_rationale(), Some("narrowed to caller's own enclosing class"));
    }

    #[test]
    fn provenance_with_more_candidates_than_the_bound_is_truncated_but_true_count_is_preserved() {
        let ids: Vec<_> = (0..(MAX_CANDIDATES_RECORDED + 10)).map(|i| test_snapshot_id(&format!("candidate_{i}"))).collect();
        let real_total = ids.len();
        let provenance = Provenance::new("resolve_calls", ids, None, vec![]).unwrap();
        assert_eq!(provenance.candidates_considered().len(), MAX_CANDIDATES_RECORDED, "the stored slice must be truncated to the bound");
        assert_eq!(provenance.candidates_considered_total_count(), real_total, "the TRUE total must be preserved even when truncated");
        assert!(provenance.candidates_were_truncated());
    }

    #[test]
    fn provenance_at_exactly_the_bound_is_not_marked_truncated() {
        let ids: Vec<_> = (0..MAX_CANDIDATES_RECORDED).map(|i| test_snapshot_id(&format!("candidate_{i}"))).collect();
        let provenance = Provenance::new("resolve_calls", ids, None, vec![]).unwrap();
        assert!(!provenance.candidates_were_truncated(), "exactly at the bound must not be reported as truncated");
    }

    #[test]
    fn oversized_selection_rationale_is_rejected_not_truncated() {
        let huge = "a".repeat(MAX_RATIONALE_LEN + 1);
        let result = Provenance::new("resolve_calls", vec![], Some(huge), vec![]);
        assert!(matches!(result, Err(ProvenanceError::RationaleTooLong { .. })));
    }

    #[test]
    fn a_resolved_outcome_has_exactly_one_candidate_recorded() {
        // The design doc's own invariant: for a Resolved outcome,
        // candidates_considered must be exactly one -- confirming
        // there never was a second real candidate, not merely that
        // one was picked.
        let provenance = Provenance::new("resolve_calls", vec![test_snapshot_id("only_one")], None, vec![]).unwrap();
        assert_eq!(provenance.candidates_considered().len(), 1);
        assert_eq!(provenance.candidates_considered_total_count(), 1);
    }

    #[test]
    fn method_limitations_are_preserved_verbatim() {
        let provenance = Provenance::new("resolve_calls", vec![], None, vec!["name-based matching cannot distinguish overloads by parameter list or generic arity".to_string()]).unwrap();
        assert_eq!(provenance.method_limitations(), &["name-based matching cannot distinguish overloads by parameter list or generic arity".to_string()]);
    }
}
