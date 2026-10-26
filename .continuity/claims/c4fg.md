---
schema: 1
id: c4fg
type: constraint
title: >-
  Secret-policy exceptions must be scoped to the exact matched occurrence
  (pattern_id + note_id + hash of matched text), never to "this pattern anywhere
  in this note"
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T13:45:27.859Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User-specified to prevent one approved false positive from silently whitelisting a real future leaked secret under the same pattern in the same note.
