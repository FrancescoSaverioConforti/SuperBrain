---
schema: 1
id: d47vv
type: decision
title: >-
  B1 scope locked to deterministic search only; tree-sitter/code-graph deferred
  to B2
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-10-30T12:57:12.140Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User confirmed (via AskUserQuestion) that B1 = single plan, ordered tasks, scoped to spec's "Ricerca deterministica" (trigram+BM25, search/retrieve, C4 time filters) — tree-sitter parsing, code graph, graph_query/get_code_context explicitly deferred to B2 per spec's own "Ordine di implementazione" section separating B1 from B2. Plan written to docs/superpowers/plans/2026-10-30-b1-deterministic-search.md. Independent review dispatched (agent a24212b41b57b4608), verdict pending.
