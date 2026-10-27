---
schema: 1
id: d3sz
type: decision
title: >-
  User authorized fixing [c14au] (relation-target narrowing) and adding Markdown
  note-body storage to propose_and_commit before A1 merge, with explicit scope
  guardrails: no editor, no Markdown rendering, no indexing (Fase B territory)
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-10-26T17:46:09.629Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Scope: (1) pending_ids reverts to HashSet<String> opaque refs, known_ids stays HashSet<NoteId> canonical; tests for URL/DOI/unresolved-symbol/mixed resolved+pending. (2) evidence.md corrected for both this and I-B. (3) propose_and_commit extended to store frontmatter+body as one atomic record; gate must scan the body too; version check covers the whole note; tests for write/read/update/exact-body-preservation/body-only-secret-rejection. Full test+relevant mutation testing+independent whole-branch review required after. No merge to main without further explicit approval (still standing per [c12av]).
