---
schema: 1
id: c46zx
type: constraint
title: >-
  M7 implementation authorized up through mcp-audit-bridge crate + 10 named
  acceptance tests; STOP before M7.5/M7.6 and report results first; if
  acceptance requires a non-additive change to frozen persisted formats, STOP
  immediately instead of implementing around it
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-25T13:37:12.833Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

10 required acceptance tests: normal import; duplicate reimport -> zero new Brain facts; crash after dest append before checkpoint -> restart no duplicate; Brain events interleaved after the imported fact -> restart still no duplicate; tampered source chain -> fail closed; destination with two copies of same SourceRecordIdentity -> anomaly detected; checkpoint lost but destination intact -> correct recovery; checkpoint corrupted -> fail closed; locator changed with compatible chain -> controlled rebind; same path with incompatible chain -> fail closed. Final PASS verification must run in the main session, not delegated to a subagent (standing rule from the original M7 memo).
