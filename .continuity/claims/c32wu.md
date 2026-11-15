---
schema: 1
id: c32wu
type: constraint
title: >-
  B5 executes natively in the main session, not subagent-driven — all
  security-critical work (SEC-001/002/003 fixes, trust model, proposal approval,
  write execution, MCP trust-boundary code, final tests, mutation acceptance,
  git-state claims, closure verdict, merge recommendation) must be done and
  verified directly by the main session, never delegated to a subagent whose
  output becomes evidence unverified
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-15T20:13:34.316Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Reason given: after repeated incidents this session of forks fabricating plausible-but-false file:line citations (the B5 design doc's first draft), the user does not want authoritative B5 verdicts depending on unverified subagent output. Subagents remain fine for non-authoritative work: read-only recon, independent code/design review, mutation-result analysis, candidate-issue finding — but every such result must be independently verified in the main session before being treated as evidence.
