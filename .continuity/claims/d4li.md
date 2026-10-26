---
schema: 1
id: d4li
type: decision
title: >-
  User authorized fixing I-C (symlink-escape check on wrong path) and the
  pending-link-state-reporting Minor before A1 merge, with explicit test
  requirements
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-10-26T18:07:52.894Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

I-C fix must check symlink_metadata on the actual .record path used by I/O, not a substitute path. Required tests: symlink planted at the real root/note-a.record pointing outside vault, confirming read_note/version-check/propose_and_commit all reject it; parent path component checks; dangling/invalid symlink cases. Must NOT claim TOCTOU is eliminated -- symlink_metadata is not a guarantee against concurrent path substitution; keep that limitation documented (already ledgered as [c12ah]). evidence.md must only claim what tests actually support. Pending-link Minor: propose_and_commit's return must surface to the caller that a link was classified Pending (per spec C6's "accettato e segnalato"), not silently drop link_state.
