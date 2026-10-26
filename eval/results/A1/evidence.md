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
| `propose_and_commit` is the only `pub` write function; raw primitive unreachable outside the crate | PASS | `storage_exposes_exactly_one_public_write_function` (Task 9, textual/regression guard); `write_record_unchecked` is `pub(crate)`, not `pub` — see Section 1's clarification |
| `note_id` validated before any path use; symlink-escape checked | PASS | `NoteId::parse` (Task 6) rejects empty/separator/`..`/absolute; `Vault::record_path_checked` canonicalizes and compares against vault root; `vault::tests::write_target_resolving_outside_vault_root_via_symlink_is_rejected` |
| Every typed error names the offending field/condition | PASS | `ValidationError::{MissingField,InvalidFieldType,InvalidEnumValue}` all carry a field name; `LinkError::{BrokenLink,SelfReference}` carry relation+id; `StorageError::Conflict`/`PathEscapesVaultRoot`/`NotFound`/`InvalidNoteId` all carry `note_id` |
| Secret-policy exception scoped to exact occurrence (pattern+note+hash) | PASS | `SecretException{pattern_id, matched_text_hash, note_id,...}`; `policy::gate::tests::exception_for_one_occurrence_does_not_cover_a_different_matched_value` |
| No wall-clock sleeps in concurrency tests | PASS | `concurrent_writers_sharing_one_vault_instance_on_same_version_only_one_succeeds` uses `std::sync::Barrier`, not `sleep` |
| Evidence for every acceptance criterion in `eval/results/A1/` | PASS | This file, alongside `eval/results/A1/dependency-exceptions.md` (Task 1, preserved unmodified) |
| No index/agent/api/model code | PASS | No such modules exist in any A1 crate |
| A1 not reported as providing crash recovery or cross-restart idempotency | PASS | Boundary statement in Section 1, verbatim per the brief |

### 4.2 Review Focus items

| Item | Owning task | Status |
|---|---|---|
| Unknown/extra frontmatter field, or wrong YAML type for a field | Task 2 | PASS — `note::tests::unknown_classification_value_is_named_field_error`, `wrong_type_for_created_is_named_field_error` |
| Self-reference in `relations.*` | Task 3 | PASS — `links::tests::self_reference_is_rejected_not_silently_resolved` |
| Classification value outside the four valid enum members | Task 2 | PASS — `note::tests::unknown_classification_value_is_named_field_error` |
| Concurrent writer retries with a new expected version after losing a conflict | Task 6 | PASS — `vault::tests::losing_writer_can_retry_with_fresh_version_and_succeed` |
| Expected version collides numerically with a different note's version | Task 6 | PASS — `vault::tests::version_numbers_do_not_collide_across_different_notes` |
| `note_id` crafted to escape the vault root (traversal / symlink) | Task 6 | PASS — `note_id::tests::*_is_rejected` (traversal) and `vault::tests::write_target_resolving_outside_vault_root_via_symlink_is_rejected` (symlink) |

### 4.3 Task 9's own deliverables

| Deliverable | Status | Evidence |
|---|---|---|
| `propose_and_commit` is the sole public write path, gate-backed | PASS | `crates/storage/src/propose.rs`; `storage_exposes_exactly_one_public_write_function` |
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
