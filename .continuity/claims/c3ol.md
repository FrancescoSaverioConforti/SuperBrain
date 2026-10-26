---
schema: 1
id: c3ol
type: constraint
title: >-
  note_id must be validated against path traversal (.., separators, absolute
  paths) and symlink escape (canonicalize and confirm containment in vault root)
  before any filesystem path is built
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T13:45:27.666Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User-specified security constraint for storage::NoteId and Vault write/read paths.
