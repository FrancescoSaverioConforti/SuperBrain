---
schema: 1
id: d36ky
type: decision
title: >-
  Proposed fix design for A3 restore/deletion TOCTOU: SQLite BEGIN IMMEDIATE on
  the deletion-log db file as the cross-process lock, held by restore from last
  is_deleted check through final promotion-rename, with a last-moment per-note
  recheck gating the rename itself (not following it); staged writes promoted
  per-note inside the lock so partial-crash restores stay individually
  consistent; seed_journal and Vault::read_note updated to respect the deletion
  log
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-10-30T08:17:44.626Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Presented to user for approval before implementation, per their explicit instruction. Scope assessed as circoscritta (contained within vault-security + storage, no schema/contract change) — pending user confirmation to proceed with TDD.
