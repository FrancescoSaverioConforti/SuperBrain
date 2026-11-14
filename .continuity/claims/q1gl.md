---
schema: 1
id: q1gl
type: question
title: 'B3.0 scope decision: text-only retrieval or graph+text integration'
status: superseded
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-14T07:41:27.608Z'
supersedes: []
superseded_by: d192dq
depends_on: []
tags: []
superseded_reason: User resolved the open B3.0 scope question
---

Does B3 evaluate only B1's deterministic text retrieval (trigram+BM25+citations+temporal filters), as Spec_mvp.md literally states, or does it also include graph retrieval? The latter requires Task 9/10/11/12 from the B2 plan (graph persistence/integration, graph_query/get_code_context, resource budgeting Gate S3) — none authorized yet, and Task 10 is explicitly not authorized. Code graph (b2-code-graph worktree) is not merged to main; no integration point exists today between codegraph-index and index/api.
