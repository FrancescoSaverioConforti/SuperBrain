---
schema: 1
id: c34fu
type: constraint
title: >-
  Security-critical trust-boundary invariants in B5 (e.g. "an MCP-originated
  caller can never get approval/write rights") must be enforced structurally —
  no public constructor or parameter that a transport layer could accidentally
  forward from untrusted payload data — not merely documented as a convention a
  caller is expected to follow
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-16T10:01:37.087Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Confirmed in Task 10: user explicitly rejected the plan's literal WritePolicy::new(bool, bool) constructor because it would let approve_write_allowed be caller-supplied even if no current code misuses it. Only parameterless named constructors (mcp_agent(), trusted_cli()) are public. Apply the same discipline to future B5 tasks (MCP server tool registration, MCP client external-tool authorization, B5.7/B5.9).
