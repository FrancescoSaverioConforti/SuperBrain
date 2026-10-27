---
schema: 1
id: c10ck
type: constraint
title: >-
  Whole-branch final review must explicitly re-check every crate's top-level pub
  exports (lib.rs re-exports), not just the diffs of individual tasks —
  task-level reviews missed storage::atomic_write staying publicly re-exported
  after Task 9 added propose_and_commit
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T17:13:24.361Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Caught only by the final whole-branch review (opus), reproduced from an external probe crate: storage::atomic_write(path, bytes) bypasses NoteId validation, the gate, and version checking entirely, defeating A1's central "no public write path other than propose_and_commit" guarantee. Process lesson for all future subagent-driven-development execution on this project.
