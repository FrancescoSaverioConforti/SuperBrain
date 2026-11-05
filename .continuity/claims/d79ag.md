---
schema: 1
id: d79ag
type: decision
title: >-
  HTML/CSS symbol semantics for B2 Task 4 decided explicitly by user: HTML =
  elements with non-empty id attribute only (never class-only or anonymous); CSS
  = rule_set with referenceable selector (id/class/tag, including composite)
  plus named @keyframes; signature_text bounded to selector only, not
  declaration block; no cross-reference resolution (deferred to later edge task)
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T08:41:32.115Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User explicitly rejected "every tag/every rule as symbol" to avoid noise from anonymous elements. Implemented in crates/codegraph-index/src/extract/html.rs and css.rs, not yet committed, pending independent review of the full extract module (Task 4, 9 language extractors).
