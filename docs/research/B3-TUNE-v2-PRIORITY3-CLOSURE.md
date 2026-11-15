# B3-TUNE-v2 Priority 3 Closure — CLOSED, no lever promoted

**Phase:** B3-TUNE-v2, Priority 3 (deterministic reranking).
**Date:** 2026-11-14.
**Verdict: CLOSED — no accepted deterministic reranking configuration.**

---

## 1. Scope recap

Priority 3 evaluated whether deterministic, auditable reranking
signals (ordering-only, never changing candidate membership) could
improve on official B1 without the cost of a learned reranker or
embeddings. Four candidates were designed
(`docs/superpowers/specs/2026-11-14-b3-tune-v2-priority3-reranking-design.md`,
following the ranking-pipeline audit in
`docs/research/B3-TUNE-v2-PRIORITY3-RANKING-AUDIT.md`): P1 (exact
structural-identifier boost), P2 (status tie-break), P3 (recency
tie-break), P4 (trust-level tie-break). The user authorized Stage 1
isolated ablation of P1 and P2 only; P3/P4 were deferred pending
evidence that P1/P2 were insufficient (decision `d211et`).

## 2. P1/P2 Stage 1 results — independent review ACCEPTED

Full detail: `eval/results/B3-TUNE-v2/priority3-stage1-summary.md`.

| Lever | Mechanism | Calibration activation | Verdict |
|---|---|---|---|
| P1 — exact structural-identifier boost | fixed additive bonus when a query token exactly whole-token-matches a note's `source:` `#<IDENTIFIER>` fragment | `identifier_boost_fired = 0/30` | **NOT EVALUATED / NO-OP** |
| P2 — status tie-break | orders bitwise-`f64`-tied candidates by status tier (`Verified`/`Reviewed` > `TestPassed` > `Draft` > `Superseded`/`Contradicted`) | `exact_ties_observed = 0` (calibration); `4` on dev, `ties_broken_by_status = 0/60` | **NOT EVALUATED / NO-OP** |

Both ablations ran through the real `Index::search` pipeline
(commits `101c780`, `87c99f0` for P1; `d95b7d8`, `f2424d6` for P2),
isolated (never composed as `B1+P1+P2`), against the frozen KEEP/REJECT
criterion carried over from Priority 2 Stage 1 (Recall@10/nDCG@10
regression ≤0.05 absolute vs. official B1, abstention confusion matrix
unchanged, at least one metric must improve to KEEP). Neither lever
ever activated on calibration, so neither could have moved a metric —
a zero-activation result is recorded NOT EVALUATED / NO-OP, never
REJECT, per the standing discipline established at Priority 2 Lever C.

**Independent review:** a second pass over the P1/P2 implementation
and ablation harness found 2 Important-severity test-quality issues
(two vacuous tests that did not actually exercise the no-op condition
they claimed to) and 0 Critical findings. Both were fixed
(RED → GREEN, commit `290dead`). Final review verdict: **APPROVE**.
This is the "independent review" referenced in continuity decision
`d212d2`.

**Activation counters (calibration, n=30):**
`identifier_boost_fired = 0`, `exact_ties_observed = 0`,
`ties_broken_by_status = 0`.

**Activation counters (dev, n=60, diagnostic only):**
`identifier_boost_fired = 0`, `exact_ties_observed = 4`,
`ties_broken_by_status = 0`.

## 3. P3/P4 — deferred, deduced NOT EVALUATED / NO-OP without implementation

**Decision: do not implement P3 (recency tie-break) or P4 (trust-level
tie-break) this round.**

Rationale: per their own design
(`2026-11-14-b3-tune-v2-priority3-reranking-design.md`, sections "P3"
and "P4"), both P3 and P4 are tie-break levers defined to apply in
**exactly the same scope as P2** — among candidates whose final BM25
scores are already bitwise-equal. Nothing in either design widens that
scope: P3 and P4 do not introduce a new notion of "near-tie" or
score-independent reordering, only a different secondary sort key
applied once the P2-style exact-tie condition is met.

P2's Stage 1 measurement already answered the only question P3/P4
could independently answer on this decision split: calibration
produced `exact_ties_observed = 0`. There is no exact BM25 tie for P3
or P4 to break on calibration, regardless of which secondary key
(recency vs. trust vs. status) is consulted once a tie exists. Since
the tie condition itself is absent, P3 and P4 are necessarily
**NOT EVALUATED / NO-OP on the decision split** — this is a logical
consequence of P2's own measurement, not a new empirical claim, and
does not require running new code to confirm:

- P3/P4 cannot fire where P2 could not fire, because they share the
  identical trigger condition (`exact_ties_observed`).
- Implementing P3/P4 and re-running calibration would reproduce
  `exact_ties_observed = 0` again, at the cost of two more additive
  hooks and two more ablation runs, to re-demonstrate a fact already
  established.
- Dev (diagnostic only) showed 4 ties existed but none crossed a
  status tier under P2; this says nothing about whether those same 4
  ties would cross a recency or trust boundary under P3/P4, but dev is
  not the decision split and was never going to decide this question
  either way.

This deduction is recorded here per user instruction, as the formal
closure reasoning for why P3/P4 are skipped without implementation.

## 4. Overall Priority 3 verdict

| Lever | Status |
|---|---|
| P1 (exact-identifier boost) | NOT EVALUATED / NO-OP (measured) |
| P2 (status tie-break) | NOT EVALUATED / NO-OP (measured) |
| P3 (recency tie-break) | NOT EVALUATED / NO-OP (deduced, not implemented) |
| P4 (trust-level tie-break) | NOT EVALUATED / NO-OP (deduced, not implemented) |

**No Priority 3 configuration is promoted.** No composition
(`B1 + P1 + P2`, or any combination involving P3/P4) was built or
measured. Official B1 (`eval/results/B3/frozen-config.json`, commit
`52d42ea`) remains the only active baseline, unmodified, through all
of Priority 1, Priority 2, and Priority 3.

## 5. Explicit confirmations

- The new v2 held-out final-test set (`eval/rag/v2-heldout/`) has not
  been read by this or any prior session in this phase.
- No composite configuration was built or measured.
- P3 and P4 were not implemented this round — their NOT EVALUATED /
  NO-OP status is deduced from P2's own measured activation count, not
  from a new ablation.
- No Priority 4 (hybrid retrieval) implementation, embeddings
  download, or benchmark has started as part of this closure.
- No modification to the B3 frozen config, runner, corpus, or labels.
- No MCP, no merge, no tag, no push.

## 6. Next step

Per user instruction: prepare design + initial audit only for
**Priority 4 — Hybrid Retrieval** (lexical + dense). See
`docs/research/B3-TUNE-v2-PRIORITY4-HYBRID-DESIGN.md`. No
implementation, no model download, no embeddings benchmark until
separately authorized.
