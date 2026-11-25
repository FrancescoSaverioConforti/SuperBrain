---
schema: 1
id: c44wf
type: constraint
title: >-
  M7 sequencing: only M7.2 mapping design + M7.3 idempotency design + bridge
  implementation/acceptance now; do not implement M7.5/M7.6 retention/rotation
  in the same pass without a new checkpoint on the segmented format
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-25T13:18:21.541Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Subagents may do recon/review but final PASS verification of acceptance criteria must be run by the main session, not delegated. STOP after M7.2/M7.3 design if a non-additive change to already-persisted Brain audit format emerges.
