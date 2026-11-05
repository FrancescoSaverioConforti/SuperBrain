---
schema: 1
id: d72ru
type: decision
title: >-
  B2 Task 0 complete: RepositorySource seam pinned at commit b1cf436 on branch
  b2-code-graph (worktree .worktrees/b2-code-graph, base 3ae43cb on main);
  awaiting GO for Task 1
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T06:44:57.506Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

codegraph-domain (RepositorySource trait + SourceError, deps: thiserror only) and codegraph-index (deps: codegraph-domain only) scaffolded. Used raw &str identifiers in the trait, not Task 1's validated types yet — deliberate, mirrors storage::Indexer::mark_current precedent, flagged as deviation from plan's literal sketch. All tests/clippy/deny green.
