---
schema: 1
id: d50vw
type: decision
title: >-
  B1 plan fully consolidated, targeted review GO-WITH-CHANGES, vault scalability
  benchmark scope set by user
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-10-30T13:10:57.429Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Targeted independent review of the 4-point consolidation (agent acfaf505ccc3896de) returned GO-WITH-CHANGES. Findings fixed in docs/superpowers/plans/2026-10-30-b1-deterministic-search.md: (1) Critical — allowlist for Sensitive-note indexable metadata was non-exhaustive against domain::NoteMetadata's actual 12 fields, missing schema_version/classification; now both added to the indexed set, full 12-field disposition stated explicitly. (2) Important — Task 4's source-leak test didn't cover relations leaking the same way; added Step 2c testing relations specifically. (3) Important — plan never stated that the search/index allowlist exclusion (source, relations) does NOT apply to api::retrieve, which may legitimately surface full frontmatter for a note the caller can already read; now stated explicitly in the Architecture section and Task 4/Task 7. (4) Important — rebuild-benchmark gate (Task 1 Step 5) depended on an unknown (user's real vault size) that wasn't escalated as a direct question. Asked user directly: vault size not yet determinable, system must be designed to grow without a small-vault ceiling. User specified: benchmark at synthetic N=100/1,000/10,000/100,000, measuring rebuild time + memory + query P50/P95, no pass/fail threshold imposed — record and assess sustainability honestly, do not introduce disk persistence preemptively if limits are found. (5) Minor — Task 6 Step 7's independent review now also covers reclassification-during-stale-window (Internal->Sensitive between index time and search time) to confirm excerpt-building re-derives classification from the fresh read, not cached Document state. Consolidation closed; plan ready for user's go-ahead to begin subagent-driven-development from Task 0.
