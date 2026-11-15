# B3-TUNE-v2 Priority 1 — Abstention Threshold Proposal

**Status:** PROPOSAL ONLY. Not frozen, not implemented, not applied
anywhere. Per the design doc's two-stop discipline (same as B3.4), this
document is for the user's explicit approval before Task 3 (runner
implementation) begins.

## Signal chosen: `ratio` (top1_score / top2_score)

Task 1 (dev, n=60) and this task's rerun (calibration, n=30) both show
`ratio` as the signal with by far the smallest overlap magnitude between
abstain-expected and answerable groups:

| Signal | Dev separation_gap | Calibration separation_gap |
|---|---|---|
| `top1_score` | -6.571 | -7.808 |
| `margin` | -3.323 | -3.515 |
| `ratio` | **-0.744** | **-0.445** |

(Negative = the groups overlap; smaller magnitude = less overlap.
No signal cleanly separates the two groups on either split — this
corpus's abstention cases do not sit in a distinct score region from its
hardest answerable cases — but `ratio` is consistently and substantially
the least-overlapping of the three, on both splits independently.)

`ratio` is also scale-invariant (unlike `top1_score`/`margin`, which
depend on the corpus's absolute BM25/trigram score range and would need
re-calibrating if corpus size or indexing weights changed), which is a
secondary reason to prefer it as the frozen signal if accepted.

## Deterministic cutoff sweep over calibration (n=30, 6 abstain-expected / 24 answerable)

Every distinct observed `ratio` value on calibration was evaluated as a
candidate cutoff (30 candidates; the full table is
`eval/results/B3-TUNE-v2/priority1-threshold-sweep.json`). Classification
rule: `ratio < cutoff` → predicted abstention (`NotPresent`), per the
design doc's "below threshold → abstain" direction. Key rows:

| Cutoff | TP | FN | FP | TN | Abstention recall | False-abstention rate | F1 | Recall@10 (Δ vs B1) | nDCG@10 (Δ vs B1) |
|---|---|---|---|---|---|---|---|---|---|
| 1.0010 (min) | 0 | 6 | 0 | 24 | 0.0 | 0.0 | — | 0.9167 (+0.000) | 0.7091 (+0.000) |
| 1.0301 | 0 | 6 | 4 | 20 | 0.0 | 0.167 | — | 0.7917 (−0.125) | 0.6280 (−0.081) |
| **1.1079** | **3** | **3** | **4** | **20** | **0.50** | **0.167** | **0.462** | **0.7917 (−0.125)** | **0.6280 (−0.081)** |
| 1.1100 | 3 | 3 | 5 | 19 | 0.50 | 0.208 | 0.429 | 0.7500 (−0.167) | 0.6017 (−0.107) |
| 1.3390 | 4 | 2 | 10 | 14 | 0.667 | 0.417 | 0.400 | 0.5417 (−0.375) | 0.4526 (−0.256) |
| 1.5169 | 6 | 0 | 14 | 10 | 1.0 | 0.583 | 0.462 | 0.3750 (−0.542) | 0.3359 (−0.373) |
| 5.1576 (max) | 6 | 0 | 23 | 1 | 1.0 | 0.958 | 0.343 | 0.0417 (−0.875) | 0.0417 (−0.667) |

Full picture (all 30 rows in the JSON): abstention recall and the
false-abstention rate rise together monotonically as the cutoff
increases — there is no cutoff that gains abstention recall "for free."
Every unit of abstention recall costs answerable-query false positives,
and because this ratio signal doesn't separate the groups cleanly (see
above), the cost is steep: reaching abstention recall 1.0 (cutoff
≥1.5169) means flagging 14-23 of the 24 answerable calibration queries as
false abstentions too, which collapses Recall@10 from 0.9167 to as low as
0.04-0.38.

## Proposed threshold: `ratio < 1.1079` → predicted abstention

**Rationale:** `1.1079` is the last cutoff in a 4-candidate plateau
(1.0301, 1.0336, 1.0852, 1.1079) that all share the identical Recall@10/
nDCG@10 cost (−0.125 / −0.081 vs. the official B1 baseline), because no
answerable query's ratio falls in that interval — only abstain-expected
queries' ratios do. Within that plateau, `1.1079` dominates: it reaches
abstention recall 0.50 (TP=3/6) and the plateau's best F1 (0.462, tied
with the "flag everything" cutoff 1.5169 but at a far lower
false-abstention rate: 0.167 vs 0.583), while the three lower candidates
in the same plateau achieve strictly worse abstention recall (0.0-0.33)
at the exact same Recall@10/nDCG@10 cost. Moving one step further to
1.1100 adds one more false positive (FP 4→5) for no abstention-recall
gain at all — a strictly worse trade. `1.1079` is the point where this
plateau's "free" improvement in abstention detection is fully captured
without yet paying the next real Recall@10/nDCG@10 cost increment.

**The trade-off this proposal explicitly accepts:** at `ratio < 1.1079`,
the B1+threshold variant would catch half of calibration's
abstain-expected queries (3/6) while incorrectly flagging 4 of 24
answerable queries (16.7% false-abstention rate) as abstentions too, and
would cost 0.125 absolute Recall@10 and 0.081 absolute nDCG@10 versus the
frozen B1 baseline's calibration numbers. This is NOT a free win — it is
a real precision/recall trade this proposal surfaces for approval, not a
claim that this cutoff dominates every metric. A reviewer preferring a
more conservative trade (lower false-abstention rate, lower Recall@10
cost, lower abstention recall) might instead favor one of the
`0.0`-abstention-recall cutoffs below `1.0301`; a reviewer preferring a
more aggressive one might favor `1.3390` (TP=4, FN=2, at a much steeper
Recall@10 cost of −0.375).

**This is a proposal, not an automatic selection.** No "best" row was
chosen by maximizing any single column (F1 alone, for instance, is
actually tied between `1.1079` and `1.5169` — the tie-break reasoning
above, not F1 alone, is what selects `1.1079`). Nothing is applied,
implemented, or frozen by this document. Task 3 (the runner's
`--abstention-threshold` flag) does not begin until this specific
proposal — signal `ratio`, cutoff `1.1079` — is explicitly approved.

## What this proposal does not do

- Does not read or use the new B3-TUNE-v2 v2-heldout set in any way.
- Does not apply any override to `index::search`'s production behavior.
- Does not freeze anything — a future `frozen-config.json`-equivalent
  for v2 would only happen after the full Priority 1 ablation (Task 4)
  and, separately, after the P1→P4 proceed decision is made from
  dev/calibration/ablation/resource evidence (never from this proposal
  alone, and never from the held-out set).
