---
schema: 1
id: d64uu
type: decision
title: >-
  P0 mutation triage: all 9 priority functions classified and closed; 15+
  secondary survivors also closed with verified tests
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-04T12:41:42.810Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Completed per-mutant triage of all round-2 survivors (31 total in deletion_log.rs) with evidence for each classification, verified by manually applying each mutant and confirming the new test fails, then reverting:
REAL GAPS CLOSED (new tests added, each manually verified to catch its mutant): is_deleted_verified_locked (mandatory test in storage/restore.rs, catches both Ok(true)/Ok(false)); is_genesis_reference (4-conjunct test); verify_file_identities (2 tests: DB identity, lock-file identity); assert_reference_bound; verify_existing_or_pending (2 survivors: empty-log genesis guard, pending-roll-forward chain_hash guard); verify_consistent (2 survivors: empty-log genesis guard, generation/chain_hash guard); acquire_existing_write_lock's NotFound arm; reconcile_locked (3 survivors: own genesis guard at 1414, &&->|| roll-forward guard at 1463, plus two pre-existing tests I fixed for a store_id-mismatch bug that was masking their real target); verify_binding_on (format_version/store_id three-way OR); migrate_legacy_empty (4 survivors: attestation blank-field OR, table_count==0, count!=0, operation_id match on the reference-absent-but-DB-metadata-survives branch); is_retryable_genesis_partial (3 survivors: unrelated-extra-table, deletions-row-count, both .any() guards); GenerationRef::deserialize (format_version/store_id/hash OR); DpapiTrustedReferenceStore::bound_path and ::read's NotFound guard (2 tests, Windows-only #[cfg(windows)]).
EQUIVALENT MUTANTS, proven by inspection (not pursued further): acquire_existing_write_lock's Instant::now()<=deadline (timing boundary, unreachable deterministically); verify_chain_on's generation<0 vs ==0/<=0 (mathematically subsumed by the adjacent `as u64 != expected_generation` check — any negative i64 cast to u64 is always >= 2^63, which can never equal expected_generation); reconcile_locked's three row.generation</==/>/<=  r.generation variants at line 1488 (both if/else branches return the IDENTICAL Err(DeletionLogError::Inconsistent) — no test can ever observe a difference); DeletionLogWriteGuard::drop mutated to no-op (the guard's own doc comment confirms file-handle Drop releases the OS lock regardless, just not "first" — observably equivalent for any test not probing lock-release timing specifically).
OUT OF SCOPE / NOT PRODUCTION-REACHABLE ON THIS PLATFORM: atomic_replace_reference's #[cfg(not(windows))] variant (dead code on this Windows-only build/CI); identity_of_open_file's Windows bit-packing formula (589) and FaultInjector::none() (486, test-only harness infrastructure) — NOT yet closed, residual gap, deferred (identity_of_open_file would need constructing files with specific nFileIndexHigh/Low values, impractical via the public API without deep FFI mocking).
Fixed a clippy::redundant_pattern_matching introduced by one of my own new tests (matches!(_, Ok(_)) -> .is_err()). Full workspace test suite re-confirmed green after the fix: 554 tests total (up from 529 baseline), 0 failures. cargo clippy --workspace --locked -- -D warnings clean (the plan's actual checkpoint command, without --all-targets; two PRE-EXISTING assert!(true) lints in domain/storage only surface under --all-targets and are unrelated to this work, left untouched). cargo deny check is currently blocked by a transient DNS resolution failure (github.com unreachable from this environment) — not a new finding; it passed cleanly earlier this session before any of today's changes and nothing since has touched Cargo.lock/dependencies.
