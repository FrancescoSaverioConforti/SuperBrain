# Agent reliability contract — design

Quality & Semantic Reliability Hardening, Phase 1, deliverable 7.
**Status: design/policy only, not yet enforced by any code — "Task 10"
(the future coding agent itself) is explicitly NOT authorized and not
touched by this document.** This contract is written now so that when
Task 10 eventually begins, the rules governing how it may use the
Code Graph are already settled, evidence-grounded, and reviewed —
not improvised under schedule pressure once the agent exists.

This contract is defined in terms of the `EvidenceTier` model proposed
in `SEMANTIC_UNCERTAINTY_MODEL.md` — read that document first. Where
this contract references "today's actual data" (because the tiered
model is not yet implemented), it maps onto what a coding agent CAN
observe from the current codebase right now: `Edge.evidence.method()`
(`StaticAnalysis`/`NameHeuristic`/`Inferred`), the ABSENCE of an edge
(which today conflates Ambiguous and Unresolved — see that document's
own central finding), and the explicit `ImportResolution`/
`ImplTargetResolution` three-way status on structural facts.

## The core principle

> The Code Graph's own absence of a relationship is NOT evidence that
> no relationship exists. An agent must never treat "I found no edge"
> as "I have proven there is no connection" — only as "this pipeline
> did not establish one," which has multiple possible real causes
> (genuinely no relationship; a known extraction/resolution gap per
> the gap inventory; a genuinely ambiguous case this pipeline
> correctly declined to guess at).

## Evidence-tier policy

### Resolved (today: `ResolutionMethod::StaticAnalysis`)

The agent MAY use this relationship as a normal, load-bearing fact —
e.g. "function A calls function B" backed by a `Resolved` `Calls`
edge may inform a refactor, an impact-analysis answer, or a
navigation suggestion without further hedging in ordinary cases.

**Caveat the agent must still carry**: `Resolved` in this codebase
means "exactly one name-matched candidate, not necessarily a
compiler-verified relationship." It does not mean a type-checker or
language server has confirmed it. For most of this pipeline's own
real-world measured false-positive rate (see the gap inventory), this
distinction rarely matters in practice — but the agent must not
represent a `Resolved` edge to a human user as "verified by a
compiler" when it was not.

### Heuristic (today: `ResolutionMethod::NameHeuristic`)

The agent MUST declare the uncertainty explicitly when surfacing this
relationship to a human, or actively seek confirmation before acting
on it for anything beyond informational display. Concretely:

- **Permitted without further confirmation**: showing the relationship
  to a human as "possible" phrased ("this looks like a call to X,
  though resolved by name-narrowing rather than exact analysis"),
  using it as ONE input among several for a low-stakes suggestion
  (e.g. ranking search results).
- **NOT permitted without further confirmation**: using a `Heuristic`
  edge as the sole justification for an automated code change, a
  claim of certainty made to a human, or a chain of reasoning where
  the uncertainty compounds silently (e.g. "A heuristically calls B,
  and B heuristically calls C, therefore A definitely affects C" — two
  compounded heuristics are weaker evidence than either alone, and the
  agent must not present the conclusion with the confidence of either
  premise).

### Ambiguous (today: absent edge where 2+ real candidates existed —
### see the uncertainty model's own central finding that this is
### currently indistinguishable from Unresolved; this policy still
### applies the moment that distinction becomes observable, and an
### agent reasoning from the RAW resolver output rather than only the
### final graph can already observe it today in some cases, e.g. by
### directly calling `resolve_calls` or reading `ImportResolution::Ambiguous`)

The agent MUST NEVER arbitrarily pick one candidate and proceed as if
it were certain. This is the single hardest rule in this contract to
violate accidentally, because an LLM-based agent's own default
behavior (pick the most plausible-sounding option and continue) is
exactly the failure mode this rule exists to block.

Required behavior when the agent's own reasoning surfaces a genuinely
ambiguous relationship:
1. Surface ALL real candidates to the human, not just the one the
   agent's own judgment favors.
2. If the task requires picking one to proceed (e.g. "rename this
   function"), the agent must ask the human to disambiguate, or seek
   independent confirmation (direct source reading, a compiler/
   language-server query, a test run) BEFORE treating one candidate as
   the real target.
3. The agent must never silently narrow an `Ambiguous` case using its
   own heuristic judgment and THEN report the result as if the Code
   Graph itself had resolved it — that would misattribute the agent's
   own guess as the pipeline's own evidence.

### Unresolved (today: absent edge where zero real candidates existed,
### or `ImportResolution::External`/`ImplTargetResolution::Unresolved`)

The agent MUST say explicitly that the Code Graph lacks sufficient
evidence — never silently proceed as though the relationship simply
does not exist. The real target may be external (stdlib/third-party/
not yet indexed in this repository's own scope) or may be missed due
to a KNOWN gap (see the gap inventory — e.g. CRC-003's import-alias
blindness, the Rust macro-call-opacity gap, the C++ namespace-
qualified-call gap). An agent that has access to the gap inventory
should check whether the specific unresolved case matches a known gap
shape and say so explicitly ("the Code Graph does not track calls
through import aliases; this may be a real, missed relationship, not
evidence that none exists") rather than reporting bare absence.

### High-impact modification gate

Before any of: deleting a symbol, renaming a public API, changing a
function's signature, removing/replacing an implemented interface, or
any other change whose blast radius depends on the Code Graph's own
`Calls`/`Extends`/`Implements`/`Instantiates` edges being complete and
correct — the agent must require evidence STRONGER than `Heuristic`
for every edge the decision depends on, specifically:

1. Prefer `Resolved` edges as the basis for the impact analysis.
2. For every `Heuristic` edge in the impact set, either upgrade it via
   independent confirmation (a real code-search grep, a compiler/
   language-server query if available, running the affected tests) or
   flag it explicitly to the human as an unverified dependency before
   proceeding.
3. For any `Ambiguous`/`Unresolved` case touching the change's own
   blast radius, treat the change as NOT SAFE to make autonomously —
   surface it and stop, per the Ambiguous/Unresolved rules above.
4. Never report "this change is safe, N callers checked" as a
   definitive claim when some of those N were `Heuristic`-tier or when
   the search space itself might be incomplete due to a known
   extraction gap (e.g. a macro-wrapped call, a namespace-qualified
   call) — the honest claim is "N callers found by [method], with
   [these specific caveats]," never a bare, unqualified "N callers."

## What this contract does NOT define

- **Enforcement mechanism.** This document states the POLICY a future
  coding agent (Task 10) must follow; it does not implement a runtime
  check, a linter, or a prompt-engineering mechanism that would force
  compliance. That is explicitly future work for whenever Task 10 is
  authorized.
- **UI/UX for surfacing uncertainty to a human.** How exactly an agent
  phrases "this is ambiguous" in a chat response, a code comment, or a
  PR description is a product decision for Task 10's own design, not
  this contract's concern.
- **What happens when the agent is WRONG despite following this
  contract.** Following this contract reduces the RATE of
  overconfident false claims; it does not make the agent infallible
  (e.g. a `Resolved` edge can still be a genuine false positive per
  this pipeline's own measured FP rate in domains like CRC-002's
  prop-shadowing case). This contract is a discipline for HANDLING
  uncertainty honestly, not a guarantee of correctness.

## Relationship to the known-gap inventory

This contract is only as useful as the agent's own access to
`KNOWN_SEMANTIC_GAPS.md` (deliverable 8) — an agent that does not know
CRC-003's alias-blindness gap exists cannot apply the "check if this
matches a known gap" step under "Unresolved" above. Task 10's own
future design should treat the gap inventory as a required input the
agent can query, not optional background reading.
