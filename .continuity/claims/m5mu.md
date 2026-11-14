---
schema: 1
id: m5mu
type: milestone
title: >-
  B3.3/B3.4 complete: metric runner built, reviewed, fixed; abstention threshold
  cannot be set from this data
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-14T12:34:54.655Z'
supersedes:
  - rk4m1
superseded_by: null
depends_on: []
tags: []
---

Branch b3-retrieval-eval, 12 commits off main. Native Rust metric runner (crates/api/src/metrics.rs + crates/api/examples/b3_metric_runner.rs) computes true Recall@10, frozen-formula nDCG@10, deterministic seeded bootstrap CIs, and an abstention confusion matrix with null-on-undefined precision/recall/F1. Independent review (opus) found 0 Critical, 3 Important, 8 Minor; fixed both Important code issues (NotPresent-on-answerable-query was silently dropped from aggregation instead of scoring 0 — latent bias, no effect on today's data since FP=0; vacuous bootstrap test strengthened) in commit e613edd, regenerated reports in c7ad08e (numbers unchanged, confirming fix is behavior-preserving on this data). Real results: dev Recall@10=0.837 CI[0.735,0.939], nDCG@10=0.714; calibration Recall@10=0.917 CI[0.792,1.0], nDCG@10=0.709. Key finding: abstention confusion matrix is TP=0, FP=0 in BOTH splits — the B1 baseline never returns NotPresent for any of 17 abstain-expected queries across dev+calibration, so no abstention precision/recall threshold can be meaningfully calibrated (precision undefined, recall=0.0 exactly). Reported to user with GO on Recall@10/nDCG@10 thresholds, HOLD on abstention pending explicit decision on how to treat the constant classifier. No frozen-config.json written (pending user approval), no merge/tag/push, no MCP, no final-test access.
