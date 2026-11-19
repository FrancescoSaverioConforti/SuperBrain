---
schema: 1
id: rk6l3
type: risk
title: >-
  R4 — Note-to-proposal provenance link exists but is unindexed/manual, not
  MISSING
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-19T11:05:51.723Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Corrected from the audit's initial characterization. Approved proposals are NEVER deleted from .proposals/ (no remove_proposal/delete_proposal/archive function exists anywhere in brain-api) — each retains operation.{SaveMemory,UpdateMemory}.note_id, originating_identity, provenance.source, created_at, and final ProposalStatus::Approved{approved_at}. So reconstructing which proposal/agent produced a given Note IS possible today without touching the canonical note format: list_proposals (crates/brain-api/src/approval.rs:274, proposal_store.rs:320) + filter by matching note_id + Approved status. The real gap is that this reconstruction is O(n) manual/unindexed — no direct Note-id -> proposal-id lookup, no field on the committed Note itself. Severity: Low (was Low-Medium), reclassified PARTIAL not MISSING. Surfaced in the 2026-11-19 architectural audit.
