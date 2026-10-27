---
schema: 1
id: d7pr
type: decision
title: >-
  A2 (Recovery) technical plan written and awaiting user approval —
  docs/superpowers/plans/2026-10-27-a2-recovery.md, no code implemented yet
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-27T07:26:18.025Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

10 tasks: (1) tighten WriteState::Recovering to C1's exact table + fault-injection seam, (2) SQLite journal enforcing state transitions, (3) before-image persistence/hashing, (4) wire journal+before-image into write path, journal-resolved reads, (5) idempotent git2 commit per operation_id + git_pending as non-error CommitResult, (6) IndexState stub + NullIndexer completing chain to IndexCurrent (real indexing stays B1), (7) Vault::open startup reconciliation against C1's 5 fault-matrix rows, (8) crash-injection tests for all 8 matrix rows + retry idempotency + concurrent-read-during-write, (9) adversarial SEC-001/002/003 re-evaluation (expected verdict: all OPEN/UNCHANGED — hash verification is after-the-fact, does not prevent the races), (10) acceptance evidence in eval/results/A2/. User chose git2 (not CLI shelling), IndexState as bare stub (not real indexing), and explicit rigor on SEC debts: hash checks must not be treated as proof of closure, each debt stays open unless adversarially tested and demonstrably eliminated.
