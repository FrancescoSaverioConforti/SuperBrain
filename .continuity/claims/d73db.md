---
schema: 1
id: d73db
type: decision
title: >-
  RepositoryId design decision: opaque UUID assigned by in-memory
  RepositoryRegistry on first registration, never derived from
  path/remote-URL/root-commit-hash — rejected those three alternatives
  explicitly
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T07:01:19.678Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Path hash breaks on move (user's explicit veto). Root-commit hash rejected: undefined for 0-commit repos, ambiguous for merged-unrelated-histories repos, differs after shallow clone/squash/orphan even when user considers it "same project." Remote-URL hash rejected: undefined for local-only repos, changes on remote rename/URL-scheme switch, differs for forks. RepositoryRegistry.associate() lets host explicitly re-link a moved root to its existing id; never inferred automatically. Registry is in-memory only in Task 1 — persistence deferred to Task 10 (gated by S1/S2) per user's explicit answer to avoid creating a disk write path that routes around those gates.
