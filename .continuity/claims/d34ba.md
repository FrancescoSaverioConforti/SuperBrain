---
schema: 1
id: d34ba
type: decision
title: >-
  A3 post-N1 stale-backup-restore follow-up complete: race fix (commit a1d304c)
  closed the one-review requirement; branch at 33 commits, awaiting user's final
  merge/tag/push authorization
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-30T07:43:11.262Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Targeted review found a resurrection race (deleted_since_backup computed before check 1). Fixed directly by controller (adjudicate-and-close, same as N1), RED→GREEN test added, 348/348 tests + clippy + cargo deny clean, evidence.md updated. No merge/tag/push performed. User asked whether adjudicate-and-close satisfies their "exactly one review" requirement or whether they want a second dispatched re-review of the fix — unresolved, pending user reply.
