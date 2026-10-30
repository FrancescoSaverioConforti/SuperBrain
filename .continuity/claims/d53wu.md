---
schema: 1
id: d53wu
type: decision
title: >-
  B1 Task 2 fix: storage gets a new public frontmatter/body splitter function,
  index does not duplicate it
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-10-30T13:58:58.213Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

During B1 implementation, Task 2's review found Document::new's (metadata, body) construction boundary would push frontmatter-splitting duplication risk into Task 3's real indexer wiring, since storage::propose::body_of/frontmatter_of are pub(crate), not reachable from index. Ruling (controller decision, implemented in fix round 1): storage grows one small additive pub fn that wraps the EXISTING private splitter helpers (reusing their logic, not duplicating delimiter/CRLF/YAML-block-scalar edge cases) and takes raw record_bytes directly — matching the shape Indexer::mark_current already receives. This is storage's first new public-surface addition in B1 beyond Task 1's list_note_ids; not a write path, doesn't touch SEC-00x surfaces. Rationale: compiler-enforced sync beats a second hand-written implementation of frontmatter-splitting edge cases.
