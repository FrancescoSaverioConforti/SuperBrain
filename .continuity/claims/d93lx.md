---
schema: 1
id: d93lx
type: decision
title: >-
  Research-grade Findings Policy adopted project-wide for SuperBrain/B2 — see
  docs/research/POLICY.md
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T17:02:54.793Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User established a standing policy: any bug/limit/vulnerability/false assumption/significant architectural decision found during dev/test/mutation/review/real-repo validation/benchmarking/debugging must be opened as a tracked finding (ID scheme B2-ID-xxx/B2-SEC-xxx/B2-PERF-xxx/B2-COV-xxx/B2-GRAPH-xxx), not just fixed. Required sections: discovery context, observed-vs-inferred evidence, root cause, impact+severity, solution+why it addresses root cause, alternatives considered, validation, literature/authoritative sources (priority order, "No directly applicable authoritative source identified" when none exist, never force a citation), residual limitations. Workflow: observation→reproduction→hypothesis→root cause→alternatives→intervention→validation→literature→residual limitations, not bug→patch→done. Findings live in docs/research/findings/<ID>.md, indexed in docs/research/findings.md, and must be recorded in Continuity so both Claude and Codex discover them in future sessions. Full policy text: docs/research/POLICY.md.
