---
schema: 1
id: d22gr
type: decision
title: >-
  A3 implementation authorized via superpowers:subagent-driven-development, same
  workflow as A1/A2 — dedicated branch+worktree, per-task independent review,
  evidence in eval/results/A3/, no merge/push to main without explicit approval,
  SEC-001/002/003 kept at actual status (not treated as fixed), stop and present
  alternatives on substantial architectural blockers or new critical
  vulnerabilities
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T07:50:19.831Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User's full procedure: TDD red/green/refactor per task; review each task before proceeding; test+clippy+cargo-deny at checkpoints; Continuity updated at significant checkpoints only (no redundant records); final verification adds mutation testing on critical components, C3/C5 acceptance criteria check, full independent branch review, and a conclusive report (commits, tests, requirement coverage, residual vulnerabilities, deferred decisions). Constraints: no spec edits without authorization, no B/C/D scope creep, no automatic branch/worktree deletion.
