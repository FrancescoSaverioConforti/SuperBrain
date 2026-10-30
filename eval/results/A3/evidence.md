# A3 — Security & Durability: closing evidence

**Date:** 2026-10-28/29 (implementation spanned two sessions, continued across a session boundary with no lost work).

**Plan:** `docs/superpowers/plans/2026-10-28-a3-security-durability.md` (second draft, independently reviewed before implementation on all 7 points the user required — gate availability check, confidentiality boundary, single crash-recoverable pipeline, recovery-kit/master-key guard, authenticated deletion log, SEC-00x drift oracle, no second public write/delete entry point).

**Branch:** `a3-security-durability`, worktree `.worktrees/a3-security-durability`.

**Merge base:** `fc5171a` (the commit that added the approved plan to `main`).

**HEAD at the original evidence run:** `988cdca`.

**Re-run after the final-review fix wave:** the final whole-branch review of `fc5171a..439237b` found one Critical and five Important defects (C1, I1–I5; see §4.7). They were fixed in commits `4437772`..`45da6de`, and **every result in §3 was re-run after the fix wave** (test/clippy/deny against `45da6de`, the last code commit; the full mutation run against `7f373c5` plus a targeted re-check at `45da6de`, see §3.4); figures from the original `988cdca` run are kept only where marked as such. This file's own commits add no production or test code.

---

## 1. Scope of this file

This is Task 8, the final task of the A3 plan. It closes A3 by running the full acceptance checklist the spec's Fase A table and the plan's own Global Constraints require, and recording the artifact — matching the depth and honesty standard A1's and A2's own `eval/results/*/evidence.md` files set (particularly A2's mutation-testing section, which this file follows closely in structure).

---

## 2. Commits in this branch (28)

| # | Commit | Task | What it did |
|---|---|---|---|
| 1 | `1622742` | 0 | Added the SEC-00x status oracle (later corrected). |
| 2 | `1705808` | 0 fix | Replaced the oracle's test-calling-tests pattern with a non-executing comment anchor per the user's explicit framing correction. |
| 3 | `0362201` | 1 | Scaffolded `vault-security`, depending only on `domain`. |
| 4 | `a3063ff` | 2 | DPAPI master key wrap + Argon2id recovery kit + existing-store guard. |
| 5 | `43cdafa` | 3 | Confidentiality-boundary audit stubs (6 surfaces, `#[ignore]`d pending Tasks 6–7). |
| 6 | `e1fd7ab` | 3 fix | Corrected task-owner mislabels in the stub file. |
| 7 | `48637a1` | 3 fix | Corrected the SAME stale task-number cross-references at the plan-document level (a genuine plan defect, not just a copy error). |
| 8 | `8450df4` | 3 fix round 2 | Corrected one remaining stale reference the first sweep missed. |
| 9 | `2b4f8cc` | 4 | Made `Gate::evaluate`'s `Sensitive` rejection conditional on `EncryptedStoreAvailability`, never bypassable, secret-content scan always runs. |
| 10 | `0de2665` | 5+5b | Hash-chained `DeletionLog` with a DPAPI-authenticated trusted reference. |
| 11 | `bb4a0a7` | 5 fix | Added the missing end-to-end test for `reconcile`'s general roll-forward branch. |
| 12 | `e0b080a` | 6 | `PiiStore` encrypted record store. |
| 13 | `525c5da` | 6 | Single crash-recoverable sensitive-write pipeline (`write_sensitive_record_unchecked`, sharing the A2 journal/lock/`WriteState` machine). |
| 14 | `de877d5` | 6 fix round 1 | PII before-image mechanism (the Critical fix — see §4). |
| 15 | `e1e6a0b` | 6 fix round 2 | Closed PII before-image orphan-cleanup and one rollback gap the round-1 re-review found. |
| 16 | `695bf2c` | 6 fix round 3 | Rewrote `roll_back_pii` around an explicit, total `LivePii` decision table; fixed 7 more self-audited instances of the same fail-open pattern. |
| 17 | `26d54bb` | 7 | Encrypted `backup()`/`restore()` gated on the live deletion log. |
| 18 | `fbc07dc` | 7 fix round 1 | Fixed `restore()` silently dropping PII ciphertext (Critical) and 3 Important findings. |
| 19 | `151674c` | 7 fix round 2 | Fixed `restore()` silently clobbering newer live PII content; made `.pii` writes atomic. |
| 20 | `988cdca` | 8 | Added two regression tests this task's own mutation-testing pass found missing (nonce freshness, Argon2id parameter strength). |
| 21 | `439237b` | 8 | First version of this evidence file. |
| 22 | `4437772` | final review C1 | `restore()` excludes (and reports) deleted notes instead of rejecting the whole restore. |
| 23 | `0cca592` | final review I2 | Deleted note ids are retired (`StorageError::NoteDeleted`); `PiiStore::get_unless_deleted`; deletion-log paths centralized on `PiiStore`; I4 #4 fingerprint test. |
| 24 | `cdf6dc0` | final review I3 | `DeletionLog::reconcile` runs on `Vault::open_with_pii_store`. |
| 25 | `d7e43e7` | final review I1 | `storage::restore_vault_from_backup`: a restored vault is readable through `Vault::open`/`read_note`. |
| 26 | `7f373c5` | final review I4+I5 | `DeletionLog::verify_chain` (whole-chain walk, on startup and restore); three mutation survivors closed. |
| 27 | `45da6de` | final review I4/I5 | Six more survivors found by the post-fix mutation re-run closed (incl. a pause-based test that hung instead of failing). |
| 28 | this commit | final review docs | This file corrected; plan's stale "Task 9" reference fixed. |

Commits 1–20 each passed an independent task review or scoped re-review before the next task began; commits 22–27 are the single bundled fix wave for the final whole-branch review and await its one scoped re-review, per `subagent-driven-development`. The SDD ledger (`.superpowers/sdd/2026-10-28-a3-security-durability/progress.md`) is the full record of every finding, ruling, and re-review verdict; this file summarizes it against the spec's acceptance criteria rather than repeating it.

---

## 3. Commands run and results

All commands were run from the `a3-security-durability` worktree root.

### 3.1 `cargo test --workspace`

```
cargo test --workspace
```

**Result at `45da6de`: 342 passed, 0 failed, 0 ignored** (317 at `988cdca`; the fix wave added 25).

| Suite | Passed at `988cdca` | Passed at `45da6de` |
|---|---|---|
| `domain` unit tests | 32 | 32 |
| `policy` unit tests | 14 | 14 |
| `storage` unit tests (inline `#[cfg(test)]` across `vault.rs`, `journal.rs`, `recovery.rs`, `fault.rs`, `git.rs`, `atomic.rs`, etc.) | 170 | 170 |
| `storage` integration — `a3_backup_restore.rs` (new: C1, I1) | — | 3 |
| `storage` integration — `a3_confidentiality_boundary.rs` | 6 | 6 |
| `storage` integration — `a3_pii_integration.rs` (+4 I2, +2 I3) | 8 | 14 |
| `storage` integration — `integration.rs` (A1/A2 suite; edited by A3, see below) | 24 | 24 |
| `vault-security` unit tests (`keys.rs`, `recovery_kit.rs`, `deletion_log.rs`, `pii_store.rs`, `backup.rs`, `availability.rs`) (+4 backup, +4 pii_store, +8 deletion_log) | 58 | 74 |
| `domain`/`policy`/`vault-security` doc-tests | 0 / 0 / 0 | 0 / 0 / 0 |
| `storage` doc-tests (incl. 4 `compile_fail` tests pinning the public write surface) | 5 | 5 |
| **Total** | **317** | **342** |

**Zero `#[ignore]`d tests remain anywhere in the workspace** — confirmed by `grep -rn '^\s*#\[ignore' crates/` returning no matches. The one test that WAS `#[ignore]`d mid-plan (the backup-blob confidentiality test, pending Task 7) was un-ignored and completed by Task 7.

**Correction (the first version of this file said `integration.rs` was "unchanged by A3"; it was not).** Task 6 (`525c5da`, `de877d5`) edited it for `propose_and_commit`'s new `pii_store: Option<&PiiStore>` parameter: a `test_pii_store()` helper and a `Some(&pii_store)` argument at each call site (47 insertions, 5 deletions against `fc5171a`). The final-review fix wave (`d7e43e7`) extended `storage_exposes_exactly_one_public_write_function` to also pin `restore.rs` to exactly one public function. No pre-existing test's assertions were weakened or removed: the 24 tests still assert exactly what they asserted at `fc5171a`, which is what confirms A3's write-pipeline extension is additive for `Internal`/`Public` notes. The A1/A2-era `storage` unit tests were likewise touched only at call sites whose crate-private signature A3 extended (for example `recovery::reconcile` gained a `pii_store` argument: the removed lines in `git diff fc5171a.. -- crates/storage/src/recovery.rs` are those calls plus the doc/body lines of the functions A3 extended).

### 3.2 `cargo clippy --workspace -- -D warnings`

```
cargo clippy --workspace -- -D warnings
```

**Result: clean** at both `988cdca` and `45da6de`. `Finished \`dev\` profile [unoptimized + debuginfo] target(s)` with zero warnings and zero errors.

**Scope of this claim, matching A2's own precedent (§2.3 of that file):** this form lints library targets, not test code. `cargo clippy --workspace --all-targets -- -D warnings` fails on exactly one pre-existing, A1-era issue, unrelated to A3: `crates/domain/src/lib.rs:16`, `fn crate_builds() { assert!(true); }` (`clippy::assertions_on_constants`). `storage` carries the identical pattern at its own `lib.rs:97`, also pre-existing, also out of A3's scope. Every A3 task dispatch ran `cargo clippy --workspace -- -D warnings` (the non-`--all-targets` form) clean before being reported done, per the plan's Global Constraints; this placeholder-test lint was flagged by multiple task implementers across this plan as out-of-scope and is recorded here, not silently absorbed.

### 3.3 `cargo deny check`

```
cargo deny check
```

**Result at `7f373c5` (2026-10-29): `advisories ok, bans ok, licenses ok, sources ok`** (the fix wave added no dependency; the new integration tests use `rusqlite`/`sha2`, already `storage` dependencies).

**Result at `45da6de` (2026-10-30): `advisories FAILED, bans ok, licenses ok, sources ok`.** The one failure is **RUSTSEC-2026-0418** (`libz-sys` build script leaks environment variables into the build directory), published between the two runs, against `libz-sys 1.1.30`, which comes in through `git2 -> libgit2-sys`, an A1/A2 dependency of `storage`. It is not introduced by A3: `Cargo.lock` pins the same `libz-sys 1.1.30` on `main`, at `fc5171a` and at HEAD. The advisory's fix is `cargo update -p libz-sys` (1.1.30 -> 1.1.31, a patch bump). It was **not** applied in this fix wave, which does not touch `Cargo.lock`; it is flagged for the plan owner, and it applies to `main` identically.

New dependencies A3 added, all resolving within the existing `deny.toml` allowlist (`Apache-2.0`, `MIT`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `Unicode-3.0`), confirmed against `cargo deny`'s actual metadata, not assumed from the plan's Tech Stack section:

| Dependency | Version | License | Purpose |
|---|---|---|---|
| `aes-gcm` | 0.11.1 | Apache-2.0 OR MIT | AEAD for `PiiStore` ciphertext and backup blobs |
| `argon2` | 0.6.0 | Apache-2.0 OR MIT | Recovery-kit passphrase KDF |
| `zeroize` (+`zeroize_derive`) | 1.9.1 / 1.5.0 | Apache-2.0 OR MIT | Key-material zeroization on drop |
| `windows` | 0.62.2 | MIT OR Apache-2.0 | DPAPI FFI (`CryptProtectData`/`CryptUnprotectData`) |
| `rusqlite` | 0.31.0 | MIT | `DeletionLog`'s SQLite table (already a `storage` dependency; `vault-security` pins the same version) |
| `hkdf` | 0.12.4 | Apache-2.0 OR MIT | Backup-blob subkey derivation, independent of the raw master key |
| `base64` (+`base64ct`) | 0.22.1 / 1.8.3 | Apache-2.0 OR MIT | Recovery-kit printable encoding |
| `getrandom` | 0.4.3 | Apache-2.0 OR MIT | CSPRNG for keys, nonces, salts (used directly instead of `aes-gcm`'s `OsRng` re-export, which had an API mismatch against the pulled `rand_core` version — documented at the point of use in `keys.rs`) |

**No new `dependency-exceptions.md` entry was needed** — every A3 dependency passes the allowlist cleanly, with no RustSec advisory, ban, or source exception required.

### 3.4 Mutation testing (`cargo mutants -p vault-security`)

Task 8 runs this pass (the plan's Global Constraints said "Task 9", stale numbering from before an earlier renumbering; there is no Task 9, and the plan now says Task 8).

| Run | Command | Tested | Caught | Unviable | Timeouts | Missed | Survivors (missed + timeouts) |
|---|---|---|---|---|---|---|---|
| Original (before `988cdca`) | `cargo mutants -p vault-security --no-times` | 342 | 172 | 39 | 56 | 75 | 131 |
| Post-fix, full (`7f373c5`) | `cargo mutants -p vault-security --no-times --timeout 60` (95 min) | 359 | 191 | 39 | 59 | 70 | 129 |
| Targeted re-check (`45da6de`) | `--re` over the six survivors below, plus `verify_chain_on` | 18 | 16 | 0 | 0 | 2 | 2 (both equivalent, see below) |

Per the plan's Global Constraints: `vault-security` holds this phase's most security-critical logic and is held to the same mutation-testing bar A1/A2 required for `domain`/`policy`/`storage`. Survivors were compared mutant-by-mutant (by file and mutation, ignoring line numbers) between the original and post-fix runs:

- **9 original survivors no longer survive:** the two Task 8 closed itself (`pii_store` `fresh_nonce` -> `[0;12]`/`[1;12]`, `recovery_kit` `argon2id_params` -> `Default`) and the four I4 targets the final review found mis-triaged: `backup.rs` `derive_backup_subkey` -> `[0;32]`/`[1;32]`, `backup.rs` `fresh_nonce` -> `[0;12]`/`[1;12]`, `deletion_log.rs` `compute_chain_hash` -> `"xyzzy"`, `pii_store.rs` fingerprint `.pii` `==` -> `!=`. Each I4 test was also verified by hand-applying its mutant (FAILED) and restoring the source.
- **7 new survivors appeared, in code the fix wave touched;** `45da6de` closed 5 of them: the sentinel `&&` -> `||` pair in `restore_with_pii_root` and `verify_safe_to_expose_after_restore` -> `Ok(())` were timeouts, not gaps (an assertion in the pause-based "restore while the deletion is pending" test panicked inside `thread::scope` before the deleter was resumed, hanging the scope; the test now resumes before asserting); `PiiStore::deletion_log_reference_path` -> `Default` (both deletion-log paths are now pinned); `verify_chain_on`'s `||` -> `&&` on the generation check (renumbering the last row is now tested). The same test reorder also killed one carried survivor (`delete !` on the check-1 failure branch).
- **The 2 remaining new survivors are equivalent:** `verify_chain_on`'s `generation < 0` -> `<=`/`==`. The guard is redundant: a negative `i64` cast to `u64` is at least 2^63 and never equals the expected generation, so the second clause rejects it anyway.

**Survivors at `45da6de`: 123** = 129 (full post-fix run) - 6 closed by `45da6de` (verified by the targeted run, not by a second full run). That is 121 carried over from the original run, same call sites with shifted line numbers, plus the 2 equivalent ones above.

#### 3.4.1 Mutation survivor triage

The groups below are the original triage of the 131 survivors of the first run (counts "of 131", file:line as of that run). They still describe the **121 carried-over survivors** at `45da6de`; the per-group counts are approximate ("~"), because several call sites contribute 2–4 near-identical guard variants each, and they were never individually re-derived. **Correction:** the first version of this file placed four real, cheap-to-close survivors in Group A ("hard to close") or omitted them: the backup subkey, the backup nonce, the chain hash and the PII fingerprint extension check. Those are now closed (§3.4) and are not part of any group below. The final review also noted `backup.rs`'s `collect_vault_files` dot-entry skip (`||` -> `&&`) as an untested survivor; it is carried over and listed in §5.

**Dead-code-on-this-platform (4 of 131 survivors).** `keys.rs:319,324` — `dpapi_protect`/`dpapi_unprotect`'s `#[cfg(not(windows))]` stub branch, which this project (Windows-only target, confirmed throughout the plan) never compiles into the binary under test. The real, `#[cfg(windows)]` implementations of these functions are separately tested and were independently, repeatedly verified sound across Task 2's and Task 5's reviews (direct FFI tracing against the `windows` crate's actual signatures, `LocalAlloc`/`LocalFree` lifetime checks). **Not a coverage gap.**

**Group A — hard to close, needs a non-`NotFound` I/O error (~45 of 131, same class as A2's own Group A).** Every `NotFound`-guard call site across `backup.rs`, `deletion_log.rs`, `keys.rs`, `pii_store.rs` follows the identical idiom this plan's Task 6/7 fix cycles established and independently re-verified multiple times: `match fs_call { Ok(_) => ..., Err(e) if e.kind()==NotFound => benign_default, Err(e) => propagate }`. Killing the guard-collapsed-to-`true`/`false` mutants needs the underlying call to fail for a reason other than "genuinely missing" (permission denied, a locked handle, a directory substituted for a file) — no test induces this on Windows without new fault-injection infrastructure beyond this plan's scope. **Justified as hard to close**, matching A2's precedent exactly.

**Group B — ordinary untested `NotFound`/boundary path, cheap to close (~55 of 131).** The `==`→`!=` variant at the same call sites (reachable by simply deleting the target file first — no existing test happens to for several specific sites); buffer-length boundary checks in `pii_store::deserialize`, `backup.rs`'s `read_u32`/`read_u64`/`read_bytes`/`deserialize_snapshot`, and `recovery_kit::from_printable_string`'s minimum-length check (`<`→`<=`/`==`, off-by-one at a format boundary). These are the bulk of the 56 timeouts too: `cargo-mutants` hangs on corrupted length/offset fields in the binary snapshot parser rather than panicking cleanly — real, cheap-to-close gaps in adversarial-input fuzz coverage for a pure parsing function (no I/O, no lock, no crypto), not a platform limitation and not a hidden correctness defect. **Real, currently untested, not closed in this pass — consistent with A2's own choice not to close its Group B in a documentation-only task.**

**Group C — distinct logic gaps, not I/O-related (2 of 131).** `deletion_log.rs:664` (`row.generation == r.generation + 1 && row.state == Pending`, `&&`→`||`): traced against the surrounding `if`/`else if` chain — the equal-generation case is already excluded by the preceding branch, but a `Pending` row at `generation + 2` (reachable only via log corruption/tampering, which the surrounding logic already treats as adversarial) is a genuinely distinct, currently-untested combination the real `&&` correctly rejects (falling through to the fail-closed `else`) while the mutant's `||` would incorrectly roll forward at the wrong generation. `deletion_log.rs:682` (`row.generation < r.generation`, the final catch-all fail-closed guard): the `==`/`>`/`<=` variants are each individually reachable only through states the preceding branches in the same chain have already excluded, making this lower-severity than the first but still a real combinatorial gap. **Both real, narrow, not a live exploit under normal operation, not closed in this pass** — flagged here rather than silently absorbed, matching A2's Group C precedent.

**Semantically-equivalent / scaffolding-only (2 of 131).** `deletion_log.rs:359` (`FaultInjector::none()`'s literal struct construction vs. a derived `Default` — bit-identical, confirmed by the mutant compiling and surviving rather than failing to compile). `deletion_log.rs:48`-adjacent (`DeletionLogState::from_sql`'s match-arm deletion, TIMEOUT): a `cargo-mutants` harness artifact from deleting a match arm's exhaustiveness, not a real missed behavioral difference — same class as A2's `fault.rs:115` scaffolding timeout.

**Security-relevant — found, then CLOSED in this task (not merely documented).** Two mutants in this run exposed a real property gap distinct from all the groups above: a mutation surviving does not by itself mean the code is wrong, but it does mean *no automated check would catch a regression that broke it*. Given this plan's own extraordinary effort to get PII-related correctness right by construction (three fix rounds on Task 6 alone), leaving these two unclosed was judged inconsistent with that standard, so both were fixed as part of this task rather than deferred:

1. **`pii_store.rs:205`, `fresh_nonce()` collapsed to a fixed `[0; NONCE_LEN]`.** No prior test proved two separate calls produce different nonce bytes (every existing round-trip test encrypts once per key before decrypting). AES-GCM nonce reuse under one key is a genuine, catastrophic AEAD failure (breaks both confidentiality and the authentication tag), not a hypothetical. **Closed by commit `988cdca`**: `fresh_nonce_never_repeats_across_many_draws` draws 50 nonces and asserts uniqueness (collision probability ≈2⁻⁹⁶ per pair across a 12-byte CSPRNG space — not flaky).
2. **`recovery_kit.rs:76`, `argon2id_params()` collapsed to `Default::default()`.** No prior test distinguished the documented OWASP-floor parameters (64 MiB / 3 iterations / parallelism 1) from the `argon2` crate's own, much weaker default — every existing test only checked that derivation round-trips, which passes under any consistent parameter set. A silent regression here would quietly weaken the recovery kit's entire brute-force-resistance threat model. **Closed by commit `988cdca`**: `argon2id_params_match_the_documented_owasp_floor_values` pins `m_cost()==65536`, `t_cost()==3`, `p_cost()==1` exactly.

One further, lower-confidence finding was flagged but not closed: `keys.rs:308`, `copy_blob`'s `blob.pbData.is_null() || blob.cbData == 0` (`||`→`&&`), inside the real (non-stub) Windows DPAPI code touching `unsafe { slice::from_raw_parts(...) }`. Flipping to `&&` would skip the early-return-empty-Vec guard on a null-pointer-with-nonzero-length (or the reverse) blob and attempt a raw-pointer read on garbage data. DPAPI's own success contract should never produce such an inconsistent blob, so this is judged lower-urgency than the two closed above — but it touches raw pointer safety adjacent to `unsafe`, so it is recorded here rather than silently grouped with the ordinary I/O-guard survivors, as a candidate for a defensive test in a future hardening pass.

**Tally check (corrected).** The first version of this file claimed "111 of 131 accounted for precisely". That was not accurate: the group counts were approximate, and four survivors were mis-grouped. The exact figures are in the §3.4 table: 131 survivors originally; 129 in the full post-fix run (9 closed, 7 new); 123 at `45da6de` (121 carried over, which is 131 minus the 9 closed and minus 1 more killed by `45da6de`, plus 2 new equivalent mutants). Every carried-over survivor belongs to one of the groups above. No survivor in any group is a known production correctness defect as of `45da6de`; the Group B ones are real untested paths, as stated.

---

## 4. What A3 now guarantees

### 4.1 Encrypted store for `Sensitive` notes, gate stays unconditionally protective

`policy::Gate::evaluate` keeps its full authority to reject `Sensitive` content at all times (`crates/policy/src/gate.rs`); A3 makes that rejection *conditional on store availability* (`EncryptedStoreAvailability`, a plain, zero-I/O enum in `vault-security`), never bypasses or weakens it. `Secret` classification remains intercepted before a `NoteMetadata` ever exists, entirely untouched by this change — independently verified by tracing the `Available` branch's control flow line-by-line: there is no early return anywhere that skips the shared secret-content scan.

When the store is available, `storage::propose_and_commit`'s sensitive branch routes the note's plaintext through `PiiStore::stage`/`commit_staged`, leaving only a `pii://<note_id>` pointer plus gate-accepted frontmatter in the Markdown record — confirmed, by a dedicated git-history scan test, to never appear in any git blob reachable from any ref.

### 4.2 One crash-recoverable operation per sensitive write — not two independently-recovered stores

This was the single point of architectural correction the user required before approving implementation, and the single hardest-won guarantee in this plan. `write_sensitive_record_unchecked` shares the *same* A2 journal, the *same* `WriteState` machine, the *same* per-note lock, and the *same* `operation_id` row as the ordinary write path — a sibling Rust function (to avoid touching ~90 existing call sites), not a parallel crash-recovery domain. The PII ciphertext's hash (`pii_content_hash`) is recorded on the exact same journal write, at the exact same pipeline point, as the pointer's own `content_hash`, mirroring the pre-existing "record intent before rename" discipline.

**This guarantee required three fix rounds on Task 6 and two on Task 7** before an independent reviewer, writing and running their own repro and mutation tests at every round (not just reading the implementer's claims), confirmed it sound. The specific defect chain, fully closed as of `695bf2c`:

- Round 1 found and fixed the original Critical bug: `PiiStore::commit_staged` overwrote the only copy of a prior operation's ciphertext before the pointer rename, with no before-image — a crash mid-update could permanently lose committed content. Fixed with a PII before-image mechanism mirroring the existing Markdown one.
- Round 2 found that round 1's fix left orphaned before-images on every successful roll-forward path — meaning a *deleted* sensitive note's real content could survive its own deletion indefinitely, still decryptable. Fixed by deleting the PII before-image on the same `GitCommitted` advance the Markdown one already uses.
- Round 3 found that round 2's own fix introduced a new variant of the same root-cause pattern (an I/O-read error on the live file treated as positive proof it was "safe to delete") — rewrote the rollback logic around an explicit, total `LivePii` decision table (`Ours`/`Other(hash)`/`Absent`/`Undecryptable`) where every destructive action requires positive verification, and self-audited 7 more instances of the same mistake across every before-image-touching function. The final re-review wrote its own repro and mutation tests against both the fixed and unfixed trees, confirmed every prior failure reproduces on the unfixed tree and is fixed on the current one, and explicitly stated the mechanism is sound with no further round needed.

### 4.3 Authenticated, hash-chained deletion log (C5)

`DeletionLog` implements C5's full two-step pending/committed protocol with its own crash-recovery state machine (`DeletionLogState`, `Pending → Completed` only) and `FaultPoint`s, its own SQLite table separate from the A2 journal. The trusted reference is DPAPI-wrapped via the *same* raw `dpapi_protect`/`dpapi_unprotect` primitives `MasterKey` uses (confirmed via direct call-site tracing, not reimplemented), with a tamper-vs-corruption distinction (`ReferenceTamperDetected` vs. `ReferenceCorrupt`). `reconcile`'s roll-forward logic was end-to-end tested for both the first-deletion-ever case and the general, later-generation case (the latter added after an independent reviewer confirmed the general branch, though logically correct, was never actually exercised by any test).

**Corrected by the final-review fix wave.** The first version of this section overstated two things. (a) "Hash-chained" was computed but never verified end to end: only the latest row was compared against the trusted reference, so a corrupted middle row (for example a rewritten `note_id`, which silently un-deletes that note) went undetected (I5). `DeletionLog::verify_chain` now walks every row from genesis (contiguous generations, each `prev_chain_hash` linked to its predecessor, each `chain_hash` recomputed) and fails `ChainBroken`; it runs inside `reconcile` and inside `verify_safe_to_expose_after_restore`. (b) `reconcile` was only ever called from tests, so in production a `Pending` deletion was never completed (I3). It now runs inside `Vault::open_with_pii_store`, before the journal's own reconcile, and any unexplained log/reference disagreement fails the open closed (`StorageError::DeletionLog`).

### 4.4 Encrypted backup/restore, gated on the live deletion log, never just the backup's own snapshot

`backup()`/`restore()` against an abstract `BackupTarget`, encrypted under an HKDF-derived subkey of the master key (independent of both the raw master key and every `PiiStore` per-record data key). `restore()` performs two checks before writing anything: the backup's own embedded `GenerationRef` snapshot against the live trusted reference (check 1), and a per-note cross-check against the live deletion log (check 2).

**Corrected by the final-review fix wave (C1, Critical).** As first shipped, check 2 rejected the WHOLE restore if any note in the snapshot was deleted. Because a deletion is a tombstone write, every backup taken after a deletion names the deleted note, so a vault that had ever deleted one sensitive note could never be restored again, while `backup()` kept reporting success. Now, once check 1 passes, every note the live log reports deleted is **excluded** (neither its record nor its ciphertext is written) and named in `RestoreOutcome::excluded_deleted_note_ids`. Exclusion is unconditional for a deleted id, deliberately not "only when the backed-up copy is the tombstone": `record_deletion` runs before the tombstone ciphertext replaces the real one, so a backup taken in that window has a current reference yet holds the real content. When check 1 fails and a deleted note is present, the refusal is still the specific `DeletedSinceBackup { note_ids }`.

**Corrected by the final-review fix wave (I1).** A restored directory was byte-identical but unreadable: `Vault::read_note` resolves through the A2 journal, which the backup does not contain, so every note read `NotFound`. `storage::restore_vault_from_backup` now restores and then seeds a fresh journal from the restored records (each at its own version and hash, resting at `JournalCommitted`, since the restored root has no git history) and opens the result; it refuses an existing journal path. A test reopens the restored root with the ordinary `Vault::open_with_pii_store` and reads a plain and a sensitive note back through `read_note`. A narrow, explicitly-scoped exception lets a never-deleted vault be restored at all (both the backup's embedded reference and the live log must positively show "no deletion has ever occurred" — any read error on either side, or a log with any row history, still fails closed).

This guarantee also required two fix rounds: round 1 found and fixed `restore()` silently dropping all PII ciphertext while reporting success (confirmed by repro: restored notes left as dangling pointers with no content behind them) plus three Important findings (the sentinel exception not checking the live log's own row history; a confidentiality test that could never fail even with the encryption removed; an untested pending-deletion-during-restore combination). Round 2 found and fixed a issue round 1's own fix introduced: `restore()` could silently overwrite newer live sensitive content with no guard and no atomicity — fixed with an existence check (mirroring the vault side's own `AlreadyExists`) and atomic stage-then-rename writes, re-verified by the same reviewer with an independent repro and mutation test before being accepted.

### 4.5 Confidentiality boundary, explicitly enumerated

Six surfaces audited with dedicated tests: git history, the A2 SQLite journal's raw columns, before-images, error/log `Debug`/`Display` output, temp/generated files under an interrupted write, and raw backup-blob bytes (strengthened mid-plan after a reviewer showed the original assertion would have passed even with the backup's encryption layer removed entirely). Five of C3's ten listed surfaces (indici, embedding, riassunti, allegati, cache) are explicitly deferred with a stated reason — no such subsystem exists yet in this codebase — recorded as a Continuity open item ([n2io]) for B1/B2/C2 to re-audit once those subsystems exist, rather than silently dropped.

### 4.6 SEC-00x status: neither closed nor entrenched

A3 does not touch `storage::vault::record_path_checked`, `ensure_resolves_inside_root`, or `atomic_write`. A drift oracle (a non-executing comment anchor, not a test requiring the vulnerability to persist — reframed per the user's explicit correction) names the current Continuity-recorded status. Confirmed by inspection: `git diff fc5171a..988cdca -- crates/storage/src/vault.rs` touches only the A3 pipeline extension (new `FaultPoint` checks, the `pii_content_hash` branch in `reconcile`'s companion module) and the Task 0 anchor; the three named functions' bodies are byte-identical to `fc5171a`. **Re-checked at `7f373c5`** after the fix wave (which added `refuse_if_deleted`, the deletion-log reconcile hook and `pub(crate)` on `parse_record` in `vault.rs`): the bodies of `record_path_checked` and `ensure_resolves_inside_root` and the whole of `atomic.rs` still hash identically to `fc5171a`. The new `restore.rs` calls `record_path_checked` to read restored records, so it inherits the same SEC-00x path-resolution properties as every other caller, no better and no worse.

- **SEC-001 (TOCTOU/symlink race):** `OPEN, UNCHANGED`.
- **SEC-002 (intra-vault symlink aliasing):** `NARROWED (read path only); write-path pre-read unchanged`.
- **SEC-003 (hard-link aliasing):** `NARROWED (read path only); write-path pre-read and outward exposure unchanged`.

All three remain due for mandatory re-evaluation again before Fase D, per the standing Continuity requirement — A3 was neither of the checkpoints that re-evaluation is due at.

### 4.7 Final whole-branch review: what the fix wave changed

The final whole-branch review (`fc5171a..439237b`) found that the first version of this file claimed more than the code delivered. One bundled fix wave closed every load-bearing finding; full RED/GREEN evidence is in `.superpowers/sdd/2026-10-28-a3-security-durability/task-8-final-review-fix-report.md`.

| Finding | Fix | Commit |
|---|---|---|
| C1 (Critical): one deletion made backup/restore permanently unusable | `restore()` excludes and reports deleted notes (`excluded_deleted_note_ids`) once check 1 passes | `4437772` |
| I1: restored vault unreadable through `Vault::read_note` | `storage::restore_vault_from_backup` seeds a fresh journal from the restored records | `d7e43e7` |
| I2: deleted ids could be re-written; no live read consulted the log | `StorageError::NoteDeleted` (deleted ids retired, checked under the per-note lock on both write paths); `PiiStore::get_unless_deleted` | `0cca592` |
| I3: `DeletionLog::reconcile` never called in production | runs inside `Vault::open_with_pii_store`, fails the open closed on mismatch | `cdf6dc0` |
| I4: four cheap mutation survivors mis-triaged | known-answer / dependence tests for the backup subkey, backup nonce, chain hash and PII fingerprint | `7f373c5`, `0cca592`, `45da6de` |
| I5: chain never verified end to end | `DeletionLog::verify_chain`, run by `reconcile` and the restore gate | `7f373c5`, `45da6de` |

---

## 5. What A3 still does NOT guarantee

- **No general-purpose note-deletion API.** `storage::propose_and_commit` remains the workspace's only public write entry point; a sensitive note's deletion is modeled as a tombstone write through that one call, detected only after the gate passes, driving `DeletionLog::record_deletion` before the tombstone pointer commits. This is deliberate (the plan's resolution to point 7 of the user's required corrections), not an oversight — a general delete API for non-sensitive notes is out of A3's scope entirely.
- **Deletion-log write is not journaled against later pipeline failures (narrowed by the fix wave, not closed).** `record_deletion` runs inside the write's lock-and-version-check, but if a LATER step of the same tombstone write fails or crashes, the old ciphertext stays on disk (still decryptable by the live master key) while the log already says "deleted". Since the fix wave (I2) it is no longer *consultable*: `PiiStore::get_unless_deleted` refuses any note with a log entry in any state, and new content can no longer be written under the id. Physical removal still needs the tombstone write to be re-issued (accepted for an already-deleted id, without a second log entry); nothing does that automatically. Making `DeletionLog`'s write recoverable from the A2 journal remains a design question for a dedicated task.
- **Only `PiiStore::get_unless_deleted` consults the deletion log on reads.** `PiiStore::get` is the documented low-level primitive (recovery, rollback, restore verification) and does not; a caller that serves content through `get` bypasses the check. `Vault::read_note` on a deleted sensitive note returns its tombstone pointer record (frontmatter plus `pii://<id>`, never content), not an error.
- **The retired-id check on a non-sensitive write needs a configured `PiiStore`.** `propose_and_commit(..., pii_store: None)` cannot see the deletion log (it lives beside the store), so an `Internal`/`Public` write to a deleted sensitive id is refused only when the caller passes the store, as production callers that ever wrote sensitive notes must.
- **A backup older than the latest deletion is still rejected as a whole** (check 1, `ReferenceStale`, or `DeletedSinceBackup` when the snapshot names a deleted note), even for notes that were never deleted. This is Task 5b's fail-closed design, deliberately left unchanged by the fix wave: C1's exclusion would make such a backup safely restorable minus its deleted notes ("un restore applica prima le cancellazioni più recenti"), but loosening check 1 is a decision for the plan owner, not a fix-wave side effect. Practical consequence: after any deletion, only backups taken after it are restorable, so take a fresh backup after deleting.
- **A restored vault has no git history and its notes rest at `JournalCommitted` (`git_pending`).** `restore_vault_from_backup` seeds the journal from the restored records; it does not restore `.git` (never backed up). Each restored note is committed only once the restored root is a git repository with a first commit and the vault is reopened.
- **The restored vault uses whatever deletion log lives beside the `PiiStore` it is opened with.** In the intended flow that is the live store and its live log. A disaster-recovery restore into a fresh `PiiStore` root on a new machine starts with an empty log there, unless the live deletion log and its trusted reference are carried over separately (by design they are never in the backup, C5: "conservato separatamente dai backup"). No tooling or runbook step does this yet.
- **No key escrow for backup/restore.** A restored `.pii` file only decrypts under the ORIGINAL master key (via its recovery kit or the original DPAPI context). If that key is permanently lost, `PiiStore::open` correctly refuses to mint a fresh one over a root that already holds ciphertext — but `backup()` does not itself escrow or re-derive the key, so losing it makes restored ciphertext as unrecoverable as live ciphertext would have been. A real disaster-recovery limitation, not a defect, and not newly introduced by A3 — the master key's own recovery kit is the intended mitigation, used independently of the backup mechanism.
- **A note reclassified `sensitive → plain → sensitive` can permanently block `Vault::open` after an ordinary crash.** Found during Task 6's review round, confirmed pre-existing (reproduces identically before and after the round-3 fix), logged as Continuity risk [rk1ai] for a dedicated follow-up task. No data loss; needs an operator to resolve.
- **Physical backup disk selection, key-rotation runbook, non-Windows platform support** remain explicitly out of scope, as stated in the plan's own Explicit Scope Boundaries.
- **No index/embedding/summary confidentiality audit** (5 of C3's 10 surfaces) — deferred because those subsystems don't exist yet; tracked as Continuity open item [n2io].
- **SEC-001/002/003 remain exactly as before** — see §4.6.
- **Fase B/C/D** (search, memory-with-evidence, coding agent) are entirely untouched.

**Minor findings from the final whole-branch review, recorded here and not fixed in code** (the plan's Global Constraints require them flagged, not closed in this wave):

- No AEAD associated data binds a `.pii` ciphertext to its `note_id`: a `.pii` file swapped onto another note's path decrypts silently as that note's content.
- YAML comments in a sensitive note's frontmatter are committed to git verbatim (only the body is moved into `PiiStore`).
- Reclassifying a note `Sensitive` -> `Internal` leaves the stale live `.pii` ciphertext of its last sensitive version in place.
- `restore()` has a check-then-act race against a concurrent `record_deletion` (and against a concurrent live PII write between the `PiiAlreadyExists` check and the rename); nothing prevents running restore concurrently with live writes.
- Names decrypted from a backup snapshot are not checked for path traversal (`..`, absolute paths) before being joined onto `destination_root`/`pii_root`; the snapshot is AEAD-authenticated under the backup key, so this needs that key.
- A partial restore failure can leave the destination (and the PII root) partially populated; there is no cleanup. `restore_vault_from_backup` removes only the journal it created.
- `journal.rs` swallows every error from its additive `ALTER TABLE ... ADD COLUMN pii_content_hash` migration, not just "duplicate column".
- `backup()`'s `MasterKey` parameter is unrelated to `PiiStore`'s own master key, and nothing ties the two together for disaster recovery: both must be preserved (recovery kit) independently.
- `.pii-store` has no `.gitignore` entry or pre-commit guard, so an operator's own `git add -A` at the vault root would commit ciphertext (and the deletion-log files) to git.
- `collect_vault_files`'s dot-entry skip (`||` -> `&&` mutant) is untested: a regression could pull dot-directories, including a nested `.pii-store` with the deletion log, into backups.
- `PiiStore` does not validate the `note_id` it is given (callers in `storage` pass a validated `NoteId`).
- `MasterKey::load_or_create` writes the new key file non-atomically.
- `DpapiTrustedReferenceStore::write` maps a DPAPI protect failure to `ReferenceTamperDetected`, which is semantically imprecise; it should be a distinct I/O-class error.

---

## 6. Rulings made on the plan owner's behalf during implementation

Collected here per the SDD process's own requirement — every decision made without stopping to ask, so the user can rework any of them:

1. **Task 2's raw DPAPI primitive is `pub(crate)`-exposed** (`dpapi_protect`/`dpapi_unprotect`, operating on arbitrary bytes) specifically so Task 5's `DpapiTrustedReferenceStore` could reuse it rather than reimplementing the FFI call, closing a gap the plan's own prose described ("reuse the same DPAPI call") without naming a concrete function. Cost if wrong: trivial — a minor duplication, not a correctness risk.
2. **The plan document's own Task 3 text had stale cross-references** (leftover from an earlier renumbering pass that updated headings but not internal prose) — fixed at the plan-document level, not just in the implementer's copy, so no later task inherits the same stale numbers. Also corrected the plan's own miscount of C3's surface list (nine claimed, ten actual).
3. **Task 6's sibling-function design** (a separate `write_sensitive_record_unchecked` rather than a conditional branch inside the existing `write_record_unchecked`) was accepted as implemented rather than redirected back to the brief's literal wording, after directly verifying it shares every crash-recovery guarantee the brief required (same journal, lock, state machine, operation row) — confirmed independently by three separate review rounds, not taken on faith.
4. **Task 7's deletion-log write ordering issue** (the item in §5 above) was judged a genuine design question rather than guessed at during the fix rounds, and deferred to a dedicated future task rather than rushed into A3's scope.
5. **Two security-relevant mutation-testing gaps** (nonce freshness, Argon2id parameter strength) were judged worth closing within Task 8 itself, rather than merely documented and deferred, given the proportional effort already invested in getting these properties right elsewhere in the plan — a narrow scope expansion of Task 8's "evidence only" framing, justified by cost (two small tests) against the risk of leaving an unguarded regression path on a load-bearing cryptographic property.

---

6. **C1, exclude-and-report (Option B) rather than tombstone detection (Option A).** Telling a backed-up tombstone apart from backed-up real content is not safely decidable inside `restore()`: the tombstone record has the same `pii://<id>` shape, only its decrypted (empty) plaintext differs, restore never decrypts `.pii` files, and `record_deletion` runs before the tombstone ciphertext lands, so a backup taken in that window has a current reference yet real content. Excluding every deleted id is correct in all cases. Cost if wrong: none to confidentiality; a deleted note is simply never restored, which is the intent.
7. **Check 1 left unchanged.** With exclusion in place, check 1's wholesale rejection of a backup older than the latest deletion is stricter than C5 requires, but it is Task 5b's reviewed fail-closed design and loosening it is the plan owner's call (§5). Cost if wrong: older backups stay unusable after any deletion until the owner decides.
8. **I1, rebuild the journal rather than back it up.** Reasons in `restore.rs`'s module docs and §4.4. `restore_vault_from_backup` is public API but not a second write path: it only bootstraps a brand-new vault (refuses an existing journal and a non-empty destination) from an authenticated backup, and `integration.rs`'s surface guard pins it. Cost if wrong: a reviewer who reads it as a second write path would want it gated differently; the function is small and self-contained.
9. **I2, deleted ids are retired (Option A) rather than reusable (Option B).** Matches the append-only, monotonic deletion log; Option B would need a "deleted then recreated" concept in `is_deleted`. A tombstone write to an already-deleted id is still accepted (to finish a half-failed deletion) without a second log entry. Cost if wrong: a user who wants to reuse an id must pick a new one.
10. **I3, a deletion-log mismatch fails `Vault::open_with_pii_store` closed**, for non-sensitive work too, mirroring the A2 journal's own reconcile. Cost if wrong: an operator must repair the log before the vault opens at all; there is no read-only "reconciliation mode" yet.

## 7. Next step

The first version of this section said the branch was ready for the final whole-branch review. That review found C1 and I1–I5 (§4.7); the bundled fix wave (`4437772`..`45da6de`) and this correction of the file are now done. Per the SDD process the branch gets exactly one scoped re-review of that wave; nothing is merged to `main` and no `a3-complete` tag is created until the user decides after it.
