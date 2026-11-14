---
schema: 1
id: rk4m1
type: risk
title: B3 dev/calibration runs show zero true abstention (TP=0 in both splits)
status: open
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-14T12:26:22.679Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Real B3.3/B3.4 runner output (eval/results/B3/dev-*.json, calibration-*.json): confusion matrix TP=0 in both dev (11 FN/60) and calibration (6 FN/30) — the retrieval baseline (B1 trigram+BM25) never returns SearchOutcome::NotPresent for abstention-expected queries, so abstention recall is exactly 0.0 (precision/F1 correctly null, zero denominator). Aggregate Recall@10 is otherwise healthy (dev 0.84, calibration 0.92). This is real empirical signal for the B3.4 threshold proposal, not a runner bug — confirmed by the runner's own tests and an independent branch review (dispatched, pending at time of capture).
