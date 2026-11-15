# B3-TUNE-v2 Priority 3 Stage 1 — P1/P2 Isolated Results

**Phase:** B3-TUNE-v2, Priority 3 (deterministic reranking), Stage 1
(two isolated ablations only: P1 exact-identifier boost, P2 status
tie-break).
**Date:** 2026-11-14.
**Scope:** `B1 + P1` vs. official B1, and `B1 + P2` vs. official B1,
each measured completely independently, through the real
`Index::search` pipeline. **No composition** (`B1 + P1 + P2`) was run
or measured. P3 (recency tie-break) and P4 (trust tie-break) were
deferred/no-go this round and are not evaluated here.

All semantics below were frozen BEFORE any ablation ran — see
`docs/superpowers/specs/2026-11-14-b3-tune-v2-priority3-stage1-design.md`.
The KEEP/REJECT criterion is the SAME frozen rule used throughout
B3-TUNE-v2 Priority 2 Stage 1: Recall@10/nDCG@10 regression ≤0.05
absolute vs. official B1 (`eval/results/B3/frozen-config.json`, commit
`52d42ea`), at least one metric must improve, abstention confusion
matrix pattern unchanged (TP=0, FP=0). Dev is diagnostic only;
calibration is the decision split. A zero-activation result is
reported NOT EVALUATED / NO-OP, never REJECT.

---

## Lever P1 — Exact structural-identifier boost — **NOT EVALUATED / NO-OP**

**Mechanism:** `Index::with_identifier_boost()` (additive, opt-in,
commit `101c780`) adds a fixed bonus (`IDENTIFIER_BOOST = 100.0`,
pinned above any realistic BM25 score on this 53-note corpus) to a
candidate's final score when its `source:`-extracted `#<IDENTIFIER>`
fragment exactly whole-token-matches a query token (case-insensitive,
never substring/prefix). Cannot change candidate membership.

**Config confirmation:** match rule is whole-token exact equality only
(verified by `identifier_boost_never_fires_on_a_substring_or_prefix_match`
and the local ablation-binary pin test, both passing) — never fuzzy,
never `SEC` matching `SEC-001`, per the user's explicit constraint.

**Commit:** `101c780` (hook), `87c99f0` (ablation).

| | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 | identifier_boost_fired |
|---|---|---|---|---|---|
| Dev | 0.8367 | — (diagnostic only) | 0.7139 | — | 0/60 |
| Calibration | 0.9167 | **+0.0000** | 0.7091 | **+0.0000** | **0/30** |

**By-category (calibration):** bugs-fixes n=6 Recall=1.0/nDCG=0.8552;
identifier-code n=5 Recall=0.8/nDCG=0.4495; notes-decisions n=4
Recall=1.0/nDCG=0.7654; patterns-knowledge n=4 Recall=1.0/nDCG=0.7366;
cross-file-concept n=3 Recall=0.6667/nDCG=0.6667; temporal n=2
Recall=1.0/nDCG=0.8155 — identical to official B1's own by-category
numbers, since the boost never fired.

**By-difficulty (calibration):** medium n=11 Recall=0.9091/nDCG=0.8301;
hard n=5 Recall=1.0/nDCG=0.5757; easy n=8 Recall=0.875/nDCG=0.6260 —
identical to B1.

**Abstention (calibration):** TP=0, TN=24, FP=0, FN=6 — unchanged from
B1's own pattern.

**Activation:** `identifier_boost_fired = 0` on BOTH dev (0/60) and
calibration (0/30). Not one query in either split contained a literal
whole-token match against any indexed note's `source:`-extracted
identifier. This mirrors Priority 2 Lever C's own finding (the alias
table's short forms never appeared as literal tokens in either split
either) — the risk flagged explicitly in the Priority 3 design doc's
own "Risk of regression" section for P1 has materialized exactly as
anticipated.

**KEEP/REJECT rationale:** Both regression tolerances pass trivially
(exact zero delta, both metrics, both splits — the harness ran every
query with the boost logic present and it never activated). Abstention
unchanged. Neither metric improves — but not because the rule was
tried and found unhelpful; because it was never triggered.

**Verdict: NOT EVALUATED / NO-OP (zero activations on dev and
calibration; closed as inconclusive).** Per the user's own standing
instruction from Priority 2 Lever C's correction, this is NOT
relabeled REJECT — REJECT would overstate what was actually tested. No
new identifiers or source fields are to be constructed to force
activations on dev/calibration, for the same calibration-leakage
reason Lever C's own correction established.

---

## Lever P2 — Status tie-break — **NOT EVALUATED / NO-OP**

**Mechanism:** `Index::with_status_tiebreak()` (additive, opt-in,
commit `d95b7d8`) orders two candidates by `status_tier` only when
their final BM25 scores are bitwise-equal `f64` values — never within
an epsilon. Frozen tiers (confirmed against the real 6-variant
`domain::NoteStatus`, which has no derived `Ord`): `Verified`/`Reviewed`
(tier 1) > `TestPassed` (tier 2) > `Draft` (tier 3) >
`Superseded`/`Contradicted` (tier 4). Cannot change candidate
membership.

**Config confirmation:** tie rule is bitwise `f64` equality only
(verified by `status_tiebreak_never_fires_on_a_nonzero_score_difference`,
which constructs a genuinely non-tied score pair and confirms ordering
is identical with and without the flag) — never an epsilon, per the
user's explicit constraint.

**Commit:** `d95b7d8` (hook), `f2424d6` (ablation).

| | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 | exact_ties_observed | ties_broken_by_status |
|---|---|---|---|---|---|---|
| Dev | 0.8367 | — (diagnostic only) | 0.7139 | — | 4 | 0/60 |
| Calibration | 0.9167 | **+0.0000** | 0.7091 | **+0.0000** | **0** | **0/30** |

**By-category (calibration):** identical to P1's own table above (same
corpus, same queries, same zero-activation aggregate) — bugs-fixes
n=6 Recall=1.0/nDCG=0.8552; identifier-code n=5 Recall=0.8/nDCG=0.4495;
notes-decisions n=4 Recall=1.0/nDCG=0.7654; patterns-knowledge n=4
Recall=1.0/nDCG=0.7366; cross-file-concept n=3
Recall=0.6667/nDCG=0.6667; temporal n=2 Recall=1.0/nDCG=0.8155.

**By-difficulty (calibration):** medium n=11 Recall=0.9091/nDCG=0.8301;
hard n=5 Recall=1.0/nDCG=0.5757; easy n=8 Recall=0.875/nDCG=0.6260.

**Abstention (calibration):** TP=0, TN=24, FP=0, FN=6 — unchanged.

**Activation:** On calibration, `exact_ties_observed = 0` — the BM25
scoring that decides calibration's rankings produced zero exact score
ties among any query's returned candidates, so P2's rule never had an
opportunity to fire at all. On dev, `exact_ties_observed = 4` — ties
DID occur — but `ties_broken_by_status = 0/60`: in every one of those
4 tied cases, the tied candidates shared the SAME status tier (or the
tie-break produced the identical ordering the existing note-id
tie-break already gave), so the rule never actually changed an
ordering even where it had an opportunity to. Dev is diagnostic only
and does not decide this lever's verdict, but its own number confirms
the rule is real and reachable in principle — it simply never crossed
a tier boundary on this corpus's actual tie structure.

**KEEP/REJECT rationale:** Both regression tolerances pass trivially
(exact zero delta on calibration). Abstention unchanged. Neither
metric improves — because the rule was never triggered on calibration,
and on dev it triggered zero actual reorderings despite ties existing.

**Verdict: NOT EVALUATED / NO-OP (zero ties on calibration; zero
ties actually broken on dev despite 4 ties present; closed as
inconclusive).** This is a genuinely different flavor of no-op from
P1's (P1 never matched at all; P2 found ties but none crossed a tier
boundary) — both are recorded as NOT EVALUATED / NO-OP per the same
standing discipline, since neither represents a tested-and-failed
hypothesis.

---

## Overall Priority 3 Stage 1 result

| Lever | Verdict |
|---|---|
| P1 (exact-identifier boost) | **NOT EVALUATED / NO-OP** (0/60 dev, 0/30 calibration activations) |
| P2 (status tie-break) | **NOT EVALUATED / NO-OP** (0 ties on calibration; 4 ties on dev, 0 broken) |

**Neither lever is accepted from this Stage 1 round.** No composition
(`B1 + P1 + P2`) was built or measured — composition requires at least
one Stage 1 result to compose with, and both close as no-op, so there
is nothing to carry forward.

**Why both came back no-op:** this corpus (53 notes) and these
dev/calibration splits (60/30 queries, phrased in natural language per
the original corpus design) essentially never contain literal
identifier short-forms as standalone query tokens, and BM25 scores on
this corpus rarely land on an exact bitwise tie across different
status tiers. This mirrors Priority 2 Lever C's own finding closely —
a deterministic signal can be correctly implemented, correctly
isolated, and still measure nothing if the evaluation data never
exercises it. Per the design doc's own instrumentation requirement,
both ablations report their activation counts explicitly rather than
letting a zero-delta aggregate masquerade as a tested REJECT.

## Explicit confirmations

- **The new v2 held-out final-test set (`eval/rag/v2-heldout/`) has not
  been read by this session or any prior Priority 3 session.**
- **No composite configuration (`B1 + P1 + P2`) has been built or
  measured.**
- **P3 (recency tie-break) and P4 (trust tie-break) were not
  implemented or measured this round** — explicitly deferred per the
  user's authorization.
- **No Priority 4 (hybrid retrieval) or learned-reranker/embeddings
  work has started.**
- No new identifiers, source fields, or status values were constructed
  to force activations on dev or calibration for either lever — doing
  so would select the ablation's test inputs by looking at the
  evaluation data they are supposed to be judged against, the same
  calibration-leakage risk Priority 2 Lever C's own correction exists
  to prevent.
- No modification to the B3 frozen config, runner, corpus, or labels.
- No MCP, no merge, no tag, no push.
