---
schema: 1
id: c13tk
type: constraint
title: >-
  C2's fix made every relation target
  (supports/contradicts/derived_from/supersedes/applies_to, including
  pending_ids) require valid NoteId syntax (lowercase-ASCII, no separators) —
  this narrows Task 3's original contract, which allowed pending_ids to be
  external-source or code-symbol references that could contain characters NoteId
  now rejects
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T17:37:39.800Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Nothing in A1 produces such references today so nothing breaks now, but Phase B (symbol links, external sources) may need relation-target validation decoupled from NoteId's filesystem-identity constraints. Surfaced by the fix-wave's scoped re-review (2026-10-26); needs an explicit decision before Phase B, not a silent default.
