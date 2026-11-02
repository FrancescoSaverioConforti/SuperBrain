---
schema: 1
id: d58wq
type: decision
title: >-
  Graphify (Graphify-Labs/graphify, CLI package graphifyy) installed and
  registered as a skill for both Claude Code and Codex globally
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-02T07:19:43.019Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

CLI via `uv tool install graphifyy`; skill registered with `graphify install --platform claude` (→ ~\.claude\skills\graphify, also created ~\.claude\CLAUDE.md) and `graphify install --platform codex` (→ ~\.codex\skills\graphify). Invoke with `/graphify .` (Claude) or `$graphify .` (Codex) in a project to build a local knowledge graph.
