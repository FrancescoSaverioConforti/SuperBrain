---
schema: 1
id: c35ax
type: constraint
title: >-
  storage's public API surface must stay resource-confined — no public method
  accepting an arbitrary relative path/filename, even for a "safe" subdirectory
  like .proposals/. Each new persisted resource type gets its own named methods
  (e.g. save_proposal/load_proposal/list_proposal_ids) taking a validated id,
  never a path, with storage validating that id itself rather than trusting the
  caller
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-16T10:52:19.347Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Confirmed in B5 Task 12: a draft public Vault::write_relative_verified_atomic(relative_dir: &Path, ...) was caught by the existing A1 regression guard test storage_exposes_exactly_one_public_write_function — a path-generic public write/read primitive is itself a second write surface that guard exists to prevent, regardless of which subdirectory it was scoped to today. User redirected to the resource-confined shape now in place. Apply this same discipline to any future storage-crate public API (e.g. anything B6 UI or MCP server tasks might need added to Vault).
