---
schema: 1
id: c24eb
type: constraint
title: >-
  In mutation testing, a TIMEOUT result can mean the test harness's own
  synchronization deadlocked under the mutation, not that the mutant survived
  cleanly — investigate hook/barrier logic for hardcoded values (e.g. a specific
  generation number) before accepting a timeout as a real gap
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-06T14:41:27.727Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Found in Task 9.5's closing mutation gate: a race test's hook gated on a hardcoded `generation == 1`, which under a mutant that broke the generation counter (every call stuck at 1) made a second call also match and deadlock on the same 2-party barrier — surfacing as TIMEOUT instead of a clean failure, masking a real bug. Fix: gate test hooks on "first call" via an atomic flag, never on a specific expected value the mutation could itself invalidate.
