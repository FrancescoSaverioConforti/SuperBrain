---
schema: 1
id: m6bf
type: milestone
title: 'B3 baseline frozen: Recall@10>=0.79, nDCG@10>=0.57, no abstention threshold'
status: open
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-14T13:18:21.817Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

eval/results/B3/frozen-config.json committed (52d42ea) per explicit user approval. Frozen: retrieval_config=B1 deterministic trigram+BM25 no embeddings; Recall@10 min 0.79; nDCG@10 min 0.57 (both = calibration's 95% bootstrap CI lower bounds); abstention has NO numeric threshold — recorded as a finding (baseline abstention recall=0, constant non-abstaining, TP=0/FP=0 both splits); bootstrap seed=0, 2000 resamples; metric formulas copied verbatim from the approved design. Hashes (corpus manifest, calibration labels) independently reverified against disk before writing. No final-test read/run, no MCP, no merge/tag/push. This is the configuration any B3.5 final-test run must be measured against; changing it after reading final-test results invalidates that run per the file's own hard_rule.
