---
schema: 1
id: d87ix
type: decision
title: >-
  Task 7 fix: TS/Tsx Implements vs Extends now resolved by syntax
  (extends_clause/implements_clause tag), not target SymbolKind lookup
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T13:01:14.935Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Independent review found extends_implements.rs grouped TypeScript/Tsx with C#/C++ in edge_kind_for, deciding Implements vs Extends via target_kind == Interface — contradicting the stated design (TS/Tsx syntax is as explicit as Rust's `for` clause). Fixed by changing base_names_in's return type to Vec<(String, Option<EdgeKind>)>: Python/Rust/JS/TS/Tsx tag Some(kind) from syntax directly (TS/Tsx's class_heritage children are extends_clause/implements_clause wrapper nodes, confirmed via parse dump); only C#/C++ tag None and fall back to kind lookup. Added 3 regression tests proving syntax wins even when target kind would imply the opposite kind. Also rewrote imports.rs's module doc comment, which falsely claimed file-level import reconstruction — it actually re-parses each snapshot's own signature_text like calls.rs, so Imports edges essentially never fire on real extracted data (only synthetic fixtures that inline import text into signature_text); documented as a known gap, not a bug to fix now.
