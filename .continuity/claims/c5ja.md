---
schema: 1
id: c5ja
type: constraint
title: >-
  Never declare A1 equivalent to the final transactional vault — recovery and
  persistent cross-restart idempotency guarantees are implemented and verified
  only in A2
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T13:45:28.039Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User-specified to prevent A1's evidence/commits from overclaiming guarantees that only land with A2's journal and crash-recovery work.
