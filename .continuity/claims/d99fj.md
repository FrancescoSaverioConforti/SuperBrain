---
schema: 1
id: d99fj
type: decision
title: >-
  Task 9.5 Priority 5(A) Case B (unqualified common-name call ambiguity) uses
  precision-first scoped resolution: narrow by same-class/scope then same-file,
  1 candidate at a level emits a heuristic edge, >1 indistinguishable candidates
  at the narrowest non-empty level emits NO edge (never N speculative edges,
  never an arbitrary pick)
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-06T10:07:37.935Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

This is CALL-RES-A1+A2 (ambiguity-suppression + scope-aware), explicitly an experimental configuration per the new Experimental Policy, not a proven-final answer — must eventually be compared against the permissive CALL-RES-A0 baseline and precision-first-without-scoping (A1 alone) once sufficient ground truth exists. No magic candidate-count thresholds; narrowing derives only from structural scope (span-containment-derived class membership, then file identity). "Unresolved: no candidate" and "unresolved: ambiguous" must be recorded as distinct outcomes, never merged.
