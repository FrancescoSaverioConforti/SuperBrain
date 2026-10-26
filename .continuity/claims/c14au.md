---
schema: 1
id: c14au
type: constraint
title: >-
  Second whole-branch review upgraded the relation-target narrowing ([c13tk])
  from a forward-looking Phase-B concern to an active A1-scope contract break:
  pending_ids being HashSet&lt;NoteId&gt; means a genuine
  external/unresolved-symbol pending reference (URL, DOI, qualified symbol path)
  cannot even be declared pending today, contradicting spec C6 and the Fase A
  test bullet requiring such references be accepted and flagged
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T17:43:25.489Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Reviewer's proposed minimal fix: keep known_ids as HashSet&lt;NoteId&gt;, revert pending_ids to HashSet&lt;String&gt; (opaque external refs), require NoteId::parse only for targets not already in pending. Alternative: explicitly rule to keep the narrowing, documented in evidence.md and the plan text (currently contradicts itself: Task 3 line ~286 vs Task 9 line ~932). User decision pending as of this review.
