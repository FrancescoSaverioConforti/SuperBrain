//! Quality & Semantic Reliability Hardening, Phase 2 — preserving
//! "negative knowledge": today, per
//! `docs/research/SEMANTIC_UNCERTAINTY_MODEL.md`'s own central
//! finding, every typed-edge resolver's "no edge beats a false edge"
//! discipline means `Ambiguous` and `Unresolved` outcomes are
//! completely INVISIBLE once a resolver finishes — they collapse to
//! the same observable state (absence of an edge) as "this pipeline
//! never looked at all." This module introduces the ONE genuinely new
//! piece of data model that design document proposed: a record kept
//! ALONGSIDE the edge set (never inside `CodeGraph`'s own edges —
//! `Edge` still requires a real `(from, to)` pair, and an ambiguous
//! attempt has one `from` but MULTIPLE possible `to`s) so a consumer
//! can distinguish "genuinely no relationship" from "genuinely
//! ambiguous between real candidates" from "zero real candidates
//! existed."
//!
//! # Scope of this Phase 2 pass
//!
//! Wired into `resolve_calls` only (via the new, additive
//! `resolve_calls_with_negative_knowledge`), as the single resolver
//! this evaluation series has repeatedly confirmed has the most
//! heavily-used and most consequential ambiguity/unresolved gap (every
//! overload-collision and same-name-collision finding this project
//! has produced — B2-ID-004, the generic-arity-collision gap named in
//! B2-ID-010's own closing report, CRC-001/002/003 — is a `Calls`
//! resolution question). `resolve_extends_implements`/
//! `resolve_instantiates`/`resolve_tests_for` are NOT wired in this
//! pass; the pattern established here generalizes directly if a future
//! pass extends it, but doing so now for every resolver would be a
//! much larger, un-reviewed surface than this phase's own scope.
//!
//! The ORIGINAL `resolve_calls` is completely unchanged in behavior —
//! `resolve_calls_with_negative_knowledge` duplicates none of its
//! scope-narrowing logic (reusing `narrow_by_scope`/`enclosing_class_of`
//! directly) and produces the IDENTICAL `Vec<Edge>` `resolve_calls`
//! itself would, plus the new negative-knowledge records as a second,
//! additive output.

use codegraph_domain::{EvidenceTier, Provenance, SymbolSnapshotId};

/// Semantic Resolution & Reliability Checkpoint, Priority 2: WHY a
/// `CallAttempt`'s own tier ended up `Unresolved` (or, for
/// `AmbiguousCandidates`, `Ambiguous`) — the minimum reason model
/// needed to stop collapsing structurally different causes into one
/// indistinguishable "no edge, no explanation" outcome.
///
/// # Discipline: never fabricate a reason the resolver cannot actually
/// observe
///
/// Every variant here is constructed ONLY where the resolver that
/// produces it has REAL, direct evidence for that specific cause —
/// never inferred from absence alone. `Unknown` exists precisely so a
/// caller that genuinely cannot distinguish WHY zero candidates
/// matched (the ordinary, overwhelmingly common case today — most
/// unresolved callees are simply outside the given snapshot pool for
/// no reason this resolver tracks) reports that honestly, rather than
/// this module inventing a false-specific classification to look more
/// complete than it is. Each variant's own doc comment below states
/// EXACTLY what real, checkable condition must hold before a caller
/// may construct it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnresolvedReason {
    /// Zero candidates matched, and the resolver has no further basis
    /// to distinguish "genuinely external" (stdlib/third-party/not
    /// yet indexed) from any other zero-candidate cause — the honest,
    /// default classification for the ordinary case where nothing
    /// more specific is actually known. This is NOT a weaker claim
    /// than the other variants below; it is the CORRECT claim whenever
    /// a caller has not verified one of the more specific conditions.
    Unknown,
    /// Zero candidates matched the callee name, AND this resolver was
    /// separately told (by its own caller, who must have checked a
    /// real `CandidateRejection` record — see
    /// `CallAttempt::unresolved_with_extraction_rejection`'s own doc
    /// comment) that a real extraction candidate sharing this exact
    /// name WAS found in the source but dropped before promotion
    /// (`SignatureTooLong`/`NameTooLong`). This is the Priority 3
    /// linkage: an extraction rejection that would otherwise look
    /// identical to a genuine external target.
    ExtractionRejected { rejection_kind: &'static str },
    /// 2+ real candidates existed and the resolver declined to pick
    /// one — mirrors `EvidenceTier::Ambiguous` exactly; kept as an
    /// `UnresolvedReason` variant too so a caller inspecting ONLY the
    /// reason enum (without separately checking `tier`) still sees an
    /// honest, non-`Unknown` explanation for why no edge resulted.
    AmbiguousCandidates,
    /// CALL-RES-A4: the call site's own callee name matched a real
    /// import-alias mapping in the caller's own file (an `ImportFact`
    /// whose `alias` field equals this exact callee name), AND that
    /// alias's own real imported-symbol name was independently
    /// confirmed, via the SAME `ImportFact`, to resolve to
    /// `ImportResolution::External` (a standard-library/third-party/
    /// not-yet-indexed target — never guessed). This is a DIFFERENT,
    /// more specific claim than plain `Unknown`: the resolver did not
    /// merely fail to find a match — it found the real alias mapping
    /// and confirmed the aliased target is genuinely outside the given
    /// snapshot pool.
    ImportTargetExternal,
    /// CALL-RES-A4: the call site's own callee name matched a real
    /// import-alias mapping in the caller's own file, but that alias's
    /// own `ImportFact` resolution was `Ambiguous` (2+ same-named
    /// candidates) rather than a single real target — the alias exists
    /// and names a real symbol, but WHICH symbol cannot be determined
    /// from import evidence alone.
    ImportedSymbolAmbiguous,
    /// CALL-RES-A4: the call site's own callee name does not match any
    /// real candidate directly, AND it does not match any import
    /// alias in the caller's own file's `ImportFact`s either — i.e.
    /// import evidence was consulted and found NOTHING relevant to
    /// this exact name, as opposed to `Unknown`, where import evidence
    /// was never consulted at all (a caller using the A4-aware entry
    /// point should prefer this reason over `Unknown` once it has
    /// confirmed the negative — see `resolve_calls_with_imports`'s own
    /// doc comment for exactly when this applies).
    NoMatchingImportAlias,
}

impl UnresolvedReason {
    /// A short, human-readable description — never used to RE-DERIVE
    /// the reason (callers should match on the enum itself), only to
    /// populate `Provenance::method_limitations`'s own free-text slot
    /// consistently.
    pub fn description(&self) -> String {
        match self {
            Self::Unknown => "zero candidates matched this callee name in the given snapshot pool, for a reason this resolver does not further distinguish".to_string(),
            Self::ExtractionRejected { rejection_kind } => format!("a real candidate sharing this exact callee name was found in the source but rejected at extraction time ({rejection_kind}), never reaching the snapshot pool this resolver was given"),
            Self::AmbiguousCandidates => "name-based matching cannot distinguish overloads by parameter list or generic arity; 2+ same-named candidates remained even after scope/arity narrowing".to_string(),
            Self::ImportTargetExternal => "the callee name matched a real import alias in the caller's own file, and that alias's own real target was confirmed to be external (standard-library/third-party/not-yet-indexed), never guessed".to_string(),
            Self::ImportedSymbolAmbiguous => "the callee name matched a real import alias in the caller's own file, but the alias's own real target name matches 2+ candidates in the given pool -- the alias is real, its target is not uniquely determinable from import evidence alone".to_string(),
            Self::NoMatchingImportAlias => "the callee name matched no real candidate directly AND no import alias in the caller's own file -- import evidence was consulted and found nothing relevant to this exact name".to_string(),
        }
    }
}

/// One call site's own resolution ATTEMPT, regardless of outcome —
/// kept even when `tier` is `Ambiguous`/`Unresolved` and NO `Edge` was
/// ever produced for it. This is deliberately NOT keyed the same way
/// an `Edge` is (`Edge` needs exactly one `to`); `candidates` here can
/// be empty (`Unresolved`), exactly one (would have produced a real
/// `Edge` — `CallAttempt` is not normally constructed for this case by
/// `resolve_calls_with_negative_knowledge`, see its own doc comment),
/// or 2+ (`Ambiguous`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallAttempt {
    pub caller: SymbolSnapshotId,
    pub callee_name: String,
    pub tier: EvidenceTier,
    pub candidates: Vec<SymbolSnapshotId>,
    pub provenance: Provenance,
    /// Semantic Resolution & Reliability Checkpoint, Priority 2: the
    /// specific, honestly-determined cause behind this attempt's own
    /// `tier`. See [`UnresolvedReason`]'s own doc comment for the
    /// never-fabricate discipline this field is built under.
    pub reason: UnresolvedReason,
}

impl CallAttempt {
    /// Builds an `Unresolved` attempt with [`UnresolvedReason::Unknown`]
    /// — the correct, honest default for the ordinary case where no
    /// more specific cause has been separately verified. Use
    /// [`CallAttempt::unresolved_with_extraction_rejection`] instead
    /// when a real `CandidateRejection` for this exact name is
    /// independently known.
    pub fn unresolved(caller: SymbolSnapshotId, callee_name: String) -> Self {
        Self::unresolved_with_reason(caller, callee_name, UnresolvedReason::Unknown)
    }

    /// Semantic Resolution & Reliability Checkpoint, Priority 3: builds
    /// an `Unresolved` attempt whose reason is
    /// [`UnresolvedReason::ExtractionRejected`] — the caller (not this
    /// module) must have already confirmed, against a real
    /// `CandidateRejection` from the SAME extraction pass, that a
    /// candidate sharing this exact `callee_name` was dropped before
    /// promotion. This function performs no such lookup itself (it has
    /// no access to the rejection list) — it only records what the
    /// caller already verified, per this module's own "never fabricate"
    /// discipline. `rejection_kind` is the rejection's own discriminant
    /// name (`"SignatureTooLong"`/`"NameTooLong"`), not a free-text
    /// guess.
    pub fn unresolved_with_extraction_rejection(caller: SymbolSnapshotId, callee_name: String, rejection_kind: &'static str) -> Self {
        Self::unresolved_with_reason(caller, callee_name, UnresolvedReason::ExtractionRejected { rejection_kind })
    }

    fn unresolved_with_reason(caller: SymbolSnapshotId, callee_name: String, reason: UnresolvedReason) -> Self {
        let provenance = Provenance::new("resolve_calls", vec![], None, vec![reason.description()]).expect("built short, within bounds");
        Self { caller, callee_name, tier: EvidenceTier::Unresolved, candidates: vec![], provenance, reason }
    }

    pub fn ambiguous(caller: SymbolSnapshotId, callee_name: String, candidates: Vec<SymbolSnapshotId>) -> Self {
        let reason = UnresolvedReason::AmbiguousCandidates;
        let provenance = Provenance::new("resolve_calls", candidates.clone(), None, vec![reason.description()]).expect("built short, within bounds");
        Self { caller, callee_name, tier: EvidenceTier::Ambiguous, candidates, provenance, reason }
    }

    /// CALL-RES-A4: builds an `Unresolved` attempt with
    /// [`UnresolvedReason::ImportTargetExternal`] — the caller (not this
    /// module) must have already confirmed, against a real `ImportFact`
    /// from the SAME file, that this exact callee name is a real import
    /// alias whose own target resolved to `ImportResolution::External`.
    pub fn unresolved_import_target_external(caller: SymbolSnapshotId, callee_name: String) -> Self {
        Self::unresolved_with_reason(caller, callee_name, UnresolvedReason::ImportTargetExternal)
    }

    /// CALL-RES-A4: builds an `Unresolved` attempt with
    /// [`UnresolvedReason::NoMatchingImportAlias`] — the caller must
    /// have already confirmed that this exact callee name matches
    /// neither a direct candidate nor any import alias in the caller's
    /// own file.
    pub fn unresolved_no_matching_import_alias(caller: SymbolSnapshotId, callee_name: String) -> Self {
        Self::unresolved_with_reason(caller, callee_name, UnresolvedReason::NoMatchingImportAlias)
    }

    /// CALL-RES-A4: builds an `Ambiguous` attempt with
    /// [`UnresolvedReason::ImportedSymbolAmbiguous`] — the caller must
    /// have already confirmed, via a real `ImportFact`, that this exact
    /// callee name is a real import alias whose own target resolved to
    /// `ImportResolution::Ambiguous` (2+ same-named candidates).
    pub fn ambiguous_import_target(caller: SymbolSnapshotId, callee_name: String, candidates: Vec<SymbolSnapshotId>) -> Self {
        let reason = UnresolvedReason::ImportedSymbolAmbiguous;
        let provenance = Provenance::new("resolve_calls", candidates.clone(), None, vec![reason.description()]).expect("built short, within bounds");
        Self { caller, callee_name, tier: EvidenceTier::Ambiguous, candidates, provenance, reason }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot, SymbolKind};

    fn test_id(name: &str) -> SymbolSnapshotId {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, "src/lib.rs").unwrap();
        SymbolSnapshotId::derive(&file, SymbolKind::Function, name, "fn x() {}", 0, 10)
    }

    #[test]
    fn unresolved_attempt_has_zero_candidates_and_the_unresolved_tier() {
        let attempt = CallAttempt::unresolved(test_id("caller"), "missing_callee".to_string());
        assert_eq!(attempt.tier, EvidenceTier::Unresolved);
        assert!(attempt.candidates.is_empty());
        assert_eq!(attempt.callee_name, "missing_callee");
    }

    #[test]
    fn ambiguous_attempt_preserves_every_real_candidate() {
        let candidates = vec![test_id("a"), test_id("b"), test_id("c")];
        let attempt = CallAttempt::ambiguous(test_id("caller"), "overloaded".to_string(), candidates.clone());
        assert_eq!(attempt.tier, EvidenceTier::Ambiguous);
        assert_eq!(attempt.candidates, candidates);
        assert_eq!(attempt.provenance.candidates_considered(), candidates.as_slice());
    }

    // --- Priority 2: reason taxonomy ---

    #[test]
    fn plain_unresolved_has_the_unknown_reason_never_a_fabricated_specific_one() {
        let attempt = CallAttempt::unresolved(test_id("caller"), "missing_callee".to_string());
        assert_eq!(attempt.reason, UnresolvedReason::Unknown, "without a separately-verified specific cause, Unknown is the honest, correct classification");
    }

    #[test]
    fn ambiguous_attempt_carries_the_ambiguous_candidates_reason() {
        let candidates = vec![test_id("a"), test_id("b")];
        let attempt = CallAttempt::ambiguous(test_id("caller"), "overloaded".to_string(), candidates);
        assert_eq!(attempt.reason, UnresolvedReason::AmbiguousCandidates);
    }

    #[test]
    fn extraction_rejected_reason_is_only_constructed_via_its_own_dedicated_constructor() {
        let attempt = CallAttempt::unresolved_with_extraction_rejection(test_id("caller"), "TooLongName".to_string(), "SignatureTooLong");
        assert_eq!(attempt.tier, EvidenceTier::Unresolved, "extraction-rejected is still an Unresolved TIER -- the reason taxonomy refines WHY, it does not introduce a fifth tier");
        assert_eq!(attempt.reason, UnresolvedReason::ExtractionRejected { rejection_kind: "SignatureTooLong" });
        assert!(
            attempt.provenance.method_limitations()[0].contains("SignatureTooLong"),
            "the provenance's own free text must name the real rejection kind, not a generic unresolved description"
        );
    }

    #[test]
    fn different_reasons_produce_distinguishable_provenance_text() {
        // The whole point of Priority 2: a consumer reading ONLY the
        // provenance text (as today's pre-Priority-2 model forced)
        // could not tell these apart; confirms the descriptions are
        // genuinely different strings, not the same generic sentence
        // reused for every reason.
        let unknown = UnresolvedReason::Unknown.description();
        let rejected = UnresolvedReason::ExtractionRejected { rejection_kind: "SignatureTooLong" }.description();
        let ambiguous = UnresolvedReason::AmbiguousCandidates.description();
        assert_ne!(unknown, rejected);
        assert_ne!(unknown, ambiguous);
        assert_ne!(rejected, ambiguous);
    }

    // --- CALL-RES-A4: import-alias reason taxonomy ---

    #[test]
    fn import_target_external_attempt_is_unresolved_with_the_correct_reason() {
        let attempt = CallAttempt::unresolved_import_target_external(test_id("caller"), "install_package".to_string());
        assert_eq!(attempt.tier, EvidenceTier::Unresolved, "a confirmed-external alias target is still an Unresolved TIER -- this reason refines WHY, it never promotes to a stronger tier than the evidence supports");
        assert_eq!(attempt.reason, UnresolvedReason::ImportTargetExternal);
        assert!(attempt.candidates.is_empty());
    }

    #[test]
    fn no_matching_import_alias_attempt_is_distinguishable_from_plain_unknown() {
        let plain_unknown = CallAttempt::unresolved(test_id("caller"), "mystery".to_string());
        let checked_no_alias = CallAttempt::unresolved_no_matching_import_alias(test_id("caller"), "mystery".to_string());
        assert_eq!(plain_unknown.reason, UnresolvedReason::Unknown);
        assert_eq!(checked_no_alias.reason, UnresolvedReason::NoMatchingImportAlias);
        assert_ne!(
            plain_unknown.reason, checked_no_alias.reason,
            "a caller that actually checked import evidence and found nothing must report a DIFFERENT, more specific reason than a caller that never checked at all"
        );
    }

    #[test]
    fn ambiguous_import_target_attempt_preserves_every_real_candidate() {
        let candidates = vec![test_id("a"), test_id("b")];
        let attempt = CallAttempt::ambiguous_import_target(test_id("caller"), "install_package".to_string(), candidates.clone());
        assert_eq!(attempt.tier, EvidenceTier::Ambiguous);
        assert_eq!(attempt.reason, UnresolvedReason::ImportedSymbolAmbiguous);
        assert_eq!(attempt.candidates, candidates);
    }

    #[test]
    fn all_four_unresolved_reasons_produce_pairwise_distinguishable_provenance_text() {
        let unknown = UnresolvedReason::Unknown.description();
        let rejected = UnresolvedReason::ExtractionRejected { rejection_kind: "SignatureTooLong" }.description();
        let external = UnresolvedReason::ImportTargetExternal.description();
        let no_alias = UnresolvedReason::NoMatchingImportAlias.description();
        let all = [&unknown, &rejected, &external, &no_alias];
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "reason texts at index {i} and {j} must be distinguishable");
                }
            }
        }
    }
}
