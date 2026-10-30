---
schema: 1
id: d33rj
type: decision
title: >-
  A3 post-N1 follow-up: stale backups (older than the latest deletion) now
  restore their still-valid notes via per-note exclusion, instead of being
  rejected wholesale — commit a67c6d6, no new trust architecture,
  DeletionLogError::ReferenceStale split into two distinct variants to avoid
  conflating "backup is stale" with "live log is untrustworthy"
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-30T07:26:20.912Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User-authorized scoped change with 6 explicit requirements (restore valid notes from old backups when the live log is complete/authenticated/consistent; exclude only deleted notes; fail closed on an untrustworthy log; preserve C5 without editing the spec; RED/GREEN tests for valid-historical-backup/successive-deletions/tampered-log/storage-API-restore; preserve all prior fixes including N1). Pre-implementation check confirmed the existing DeletionLog/reconcile/is_deleted primitives already prove all required properties — only the restore gate's policy decision needed to change, not the trust model. Core fix: split the overloaded ReferenceStale error into ReferenceStale (live log disagrees with its own reference inside reconcile — always fails closed) and BackupReferenceStale (returned only after reconcile already succeeded — backup's own snapshot is merely older than an already-trustworthy live state — now safe to fall through to per-note exclusion). 4 new tests, 3 pre-existing tests' assertions updated to match the corrected (still-safe) outcome. 347/347 tests passing, clippy/deny clean. One targeted independent review dispatched per the user's explicit request before final authorization.
