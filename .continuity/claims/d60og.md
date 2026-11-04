---
schema: 1
id: d60og
type: decision
title: >-
  P0 DeletionLog: full vault-security suite hangs on a stale test assertion
  (deadlock, not flake)
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-04T10:54:25.912Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Root cause found by running crates/vault-security tests single-threaded with --nocapture: test `backup::tests::a_backup_taken_while_a_deletion_is_still_pending_and_then_restored_does_not_expose_the_note_that_deletion_targets` (backup.rs ~line 1209) panics on `assert!(reference.read().unwrap().is_none(), "the trusted reference must not be updated yet, mid-pause")`. Under the new bootstrap_new genesis-reference design (plan 2026-11-01-p0-deletion-log-fail-closed.md), `reference.read()` now legitimately returns `Some(genesis)` pre-deletion instead of `None` — the assertion predates that design and was never updated, same category as other half-adapted tests found uncommitted in deletion_log.rs/pii_store.rs/a3_*.rs.
The panic happens inside `std::thread::scope` while a spawned thread is still parked on `resume_rx.recv()` holding the DeletionLog connection mutex (paused via FaultInjector::pause_at). Because the panicking closure never reaches `resume_tx.send(())`, `thread::scope` blocks forever joining that thread before it can propagate the panic — this is why `cargo test -p vault-security` hangs indefinitely (confirmed hung 50+ min, ~12s actual CPU time, i.e. truly parked not spinning) instead of just failing fast.
Two backgrounded cargo test runs had to be force-killed via Windows PIDs (bash kill -9 on the wrong PID namespace did not work; required PowerShell Stop-Process by native Windows PID).
