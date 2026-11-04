---
schema: 1
id: d62sn
type: decision
title: >-
  P0: both previously-hung pending-restore tests fixed and verified; handoff doc
  reconciled
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-04T11:07:08.482Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

docs/handoffs/P0-deletionlog-handoff.md (dated 2026-11-03, written by the prior session) was found AFTER my initial empty-glob check — it exists and documents the exact same two hung tests I independently diagnosed: a_backup_taken_while_a_deletion_is_still_pending_and_then_restored_does_not_expose_the_note_that_deletion_targets and restoring_from_a_sentinel_backup_while_the_deletion_is_still_pending_is_rejected_not_silently_succeeded. The prior session had reproduced both hangs but documented them as still unresolved/not-yet-green (handoff section 5/7). My fixes (this session) resolve both:
1. Split the first into two tests with distinct properties per the user's instruction: a_backup_attempted_while_a_deletion_is_paused_mid_protocol_is_rejected_not_served (asserts WriteLockUnavailable + no blob written, using a second short-timeout handle) and a_backup_taken_after_a_pending_deletion_completes_uses_the_updated_reference_and_restore_excludes_the_note (sequential: pause/resume fully, then backup+restore, asserting generation 1 + exclusion).
2. Restructured the second (renamed restoring_from_a_pre_deletion_backup_while_a_deletion_is_still_pending_waits_then_excludes_not_silently_succeeded): takes its backup BEFORE the deletion starts (embeds genesis), then pauses the deletion and runs restore concurrently — preserving the original "restore racing a still-pending deletion must never expose the note" property without attempting a backup while the write lock is held.
Both deadlocks had the same root mechanism: a panicking assert! inside std::thread::scope left a spawned thread parked on resume_rx.recv() forever, since thread::scope blocks the panic from propagating until all its threads join. Fixed via a Drop-based ResumeGuard wrapping the resume Sender in both affected tests.
Verification after fix: cargo test -p vault-security --locked -- --test-threads=1 -> 97/97 passed, 15.49s, no hang. Default-threaded run -> 97/97 passed, 5.99s. cargo clippy -p vault-security --locked -- -D warnings clean. Relevant storage A3 integration tests (a3_confidentiality_boundary, a3_pii_integration, a3_backup_restore) all pass, 24/24. Made DeletionLog::is_genesis_reference pub(crate) (was private) so backup.rs's test module could assert the real genesis predicate instead of reimplementing it — not re-exported from lib.rs, stays internal to the crate.
Nothing committed yet. Workspace-wide cargo test --workspace --locked and cargo deny check (handoff section 7's remaining items) have not been run this session.
