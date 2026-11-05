---
schema: 1
id: d71fm
type: decision
title: >-
  B2 Code Graph design plan complete: 2 new crates (codegraph-domain,
  codegraph-index), 15 tasks, GO to start Task 0 through Task 8 (unblocked);
  Task 9 disk/Task 10/Task 11-12 api-wiring/Task 14 remain NO-GO pending gates
  S1/S2/S3
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T06:36:58.281Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Plan: docs/superpowers/plans/2026-11-05-b2-code-graph.md. D2 decided: single authoritative writer, same process/Vault-handle as rest of SuperBrain, no separate watcher process. D4: B2 explicitly accepts DL-04 rollback risk unchanged, does not attempt to fix it. Two items flagged needing separate authorization (not yet decided): who supplies the repository allowlist for Task 2, and Task 10's on-disk persistence format (deliberately left TBD pending S1/S2 fix shape).
