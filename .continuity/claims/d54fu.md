---
schema: 1
id: d54fu
type: decision
title: >-
  B1 Task 4 scoped to existing surfaces only; SearchResponse audit deferred to
  Task 6
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-10-30T16:59:54.915Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

B1 plan's Task 4 brief named SearchResponse (Task 6's not-yet-built output type) as a surface to audit for source/relations leakage — a forward dependency the pre-flight scan missed. Asked user directly: Task 4's confidentiality audit covers only surfaces that exist now (in-memory TrigramIndex/Bm25Index postings, log/error output from Task 3's fault-injection path). [n2io]'s index-surface closure is correspondingly PARTIAL after Task 4: postings/log surfaces close, but the SearchResponse/search-API-response surface stays open until Task 6 adds its own source/relations-leak test. Task 6's dispatch brief must carry this forward explicitly as an added requirement, not just inherit it implicitly from the plan text.
