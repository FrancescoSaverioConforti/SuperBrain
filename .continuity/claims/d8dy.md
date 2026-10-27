---
schema: 1
id: d8dy
type: decision
title: >-
  A2 plan underwent a 6-point architectural review (IndexState honesty,
  concurrent-read race, rollback distinguishability, test visibility, git
  idempotency, test validity) — all fixed in
  docs/superpowers/plans/2026-10-27-a2-recovery.md, independent reviewer
  dispatched to verify
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-27T07:45:11.793Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Standing rules now baked into the A2 plan's Global Constraints, applicable to future storage/journal work: (1) IndexState::Current must never be returned by any A2 code — only Stale is honest pre-B1; (2) read_note/current_version must share the writer's per-note lock, not just rely on hash verification, to avoid reading mid-rename; (3) a rolled-back write must end at WriteState::Failed, never at a committed state, and must never git-commit under its own operation_id; (4) tests touching pub(crate)/#[cfg(test)] internals must be inline `#[cfg(test)] mod` in src/*.rs, never tests/*.rs integration files (those can't see either); (5) git-commit idempotency must use a journal-stored git_commit_oid as the primary O(1) key, with history-scan only as fallback repair, and staging must be isolated via TreeBuilder from HEAD, never the shared .git/index; (6) adversarial/characterization tests must assert a concrete falsifiable outcome, never just print and pass unconditionally. User has not yet approved the revised plan or an execution method — independent review in progress.
