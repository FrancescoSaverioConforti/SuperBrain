---
schema: 1
id: rk10ph
type: risk
title: >-
  New finding (not M4, not yet actioned): queued MCP requests never observe
  cancellation — RequestContext::ct is read nowhere in mcp-server
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-19T11:43:55.837Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Confirmed during M4's analysis: a client disconnect or explicit notifications/cancelled correctly fires rmcp's own per-request CancellationToken (service.rs ct.cancel()), but mcp-server's 12 tool handlers never check context.ct anywhere (zero matches). A request already queued on acquire_concurrency_permit().await keeps waiting indefinitely regardless of disconnect/cancellation, then on completion tries to send into a possibly-already-closed channel. This is orthogonal to M4's queue-depth-cap question — a correctness/responsiveness gap, not a capacity policy choice. Small, low-regression-risk fix available (select! acquire_concurrency_permit against context.ct.cancelled()) but NOT implemented or approved — flagged to the user, awaiting a decision on whether to track it as its own roadmap item.
