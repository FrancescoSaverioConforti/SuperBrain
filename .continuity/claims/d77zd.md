---
schema: 1
id: d77zd
type: decision
title: >-
  B2 Task 3 implementation complete (not yet committed): tree-sitter abstraction
  for 9 language variants, no version conflicts found (every grammar crate's
  only runtime dep is tree-sitter-language ^0.1, tree-sitter itself is each
  grammar's dev-dependency only), 0 missed mutants; independent review in
  progress
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-05T07:44:51.567Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Versions: tree-sitter 0.27.1, tree-sitter-language 0.1.9 (shared ABI, exactly one resolved version across whole graph, confirmed via cargo tree -i). Grammars: tree-sitter-rust 0.24.2, -python 0.25.0, -c-sharp 0.23.5, -cpp 0.23.4, -javascript 0.25.0, -typescript 0.23.2 (exposes both LANGUAGE_TYPESCRIPT and LANGUAGE_TSX), -html 0.23.2, -css 0.25.0. No stop-and-report needed — no incompatible major version found. Awaiting independent review before commit.
