---
schema: 1
id: d1ec
type: decision
title: >-
  A1 scope is domain+policy+storage only: schema validation, policy gate, atomic
  file writes with optimistic concurrency — no SQLite journal, no git
  integration, no encryption/backup
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T13:45:26.938Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Journal persistence, crash recovery (A2), and encryption/deletion-registry/backup (A3) are deferred. A1's implementation plan is docs/superpowers/plans/2026-10-26-a1-schema-storage.md, reviewed and approved.
