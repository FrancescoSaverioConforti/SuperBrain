---
schema: 1
id: c12ah
type: constraint
title: >-
  Vault's symlink-escape check (record_path_checked) has a residual TOCTOU gap
  between canonicalization and the later atomic_write/rename — accepted for A1,
  revisit only if a future phase extends the threat model to adversarial
  concurrent filesystem tampering
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T14:47:47.681Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Confirmed by independent review (empirical reproduction, not assumption) during Task 6. Consistent with A1's explicitly non-crash-safe, single-process-trust threat model. Closing it for real would need openat-style directory-fd-relative opens, out of scope for this plan.
