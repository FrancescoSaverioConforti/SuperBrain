---
schema: 1
id: c26kq
type: constraint
title: 'B3 execution order and stop gate: B3.1-B3.4 then STOP for review'
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-14T07:45:54.578Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Authorized now: B3.1 (corpus/query set) → B3.2 (ground truth + annotation protocol) → B3.3 (Recall@10/nDCG@10 runner + CI) → B3.4 (calibration + threshold freeze). After B3.4, STOP and report results before final test (B3.5). Report only: corpus/query set, split, annotation protocol, metric runner, dev/calibration results, proposed thresholds, blockers, GO/NO-GO for minimal MCP + final test. No merge/tag/push. A5 not authorized. Task 10 not authorized.
