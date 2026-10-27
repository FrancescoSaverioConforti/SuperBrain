# A1 — Schema e storage: closing evidence

**Date:** 2026-10-26

**Git commit hash (Task 9's implementation commit, which this file was part of):** `8b63b35fe95ccbf973273ad962399570d8d4030d`

> Note on this hash: because a commit hash is content-addressed, it cannot be known while this file is still being written as part of that same commit. The value above is the real `git rev-parse HEAD` read immediately after that commit was created. Since this file's own content cannot retroactively change the already-made commit without amending it (which this project's workflow avoids except when explicitly requested), a second, small follow-up commit updates only this recorded hash from a placeholder to the verified value above — see `git log --oneline -- eval/results/A1/evidence.md` for both commits.

**Plan:** `docs/superpowers/plans/2026-10-26-a1-schema-storage.md`
**Task brief:** `.superpowers/sdd/2026-10-26-a1-schema-storage/task-9-brief.md`

---

## 1. Scope of this evidence file

This file closes out Task 9, the final task of the A1 ("Fase A — Vault affidabile") plan. It covers:

- The new `storage::propose_and_commit` gate-backed public write entry point and its `ProposeError` type (`crates/storage/src/propose.rs`).
- The API-surface regression test pinning `storage`'s public write surface to exactly one function.
- End-to-end integration tests (`crates/storage/tests/integration.rs`) exercising the full parse → classify-links → gate → write pipeline, including rejection paths that prove the record file is never created.
- Full-workspace verification: build, test, clippy, `cargo-deny`, and `cargo-mutants` across `domain`, `storage`, and `policy`.
- The acceptance-criteria mapping for the whole A1 plan (not just Task 9), since this is the final task and the evidence directory is the plan's closing deliverable.

**A1's explicit boundary** (restated here per the brief, verbatim): No SQLite journal, no git commit, no crash recovery, no encryption, no backup/restore in this evidence — those are verified under A2/A3. Cross-process/cross-restart write safety is not provided — only in-process concurrency via a shared `Vault` instance is guaranteed. `propose_and_commit` is the only `pub` write function in `storage`; `write_record_unchecked` is private and unreachable from outside the crate, enforced by the compiler, not by convention. The crate dependency table was amended during A1 implementation: `storage` now depends on `domain` and `policy` (was `domain` only in the spec's original table), so `storage` can call the gate before writing — `policy` still depends only on `domain`, so no cycle exists.

**Clarification on "private" vs. compiler enforcement**: `write_record_unchecked` is declared `pub(crate)`, not a bare private `fn`. This is a deliberate, minimal visibility choice, not a loosening of the guarantee: Rust's module-privacy model scopes a bare, non-`pub` item to its *defining module only* (`vault.rs`), not the whole crate, so `propose.rs` — a sibling module of `vault.rs`, not a descendant — could not call a plain-private method there even though both live in `storage`. `pub(crate)` is the narrowest visibility that permits this same-crate, cross-module call while remaining **completely unreachable from any other crate**, including `policy` (which has no dependency edge toward `storage` at all) and any future external consumer. The "no bypass" guarantee is therefore still a hard compiler fact: no code outside the `storage` crate can name or call `write_record_unchecked`, full stop.

> **Correction (A1 final-review fix wave, see Section 7).** The paragraph above was true of `write_record_unchecked` but the *overall* "`propose_and_commit` is the only `pub` write function" claim was **false at Task 9 close**: `storage::atomic_write` (the raw temp-file+rename primitive, with no `NoteId` validation, no gate, and no version check) was still `pub` and re-exported from `crates/storage/src/lib.rs`, so any external crate could write arbitrary bytes to any path (final-review Finding C1, reproduced from an external probe crate). It is now `pub(crate)` and not re-exported, and the claim is now enforced by `compile_fail` doctests in `crates/storage/src/lib.rs` (Section 7.1). Two other statements in this file were also overclaims at Task 9 close and are corrected in Section 4: note-id safety ignored case-insensitive filesystems (Finding C2), and "unknown/extra frontmatter field" was marked PASS without any test of an actually-unknown key (Finding I1).

---

## 2. Commands run and results

All commands were run from the repository root in the `worktree-a1-schema-storage` worktree, with `cargo` on `PATH` via `$HOME/.cargo/bin` (Windows, not on default `PATH`).

### 2.1 Baseline (before Task 9 changes)

```
cargo test --workspace
```
Result: 47 passed (19 domain, 12 policy, 16 storage), 0 failed. Confirmed baseline matched Tasks 1–8's expected state before any Task 9 changes were made.

### 2.2 TDD sequence for Task 9

1. Wrote `crates/storage/tests/integration.rs` containing the API-surface test (`storage_exposes_exactly_one_public_write_function`) and the four end-to-end `propose_and_commit` tests (valid note / secret-classified rejection / broken-link rejection / version-conflict), per the brief's Steps 1–2.
2. Ran `cargo test -p storage` — confirmed failure for the expected reason: `include_str!("../src/propose.rs")` could not find the file (didn't exist yet), and `storage::propose_and_commit` / `storage::ProposeError` did not resolve. This is a compile-time failure, not a runtime assertion failure, which is the correct failure mode before any implementation exists.
3. Implemented `crates/storage/src/propose.rs` (`propose_and_commit`, `ProposeError`) and wired it into `crates/storage/src/lib.rs` (`mod propose; pub use propose::{propose_and_commit, ProposeError};`).
4. Changed `Vault::write_record_unchecked`'s visibility from a bare private `fn` (with a now-obsolete `#[allow(dead_code)]`, since Task 8 left it uncalled in production code) to `pub(crate) fn`, the minimum visibility that lets `propose.rs` call it from within the same crate — see the clarification in Section 1.
5. Ran `cargo test -p storage propose` and `cargo test -p storage storage_exposes_exactly_one_public_write_function` — both now **pass**.

### 2.3 Full verification suite (Step 5)

```
cargo build --workspace
```
Result: success, no warnings.

```
cargo test --workspace
```
Result: **53 passed, 0 failed** — 19 domain, 12 policy, 17 storage (16 original + 1 new mutation-killing test, `vault::tests::non_not_found_io_error_on_read_propagates_as_io_error_not_version_zero`, added during the mutation-testing pass described below), 5 new `storage` integration tests. No wiring regressions.

```
cargo clippy --workspace -- -D warnings
```
Result: clean, no warnings, no errors.

```
cargo deny check
```
Result: `advisories ok, bans ok, licenses ok, sources ok`, exit code 0. Pre-existing informational warnings only (two unmatched license-allowlist entries carried over from Task 1's `deny.toml`, and a duplicate `syn` v2/v3 lock entry from unrelated transitive deps) — neither is new to Task 9 and neither fails the check.

```
cargo install cargo-mutants --locked
```
Result: `cargo-mutants v27.1.0` installed successfully (not previously present in this environment).

```
cargo mutants --package domain --package storage --package policy
```
Run twice: once before adding a mutation-killing test, once after. See Section 3 for the full survivor analysis and dispositions.

- **Run 1** (before the new test): `58 mutants tested in 70s: 5 missed, 40 caught, 13 unviable`.
- **Run 2** (after adding `non_not_found_io_error_on_read_propagates_as_io_error_not_version_zero`): `58 mutants tested in 67s: 4 missed, 41 caught, 13 unviable`.

---

## 3. Mutation testing: survivors and dispositions

`cargo-mutants` was run against all three in-scope crates (`domain`, `storage`, `policy`). 58 mutants were generated in total; 13 were "unviable" (did not compile — cargo-mutants' own filter, not a test gap) and are not analyzed further here. Of the 45 viable mutants, one round of remediation reduced survivors from 5 to 4.

### 3.1 Mutant killed by a new test

| Location | Mutation | Disposition |
|---|---|---|
| `crates/storage/src/vault.rs:190:23` | `replace match guard e.kind() == std::io::ErrorKind::NotFound with true in Vault::write_record_unchecked` | **Killed.** Added `vault::tests::non_not_found_io_error_on_read_propagates_as_io_error_not_version_zero`: creates a directory at the exact path `write_record_unchecked` would read as a record file, forcing `std::fs::read` to fail with `PermissionDenied` (confirmed empirically on this platform) rather than `NotFound`. Before the fix, the mutant made *any* read error silently treated as "note doesn't exist yet" (`Version(0)`), which would let a write proceed over/through an unreadable path instead of surfacing `StorageError::Io`. The new test asserts `StorageError::Io(_)` is returned, which fails under the mutant and passes against the real guard. |

### 3.2 Remaining survivors — all in `crates/storage/src/note_id.rs`, judged equivalent/unreachable on this platform

All four remaining survivors are on the same two source lines (43 and 50) inside `NoteId::parse`:

```rust
Component::Normal(_) => normal_segments += 1,   // line 43
...
if normal_segments > 1 || raw.contains('/') || raw.contains('\\') {   // line 50
```

| Location | Mutation | Disposition |
|---|---|---|
| `note_id.rs:43:57` | `replace += with *=` | **Justified, not killed.** |
| `note_id.rs:50:28` | `replace > with <` | **Justified, not killed.** |
| `note_id.rs:50:32` | `replace \|\| with &&` (first operator) | **Justified, not killed.** |
| `note_id.rs:50:53` | `replace \|\| with &&` (second operator) | **Justified, not killed.** |

**Justification (one disposition covers all four, since they share the same root cause):** `std::path::Path::components()` never yields more than one `Component::Normal` segment for a string that contains neither `/` nor `\`, on this build (verified empirically: an inline probe test run during this task against inputs `"a..b"`, `"a.b"`, `"a:b"`, `"a b"`, `"a;b"` — none containing a path separator character — each produced exactly 1 `Normal` component, never 2+). Since `NoteId::parse` only reaches `normal_segments > 1` as true when a separator character was present in the raw string (which also makes `raw.contains('/') || raw.contains('\\')` true by itself), the `normal_segments > 1` clause and the `+=` increment that feeds it can never independently flip the final `||` expression's boolean outcome on this platform: the two `contains` checks alone already decide every case the existing tests exercise (`note_id_with_path_separator_is_rejected` covers both `/` and `\` directly, and `valid_note_id_is_accepted`/`empty_note_id_is_rejected`/`note_id_with_parent_dir_component_is_rejected`/`absolute_path_as_number_is_rejected` cover the no-separator cases). No input string exists, under this platform's `Path` parsing rules, that would make these four mutations observably change `NoteId::parse`'s return value from the real code's — they are equivalent mutants in this execution environment, not a gap in the test plan's intent (which is "reject embedded path separators," fully covered). This was confirmed empirically during this task (probe added, run, and removed — not left in the committed diff) rather than assumed from reading the code alone.

**Superseded by the fix wave (Section 7.5):** fix C2 rewrote `NoteId::parse` and removed the `normal_segments` counter entirely, so these four equivalent mutants no longer exist. The dispositions above describe the Task 9 code only.

**Scope note:** this reasoning is platform- and `std`-version-dependent (it relies on `Path::new(...).components()`'s actual splitting behavior on this build, not on a language guarantee), so it is recorded here as the evidence for *this* environment's test run rather than claimed as a universal proof. If a future `std` change or a different platform's `Path` component-splitting ever made multiple `Normal` segments reachable without an embedded separator character, these four mutants would become live gaps and this disposition would need re-evaluation — flagged here rather than silently assumed permanent.

---

## 4. Acceptance criteria mapping (full A1 plan)

Per the plan's Global Constraints and Review Focus sections (`docs/superpowers/plans/2026-10-26-a1-schema-storage.md`), since Task 9 is the plan's final task:

### 4.1 Global Constraints

| Constraint | Status | Evidence |
|---|---|---|
| Only allowlisted SPDX licenses (Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC, Zlib) | PASS | `cargo deny check` → `licenses ok` |
| No secrets, no network calls in `domain`/`storage`/`policy` | PASS | No network-capable dependency in any of the three crates' `Cargo.toml`; `policy::secrets` pattern-matches in-process only |
| `classification: secret` always rejected | PASS | `domain::note::tests::secret_classification_is_always_rejected` (Task 2); `policy::gate::tests::secret_classification_is_rejected_before_content_is_even_scanned` (Task 8); `storage` integration test `secret_classified_note_is_rejected_by_propose_and_commit_and_the_record_file_is_never_created` (Task 9, end-to-end) |
| `classification: sensitive` rejected with a forward-pointing reason (A3) | PASS | `policy::gate::tests::sensitive_classification_is_rejected_pending_encrypted_store`; `GateRejection::SensitiveRequiresEncryptedStore` names the reason in its variant and doc comment |
| No journal/git/crash-recovery/backup/encryption/deletion-registry in this plan | PASS (by omission, as required) | No such code exists in `domain`/`storage`/`policy`; `operation_id: Uuid` and `WriteState` exist only as unused-for-persistence interface shapes per the plan's explicit instruction |
| Expected-version check and write in the same critical section | PASS | `Vault::write_record_unchecked` holds `_guard` (per-note mutex) across both the read-compare and the `atomic_write` call (Task 6); proven under concurrency by `vault::tests::concurrent_writers_sharing_one_vault_instance_on_same_version_only_one_succeeds` |
| Content+version as one atomically-replaced unit | PASS | `serialize_record`/`parse_record` in `vault.rs` build one buffer, written by exactly one `atomic_write` call; `vault::tests::record_file_never_shows_mismatched_content_and_version` |
| `propose_and_commit` is the only `pub` write function; raw primitive unreachable outside the crate | **Was FAIL at Task 9 close** (Finding C1: `atomic_write` was `pub` and re-exported, so it was a public bypass; the Task 9 PASS was an overclaim). **PASS after fix C1** | Both raw primitives (`atomic_write`, `Vault::write_record_unchecked`) are `pub(crate)`, and `lib.rs` re-exports neither. Compiler-level proof: 4 `compile_fail` doctests plus a positive control in `crates/storage/src/lib.rs` (`cargo test --doc -p storage`). Textual guard: `storage_exposes_exactly_one_public_write_function`, which now also scans `lib.rs` re-exports and `atomic.rs`. External probe crate: all 4 bypass attempts fail to compile (Section 7.1). |
| `note_id` validated before any path use; symlink-escape checked | **Was FAIL at Task 9 close** (Finding C2: `NoteId` was case-sensitive with no charset, so `note-a` and `NOTE-A` aliased one file on case-insensitive filesystems, causing reproduced data loss). **PASS after fix C2** | `NoteId::parse` accepts only `[a-z0-9._-]+`. It validates and never normalizes, and it rejects uppercase (`MustBeLowercase`), other characters and whitespace (`InvalidCharacter`), a leading or trailing `.`, any `..`, Windows reserved device names, and ids over 128 bytes. `propose_and_commit` validates the note id, and every relation target not declared in `pending_ids`, before the gate or the vault runs. Pending targets are opaque external references and are deliberately not `NoteId`s (fix I-A, Section 8.1). The lock map is keyed by `NoteId`. Tests: `note_id::tests::*` (14), `case_variant_of_an_existing_note_id_is_rejected_before_the_vault_and_original_is_unchanged`. **Symlink-escape half: was FAIL until the third fix wave; the PASS previously recorded here was an overclaim** (Finding I-C, Section 9.1). `record_path_checked` canonicalized the bare `root/<id>` path, which no I/O opens, and never checked `root/<id>.record`, which every read and write opens. The cited test, `write_target_resolving_outside_vault_root_via_symlink_is_rejected`, planted its link at the bare path, so it could not detect an escape. **PASS after fix I-C, for the check itself**: the real `.record` path is now checked, with the tests listed in Section 9.1. Not covered: the check-then-use TOCTOU window (L3). |
| Every typed error names the offending field/condition | PASS | `ValidationError::{MissingField,InvalidFieldType,InvalidEnumValue,UnknownField}` all carry a field name. `LinkError::{BrokenLink,SelfReference}` carry the relation and id. `StorageError::Conflict`/`PathEscapesVaultRoot`/`NotFound` carry `note_id`. `ProposeError::InvalidNoteId { field, value, error }` names the field and the raw value. `NoteIdError` variants name the offending character, name or length. (`StorageError::InvalidNoteId` and `StorageError::Atomic` were removed by fixes C2 and C1.) |
| Secret-policy exception scoped to exact occurrence (pattern+note+hash) | PASS | `SecretException{pattern_id, matched_text_hash, note_id,...}`; `policy::gate::tests::exception_for_one_occurrence_does_not_cover_a_different_matched_value` |
| No wall-clock sleeps in concurrency tests | PASS | `concurrent_writers_sharing_one_vault_instance_on_same_version_only_one_succeeds` uses `std::sync::Barrier`, not `sleep` |
| Evidence for every acceptance criterion in `eval/results/A1/` | PASS | This file, alongside `eval/results/A1/dependency-exceptions.md` (Task 1, preserved unmodified) |
| No index/agent/api/model code | PASS | No such modules exist in any A1 crate |
| A1 not reported as providing crash recovery or cross-restart idempotency | PASS | Boundary statement in Section 1, verbatim per the brief |

### 4.2 Review Focus items

| Item | Owning task | Status |
|---|---|---|
| Unknown/extra frontmatter field, or wrong YAML type for a field | Task 2 | **Was FAIL at Task 9 close; the original PASS was an overclaim** (Finding I1). The two cited tests covered only a bad enum *value* and a wrong *type*, never an unknown *key*. In fact `totally_unknown: 1` parsed `Ok`, and a typo'd relation key, `supprots: ["missing-id"]`, parsed and **committed** while its broken link silently vanished. **PASS after fix I1**: `ValidationError::UnknownField`, plus `#[serde(deny_unknown_fields)]` on `Relations`. Tests: `note::tests::unknown_top_level_key_is_rejected_with_named_unknown_field`, `typo_in_relation_key_is_rejected_not_silently_dropped`, `misspelled_top_level_field_is_reported_as_unknown_not_just_missing`, and `relations_struct_itself_denies_unknown_fields_when_deserialized_directly`; end-to-end, `typoed_relation_key_is_rejected_end_to_end_and_the_record_file_is_never_created` and `unknown_top_level_frontmatter_key_is_rejected_end_to_end`. Wrong YAML type is still covered by `wrong_type_for_created_is_named_field_error` (PASS). |
| Self-reference in `relations.*` | Task 3 | PASS — `links::tests::self_reference_is_rejected_not_silently_resolved` |
| Classification value outside the four valid enum members | Task 2 | PASS — `note::tests::unknown_classification_value_is_named_field_error` |
| Concurrent writer retries with a new expected version after losing a conflict | Task 6 | PASS — `vault::tests::losing_writer_can_retry_with_fresh_version_and_succeed` |
| Expected version collides numerically with a different note's version | Task 6 | PASS — `vault::tests::version_numbers_do_not_collide_across_different_notes` |
| `note_id` crafted to escape the vault root (traversal / symlink) | Task 6 | Traversal: PASS, `note_id::tests::*_is_rejected`. Symlink: **was FAIL until the third fix wave; the earlier PASS was an overclaim** (Finding I-C). The cited test planted its link at a path no I/O touches, and a link at the real `root/<id>.record` path was followed by `read_note` and by the version check. **PASS after fix I-C** for a link that already exists when the check runs: `vault::tests::read_note_rejects_record_path_symlinked_to_a_file_outside_the_vault_root`, `write_rejects_record_path_symlinked_outside_and_never_reads_the_outside_version`, `dangling_record_symlink_is_rejected_not_treated_as_a_missing_note`, `self_referential_record_symlink_is_rejected_cleanly`, and end-to-end `propose_and_commit_rejects_a_record_path_symlinked_outside_the_vault_root`. A link swapped in *after* the check is not covered (L3, TOCTOU). |
| `note_id` that aliases another note's file (case variant, trailing dot, reserved name) | Final review C2 | **Was FAIL at Task 9 close** (not covered by the original Review Focus list). **PASS after fix C2**: see the `note_id` row in 4.1 |

### 4.3 Task 9's own deliverables

| Deliverable | Status | Evidence |
|---|---|---|
| `propose_and_commit` is the sole public write path, gate-backed | **Was FAIL at Task 9 close (C1)**, **PASS after fix C1** | `crates/storage/src/propose.rs`, `compile_fail` doctests in `crates/storage/src/lib.rs`, and `storage_exposes_exactly_one_public_write_function` |
| Valid note passes gate and commits | PASS | `valid_note_passes_gate_and_is_committed_through_the_controlled_entry_point` |
| Secret-classified note rejected, file never created | PASS | `secret_classified_note_is_rejected_by_propose_and_commit_and_the_record_file_is_never_created` |
| Broken-link note rejected, file never created | PASS | `note_with_broken_link_is_rejected_by_propose_and_commit_and_the_record_file_is_never_created` |
| Version conflict surfaces end-to-end | PASS | `version_conflict_surfaces_end_to_end_through_propose_and_commit` |
| Full workspace build/test/clippy/deny/mutants | PASS | Section 2, this file |
| Mutation survivors each dispositioned | PASS | Section 3, this file |
| Evidence file written | PASS | This file |

---

## 5. Files created or modified in this task

- Created: `crates/storage/src/propose.rs` (`propose_and_commit`, `ProposeError`)
- Modified: `crates/storage/src/lib.rs` (added `mod propose;` and its re-exports)
- Modified: `crates/storage/src/vault.rs` (`write_record_unchecked` visibility: bare-private + `#[allow(dead_code)]` → `pub(crate)`, doc comment updated; added `non_not_found_io_error_on_read_propagates_as_io_error_not_version_zero` test to kill a mutation-testing survivor)
- Created: `crates/storage/tests/integration.rs` (API-surface test + 4 end-to-end tests)
- Created: `eval/results/A1/evidence.md` (this file)
- Unchanged, preserved: `eval/results/A1/dependency-exceptions.md` (Task 1)

---

## 6. Known limitations (accepted for A1, not defects)

All four limitations below are recorded in the SDD ledger and the Continuity memory. They are accepted only within A1's explicit scope: single process, non-adversarial, and not crash-safe (Section 1). Each one is also documented at its code site, so a future reader does not need the ledger to find it.

| # | Limitation | Why it is accepted for A1 | Code site / record |
|---|---|---|---|
| L1 | **Windows directory-fsync gap.** `atomic_write`'s directory fsync after the rename is `#[cfg(unix)]`-only. On Windows, durability of the rename's directory entry relies only on `rename`'s own guarantees. | Rust's std has no reliable way to fsync a directory handle on Windows. A1 makes no crash-durability claim, since crash safety is A2's journal. | `crates/storage/src/atomic.rs` (comment above the `#[cfg(unix)]` block); Continuity [c8yv]; ledger "Known accepted limitations" |
| L2 | **The API-surface test is textual, not type-level.** `storage_exposes_exactly_one_public_write_function` greps source text. | It is a belt-and-braces guard. The real guarantee is Rust privacy, now proven by the `compile_fail` doctests in `crates/storage/src/lib.rs`, which were added by fix C1. | `crates/storage/tests/integration.rs`; Continuity [c8yv] |
| L3 | **Residual TOCTOU window** between `Vault::record_path_checked`'s canonicalization (the symlink-escape check) and the later I/O (`std::fs::read`, or `atomic_write`'s create/rename). A concurrent process could swap the record path, or a component of it, for a symlink inside that window. Fix I-C (Section 9.1) changed *which* path is checked, from a decoy to the real `.record` path. It did not change *when*: the window is exactly as open as before. This is tracked as technical debt **SEC-001** in `docs/technical-debt.md`, alongside **SEC-002** (intra-vault symlink aliasing between two notes' records, Section 9.1) and **SEC-003** (the same aliasing via hard links, found during the fourth whole-branch review — hard links defeat canonicalization-based checks entirely, since a hard link is a genuine regular file, not a redirect). All three share the same closure mechanism: verify the opened handle itself (regular file, link count 1, path inside root), not another check on the path before opening it. | A1's threat model is explicitly single-process and non-adversarial. Closing the window needs handle-relative (`openat`-style) I/O, which is out of A1 scope. | Comment on `record_path_checked` in `crates/storage/src/vault.rs`; Continuity [c12ah], [c16c6], [c17e8]; `docs/technical-debt.md` SEC-001/002/003 |
| L4 | **`WriteState::Recovering`'s outgoing transitions are broader than contract C1's crash-recovery table.** For example, `Recovering -> IntentRecorded` is allowed, but C1 never describes it. | In A1, `WriteState` is an interface-only placeholder that nothing persists or drives. A2 owns the real recovery semantics and must tighten this set against C1's table. | Comment in `valid_transition`, `crates/domain/src/write_state.rs`; Continuity [c10b3] |

Final-review findings that were **not** part of this fix wave, because the user authorized only C1, C2, I1 and I2: I3 (CI lacks the secret-scanner and mutation-testing steps), I4 (`known_ids`/`pending_ids`/`SecretPolicy` are caller-supplied, with no registered-policy store), I5 (whether the `.record` format satisfies the spec's "Markdown file as source of truth"; a plan-level question for A2), and the Minor findings. They remain open and are not claimed as resolved here.

> **Update (second fix wave, Section 8).** The second whole-branch review found that I5 was broader than this paragraph says: `propose_and_commit` could not store a note body at all, only frontmatter. That gap (I-B) is now **closed**. A full note (`---` / frontmatter / `---` / body) is accepted, stored byte-for-byte and secret-scanned in full (Section 8.2). The remaining part of I5 is still open for A2: the on-disk file is a `<id>.record` with a version header line, not a plain `.md` file. I3, I4 and the Minors are unchanged.

---

## 7. A1 final-review fix wave (C1, C2, I1, I2)

The final whole-branch review found two Critical findings (C1, C2) and several Important ones. The user authorized fixing C1, C2, I1 and I2. Pre-fix baseline: commit `e6950b5`, `cargo test --workspace` gave 53 passed (19 domain, 12 policy, 17 storage, 5 integration, 0 doctests).

Commits: `8cd6636` (C1), `2fb35e2` (C2), `7ef705f` (I1), followed by the I2 / evidence commit that adds this section (see `git log --oneline -- eval/results/A1/evidence.md`).

### 7.1 C1: `atomic_write` was a public bypass

- **Defect:** `crates/storage/src/lib.rs` contained `pub use atomic::{atomic_write, AtomicWriteError};`, so any external crate could write raw bytes to any path.
- **Fix:** `atomic_write` and `AtomicWriteError` are now `pub(crate)`, and `lib.rs` no longer re-exports them. `StorageError::Atomic` was removed, and an atomic-write failure is now folded into `StorageError::Io(std::io::Error)` at the crate boundary. A full re-audit of `storage`'s public surface found these `pub use` re-exports: `NoteId`, `NoteIdError`, `propose_and_commit`, `ProposeError`, `StorageError`, `Vault`, `VaultRecord`, `Version`. The public `Vault` methods are `new`, `current_version` and `read_note`, all read-only or setup. Every item is intentional, and only `propose_and_commit` writes.
- **Proof:** `cargo test --doc -p storage` runs 4 `compile_fail` doctests plus 1 positive control and reports 5 passed. The blocks cover `storage::atomic_write`, `storage::atomic::atomic_write`, `storage::AtomicWriteError`, and `vault.write_record_unchecked(...)`. **TDD red** (before the fix): two of the blocks, the root `atomic_write` call and the `AtomicWriteError` type, failed with "Test compiled successfully, but it's marked `compile_fail`". **Caveat (verified):** stable rustdoc does not enforce the pinned `E0xxx` codes; a deliberately wrong `E0999` still passed. The positive control, which shares every setup expression, is what stops these blocks from passing for an unrelated reason on stable.
- **External probe crate** (outside the repo, `storage = { path = ... }`): against the fixed code, all four bypass attempts fail to compile, with `E0425 cannot find function atomic_write in crate storage`, `E0425 cannot find type AtomicWriteError in crate storage`, `E0603 module atomic is private`, and `E0624 method write_record_unchecked is private`. Against pre-fix commit `e6950b5`, a probe depending on that git revision compiled and ran: `C1 atomic_write from external crate: true; file now contains Some("bypassed the gate")`.

### 7.2 C2: case-insensitive collision in `NoteId`

- **Defect:** `NoteId` had no charset restriction and was case-sensitive. The pre-fix probe against `e6950b5` showed: `C2 write note-a: Ok(Version(1))`, then `C2 write NOTE-A at note-a's version: Ok(Version(2))`, then `C2 note-a now: version=Version(2) clobbered=true`, which is silent data loss.
- **Fix:** `NoteId::parse` accepts only `[a-z0-9._-]+` and never normalizes. New variants: `MustBeLowercase { found }`, `InvalidCharacter { found }`, `LeadingOrTrailingDot`, `ReservedName { name }` (`con`, `prn`, `aux`, `nul`, `com0-9`, `lpt0-9`, with or without an extension) and `TooLong { max: 128, actual }`. Any `..` is `ContainsParentRef`. `propose_and_commit` validates the note id and every relation target up front and returns `ProposeError::InvalidNoteId { field, value, error }` before the gate or the vault runs. `known_ids`/`pending_ids` are now `HashSet<NoteId>`, and `Vault`'s lock map is keyed by `NoteId`. *(Corrected by the second fix wave: making `pending_ids` a `HashSet<NoteId>` broke spec C6, and it is `HashSet<String>` again. See Section 8.1. `known_ids` stays `HashSet<NoteId>`.)* As a result, the lock key, the record path and the link identity are the same canonical value. `StorageError::InvalidNoteId` was removed because it can no longer be constructed.
- **Proof:** 9 new `note_id::tests`: `NOTE-A`, `Note-A`, `note-A` and a mixed-case date id are rejected with `MustBeLowercase`; ids round-trip byte-for-byte; whitespace, non-ASCII and punctuation are rejected; leading or trailing dots, `..`, reserved names and length are each covered. End-to-end, `case_variant_of_an_existing_note_id_is_rejected_before_the_vault_and_original_is_unchanged` writes `note-a`, then retries `NOTE-A`, `Note-A` and `note-A` at `note-a`'s current version. Each retry returns `InvalidNoteId { field: "id", error: MustBeLowercase }`, `note-a`'s record is `==` the original, and the directory listing is unchanged. Two more tests show that relation targets are validated (`NOTE-B` is rejected) and that a canonical target resolves.
- **External probe crate** against the fixed code: `write NOTE-A at note-a's version Version(1): Err(InvalidNoteId { field: "id", value: "NOTE-A", error: MustBeLowercase { found: 'N' } })`, and the same for `Note-A`. It reports `note-a unchanged: true`, and `vault directory unchanged (names + mtimes): true`. It also shows `parse("trail.")` = `LeadingOrTrailingDot`, `parse("nul")` = `ReservedName`, and `parse(" a")` = `InvalidCharacter`.

### 7.3 I1: unknown frontmatter keys were accepted

- **Fix:** added `ValidationError::UnknownField(String)`. `parse_frontmatter` rejects unknown top-level keys before it checks for missing fields, and it rejects any `relations` key that is not one of the five relation names, reported as `relations.<key>`. `Relations` also carries `#[serde(deny_unknown_fields)]`.
- **Proof:** 4 new `note::tests`. TDD red was observed first: all 4 failed behaviorally, with unknown keys parsing `Ok`, a misspelled key surfacing as `MissingField`, and `Relations` accepting `supprots`. There are also 2 new end-to-end tests showing that neither case commits and that no record file is created.

### 7.4 I2: evidence omitted known limitations

- **Fix:** added Section 6 above with all four limitations, and added code-site comments on `record_path_checked` in `vault.rs` (L3) and in `valid_transition` in `write_state.rs` (L4). L1's site comment already existed in `atomic.rs`.

### 7.5 Verification after the fix wave

| Command | Result |
|---|---|
| `cargo build --workspace` | success, exit 0 |
| `cargo test --workspace` | **76 passed, 0 failed**: 23 domain (+4), 12 policy, 26 storage lib (+9), 10 storage integration (+5), 5 storage doctests (+5, new). The baseline was 53. |
| `cargo test --doc -p storage` | 5 passed (4 `compile fail ... ok` plus 1 positive control) |
| `cargo clippy --workspace -- -D warnings` | clean, exit 0 (`clippy --all-targets` still fails only on the 3 pre-existing `assert!(true)` placeholder tests, a known Minor finding outside this wave) |
| `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok`, exit 0. No dependency or `Cargo.lock` change in this wave. |
| `cargo mutants --package domain --package storage --package policy` | Run 1: `65 mutants tested in 79s: 1 missed, 50 caught, 14 unviable`. Run 2: **`65 mutants tested in 76s: 51 caught, 14 unviable`** (0 missed) |

**Mutation survivor in run 1, killed:** `vault.rs:197:23`, which replaces the match guard `e.kind() == NotFound` with `true` in `write_record_unchecked`. The Task 9 test `non_not_found_io_error_on_read_propagates_as_io_error_not_version_zero` used to kill this mutant only because the mutant's write then failed as `StorageError::Atomic`. Fix C1 folded that variant into `StorageError::Io`, so the error variant no longer distinguished the two paths, and the mutant became live. The test now also asserts that no write was attempted: the vault directory must contain only the pre-created `note-a.record` directory, with no orphaned `.tmp-*` file. Run 2 confirms the mutant is killed. The four `note_id.rs` equivalent mutants from Section 3.2 no longer exist, because the `normal_segments` code was removed.

---

## 8. Second fix wave (I-A, I-B)

The second whole-branch review (verdict "Ready to merge: With fixes", 0 Critical) found two Important issues, and the user authorized fixing both. Guardrails: no Markdown editor, no Markdown rendering, no indexing, and no A2/A3 functionality. Baseline: `6374aca`, 76 tests passing. Commits: `0a3c198` (I-A), `d9f746e` (I-B), followed by the docs commit that adds this section.

### 8.1 I-A: `pending_ids` narrowing broke spec C6 (contract break, now fixed)

- **Defect:** fix C2 made `pending_ids: &HashSet<NoteId>` and parsed every relation target as a `NoteId`. Spec C6 and the Fase A test bullet ("un riferimento pending ... viene accettato e segnalato") say a pending reference may be an external source, a not-yet-imported note or an unresolved code symbol. A URL (`https://...`), a DOI (`doi:10.1000/xyz`) or a symbol path (`src/main.rs::CorsMiddleware`) is not a valid `NoteId`, so it could not even be declared pending, and it was rejected as `InvalidNoteId`.
- **Fix** (`crates/storage/src/propose.rs`): `pending_ids` is `&HashSet<String>` again and is treated as opaque. `known_ids` stays `&HashSet<NoteId>`. A relation target found verbatim in `pending_ids` skips `NoteId::parse`. Every other target must still parse as a `NoteId`; otherwise the result is `ProposeError::InvalidNoteId`. A target that parses but is not in `known_ids` is still a `LinkError::BrokenLink`. `pending_ids` is passed to `domain::classify_links` unchanged, and that function and its mixed resolved+pending ⇒ `Pending` rule are also unchanged. Pending targets never reach the filesystem, so the C2 aliasing concern does not apply to them.
- **Proof** (`crates/storage/tests/integration.rs`): `pending_url_reference_in_derived_from_is_accepted`, `pending_doi_reference_in_supports_is_accepted` and `pending_unresolved_code_symbol_in_applies_to_is_accepted` each commit and read back the note. `mixed_resolved_and_opaque_pending_targets_commit_and_classify_as_pending` commits, and the same note classifies as `LinkState::Pending`. `non_note_id_target_not_declared_pending_is_still_rejected_as_invalid_note_id` and `valid_note_id_target_neither_known_nor_pending_is_still_a_broken_link` cover the two rejection paths, and in both no file is written. The existing `relation_target_that_is_not_a_canonical_note_id_is_rejected_as_invalid_note_id` still passes. **TDD red:** the new tests did not compile against the old signature (`E0308 expected &HashSet<NoteId>, found &HashSet<String>`), because the API could not express an opaque pending reference at all.

### 8.2 I-B: `propose_and_commit` could not store a note body (gap now closed)

- **Defect:** the whole input was parsed as YAML frontmatter, so no note body could be stored. Fix I1's strict unknown-key check also closed the only workaround, a `body:` key.
- **Fix** (`crates/storage/src/propose.rs`): the parameter `yaml` was renamed to `note_text`, and the parameter count did not change. A private helper, `frontmatter_of`, applies this rule: if the first line is exactly `---` (LF or CRLF), the frontmatter runs up to the next line that is exactly `---`, and everything after that line is the opaque body. Otherwise the whole text is bare frontmatter, which is the original form. That includes an opening `---` with no closing line, a plain YAML document-start marker. The rule is unambiguous because a column-0 `---` line inside YAML can only be a document separator. Only the frontmatter part goes to `domain::parse_frontmatter`, whose contract is unchanged. The **whole** `note_text` goes to `gate.evaluate`, so the secret scanner sees the body. The whole `note_text` is also what `write_record_unchecked` stores, byte-for-byte. `vault.rs` is unchanged. The body is never parsed as YAML or Markdown.
- **Version semantics:** frontmatter and body are one record with one `Version`. An update replaces both in one `atomic_write`, and a stale version is the same `StorageError::Conflict` as before. No code change was needed for this.
- **Proof:** `note_with_frontmatter_and_body_round_trips_byte_for_byte` uses a body that contains a `---` thematic break and a fake `id:` line, and `read_note` returns content `==` the input. `updating_a_note_replaces_its_body_atomically` checks that the old body is gone and that the version is 2. `version_conflict_on_a_note_with_a_body_is_the_ordinary_storage_conflict` checks `Conflict{expected: 0, actual: 1}` and that the original is unchanged. `secret_only_in_the_body_is_rejected_by_the_gate_and_the_record_file_is_never_created` gets `SecretContentDetected{pattern_id: "aws-access-key"}`, then `NotFound`, and an empty directory. `frontmatter_only_note_without_delimiters_still_commits_unchanged` is the backward-compatibility test. There are also 9 `propose::tests` unit tests for the splitting rule: LF, CRLF, closing line at end of text, empty frontmatter, later `---` lines treated as body, an indented `---` in a block scalar, near-miss `---x`/`--- `/`----` lines, and an unterminated opening line. **TDD red:** before the implementation, 4 of the 5 integration tests failed, because body input was rejected as a schema error; the frontmatter-only regression test passed, as intended. **Negative check:** a temporary edit that passed only `frontmatter_of(note_text)` to the gate made the body-secret test fail. The edit was reverted.

### 8.3 Verification after the second fix wave

| Command | Result |
|---|---|
| `cargo build --workspace` | success, exit 0 |
| `cargo test --workspace` | **96 passed, 0 failed**: 23 domain, 12 policy, 35 storage lib (+9 `propose::tests`), 21 storage integration (+11), 5 storage doctests. The baseline was 76. |
| `cargo test --doc -p storage` | 5 passed: 4 `compile_fail` plus the positive control, which was updated for the new `pending_ids` type |
| `cargo clippy --workspace -- -D warnings` | clean, exit 0 (forced re-check of all 3 crates) |
| `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok`, exit 0. No dependency change. |
| `cargo mutants --package domain --package storage --package policy` | **`73 mutants tested in 89s: 59 caught, 14 unviable`**, 0 missed. The previous run gave 65/51/14/0; the 8 new mutants come from the new code, and all are caught. |

---

## 9. Third fix wave (I-C, pending link state)

The third whole-branch review (0 Critical) found one new Important issue, I-C, and the user also authorized one of its Minors. Baseline: `7a273a0`, 96 tests passing. Commits: `27b517e` (I-C), `b83021f` (pending link state), followed by the docs commit that adds this section.

### 9.1 I-C: the symlink-escape check guarded the wrong path

- **Defect:** `Vault::record_path_checked` built `candidate = root/<id>.record`, the path that `read_note` and `write_record_unchecked` open. Its symlink check, however, ran on `root/<id>`, without the suffix, and the function returned `candidate` without checking it. A symlink at `root/note-a.record` pointing outside the root was followed by every read and by the version check. The Task 6 test planted its link at `root/escape`, which no I/O opens, so it passed whether or not the check worked.
- **Fix** (`crates/storage/src/vault.rs`, `record_path_checked` and the new private `ensure_resolves_inside_root`): if `symlink_metadata` finds an entry at the `.record` path (it does for a symlink even when the link's target is missing), the **full** path is canonicalized. Otherwise, for a first write, the parent directory is canonicalized. The result must start with the canonical root, or the call fails with `PathEscapesVaultRoot`. If canonicalization fails (a dangling link, a link to itself), the check fails closed with `PathEscapesVaultRoot` rather than treating the path as "no note yet". The bare-path check was removed because it guarded nothing.
- **Proof, and TDD red against the pre-fix code** (all in `vault::tests` unless noted):

| Test | Pre-fix result (observed) | Post-fix |
|---|---|---|
| `read_note_rejects_record_path_symlinked_to_a_file_outside_the_vault_root` | `Ok(VaultRecord { content: "OUTSIDE CONTENT", version: Version(7) })`: the outside file was read | `PathEscapesVaultRoot` |
| `write_rejects_record_path_symlinked_outside_and_never_reads_the_outside_version` | `unwrap_err()` on `Ok(Version(8))`: the version check used the outside file's version 7 | `PathEscapesVaultRoot` at `Version(7)` and at `Version(0)`; link and outside file untouched |
| `propose_and_commit_rejects_a_record_path_symlinked_outside_the_vault_root` (integration) | `unwrap_err()` on `Ok(Version(8))` | `ProposeError::Storage(PathEscapesVaultRoot { note_id: "note-a" })` |
| `dangling_record_symlink_is_rejected_not_treated_as_a_missing_note` | `Err(NotFound)`, so a `Version(0)` write would have gone ahead | `PathEscapesVaultRoot` for both read and write; no file is created at the target |
| `self_referential_record_symlink_is_rejected_cleanly` | `Err(Io(FilesystemLoop))` (clean, but not identified as a path problem) | `PathEscapesVaultRoot` for both read and write |
| `record_symlink_resolving_inside_the_vault_root_is_not_an_escape` | passes (control) | passes: a link whose target is inside the root is read normally |
| `symlink_at_the_bare_note_id_path_is_never_touched_by_io` (replaces `write_target_resolving_outside_vault_root_via_symlink_is_rejected`) | would fail: the old bare-path check rejected the write | the write lands in `root/escape.record` and nothing is written through the link |
| `existing_file_under_a_symlinked_directory_component_escapes`, `new_file_under_a_symlinked_directory_component_escapes`, `real_subdirectory_component_inside_the_root_is_accepted` | not compilable pre-fix (they call the new helper directly) | a symlinked directory component is caught for an existing file and for a new file; a real subdirectory is accepted |
| `vault_root_reached_through_a_symlinked_ancestor_still_works` | passes | passes: the root is the trust anchor, and both sides are canonicalized |

- **Directory components:** `NoteId` has no separators, so today the only directory above `<id>.record` is the root itself. A symlinked *intermediate* component cannot be reached through `Vault`. It is tested on `ensure_resolves_inside_root` directly, to show that canonicalizing the full path covers it if the layout ever gains subdirectories.
- **Negative control:** with the full-path branch disabled (always canonicalizing only the parent, which is what the original Task 6 brief specified), the four escape tests in `vault::tests` fail. The edit was reverted.
- **What this does not do:** it does not close the TOCTOU window (L3, **SEC-001**, Continuity [c12ah]). The check and the I/O are still separate steps, so a symlink swapped in between them is not detected. A link whose target is *inside* the root, for example one pointing at another note's record, is accepted — that is aliasing, not escape, and this fix does not address it (tracked as **SEC-002**, Continuity [c16c6]). The fourth whole-branch review additionally found that a **hard link** (not a symlink) at a `.record` path produces the identical aliasing effect, both to a file outside the vault and to another note's record inside it, and canonicalization cannot detect it since a hard link is a genuine regular file, not a redirect (tracked as **SEC-003**, Continuity [c17e8]). All three (SEC-001/002/003) share one root cause — a check on the path cannot see how the path will resolve when actually opened — and one proposed closure mechanism: verify the opened handle itself (regular file, link count 1, path inside root) rather than add another path-level check. See `docs/technical-debt.md`. All three are within A1's single-process, non-adversarial threat model.

### 9.2 Minor: the pending link state was computed but not reported

- **Defect:** `propose_and_commit` computed `link_state` and returned only `Version`. A pending reference was accepted but not flagged, contrary to spec C6 ("accettato e segnalato").
- **Fix** (`crates/storage/src/propose.rs`): the function now returns `Result<CommitOutcome, ProposeError>`, where `pub struct CommitOutcome { pub version: Version, pub link_state: domain::LinkState }`. `CommitOutcome` is re-exported from `lib.rs`. It is a plain result type, not a write path, so the public write surface is unchanged and `storage_exposes_exactly_one_public_write_function` still passes.
- **Proof:** `mixed_resolved_and_opaque_pending_targets_commit_and_classify_as_pending` now asserts the returned `CommitOutcome { version: Version(1), link_state: Pending }` instead of re-classifying the note itself. The three single-reference pending tests assert `Pending`, `valid_note_passes_gate_...` asserts `Unlinked`, and `canonical_relation_target_resolves_against_known_note_ids` asserts `Resolved`. **TDD red:** compile failure, `E0432 unresolved import storage::CommitOutcome` and `E0609 no field link_state on type storage::Version`, because the old API could not express the flag. **Negative control:** hard-coding `link_state: Resolved` in the returned outcome makes 5 integration tests fail. The edit was reverted.

### 9.3 Verification after the third fix wave

| Command | Result |
|---|---|
| `cargo build --workspace` | success, exit 0 |
| `cargo test --workspace` | **106 passed, 0 failed**: 23 domain, 12 policy, 44 storage lib (+9), 22 storage integration (+1), 5 storage doctests. The baseline was 96. |
| `cargo test --doc -p storage` | 5 passed (4 `compile_fail` plus the positive control) |
| `cargo clippy --workspace -- -D warnings` | clean, exit 0 (forced re-check of all 3 crates) |
| `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok`, exit 0. No dependency change. |
| `cargo mutants --package domain --package storage --package policy` | **`72 mutants tested in 85s: 58 caught, 14 unviable`**, 0 missed. The previous run gave 73/59/14/0. The net change of one mutant comes from removing the bare-path check and adding `ensure_resolves_inside_root`; every viable mutant is caught. |

---

## 10. Fourth whole-branch review: documentation-only findings

The fourth full whole-branch review (844333e..6e274d8, 36 commits) found **zero Critical and zero new code-level findings** within A1's stated scope. C1, C2, I-A, I-B and I-C all held up under a fresh, cover-to-cover read of `propose.rs`, `vault.rs` and `note_id.rs`, under an adversarial probe of how C2 (NoteId canonicalization), I-A (opaque pending references) and I-C (the real-path symlink check) compose together, and under a sixth-plus independent re-run of the full verification suite (106 tests, clean clippy/deny, 72 mutants tested / 0 missed — exact match to the third wave). The review's own assessment: after four rounds the code has converged; the remaining deferred items are a reasonable, now-documented set of known limits for an MVP's first phase, not a sign of a deeper problem.

It found two **documentation-only** gaps, both corrected directly (no code change):

### 10.1 Hard links were a missing member of the filesystem-aliasing class (now SEC-003)

The review reproduced, empirically on Windows, that a **hard link** at a `.record` path produces the identical aliasing effect as SEC-001/SEC-002's symlink cases — both to a file outside the vault (same effect as pre-fix I-C: `read_note` and the version check silently follow the link) and, by the same mechanism, to another note's own record inside the vault (same effect as SEC-002). Canonicalization cannot detect this, because a hard link is a second directory entry to the same inode, not a redirect — the path genuinely is a regular file genuinely inside the root.

This also meant SEC-002's original proposed fix ("reject any symlink at a record path, same mechanism as SEC-001, at no extra cost") was incomplete: rejecting symlinks does not reject hard links. `docs/technical-debt.md` has been corrected: SEC-002's closure note no longer claims "no extra cost," and a new **SEC-003** entry tracks the hard-link case specifically, with all three (SEC-001/002/003) now documented as sharing one root cause and one proposed closure mechanism — verify the *opened handle* (regular file, link count 1, path inside root) rather than add yet another check on the path before opening it. The plan file's "Outstanding technical debt" section and this evidence file's Section 6 (L3) and Section 9.1 are updated to cross-reference all three. Continuity constraint `[c17e8]` records SEC-003.

### 10.2 The accumulated Minor findings were not recorded in any git-tracked file

The SDD execution ledger (`.superpowers/sdd/.../progress.md`) is git-ignored, so after merge the only tracked trace of roughly a dozen Minor findings accumulated across all four review rounds would have been the single line at the end of Section 6 ("and the Minor findings. They remain open."). The table below is the enumerated record the review asked for — each item re-checked as still real and unchanged by the fourth review, none newly introduced by it:

| # | Minor finding | Where | Status |
|---|---|---|---|
| M1 | `schema_version` is never validated beyond the `as u32` truncation noted earlier — `999` and other out-of-range-but-fits values are accepted. | `crates/domain/src/note.rs` | Open |
| M2 | A YAML-escaped secret (e.g. an escape sequence that decodes to a real-looking AWS key) evades the gate's raw-text scanner. | `crates/policy/src/secrets.rs` | Open |
| M3 | Date-shaped fields (`created`, `updated`, `event_period`) are typed as `String` and never validated as real dates. | `crates/domain/src/note.rs` | Open |
| M4 | `ProposeError::Schema` is never constructed by `propose_and_commit` (schema errors arrive via `Rejected(Schema(..))` instead); `ProposeError` also has no `Display`/`std::error::Error` impl, unlike `StorageError`/`NoteIdError`. | `crates/storage/src/propose.rs` | Open |
| M5 | `atomic_write` does not clean up its temp file if the rename fails. | `crates/storage/src/atomic.rs` | Open |
| M6 | The secret-pattern regexes are recompiled on every `scan` call rather than cached. | `crates/policy/src/secrets.rs` | Open |
| M7 | `Vault`'s per-note lock map grows without bound and is not pruned; a panic while holding a note's lock poisons it for every later caller. | `crates/storage/src/vault.rs` | Open |
| M8 | The migration mechanism's test names (`example_pair_v1_migrates...`, etc.) overstate what's proven — `MigrationError::UnsupportedVersion` is never actually constructed or tested (flagged originally by Task 7's own review). | `crates/domain/src/migration.rs` | Open |
| M9 | The `---`-frontmatter/body split rule lives only inside `storage` (`propose.rs`'s private splitter); a future reader of `domain` or of raw vault records has no shared place to find the same rule. | `crates/storage/src/propose.rs` | Open |
| M10 | Pending-reference matching happens before any identity rule: a case-variant of a note's own id (e.g. `NOTE-A` declared pending while the note is `note-a`), or even an empty string, can be accepted as pending if the caller supplies it. In scope of I4 (caller-supplied `pending_ids`), but worth naming concretely. | `crates/storage/src/propose.rs` | Open |
| M11 | `ensure_resolves_inside_root`'s error handling: any `symlink_metadata` error other than "not found" falls through to the parent-only check rather than failing closed uniformly for every error kind. | `crates/storage/src/vault.rs` | Open |
| M12 | `CommitOutcome.link_state` has no field-level doc comment (the struct-level doc covers it, but the field itself doesn't repeat it). | `crates/storage/src/propose.rs` | Open |
| M13 | `clippy --all-targets` still fails, only on the 3 pre-existing `assert!(true)` placeholder tests from Task 1's scaffolding (CI only runs plain `clippy --workspace`, which is clean). | `crates/{domain,policy,storage}/src/lib.rs` | Open (pre-existing, re-confirmed each review round) |

None of M1–M13 is a merge blocker for A1; all are routine cleanup or forward-looking notes for later phases. I3 (no CI secret-scanner/mutation-testing step), I4 (`known_ids`/`pending_ids`/`SecretPolicy` fully caller-supplied, no registered-policy store) and the remaining half of I5 (the `.record`-vs-plain-Markdown on-disk format question) remain open exactly as described in Section 6 and its update note — unchanged by this review.

### 10.3 Verification after the fourth review (read-only — no code change)

| Command | Result |
|---|---|
| `cargo build --workspace` | success |
| `cargo test --workspace` | **106 passed, 0 failed** (23 domain, 12 policy, 44 storage lib, 22 integration, 5 doctests) — unchanged, since no code changed |
| `cargo clippy --workspace -- -D warnings` | clean |
| `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` |
| `cargo mutants --package domain --package storage --package policy` | **72 tested, 58 caught, 14 unviable, 0 missed** — exact match to the third wave |

Documentation-only commits for this round: `703f8a0` (SEC-001 entry + plan cross-reference), `6e274d8` (SEC-002 entry), and this section's own commit (SEC-002 correction, SEC-003 entry, plan file update, this Section 10).
