---
schema: 1
id: c28pk
type: constraint
title: >-
  B3 final-test (test-20261114T132816Z-4a5f7a2.json) is CONSUMED, unusable for
  v2 tuning
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-14T13:35:35.251Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

The B3.5 final-test run has been read and reported; per the frozen config's own hard rule, it cannot be used as held-out evidence for B3-TUNE-v2 or any later tuning cycle. Any v2 configuration requires a NEW, never-before-read held-out final-test (new query/label split or new held-out set) before it can be claimed better than the B1 baseline. Dev and calibration splits MAY be reused for v2 development/diagnosis.
