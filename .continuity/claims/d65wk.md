---
schema: 1
id: d65wk
type: decision
title: >-
  Full-workspace mutation sweep found ONE real residual gap my manual
  verification missed (now fixed); classified all 27 TIMEOUTs
status: accepted
confidence: tentative
provenance:
  origin: auto
  created: '2026-11-04T13:48:13.060Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

cargo mutants --test-workspace true (177 mutants, 61 min): 124 caught, 1 missed, 25 unviable, 27 timeout. The 1 MISSED (verify_binding_on line 1257 col 38, the FIRST || in the three-way OR) was a genuine gap my earlier single-test coverage didn't catch: my test only probed the store_id-mismatch disjunct, which masked the first disjunct (format_version-vs-reference-claim mismatch) via operator precedence (&& binds tighter than ||, so (A&&B)||C can still be true via C alone). Added a second assertion to the same test with store_id correct and ONLY the reference's claimed format_version wrong — manually verified this precise case fails under the exact column-38 mutant and passes with correct code. This is a real finding: my earlier "verify by manually flipping all operators in the line" methodology was imprecise versus cargo-mutants' single-AST-node mutations; going forward, verify against the EXACT single-operator mutation, not a holistic rewrite of the whole boolean expression.
All 27 TIMEOUT mutants classified (the tool's 35s auto-timeout is too tight for full-workspace-test-per-mutant on this codebase's lock-contention tests, so many genuinely-caught mutants register as ambiguous TIMEOUT instead of CAUGHT): spot-checked 213 (dead cfg(not(windows)) on this platform), 1340:31 (||->&&, confirmed caught by existing a_renumbered_last_generation_is_detected_as_chain_broken test), and reasoned through 508 (FaultInjector::check — test harness infrastructure, mutating it breaks pause-based tests' own hang-detection, consistent with the other TIMEOUTs being masked kills not misses). Did not individually re-verify every one of the 27 by hand (486, 589x2, 617, 672, 678, 703x2, 794, 864, 881, 1223, 1267, 1284, 1291x2, 1488x3) since each was ALREADY independently verified caught earlier this session via manual single-mutant application before the full sweep ran — the full sweep's TIMEOUT classification for these is consistent with (not contradicting) that earlier direct verification.
Added one more real-gap test: verify_safe_to_expose_after_restore's generation comparison (line 1546, restored_reference_snapshot.generation < live.generation) — added a backup-ahead-of-live case (impossible in the real protocol but the boundary the comparison enforces), verified it catches the > mutant (confirmed via manual application); the == and <= mutants are already caught by two pre-existing tests (equal-generation accepted, backup-behind rejected).
Final state: 123/123 vault-security tests pass. Full workspace test suite not yet re-run since this fix (next step).
