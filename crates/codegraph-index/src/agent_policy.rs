//! Quality & Semantic Reliability Hardening, Phase 2 — the Agent
//! Reliability Contract (`docs/research/AGENT_RELIABILITY_CONTRACT.md`)
//! made PARTIALLY executable: real, unit-tested policy functions a
//! future coding agent integration ("Task 10," explicitly NOT
//! authorized and NOT touched by this module) could call to decide
//! what it is and is not permitted to do with a given
//! [`codegraph_domain::EvidenceTier`] outcome. This module does not
//! integrate with any agent — it is the POLICY layer alone, callable
//! and testable independent of any agent ever existing.
//!
//! # Core principle (verbatim from the design document)
//!
//! > The Code Graph's own absence of a relationship is NOT evidence
//! > that no relationship exists.
//!
//! Every function below is a direct, mechanical translation of one
//! rule from that document's own "Evidence-tier policy" section — this
//! module adds no new policy, it makes the existing, reviewed policy
//! checkable in code.

use codegraph_domain::EvidenceTier;

/// The kinds of action a caller (today: a test; eventually, Task 10's
/// own agent) might want to take with a resolved-or-not relationship.
/// Deliberately a small, closed set matching exactly the contract
/// document's own named scenarios — not a general-purpose permission
/// system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntendedUse {
    /// Display the relationship to a human as informational/possible,
    /// or use it as one weak input among several for a low-stakes
    /// suggestion (e.g. ranking search results).
    InformationalDisplay,
    /// Present the relationship to a human AS A CERTAIN FACT (e.g.
    /// "function A calls function B" stated flatly, no hedging), or
    /// use it as the sole justification for an automated action.
    AssertAsFactOrSoleJustification,
    /// A high-impact operation whose blast radius depends on this
    /// relationship being complete and correct: delete, rename a
    /// public API, change a function signature, remove/replace an
    /// implemented interface, or any other change the contract
    /// document names under "High-impact modification gate."
    HighImpactModification,
}

/// What the policy says about one `(EvidenceTier, IntendedUse)` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyVerdict {
    /// Proceed; no special caveat required beyond ordinary use.
    Permitted,
    /// Proceed, but the uncertainty MUST be declared explicitly to the
    /// human (or independent confirmation sought first) — never
    /// presented as if it were `Resolved`.
    PermittedWithDisclosedUncertainty,
    /// Must NOT proceed on this evidence alone. The contract requires
    /// surfacing the situation to a human (for `Ambiguous`, every real
    /// candidate; for `Unresolved`, an explicit statement that the
    /// Code Graph lacks sufficient evidence) rather than silently
    /// picking one option or silently assuming absence.
    Forbidden { reason: &'static str },
}

/// The one rule hardest to violate accidentally, per the contract
/// document's own framing: an `Ambiguous` outcome must never be
/// silently collapsed into a single chosen candidate. This function
/// exists specifically so a caller cannot "accidentally" treat
/// `Ambiguous` as if it were `Resolved` by, say, arbitrarily taking
/// `candidates[0]` — the TYPE this function returns forces the caller
/// to handle the ambiguity explicitly rather than reach past it.
pub fn evaluate(tier: EvidenceTier, intended_use: IntendedUse) -> PolicyVerdict {
    match (tier, intended_use) {
        // Resolved: usable normally for anything, including high-impact
        // work -- this is the contract's own strongest tier.
        (EvidenceTier::Resolved, _) => PolicyVerdict::Permitted,

        // Heuristic: fine for informational display; forbidden to
        // assert as fact or use as sole justification without first
        // disclosing the uncertainty; forbidden outright as a basis
        // for a high-impact modification (the contract requires
        // upgrading via independent confirmation first, which this
        // policy layer cannot itself perform -- it can only refuse to
        // bless the unconfirmed case).
        (EvidenceTier::Heuristic, IntendedUse::InformationalDisplay) => PolicyVerdict::PermittedWithDisclosedUncertainty,
        (EvidenceTier::Heuristic, IntendedUse::AssertAsFactOrSoleJustification) => {
            PolicyVerdict::Forbidden { reason: "Heuristic evidence must never be presented as certain fact or used as the sole justification for an action without disclosing the uncertainty or seeking independent confirmation first" }
        }
        (EvidenceTier::Heuristic, IntendedUse::HighImpactModification) => {
            PolicyVerdict::Forbidden { reason: "a high-impact modification requires evidence stronger than Heuristic for every edge it depends on; upgrade via independent confirmation (code search, compiler/language-server query, running affected tests) before proceeding" }
        }

        // Ambiguous: NEVER permitted to pick one candidate and
        // proceed, for ANY intended use -- including informational
        // display, which must show ALL real candidates, never one
        // chosen arbitrarily.
        (EvidenceTier::Ambiguous, _) => {
            PolicyVerdict::Forbidden { reason: "an Ambiguous outcome has 2+ real candidates and the resolver deliberately declined to choose -- never pick one arbitrarily; surface every real candidate to the human, or seek independent confirmation (direct source reading, compiler/language-server query) before treating any one of them as the target" }
        }

        // Unresolved: the agent must declare insufficient evidence
        // explicitly, for ANY intended use -- "no edge" must never be
        // silently treated as "no relationship."
        (EvidenceTier::Unresolved, _) => {
            PolicyVerdict::Forbidden { reason: "an Unresolved outcome means zero real candidates were found in the given pool, not that no relationship exists -- the target may be external, or missed by a known extraction/resolution gap; declare explicitly that the Code Graph lacks sufficient evidence rather than proceeding as though the relationship does not exist" }
        }
    }
}

/// The "High-impact modification gate," evaluated over a WHOLE set of
/// edges/attempts the modification's own blast-radius analysis
/// depends on — not just one. Per the contract document's own rule 4:
/// "never report 'this change is safe, N callers checked' as a
/// definitive claim when some of those N were Heuristic-tier or when
/// the search space itself might be incomplete." Returns `Permitted`
/// only when EVERY tier in the set is `Resolved`; any `Heuristic`,
/// `Ambiguous`, or `Unresolved` tier anywhere in the set blocks the
/// whole modification, per the contract's own "treat the change as
/// NOT SAFE to make autonomously" rule.
pub fn evaluate_high_impact_modification(tiers_in_blast_radius: &[EvidenceTier]) -> PolicyVerdict {
    if tiers_in_blast_radius.is_empty() {
        // An empty blast radius is not evidence of safety either --
        // per the core principle itself, applied to the meta-question
        // "did this analysis even find anything to check." A caller
        // with a genuinely empty, deliberately-scoped blast radius
        // should not be routing through this function at all; reaching
        // here with nothing to evaluate is itself treated as
        // insufficient evidence, never silently "Permitted."
        return PolicyVerdict::Forbidden { reason: "no edges were found in the blast radius at all -- this is not evidence the modification is safe; it may mean the search space itself is incomplete (a known extraction/resolution gap) rather than that nothing depends on this symbol" };
    }
    if tiers_in_blast_radius.iter().all(|t| *t == EvidenceTier::Resolved) {
        return PolicyVerdict::Permitted;
    }
    if tiers_in_blast_radius.iter().any(|t| *t == EvidenceTier::Ambiguous) {
        return PolicyVerdict::Forbidden { reason: "at least one Ambiguous relationship exists within this modification's own blast radius -- treat the change as NOT SAFE to make autonomously until every ambiguous case is resolved by independent confirmation" };
    }
    if tiers_in_blast_radius.iter().any(|t| *t == EvidenceTier::Unresolved) {
        return PolicyVerdict::Forbidden { reason: "at least one Unresolved relationship exists within this modification's own blast radius -- the search space may be incomplete; treat the change as NOT SAFE to make autonomously" };
    }
    // Everything remaining is Resolved or Heuristic, with at least one
    // Heuristic (the all-Resolved case already returned above).
    PolicyVerdict::Forbidden { reason: "at least one Heuristic-tier relationship exists within this modification's own blast radius -- upgrade it via independent confirmation before proceeding; a high-impact modification requires evidence stronger than Heuristic for every edge it depends on" }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- evaluate: Resolved ---

    #[test]
    fn resolved_is_permitted_for_every_intended_use() {
        assert_eq!(evaluate(EvidenceTier::Resolved, IntendedUse::InformationalDisplay), PolicyVerdict::Permitted);
        assert_eq!(evaluate(EvidenceTier::Resolved, IntendedUse::AssertAsFactOrSoleJustification), PolicyVerdict::Permitted);
        assert_eq!(evaluate(EvidenceTier::Resolved, IntendedUse::HighImpactModification), PolicyVerdict::Permitted);
    }

    // --- evaluate: Heuristic ---

    #[test]
    fn heuristic_is_permitted_with_disclosure_for_informational_display() {
        assert_eq!(evaluate(EvidenceTier::Heuristic, IntendedUse::InformationalDisplay), PolicyVerdict::PermittedWithDisclosedUncertainty);
    }

    #[test]
    fn heuristic_is_forbidden_as_sole_justification_or_asserted_fact() {
        assert!(matches!(evaluate(EvidenceTier::Heuristic, IntendedUse::AssertAsFactOrSoleJustification), PolicyVerdict::Forbidden { .. }));
    }

    #[test]
    fn heuristic_is_forbidden_for_high_impact_modification() {
        assert!(matches!(evaluate(EvidenceTier::Heuristic, IntendedUse::HighImpactModification), PolicyVerdict::Forbidden { .. }));
    }

    // --- evaluate: Ambiguous -- the single hardest rule to violate accidentally ---

    #[test]
    fn ambiguous_is_forbidden_for_every_intended_use_including_mere_display() {
        // The contract's own framing: an LLM-based agent's default
        // behavior (pick the most plausible option and continue) is
        // exactly what this must block -- even for the "weakest"
        // intended use, informational display, since showing ONE
        // arbitrarily-chosen candidate as "the" answer is already the
        // violation, regardless of how low-stakes the display context is.
        assert!(matches!(evaluate(EvidenceTier::Ambiguous, IntendedUse::InformationalDisplay), PolicyVerdict::Forbidden { .. }));
        assert!(matches!(evaluate(EvidenceTier::Ambiguous, IntendedUse::AssertAsFactOrSoleJustification), PolicyVerdict::Forbidden { .. }));
        assert!(matches!(evaluate(EvidenceTier::Ambiguous, IntendedUse::HighImpactModification), PolicyVerdict::Forbidden { .. }));
    }

    // --- evaluate: Unresolved ---

    #[test]
    fn unresolved_is_forbidden_for_every_intended_use() {
        assert!(matches!(evaluate(EvidenceTier::Unresolved, IntendedUse::InformationalDisplay), PolicyVerdict::Forbidden { .. }));
        assert!(matches!(evaluate(EvidenceTier::Unresolved, IntendedUse::AssertAsFactOrSoleJustification), PolicyVerdict::Forbidden { .. }));
        assert!(matches!(evaluate(EvidenceTier::Unresolved, IntendedUse::HighImpactModification), PolicyVerdict::Forbidden { .. }));
    }

    #[test]
    fn unresolved_forbidden_reason_mentions_the_absence_is_not_proof_principle() {
        // Pins the CONTENT of the reason, not just that it exists --
        // confirms this specific invariant is actually reflected in
        // the policy's own text, not merely in this module's doc
        // comment.
        if let PolicyVerdict::Forbidden { reason } = evaluate(EvidenceTier::Unresolved, IntendedUse::InformationalDisplay) {
            assert!(reason.contains("not that no relationship exists") || reason.to_lowercase().contains("not evidence"), "reason must explicitly reject absence-as-proof-of-absence, got: {reason:?}");
        } else {
            panic!("Unresolved must always be Forbidden");
        }
    }

    // --- evaluate_high_impact_modification ---

    #[test]
    fn all_resolved_blast_radius_is_permitted() {
        let tiers = vec![EvidenceTier::Resolved, EvidenceTier::Resolved, EvidenceTier::Resolved];
        assert_eq!(evaluate_high_impact_modification(&tiers), PolicyVerdict::Permitted);
    }

    #[test]
    fn a_single_heuristic_edge_among_resolved_ones_blocks_the_whole_modification() {
        let tiers = vec![EvidenceTier::Resolved, EvidenceTier::Resolved, EvidenceTier::Heuristic];
        assert!(matches!(evaluate_high_impact_modification(&tiers), PolicyVerdict::Forbidden { .. }));
    }

    #[test]
    fn a_single_ambiguous_edge_blocks_the_whole_modification() {
        let tiers = vec![EvidenceTier::Resolved, EvidenceTier::Ambiguous];
        assert!(matches!(evaluate_high_impact_modification(&tiers), PolicyVerdict::Forbidden { .. }));
    }

    #[test]
    fn a_single_unresolved_edge_blocks_the_whole_modification() {
        let tiers = vec![EvidenceTier::Resolved, EvidenceTier::Unresolved];
        assert!(matches!(evaluate_high_impact_modification(&tiers), PolicyVerdict::Forbidden { .. }));
    }

    #[test]
    fn an_empty_blast_radius_is_never_silently_treated_as_safe() {
        // The meta-level reflection of the core principle: "I found
        // nothing to check" is not the same as "I checked and it's
        // safe." A future change that made this return Permitted on
        // an empty slice would reintroduce exactly the absence-as-
        // proof-of-absence failure this whole module exists to block.
        assert!(matches!(evaluate_high_impact_modification(&[]), PolicyVerdict::Forbidden { .. }));
    }

    #[test]
    fn ambiguous_takes_priority_over_unresolved_when_both_are_present() {
        // Both are real problems; the specific ORDER reported is a
        // deliberate choice (Ambiguous surfaced first, since it is the
        // "hardest to violate accidentally" rule per the contract's
        // own framing) -- pinned here so a future refactor does not
        // silently flip which reason a caller sees first, which could
        // otherwise go unnoticed since both paths correctly return
        // Forbidden either way.
        let tiers = vec![EvidenceTier::Ambiguous, EvidenceTier::Unresolved];
        if let PolicyVerdict::Forbidden { reason } = evaluate_high_impact_modification(&tiers) {
            assert!(reason.contains("Ambiguous"), "expected the Ambiguous-specific reason to be surfaced first, got: {reason:?}");
        } else {
            panic!("expected Forbidden");
        }
    }
}
