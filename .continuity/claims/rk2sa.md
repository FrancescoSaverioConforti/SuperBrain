---
schema: 1
id: rk2sa
type: risk
title: 'B3 latency/real-use acceptance criteria blocked: no MCP server exists'
status: open
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-14T07:41:27.585Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Fase B acceptance criteria require 'Latenza da VS Code' (P50/P95) and 'Uso reale' (2 weeks MCP in VS Code), but no MCP server exists in real code — only search.rs/retrieve.rs in crates/api; no graph_query/get_code_context, no MCP module. This is a B1/B2 delivery gap inherited by B3, not introduced by it. B3 cannot close on these two criteria without at least a minimal read-only MCP server for search/retrieve.
