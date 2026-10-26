---
schema: 1
id: c2jq
type: constraint
title: >-
  Vault records persist content+version as a single atomically-replaced unit
  (one file, one atomic_write call) — never as two separate files
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T13:45:27.481Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User-specified after finding the original content+.version sidecar design didn't guarantee joint atomic replacement.
