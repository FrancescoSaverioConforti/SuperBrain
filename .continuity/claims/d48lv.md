---
schema: 1
id: d48lv
type: decision
title: >-
  B1 plan independently reviewed: GO-WITH-CHANGES, 3 fixes applied, ready for
  user authorization
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-10-30T12:59:45.049Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

docs/superpowers/plans/2026-10-30-b1-deterministic-search.md reviewed by a fresh subagent (verified against real code: Indexer trait at vault.rs:413-422 is pub(crate), NullIndexer at 428, mark_index_current at 1667 called under the per-note lock — deadlock reasoning for a same-note read_note callback from mark_current confirmed sound; Journal pub(crate) at journal.rs:149; propose_and_commit sole public write confirmed; sensitive-note pointer-only body confirmed universal via propose.rs, not just A3's deletion-specific citation). Verdict GO-WITH-CHANGES, no invalidating error in core architecture. Fixed in the plan: (1) Task 0 reframed as a documentation anchor, not a behavioral drift oracle like A3's (index has no SEC-00x fixtures to assert on); (2) sensitive-note pointer-only claim now cites propose.rs's Sensitive-branch doc comments instead of A3 §5's deletion-specific line; (3) Task 3 gained an explicit test pinning BM25 zero-floor scoring (no smoothing) as an implementation constraint, since Task 7's citation-correctness test depends on it and not all BM25 variants guarantee it. Plan awaiting user's go-ahead to implement — no code written, no merge/tag/push, no spec changes.
