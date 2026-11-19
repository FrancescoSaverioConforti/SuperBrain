---
schema: 1
id: rk5sa
type: risk
title: >-
  R3 — mcp-client spawns external MCP servers with unsanitized full parent
  environment inheritance
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-19T11:05:51.103Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

crates/mcp-client/src/registration.rs:138, connect_now: tokio::process::Command::new(command) has no .env_clear()/.envs() call, so every external MCP child process inherits the full parent environment by default. Re-verified severity: SuperBrain's own codebase reads only 3 env vars total (SUPERBRAIN_MCP_CLIENT_DISABLED kill-switch; CARGO_CFG_WINDOWS build-time; SUPERBRAIN_F2_CHILD_LOCK_DIR test-only) — none are secrets; master keys come from DPAPI, never env vars. Real residual risk is narrower than first flagged: not "SuperBrain leaks its own secrets" but "SuperBrain doesn't isolate spawned external processes from whatever the ambient user shell/OS environment happens to carry" (e.g. unrelated API keys/cloud creds in the user's shell). Severity: Medium, not High. Surfaced in the 2026-11-19 Data Governance/Security/Trust architectural audit, new finding not among B5 closure's original 10 open items. Fix: explicit .env_clear() + minimal allowlist on the spawned Command, additive, no frozen-contract impact.
