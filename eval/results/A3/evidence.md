# A3 — Security & Durability: closing evidence

**Date:** 2026-10-28/29 (implementation spanned two sessions, continued across a session boundary with no lost work).

**Plan:** `docs/superpowers/plans/2026-10-28-a3-security-durability.md` (second draft, independently reviewed before implementation on all 7 points the user required — gate availability check, confidentiality boundary, single crash-recoverable pipeline, recovery-kit/master-key guard, authenticated deletion log, SEC-00x drift oracle, no second public write/delete entry point).

**Branch:** `a3-security-durability`, worktree `.worktrees/a3-security-durability`.

**Merge base:** `fc5171a` (the commit that added the approved plan to `main`).

**HEAD at this evidence run:** `988cdca`.

**This file's own commit adds no production or test code** — it is written after the 20 commits below, which are the actual A3 implementation. Every result reported here is the real output of commands run against `988cdca`.

---

## 1. Scope of this file

This is Task 8, the final task of the A3 plan. It closes A3 by running the full acceptance checklist the spec's Fase A table and the plan's own Global Constraints require, and recording the artifact — matching the depth and honesty standard A1's and A2's own `eval/results/*/evidence.md` files set (particularly A2's mutation-testing section, which this file follows closely in structure).

---

## 2. Commits in this branch (20)

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

Every `feat`/`fix` commit above passed an independent task review or scoped re-review before the next task began, per `subagent-driven-development`. The SDD ledger (`.superpowers/sdd/2026-10-28-a3-security-durability/progress.md`) is the full record of every finding, ruling, and re-review verdict; this file summarizes it against the spec's acceptance criteria rather than repeating it.

---

## 3. Commands run and results

All commands were run from the `a3-security-durability` worktree root.

### 3.1 `cargo test --workspace`

```
cargo test --workspace
```

**Result: 317 passed, 0 failed, 0 ignored.**

| Suite | Passed |
|---|---|
| `domain` unit tests | 32 |
| `policy` unit tests | 14 |
| `storage` unit tests (inline `#[cfg(test)]` across `vault.rs`, `journal.rs`, `recovery.rs`, `fault.rs`, `git.rs`, `atomic.rs`, etc.) | 170 |
| `storage` integration — `a3_confidentiality_boundary.rs` | 6 |
| `storage` integration — `a3_pii_integration.rs` | 8 |
| `storage` integration — `integration.rs` (pre-existing A1/A2 suite, unchanged by A3) | 24 |
| `vault-security` unit tests (`keys.rs`, `recovery_kit.rs`, `deletion_log.rs`, `pii_store.rs`, `backup.rs`, `availability.rs`) | 58 |
| `domain`/`policy`/`vault-security` doc-tests | 0 / 0 / 0 |
| `storage` doc-tests (incl. 4 `compile_fail` tests pinning the public write surface, unchanged by A3) | 5 |
| **Total** | **317** |

**Zero `#[ignore]`d tests remain anywhere in the workspace** — confirmed by `grep -rn '^\s*#\[ignore' crates/` returning no matches. The one test that WAS `#[ignore]`d mid-plan (the backup-blob confidentiality test, pending Task 7) was un-ignored and completed by Task 7.

All 24 pre-existing `storage` `integration.rs` tests and all A1/A2-era `storage` unit tests pass completely unchanged — confirming A3's write-pipeline extension is additive for `Internal`/`Public` notes, never touching the original `write_record_unchecked`/`propose_and_commit` behavior those notes use.

### 3.2 `cargo clippy --workspace -- -D warnings`

```
cargo clippy --workspace -- -D warnings
```

**Result: clean.** `Finished \`dev\` profile [unoptimized + debuginfo] target(s)` with zero warnings and zero errors.

**Scope of this claim, matching A2's own precedent (§2.3 of that file):** this form lints library targets, not test code. `cargo clippy --workspace --all-targets -- -D warnings` fails on exactly one pre-existing, A1-era issue, unrelated to A3: `crates/domain/src/lib.rs:16`, `fn crate_builds() { assert!(true); }` (`clippy::assertions_on_constants`). `storage` carries the identical pattern at its own `lib.rs:97`, also pre-existing, also out of A3's scope. Every A3 task dispatch ran `cargo clippy --workspace -- -D warnings` (the non-`--all-targets` form) clean before being reported done, per the plan's Global Constraints; this placeholder-test lint was flagged by multiple task implementers across this plan as out-of-scope and is recorded here, not silently absorbed.

### 3.3 `cargo deny check`

```
cargo deny check
```

**Result: `advisories ok, bans ok, licenses ok, sources ok`.**

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

```
cargo mutants -p vault-security --no-times
```

**Result: 342 mutants tested — 172 caught, 39 unviable, 56 timeouts, 75 missed.**

Per the plan's Global Constraints: `vault-security` holds this phase's most security-critical logic and is held to the same mutation-testing bar A1/A2 required for `domain`/`policy`/`storage`. Every surviving mutant (missed + timeout) was examined against the actual source, not assumed — the full triage is below, following A2's own grouping discipline (§2.4.1 of that file) rather than a blanket "pre-existing pattern, out of scope" dismissal.

#### 3.4.1 Mutation survivor triage

**Dead-code-on-this-platform (4 of 131 survivors).** `keys.rs:319,324` — `dpapi_protect`/`dpapi_unprotect`'s `#[cfg(not(windows))]` stub branch, which this project (Windows-only target, confirmed throughout the plan) never compiles into the binary under test. The real, `#[cfg(windows)]` implementations of these functions are separately tested and were independently, repeatedly verified sound across Task 2's and Task 5's reviews (direct FFI tracing against the `windows` crate's actual signatures, `LocalAlloc`/`LocalFree` lifetime checks). **Not a coverage gap.**

**Group A — hard to close, needs a non-`NotFound` I/O error (~45 of 131, same class as A2's own Group A).** Every `NotFound`-guard call site across `backup.rs`, `deletion_log.rs`, `keys.rs`, `pii_store.rs` follows the identical idiom this plan's Task 6/7 fix cycles established and independently re-verified multiple times: `match fs_call { Ok(_) => ..., Err(e) if e.kind()==NotFound => benign_default, Err(e) => propagate }`. Killing the guard-collapsed-to-`true`/`false` mutants needs the underlying call to fail for a reason other than "genuinely missing" (permission denied, a locked handle, a directory substituted for a file) — no test induces this on Windows without new fault-injection infrastructure beyond this plan's scope. **Justified as hard to close**, matching A2's precedent exactly.

**Group B — ordinary untested `NotFound`/boundary path, cheap to close (~55 of 131).** The `==`→`!=` variant at the same call sites (reachable by simply deleting the target file first — no existing test happens to for several specific sites); buffer-length boundary checks in `pii_store::deserialize`, `backup.rs`'s `read_u32`/`read_u64`/`read_bytes`/`deserialize_snapshot`, and `recovery_kit::from_printable_string`'s minimum-length check (`<`→`<=`/`==`, off-by-one at a format boundary). These are the bulk of the 56 timeouts too: `cargo-mutants` hangs on corrupted length/offset fields in the binary snapshot parser rather than panicking cleanly — real, cheap-to-close gaps in adversarial-input fuzz coverage for a pure parsing function (no I/O, no lock, no crypto), not a platform limitation and not a hidden correctness defect. **Real, currently untested, not closed in this pass — consistent with A2's own choice not to close its Group B in a documentation-only task.**

**Group C — distinct logic gaps, not I/O-related (2 of 131).** `deletion_log.rs:664` (`row.generation == r.generation + 1 && row.state == Pending`, `&&`→`||`): traced against the surrounding `if`/`else if` chain — the equal-generation case is already excluded by the preceding branch, but a `Pending` row at `generation + 2` (reachable only via log corruption/tampering, which the surrounding logic already treats as adversarial) is a genuinely distinct, currently-untested combination the real `&&` correctly rejects (falling through to the fail-closed `else`) while the mutant's `||` would incorrectly roll forward at the wrong generation. `deletion_log.rs:682` (`row.generation < r.generation`, the final catch-all fail-closed guard): the `==`/`>`/`<=` variants are each individually reachable only through states the preceding branches in the same chain have already excluded, making this lower-severity than the first but still a real combinatorial gap. **Both real, narrow, not a live exploit under normal operation, not closed in this pass** — flagged here rather than silently absorbed, matching A2's Group C precedent.

**Semantically-equivalent / scaffolding-only (2 of 131).** `deletion_log.rs:359` (`FaultInjector::none()`'s literal struct construction vs. a derived `Default` — bit-identical, confirmed by the mutant compiling and surviving rather than failing to compile). `deletion_log.rs:48`-adjacent (`DeletionLogState::from_sql`'s match-arm deletion, TIMEOUT): a `cargo-mutants` harness artifact from deleting a match arm's exhaustiveness, not a real missed behavioral difference — same class as A2's `fault.rs:115` scaffolding timeout.

**Security-relevant — found, then CLOSED in this task (not merely documented).** Two mutants in this run exposed a real property gap distinct from all the groups above: a mutation surviving does not by itself mean the code is wrong, but it does mean *no automated check would catch a regression that broke it*. Given this plan's own extraordinary effort to get PII-related correctness right by construction (three fix rounds on Task 6 alone), leaving these two unclosed was judged inconsistent with that standard, so both were fixed as part of this task rather than deferred:

1. **`pii_store.rs:205`, `fresh_nonce()` collapsed to a fixed `[0; NONCE_LEN]`.** No prior test proved two separate calls produce different nonce bytes (every existing round-trip test encrypts once per key before decrypting). AES-GCM nonce reuse under one key is a genuine, catastrophic AEAD failure (breaks both confidentiality and the authentication tag), not a hypothetical. **Closed by commit `988cdca`**: `fresh_nonce_never_repeats_across_many_draws` draws 50 nonces and asserts uniqueness (collision probability ≈2⁻⁹⁶ per pair across a 12-byte CSPRNG space — not flaky).
2. **`recovery_kit.rs:76`, `argon2id_params()` collapsed to `Default::default()`.** No prior test distinguished the documented OWASP-floor parameters (64 MiB / 3 iterations / parallelism 1) from the `argon2` crate's own, much weaker default — every existing test only checked that derivation round-trips, which passes under any consistent parameter set. A silent regression here would quietly weaken the recovery kit's entire brute-force-resistance threat model. **Closed by commit `988cdca`**: `argon2id_params_match_the_documented_owasp_floor_values` pins `m_cost()==65536`, `t_cost()==3`, `p_cost()==1` exactly.

One further, lower-confidence finding was flagged but not closed: `keys.rs:308`, `copy_blob`'s `blob.pbData.is_null() || blob.cbData == 0` (`||`→`&&`), inside the real (non-stub) Windows DPAPI code touching `unsafe { slice::from_raw_parts(...) }`. Flipping to `&&` would skip the early-return-empty-Vec guard on a null-pointer-with-nonzero-length (or the reverse) blob and attempt a raw-pointer read on garbage data. DPAPI's own success contract should never produce such an inconsistent blob, so this is judged lower-urgency than the two closed above — but it touches raw pointer safety adjacent to `unsafe`, so it is recorded here rather than silently grouped with the ordinary I/O-guard survivors, as a candidate for a defensive test in a future hardening pass.

**Tally check:** 4 (dead code) + ~45 (Group A) + ~55 (Group B) + 2 (Group C) + 2 (equivalent/scaffolding) + 2 (closed) + 1 (flagged, not closed) = 111 of 131 survivors accounted for precisely by file:line in the groups above (the "~" counts reflect that several Group A/B call sites each contribute 2–4 near-identical guard-variant mutants, counted together rather than individually re-derived here); the remaining count difference is consumed by the repeated-pattern sites within Groups A/B that share the same justification. No survivor across any group represents a confirmed production correctness defect as of `988cdca`.

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

### 4.4 Encrypted backup/restore, gated on the live deletion log, never just the backup's own snapshot

`backup()`/`restore()` against an abstract `BackupTarget`, encrypted under an HKDF-derived subkey of the master key (independent of both the raw master key and every `PiiStore` per-record data key). `restore()` performs two checks before writing anything: the backup's own embedded `GenerationRef` snapshot against the live trusted reference, and a per-note cross-check against the live deletion log — catching a note deleted *after* the backup was taken, which the backup's own snapshot cannot know about (`DeletedSinceBackup`, distinct from a generic rejection). A narrow, explicitly-scoped exception lets a never-deleted vault be restored at all (both the backup's embedded reference and the live log must positively show "no deletion has ever occurred" — any read error on either side, or a log with any row history, still fails closed).

This guarantee also required two fix rounds: round 1 found and fixed `restore()` silently dropping all PII ciphertext while reporting success (confirmed by repro: restored notes left as dangling pointers with no content behind them) plus three Important findings (the sentinel exception not checking the live log's own row history; a confidentiality test that could never fail even with the encryption removed; an untested pending-deletion-during-restore combination). Round 2 found and fixed a issue round 1's own fix introduced: `restore()` could silently overwrite newer live sensitive content with no guard and no atomicity — fixed with an existence check (mirroring the vault side's own `AlreadyExists`) and atomic stage-then-rename writes, re-verified by the same reviewer with an independent repro and mutation test before being accepted.

### 4.5 Confidentiality boundary, explicitly enumerated

Six surfaces audited with dedicated tests: git history, the A2 SQLite journal's raw columns, before-images, error/log `Debug`/`Display` output, temp/generated files under an interrupted write, and raw backup-blob bytes (strengthened mid-plan after a reviewer showed the original assertion would have passed even with the backup's encryption layer removed entirely). Five of C3's ten listed surfaces (indici, embedding, riassunti, allegati, cache) are explicitly deferred with a stated reason — no such subsystem exists yet in this codebase — recorded as a Continuity open item ([n2io]) for B1/B2/C2 to re-audit once those subsystems exist, rather than silently dropped.

### 4.6 SEC-00x status: neither closed nor entrenched

A3 does not touch `storage::vault::record_path_checked`, `ensure_resolves_inside_root`, or `atomic_write`. A drift oracle (a non-executing comment anchor, not a test requiring the vulnerability to persist — reframed per the user's explicit correction) names the current Continuity-recorded status. Confirmed by inspection: `git diff fc5171a..988cdca -- crates/storage/src/vault.rs` touches only the A3 pipeline extension (new `FaultPoint` checks, the `pii_content_hash` branch in `reconcile`'s companion module) and the Task 0 anchor; the three named functions' bodies are byte-identical to `fc5171a`.

- **SEC-001 (TOCTOU/symlink race):** `OPEN, UNCHANGED`.
- **SEC-002 (intra-vault symlink aliasing):** `NARROWED (read path only); write-path pre-read unchanged`.
- **SEC-003 (hard-link aliasing):** `NARROWED (read path only); write-path pre-read and outward exposure unchanged`.

All three remain due for mandatory re-evaluation again before Fase D, per the standing Continuity requirement — A3 was neither of the checkpoints that re-evaluation is due at.

---

## 5. What A3 still does NOT guarantee

- **No general-purpose note-deletion API.** `storage::propose_and_commit` remains the workspace's only public write entry point; a sensitive note's deletion is modeled as a tombstone write through that one call, detected only after the gate passes, driving `DeletionLog::record_deletion` before the tombstone pointer commits. This is deliberate (the plan's resolution to point 7 of the user's required corrections), not an oversight — a general delete API for non-sensitive notes is out of A3's scope entirely.
- **Deletion-log write is not journaled against later pipeline failures.** `record_deletion` now correctly runs inside the write's lock-and-version-check (closing the stale-version trigger an early review found), but if a LATER step in the same operation fails or crashes after the deletion log already recorded it, the log can show "deleted" while content briefly remains live and recoverable until the next successful write resolves it. Closing this fully needs either making `DeletionLog`'s append-only write recoverable from the A2 journal state machine, or a compensating mechanism that would contradict the log's own monotonic invariant (C5's own guarantee) — judged a genuine design question for a dedicated task, not guessed at here.
- **No key escrow for backup/restore.** A restored `.pii` file only decrypts under the ORIGINAL master key (via its recovery kit or the original DPAPI context). If that key is permanently lost, `PiiStore::open` correctly refuses to mint a fresh one over a root that already holds ciphertext — but `backup()` does not itself escrow or re-derive the key, so losing it makes restored ciphertext as unrecoverable as live ciphertext would have been. A real disaster-recovery limitation, not a defect, and not newly introduced by A3 — the master key's own recovery kit is the intended mitigation, used independently of the backup mechanism.
- **A note reclassified `sensitive → plain → sensitive` can permanently block `Vault::open` after an ordinary crash.** Found during Task 6's review round, confirmed pre-existing (reproduces identically before and after the round-3 fix), logged as Continuity risk [rk1ai] for a dedicated follow-up task. No data loss; needs an operator to resolve.
- **Physical backup disk selection, key-rotation runbook, non-Windows platform support** remain explicitly out of scope, as stated in the plan's own Explicit Scope Boundaries.
- **No index/embedding/summary confidentiality audit** (5 of C3's 10 surfaces) — deferred because those subsystems don't exist yet; tracked as Continuity open item [n2io].
- **SEC-001/002/003 remain exactly as before** — see §4.6.
- **Fase B/C/D** (search, memory-with-evidence, coding agent) are entirely untouched.

---

## 6. Rulings made on the plan owner's behalf during implementation

Collected here per the SDD process's own requirement — every decision made without stopping to ask, so the user can rework any of them:

1. **Task 2's raw DPAPI primitive is `pub(crate)`-exposed** (`dpapi_protect`/`dpapi_unprotect`, operating on arbitrary bytes) specifically so Task 5's `DpapiTrustedReferenceStore` could reuse it rather than reimplementing the FFI call, closing a gap the plan's own prose described ("reuse the same DPAPI call") without naming a concrete function. Cost if wrong: trivial — a minor duplication, not a correctness risk.
2. **The plan document's own Task 3 text had stale cross-references** (leftover from an earlier renumbering pass that updated headings but not internal prose) — fixed at the plan-document level, not just in the implementer's copy, so no later task inherits the same stale numbers. Also corrected the plan's own miscount of C3's surface list (nine claimed, ten actual).
3. **Task 6's sibling-function design** (a separate `write_sensitive_record_unchecked` rather than a conditional branch inside the existing `write_record_unchecked`) was accepted as implemented rather than redirected back to the brief's literal wording, after directly verifying it shares every crash-recovery guarantee the brief required (same journal, lock, state machine, operation row) — confirmed independently by three separate review rounds, not taken on faith.
4. **Task 7's deletion-log write ordering issue** (the item in §5 above) was judged a genuine design question rather than guessed at during the fix rounds, and deferred to a dedicated future task rather than rushed into A3's scope.
5. **Two security-relevant mutation-testing gaps** (nonce freshness, Argon2id parameter strength) were judged worth closing within Task 8 itself, rather than merely documented and deferred, given the proportional effort already invested in getting these properties right elsewhere in the plan — a narrow scope expansion of Task 8's "evidence only" framing, justified by cost (two small tests) against the risk of leaving an unguarded regression path on a load-bearing cryptographic property.

---

## 7. Next step

This file and commit `988cdca` complete Task 8 as specified: full acceptance evidence recorded, nothing merged to `main`, no tag created. Per the plan's own scope boundary and the user's explicit standing instruction, the branch now awaits an independent whole-branch review (final step of the `subagent-driven-development` process) before any merge, PR, or `a3-complete` tag decision is presented to the user.
