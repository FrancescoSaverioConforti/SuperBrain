---
schema: 1
id: d42zg
type: decision
title: >-
  Targeted review of F1/F2 fix wave (commit e2f8922) returned GO for merge —
  both fixes verified structural, not timing-dependent, via repeated runs (5x
  each, no flakes) and the implementer's own repro scenarios reconstructed
  independently. One new Minor finding: "no PiiStore" exception in
  note_is_deleted/refuse_read_if_deleted depends on which constructor the caller
  picks at open time, not a fixed vault configuration — a deleted non-sensitive
  note stays readable if reopened via open_with_default_indexer after being
  written/deleted while open_with_pii_store was in use elsewhere. Not
  exploitable remotely; requires picking the wrong constructor or local file
  access (which already exposes plaintext .record).
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-30T12:15:59.930Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

F3 residual confirmed real but confirmed non-exploitable (forwarding an exempted empty-plaintext row never leaks content; read_note still withholds it as Deleted). F2 introduces a new but acceptable availability side effect: Vault::open_with_pii_store can now wait up to 30s for the write lock. Accepted open debt restated: F4 PiiStore::get/get_before_image still pub (doc-comment only), deleted note's .record/.pii bytes persist on disk in restored vaults with only read gates withholding them, F3's schema-change-requiring residual. Awaiting user's final merge decision.
