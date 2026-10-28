---
schema: 1
id: d19hz
type: decision
title: >-
  A2 pushed to origin/main (cfeef6e) and tag a2-complete pushed (-> ffeb9ab);
  A1+A2 now public on GitHub, branch/worktree a2-recovery still intact, A3 not
  started
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T07:14:07.612Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Clean fast-forward push, no force, no --tags (pushed main and the single tag explicitly). SEC-001/002/003 remain in their real state (OPEN / NARROWED-write-path-open / NARROWED-write-path-and-hardlink-open) per [d18wp]; this push did not change their status, only made the completed A2 work remote.
