---
schema: 1
id: m8pc
type: milestone
title: >-
  B3-TUNE-v2 Task 0 complete: v2 held-out set built, hash-verified, blind to
  tuning session
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-14T14:19:13.239Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

eval/rag/v2-heldout/labels.jsonl + manifest.json committed by dispatched subagent (commits fb03c3e, b715f58). Manifest: 60 queries, abstain_count=15, all 6 categories represented (bugs-fixes=10, cross-file-concept=11, identifier-code=11, notes-decisions=11, patterns-knowledge=9, temporal=8), all 3 difficulties (easy=22, medium=26, hard=12). sha256 be4fa30a3cef7a342f77552b799dd8d27855d1236f371e2e376d0e15c6bdd232 independently re-verified against the committed git blob by this session without reading labels.jsonl's content. Subagent confirmed it never read eval/rag/test/labels.jsonl and made zero changes to runner.py/validate_corpus.py. Content stays unread for the remainder of B3-TUNE-v2 Priority 1 tuning (Tasks 1-4); the held-out result will never decide the P1->P4 proceed decision per prior correction.
