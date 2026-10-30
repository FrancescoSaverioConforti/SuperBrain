---
schema: 1
id: c18mn
type: constraint
title: >-
  A3 merge pending second targeted review: user required a further scoped review
  of the TOCTOU window between restore's last DeletionLog read and publication
  of restored data. If reviewer confirms a real vulnerability, do NOT fix it
  autonomously — present findings + architectural impact to user first and wait.
  No merge/tag/push until user authorizes.
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-30T07:56:30.489Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User's explicit instruction after the a1d304c race fix: review must stay narrowly scoped (not general A3 review), and any confirmed finding must be presented, not auto-fixed.
