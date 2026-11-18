---
schema: 1
id: c39sx
type: constraint
title: >-
  Shared policy/domain types with existing real call sites (e.g.
  policy::ReadPolicy::new) are extended additively — new builder methods or new
  optional fields — never by changing an existing constructor's arity, even for
  a capability-complete design
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-18T09:05:22.757Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Why: ReadPolicy::new(search, retrieve) has 16 real positional call sites across crates/api's own tests and example binaries; crates/api is frozen untouched by the B5.0 contract. Adding a third positional bool would have forced touching all 16 for a capability api itself has no concept of. The fix was ReadPolicy::with_get_code_context(bool) as an additive builder, defaulting to false/fail-closed for every existing new() caller.
