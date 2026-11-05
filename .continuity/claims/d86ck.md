---
schema: 1
id: d86ck
type: decision
title: >-
  B2 Task 7 implementation complete (not yet committed): typed non-lineage edges
  (Calls/Imports/Implements/Extends/TestsFor), 0 missed mutants across 111
  mutants in 5 files; independent review in progress
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-05T12:41:38.251Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Resolvers re-parse each SymbolSnapshot's own signature_text (includes body per Task 4 extraction) with the matching LanguageParser and walk for call/import/inheritance syntax. Calls: ambiguous target -> one heuristic edge per candidate (not zero), since a call genuinely can invoke one of several overloads. Imports/Extends/Implements: ambiguous target -> no edge (names exactly one thing). TestsFor: naming convention alone never sufficient, requires an actual call too, always NameHeuristic/Speculative. C#/C++ (no syntactic class/interface distinction) resolve Implements vs Extends via target's own already-known SymbolKind. Known documented gap: Rust's #[test] attribute is not usable since Task 4's extraction boundary starts signature_text at `fn`, not the preceding attribute. Known documented gap: calls inside Rust macro invocations (assert_eq!) are invisible since tree-sitter treats macro args as opaque token_tree. Awaiting independent review before commit.
