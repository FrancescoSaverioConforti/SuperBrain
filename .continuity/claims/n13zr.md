---
schema: 1
id: n13zr
type: next_action
title: >-
  M5 Task 1 (pure audit hash-chain primitives) CLOSED, committed e42a315, not
  pushed — resume at M5 Task 2 (filesystem writer)
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-19T16:23:24.465Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

crates/brain-api/src/audit_chain.rs: compute_record_hash/verify_record_hash/verify_link/scan_chain, reusing deletion_log.rs's own chain-hash formula and genesis convention. 21 tests passing, full workspace suite clean. Not yet wired to anything (module has #![allow(dead_code)], explicitly not called from anywhere in the crate). Three design corrections resolved before implementation: (1) record_durable must be async fn with a single tokio::sync::Mutex held across the whole spawn_blocking().await, never a sync fn trying to await — verified by direct execution with 50 concurrent callers; (2) a partial trailing record needs an explicit recovery procedure (preserve original file under a new name, append a recovery-marker record) not silent resumption; (3) approval-attempt identity needs a dedicated attempt_id field distinct from proposal_id/correlation_id. Remaining M5 tasks (filesystem writer+locking, DPAPI head reference, AuditSink/DurableAuditWriter adapters, CommitOutcome/ApprovalAttempted schema, composition-root wiring) are separate, each needing its own authorization — not started.
