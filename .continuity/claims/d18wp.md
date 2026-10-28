---
schema: 1
id: d18wp
type: decision
title: >-
  A2 (Recovery) merged to main as ffeb9ab (--no-ff from a2-recovery @ f807c34),
  tagged a2-complete, post-merge verification: 220/220 tests, clippy clean,
  cargo deny clean
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T07:08:49.942Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Merge commit ffeb9ab, tag a2-complete. 29 commits from a2-recovery (base c465e8d) merged with full history preserved, no squash/rebase/force. Branch a2-recovery and its worktree deliberately NOT deleted per user instruction. NOT pushed to origin/main per user instruction -- push requires separate authorization after this report. A3 not started per user instruction.
SEC-001/002/003 actual current status (eval/results/A2/sec-debt-reevaluation.md), none closed: SEC-001 verdict OPEN UNCHANGED; SEC-002 verdict NARROWED (read path only, write-path pre-read still open); SEC-003 verdict NARROWED (read path only, write-path pre-read and outward hard-link exposure still open). All three still require re-evaluation again before Fase D per [d5b9].
Full history: 10 plan tasks (independent review each) -> final whole-branch review found C1/I1/I2 (one Critical, two Important) -> one fix wave + one scoped re-review (verified, 199->209 tests) -> user authorized closing the re-review's own parked residual -> two further targeted fixes (88b20f6, e5e2506, comment fix f807c34) each via TDD + one scoped independent review -> 209->220 tests final. Evidence in eval/results/A2/{evidence.md,dependency-exceptions.md,sec-debt-reevaluation.md}, all present and current at merge time.
