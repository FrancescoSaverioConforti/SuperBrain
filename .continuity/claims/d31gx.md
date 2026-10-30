---
schema: 1
id: d31gx
type: decision
title: >-
  A3 final fix wave complete (6 commits, C1+I1-I5 all addressed, 342/342 tests
  passing) plus an independent controller fix for a newly-published RustSec
  advisory (libz-sys, Windows-inapplicable but bumped anyway per A2's own
  precedent) — final scoped re-review now running, last gate before merge
  decision
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-30T06:17:13.144Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Supersedes [d30cn]. Branch a3-security-durability, HEAD f814e6c. C1: restore() now excludes deleted notes and reports them via RestoreOutcome::excluded_deleted_note_ids (Option B — Option A judged unsafe since a backup taken mid-tombstone-write could hold real content shaped like a live pointer). I1: new storage::restore_vault_from_backup rebuilds a fresh journal from restored records and opens a real, readable Vault — controller independently verified this is not a second write entry point (hard preconditions: journal path must not exist, destination must be empty, no gate call, only re-derives bookkeeping from already-authenticated bytes). I2: deleted note ids are retired (StorageError::NoteDeleted refuses non-tombstone writes to them); PiiStore::get_unless_deleted checks the deletion log on read. I3: DeletionLog::reconcile now runs inside Vault::open_with_pii_store. I4/I5: 4 mutation survivors closed with dedicated tests, DeletionLog::verify_chain walks the full hash chain on both startup and restore; a second mutation-testing pass found and closed 6 more survivors. Implementer flagged two judgment calls left for the controller/user: old backups still can't restore after any deletion (left strict, deliberately conservative), and the RustSec advisory. Controller applied the libz-sys fix directly (lockfile-only, zero behavior change). Final scoped re-review dispatched — per SDD process this is the last round; residual findings get adjudicated directly, not another fix dispatch.
