---
schema: 1
id: n4oy
type: next_action
title: 'B3-TUNE-v2 initial design drafted, not authorized for implementation'
status: open
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-14T13:36:37.477Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

docs/superpowers/specs/2026-11-14-b3-tune-v2-design.md (commit 9a98efd) sketches priorities: (1) abstention thresholding (score-based NotPresent cutoff), (2) lexical tuning (BM25 params, normalization, synonym table), (3) deterministic reranking, (4) hybrid retrieval only if 1-3 insufficient. Dev/calibration reusable for v2 dev; B3's final-test is consumed (c28pk) — v2 needs a brand-new held-out final-test before claiming improvement over B1. No implementation, no new runs, no plan written yet — awaiting separate authorization to proceed past this initial design.
