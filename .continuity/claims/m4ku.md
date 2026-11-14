---
schema: 1
id: m4ku
type: milestone
title: >-
  B3.1/B3.2 complete: corpus + ground truth built, reviewed, fixed, GO for
  B3.3/B3.4
status: open
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-14T08:55:07.133Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Branch b3-retrieval-eval (10 commits off main @7b188da): 53-note frozen corpus, 120 queries (100 answerable + 20 abstention) split 60/30/30+1 canary, extended label schema, validate_corpus + build_manifest tooling. Independent review (opus) found 2 Critical (per-note-templated answers/citations didn't match ~25/100 questions; 3 mistargeted relevance labels) + 7 Important + 9 Minor. Critical and Important fixes applied and verified (commit 5a2bd2a): per-query answer/citation authoring, validate_corpus gained span-correctness checks, SuperBrain code-note commits pinned, build_manifest gained provenance-drift cross-check, README's broken documented command fixed. 9 Minors deferred to ledger (uneven split balance, near-dup dev/test queries, weak test assertions, etc.) — none blocking. No B3.3 (metrics runner) or B3.4 (calibration) work done yet; no merge/tag/push. GO verdict given to user for proceeding to B3.3/B3.4.
