---
schema: 1
id: d75rg
type: decision
title: >-
  B2 Task 2 implementation complete (not yet committed): RepositoryAllowlist +
  LocalGitRepositorySource, resolves content via git blob at commit (never
  working tree), SHA-256 CommitHash explicitly unsupported (git2's
  unstable-sha256 feature not enabled in workspace), 0 missed mutants;
  independent review in progress
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-05T07:20:56.000Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

read_file resolves git tree/blob objects, never std::fs::read on working tree — verified via test that writes a second commit and confirms reading the first commit's hash still returns the first commit's content. No disk persistence, no filesystem writes outside test fixtures. Awaiting independent review result before commit.
