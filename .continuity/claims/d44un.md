---
schema: 1
id: d44un
type: decision
title: >-
  A3 merged locally to main: merge commit
  083bb54a0fce61ab40d088a0fe9f9332c68f9c8b (--no-ff, from a3-security-durability
  HEAD 12551c4), tagged a3-complete. Post-merge verification on main: 377/377
  tests, clippy clean, cargo deny clean — all green.
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-30T12:37:26.514Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Merge was clean, no conflicts (confirmed beforehand: main's 25 divergent commits touch only .continuity/claims/*.md, zero file overlap with A3's changes). eval/results/A3/evidence.md (committed on the branch) documents all findings and accepted residual debt: F4 partial (PiiStore::get/get_before_image still pub), the constructor-dependent "no PiiStore" exception, the F3 tombstone-exemption residual (confirmed non-exploitable), deleted-note bytes persisting on disk in restored vaults. SEC-001/002/003 unchanged per standing requirement. No push to GitHub yet (local main is now 68 commits ahead of origin/main) — user's standing instruction requires separate authorization before push. Branch and worktree not deleted. Phase B not started.
