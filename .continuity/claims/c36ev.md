---
schema: 1
id: c36ev
type: constraint
title: >-
  storage's A1 invariant is "propose_and_commit is the only function reaching a
  raw write primitive," not "propose.rs has exactly one pub fn" — new public
  write-shaped functions in propose.rs are allowed only as typed,
  resource-confined facades that themselves call propose_and_commit and never a
  raw primitive directly; each addition requires explicit allowlisting in the
  storage_exposes_exactly_one_raw_write_authority guard test, never
  renaming/moving to dodge the check
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-18T07:23:31.381Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Why: update_note_body/add_relation genuinely needed to be new pub fns in propose.rs (Task 14 prerequisite), which the old literal "exactly 1 pub fn" guard rejected even though neither bypasses the real gate. User directed fixing the guard's assertion to match the real invariant rather than hiding the new functions elsewhere to pass the old grep.
