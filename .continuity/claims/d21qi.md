---
schema: 1
id: d21qi
type: decision
title: >-
  A3 plan second draft passed independent review on all 7 user-required
  corrections, verified against actual gate.rs/journal.rs/fault.rs code; two
  minor gaps (C3 indici/embedding/riassunti scope note,
  pending-deletion-during-backup test) found and fixed inline — plan awaiting
  user's go-ahead to implement
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T07:42:50.089Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

docs/superpowers/plans/2026-10-28-a3-security-durability.md. Reviewer confirmed: gate stays unconditionally protective (conditional only on store availability), single crash-recoverable transaction for sensitive writes (create+update), recovery kit wraps the live master key with no DPAPI in its own path, DeletionLog reference is DPAPI-authenticated, Task 0 is a drift oracle not a vulnerability-preservation test, no second public write/delete entry point (deletion is a propose_and_commit tombstone), dependency direction domain->policy/storage->vault-security confirmed acyclic. Supersedes [d20os]'s "awaiting approval" state with "awaiting approval, review passed."
