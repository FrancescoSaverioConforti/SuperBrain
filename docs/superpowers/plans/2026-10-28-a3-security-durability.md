# A3 — Security & Durability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the vault an encrypted store for `classification: sensitive` notes, a crash-safe monotonic deletion log that a restore checks before exposing anything, and a verifiable encrypted backup/restore path — the three deliveries the spec lists under Fase A items 5–6, closing contract C5.

**Architecture:** A new `crates/vault-security` crate owns three subsystems that compose through `storage`'s *existing* single write path rather than beside it:

1. A `PiiStore` keyed by per-record data keys wrapped by a master key (DPAPI primary, Argon2id passphrase recovery kit secondary). `Gate::evaluate` keeps its unconditional authority to reject `Sensitive` content — it is not weakened — but gains one new input, an `EncryptedStoreAvailability` check, so the existing `SensitiveRequiresEncryptedStore` rejection becomes *conditional on the store actually being open and reachable for this call*, rather than always firing (today it always fires, because A1 had no store to check against). `Secret` classification is untouched: it is intercepted at parse time, before a `NoteMetadata` even exists, and never reaches this check.
2. A `DeletionLog`: an independent hash-chained, monotonically-numbered SQLite table (separate database file from the A2 journal) implementing C5's two-step pending/committed protocol with its own crash-recovery state machine, modeled the same way A2 modeled `WriteState` (an enum + `valid_transition`), with its own `FaultInjector` points. Its trusted reference is itself protected by the same master-key infrastructure as the `PiiStore` (DPAPI-wrapped, authenticated), never a bare unauthenticated file.
3. `Backup`/`Restore` against an abstract `BackupTarget` (a trait over a directory path — no physical disk decision here), which encrypts with the master key, snapshots the deletion log's trusted reference alongside the data, and on restore applies C5's reconciliation rule — checked against the *live* deletion log, not the backup's own embedded snapshot — before exposing anything.

A sensitive-note write is **one operation**, tracked by the *existing* A2 journal and `WriteState` machine, extended with one conditional step (persist-to-`PiiStore`) that sits inside the same `operation_id`-keyed row `write_record_unchecked` already drives — the same way the git-commit step is already a conditional extension of that same pipeline today. It is not two independently-recovered transactions racing each other. Task 6 designs this extension and its crash-recovery rows explicitly, including the update-to-an-already-sensitive-note case.

None of this subsumes SEC-001/SEC-002/SEC-003. Those are living, open symlink/hard-link/TOCTOU defects in `storage`'s path-resolution layer (`record_path_checked`, `ensure_resolves_inside_root`) that operate on *paths*, independent of whether the bytes behind a path are encrypted. A3 adds a new, separate ciphertext store; it does not touch `record_path_checked`. Task 0 adds a drift-detecting oracle over their current, Continuity-recorded status — not a requirement to preserve them, and not a claim that A3 fixes them either.

**Tech Stack:** Rust workspace, new crate `vault-security` (members: `domain`, `policy`, `storage`, `vault-security`). `aes-gcm` (AEAD for data-key-wrapped record ciphertext and backup blobs), `argon2` (recovery-kit passphrase KDF), `zeroize` (key material), `windows` crate's `Win32::Security::Cryptography` DPAPI bindings (`CryptProtectData`/`CryptUnprotectData`) gated behind `#[cfg(windows)]` — this MVP targets the user's Windows laptop only; a non-Windows build fails fast with a named `UnsupportedPlatform` error rather than silently degrading. `rusqlite` (already a workspace dependency via `storage`) for the deletion-log database. `sha2`, `uuid`, `thiserror`, `serde`/`serde_yaml` are already in the workspace and reused as-is.

**Spec:** `docs/Spec_mvp.md` — Fase A, consegne 5–6; criteri di accettazione "Restore da backup" and "Cancellazione dopo restore"; contratto C5; contratto C3 (classification boundary, already enforced by A1's gate and schema).

## Global Constraints

- Perimeter is frozen per `docs/Spec_mvp.md` header: one user, one agent, one model, one active project — no multi-tenant key material, no per-user ACLs.
- Only SPDX-allowlisted licenses (Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC, Zlib); every new crate dependency is checked against this list and against `cargo deny` before being added, with any exception motivated and time-bound exactly as A1/A2 did for `unsafe-libyaml` (RUSTSEC-2024-0449, [d5q0]).
- `storage::propose_and_commit` remains the only public write entry point in the workspace ([c1p2]); A3 does not add a second one, and does not add a public delete entry point either. C5 governs the trustworthiness of a deletion *log*, not the shape of a deletion API, and the spec never asks for a general-purpose delete function — see Task 6's design note on why deletion of a sensitive note is modeled as a `propose_and_commit` call (a new write that supersedes the record with a tombstone/removal marker and drives the deletion log from inside the same gated call), not a new public function.
- `Gate::evaluate`'s rejection of `Sensitive` classification stays in force at all times; A3 makes it *conditional on store availability*, never removes or bypasses it. A `Sensitive` note submitted while the store is unavailable (not yet opened, key unreachable, an orphan-scan found an unrecoverable inconsistency) is rejected exactly as today, with the same `GateRejection::SensitiveRequiresEncryptedStore`.
- `secret` classification stays permanently rejected before a `NoteMetadata` ever exists (A1's `evaluate_schema_error` path); A3 never builds a store for it and the new availability check never runs for it.
- `sensitive` and `secret` data never enter git-tracked paths (C3) — this is a hard invariant the plan tests across every surface (git, journal, before-images, logs, backups — see Task 3), not a target to approach.
- Every SDD task dispatch must run `cargo clippy --workspace -- -D warnings` before reporting DONE ([c11mp]).
- `cargo deny check` must stay clean for the whole workspace at the end of every task that touches `Cargo.toml`/`Cargo.lock`.
- A1's acceptance criteria named `domain`, `policy`, `storage` for mutation testing because those were the only crates that existed. `vault-security` holds this phase's most security-critical logic (key wrapping, AEAD usage, the deletion-log state machine) and is held to the same bar: Task 9 runs `cargo mutants` against it and every surviving mutant is examined and either justified in the evidence file or covered by a new test, exactly as A1/A2 required for their crates.
- SEC-001/SEC-002/SEC-003 stay exactly as recorded in Continuity ([d5b9], [c16c6], [c17e8]) — A3 neither fixes nor entrenches them. Task 0 pins their *current documented status* as a drift oracle, so a status change is caught and flagged for a deliberate decision, never silently absorbed as "expected" by this plan.
- No new architectural revision to `docs/Spec_mvp.md`; this plan implements the frozen spec and amends it only from implementation/test evidence (per the spec's own header rule). Any genuine spec gap found during implementation is reported to the user as a decision point, never resolved by editing the spec unilaterally.

## Review Focus

- **Master key loss with no recovery kit generated yet, and the inverse failure: silent re-keying over an existing, now-undecryptable store.** A user who loses the DPAPI-protected key material (new Windows profile, `CryptUnprotectData` failing) must get a named, closed-form error, never a silent `Inconsistent` and never a crash — and symmetrically, `load_or_create` must never mint a *fresh* master key when a `PiiStore` with existing ciphertext is present but undecryptable: that would silently orphan every existing sensitive note behind a key that can no longer unwrap them, while reporting success. Task 2 pins both directions, and Task 2 also proves the recovery kit restores the *same* key DPAPI wraps (a cross-machine restore test, not just a round-trip on one machine).
- **Sensitive-note write crashes between ciphertext persisted to `PiiStore` and the Markdown pointer becoming visible — including on an UPDATE to an already-sensitive note, not just a first write.** This is the A2 crash matrix's "dopo il rename, prima di journal_committed" row, extended with a new intermediate step; a naive implementation that only wires the happy path leaves a window where the journal says `JournalCommitted` (pointer visible, readable via the API) but the `PiiStore` either has no ciphertext yet, or — on an update — still has the *previous* version's ciphertext under the pointer. Task 6 pins this with new `FaultPoint`s in the same idiom as A2, and a dedicated update-path test, not just a create-path one.
- **Restore reads a syntactically-valid but stale trusted reference (lower generation than the log).** The spec explicitly separates "corrupt" from "valid but obsolete" (`docs/Spec_mvp.md` C5, test bullet "registro di cancellazione valido ma obsoleto rispetto al riferimento"); a restore implementation that only checks hash-validity and skips the generation-ordering comparison would pass a naive test suite while violating the actual requirement. Task 5 pins this as its own case, distinct from "reference missing" and "reference corrupt."
- **A note is deleted AFTER a backup was taken, and that backup is later restored.** The spec's deletion guarantee must survive a restore from a backup that predates the deletion (`docs/Spec_mvp.md` line 131, "Restore di un backup precedente a una cancellazione: il dato non riappare"); this is specifically about `restore()` consulting the *current, live* deletion log (not the backup's embedded snapshot) before exposing anything, and failing closed if the current log cannot be reached. Task 8 pins this as its own test, distinct from the plain round-trip test and from Task 6's orphan-ciphertext check.
- **Confidentiality leaks through a side channel other than the Markdown record itself** — git blob history (including the before-image dotdir and any stray staged-but-uncommitted blob), the A2 SQLite journal's columns, `Debug`-formatted error messages that might echo plaintext or key material, OS temp files from an interrupted write, and the backup blob format. The round-trip tests elsewhere only prove the *intended* read path never returns plaintext; they do not by themselves prove no *other* path does. Task 3 is a dedicated audit across exactly these surfaces.

---

## File Structure

- `Cargo.toml` (workspace root) — add `crates/vault-security` to `members`.
- `crates/vault-security/Cargo.toml` — new crate; depends on `domain`, `policy`; dev-depends on nothing extra beyond what tests need.
- `crates/vault-security/src/lib.rs` — crate docs (mirrors `storage`'s public-surface doc pattern), module declarations, `pub use` re-exports only.
- `crates/vault-security/src/keys.rs` — `MasterKey`, `WrappedKey`, `KeySource` (DPAPI / RecoveryKit), wrap/unwrap, zeroization, and the existing-undecryptable-store guard (Task 2).
- `crates/vault-security/src/recovery_kit.rs` — Argon2id passphrase-derived recovery key generation/verification, independent of DPAPI, written once and stored offline per C5 ("riferimento affidabile ... indipendente dai backup").
- `crates/vault-security/src/pii_store.rs` — `PiiStore`: per-record AEAD encryption, `pii://<note_id>` pointer scheme, orphan-detection helper, and the on-disk shape that Task 6's crash-recovery extension depends on (a durable, hash-verifiable "staged" ciphertext write distinct from the "committed" one — see Task 6).
- `crates/vault-security/src/deletion_log.rs` — `DeletionLog`, `DeletionLogEntry`, `GenerationRef` (the "riferimento affidabile," itself DPAPI-protected via `keys.rs`), the pending/committed state machine, `FaultPoint` variants local to this crate (mirroring `storage::fault`'s idiom, not reusing its private type).
- `crates/vault-security/src/backup.rs` — `BackupTarget` trait, `FilesystemBackupTarget` (test/dev impl: a directory), `backup()`, `restore()`.
- `crates/storage/tests/a3_*.rs` — integration tests that need both `storage::Vault` and `vault-security` together (Tasks 3, 6, 7, 8 need this; kept in `storage`'s own `tests/` dir, not `vault-security`'s, since they exercise `storage::propose_and_commit`'s behavior and `vault-security` must not depend back on `storage`).
- Modify: `crates/policy/src/gate.rs` — `Gate::evaluate` gains the `EncryptedStoreAvailability` parameter (Task 4); `GateRejection` is otherwise unchanged.
- Modify: `crates/storage/src/propose.rs` and `crates/storage/src/vault.rs` — the conditional `PiiStore` step inside the existing write pipeline (Task 6); **this requires `storage` to depend on `vault-security`**, a one-direction dependency (`vault-security` never depends on `storage`), avoiding any cycle: `domain -> policy -> vault-security`, and separately `domain -> policy -> storage -> vault-security`.

## Task 0: Pin the current SEC-00x status as a drift oracle (no fix, no entrenchment)

**Files:**
- Test: `crates/storage/src/vault.rs` (append to existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: existing `sec_001_toctou_window_between_path_check_and_write_still_exists` and the SEC-002/SEC-003 fixtures already in `vault.rs` (lines ~3052 onward, and the hard-link/symlink fixtures near `root_and_outside_record`/`symlink_dir`).
- Produces: nothing later tasks depend on; this is a status oracle, not a contract.

Framing correction from the prior draft: a test must never be written in a way that *requires* a known vulnerability to keep behaving a specific way, because that reads as the plan mandating the vulnerability's preservation. The actual need is the opposite — an oracle that notices if the status changes at all (fixed, worsened, or behavior altered in an unrecorded way) and forces a deliberate decision, rather than letting a side effect of A3 silently reclassify an open defect as closed or as newly acceptable.

- [ ] **Step 1: Write `a3_sec_00x_status_is_unchanged_from_a2_or_this_test_must_be_updated_deliberately`.** The test does not assert "the vulnerable behavior must occur" as a requirement; it asserts "the behavior observed today matches what Continuity records as the current status ([d5b9] SEC-001 open, [c16c6] SEC-002 narrowed, [c17e8] SEC-003 narrowed)." If a future change (inside or outside this plan) fixes one of them, this test fails, and the fix author updates both this test and the corresponding Continuity record in the same change — a deliberate, visible decision, not a silent one. The test body reuses the existing SEC-001/002/003 fixtures to observe current behavior; it does not introduce new assertions that something *must* stay broken.

```rust
#[test]
fn a3_sec_00x_status_is_unchanged_from_a2_or_this_test_must_be_updated_deliberately() {
    // Status oracle, not a vulnerability requirement: reuses the existing
    // SEC-001/002/003 fixtures to observe today's behavior and compares it
    // against what Continuity records ([d5b9], [c16c6], [c17e8]) as current
    // status. A status change is expected to make THIS test fail — the fix
    // is to update both this test and the Continuity record together, as a
    // visible decision, never to "fix" SEC-00x as a silent side effect of
    // unrelated A3 work.
}
```

- [ ] **Step 2: Run `cargo test -p storage a3_sec_00x_status -- --nocapture`**

Expected: PASS against today's codebase, confirming the oracle correctly observes the status Continuity already records.

- [ ] **Step 3: Commit**

```bash
git add crates/storage/src/vault.rs
git commit -m "test(A3): add a drift oracle over SEC-001/002/003's current status

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 1: Scaffold `vault-security` crate

**Files:**
- Create: `crates/vault-security/Cargo.toml`
- Create: `crates/vault-security/src/lib.rs`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Consumes: `domain::Classification`. `vault-security` takes note ids as `&str` at its boundary rather than depending on `storage::NoteId`, keeping the dependency one-directional per the File Structure note above.
- Produces: an empty, compiling crate with `cargo deny` clean, ready for Tasks 2–5, 7–8 to fill in.

- [ ] **Step 1: Create `crates/vault-security/Cargo.toml`** with `name = "vault-security"`, `edition` matching the workspace, a dependency on `domain` (path dep) only — **not** `policy` and **not** `storage`; the `EncryptedStoreAvailability` type that `policy::Gate::evaluate` consumes (Task 4) is defined in `vault-security` and `policy` depends on `vault-security`, not the reverse, so `vault-security` itself needs neither. Plus `thiserror`, `sha2`, `uuid`, `serde`, `serde_yaml` at the versions already pinned in the workspace `Cargo.lock`. Do not add `aes-gcm`/`argon2`/`zeroize`/`windows`/`rusqlite` yet — Tasks 2, 4, 5 add those with their own license/deny check at the point of use.

- [ ] **Step 2: Create `crates/vault-security/src/lib.rs`** with a crate-level doc comment stating the crate's one job (encrypted PII store + deletion log + backup, for `sensitive`-classified data only) and an empty `mod` list.

- [ ] **Step 3: Add `"crates/vault-security"` to the workspace `members` array in the root `Cargo.toml`.**

- [ ] **Step 4: Run `cargo build --workspace` and `cargo deny check`**

Expected: both succeed; the new crate compiles with zero warnings.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/vault-security
git commit -m "chore(A3): scaffold vault-security crate

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: Master key — DPAPI wrap + Argon2id recovery kit, with existing-store guards

**Files:**
- Create: `crates/vault-security/src/keys.rs`
- Create: `crates/vault-security/src/recovery_kit.rs`
- Modify: `crates/vault-security/src/lib.rs` (module decls + re-exports)
- Modify: `crates/vault-security/Cargo.toml` (add `aes-gcm`, `argon2`, `zeroize`, `windows` with license check)

**Interfaces:**
- Consumes: nothing from other A3 tasks.
- Produces (for Task 5, Task 7, Task 8):
  - `pub struct MasterKey` (32-byte AEAD key, `zeroize`s on drop).
  - `pub enum KeySource { Dpapi, RecoveryKit }`.
  - `pub fn MasterKey::load_or_create(key_store_path: &Path, store_fingerprint: Option<&StoreFingerprint>) -> Result<MasterKey, KeyError>` — see Step 3's correction below: this is **not** "try DPAPI, else generate fresh" unconditionally. `store_fingerprint`, when `Some`, is a cheap, non-secret marker the caller (Task 7's `PiiStore::open`) computes from whatever ciphertext already exists on disk (e.g., a hash of the first record's AEAD tag, or simply "at least one `.pii` file exists") — enough to answer "does a store already exist that this key must be able to open," without the key material itself being needed to compute it.
  - `pub fn MasterKey::unwrap_with_dpapi(wrapped_bytes: &[u8]) -> Result<MasterKey, KeyError>`.
  - `pub enum KeyError { Io(..), DpapiUnavailable, DpapiDecryptFailed, RecoveryKitRequired, ExistingStoreNotDecryptable, PlatformUnsupported }` — `ExistingStoreNotDecryptable` is the new, named failure for the silent-re-keying case (Review Focus item 1); `PlatformUnsupported` is the named error for non-Windows builds (per Tech Stack).
  - `recovery_kit::generate(passphrase: &str, master_key: &MasterKey) -> Result<RecoveryKit, RecoveryKitError>` producing an Argon2id-derived key that independently re-wraps/unwraps **the same `MasterKey` instance passed in** (not a fresh one it generates itself — the whole point is this must be able to recover the key actually protecting the live store), plus a human-printable recovery blob (`RecoveryKit::to_printable_string()` / `from_printable_string()`).
  - `recovery_kit::unwrap_master_key(kit_text: &str, passphrase: &str) -> Result<MasterKey, RecoveryKitError>`.

- [ ] **Step 1: Write the failing tests for both directions of Review Focus item 1, plus the cross-machine recovery proof**

```rust
#[test]
#[cfg(windows)]
fn unwrap_with_dpapi_on_bytes_wrapped_by_a_different_users_dpapi_fails_closed_not_as_corruption() {
    // CryptUnprotectData fails when the calling user/machine context differs
    // from the one that wrapped the bytes. Simulate by corrupting the wrapped
    // blob's DPAPI envelope (not the plaintext key) and assert the error is
    // specifically DpapiDecryptFailed, distinguishable from Io or a panic.
}

#[test]
fn load_or_create_with_no_key_file_and_no_existing_store_generates_a_fresh_key() {
    // The ONLY case where minting a fresh key is correct: nothing to protect
    // yet. store_fingerprint is None (or indicates "store is empty").
}

#[test]
fn load_or_create_with_no_key_file_but_an_existing_undecryptable_store_refuses_to_mint_a_fresh_key() {
    // The critical negative case (Review Focus item 1, second direction):
    // key file is missing/corrupt AND store_fingerprint indicates ciphertext
    // already exists on disk. Must return Err(ExistingStoreNotDecryptable),
    // NEVER Ok(a new MasterKey) — a fresh key here would silently orphan
    // every existing sensitive note while reporting success.
}

#[test]
fn recovery_kit_generated_from_a_live_master_key_recovers_that_exact_key_independent_of_dpapi() {
    // generate() against a REAL MasterKey (the one load_or_create produced,
    // not a standalone one); then, WITHOUT calling any DPAPI function (this
    // is the cross-machine proof: DPAPI is tied to the user/machine, so the
    // test must prove the recovery path alone, with no DPAPI call in it,
    // reconstructs byte-identical key material from kit_text + passphrase).
    // Wrong passphrase must fail closed, not silently return a different key.
}
```

- [ ] **Step 2: Run `cargo test -p vault-security keys:: recovery_kit:: -- --nocapture`**

Expected: FAIL (types/functions not defined yet).

- [ ] **Step 3: Implement `MasterKey`, `KeySource`, `KeyError`, `load_or_create`, `unwrap_with_dpapi` in `keys.rs`.** The corrected `load_or_create` logic: (a) if the key file exists, try to unwrap it — on success return the key, on `DpapiDecryptFailed` return that error directly (never fall through to generating a new key); (b) if the key file does NOT exist, check `store_fingerprint` — if it indicates no existing ciphertext, generate a fresh key, wrap it, write the file, return it; if it indicates existing ciphertext, return `Err(ExistingStoreNotDecryptable)` without touching the key file at all. Use the `windows` crate's `Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData}` behind `#[cfg(windows)]`; a `#[cfg(not(windows))]` stub returns `Err(KeyError::PlatformUnsupported)` from every function. Key bytes live in a `zeroize::Zeroizing<[u8; 32]>` or equivalent; never `Debug`-derive a type that contains raw key bytes (this is also one of Task 3's audit surfaces).

- [ ] **Step 4: Implement `recovery_kit.rs`**: `RecoveryKit::generate(passphrase, master_key)` derives a 32-byte key via `argon2::Argon2` (document the exact params in a doc comment — at least 64 MiB memory, 3 iterations, tuned so derivation takes roughly 0.5–1s on the target laptop), uses that key to AEAD-wrap `master_key`'s actual bytes, and returns a `RecoveryKit` holding the wrapped bytes + the Argon2 salt + params (not the passphrase). `to_printable_string`/`from_printable_string` base32- or base64-encode the wrapped blob + salt + params into one copyable string, meant for offline storage per C5.

- [ ] **Step 5: Run the Step 1 tests.**

Expected: PASS.

- [ ] **Step 6: Run `cargo deny check`** after adding `aes-gcm`, `argon2`, `zeroize`, `windows` to `Cargo.toml` — confirm each resolves to an SPDX-allowlisted license; if any transitive dependency does not, record the exception the same way `unsafe-libyaml` was recorded ([d5q0]) before proceeding.

- [ ] **Step 7: Commit**

```bash
git add crates/vault-security/src/keys.rs crates/vault-security/src/recovery_kit.rs crates/vault-security/src/lib.rs crates/vault-security/Cargo.toml Cargo.lock
git commit -m "feat(A3): DPAPI master key wrap with Argon2id recovery kit and existing-store guard

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: Confidentiality-boundary audit — enumerate every surface that must never see plaintext or key material

**Files:**
- Create: `crates/storage/tests/a3_confidentiality_boundary.rs`

**Interfaces:**
- Consumes: `storage::propose_and_commit` and `storage::Vault::read_note` (existing, A1/A2), `vault_security::pii_store::PiiStore` (Task 7 — this task's tests are written now but can only fully compile and run once Task 7 lands; see Step 0 below for ordering).
- Produces: a standing regression suite later tasks must keep green; no new production types.

This task exists because the plan's own Review Focus item 5 identified that no single round-trip test proves confidentiality — it only proves the *intended* path behaves. This task is the dedicated, enumerated check across every *other* path data could leak through, mapping directly to contract C3's explicit list of boundaries ("file, frontmatter e metadati, allegati, file generati, cache, log, indici, embedding, riassunti, backup").

- [ ] **Step 0 (ordering note): write these tests now as failing/`#[ignore]`d stubs naming each surface; un-ignore and complete each one as the subsystem it tests (Task 6's write path, Task 7's `PiiStore`, Task 8's backup) lands.** This keeps the enumeration itself — the actual deliverable of this task — reviewable up front, per the user's request to resolve this boundary before implementation proceeds, rather than having it discovered piecemeal.

- [ ] **Step 1: Write the git-history surface test**

```rust
#[test]
fn no_git_blob_at_any_commit_ever_contains_sensitive_plaintext_or_the_pii_pointers_target_bytes() {
    // propose_and_commit a sensitive note (once Task 6 lands); then run
    // `git cat-file --batch-all-objects` (or an equivalent walk of every
    // blob reachable from every ref, not just HEAD) against the vault repo
    // and assert the plaintext content never appears in any blob — the
    // pointer string is fine, the plaintext is not. Covers C3's explicit
    // "doppio controllo... in più una scansione periodica della cronologia."
}
```

- [ ] **Step 2: Write the A2 journal surface test**

```rust
#[test]
fn the_a2_sqlite_journals_columns_never_contain_sensitive_plaintext() {
    // After a sensitive-note write, inspect every column of the journal row
    // for that operation_id directly (content_hash is a hash, fine; but
    // nothing in the row may be the plaintext itself or the unwrapped data
    // key).
}
```

- [ ] **Step 3: Write the before-image surface test**

```rust
#[test]
fn before_images_for_sensitive_notes_never_contain_plaintext_pointer_target_bytes() {
    // The before-image mechanism (vault.rs's .before-images dir) persists
    // the PREVIOUS record's bytes before a rename. For a sensitive note the
    // "record" is the pointer-only Markdown file, so this test's main job
    // is to confirm Task 6's write path never routes raw plaintext through
    // persist_before_image by accident (e.g. if an update path is coded to
    // snapshot "the content" generically before checking classification).
}
```

- [ ] **Step 4: Write the error-message/log surface test**

```rust
#[test]
fn no_error_variant_debug_or_display_output_from_pii_store_or_propose_and_commit_echoes_plaintext_or_key_bytes() {
    // Force each PiiStoreError and the sensitive-path ProposeError variants
    // and assert their {:?} and {} output never contains the plaintext
    // content or raw key/data-key bytes used in the test fixture — only
    // note ids, generation numbers, and hashes are expected to appear.
}
```

- [ ] **Step 5: Write the temp-file surface test**

```rust
#[test]
fn an_interrupted_sensitive_write_leaves_no_recoverable_plaintext_in_any_temp_file_under_the_vault_root() {
    // Using Task 6's new FaultPoints, interrupt a sensitive write partway
    // through; scan every file under the vault root (not just the expected
    // ones) for the plaintext content. Only ciphertext-shaped bytes (or
    // nothing, if the step hadn't run yet) may be present anywhere.
}
```

- [ ] **Step 6: Write the backup-blob surface test**

```rust
#[test]
fn a_backup_blob_written_to_a_backuptarget_contains_no_plaintext_sensitive_content_outside_its_aead_envelope() {
    // Once Task 8 lands: take a backup of a vault containing a sensitive
    // note; read the raw bytes BackupTarget::write_blob received (not
    // through restore()) and assert the plaintext does not appear as a
    // substring anywhere in that blob.
}
```

- [ ] **Step 6b: Record, explicitly, which of C3's listed surfaces this task does NOT cover and why.** C3 (`docs/Spec_mvp.md` line 364) names nine surfaces: "file, frontmatter e metadati, allegati, file generati, cache, log, indici, embedding, riassunti, backup." This task's Steps 1–6 cover file content, frontmatter/metadata (via the pointer scheme), log/error output, temp files, and backup. **Indici (search index), embedding, and riassunti (summaries) are out of scope here because those subsystems do not exist yet** — they belong to Fase B (index) and Fase C (summaries), neither implemented. This is a deliberate deferral, not an oversight: add a doc comment at the top of `a3_confidentiality_boundary.rs` stating exactly this, and record a Continuity open item so B1/B2 and C2's own plans re-audit the sensitive-classification boundary against an index/embedding/summary surface that will exist by then.

- [ ] **Step 7: As each dependency lands (Tasks 6–8), un-ignore and run the corresponding test; all must PASS before Task 9's whole-branch review.**

- [ ] **Step 8: Commit** (initial stub version now; later tasks' commits amend this file's `#[ignore]` status as they land, each in that task's own commit)

```bash
git add crates/storage/tests/a3_confidentiality_boundary.rs
git commit -m "test(A3): enumerate the confidentiality boundary across every non-obvious surface

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: `Gate::evaluate` — conditional, not unconditional, rejection of `Sensitive`

**Files:**
- Modify: `crates/policy/src/gate.rs`
- Modify: `crates/policy/Cargo.toml` (add path dependency on `vault-security`, for the `EncryptedStoreAvailability` type only — `policy` does not gain any crypto or I/O dependency)

**Interfaces:**
- Consumes: nothing from `vault-security`'s I/O — only a plain data type.
- Produces (for Task 6): `Gate::evaluate`'s new signature and the exact decision table `propose_and_commit` must drive it with.
  - `pub enum EncryptedStoreAvailability { Available, Unavailable { reason: StoreUnavailableReason } }` (defined in `vault-security`, re-exported, zero I/O — a plain enum the caller constructs after it has already tried to open the store).
  - `pub enum StoreUnavailableReason { NotYetConfigured, KeyUnreachable, StoreInconsistent }` — mirrors `KeyError`'s and `PiiStoreError`'s shapes loosely but is intentionally its own small enum, since `policy` must not depend on `vault-security`'s crypto error types directly (only the plain availability type, keeping `policy` free of any crypto/IO dependency surface).
  - `pub fn Gate::evaluate(&self, note: &NoteMetadata, content: &str, link_state_result: Result<LinkState, LinkError>, store: EncryptedStoreAvailability) -> GateDecision` — the Sensitive branch becomes: `if matches!(note.classification, Sensitive) { match store { Available => fall through to the secret-content scan same as any other classification, Unavailable { reason } => Reject(SensitiveRequiresEncryptedStore { note_id, reason }) } }`. The existing short-circuit order (schema → link → classification/availability → secret-content scan → Pass) is unchanged; only the classification step's outcome becomes conditional.
  - `GateRejection::SensitiveRequiresEncryptedStore` gains the `reason: StoreUnavailableReason` field (an additive, non-breaking change to an existing variant's payload — every existing match arm using `{ .. }` still compiles; `storage`'s own match arms are checked in Task 6).

- [ ] **Step 1: Write the failing tests for both outcomes**

```rust
#[test]
fn sensitive_note_is_rejected_when_the_store_is_unavailable_exactly_as_before() {
    // Same assertion as A1's existing sensitive_classification_is_rejected_pending_encrypted_store,
    // but now driven by passing EncryptedStoreAvailability::Unavailable explicitly,
    // proving the rejection is still unconditional for THAT input, not removed.
}

#[test]
fn sensitive_note_passes_the_classification_step_when_the_store_is_available_and_still_runs_the_secret_content_scan() {
    // With Available, a sensitive note with no secret-shaped content reaches
    // Pass; a sensitive note WITH secret-shaped content still gets
    // Reject(SecretContentDetected) — proving the secret scanner is not
    // skipped just because classification passed.
}
```

- [ ] **Step 2: Run `cargo test -p policy gate:: -- --nocapture`**

Expected: FAIL (signature mismatch — this is a deliberate breaking change to `evaluate`'s signature, confined to this crate's own tests until Task 6 updates `storage`'s call site).

- [ ] **Step 3: Implement the signature change and decision-table update in `gate.rs`.** Update every existing test in `gate.rs` that calls `evaluate` to pass `EncryptedStoreAvailability::Unavailable { reason: StoreUnavailableReason::NotYetConfigured }` where the old tests implicitly assumed "no store exists" (A1-era tests), keeping their assertions intact — this is the minimal change that preserves A1's existing guarantees under the new signature.

- [ ] **Step 4: Run Step 1 tests plus the full existing `policy` suite.**

Expected: PASS; zero regressions in A1's pre-existing `gate.rs` tests (their assertions are unchanged, only the call site gained one argument).

- [ ] **Step 5: Commit**

```bash
git add crates/policy/src/gate.rs crates/policy/Cargo.toml Cargo.lock
git commit -m "feat(A3): make the gate's Sensitive rejection conditional on store availability

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 5: `DeletionLog` — hash-chained, monotonic, crash-safe per C5, with an authenticated trusted reference

**Files:**
- Create: `crates/vault-security/src/deletion_log.rs`
- Modify: `crates/vault-security/src/lib.rs`

**Interfaces:**
- Consumes: `keys::MasterKey` (Task 2) — the trusted reference is wrapped the same way the master key itself is, correcting the prior draft's unauthenticated `FileTrustedReferenceStore`.
- Produces (for Task 6, Task 8):
  - `pub struct DeletionLog { db_path: PathBuf }` — its own SQLite file, e.g. `<root>/deletion-log.sqlite`, never the A2 journal's file.
  - `pub struct GenerationRef { pub generation: u64, pub chain_hash: String }` — the "riferimento affidabile." `pub trait TrustedReferenceStore { fn read(&self) -> Result<Option<GenerationRef>, DeletionLogError>; fn write(&self, r: &GenerationRef) -> Result<(), DeletionLogError>; }`. `pub struct DpapiTrustedReferenceStore` is the real implementation: it DPAPI-wraps the serialized `GenerationRef` the same way `keys.rs` wraps the master key, so tampering with the on-disk reference file is detected as a decrypt failure, not silently accepted as a different generation. A `FileTrustedReferenceStore` (plain, unauthenticated) exists **only** behind `#[cfg(test)]`, explicitly documented as test-only and never constructible from production code — this directly corrects the prior draft's ungated test/dev impl.
  - `pub enum DeletionLogState { Pending, Completed }` per generation-entry with its own `valid_transition` (`Pending -> Completed` only), exactly like `domain::write_state::WriteState`.
  - `pub fn DeletionLog::record_deletion(&self, note_id: &str, trusted_ref: &dyn TrustedReferenceStore) -> Result<DeletionReceipt, DeletionLogError>` — runs the full three-step protocol synchronously and returns once `Completed`.
  - `pub fn DeletionLog::is_deleted(&self, note_id: &str) -> Result<bool, DeletionLogError>` — the read side; during a `Pending` window for a given note, this must return `true` (not consultable) per spec line "il dato diventa non consultabile dal passo 1."
  - `pub fn DeletionLog::reconcile(&self, trusted_ref: &dyn TrustedReferenceStore) -> Result<ReconcileOutcome, DeletionLogError>` — run at startup; implements the roll-forward rule and the fail-closed rule for any other disagreement.
  - `pub enum DeletionLogError { Io(..), ChainBroken, ReferenceStale { log_generation: u64, reference_generation: u64 }, ReferenceMissing, ReferenceCorrupt, ReferenceTamperDetected, Inconsistent }` — `ReferenceTamperDetected` is new, distinct from `ReferenceCorrupt` (a decrypt/auth failure vs. a structurally-unreadable file).
  - Crate-local `enum FaultPoint { AfterGenerationWrittenPending, AfterReferenceUpdated, BeforeMarkedCompleted }` with its own `FaultInjector`, same idiom as `storage::fault` (a sibling module, not a shared one — `vault-security` does not depend on `storage`).

- [ ] **Step 1: Write the failing crash-between-steps test**

```rust
#[test]
fn crash_between_log_write_and_reference_update_leaves_data_non_consultable_through_restart_and_converges_on_reconcile() {
    // 1. record_deletion("note-a", &trusted_ref) with an injector that
    //    interrupts at AfterGenerationWrittenPending.
    // 2. Assert is_deleted("note-a") == true even in this half-finished state.
    // 3. Open a FRESH DeletionLog against the same db_path (simulating
    //    restart) and call reconcile() against the (still stale) reference.
    // 4. Assert reconcile rolls the reference forward, the entry is now
    //    Completed, and is_deleted still == true throughout.
}

#[test]
fn reference_ahead_of_log_or_any_other_unexplained_mismatch_blocks_in_reconciliation_mode_not_silently() { }

#[test]
fn a_tampered_trusted_reference_file_is_detected_as_tamper_distinct_from_plain_corruption() {
    // Flip a byte in the DPAPI-wrapped reference blob (not truncate/delete
    // it — that's ReferenceCorrupt/ReferenceMissing, tested separately).
    // Assert reconcile()/the read side report ReferenceTamperDetected, not
    // a value that happens to parse as a lower or higher generation.
}
```

- [ ] **Step 2: Run `cargo test -p vault-security deletion_log:: -- --nocapture`**

Expected: FAIL.

- [ ] **Step 3: Implement the SQLite schema** (one table: `generation INTEGER PRIMARY KEY, note_id TEXT NOT NULL, prev_chain_hash TEXT NOT NULL, chain_hash TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('pending','completed'))`), `DeletionLogState::valid_transition`, `record_deletion`/`is_deleted`/`reconcile`, and `DpapiTrustedReferenceStore` (wrap/unwrap via the same primitive `keys.rs` exposes for the master key — reuse, don't reimplement, the DPAPI call). `chain_hash` = `sha256(prev_chain_hash || generation || note_id)`.

- [ ] **Step 4: Run Step 1 tests.**

Expected: PASS.

- [ ] **Step 5: Write and pass the remaining C5 test-bullet cases explicitly**, each its own test function: registro assente durante restore, registro corrotto durante restore, registro valido ma obsoleto, due cancellazioni consecutive con crash tra le due, lettura concorrente durante una cancellazione.

- [ ] **Step 6: Commit**

```bash
git add crates/vault-security/src/deletion_log.rs crates/vault-security/src/lib.rs
git commit -m "feat(A3): hash-chained deletion log with an authenticated trusted reference

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 5b: Deletion-log restore-time gate (the "stale reference" acceptance check)

**Files:**
- Modify: `crates/vault-security/src/deletion_log.rs` (add `DeletionLog::verify_safe_to_expose_after_restore`)

**Interfaces:**
- Consumes: `DeletionLog::reconcile`, `GenerationRef`, `TrustedReferenceStore` (Task 5).
- Produces (for Task 8): `pub fn DeletionLog::verify_safe_to_expose_after_restore(&self, restored_reference_snapshot: &GenerationRef, live_trusted_ref: &dyn TrustedReferenceStore) -> Result<(), DeletionLogError>`.

- [ ] **Step 1: Write the failing test for the "valid but obsolete" case specifically**

```rust
#[test]
fn restore_with_a_syntactically_valid_but_lower_generation_reference_is_rejected_as_stale_not_accepted() { }

#[test]
fn restore_with_reference_matching_live_generation_and_valid_chain_is_accepted() { }
```

- [ ] **Step 2: Run `cargo test -p vault-security verify_safe_to_expose -- --nocapture`**

Expected: FAIL.

- [ ] **Step 3: Implement `verify_safe_to_expose_after_restore`**: compare `restored_reference_snapshot.generation` against `live_trusted_ref`'s current generation; equal or ahead is fine, strictly behind is `DeletionLogError::ReferenceStale`.

- [ ] **Step 4: Run Step 1 tests.**

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/vault-security/src/deletion_log.rs
git commit -m "feat(A3): reject restore against a stale deletion-log reference

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 6: `PiiStore` and the single, crash-recoverable sensitive-write pipeline

**Files:**
- Create: `crates/vault-security/src/pii_store.rs`
- Modify: `crates/storage/Cargo.toml` (path dependency on `vault-security`)
- Modify: `crates/storage/src/propose.rs`
- Modify: `crates/storage/src/vault.rs` (the pipeline extension and its new `FaultPoint`s)
- Modify: `crates/storage/src/fault.rs` (new `FaultPoint` variants)
- Test: `crates/storage/tests/a3_pii_integration.rs`

This task is the direct correction for point 3 and (together with Task 4) point 1: the gate decision and the write pipeline are designed together so there is exactly **one** crash-recoverable state machine per sensitive write, not two.

**Interfaces:**
- Consumes: `keys::MasterKey` (Task 2), `policy::EncryptedStoreAvailability` (Task 4).
- Produces (for Task 3's audit, Task 8):
  - `pub struct PiiStore { root: PathBuf, master_key: MasterKey }` — `root` is a directory distinct from the Markdown vault root, never git-initialized; `PiiStore::open` computes the `store_fingerprint` Task 2's `load_or_create` needs by checking whether `root` already contains any `.pii` file.
  - `pub fn PiiStore::open(root: PathBuf, key_store_path: &Path) -> Result<Self, PiiStoreError>` — internally calls `MasterKey::load_or_create` with the fingerprint it just computed; this is how `propose_and_commit` answers `EncryptedStoreAvailability` (`Available` iff `open` succeeded for this call's configured paths; `Unavailable { reason }` mapped from whatever `PiiStoreError`/`KeyError` `open` returned).
  - `pub fn PiiStore::stage(&self, note_id: &str, plaintext: &[u8], operation_id: Uuid) -> Result<StagedPii, PiiStoreError>` — writes ciphertext to a durable, hash-verifiable *staging* location (`<root>/.staging/<note_id>-<operation_id>.pii`), fsynced, **not yet** the live `<note_id>.pii`. Returns `StagedPii { content_hash: String }`. This is the new intermediate durability point the crash-recovery design needs — directly analogous to `storage::vault`'s before-image-before-rename ordering.
  - `pub fn PiiStore::commit_staged(&self, note_id: &str, operation_id: Uuid) -> Result<(), PiiStoreError>` — renames the staged file onto the live `<note_id>.pii` path (one atomic rename, same primitive family as `storage::atomic::atomic_write`), then removes the staging entry.
  - `pub fn PiiStore::get(&self, note_id: &str) -> Result<Vec<u8>, PiiStoreError>` — unwraps, decrypts, returns plaintext; a hash/tag mismatch is `PiiStoreError::Inconsistent { note_id }`.
  - `pub fn PiiStore::find_orphans(&self, known_pointers: &HashSet<String>) -> Vec<String>` — enumerates `.pii` files with no corresponding pointer in `known_pointers`, excluding anything still in `.staging` (a staged-but-not-yet-committed file is not an orphan; it is mid-write).
  - `pub enum PiiStoreError { Io(..), Inconsistent { note_id: String }, NotFound { note_id: String }, Key(KeyError) }`.
  - `pub struct PiiPointer { pub uri: String }` with `PiiPointer::parse`.

**`storage` side — the pipeline extension:**

- New `FaultPoint` variants in `fault.rs`: `AfterPiiStagedBeforeCommit` (ciphertext durably staged, live `.pii` not yet updated, Markdown pointer not yet renamed), `AfterPiiCommittedBeforePointerRename` (live `.pii` updated, Markdown pointer rename not yet run). These slot into the *same* sequence `write_record_unchecked` already uses, immediately before the existing `AfterRenameBeforeContentPersisted` point — for a sensitive note, "persist content" means staging+committing `PiiStore`, then the Markdown rename carries only the pointer bytes.
- The journal's existing `content_hash` column (Task-A2-era) stores the **pointer file's** hash as it does today; a new, additive, nullable column `pii_content_hash` stores the staged ciphertext's hash, written on the same journal write as `AfterPiiStagedBeforeCommit`'s advance — mirroring exactly how `content_hash` itself is recorded before the rename today ([d9mm]'s pattern, extended).
- `recovery::reconcile` gains one new branch: a row with a non-null `pii_content_hash` and a `previous_state` at or before `ContentPersisted` must verify `PiiStore` has a matching staged-or-committed ciphertext before rolling forward; if the staged ciphertext is missing or its hash doesn't match, roll back exactly as the existing before-image path does for ordinary content (restore the previous pointer, discard the orphaned stage, advance to `Failed`).

- [ ] **Step 1: Write the failing happy-path test**

```rust
#[test]
fn a_sensitive_note_is_written_to_the_pii_store_and_the_markdown_record_holds_only_a_pointer() {
    // propose_and_commit a classification: sensitive note with a real
    // PiiStore open and EncryptedStoreAvailability::Available; assert
    // read_note's returned content contains "pii://<id>", NOT the plaintext
    // body; assert PiiStore::get independently decrypts to the real content.
}
```

- [ ] **Step 2: Run it.**

Expected: FAIL.

- [ ] **Step 3: Write the failing crash-between-ciphertext-and-pointer test (Review Focus item 2, create path)**

```rust
#[test]
fn crash_after_pii_staged_before_pointer_committed_recovers_without_a_readable_half_written_pointer() {
    // Interrupt at AfterPiiStagedBeforeCommit. Assert: (a) read_note for
    // this note_id before recovery runs returns NotFound or the PREVIOUS
    // version if this was an update, never a pointer to ciphertext that
    // doesn't exist yet; (b) after Vault::open (reconcile) runs, the
    // operation resolves deterministically — either rolled forward (pointer
    // now committed, PiiStore::get works) or rolled back (Failed, no
    // orphaned staged ciphertext left behind) — never left ambiguous.
}
```

- [ ] **Step 4: Write the failing crash-on-update test (Review Focus item 2, update path — explicitly required, not optional)**

```rust
#[test]
fn crash_mid_update_of_an_already_sensitive_note_never_exposes_a_pointer_whose_ciphertext_is_the_wrong_version() {
    // Write a sensitive note (v1), let it fully commit. Start an update to
    // v2 with an injector at AfterPiiStagedBeforeCommit. Assert that at every
    // observable point — mid-crash and after recovery completes — read_note
    // plus PiiStore::get together are self-consistent: either the full v1
    // pair (pointer + v1 ciphertext) or the full v2 pair, NEVER a v2 pointer
    // reading back v1 ciphertext or vice versa. This is the specific
    // multi-store consistency failure mode the previous draft never named.
}
```

- [ ] **Step 5: Implement the `propose.rs`/`vault.rs`/`pii_store.rs` changes per the Interfaces blocks above.** `propose_and_commit` gains no new required parameter in its public signature from the caller's point of view beyond what's needed to locate the `PiiStore` (recommended: derive its root as `<vault_root>/.pii-store`, documented exactly like `BEFORE_IMAGE_DIR`); if a `&PiiStore` ends up needing to be passed explicitly instead, grep every existing caller of `propose_and_commit` in `storage`'s test suite first and update them all in this same step.

- [ ] **Step 6: Run Steps 1/3/4's tests plus the full existing `storage` suite.**

Expected: PASS, and every pre-existing A1/A2 test still passes unchanged.

- [ ] **Step 7: Write and implement the orphan-detection tests**

```rust
#[test]
fn deleting_the_pointer_note_without_going_through_the_deletion_log_leaves_detectable_orphaned_ciphertext() { }

#[test]
fn a_properly_deleted_sensitive_note_goes_through_the_deletion_log_and_is_not_flagged_as_an_unexplained_orphan() {
    // Deletion of a sensitive note is modeled as a propose_and_commit call
    // (see Global Constraints) whose gate-accepted write is a tombstone
    // record; propose_and_commit's sensitive branch, on observing this is a
    // deletion (not a content update), calls DeletionLog::record_deletion
    // BEFORE committing the tombstone pointer, inside the same operation —
    // no second public entry point, no propose_delete.
}
```

- [ ] **Step 8: Run Step 7 tests.**

Expected: PASS.

- [ ] **Step 9: Un-ignore and run Task 3's git-history, journal, before-image, and error-message surface tests now that this task's write path exists.**

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/storage/Cargo.toml crates/storage/src/propose.rs crates/storage/src/vault.rs crates/storage/src/fault.rs crates/storage/tests/a3_pii_integration.rs crates/storage/tests/a3_confidentiality_boundary.rs crates/vault-security/src/pii_store.rs crates/vault-security/src/lib.rs Cargo.lock
git commit -m "feat(A3): single crash-recoverable pipeline for sensitive-note writes

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 7: `Backup`/`Restore` against an abstract `BackupTarget`, gated on the live deletion log

**Files:**
- Create: `crates/vault-security/src/backup.rs`
- Modify: `crates/vault-security/src/lib.rs`

**Interfaces:**
- Consumes: `keys::MasterKey` (Task 2), `pii_store::PiiStore` (Task 6), `deletion_log::{DeletionLog, DeletionLog::verify_safe_to_expose_after_restore}` (Tasks 5–5b).
- Produces: nothing further A3 tasks depend on.
  - `pub trait BackupTarget { fn write_blob(&self, name: &str, bytes: &[u8]) -> Result<(), BackupError>; fn read_blob(&self, name: &str) -> Result<Vec<u8>, BackupError>; fn list_blobs(&self) -> Result<Vec<String>, BackupError>; fn is_empty(&self) -> Result<bool, BackupError>; }`
  - `pub struct FilesystemBackupTarget { dir: PathBuf }` implementing it — the physical backup disk stays an explicitly open decision per the spec's own closing section, out of scope here.
  - `pub fn backup(vault_root: &Path, pii_store: &PiiStore, deletion_log: &DeletionLog, master_key: &MasterKey, target: &dyn BackupTarget) -> Result<BackupManifest, BackupError>` — encrypts a snapshot of the Markdown vault + `PiiStore` + a `GenerationRef` snapshot, writes it via `target.write_blob`, returns `BackupManifest { hash: String, generation: u64, created_at: ... }`.
  - `pub fn restore(target: &dyn BackupTarget, master_key: &MasterKey, destination_root: &Path, live_deletion_log: &DeletionLog, live_trusted_ref: &dyn TrustedReferenceStore) -> Result<RestoreOutcome, BackupError>` — reads the blob, decrypts, then performs **two** checks before writing anything to `destination_root`: (1) `DeletionLog::verify_safe_to_expose_after_restore` against the backup's own embedded `GenerationRef` snapshot (Task 5b's check — catches a stale snapshot *within* the backup), and (2) cross-checks every note id the backup would restore against `live_deletion_log.is_deleted(note_id)` — catching the Review Focus case where a note was deleted *after* this backup was taken, which the backup's own embedded snapshot cannot know about. Only if both pass does extraction proceed.
  - `pub enum BackupError { Io(..), TargetEmpty, HashMismatch, DeletionLogRejected(DeletionLogError), DeletedSinceBackup { note_ids: Vec<String> }, AlreadyExists }`.

- [ ] **Step 1: Write the failing round-trip test**

```rust
#[test]
fn full_backup_then_restore_to_an_empty_directory_yields_byte_identical_content_with_verified_hashes() { }

#[test]
fn restore_refuses_to_write_anything_when_the_backups_own_reference_snapshot_is_stale() { }
```

- [ ] **Step 2: Run them.**

Expected: FAIL.

- [ ] **Step 3: Write the failing deleted-after-backup test (Review Focus item 4)**

```rust
#[test]
fn a_note_deleted_after_the_backup_was_taken_does_not_reappear_on_restore_even_though_the_backups_own_snapshot_predates_the_deletion() {
    // backup() a vault containing sensitive note "note-a"; THEN delete
    // note-a (through the Task 6 deletion path, recording it in the LIVE
    // deletion log); THEN restore() from the earlier backup against that
    // live log. Assert note-a is NOT present/readable in the restored
    // output, and the error/outcome names note-a explicitly
    // (DeletedSinceBackup), distinct from a generic rejection.
}
```

- [ ] **Step 4: Implement `backup()`/`restore()` per the Interfaces block, including the two-check gate in Step 3's test.** Encrypt the whole serialized snapshot under an HKDF-derived subkey of `master_key` (not `master_key` directly), keeping backup ciphertext and `PiiStore`'s live per-record keys cryptographically independent. The manifest hash is computed over the plaintext snapshot before encryption.

- [ ] **Step 4b: Write and implement the backup-during-pending-deletion test — independent review flagged that Task 5's `pending`-crash cases are tested in isolation but never combined with a concurrent backup.**

```rust
#[test]
fn a_backup_taken_while_a_deletion_is_still_pending_and_then_restored_does_not_expose_the_note_that_deletion_targets() {
    // Start record_deletion("note-a") and pause it (via the Task 5
    // FaultInjector) exactly at AfterGenerationWrittenPending — the log
    // entry exists and is Pending, the trusted reference has not been
    // updated yet (C5's own "crash tra passo 2 e 3" window, but here
    // reached by a backup running concurrently, not a crash). While paused,
    // run backup() against this same vault. Resume and let record_deletion
    // complete normally. Then restore() from that backup against the NOW
    // fully-updated live trusted ref. Assert note-a is NOT exposed by the
    // restore — is_deleted's "pending window is already non-consultable"
    // rule (C5 line 409) must hold for a backup taken mid-window exactly as
    // it holds for an ordinary read taken mid-window.
}
```

- [ ] **Step 5: Run Steps 1, 3, and 4b's tests.**

Expected: PASS.

- [ ] **Step 6: Write and implement the second-backup-cycle test**

```rust
#[test]
fn a_second_backup_to_an_already_used_target_replaces_rather_than_corrupts_the_prior_one_and_restore_still_round_trips() { }
```

- [ ] **Step 7: Run it plus full `cargo test -p vault-security`.**

Expected: PASS.

- [ ] **Step 8: Un-ignore and run Task 3's backup-blob surface test.**

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/vault-security/src/backup.rs crates/vault-security/src/lib.rs crates/storage/tests/a3_confidentiality_boundary.rs
git commit -m "feat(A3): encrypted backup/restore gated on the live deletion log, not just the backups own snapshot

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 8: Whole-branch review and acceptance-criteria evidence

**Files:**
- Create: `eval/results/a3-security-durability.md` (match whatever format A1/A2 already used in `eval/results/` — check existing files there before choosing).

**Interfaces:**
- Consumes: everything from Tasks 0–7.
- Produces: the acceptance-criteria record for A3.

- [ ] **Step 1: Run `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo deny check`, and `cargo mutants -p vault-security`.**

Expected: all green; record the exact test count (A2 ended at 220); every surviving mutant is examined and either justified in Step 4's evidence file or covered by a new test before this task is considered done.

- [ ] **Step 2: Re-run Task 0's drift oracle explicitly and confirm by inspection that no task changed SEC-001/002/003's actual behavior without a corresponding, deliberate update to both the test and Continuity** — `git diff a3-complete-base..HEAD -- crates/storage/src/vault.rs` should show only the pipeline-extension lines from Task 6 (new `FaultPoint` checks, the `pii_content_hash` branch in `reconcile`) and Task 0's test addition; `record_path_checked`/`ensure_resolves_inside_root`/`atomic_write`'s bodies themselves must show no changes.

- [ ] **Step 3: Confirm every Task 3 confidentiality-boundary test is un-ignored and passing** — this is the explicit, enumerated proof for point 2 of the requested revision, not an implicit claim.

- [ ] **Step 4: Measure and record backup/restore timing** on a representative vault size.

- [ ] **Step 5: Write `eval/results/a3-security-durability.md`** recording: commit hash, test counts, a traceability table mapping every C5 test-bullet and every Review Focus item to its test function name, confirmation that the gate's `Sensitive` rejection stayed in force throughout (Task 4/6), confirmation no second public write/delete entry point was added (Task 6's design note), SEC-001/002/003 status unchanged-or-deliberately-updated (link to Task 0), and the explicit list of what A3 does NOT cover.

- [ ] **Step 6: Dispatch an independent reviewer** (fresh subagent, no prior context) with explicit instructions to focus on: confidentiality (Task 3's enumerated surfaces), crash consistency (Task 6's single-pipeline design, both create and update paths), deletion/restore (Task 5b and Task 7's live-log cross-check), and the `policy`/`storage`/`vault-security` dependency direction (no cycle, no second public write/delete entry point) — matching exactly the focus the user requested for this review round.

- [ ] **Step 7: Commit the evidence file; do NOT merge to `main`.**

```bash
git add eval/results/a3-security-durability.md
git commit -m "docs(A3): record acceptance-criteria evidence, branch ready for review

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Explicit Scope Boundaries (out of this plan)

- No merge to `main`. No tagging `a3-complete`. Those require the user's explicit decision after Task 8's independent review, exactly as A1 and A2 required ([c12av], [d14hn]).
- No change to `docs/Spec_mvp.md`.
- SEC-001 (TOCTOU), SEC-002 (symlink aliasing), SEC-003 (hard-link aliasing) are NOT fixed by this plan and are NOT entrenched by it either — Task 0 is a drift oracle over their current status, not a requirement that they remain as they are.
- No second public write or delete entry point is added to `storage`. Deletion of a sensitive note is a `propose_and_commit` call whose gate-accepted content is a tombstone, driving the deletion log from inside that one call.
- Physical backup disk selection, key-rotation runbook details beyond what C5's protocol itself requires, and non-Windows platform support are explicitly deferred.
- Fase B/C/D (search, memory-with-evidence, coding agent) are untouched.
