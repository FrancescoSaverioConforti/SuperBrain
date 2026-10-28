---
schema: 1
id: d17yz
type: decision
title: >-
  A2 item-26 residual fully closed (both variants): commits 88b20f6 + e5e2506 +
  f807c34, 213->220 tests, scoped review found no Critical/Important issues,
  branch a2-recovery ready for user's merge-authorization decision
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T07:00:00.469Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Second variant (two stalled ops on same note, neither ever committed) closed via two additive guards: a live write guard (vault.rs, refuses a write when another row on the same note is ContentPersisted/Recovering/IntentRecorded-with-hash, before any filesystem effect) and a recovery-correctness guard (recovery.rs, skips restoring a before-image that's solely a never-committed sibling's content). No new WriteState, no transition-table change, no new public write API (one new StorageError variant). Scoped re-review independently traced both guards against the code: confirmed exhaustive blocked-state set, confirmed JournalCommitted deliberately not blocked (preserves I1/7989cf6 git-pending flow), confirmed no regression to 88b20f6/C1/I1/M2, confirmed a deliberately-removed test was justified (it pinned the old incorrect behavior). Two Minors found and fixed: a misleading guard comment, and evidence.md missing this fix's entry (now items 29-30). One accepted trade-off documented: a note with any stalled op becomes temporarily unwritable until the next Vault::open (read_note already failed closed in that window; only incorrect write-acceptance was removed). 220/220 tests, clippy clean, cargo deny clean. No merge performed; presenting to user again per their explicit request to re-request merge authorization.
