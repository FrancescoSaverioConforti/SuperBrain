---
schema: 1
id: d20os
type: decision
title: >-
  A3 (Security & Durability) technical plan written and awaiting user approval —
  docs/superpowers/plans/2026-10-28-a3-security-durability.md, no code
  implemented yet
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T07:25:04.372Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

New crate vault-security (DPAPI+Argon2id key wrap, PiiStore for sensitive-only AEAD encryption, hash-chained DeletionLog per C5, Backup/Restore against abstract BackupTarget). Explicitly does NOT touch record_path_checked/SEC-001/002/003 — Task 0 pins their open status as a regression guard. Only sensitive classification is encrypted (internal/public stay plaintext Markdown per C3). Backup target is an abstract filesystem trait; physical disk choice stays open per spec's own list. Plan ends at "ready for independent review," no merge/tag.
