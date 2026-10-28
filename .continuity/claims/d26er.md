---
schema: 1
id: d26er
type: decision
title: >-
  A3 Task 6's Critical PII before-image data-loss bug is genuinely closed after
  3 fix rounds — independent re-review wrote its own repro suite, reproduced
  every prior failure on the base commit, confirmed all fixed on head, and
  explicitly stated the mechanism is sound with no 4th round needed
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T17:17:43.082Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Supersedes [d25r9]. Round 3 rewrote roll_back_pii around an explicit LivePii enum (Ours/Other(hash)/Absent/Undecryptable) with a total decision table — every destructive action gated on positive verification, no error path defaults to delete. All 5 reported paths fixed plus 7 more self-audited instances of the same fail-open pattern. Task 6 (PiiStore + single crash-recoverable sensitive-write pipeline, commits bb4a0a7..695bf2c) is complete. Two Minor items deferred to final whole-branch review. One new non-blocking follow-up logged: a note reclassified sensitive->plain->sensitive can permanently block Vault::open after an ordinary crash (no data loss, needs an operator) — pre-existing, not introduced by this fix, worth a dedicated task later.
