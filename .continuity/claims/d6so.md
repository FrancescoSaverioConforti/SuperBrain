---
schema: 1
id: d6so
type: decision
title: >-
  Claude Session Supervisor project is cancelled/deferred indefinitely; A2
  starts directly using manual Claude Code session rotation at checkpoints
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-27T07:13:29.195Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User reversed the prior resume-point directive (external Supervisor before A2). Continuity already handles context persistence/recovery across sessions, so no separate orchestration layer is needed. No new scripts, hooks, or orchestration systems are to be built for session management. A2 planning begins now, following A1's TDD + independent-review workflow.
