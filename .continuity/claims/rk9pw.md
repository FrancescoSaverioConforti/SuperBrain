---
schema: 1
id: rk9pw
type: risk
title: >-
  M4 (MCP server concurrency queue cap) verdict: DEFER WITH ACCEPTED RISK — not
  implemented
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-19T11:43:55.340Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Read-only analysis (2026-11-19) confirmed: rmcp's own dispatch loop spawns an unbounded task per incoming tools/call with zero backpressure upstream of SuperBrainMcpServer's own Arc<Semaphore> (MAX_CONCURRENT_REQUESTS=16, no acquisition timeout). Queue depth is structurally unbounded, but realistic risk is narrow given the stdio-only/local-single-user threat model and small ordinary query sizes (MAX_QUERY_LEN=4096; only large save_memory/update_memory bodies approach M2's 1 MiB cap). Verdict: keep strategy A (current unbounded queue) — the original "queue, don't reject" decision stands, not overridden. Both capped alternatives (reject-when-full, reject-immediately) risk breaking legitimate bursts and require inventing new client-facing error semantics for no threat-model-justified gain today.
