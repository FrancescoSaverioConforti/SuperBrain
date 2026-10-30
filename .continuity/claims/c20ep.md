---
schema: 1
id: c20ep
type: constraint
title: >-
  Pause for user check-in after Task 4's review completes, before continuing B1
  execution
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-30T17:10:13.552Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User asked to stop and check in once Task 4's (confidentiality boundary audit) review verdict comes back, before the SDD loop continues automatically to Task 5. This overrides subagent-driven-development's default "continuous execution, don't pause between tasks" rule for this one checkpoint. Applies once — not a standing instruction to pause after every future task unless the user says so again.
