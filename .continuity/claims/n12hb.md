---
schema: 1
id: n12hb
type: next_action
title: >-
  M4-C (MCP request cancellation awareness) CLOSED, committed 6a13ea9, not
  pushed — resolves rk10ph; resume remediation at M5
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-19T12:07:07.394Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

acquire_concurrency_permit now races the semaphore acquire against the request's real rmcp CancellationToken (tokio::select!, biased toward cancellation) — a request cancelled while queued never enters the real handler body, never reaches propose_and_commit. All 12 tool handlers gained a cancellation_token parameter via rmcp's own FromContextPart extractor; tokio-util added as a direct dependency (already transitive via rmcp, same locked version). Verified with a real MCP wire test (genuine notifications/cancelled over a real duplex connection). Full workspace suite + mcp-server clippy pass, no regressions. M4's own queue-depth-cap verdict (rk9pw, DEFER WITH ACCEPTED RISK) is unchanged by this — M4-C only fixed the orthogonal cancellation-responsiveness gap. M5 not started.
