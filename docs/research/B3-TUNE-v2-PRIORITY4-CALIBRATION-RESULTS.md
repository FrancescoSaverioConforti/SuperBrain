# B3-TUNE-v2 Priority 4 — Calibration Results and Mechanical Verdict

**Date:** 2026-11-14. **Status:** Calibration run, executed EXACTLY
ONCE per configuration, per explicit user authorization. No re-run.
No threshold change after seeing these results. No model change. No
representation-semantics change. No RRF tuning. No weighted fusion.
`eval/rag/v2-heldout/` was not opened. The old B3 final-test was not
re-run (B1's numbers below are reused verbatim from its existing
frozen report, never re-measured).

Frozen model: `all-MiniLM-L6-v2` (Stage 1 Task 4, final, not
reopened). Frozen representation semantics, candidate depths, RRF
constant `k=60`, similarity metric: unchanged from
`docs/superpowers/specs/2026-11-14-b3-tune-v2-priority4-stage1-dev-freeze.md`.
Frozen promotion criteria: unchanged from
`docs/research/B3-TUNE-v2-PRIORITY4-STAGE1-TASK6-PROMOTION-CRITERIA.md`,
as approved by the user.

Calibration split: `eval/rag/calibration/labels.jsonl` (n=30 rows, 24
answerable after excluding the canary and abstain-expected rows;
`sha256=7ff4303a12003dbfb1e00b3875f9c1395f8df27d286952c2139562dd2151b049`).

## Methodology note: B1 cost-control reused, not re-run

B1's same-harness cost-control figures are reused directly from Stage
1 (`eval/results/B3-TUNE-v2/priority4-cost-B1-cost-control.json`:
warm-query P95 = 124.538ms, build time = 144.964ms), rather than
re-measured against the calibration split specifically. Rationale: B1's
own search cost does not depend on which 24 vs. 49 specific questions
are asked — only on corpus size (unchanged, 53 notes) and the
underlying `index::search` code path (unchanged, same binary). A
second B1 cost-control run against 24 calibration questions instead of
49 DEV questions would measure sampling noise in process-level timing,
not a real methodology difference. B1's own frozen Recall@10/nDCG@10
quality numbers (`eval/results/B3/calibration-20261114T123333Z-e613edd.json`:
Recall@10=0.9166666666666666, nDCG@10=0.7090535447985884, n=24) are
likewise reused verbatim, never re-measured — consistent with the
user's explicit instruction.

---

## 1. MiniLM dense-only — calibration results

Source: `eval/results/B3-TUNE-v2/priority4-calibration-dense-all-MiniLM-L6-v2.json`,
`priority4-calibration-cost-all-MiniLM-L6-v2.json`.

| Metric | Value |
|---|---|
| Recall@10 | 1.0000, CI [1.0000, 1.0000] |
| nDCG@10 | 0.8614, CI [0.7608, 0.9455] |
| Delta Recall@10 vs. B1 frozen calibration | +0.0833 |
| Delta nDCG@10 vs. B1 frozen calibration | +0.1524 |
| candidate_recall_rate | 1.0000 (24/24) — **DEV's 1.0 finding replicates on calibration** |
| Abstention confusion matrix | TP=0, FP=0, TN=0, FN=6 — same constant-non-abstaining pattern as B1 throughout dev/calibration. **Tripwire: no red flag.** |
| Warm-query P50 / P95 | 8.750ms / 9.073ms |
| Embedding-inference P50 / P95 | 8.576ms / 8.905ms |
| Vector-search P50 / P95 | 0.1794ms / 0.1900ms |
| Peak RAM (after queries) | 159,158,272 bytes ≈ 151.8MB |
| Model-weight footprint | 90,405,214 bytes ≈ 86.2MB (90.4MB decimal) |
| Vector-index footprint | 81,408 bytes for 53 notes ≈ 1536 bytes/note |
| Build/index time | 549.56ms |

### Category breakdown (calibration, n=24)

| Category | Recall@10 | nDCG@10 | n | low_confidence |
|---|---|---|---|---|
| bugs-fixes | 1.0 | 0.8436 | 6 | no |
| identifier-code | 1.0 | 0.8524 | 5 | no |
| notes-decisions | 1.0 | 0.8577 | 4 | yes |
| patterns-knowledge | 1.0 | 0.8223 | 4 | yes |
| cross-file-concept | 1.0 | 0.8770 | 3 | yes |
| temporal | 1.0 | 1.0000 | 2 | yes |

### Difficulty breakdown

| Difficulty | Recall@10 | nDCG@10 | n |
|---|---|---|---|
| easy | 1.0 | 0.9539 | 8 |
| medium | 1.0 | 0.7983 | 11 |
| hard | 1.0 | 0.8524 | 5 |

## 2. MiniLM + B1 hybrid-RRF — calibration results

Source: `eval/results/B3-TUNE-v2/priority4-calibration-hybrid-all-MiniLM-L6-v2.json`.

| Metric | Value |
|---|---|
| Recall@10 | 0.9583, CI [0.8750, 1.0000] |
| nDCG@10 | 0.7945, CI [0.6658, 0.9013] |
| Delta Recall@10 vs. B1 frozen calibration | +0.0417 |
| Delta nDCG@10 vs. B1 frozen calibration | +0.0855 |
| Delta nDCG@10 vs. same-run dense-standalone | **-0.0669** |
| candidate_recall_rate (dense side of the fused set) | 1.0000 (24/24) — replicates |
| Fusion-step cost | P50=0.0564ms, P95=0.0982ms — negligible relative to embedding inference |
| Abstention confusion matrix | TP=0, FP=0, TN=0, FN=6 — unchanged. **Tripwire: no red flag.** |
| Warm-query / embedding-inference / vector-search | Same per-query embedding-inference and vector-search cost as dense-only above (same model, same corpus); fusion adds the negligible cost above on top |
| Peak RAM, model-weight, vector-index, build time | Same as dense-only above (identical model, corpus, and index — hybrid only adds the lexical candidate list and fusion step, not a second model or index) |

### B1-lexical-only, reproduced inside this same run (for internal consistency check)

| Metric | Value |
|---|---|
| Recall@10 | 0.9167 — matches B1's frozen calibration figure (0.916666...) exactly |
| nDCG@10 | 0.7091 — matches B1's frozen calibration figure (0.709053...) exactly |

This exact match confirms the harness's lexical path (via the real
`api::search`) reproduces B1's own frozen calibration numbers, not an
independent approximation — the three-way comparison's B1 column is
internally verified, not merely assumed.

### Category breakdown (hybrid-RRF, calibration, n=24)

| Category | Recall@10 | nDCG@10 | n | low_confidence |
|---|---|---|---|---|
| bugs-fixes | 1.0 | 0.9167 | 6 | no |
| identifier-code | 0.8 | 0.6262 | 5 | no |
| notes-decisions | 1.0 | 0.8155 | 4 | yes |
| patterns-knowledge | 1.0 | 0.7217 | 4 | yes |
| cross-file-concept | 1.0 | 0.7630 | 3 | yes |
| temporal | 1.0 | 1.0000 | 2 | yes |

### Difficulty breakdown (hybrid-RRF)

| Difficulty | Recall@10 | nDCG@10 | n |
|---|---|---|---|
| easy | 0.875 | 0.7664 | 8 |
| medium | 1.0 | 0.8006 | 11 |
| hard | 1.0 | 0.8262 | 5 |

**Calibration replicates DEV's headline finding:** hybrid-RRF
underperforms dense-standalone on calibration too (nDCG@10 0.7945 vs.
0.8614, a -0.0669 absolute gap) — not a DEV-only artifact of the
smaller 53-note corpus's query mix.

---

## 3. Mechanical gate-by-gate verdict

All thresholds below are the frozen, user-approved criteria from
`B3-TUNE-v2-PRIORITY4-STAGE1-TASK6-PROMOTION-CRITERIA.md`, unmodified
after seeing these results.

| Gate | Threshold | Dense-only | Hybrid-RRF |
|---|---|---|---|
| Recall@10 regression | ≤ 0.05 absolute below B1 (0.9167) | 1.0000, delta +0.0833 → **PASS** | 0.9583, delta +0.0417 → **PASS** |
| nDCG@10 improvement vs. B1 | ≥ +0.03 absolute above B1 (0.7091) | +0.1524 → **PASS** | +0.0855 → **PASS** |
| nDCG@10 vs. same-run dense-standalone | ≥ 0.00 (hybrid must not be worse) | n/a (this is the comparison point) | **-0.0669 → FAIL** |
| Warm-query P95 latency | ≤ 3× B1 P95 (124.538ms → 373.614ms) | 9.073ms → **PASS** | 9.073ms + negligible fusion → **PASS** |
| Peak RAM | ≤ 300MB | 151.8MB → **PASS** | 151.8MB (same model/index) → **PASS** |
| Model-weight footprint | ≤ 150MB | 90.4MB → **PASS** | 90.4MB (same model) → **PASS** |
| Vector-index disk footprint | ≲ 2KB/note | ~1536 bytes/note → **PASS** | ~1536 bytes/note (same index) → **PASS** |
| Build/index time | ≤ 5× B1 build (144.964ms → 724.82ms) | 549.56ms (≈3.79×) → **PASS** | 549.56ms (same build) → **PASS** |

**Dense-only: passes all gates — 7/7.**
**Hybrid-RRF: fails exactly one gate — 6/7 (nDCG@10 vs. dense-standalone).**

## 4. Final verdict

**Rule path: only one candidate passes all gates → promote that
candidate.**

Hybrid-RRF fails the "nDCG@10 vs. dense-standalone" gate — the single
criterion added specifically because of the DEV-stage headline finding
(Section 1 of the Stage 1 Task 6 proposal), and that finding is now
confirmed to replicate on calibration (Section 2 above). Per the
frozen rule, this gate failure is decisive on its own, regardless of
how well hybrid-RRF does on every other gate — the rule does not
require failing a majority of gates, only this one, since it exists
precisely to block promoting a fusion strategy that is measurably
worse than the simpler dense-only alternative.

**VERDICT: PROMOTE `MiniLM dense-only`. REJECT `MiniLM + B1 hybrid-RRF`
for promotion, despite passing 6 of 7 gates, because it fails the gate
requiring it not be worse than dense-standalone on the same
calibration data.**

This is not the "both pass → compare nDCG@10" rule path (that path
does not apply here, since hybrid did not pass all gates) and not the
"neither passes → REJECT the whole Priority 4 candidate" path (dense-
only did pass). Exactly one candidate — dense-only — cleared every
frozen gate.

## 5. What this calibration run does not do

- Does not read `eval/rag/v2-heldout/` — never opened.
- Does not re-run the old B3 final-test — B1's numbers are reused
  verbatim from its existing frozen report.
- Does not change the model, representation semantics, candidate
  depths, RRF constant, or any promotion-criteria threshold after
  seeing these results.
- Does not implement weighted score fusion.
- Does not merge, tag, or push.

## 6. Next step

Per the Stage 1/calibration authorization's STOP requirement, this
session stops here. The next step, if and when authorized, concerns
what to do with a PROMOTED dense-only configuration (e.g. held-out
validation, real index-format implementation, decommissioning the
hybrid-RRF code path) — not started by this document.
