---
schema: 1
id: d30cn
type: decision
title: >-
  A3 final whole-branch review found a Critical bug (once any sensitive note is
  deleted, backup/restore breaks permanently for the whole vault) plus 5
  Important findings — user chose to dispatch one bundled fix wave + one scoped
  re-review, per SDD's final-review rule, no further open-ended rounds
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-29T09:08:44.885Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Supersedes [d29wp]. Independent review (opus, with own repro tests) confirmed C1 by reading backup.rs directly: restore's deleted-note check rejects the whole restore if ANY note id in the snapshot is_deleted()==true, never distinguishing "this backup already contains the tombstone (safe)" from "this backup predates the deletion (unsafe)". Also found: PiiStore::get never consults is_deleted (deleted content stays readable indefinitely on certain failure paths; a deleted id can be resurrected by writing new content); DeletionLog::reconcile is never called in production, only in tests; restored vaults produce byte-identical files but can't be read through Vault::read_note since the journal isn't included in backups; mutation-testing triage in evidence.md mislabeled 4 real, cheap-to-close gaps (backup HKDF subkey and nonce, deletion-log chain-hash, PiiStore fingerprint's .pii-extension guard) and the hash chain is never verified end-to-end. User approved one bundled fix wave covering all of these, followed by exactly one scoped re-review — no further rounds; any residual findings surface directly to the user.
