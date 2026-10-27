---
schema: 1
id: d10sm
type: decision
title: >-
  Fourth A2 plan correction: Recovering->Failed is a valid transition (not
  excluded) — reconcile marks every row Recovering before fixup, including row
  2's rollback, which must resolve to Failed
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-27T08:32:39.795Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Independent reviewer (3rd pass) caught a self-contradiction: Task 1 excluded Recovering->Failed while Task 7's "mark Recovering first" mechanism required it for rollback. Fixed: edge is now valid; test inverted (recovering_can_resolve_to_failed_via_a_rollback); added rollback_genuinely_passes_through_recovering_before_resolving_to_failed to prove row 2 isn't silently special-cased around the generic Recovering step. Also documented as an accepted (non-blocking) tradeoff: reconcile's unbounded git-history scan has no stated worst-case latency bound on large repos, recorded in Global Constraints and Task 10 evidence. A 4th independent review is now running to confirm this fix and do a final full-document contradiction sweep before the user's conditional approval for subagent-driven implementation can be considered met.
