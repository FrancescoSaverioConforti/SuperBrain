---
schema: 1
id: d92ou
type: decision
title: >-
  Real-repo smoke tests must use only MIT/BSD/Apache-2.0-licensed repos, cloned
  to scratchpad (never inside SuperBrain), deleted after the report is finalized
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T16:33:34.815Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User constraint for all B2 real-repository validation checkpoints (not just this Python/DL one): only clone repos with an explicit, clear, permissive LICENSE (MIT/BSD/Apache-2.0 or equivalent) — never repos with no LICENSE file, ambiguous terms, or restrictive/custom licenses. Clone target is the session scratchpad/temp directory, never inside the SuperBrain project tree. Treat the clone as strictly read-only. After finalizing the report, delete the temporary clone and keep only the report + necessary metadata (repo URL, exact commit, license identifier). Report must record repo, exact commit, license type, and LICENSE file path/SPDX id.

Before B2's final closure, separate real-repo smoke tests are also required for Rust, TypeScript/JavaScript, and C#/C++ — same license requirements apply to those too. Task 10 remains NO-GO until this Python/DL smoke test's results are reviewed by the user.
