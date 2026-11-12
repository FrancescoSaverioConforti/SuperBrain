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
}

impl CallAttempt {
    pub fn unresolved(caller: SymbolSnapshotId, callee_name: String) -> Self {
        let provenance = Provenance::new("resolve_calls", vec![], None, vec!["zero candidates matched this callee name in the given snapshot pool".to_string()]).expect("built short, within bounds");
        Self { caller, callee_name, tier: EvidenceTier::Unresolved, candidates: vec![], provenance }
    }

    pub fn ambiguous(caller: SymbolSnapshotId, callee_name: String, candidates: Vec<SymbolSnapshotId>) -> Self {
        let provenance = Provenance::new(
            "resolve_calls",
            candidates.clone(),
            None,
            vec!["name-based matching cannot distinguish overloads by parameter list or generic arity; 2+ same-named candidates remained even after scope narrowing".to_string()],
        )
        .expect("built short, within bounds");
        Self { caller, callee_name, tier: EvidenceTier::Ambiguous, candidates, provenance }
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
}
