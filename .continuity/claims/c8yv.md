---
schema: 1
id: c8yv
type: constraint
title: >-
  Known non-blocking limitations in A1, accepted as-is: (1) atomic_write's
  directory-fsync step is Unix-only — Windows relies solely on rename's own
  durability, no directory fsync; (2) storage's API-surface test (one public
  write function) is a textual grep check, not semantic reflection —
  belt-and-suspenders on top of the real guarantee (write_record_unchecked's
  actual Rust privacy)
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T13:49:13.721Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Surfaced by independent plan review (2026-10-26). Accepted for A1 since the project runs on Windows and the spec itself says durability ordering is tested per-platform; revisit if A2's journal work surfaces a real durability bug. Documented in the plan and must stay documented in A1 evidence.
