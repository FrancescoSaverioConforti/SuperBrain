---
schema: 1
id: n9sk
type: next_action
title: >-
  B3-TUNE-v2 Priority 2 Stage 1 plan written, STOPPED for user review before
  implementation
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-14T14:53:07.930Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

docs/superpowers/plans/2026-11-14-b3-tune-v2-priority2-lexical-stage1.md committed (73073a4), building on the approved design (a9e8e90). 9 tasks: Task 1-3 Lever A (additive Bm25Index::new_with_params/Index::with_bm25_params, zero existing-call-site changes, k1/b grid on dev, one candidate to calibration), Task 4-5 Lever B (new strip_trailing_sentence_punctuation fn, never wired into frozen bm25::tokenize, diagnostic-gated against corpus+dev tokens before finalizing the stripped-character set, query-side-only ablation), Task 6-8 Lever C (structural alias candidates from corpus frontmatter + dev cross-ref only, calibration never used to select, human-reviewed expansions, frozen table, whole-token-only append-style expansion), Task 9 (per-lever KEEP/REJECT summary + optional non-gating signal-separation note, STOP). Explicitly excludes Stage 2 composition (needs separate authorization). Not yet executed -- awaiting user's plan review/approval and choice of execution method (native vs subagent-driven) before any implementation begins.
