---
schema: 1
id: d37zw
type: decision
title: >-
  A3 TOCTOU fix revised design (round 2): separate OS-level file lock (not
  SQLite BEGIN IMMEDIATE) around record_deletion's existing 3-step sequence and
  restore's final span, to avoid delaying Pending-row durability per C5;
  publication gated by reusing A2's journal WriteState machinery
  (ContentPersisted -> recheck -> JournalCommitted) instead of a new marker type
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-10-30T08:20:50.985Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Open question posed back to user: this likely requires vault_security::restore to change from "writes directly to disk" to "stages, lets storage promote via journal" — a cross-crate interface change between vault-security and storage, not a C5/schema redesign. Awaiting user's choice: accept that interface change, or keep restore's direct-write contract and build equivalent staging inside vault-security itself. Also awaiting confirmation of a caveat: whether Vault's live (non-restore) delete path holds any Vault-internal lock while calling record_deletion, which affects the lock-ordering proof.
