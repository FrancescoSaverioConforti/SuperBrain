---
schema: 1
id: d88ac
type: decision
title: >-
  Task 7 (typed non-lineage edges) committed at e766c8e on b2-code-graph —
  superseding tentative d86ck
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T13:12:17.459Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Implemented Calls/Imports/Implements/Extends/TestsFor resolvers in crates/codegraph-index/src/edges/ (resolution.rs, calls.rs, imports.rs, extends_implements.rs, tests_for.rs). Edge { from, to, kind: EdgeKind, evidence: Evidence }. Rule: heuristic never indistinguishable from resolved — resolved_evidence() is the only StaticAnalysis constructor, used only on syntactic certainty; everything else is NameHeuristic+Confidence or no edge. Per-language: Implements/Extends resolve via syntax for Python/Rust/JS/TS/Tsx (base_names_in tags Some(EdgeKind)); only C#/C++ lack syntactic distinction and fall back to target SymbolKind lookup (tagged None). TestsFor requires naming convention AND an actual call, never naming alone. Known gaps documented in code: Rust macro-wrapped calls invisible to tree-sitter call_expression walk; Imports essentially never fires on real extracted data since signature_text never includes file-level imports outside a symbol's own span (only synthetic fixtures exercise it). Fixed one real review-found bug (TS/Tsx Implements/Extends was wrongly using C#/C++'s kind-lookup fallback instead of its own explicit syntax) before commit. 113 mutants on the 5 files: 101 caught, 12 unviable, 0 missed. Workspace tests/clippy/deny all clean. No merge/tag/push; SymbolKind baseline unchanged.
