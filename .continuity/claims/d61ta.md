---
schema: 1
id: d61ta
type: decision
title: >-
  P0 test fix: stale assertion fixed, but exposed a second deeper issue — test's
  core scenario is now impossible under the new lock design
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-04T10:58:37.878Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Fixed the stale assertion in a_backup_taken_while_a_deletion_is_still_pending_and_then_restored_does_not_expose_the_note_that_deletion_targets (backup.rs ~1165): replaced `reference.read().unwrap().is_none()` with an explicit genesis check (made DeletionLog::is_genesis_reference pub(crate) to let backup.rs's test module call it, since it was previously private to deletion_log.rs). Also added a ResumeGuard (Drop-based RAII) wrapping resume_tx so a panic between pause and resume no longer leaves the deleter thread parked on resume_rx.recv() forever — this was the actual deadlock mechanism (thread::scope blocks the panic from propagating until all spawned threads join).
After that fix, isolated run no longer hangs (finishes in ~30s) but now fails with a NEW error: DeletionLogRejected(WriteLockUnavailable). Root cause: the test's premise is "run backup_with_reference WHILE record_deletion is paused mid-protocol, holding the write lock" (that's the entire point of the test — observe the backup's snapshot before the pending deletion's reference update lands). But backup_with_reference calls DeletionLog::with_verified, which under the P0 design acquires the SAME write lock the paused deleter thread is holding for its whole span. So the test's original scenario — a concurrent verified reader running successfully DURING a paused writer's locked span — is now structurally impossible by the new fail-closed design itself: with_verified is correctly blocking against the held lock, which is the P0 fix working as intended, not a bug in the lock code.
This means the test's very premise needs redesign, not just its assertion: either (a) the test should assert backup_with_reference BLOCKS/fails while the deletion is paused and holds the lock (changing the test's expected outcome entirely), or (b) the pause point needs to move to a point in record_deletion's protocol where the write lock is NOT yet held (contradicts C5's "holds it for its whole span" invariant), or (c) this exact concurrent-backup-during-pending-deletion scenario is no longer something P0's design permits to test this way at all and the test needs to be rethought/retired/replaced with a sequential (pause, release, then backup) version. Did not pick an option — this needs the user's decision since it changes what the test asserts, not just how.
