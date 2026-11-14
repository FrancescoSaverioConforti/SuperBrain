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

    // --- CALL-RES-A4 reliability-policy re-check ---
    //
    // Everything above this point only ever exercised `evaluate`/
    // `evaluate_high_impact_modification` against hand-built
    // `EvidenceTier` values -- a grep across this crate's own `src/`
    // confirms NEITHER function is called from any real A4 resolver
    // path (`calls.rs`, `negative_knowledge.rs`) or from any
    // production pipeline entry point. This module closes that gap:
    // real A4 output (from `resolve_calls_with_namespace_context`/
    // `resolve_calls_with_imports`, using the exact same real-
    // extraction fixture shapes the sibling `call_res_a4_namespace`/
    // `call_res_a4_import_alias` test modules in `edges::calls` already
    // proved produce A4's own real mechanisms) is now actually routed
    // through this policy layer and checked, rather than merely
    // reasoned about in prose.
    mod call_res_a4_policy_recheck {
        use codegraph_domain::{CommitHash, EvidenceTier, FileIdentity, RepositoryRegistry, RepositoryRoot, ResolutionMethod};

        use super::*;
        use crate::csharp_declarations::{extract_csharp_type_declaration_contexts, resolve_csharp_type_declaration_contexts};
        use crate::edges::{resolve_calls_with_imports, resolve_calls_with_namespace_context};
        use crate::extract::extract_symbols_reporting_rejections;
        use crate::imports::resolve_import_facts;
        use crate::negative_knowledge::UnresolvedReason;
        use crate::parse::parser_for;
        use crate::snapshot::{promote, SymbolSnapshot};

        fn test_repo() -> codegraph_domain::RepositoryId {
            let mut registry = RepositoryRegistry::new();
            registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
        }

        /// Same convention `negative_knowledge.rs`'s own test module
        /// uses for a plain, real-shaped `SymbolSnapshotId` where the
        /// test only needs policy to inspect `tier` (never the id's own
        /// identity) -- used by the two tests below that construct a
        /// `CallAttempt` directly rather than through a full real-file
        /// resolver run.
        fn policy_recheck_test_id(name: &str) -> codegraph_domain::SymbolSnapshotId {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, "src/lib.rs").unwrap();
            codegraph_domain::SymbolSnapshotId::derive(&file, codegraph_domain::SymbolKind::Function, name, "fn x() {}", 0, 10)
        }

        fn real_snapshots_contexts_and_tree(
            repo: codegraph_domain::RepositoryId,
            path: &str,
            source: &str,
        ) -> (FileIdentity, crate::parse::ParseTree, Vec<SymbolSnapshot>, Vec<(FileIdentity, crate::csharp_declarations::RawCSharpTypeDeclarationContext)>) {
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, path).unwrap();
            let parser = parser_for(&file).expect("a supported extension must have a parser");
            let tree = parser.parse(source.as_bytes()).expect("fixture source must parse");
            let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
            assert!(rejections.is_empty(), "fixture source must not trigger any extraction rejection: {rejections:?}");
            let snapshots: Vec<SymbolSnapshot> = candidates.into_iter().map(|c| promote(&c, file.clone()).expect("a real extracted candidate must always promote")).collect();
            let raw_contexts: Vec<_> = extract_csharp_type_declaration_contexts(&tree).into_iter().map(|c| (file.clone(), c)).collect();
            (file, tree, snapshots, raw_contexts)
        }

        fn find<'a>(snapshots: &'a [SymbolSnapshot], name: &str) -> &'a SymbolSnapshot {
            snapshots.iter().find(|s| s.name().as_str() == name).unwrap_or_else(|| panic!("no snapshot named {name:?} among {:?}", snapshots.iter().map(|s| s.name().as_str()).collect::<Vec<_>>()))
        }

        /// Verification point 1: a real namespace-narrowed-to-one call
        /// (the exact `namespace_import_excludes_the_non_imported_namespace_candidate`
        /// shape from `edges::calls`'s own A4 test module -- reused here
        /// rather than re-derived, since that module already proves
        /// this fixture produces a real narrowed edge) must map to
        /// `EvidenceTier::Heuristic`, never `Resolved` -- i.e. deterministic
        /// import/namespace narrowing increases confidence ONLY as far as
        /// the evidence actually supports (NameHeuristic), never silently
        /// promoted to the StaticAnalysis/Resolved tier that real syntactic
        /// certainty (e.g. exactly one candidate existed with no ambiguity
        /// at all) alone earns.
        #[test]
        fn namespace_narrowed_edge_is_heuristic_tier_never_resolved_and_policy_gates_it_accordingly() {
            let repo = test_repo();
            let (a_file, _a_tree, a_snapshots, a_raw) = real_snapshots_contexts_and_tree(repo, "src/NsA/Foo.cs", "namespace NsA;\nclass Foo { public void Target() {} }\n");
            let (b_file, _b_tree, b_snapshots, b_raw) = real_snapshots_contexts_and_tree(repo, "src/NsB/Foo.cs", "namespace NsB;\nclass Foo { public void Target() {} }\n");
            let (caller_file, caller_tree, caller_snapshots, _caller_raw) = real_snapshots_contexts_and_tree(repo, "src/Caller.cs", "using NsA;\nclass Caller { public void Run() { Target(); } }\n");

            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            all_snapshots.extend(caller_snapshots.clone());
            let mut all_raw = a_raw;
            all_raw.extend(b_raw);
            let csharp_contexts = resolve_csharp_type_declaration_contexts(&all_raw, &all_snapshots);
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges, _attempts) = resolve_calls_with_namespace_context(&all_snapshots, &[], &import_facts, &csharp_contexts).unwrap();
            let run_method = find(&caller_snapshots, "Run");
            let target_in_a = find(&a_snapshots, "Target");
            let edge = edges.iter().find(|e| e.from == run_method.id() && e.to == target_in_a.id()).expect("fixture premise: A4 narrows this real ambiguity to exactly one candidate");

            assert_eq!(edge.evidence.method(), ResolutionMethod::NameHeuristic, "A4's own namespace-narrowing mechanism must record NameHeuristic -- it is a judgment call among same-named candidates using a visibility fact, not syntactic certainty");
            let tier = EvidenceTier::from_resolution_method(edge.evidence.method());
            assert_eq!(tier, EvidenceTier::Heuristic, "a namespace-narrowed-to-one edge must map to Heuristic, never Resolved -- confirms reliability is increased only as far as the real evidence (a visibility fact, not a syntactic certainty) supports");

            // Route the real tier through the actual policy layer: must
            // be forbidden for high-impact use without disclosed
            // uncertainty/independent confirmation, permitted-with-
            // disclosure for mere display.
            assert_eq!(evaluate(tier, IntendedUse::InformationalDisplay), PolicyVerdict::PermittedWithDisclosedUncertainty);
            assert!(matches!(evaluate(tier, IntendedUse::AssertAsFactOrSoleJustification), PolicyVerdict::Forbidden { .. }));
            assert!(matches!(evaluate(tier, IntendedUse::HighImpactModification), PolicyVerdict::Forbidden { .. }));
            assert!(matches!(evaluate_high_impact_modification(&[tier]), PolicyVerdict::Forbidden { .. }), "a blast radius containing even one real A4-recovered NameHeuristic edge must block an autonomous high-impact modification");
            let _ = (a_file, b_file);
        }

        /// Verification point 3 + 7: the real cross-namespace ambiguity
        /// that A4 deliberately declines to resolve (reusing
        /// `using_an_unrelated_third_namespace_never_manufactures_a_false_resolution`'s
        /// own fixture shape) must carry `EvidenceTier::Ambiguous`, and
        /// the policy layer must forbid EVERY intended use for it,
        /// including mere informational display -- confirms "Ambiguous
        /// stays Forbidden" and "still-ambiguous cases remain
        /// non-actionable" against real A4-produced data, not a
        /// hand-built `EvidenceTier::Ambiguous` value.
        #[test]
        fn real_a4_ambiguous_attempt_is_forbidden_for_every_intended_use_including_display() {
            let repo = test_repo();
            let (a_file, _a_tree, a_snapshots, a_raw) = real_snapshots_contexts_and_tree(repo, "src/NsA/Foo.cs", "namespace NsA;\nclass Foo { public void Target() {} }\n");
            let (b_file, _b_tree, b_snapshots, b_raw) = real_snapshots_contexts_and_tree(repo, "src/NsB/Foo.cs", "namespace NsB;\nclass Foo { public void Target() {} }\n");
            let (caller_file, caller_tree, caller_snapshots, _caller_raw) = real_snapshots_contexts_and_tree(repo, "src/Caller.cs", "using NsC;\nclass Caller { public void Run() { Target(); } }\n");

            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            all_snapshots.extend(caller_snapshots.clone());
            let mut all_raw = a_raw;
            all_raw.extend(b_raw);
            let csharp_contexts = resolve_csharp_type_declaration_contexts(&all_raw, &all_snapshots);
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (_edges, attempts) = resolve_calls_with_namespace_context(&all_snapshots, &[], &import_facts, &csharp_contexts).unwrap();
            let run_method = find(&caller_snapshots, "Run");
            let attempt = attempts.iter().find(|a| a.caller == run_method.id() && a.callee_name == "Target").expect("fixture premise: a real, undecidable cross-namespace ambiguity");
            assert_eq!(attempt.tier, EvidenceTier::Ambiguous);
            assert_eq!(attempt.candidates.len(), 2, "both real candidates must be preserved, never silently dropped to one");

            assert!(matches!(evaluate(attempt.tier, IntendedUse::InformationalDisplay), PolicyVerdict::Forbidden { .. }), "a real Ambiguous CallAttempt must be forbidden even for mere display -- never surfaced as a single chosen candidate");
            assert!(matches!(evaluate(attempt.tier, IntendedUse::AssertAsFactOrSoleJustification), PolicyVerdict::Forbidden { .. }));
            assert!(matches!(evaluate(attempt.tier, IntendedUse::HighImpactModification), PolicyVerdict::Forbidden { .. }));
            let _ = (a_file, b_file);
        }

        /// Verification point 2: re-confirms, through the REAL resolver
        /// entry point (not the `exclude_definitely_not_visible_by_namespace`
        /// unit directly, which `edges::calls`'s own test module already
        /// covers), that an undetermined-namespace candidate is never
        /// collapsed into "not visible" -- and additionally that the
        /// resulting real `Ambiguous` outcome is, as always, Forbidden
        /// under policy for every intended use. This closes the policy-
        /// layer side of the same invariant `edges::calls::call_res_a4_namespace::a_candidate_with_an_undetermined_namespace_is_never_excluded`
        /// already closes at the resolver level.
        #[test]
        fn unknown_visibility_never_collapses_to_not_visible_and_the_resulting_ambiguity_is_still_forbidden() {
            let repo = test_repo();
            let (a_file, _a_tree, a_snapshots, a_raw) = real_snapshots_contexts_and_tree(repo, "src/NsA/Foo.cs", "namespace NsA;\nclass Foo { public void Target() {} }\n");
            let (b_file, _b_tree, b_snapshots, _b_raw) = real_snapshots_contexts_and_tree(repo, "src/NsB/Foo.cs", "namespace NsB;\nclass Foo { public void Target() {} }\n");
            let (caller_file, caller_tree, caller_snapshots, _caller_raw) = real_snapshots_contexts_and_tree(repo, "src/Caller.cs", "using NsA;\nclass Caller { public void Run() { Target(); } }\n");

            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            all_snapshots.extend(caller_snapshots.clone());
            // Deliberately only resolve contexts for NsA::Foo -- NsB::Foo
            // has NO CSharpTypeDeclarationContext entry, simulating a
            // genuinely undetermined namespace for that one candidate.
            let csharp_contexts = resolve_csharp_type_declaration_contexts(&a_raw, &all_snapshots);
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges, attempts) = resolve_calls_with_namespace_context(&all_snapshots, &[], &import_facts, &csharp_contexts).unwrap();
            let run_method = find(&caller_snapshots, "Run");

            assert!(edges.iter().all(|e| e.from != run_method.id()), "an undetermined-namespace candidate must never be silently excluded -- confirmed again here at the entry point this policy re-check exercises");
            let attempt = attempts.iter().find(|a| a.caller == run_method.id() && a.callee_name == "Target").expect("the real ambiguity (one determined-visible, one undetermined) must remain Ambiguous");
            assert_eq!(attempt.tier, EvidenceTier::Ambiguous);
            assert_eq!(attempt.candidates.len(), 2);
            assert!(matches!(evaluate(attempt.tier, IntendedUse::InformationalDisplay), PolicyVerdict::Forbidden { .. }), "the policy layer must still forbid this, since an undetermined-namespace candidate surviving filtering correctly produces a real, honest Ambiguous outcome, not a false Resolved one");
            let _ = (a_file, b_file);
        }

        /// Verification point 4: `ImportTargetExternal` (A4's own
        /// specific "checked the alias, confirmed external" reason) and
        /// plain `Unknown` (a generic local-resolution failure, never
        /// having consulted import evidence at all) are DIFFERENT
        /// `UnresolvedReason` values with different prose, but this test
        /// confirms, against the real policy layer rather than by
        /// reading the enum definitions, that they are NOT distinguished
        /// at the `EvidenceTier`/policy-outcome level: both are
        /// `EvidenceTier::Unresolved`, and `agent_policy::evaluate`
        /// forbids both identically for every intended use, with
        /// IDENTICAL reason text (the policy's own `Unresolved` arm is
        /// blind to `UnresolvedReason` entirely -- it only ever sees
        /// `tier`). Reported as a finding below, not forced into a
        /// false "yes, distinct" answer.
        #[test]
        fn import_target_external_and_plain_unknown_share_the_same_tier_and_identical_policy_outcome() {
            let caller = policy_recheck_test_id("caller");
            let external = crate::negative_knowledge::CallAttempt::unresolved_import_target_external(caller, "t".to_string());
            let unknown = crate::negative_knowledge::CallAttempt::unresolved(caller, "missing".to_string());

            assert_eq!(external.tier, EvidenceTier::Unresolved);
            assert_eq!(unknown.tier, EvidenceTier::Unresolved);
            assert_ne!(external.reason, unknown.reason, "the REASON values are genuinely distinct -- ImportTargetExternal vs. Unknown");

            let verdict_external = evaluate(external.tier, IntendedUse::InformationalDisplay);
            let verdict_unknown = evaluate(unknown.tier, IntendedUse::InformationalDisplay);
            assert_eq!(verdict_external, verdict_unknown, "FINDING: the policy layer's own evaluate() takes only EvidenceTier, never UnresolvedReason -- ImportTargetExternal (checked, confirmed external) and plain Unknown (never checked) are POLICY-IDENTICAL (same Forbidden verdict, same reason text), even though their own UnresolvedReason/provenance text differs. The distinction A4 adds is real at the reason/provenance layer but does NOT currently change what an agent is permitted to do -- both remain uniformly Forbidden, which is the SAFE default, but means 'unresolved-external' and 'local-resolution failure' are not routed to different policy outcomes today.");
        }

        /// Verification point 5: CRC-002 (local-binding shadowing) must
        /// not regress under THIS policy-layer re-evaluation either --
        /// re-runs the exact CRC-002 fixture shape already proven byte-
        /// identical at the resolver level in `edges::calls`'s own
        /// `crc_002_local_binding_shadowing_regression_check`, and
        /// additionally confirms the real resulting tiers/policy verdicts
        /// for this call site are themselves identical before/after A4.
        #[test]
        fn crc_002_shadowing_does_not_regress_under_policy_reevaluation() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let other_file = FileIdentity::parse(repo, commit.clone(), "components/other.py").unwrap();
            let other_parser = parser_for(&other_file).unwrap();
            let other_tree = other_parser.parse(b"def Button():\n    pass\n").unwrap();
            let (other_candidates, _) = extract_symbols_reporting_rejections(&other_tree);
            let unrelated_snapshots: Vec<SymbolSnapshot> = other_candidates.into_iter().map(|c| promote(&c, other_file.clone()).unwrap()).collect();

            let caller_file = FileIdentity::parse(repo, commit, "components/card.py").unwrap();
            let caller_parser = parser_for(&caller_file).unwrap();
            let caller_tree = caller_parser.parse(b"import os\n\n\ndef render(Button):\n    return Button()\n").unwrap();
            let (caller_candidates, _) = extract_symbols_reporting_rejections(&caller_tree);
            let caller_snapshots: Vec<SymbolSnapshot> = caller_candidates.into_iter().map(|c| promote(&c, caller_file.clone()).unwrap()).collect();

            let mut all_snapshots = unrelated_snapshots;
            all_snapshots.extend(caller_snapshots.clone());
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges_before, attempts_before) = crate::edges::resolve_calls_with_extraction_rejections(&all_snapshots, &[]).unwrap();
            let (edges_after, attempts_after) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            assert_eq!(edges_before, edges_after, "CRC-002 must stay byte-identical at the resolver level under this re-check too");

            let tiers_before: Vec<EvidenceTier> = edges_before.iter().map(|e| EvidenceTier::from_resolution_method(e.evidence.method())).chain(attempts_before.iter().map(|a| a.tier)).collect();
            let tiers_after: Vec<EvidenceTier> = edges_after.iter().map(|e| EvidenceTier::from_resolution_method(e.evidence.method())).chain(attempts_after.iter().map(|a| a.tier)).collect();
            assert_eq!(tiers_before, tiers_after, "the real tier sequence for CRC-002 must be identical before/after A4");

            let verdicts_before: Vec<PolicyVerdict> = tiers_before.iter().map(|t| evaluate(*t, IntendedUse::InformationalDisplay)).collect();
            let verdicts_after: Vec<PolicyVerdict> = tiers_after.iter().map(|t| evaluate(*t, IntendedUse::InformationalDisplay)).collect();
            assert_eq!(verdicts_before, verdicts_after, "every real policy verdict for CRC-002's own call sites must be identical before/after A4 -- no regression at the policy layer");
        }

        /// Verification point 6: spot-checks that A4's own
        /// `ImportedSymbolAmbiguous` outcome -- the real "found the
        /// alias, but its target is still 2+ candidates" case that the
        /// Polly/pyjanitor metrics pass (commit `ab0591d`) separately
        /// counted at corpus scale -- is itself `EvidenceTier::Ambiguous`
        /// and therefore Forbidden under policy for every intended use,
        /// exactly like a plain name-based ambiguity. Confirms A4's own
        /// NEW ambiguity-reason variant does not accidentally bypass the
        /// policy's "Ambiguous is always Forbidden" rule by virtue of
        /// being a different code path.
        #[test]
        fn import_alias_ambiguous_target_is_policy_coherent_with_plain_ambiguous() {
            let caller = policy_recheck_test_id("caller");
            let candidates = vec![policy_recheck_test_id("candidate")]; // any real-shaped id; policy only inspects tier
            let attempt = crate::negative_knowledge::CallAttempt::ambiguous_import_target(caller, "t".to_string(), candidates);
            assert_eq!(attempt.tier, EvidenceTier::Ambiguous);
            assert_eq!(attempt.reason, UnresolvedReason::ImportedSymbolAmbiguous);
            assert!(matches!(evaluate(attempt.tier, IntendedUse::InformationalDisplay), PolicyVerdict::Forbidden { .. }), "ImportedSymbolAmbiguous must be exactly as Forbidden as a plain name-based Ambiguous -- the policy layer is reason-blind by design, so A4's own new ambiguity source cannot accidentally slip past the Ambiguous-is-always-Forbidden rule");
        }
    }
}
