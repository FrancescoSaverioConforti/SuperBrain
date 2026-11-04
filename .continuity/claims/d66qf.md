---
schema: 1
id: d66qf
type: decision
title: >-
  DL-02/DL-03 confirmed non-reproducible: original audit probe re-run against
  fixed code, every scenario now fails closed
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-04T13:51:41.055Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Took the original audit's exact reproduction probe (docs/security/reproductions/deletion-read-gate.rs, which produced after_missing_log_returns_plaintext=true, missing_reference_other_note_returns_plaintext=true, tampered_chain_deleted_note_returns_plaintext=true pre-fix) and attempted to compile it verbatim against the current fixed crate: it FAILS TO COMPILE (DeletionLog::open is no longer pub, only pub(crate)/#[cfg(test)]) — the exact vulnerable unauthenticated-open entry point is structurally gone from the public API. Ported the same attack scenario to the real public entry points (PiiStore::bootstrap_deletion_log/open_deletion_log, DeletionLog::open_existing under the hood) as docs/security/reproductions/deletion-read-gate-p0-rerun.rs and ran it standalone via rustc (not cargo test) against target/debug/deps' current rlibs, exactly mirroring the original audit's methodology.
Results (saved verbatim to docs/security/reproductions/deletion-read-gate-p0-rerun.stdout.txt): after_missing_log_returns_plaintext=false (was true) -> Err(DeletionLogUnavailable{reason:"...lost required coordination state"}); missing_reference_other_note_returns_plaintext=false (was true) -> same error; tampered_chain_deleted_note_returns_plaintext=false (was true) -> Err(DeletionLogUnavailable{reason:"deletion log hash chain is broken"}), with tampered_chain_detectable_at_reopen=true (ChainBroken surfaces even at open_deletion_log, before any read is attempted). Added a new assertion not in the original probe: no_implicit_recreation_confirmed=true, proving open_existing never silently recreates a missing DB file (the original DeletionLog::open did, which was part of the original bug's mechanism).
All original audit source/binary artifacts and this rerun's own scratch synthetic directories (.superpowers/p0-rerun-missing-log-*, .superpowers/security-audit-build-rerun) were cleaned up after use; nothing untracked remains except the two new committed-evidence files.
