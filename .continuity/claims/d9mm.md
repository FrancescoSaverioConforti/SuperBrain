---
schema: 1
id: d9mm
type: decision
title: >-
  read_note/current_version must never fall back to a before-image on hash
  mismatch — always StorageError::Inconsistent; only reconcile() may roll back
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-27T08:27:50.262Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Third correction round on the A2 plan (docs/superpowers/plans/2026-10-27-a2-recovery.md, Task 4). A read-time before-image fallback was found to pair one operation's content with a different operation's committed version number (e.g. op2's before-image "v1" returned as version 2). Fix: reads have no fallback; a hash mismatch on the journal-resolved row is unconditionally Inconsistent. Only reconcile() (Task 7) may demote an operation to Failed and restore a before-image, since only it can correctly re-derive which prior operation's row is now linearized. Independent reviewer dispatched to verify this plus two other fixes (illegal IntentRecorded->JournalCommitted test transitions in Task 7; ScanDepth::Unbounded required for reconcile's git-commit lookup to avoid duplicate commits). User's approval is conditional on this reviewer confirming all three — not yet resolved.
