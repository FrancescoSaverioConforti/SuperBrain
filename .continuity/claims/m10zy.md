---
schema: 1
id: m10zy
type: milestone
title: >-
  B3-TUNE-v2 Priority 2 Stage 1 complete and independently reviewed: Lever A
  REJECT, Lever B KEEP (corrected), Lever C REJECT (no-op)
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-14T17:56:23.703Z'
supersedes:
  - m9rr
superseded_by: null
depends_on: []
tags: []
---

Final state after independent review + fix pass (commit b89871d, on top of ee5c6a4..2312ac6). Verdicts unchanged but corrected characterization: Lever A (BM25 k1=1.8/b=0.75) REJECT -- A-score and A-full are mathematically IDENTICAL on this corpus (BM25 term scores always >0 for k1>=0,b in[0,1] with term overlap, so candidate-set membership can't change; confirmed by invariant Recall@10=0.836735 across all 16 grid points), not just 'no drift observed'. Lever B (punctuation normalization) KEEP -- but B-symmetric's numbers came from a standalone scorer (whole-corpus IDF/avgdl, not per-query candidate-scoped like real B1), confirmed biased AGAINST this lever by ~0.015 nDCG@10 vs a matched control; true effect is +0.019 nDCG@10, larger than the reported +0.0036 vs real B1 -- KEEP stands but real-pipeline confirmation recommended before any Stage 2 composition. Lever C (7-entry alias table) REJECT -- independently verified the table NEVER fired on a single dev or calibration query (expansions_fired=0/60, 0/30, now tracked explicitly in the ablation report schema) -- this is a no-op finding, not a tested-and-tied negative; root cause was Task 6's diagnostic checking note-reference instead of literal short-form occurrence in query text. Only Lever B accepted from Stage 1. No Stage 2 composition executed. Held-out v2 set confirmed never read (reviewer independently verified the file's git 'M' status was a pure line-ending artifact, now restored clean). Full regression suite green throughout.
