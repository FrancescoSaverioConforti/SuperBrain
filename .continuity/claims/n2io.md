---
schema: 1
id: n2io
type: next_action
title: >-
  B1/B2/C2 plans must re-audit the sensitive-classification confidentiality
  boundary against index, embedding, summary, attachment, and cache surfaces
  once those subsystems exist
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T08:21:18.678Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

A3's Task 3 (crates/storage/tests/a3_confidentiality_boundary.rs) explicitly defers 5 of C3's 10 listed surfaces (indici, embedding, riassunti, allegati, cache) because no such subsystem exists yet in the codebase. File content, metadata, log output, generated/temp files, and backup are covered by A3's stubs. This open item is the reminder to close that gap when B1/B2 (index) or C2 (summaries) land.
