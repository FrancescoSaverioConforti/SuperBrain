---
schema: 1
id: rk8fd
type: risk
title: >-
  R6 — double-approval safety is emergent (storage's per-note mutex), not an
  explicit proposal-store invariant
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-19T11:05:52.984Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

crates/brain-api/src/proposal_store.rs save_proposal/load_proposal have no locking/CAS of their own (plain read-then-write). The real concurrent-double-approve test (approval.rs:831-865, two real OS threads + Barrier, asserts exactly one Ok + one Conflict) passes because storage::Vault's pre-existing per-note Mutex (vault.rs ~1004-1360) is held across propose_and_commit/update_note_body's version-check-and-write critical section inside execute() — the losing approve_proposal call fails storage's own expected_version check BEFORE it ever reaches transition_to/save_proposal. Safety today is emergent from "every ProposedOperation variant's execute() unconditionally hits a version-checked note write," not an explicit proposal-level invariant. A future proposal type that doesn't route through that path would need its own guard. Severity: Low, technical debt not a live bug. Surfaced in the 2026-11-19 architectural audit.
