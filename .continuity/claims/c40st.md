---
schema: 1
id: c40st
type: constraint
title: >-
  graph_query's depth/result bounds (MAX_GRAPH_QUERY_DEPTH=50,
  MAX_GRAPH_QUERY_RESULTS=500 in crates/brain-api/src/read.rs) are provisional,
  pending plan Task 35's numeric pass over every capability at once
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-18T11:37:04.553Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

CodeGraph::reachable_from itself has no caller-facing bound (cycle-safe for any depth via its own visited-set). The brain-api-level constants exist only to keep one public query's cost/size predictable and may be revised, not re-derived from scratch, at Task 35.
