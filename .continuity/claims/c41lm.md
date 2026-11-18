---
schema: 1
id: c41lm
type: constraint
title: >-
  Never fix the pre-existing codegraph-index manual_contains clippy lint or
  rustfmt-version drift incidentally while executing B5 plan tasks
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-18T19:24:15.541Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User directive (2026-11-18): these are known, pre-existing, unrelated issues (codegraph-index::agent_policy.rs clippy lint; rustfmt version drift on files predating each task). Always document them as technical debt separate from task regressions, never auto-fix them as a side effect of verifying a task's own code is clean.
