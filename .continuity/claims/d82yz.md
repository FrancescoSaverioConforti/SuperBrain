---
schema: 1
id: d82yz
type: decision
title: >-
  SymbolSnapshotId encoding rewritten at Task 5: length-prefixed fields +
  "superbrain-symbol-snapshot-v1" domain separator, replacing Task 1's naive
  unprefixed-concatenation scheme which had a real structural collision flaw
  (("ab","c") vs ("a","bc") hashed identically)
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-05T09:19:10.875Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Fixed directly in snapshot_id.rs rather than adding a parallel derive_v2 — justified because nothing had persisted a Task 1 id anywhere yet (no storage/API/stable format existed to migrate). Not committed yet, pending independent review of Task 5 (promotion boundary: SymbolSnapshot, promote, kind_hint->SymbolKind mapping, ValidatedName, BoundedSignature).
