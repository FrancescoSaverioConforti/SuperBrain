---
schema: 1
id: rk7us
type: risk
title: >-
  R9 — proposal temp-file write lacks explicit O_EXCL, mitigated by
  unpredictable naming + post-write re-verification
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-19T11:05:52.492Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

crates/storage/src/handle_safety.rs write_verified_atomic: temp filename is {file_name}.tmp-{pid}-{nanos}, not predictable, but the open call has no O_EXCL/exclusive-create flag at that specific site. Practical window is narrowed by the unpredictable name and the subsequent post-rename identity re-verification (same primitive already closing SEC-001/002/003), but not structurally guaranteed the same way note writes are. Severity: Low. Surfaced in the 2026-11-19 architectural audit's filesystem-safety verification pass; no contradicting evidence, no new vulnerability demonstrated — a hardening opportunity, not a confirmed exploit path.
