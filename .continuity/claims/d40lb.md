---
schema: 1
id: d40lb
type: decision
title: >-
  Targeted review of commit 41005c6 (same tree as e3ad7ef) returned NO-GO: F1
  (Important, confirmed/reproduced) — deletion recheck in recovery.rs and
  read_note's gate only covers Sensitive (pii_content_hash) rows; a deleted
  non-sensitive note can still be served after crash recovery or an ordinary
  delete, and RestoreOutcome misreports it as excluded while it's actually
  readable
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-30T11:47:39.341Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Minor findings also returned: F2 (reconcile's roll-forward writes don't take the new write lock — availability/fail-closed issue, not confidentiality), F3 (tombstone exemption can collide with legitimately-empty Sensitive note content, confirmed non-exploitable), F4 (raw PiiStore getters still pub, doc-comment-only enforcement, no current exploit path). Baseline 366/366 passing; 7 throwaway repro probes run and reverted, 2 failed confirming F1. Controller has not fixed anything, per user's explicit instruction to present findings first. Awaiting user's decision on fix-wave scope.
