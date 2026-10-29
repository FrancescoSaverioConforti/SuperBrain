---
schema: 1
id: d28fr
type: decision
title: >-
  A3 Task 8 mutation testing on vault-security (342 mutants: 172 caught, 39
  unviable, 56 timeouts, 75 missed) found two genuine security-relevant test
  gaps, now being fixed: AEAD nonce freshness and Argon2id parameter strength
  are both currently unverified by any test
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-29T08:47:33.938Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

fresh_nonce() in pii_store.rs could be replaced with a fixed [0; NONCE_LEN] and every existing test would still pass (AES-GCM nonce reuse under the same key is a real catastrophic failure, not hypothetical). argon2id_params() in recovery_kit.rs could silently fall back to the argon2 crate's own weak Default and every existing test would still pass (defeats the documented OWASP-floor brute-force resistance the recovery kit's whole threat model relies on). Most other missed/timeout mutants (dead code on this Windows-only platform, NotFound-guard I/O-error paths, binary-parsing boundary conditions, a few deletion_log.rs reconcile boundary combinations) were triaged as safely deferred, matching A1/A2's own documented precedent for this class of gap. Fix dispatched: one dedicated regression test per gap, asserting actual randomness across calls and the exact documented KDF parameter values respectively.
