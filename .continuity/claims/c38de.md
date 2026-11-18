---
schema: 1
id: c38de
type: constraint
title: >-
  brain-api implements query/service semantics only, never the lifecycle of
  objects it queries (index::Index, codegraph_index::CodeGraph) — those objects
  must be already built and supplied by the caller; brain-api never constructs,
  persists, loads, or orchestrates them itself
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-18T09:05:22.151Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Why: neither a real Index (needs a RealIndexer kept in sync with the vault's writes) nor a CodeGraph (no repository-level persistence/loading exists anywhere) can be built from a storage::Vault alone. Rather than invent that infrastructure inside brain-api (scope creep into a new subsystem), the user directed accepting the already-built object as a function parameter and documenting the lifecycle gap explicitly as out of scope, not hidden.
