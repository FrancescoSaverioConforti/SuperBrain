---
schema: 1
id: d63og
type: decision
title: >-
  P0 mutation testing surfaced 42 missed mutants, several on security-critical
  DL-02/DL-03 enforcement points
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-04T11:35:03.075Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Ran cargo mutants --package vault-security --file crates/vault-security/src/deletion_log.rs (177 total mutants): 107 caught, 42 missed, 1 timeout, 27 unviable. Full list in mutants.out/missed.txt. Most serious: is_deleted_verified_locked (line 1162) — the exact final-recheck gate restore.rs:522-523 calls under the write lock immediately before promoting a restored note to readable (per the P0 design's whole point) — can be mutated to unconditionally return Ok(true) OR Ok(false) with no test failing. Also missed on the same critical path: verify_existing_or_pending's genesis match guard (861), verify_consistent's genesis/generation/chain-hash guards (1241/1243), is_genesis_reference's whole boolean body (880-883 — can return `true` unconditionally), verify_file_identities (1223), assert_reference_bound (1267), acquire_existing_write_lock's NotFound arm and timeout comparison (1284/1291), verify_chain_on's comparison operator (1340), reconcile_locked's generation comparison (1488). Did not attempt fixes yet — surfacing this to the user first per project standing rule (any new Critical/Important finding stops for presentation, not silent fix), and because 42 survivors need triage (real test gap vs. equivalent mutant vs. already covered by a workspace-level test cargo-mutants' file-scoped run didn't exercise) before deciding what to add.
