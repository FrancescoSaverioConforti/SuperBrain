---
schema: 1
id: x1ks
type: rejected_alternative
title: >-
  A3 draft 1 design: gate unconditionally rejects Sensitive forever,
  PiiStore+Markdown pointer as two separately-recovered transactions,
  unauthenticated deletion-log reference file, Task 0 test requiring SEC-00x to
  stay vulnerable, and a new propose_delete public entry point
status: rejected
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T07:41:31.315Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
reason: >-
  User rejected on 7 grounds: (1) gate must stay unconditionally protective,
  with Sensitive rejection made conditional on an explicit store-availability
  check rather than silently bypassed; Secret stays untouched. (2) needed an
  explicit enumerated confidentiality-boundary audit (git history, A2 journal,
  before-images, logs, temp files, backups), not an implicit round-trip-only
  claim. (3) a sensitive write must be ONE crash-recoverable transaction inside
  the existing A2 journal/WriteState machine, covering updates to
  already-sensitive notes too, not two independently-recovered stores. (4)
  recovery kit must provably wrap the SAME live master key, restore
  cross-machine without DPAPI, and load_or_create must never mint a fresh key
  over an existing undecryptable store. (5) DeletionLog's trusted reference must
  be DPAPI-authenticated, not a bare file, and must handle
  deletion-after-backup. (6) Task 0 must never require known vulnerabilities
  (SEC-001/002/003) to persist — reframed as a drift oracle over their current
  documented status, not a preservation requirement. (7) no second public
  write/delete entry point — C5 requires a trustworthy deletion LOG, not a
  specific API; deletion of a sensitive note is a tombstone write through the
  existing single propose_and_commit gate. See
  docs/superpowers/plans/2026-10-28-a3-security-durability.md (second draft) for
  the corrected design.
---


