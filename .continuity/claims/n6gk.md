---
schema: 1
id: n6gk
type: next_action
title: 'B3-TUNE-v2 Priority 1 plan written, awaiting authorization to execute'
status: open
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-14T13:54:35.749Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

docs/superpowers/plans/2026-11-14-b3-tune-v2-priority1-abstention.md (commit 5c01e4a), 5 tasks: (1) build/hash/freeze a NEW held-out query set at eval/rag/v2-heldout/ before any tuning; (2) signal analysis comparing top1 score/margin/ratio on dev (diagnostic binary, no production code change — SearchResult::score already exists); (3) propose ONE global threshold from calibration, STOP for approval; (4) implement the threshold as an optional --abstention-threshold runner flag only after approval, never changing index::search's own behavior; (5) run dev+calibration ablation, log per the experimental rule, propose KEEP/REJECT, STOP — explicitly does not run the new held-out set. Priorities 2-4 remain design-only. No implementation started yet; user has not yet authorized execution of this plan.
