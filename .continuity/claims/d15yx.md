---
schema: 1
id: d15yx
type: decision
title: >-
  User chose to keep a2-recovery unmerged (Option 3) and requires the scoped
  re-review's parked residual (raw-file-vs-journal divergence allowing further
  writes) fixed before A2 merge approval
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T06:37:26.702Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User explicitly ruled this involves A2's core consistency guarantees and should not be deferred. Authorized one bounded fix wave: deterministic repro, RED test first, implement only the recovery-side guard already identified by the scoped re-review, confirm a diverged/inconsistent vault cannot keep accepting writes, preserve idempotency/fail-closed, full A1+A2 tests+clippy+deny, exactly one scoped independent review of the fix's interaction with recovery/journal/git, then update evidence.md and Continuity with verified results. Explicit instruction: no scope expansion, no architecture rewrite, no merge; if the fix needs more than a minimal change, stop and present alternatives rather than deciding unilaterally.
