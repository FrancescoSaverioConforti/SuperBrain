---
schema: 1
id: d2da
type: decision
title: >-
  Crate dependency table amended from spec for A1: domain (no deps) -> policy
  (depends on domain only) -> storage (depends on domain AND policy)
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T13:45:27.121Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Spec's original table had storage depending only on domain. Amended so storage can call policy::Gate::evaluate internally from its own crate-private write primitive, making gate-bypass a compile-time impossibility rather than a tested convention. No dependency cycle: policy never depends on storage.
