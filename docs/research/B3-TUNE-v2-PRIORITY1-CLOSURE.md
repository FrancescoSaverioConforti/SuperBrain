# B3-TUNE-v2 Priority 1 Closure — REJECT

**Phase:** B3-TUNE-v2, Priority 1 (abstention thresholding).
**Date:** 2026-11-14.
**Verdict: REJECT — single global score-derived abstention threshold insufficient.**

---

## 1. Scope recap

Priority 1's plan
(`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority1-abstention.md`)
measured whether a single, global, score-derived signal threshold could
give B1's retrieval a working abstention signal, without touching
`index::search`'s production behavior — purely as a reporting-layer
override, measured against dev/calibration. Tasks executed: Task 0
(held-out set construction, isolated via subagent dispatch), Task 1
(signal analysis on dev), Task 2 (deterministic cutoff sweep on
calibration + threshold proposal). Task 3 (runner implementation) and
Task 4 (ablation with a chosen threshold) were **never executed** — this
closure happens at the Task 2 decision point, per user instruction.

## 2. Signal analysis (Task 1, Task 2 Step 1) — ACCEPTED

Three candidate signals were compared on both `dev` (n=60, 11
abstain-expected/49 answerable) and `calibration` (n=30, 6
abstain-expected/24 answerable), using each query's top-1/top-2
`SearchResult::score`:

| Signal | Dev separation_gap | Calibration separation_gap |
|---|---|---|
| `top1_score` | -6.571 | -7.808 |
| `margin` (top1 − top2) | -3.323 | -3.515 |
| `ratio` (top1 / top2) | **-0.744** | **-0.445** |

`ratio` is confirmed, independently on both splits, as the signal with
the smallest overlap magnitude between the abstain-expected and
answerable groups — a smaller gap means the two groups' value ranges sit
closer together, i.e. less separation, but `ratio`'s gap is consistently
and substantially the smallest of the three. This conclusion is
**accepted** and is not what this closure rejects.

Reports: `eval/results/B3-TUNE-v2/priority1-signal-analysis.json` (dev),
`priority1-signal-analysis-calibration.json` (calibration). Commits
`629fff6`, `8b10381`.

## 3. Deterministic cutoff sweep (Task 2 Steps 2-3) — evidence preserved

Every one of the 30 distinct `ratio` values observed on calibration was
evaluated as a candidate cutoff (`ratio < cutoff` → predicted
abstention), producing a full Pareto/sensitivity table: confusion
matrix, abstention precision/recall/F1, false-abstention rate, and
Recall@10/nDCG@10 (with delta vs. the official B1 baseline) at every
cutoff.

Report: `eval/results/B3-TUNE-v2/priority1-threshold-sweep.json`.
Commit `8b10381`.

### Guardrail check (user-specified, this session)

Guardrails: abstention recall > 0, false-abstention rate ≤ 10%,
|Recall@10 delta| ≤ 0.05, |nDCG@10 delta| ≤ 0.05.

**Result: zero of the 30 swept cutoffs satisfy all four guardrails
simultaneously.**

- The most conservative cutoff within the false-abstention-rate ≤ 10%
  guardrail (`1.0090`, FAR = 8.3%) still has abstention recall = 0 — no
  genuine abstention detection at all.
- The first cutoff with any abstention recall at all (`1.0336`,
  abstention recall = 0.167) already has FAR = 16.7%, Recall@10 delta
  = −0.125, nDCG@10 delta = −0.081 — all three remaining guardrails
  violated simultaneously, at the very first step where abstention
  recall becomes nonzero.
- The Pareto frontier among all recall > 0 candidates is exactly
  `{1.0336, 1.1079, 1.3390, 1.5169}` (each dominates the next on at
  least one dimension); none of these four falls within the guardrails.

| Cutoff | TP | FN | FP | TN | Abst. recall | FAR | Recall@10 (Δ) | nDCG@10 (Δ) |
|---|---|---|---|---|---|---|---|---|
| 1.0090 | 0 | 6 | 2 | 22 | 0.0 | 8.3% | 0.8750 (−0.042) | 0.6828 (−0.026) |
| 1.0336 | 1 | 5 | 4 | 20 | 0.167 | 16.7% | 0.7917 (−0.125) | 0.6280 (−0.081) |
| 1.1079 | 3 | 3 | 4 | 20 | 0.500 | 16.7% | 0.7917 (−0.125) | 0.6280 (−0.081) |
| 1.3390 | 4 | 2 | 10 | 14 | 0.667 | 41.7% | 0.5417 (−0.375) | 0.4526 (−0.256) |
| 1.5169 | 6 | 0 | 14 | 10 | 1.000 | 58.3% | 0.3750 (−0.542) | 0.3359 (−0.373) |

## 4. Decision history

1. First proposal (this session, prior to user review): signal `ratio`,
   cutoff `1.1079`. Rationale: dominates a 4-candidate plateau sharing
   identical Recall@10/nDCG@10 cost. **NOT APPROVED by user** — FAR
   16.7% too high, Recall@10 (0.7917) too close to the old frozen B1
   acceptance floor (0.79), nDCG@10 cost too high.
2. User requested the conservative-side guardrail shortlist from the
   existing sweep (no new run). Result: no cutoff satisfies all four
   guardrails (section 3 above).
3. **Final user decision: REJECT the single-signal / single-global-
   threshold configuration entirely.** Rationale (verbatim from the
   user): "il problema non è la scelta del cutoff; la separabilità di
   un singolo segnale è insufficiente." Explicitly declined for now:
   per-category thresholds, ad hoc signal combinations (e.g.
   ratio+top1), multi-signal rules, and further tuning against
   calibration — with n=6 abstention-positive queries in calibration,
   additional search over threshold space risks overfitting to noise
   rather than finding real signal.

## 5. Verdict

**REJECT — single global score-derived abstention threshold
insufficient**, on this corpus, with this signal set, measured against
calibration's 6 abstain-expected queries. `ratio` remains the best of
the three signals measured, but no cutoff of it alone delivers
meaningful abstention detection without an unacceptable cost to
Recall@10/nDCG@10 or an unacceptably high false-abstention rate.

This is a **negative result about single-signal thresholding**, not a
negative result about every possible path to abstention-awareness.
Multi-signal approaches, per-category thresholds (once there's a larger
corpus to support them), or architectural changes (e.g. retrieval
confidence calibration, lexical tuning that sharpens the score
distribution itself) remain open for a future, separately-authorized
round — none of which this closure authorizes.

## 6. Preserved evidence

All signal-analysis and sweep evidence remains committed and
unmodified, as instructed:

- `crates/api/examples/b3_tune_v2_signal_analysis.rs` (commits
  `629fff6`, `8b10381`)
- `eval/results/B3-TUNE-v2/priority1-signal-analysis.json` (dev)
- `eval/results/B3-TUNE-v2/priority1-signal-analysis-calibration.json`
- `eval/results/B3-TUNE-v2/priority1-threshold-sweep.json` (full
  30-cutoff Pareto table)
- `eval/results/B3-TUNE-v2/priority1-threshold-proposal.md` (the
  rejected `1.1079` proposal and its rationale, kept for the record)
- This document.

## 7. What did NOT happen

- Task 3 (runner `--abstention-threshold` implementation): **not
  executed.**
- Task 4 (dev/calibration ablation with a chosen threshold): **not
  executed.**
- The new v2 held-out set (`eval/rag/v2-heldout/`): **remains
  completely unread** by this or any prior session in this phase — its
  content was never opened at any point during Priority 1, including
  this closure.
- No per-category threshold, multi-signal rule, or further calibration
  tuning was introduced.
- No modification to the B3 frozen config, runner, corpus, or labels.
- No MCP, no merge, no tag, no push.

## 8. Next step

Per user instruction: prepare a design/plan for **Priority 2 — Lexical
Tuning**, starting from the official B1 baseline (not from any rejected
Priority 1 threshold). Design-only; no implementation until separately
authorized.
