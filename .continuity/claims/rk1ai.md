---
schema: 1
id: rk1ai
type: risk
title: >-
  A note reclassified sensitive->plain->sensitive can permanently block
  Vault::open after an ordinary crash (needs an operator, no data loss) —
  pre-existing behavior found during A3 Task 6 review, not fixed, worth a
  dedicated task
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T17:17:49.308Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Root cause: after a non-sensitive write on a previously-sensitive note, the last-committed row has no pii_content_hash to verify a later sensitive write's before-image against, so recovery can get permanently stuck at Inconsistent after a crash. Also: a non-sensitive write leaves the previous sensitive note's .pii file live and decryptable, which may conflict with C3/C5's intent. Found and confirmed pre-existing (reproduces identically on commits before and after Task 6's round-3 fix) during the A3 Task 6 independent re-review.
