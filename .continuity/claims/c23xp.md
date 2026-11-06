---
schema: 1
id: c23xp
type: constraint
title: >-
  Every significant resolver/architecture change in Code Graph must be treated
  as an experimental hypothesis, measured against
  precision/recall/F1/FP/FN/unresolved/ambiguity/fan-out/runtime/memory/coding-agent-usefulness
  — never adopted just because it sounds theoretically correct
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-06T10:07:30.830Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Full policy at docs/research/EXPERIMENTAL_POLICY.md ("Evidence-before-claims Experimental Policy"), companion to POLICY.md and EVALUATION_POLICY.md. Introduces stable CALL-RES-A0..A8 configuration IDs (never renumbered), requires distinguishing literature-supported-principle vs project-hypothesis vs measured-result vs inference vs unverified-future-hypothesis, requires separating "unresolved: no candidate" from "unresolved: ambiguous", bans silent deletion of superseded results (erratum-style correction only), and requires a traceable Claim->Evaluation->Configuration->RawObservations->Aggregation->Result->Commit chain for any paper claim. Binding for Claude and Codex in all future B2 sessions.
