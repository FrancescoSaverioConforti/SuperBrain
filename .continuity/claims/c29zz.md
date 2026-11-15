---
schema: 1
id: c29zz
type: constraint
title: >-
  MCP/agent side can never: self-approve a write proposal, impersonate the
  trusted human approval surface, escalate a read permission to write, bypass
  approval via another Brain API call, or mutate proposal status directly in
  storage — approval is a distinct, privileged capability
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-15T18:59:13.272Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Core security invariant for the B5 human-in-the-loop write model, stated explicitly by the user to prevent the MCP write-proposal mechanism from silently degrading into autonomous write.
