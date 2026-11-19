---
schema: 1
id: n11xv
type: next_action
title: >-
  M1 (MCP server composition-root launcher) CLOSED, committed 032d09e, not
  pushed — resume remediation at M2
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-19T11:22:15.359Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

crates/mcp-server/examples/serve_stdio.rs + tests/launcher.rs (6 real-child-process acceptance tests) + tokio io-std feature flag. Closes B5 closure doc open item #2 and audit risk R1 (rk5sa's sibling finding — MCP_MAX_PAYLOAD_BYTES/concurrency semaphore now enforced by a real binary, not just library code). Full workspace suite (50 binaries) and workspace clippy pass with no regressions; pre-existing codegraph-index lint and rustfmt drift (open items #9/#10) deliberately untouched. R2 (client-side 16 MiB external-server cap) and R3 (rk5sa, child-process env inheritance) are NOT touched by M1 — still open. No push authorized yet. M2 not started.
