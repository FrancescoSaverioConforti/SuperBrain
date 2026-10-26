---
schema: 1
id: c11mp
type: constraint
title: >-
  Every SDD task dispatch for this project must explicitly instruct the
  implementer to run `cargo clippy --workspace -- -D warnings` before reporting
  DONE
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T14:26:25.042Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Task 4's implementer never ran clippy and it slipped through to the independent reviewer, costing a full fix round. Applying this instruction to every subsequent task dispatch in A1 (and future phases) prevents the same gap recurring.
