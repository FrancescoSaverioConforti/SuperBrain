---
schema: 1
id: d24rw
type: decision
title: >-
  A3 Task 6 Critical fix round 1 partially addressed the data-loss bug (recovery
  path verified correct) but surfaced a new serious issue: PII before-images are
  never cleaned up after successful recovery, so deleted/superseded sensitive
  ciphertext can survive indefinitely — fix round 2 dispatched
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T12:39:16.278Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Supersedes [d23jz]. Independent re-review (opus-tier, with its own repro tests) confirmed the PII before-image mechanism's recovery-side restoration is correct and hash-verified. But found: (1) the live in-process rollback path (not startup recovery) still looks up the before-image by the wrong hash, so it can never succeed on an update — confirmed non-data-losing (reopening heals via the fixed recovery path) but still a real defect; (2) NEW and more serious: no roll-forward path ever deletes the PII before-image once an operation succeeds, so a prior or DELETED sensitive note's plaintext stays decryptable by the master key indefinitely — for a tombstone/deletion write specifically, this means deleted content survives its own deletion, undermining C5's core deletion guarantee. find_orphans does not scan .before-images, so this leak isn't even detectable today. Fix round 2 dispatched: before-image cleanup on every roll-forward path (mirroring the Markdown before-image's existing GitCommitted-triggered delete), the wrong-hash fix, fail-closed on non-NotFound I/O errors in commit_staged, and restoring the Markdown pointer in one remaining unguarded rollback branch.
