# B5 — Brain API + MCP Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the open filesystem TOCTOU gap, then build a Brain API service layer and MCP server/client so external coding agents can use SuperBrain's search/retrieval/knowledge/memory/rules capabilities over MCP, with all writes gated behind a human-approved proposal mechanism.

**Architecture:** `crates/brain-api` (new) sits between MCP and the existing read-only `crates/api` + `crates/storage`/`policy`/`domain`/`index`/`codegraph-index`. It delegates reads to `api` unchanged, owns `PendingWriteProposal` persistence and lifecycle, and is the only caller of `storage::propose_and_commit` for approved proposals. A new MCP server crate wraps `brain-api`; a separate MCP client crate lets SuperBrain call external tools under its own permission gate. No component skips `brain-api` to reach `storage` directly.

**Tech Stack:** Rust (2021 edition), existing workspace crates (`domain`, `policy`, `storage`, `vault-security`, `index`, `codegraph-domain`, `codegraph-index`, `api`), `windows` crate (0.62, `Win32_Storage_FileSystem`) for Windows-specific handle verification, an MCP protocol crate for the server/client transport (exact crate chosen in Task 15 after checking what's available/maintained — this is an implementation detail, not a design decision).

**Spec:** `docs/superpowers/specs/2026-11-15-b5-brain-api-mcp-design.md` — the plan argues from this spec; executors should read both.

## Global Constraints

- `storage::propose_and_commit` (`crates/storage/src/propose.rs:179`) remains the only function that commits Note bytes to the vault — nothing new bypasses it.
- `crates/api`'s read-only, no-transport contract (`crates/api/src/lib.rs:1`) is never modified to add write/proposal semantics; all write/proposal code goes in the new `crates/brain-api`.
- No caller (MCP server, future B6 UI, internal code) reaches `storage` directly except through `brain-api`'s approval-gated execution path.
- An MCP-originated `RequestContext`/policy object is always constructed with `approve_write_allowed: false`, hard-coded at the construction site — never a value the transport can set.
- `PendingWriteProposal`'s `originating_identity` is derived from the MCP connection/session binding (B5.8), never from a client-supplied request field.
- Proposal status transitions are validated server-side via an explicit pairwise table (modeled on `domain::write_state::valid_transition`, `crates/domain/src/write_state.rs:12-32`); nothing moves a proposal backward; only `approve_proposal`/`reject_proposal` may move Pending → Approved/Rejected.
- Permission checks happen before a proposal is loaded or its transition validated (no information leak to unauthorized callers).
- DenseV2 retrieval behavior (B4, frozen) is never modified.
- No new crate introduces a network call that isn't explicitly part of the MCP transport itself.
- `B5.SEC` (Tasks 1–6) must be fully closed — implementation, adversarial tests, review, documented residual risk — before any task in Tasks 7+ begins (hard gate; see Task 6's acceptance criteria).

## Review Focus

- **A proposal that outlives its `expires_at` while still persisted as `Pending`**: a reasonable person expects `approve_proposal` to refuse it, not just the listing view to hide it. Test owned by Task 23 (proposal approval execution).
- **Two concurrent `approve_proposal` calls racing on the same proposal**: a reasonable person expects exactly one to succeed and the Note to be written exactly once, never a double-commit or a double-spend of the same proposal. Test owned by Task 23.
- **An MCP tool call with a syntactically valid but semantically empty/zero-length query or payload** (e.g. `search` with `query: ""`, `save_memory` with an empty body): a reasonable person expects a typed `InvalidInput` error, not a panic or a silently-empty success. Test owned by Task 16 (search/retrieve MCP tools) and Task 18 (memory proposal tools).
- **A hard-link or symlink swapped into a vault-root-relative path between B5.SEC's fix and B5.1's new proposal-storage code reusing the same primitive incorrectly** (i.e., a future task writing a new unchecked `root.join(...)` instead of calling the new verified-open primitive): a reasonable person expects every new path-touching call site introduced after Task 6 to go through the Task 6 primitive, not a fresh unchecked join. Enforced by Task 6's own compile-time/architecture test plus Task 9's proposal-storage tests reusing it explicitly.
- **An external MCP tool (client side, Tasks 27–30) returning a malformed or oversized response, or one shaped to look like a different tool's output**: a reasonable person expects SuperBrain to reject it as `ExternalToolUnavailable`/`InvalidInput`, never execute it or store it as trusted data. Test owned by Task 29 (MCP client foundation) and Task 33 (adversarial).

---

## File Structure

```
crates/storage/src/
  handle_safety.rs       (new) — VerifiedRecordHandle, open_verified, Windows+Unix identity checks
  vault.rs                (modify) — record_path_checked/before_image_path_checked rewired to open_verified
  recovery.rs, restore.rs (modify) — same rewire at their 5 call sites
  Cargo.toml               (modify) — add windows crate as cfg(windows) dependency

crates/brain-api/           (new crate)
  Cargo.toml
  src/lib.rs                — RequestContext (write-capable), ApiError (typed error set), dispatch table
  src/read.rs                — thin delegation wrappers to crates/api for the 9 read-shaped capabilities
  src/policy_write.rs         — WritePolicy, WriteOperation (lives here, not in crates/policy, since
                                 the only write-shaped policy consumer is brain-api; see Task 10 rationale)
  src/proposal.rs             — PendingWriteProposal, ProposalId, ProposalStatus, ProposalProvenance,
                                 valid_proposal_transition
  src/proposal_store.rs       — .proposals/ persistence (uses storage::handle_safety primitives)
  src/graph.rs                — graph_query, get_entity, link_entities-as-proposal
  src/memory.rs               — get_previous_solution, save_memory/update_memory-as-proposal
  src/knowledge.rs            — get_rules, get_regulations, get_canon
  src/approval.rs             — approve_proposal, reject_proposal, list_proposals (CLI-only origin)
  src/audit.rs                — structured audit event emission (B5.12)
  src/error.rs                — the frozen typed error enum shared by every operation

crates/mcp-server/          (new crate)
  Cargo.toml
  src/lib.rs                 — server bootstrap, tool/resource registration
  src/tools/*.rs              — one file per capability group, each a thin MCP-schema wrapper over brain-api

crates/mcp-client/           (new crate)
  Cargo.toml
  src/lib.rs                  — server registration/config, connection lifecycle
  src/permission.rs            — external tool authorization model (B5.9)
  src/audit.rs                  — invocation audit records

docs/research/B5-BRAIN-API-MCP-CLOSURE.md  (new, Task 36)
docs/superpowers/specs/2026-11-15-b5-brain-api-mcp-design.md (existing, read-only reference)
```

---

## Task 1: `VerifiedRecordHandle` — Unix identity verification primitive

**Files:**
- Create: `crates/storage/src/handle_safety.rs`
- Modify: `crates/storage/src/lib.rs` (add `mod handle_safety;`, no new pub export yet — stays `pub(crate)` like `atomic`)
- Test: inline `#[cfg(test)] mod tests` in `handle_safety.rs`

**Interfaces:**
- Consumes: nothing new (uses `std::fs`, `std::os::unix`/`std::os::windows`).
- Produces: `pub(crate) struct VerifiedRecordHandle { file: std::fs::File, path: PathBuf }`, `pub(crate) enum OpenMode { ReadExisting, CreateOrOpenForWrite }`, `pub(crate) fn open_verified(root: &Path, candidate_relative: &Path, mode: OpenMode) -> Result<VerifiedRecordHandle, StorageError>`. Later tasks (4, 5, 9) call `open_verified` and read/write through `VerifiedRecordHandle`'s `.file` field (crate-visible) instead of `record_path_checked` + a separate `std::fs::read`/`atomic_write` call.

- [ ] **Step 1: Write the failing test for Unix link-count rejection**

```rust
#[cfg(all(test, unix))]
#[test]
fn rejects_a_file_with_more_than_one_hard_link() {
    let dir = unique_test_dir();
    let target = dir.join("real.record");
    std::fs::write(&target, b"content").unwrap();
    let alias = dir.join("alias.record");
    std::fs::hard_link(&target, &alias).unwrap();
    let result = open_verified(&dir, Path::new("alias.record"), OpenMode::ReadExisting);
    assert!(matches!(result, Err(StorageError::PathEscapesVaultRoot { .. })));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p storage handle_safety::tests::rejects_a_file_with_more_than_one_hard_link`
Expected: FAIL with "cannot find function `open_verified`" (module doesn't exist yet).

- [ ] **Step 3: Implement `open_verified` for the Unix link-count + root-confinement check**

In `crates/storage/src/handle_safety.rs`:
- Build the candidate path as `root.join(candidate_relative)`.
- Open with `std::fs::OpenOptions` set for the given `OpenMode` (`ReadExisting`: `.read(true)`; `CreateOrOpenForWrite`: `.read(true).write(true).create(true)`), plus `O_NOFOLLOW` via `std::os::unix::fs::OpenOptionsExt::custom_flags(libc::O_NOFOLLOW)` — this crate has no `libc` dependency yet, so either add it (`libc = "0.2"` to `Cargo.toml`) or use the `rustix` crate already available transitively; prefer `libc` since it's the minimal, most common choice and nothing else in the workspace already pulls in `rustix`.
- On the opened handle: `file.metadata()?`, reject if `!metadata.is_file()` (catches a symlink that `O_NOFOLLOW` turned into an open failure already, but also catches other non-regular-file reparse-like entries), reject if `std::os::unix::fs::MetadataExt::nlink(&metadata) != 1`.
- Verify root confinement on the **handle**, not the path string: canonicalize `root` once, then resolve the open fd's real path via `/proc/self/fd/<fd>` readlink (Linux-specific; document this as the Unix mechanism, since the design doc flagged this as the one open "which syscall" question, and `/proc/self/fd` readlink is the standard, simplest answer for a project whose two target platforms are Windows+Linux dev machines) using `std::os::unix::io::AsRawFd` to get the fd, then `std::fs::read_link(format!("/proc/self/fd/{fd}"))`, and check `.starts_with(&canonical_root)`.
- Any failure at any step returns `StorageError::PathEscapesVaultRoot { note_id: ... }` (reuse this existing variant — the design doc's stated rationale: a symlink, a multi-linked file, and an out-of-root resolution all mean the same thing to the caller, "this path is not trustworthy").
- Return `Ok(VerifiedRecordHandle { file, path: candidate })` on success.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p storage handle_safety::tests::rejects_a_file_with_more_than_one_hard_link`
Expected: PASS

- [ ] **Step 5: Write and pass the companion tests**

```rust
#[test]
fn accepts_a_plain_single_link_file_inside_root() { /* open_verified Ok(_) for a normal file */ }

#[test]
fn rejects_a_symlink_at_the_final_component_even_when_its_target_is_inside_root() {
    // symlink real.record -> alias.record, both inside root; O_NOFOLLOW must
    // make the open itself fail on the symlink, not merely detect it after.
}

#[test]
fn rejects_when_nothing_exists_and_create_or_open_for_write_is_used_but_parent_is_a_symlinked_directory() {
    // dir/subdir is a symlink to somewhere outside root; candidate is
    // dir/subdir/new.record (doesn't exist yet) — CreateOrOpenForWrite must
    // still reject via the parent-resolution path.
}
```

Run: `cargo test -p storage handle_safety`
Expected: all PASS

- [ ] **Step 6: Commit**

```bash
git add crates/storage/src/handle_safety.rs crates/storage/src/lib.rs crates/storage/Cargo.toml
git commit -m "feat(storage): add VerifiedRecordHandle Unix identity verification (SEC-001/002/003, part 1)"
```

## Task 2: `VerifiedRecordHandle` — Windows identity verification

**Files:**
- Modify: `crates/storage/src/handle_safety.rs`
- Modify: `crates/storage/Cargo.toml` (add `[target.'cfg(windows)'.dependencies] windows = { version = "0.62", features = ["Win32_Foundation", "Win32_Storage_FileSystem"] }`, mirroring `crates/vault-security/Cargo.toml`'s existing stanza exactly)

**Interfaces:**
- Consumes: Task 1's `OpenMode`, `VerifiedRecordHandle` struct shape (same struct, `#[cfg(windows)]` branch of `open_verified`'s body).
- Produces: same `open_verified` signature now also correct on Windows (the function is one signature with `#[cfg(unix)]`/`#[cfg(windows)]` bodies, not two functions).

- [ ] **Step 1: Write the failing test for Windows link-count rejection**

```rust
#[cfg(all(test, windows))]
#[test]
fn rejects_a_file_with_more_than_one_hard_link() {
    let dir = unique_test_dir();
    let target = dir.join("real.record");
    std::fs::write(&target, b"content").unwrap();
    let alias = dir.join("alias.record");
    std::fs::hard_link(&target, &alias).unwrap();
    let result = open_verified(&dir, Path::new("alias.record"), OpenMode::ReadExisting);
    assert!(matches!(result, Err(StorageError::PathEscapesVaultRoot { .. })));
}

#[cfg(all(test, windows))]
#[test]
fn rejects_a_directory_junction_at_the_final_component() {
    // Uses `mklink /J` via std::process::Command in test setup (junctions
    // have no stable std-only creation API); skip with a printed message if
    // junction creation fails in a sandboxed CI environment rather than
    // failing the test suite on an environment limitation.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p storage handle_safety::tests --target x86_64-pc-windows-msvc` (or default target on a Windows dev machine)
Expected: FAIL — Windows branch not implemented yet (either compile error or wrong-variant assertion failure).

- [ ] **Step 3: Implement the Windows branch of `open_verified`**

- Open via `std::fs::OpenOptions` with `std::os::windows::fs::OpenOptionsExt::custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT.0)` so a reparse point (symlink or junction) at the final component is opened as the reparse point itself, not followed.
- On the handle: `std::os::windows::io::AsRawHandle` + `windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION}` (same call pattern as `crates/vault-security/src/deletion_log.rs:574-597`'s `identity_of_open_file`) to read `dwFileAttributes` (reject if `FILE_ATTRIBUTE_REPARSE_POINT` bit is set — the open succeeded only because we asked to open the reparse point itself; its presence means this is a symlink/junction, which must be rejected outright, never followed or treated as equivalent to a regular file), `nNumberOfLinks` (reject if `!= 1`), `dwVolumeSerialNumber`/`nFileIndexHigh`/`nFileIndexLow` (not needed for the root-confinement check here, since `GetFinalPathNameByHandleW` is used instead — see next point — but worth noting these are the same fields `identity_of_open_file` already reads for its own device+inode-equivalent check).
- Root confinement: `windows::Win32::Storage::FileSystem::GetFinalPathNameByHandleW` on the open handle to get its OS-resolved real path, compare `.starts_with(&canonical_root)`.
- Any failure returns `StorageError::PathEscapesVaultRoot`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p storage handle_safety` (on a Windows dev machine, per this project's actual dev environment)
Expected: PASS (junction test either passes or is skipped with a printed diagnostic, never silently green on an untested path)

- [ ] **Step 5: Commit**

```bash
git add crates/storage/src/handle_safety.rs crates/storage/Cargo.toml
git commit -m "feat(storage): add VerifiedRecordHandle Windows identity verification (SEC-001/002/003, part 2)"
```

## Task 3: Extend `identity_of_open_file` is NOT done — document why `handle_safety` is a new module, not an extension of `deletion_log.rs`

This task is a documentation-only correction step, not new code — included because the design doc's Approach A text says B5.SEC "extends the existing `identity_of_open_file`/`FileIdentity` pattern" (`crates/vault-security/src/deletion_log.rs:574-597`), but Tasks 1–2 above actually wrote a **new, separate** function in `storage::handle_safety`, not a modification of `vault_security::deletion_log::identity_of_open_file`. This is intentional and correct — `identity_of_open_file` is `vault-security`-crate-private (not `pub`, not re-exported) and solves a narrower problem (device+inode equality between two already-open handles for the deletion log's own self-consistency check); `storage`'s TOCTOU fix needs link-count and reparse-point rejection, which `identity_of_open_file` has neither of, and `storage` cannot reach into `vault-security`'s private function anyway. Reusing the *pattern* (open a handle, call `GetFileInformationByHandle`/`MetadataExt`, reject on any anomaly) while writing a new function in the crate that actually needs it is the correct move, not a deviation — but the design doc's wording is imprecise enough that a future reader could expect `deletion_log.rs` itself to change. No code changes in this task.

- [ ] **Step 1: Add a one-line doc comment to `handle_safety.rs`'s module doc clarifying this relationship**

```rust
//! Pattern note: this module's handle-verification approach (open, then call
//! GetFileInformationByHandle/MetadataExt on the open handle, reject on any
//! anomaly) follows the same shape as
//! `vault_security::deletion_log::identity_of_open_file`, but is a separate,
//! new function — not a modification of that one, which is crate-private to
//! `vault-security` and solves a narrower problem (device+inode equality
//! between two already-open handles). This module additionally checks link
//! count and rejects reparse points/symlinks, which `identity_of_open_file`
//! does not.
```

- [ ] **Step 2: Commit**

```bash
git add crates/storage/src/handle_safety.rs
git commit -m "docs(storage): clarify handle_safety's relationship to vault-security's identity_of_open_file pattern"
```

## Task 4: Rewire `record_path_checked`'s 4 `vault.rs` call sites to `open_verified`

**Files:**
- Modify: `crates/storage/src/vault.rs` (lines ~887, ~1194, ~1460, ~1708 — exact line numbers will have shifted slightly after Tasks 1–3's additions; locate by the `self.record_path_checked(note_id)?` / `self.record_path_checked(&note_id)?` call pattern)
- Test: inline tests in `vault.rs`'s existing `#[cfg(test)] mod tests` block, alongside the existing `sec_001`/`sec_002`/`sec_003` oracle tests

**Interfaces:**
- Consumes: Task 1/2's `storage::handle_safety::{open_verified, OpenMode, VerifiedRecordHandle}`.
- Produces: `read_note_locked`, `write_record_impl` (and whichever functions own vault.rs:1460/1708 — confirm their names by reading those two call sites directly before writing this task's steps, since they were not individually identified during design) now read/write through a `VerifiedRecordHandle`'s `.file` instead of a bare path.

- [ ] **Step 1: Invert the existing vulnerability-confirming test `record_symlink_resolving_inside_the_vault_root_is_not_an_escape`**

Locate this test (around vault.rs:2993-3018 per design-doc recon). Rewrite its assertion from "read succeeds through the symlink" to "read is rejected":

```rust
#[test]
fn record_symlink_resolving_inside_the_vault_root_is_rejected_as_an_alias_not_followed() {
    // (renamed from `...is_not_an_escape`) — SEC-002 closure: an in-root
    // symlink at a .record path must now be REJECTED, not read through.
    let dir = unique_test_dir();
    // ... same setup as before ...
    let result = vault.read_note(&note_id);
    assert!(matches!(result, Err(StorageError::PathEscapesVaultRoot { .. })));
}
```

- [ ] **Step 2: Run it to verify it now fails (the old behavior still stands until this task's implementation step)**

Run: `cargo test -p storage record_symlink_resolving_inside_the_vault_root_is_rejected_as_an_alias_not_followed`
Expected: FAIL (old code still reads through the symlink)

- [ ] **Step 3: Rewire the 4 call sites to use `open_verified`**

Replace each `let path = self.record_path_checked(note_id)?; /* ...later... */ std::fs::read(&path)` (or `atomic_write(&record_path, ...)`) pair with: `let handle = crate::handle_safety::open_verified(&self.root, &relative_record_path(note_id), OpenMode::ReadExisting)?;` then read via `handle.file` (e.g. `let mut buf = Vec::new(); handle.file.read_to_end(&mut buf)?;` — wrap any `std::io::Error` into `StorageError::Io` exactly as the existing code already does). For the write call site (vault.rs ~1194/1239/1299/1313 region, inside whatever function owns `write_record_impl`'s body), the current-bytes read and the post-write verify read both move to `OpenMode::ReadExisting`/re-open-and-reread through a fresh `open_verified` call (each distinct read is its own verified-open, not one handle reused across the whole function, since the write in between changes the file); the `atomic_write` call itself stays as today (Task 5 handles making its own temp-file/rename path verified-safe) but the content-hash verify read after it must go through `open_verified` too.

- [ ] **Step 4: Run the inverted test and all existing vault tests**

Run: `cargo test -p storage vault::`
Expected: the inverted test now PASSes; run the full `vault` test module and fix any other test whose setup assumed the old check-then-use behavior (expect the `sec_001`/`sec_002` oracle tests, handled in Task 6, to still fail at this point — that's correct, they're inverted in Task 6 once all call sites are done).

- [ ] **Step 5: Commit**

```bash
git add crates/storage/src/vault.rs
git commit -m "fix(storage): rewire vault.rs's 4 record_path_checked call sites to VerifiedRecordHandle (SEC-001/002/003)"
```

## Task 5: Rewire `before_image_path_checked`'s 3 callers and `atomic_write`'s rename-destination safety

**Files:**
- Modify: `crates/storage/src/vault.rs` (before-image callers at ~2204, ~2216, ~2230; `before_image_path_checked` itself at ~2177)
- Modify: `crates/storage/src/atomic.rs` (the `atomic_write` function itself)

**Interfaces:**
- Consumes: Task 1/2's `open_verified`.
- Produces: `atomic_write`'s signature changes to accept an already-verified parent directory context rather than a bare `&Path`, OR (simpler, equivalent guarantee) `atomic_write` itself calls `open_verified` on the **parent directory** of its target to confirm the parent is inside the vault root and not a symlinked directory, before constructing its temp-file path inside that parent. Exact signature: `pub(crate) fn atomic_write(verified_parent: &VerifiedDirHandle, file_name: &str, contents: &[u8]) -> Result<(), AtomicWriteError>` — this is a breaking change to `atomic_write`'s signature, so every caller (the 4 in Task 4 plus the before-image callers here) must be updated together; decide in this task whether `VerifiedDirHandle` is a second small type in `handle_safety.rs` (a directory-only analog of `VerifiedRecordHandle`) or whether `atomic_write` instead takes the full target path plus the caller's already-done `open_verified` result for the parent — pick whichever keeps `atomic_write`'s existing callers (Task 4's 4 call sites) simplest to update, since both achieve the same safety property.

- [ ] **Step 1: Write the failing test for before-image path hard-link aliasing (the sec_003 gap this specific call path has never had its own test for, per the design doc's "new adversarial tests needed" note)**

```rust
#[test]
fn before_image_write_rejects_a_hard_linked_before_images_directory_entry() {
    // Set up .before-images/<note>-<op>.record as a hard link to a file
    // outside the vault root; persist_before_image must reject, not write
    // through the alias.
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p storage before_image_write_rejects_a_hard_linked_before_images_directory_entry`
Expected: FAIL

- [ ] **Step 3: Implement the rewire**

Change `before_image_path_checked`'s 3 call sites and `atomic_write`'s internal temp-file/rename logic to use `open_verified`/the new directory-verification primitive from Task 1/2, following the same pattern as Task 4's rewire. The rename's *destination* name is still just a name (POSIX/Windows rename never follows a symlink at the destination, which is already safe per the design doc's own analysis) — what this task closes is the *parent directory* being symlink/hard-link-aliased, and the *source temp file* being read back for integrity verification through an unverified path.

- [ ] **Step 4: Run the test and the full atomic/vault test suites**

Run: `cargo test -p storage`
Expected: PASS, including the Task 4 inverted test and this task's new test.

- [ ] **Step 5: Commit**

```bash
git add crates/storage/src/vault.rs crates/storage/src/atomic.rs
git commit -m "fix(storage): close before-image and atomic_write rename-parent aliasing gaps (SEC-001/002/003)"
```

## Task 6: Rewire `recovery.rs` (4 sites) and `restore.rs` (1 site); invert the 4 SEC-00x drift oracles; close B5.SEC

**Files:**
- Modify: `crates/storage/src/recovery.rs` (lines ~273, ~335, ~750, ~963)
- Modify: `crates/storage/src/restore.rs` (line ~490)
- Modify: `crates/storage/src/vault.rs` (the 4 `sec_001`/`sec_002`/`sec_003` oracle tests around ~4206-4584, plus the "SEC-00x status anchor" comment block)
- Modify: `docs/security/security-audit.md` (update SEC-001/002/003 rows from open to closed, with a pointer to this task's commits)

**Interfaces:**
- Consumes: Task 1/2's `open_verified`.
- Produces: B5.SEC is CLOSED — nothing downstream in this plan may build on `record_path_checked`'s old pattern again.

- [ ] **Step 1: Rewire `recovery.rs`'s 4 call sites and `restore.rs`'s 1 call site to `open_verified`**

Same mechanical rewire as Task 4, applied to `recovery.rs:273,335,750,963` and `restore.rs:490`. These run during crash recovery (single-process reconciliation context) — per the design doc's resolved open question, they get the same handle-verification primitive, not a deferred/weaker check.

- [ ] **Step 2: Invert all four SEC-00x drift-oracle tests in `vault.rs`**

- `sec_001_toctou_window_between_path_check_and_write_still_exists` → rename to `sec_001_toctou_window_between_path_check_and_write_is_closed`, invert assertion: the symlink-swap-during-the-window attempt must now fail with `StorageError::PathEscapesVaultRoot`, not succeed.
- `sec_001_toctou_read_side_leaks_an_outside_records_version_and_bytes_into_the_before_image` → rename to `...is_rejected_not_leaked`, invert: the before-image capture must reject rather than capture the outside file's bytes.
- `sec_002_read_path_rejects_both_aliasing_shapes_but_write_path_pre_read_does_not` → rename to `sec_002_both_read_and_write_paths_reject_both_aliasing_shapes`, invert the write-path assertion.
- `sec_003_hard_link_aliasing_is_invisible_to_canonicalization_after_a2` → rename to `sec_003_hard_link_aliasing_is_rejected_on_both_read_and_write`, invert the write-path assertion (the read-side sub-case already passed before this task, per the design doc's recon).

- [ ] **Step 3: Update the "SEC-00x status anchor" comment block** (per its own instruction, update it and continuity in the same change)

Edit the comment at vault.rs:4562-4584 to state SEC-001/002/003 closed, with the commit hash of this task once committed.

- [ ] **Step 4: Run the full storage crate test suite**

Run: `cargo test -p storage`
Expected: ALL PASS, including all 4 inverted oracles, Task 4's inverted symlink test, Task 5's before-image test, and every pre-existing test (none of which should have needed changes beyond what Tasks 4–5 already touched).

- [ ] **Step 5: Add the remaining adversarial tests the design doc's security-semantics section requires, not yet covered**

```rust
#[test]
fn concurrent_symlink_substitution_during_recovery_reconcile_is_rejected() { /* recovery.rs path */ }

#[test]
fn concurrent_symlink_substitution_during_restore_promote_is_rejected() { /* restore.rs path */ }

#[test]
fn first_write_hard_link_scenario_is_rejected() {
    // sec_003's existing oracle only covered the existing-note write case;
    // this covers a first-write (no existing .record file yet) where the
    // PARENT directory entry for the new file name is itself a hard-link-
    // adjacent trick — confirm this is reachable/meaningful for a first
    // write (hard links alias existing files, not not-yet-created ones, so
    // this test's real content is: a sibling hard-linked file must not be
    // mistaken for the candidate during the "nothing exists yet" parent-dir
    // resolution path) and assert rejection or no-op as appropriate once
    // the actual mechanics are worked out while writing this test.
}

#[test]
fn rename_replace_race_between_verified_open_and_final_rename_is_closed_or_documented() {
    // If a residual race remains at the rename step specifically (the design
    // doc's own analysis says POSIX/Windows rename-to-existing-name never
    // follows a destination symlink, so this is expected to pass showing the
    // residual risk is already closed by rename semantics, not by this
    // primitive) — if it does NOT pass, this is a real finding, write up
    // what's left as residual risk for the closure document (Task 36)
    // rather than silently weakening the test.
}
```

Run: `cargo test -p storage`
Expected: PASS, or a documented residual-risk finding captured for Task 36 if `rename_replace_race_...` doesn't pass cleanly.

- [ ] **Step 6: Update `docs/security/security-audit.md`'s SEC-001/002/003 rows**

Change status from open/deferred to closed, citing this task's commit range.

- [ ] **Step 7: Commit**

```bash
git add crates/storage/src/recovery.rs crates/storage/src/restore.rs crates/storage/src/vault.rs docs/security/security-audit.md
git commit -m "fix(storage): close SEC-001/002/003 across recovery/restore paths; invert drift oracles; B5.SEC CLOSED"
```

**B5.SEC gate check (must pass before any task below starts):** `cargo test -p storage` all green, `cargo clippy -p storage --all-targets --all-features -- -D warnings` clean, all 4 oracles inverted and passing, `docs/security/security-audit.md` updated. If the rename-race test in Step 5 revealed a real residual risk, record it now as a continuity constraint before proceeding, since Task 36's closure doc will need it either way.

---

## Task 7: Freeze the Brain API contract schemas (B5.0)

**Files:**
- Create: `docs/superpowers/specs/2026-11-15-b5-0-brain-api-contract-freeze.md`

**Interfaces:**
- Consumes: the design doc's capability table.
- Produces: the exact input/output/error schema for all 12 capabilities that Tasks 10–23 implement against. This is a documentation task — no code — but it's the one place "what are the exact fields" gets decided instead of guessed per-task.

- [ ] **Step 1: Ground `get_rules`/`get_regulations`/`get_canon` scope against real existing docs**

Search `docs/` for any existing formal definition of "engineering constitution," "project rules," "regulations," "canon." Read whatever is found (candidates to check: `docs/Spec_mvp.md`, any `docs/rules/` or `docs/canon/` directory, `CLAUDE.md`-equivalent files in this repo if any). Record in the new doc: what each term actually refers to in this project today, their precedence order if any exists, and if no formal definition exists anywhere, propose one grounded in what the user actually asked for in the B5 brief ("engineering constitution; project rules; regulations esterne; canon" as four distinct things) and flag it explicitly as a new definition being introduced here, not found.

- [ ] **Step 2: Write the schema for each of the 12 capabilities**

For each: stable name, input fields (name, type, required/optional, constraints), output fields, which of the frozen error variants it can return, permission requirement (`ReadOperation::X` or `WriteOperation::X`), determinism statement, empty/not-found behavior, pagination/limit defaults (exact numbers deferred to Task 34, but note here which capabilities need them: `search`, `graph_query`, `get_previous_solution` at minimum).

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-11-15-b5-0-brain-api-contract-freeze.md
git commit -m "docs(B5.0): freeze Brain API contract schemas for all 12 capabilities"
```

## Task 8: Scaffold `crates/brain-api` crate

**Files:**
- Create: `crates/brain-api/Cargo.toml`
- Create: `crates/brain-api/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members` — add `"crates/brain-api"`)

**Interfaces:**
- Consumes: `api`, `storage`, `policy`, `domain` crates (path dependencies).
- Produces: an empty-but-building crate; `pub struct RequestContext { pub schema_version: u32, pub correlation_id: uuid::Uuid, pub write_policy: Option<WritePolicy> }` (the `Option<WritePolicy>` field this task adds — note this is a NEW struct in `brain-api`, not a modification of `api::RequestContext`, per the frozen constraint that `api` is untouched); re-exports to be filled in by later tasks.

- [ ] **Step 1: Write the crate-builds test**

```rust
#[test]
fn crate_builds() { assert!(true); }
```

- [ ] **Step 2: Run it to verify it fails (crate doesn't exist/compile yet)**

Run: `cargo test -p brain-api`
Expected: FAIL (no such package)

- [ ] **Step 3: Write `Cargo.toml` and minimal `lib.rs`**

```toml
[package]
name = "brain-api"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
publish = false
description = "SuperBrain Brain API orchestration layer: read delegation, write-proposal lifecycle, approval-gated execution."

[dependencies]
api = { path = "../api" }
storage = { path = "../storage" }
policy = { path = "../policy" }
domain = { path = "../domain" }
index = { path = "../index" }
uuid = { version = "1", features = ["v4"] }
```

Add `"crates/brain-api"` to the workspace `members` list in the root `Cargo.toml`.

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p brain-api`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/brain-api/Cargo.toml crates/brain-api/src/lib.rs Cargo.toml
git commit -m "feat(brain-api): scaffold new crate for write-proposal/approval orchestration"
```

## Task 9: `ProposalId`, `ProposalStatus`, `ProposalProvenance`, `valid_proposal_transition`

**Files:**
- Create: `crates/brain-api/src/proposal.rs`
- Modify: `crates/brain-api/src/lib.rs` (add `mod proposal; pub use proposal::{ProposalId, ProposalStatus, ProposalProvenance, PendingWriteProposal, valid_proposal_transition};`)
- Test: inline in `proposal.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: the exact types from the design doc's "PendingWriteProposal (concrete shape)" section — `ProposalId(String)` with a validating constructor (following `storage::NoteId`'s real shape), `ProposalStatus { Pending, Approved { approved_at: String }, Rejected { reason: String, rejected_at: String }, Expired }`, `ProposalProvenance { source: String, created_at: String }`, `PendingWriteProposal { id, originating_identity, operation, provenance, expires_at: Option<String>, status }`, `valid_proposal_transition(from: &ProposalStatus, to: &ProposalStatus) -> bool`.

- [ ] **Step 1: Write the failing tests for `valid_proposal_transition`**

```rust
#[test]
fn pending_can_advance_to_approved() {
    assert!(valid_proposal_transition(&ProposalStatus::Pending, &ProposalStatus::Approved { approved_at: "x".into() }));
}
#[test]
fn pending_can_advance_to_rejected() {
    assert!(valid_proposal_transition(&ProposalStatus::Pending, &ProposalStatus::Rejected { reason: "x".into(), rejected_at: "y".into() }));
}
#[test]
fn pending_can_advance_to_expired() {
    assert!(valid_proposal_transition(&ProposalStatus::Pending, &ProposalStatus::Expired));
}
#[test]
fn approved_is_terminal() {
    let approved = ProposalStatus::Approved { approved_at: "x".into() };
    assert!(!valid_proposal_transition(&approved, &ProposalStatus::Rejected { reason: "x".into(), rejected_at: "y".into() }));
    assert!(!valid_proposal_transition(&approved, &ProposalStatus::Pending));
}
#[test]
fn rejected_is_terminal() {
    let rejected = ProposalStatus::Rejected { reason: "x".into(), rejected_at: "y".into() };
    assert!(!valid_proposal_transition(&rejected, &ProposalStatus::Approved { approved_at: "z".into() }));
}
#[test]
fn expired_is_terminal() {
    assert!(!valid_proposal_transition(&ProposalStatus::Expired, &ProposalStatus::Approved { approved_at: "z".into() }));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p brain-api proposal::tests`
Expected: FAIL (module doesn't exist)

- [ ] **Step 3: Implement the types and the explicit pairwise match**

`valid_proposal_transition` follows `domain::write_state::valid_transition`'s style: an explicit `matches!` over `(from, to)` variant pairs, not a derived/generic state-machine library. `ProposalId`'s constructor validates non-empty, following `NoteId::parse`'s pattern (read `crates/storage/src/note_id.rs` for its exact validation rules — lowercase, charset, etc. — and decide whether `ProposalId` needs the same constraints or a simpler one, e.g. just "must parse as a UUID string," since proposal ids are generated by the system, not typed by a human the way note ids sometimes are; document the choice in a one-line comment).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p brain-api proposal::tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/brain-api/src/proposal.rs crates/brain-api/src/lib.rs
git commit -m "feat(brain-api): add PendingWriteProposal types and status-transition validation"
```

## Task 10: `WritePolicy`/`WriteOperation`

**Files:**
- Create: `crates/brain-api/src/policy_write.rs`
- Modify: `crates/brain-api/src/lib.rs`
- Test: inline

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub struct WritePolicy { propose_write_allowed: bool, approve_write_allowed: bool }` with `pub fn new(propose_write_allowed: bool, approve_write_allowed: bool) -> Self`, `pub fn allows(&self, op: WriteOperation) -> bool`, and two named constructors `pub fn mcp_agent() -> Self` (always `approve_write_allowed: false`) and `pub fn trusted_cli() -> Self` (the only constructor that can set `approve_write_allowed: true`); `pub enum WriteOperation { ProposeWrite, ApproveProposal, RejectProposal }`.

**Rationale note for this task's location:** this type lives in `brain-api`, not `crates/policy`, because its only consumer is `brain-api` (per the design doc's crate-boundary decision) and `crates/policy` is a shared dependency of `storage`/`api`/`brain-api` alike — putting a brain-api-specific concept there would leak brain-api's concerns into crates that have no business knowing about proposals. This mirrors how `ReadPolicy`/`ReadOperation` living in `policy` makes sense (both `api` and `brain-api` need read permission), but write/approval permission is brain-api-only.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn mcp_agent_policy_never_allows_approve() {
    let p = WritePolicy::mcp_agent();
    assert!(!p.allows(WriteOperation::ApproveProposal));
    assert!(!p.allows(WriteOperation::RejectProposal));
}
#[test]
fn mcp_agent_policy_allows_propose() {
    assert!(WritePolicy::mcp_agent().allows(WriteOperation::ProposeWrite));
}
#[test]
fn trusted_cli_policy_allows_approve_and_reject() {
    let p = WritePolicy::trusted_cli();
    assert!(p.allows(WriteOperation::ApproveProposal));
    assert!(p.allows(WriteOperation::RejectProposal));
}
```

- [ ] **Step 2: Run to verify failure** — Run: `cargo test -p brain-api policy_write::tests` — Expected: FAIL

- [ ] **Step 3: Implement** per the Interfaces block above.

- [ ] **Step 4: Run to verify pass** — Run: `cargo test -p brain-api policy_write::tests` — Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/brain-api/src/policy_write.rs crates/brain-api/src/lib.rs
git commit -m "feat(brain-api): add WritePolicy/WriteOperation with transport-bound constructors"
```

## Task 11: Frozen typed error enum

**Files:**
- Create: `crates/brain-api/src/error.rs`
- Modify: `crates/brain-api/src/lib.rs`

**Interfaces:**
- Produces: `pub enum BrainApiError { NotFound, InvalidInput(String), PermissionDenied, RuntimeUnavailable, DenseUnavailable, ExternalToolUnavailable, Timeout, Conflict, CorruptData, UnsupportedOperation }` — the exact frozen set from the design doc (B5.14), implementing `std::fmt::Display`/`std::error::Error` following `api::ApiError`'s existing pattern (`crates/api/src/lib.rs:49-74`).

- [ ] **Step 1: Write a failing test asserting each variant exists and displays a stable, non-empty message**

```rust
#[test]
fn every_error_variant_has_a_stable_display_message() {
    let cases = [
        BrainApiError::NotFound, BrainApiError::InvalidInput("x".into()), BrainApiError::PermissionDenied,
        BrainApiError::RuntimeUnavailable, BrainApiError::DenseUnavailable, BrainApiError::ExternalToolUnavailable,
        BrainApiError::Timeout, BrainApiError::Conflict, BrainApiError::CorruptData, BrainApiError::UnsupportedOperation,
    ];
    for case in cases { assert!(!case.to_string().is_empty()); }
}
```

- [ ] **Step 2: Run to verify failure** — Expected: FAIL (type doesn't exist)

- [ ] **Step 3: Implement**, modeled on `api::ApiError`'s `Display` impl.

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/brain-api/src/error.rs crates/brain-api/src/lib.rs
git commit -m "feat(brain-api): add frozen typed error enum"
```

## Task 12: `.proposals/` storage — persist and load

**Files:**
- Create: `crates/brain-api/src/proposal_store.rs`
- Modify: `crates/brain-api/src/lib.rs`
- Modify: `crates/storage/src/lib.rs` (export `handle_safety::{open_verified, OpenMode, VerifiedRecordHandle}` as `pub(crate)` → needs to become visible to `brain-api`; since `brain-api` is a different crate, these need to be `pub` from `storage`, scoped carefully — decide here whether to make a narrow `pub` re-export specifically for this purpose, e.g. a new `pub mod safe_io { pub use crate::handle_safety::...; }` with its own doc comment explaining it's for `brain-api`'s proposal storage only, not a general-purpose public API, OR whether `brain-api`'s proposal storage instead calls through a small new `pub fn` on `storage::Vault` itself (e.g. `Vault::verified_open_relative(&self, relative: &Path, mode: OpenMode) -> Result<VerifiedRecordHandle, StorageError>`) so `storage` keeps owning all filesystem-safety logic and `brain-api` never touches raw handles directly — **prefer the second option**: it keeps the "storage crate owns all vault-root-relative I/O safety" invariant intact rather than spreading handle-safety-aware code into a second crate.)

**Interfaces:**
- Consumes: `storage::Vault::verified_open_relative` (new method this task also adds to `storage`, exposing Task 1/2's primitive at the `.proposals/` relative path `".proposals/{proposal_id}.proposal"`), `atomic_write`'s now-verified-parent form (Task 5).
- Produces: `pub fn save_proposal(vault: &storage::Vault, proposal: &PendingWriteProposal) -> Result<(), BrainApiError>`, `pub fn load_proposal(vault: &storage::Vault, id: &ProposalId) -> Result<PendingWriteProposal, BrainApiError>`, `pub fn list_proposals(vault: &storage::Vault) -> Result<Vec<PendingWriteProposal>, BrainApiError>`. Serialization format: deterministic (pick one — e.g. a small hand-written key:value format mirroring `Note`'s frontmatter+body shape, or `serde_json` with sorted keys; prefer `serde_json` with `#[derive(Serialize, Deserialize)]` plus a fixed field order via explicit struct field order, since it's the simplest deterministic choice and this project already depends on `serde`/`serde_yaml` elsewhere).

- [ ] **Step 1: Add `storage::Vault::verified_open_relative`**

```rust
// in crates/storage/src/vault.rs, new pub method on Vault
pub fn verified_open_relative(&self, relative: &Path, mode: handle_safety::OpenMode) -> Result<handle_safety::VerifiedRecordHandle, StorageError> {
    handle_safety::open_verified(&self.root, relative, mode)
}
```
Export `handle_safety::OpenMode` and `handle_safety::VerifiedRecordHandle` as `pub` from `storage` (add to `crates/storage/src/lib.rs`'s `pub use` list), since `brain-api` needs to construct/receive these types across the crate boundary; `open_verified` itself stays `pub(crate)` — only `Vault::verified_open_relative` exposes it, keeping a single, reviewable entry point.

- [ ] **Step 2: Write failing round-trip test**

```rust
#[test]
fn save_then_load_proposal_round_trips_exactly() {
    let dir = unique_test_dir();
    let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
    let proposal = PendingWriteProposal { /* ... concrete fields ... */ };
    save_proposal(&vault, &proposal).unwrap();
    let loaded = load_proposal(&vault, &proposal.id).unwrap();
    assert_eq!(loaded, proposal); // requires PartialEq derive on PendingWriteProposal and its field types
}
```

- [ ] **Step 3: Run to verify failure** — Expected: FAIL (functions don't exist)

- [ ] **Step 4: Implement `save_proposal`/`load_proposal`/`list_proposals`**

`save_proposal` serializes to JSON and writes via `vault.verified_open_relative(".proposals/{id}.proposal", OpenMode::CreateOrOpenForWrite)` + the verified-parent `atomic_write` form from Task 5 (ensure the `.proposals` directory itself is created if missing, with the same root-confinement discipline — decide whether directory creation needs its own verified step or whether `std::fs::create_dir_all` on a path already confirmed-safe by the canonical root check is sufficient, since the directory doesn't yet contain anything an attacker could alias at creation time; document the choice). `load_proposal` opens via `OpenMode::ReadExisting` and deserializes. `list_proposals` reads the `.proposals` directory entries and loads each.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p brain-api proposal_store::tests`
Expected: PASS

- [ ] **Step 6: Write and pass adversarial tests for this storage**

```rust
#[test]
fn load_proposal_rejects_a_symlinked_proposal_file() { /* same shape as vault.rs's inverted SEC-002 test, applied to .proposals */ }
#[test]
fn load_proposal_rejects_a_hard_linked_proposal_file() { /* same shape as SEC-003 */ }
#[test]
fn save_proposal_is_atomic_on_concurrent_writes_to_the_same_id() { /* two threads/processes writing the same proposal_id; one wins cleanly, neither corrupts the file */ }
```

Run: `cargo test -p brain-api proposal_store`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/storage/src/vault.rs crates/storage/src/lib.rs crates/brain-api/src/proposal_store.rs crates/brain-api/src/lib.rs
git commit -m "feat(brain-api): add .proposals/ storage reusing storage's verified-handle primitives"
```

## Task 13: `RequestContext` with `WritePolicy`; `save_memory`/`update_memory`/`link_entities` as proposal-creating operations

**Files:**
- Modify: `crates/brain-api/src/lib.rs`
- Create: `crates/brain-api/src/memory.rs` (save_memory/update_memory proposal creation)
- Create: `crates/brain-api/src/graph.rs` (link_entities proposal creation — graph_query/get_entity reads come in Task 17)

**Interfaces:**
- Consumes: Task 9's `PendingWriteProposal`/`ProposalId`, Task 10's `WritePolicy`/`WriteOperation`, Task 12's `save_proposal`.
- Produces: `pub fn save_memory(ctx: &RequestContext, vault: &storage::Vault, body: String, originating_identity: String) -> Result<ProposalId, BrainApiError>`, `pub fn update_memory(ctx: &RequestContext, vault: &storage::Vault, note_id: storage::NoteId, body: String, originating_identity: String) -> Result<ProposalId, BrainApiError>`, `pub fn link_entities(ctx: &RequestContext, vault: &storage::Vault, from: storage::NoteId, to: storage::NoteId, relation: String, originating_identity: String) -> Result<ProposalId, BrainApiError>`. Each checks `ctx.write_policy.as_ref().is_some_and(|p| p.allows(WriteOperation::ProposeWrite))` before creating a proposal; denies with `BrainApiError::PermissionDenied` otherwise.

- [ ] **Step 1: Write failing tests for permission-gating and happy path**

```rust
#[test]
fn save_memory_denied_without_propose_write_permission() {
    let ctx = RequestContext { schema_version: 1, correlation_id: uuid::Uuid::new_v4(), write_policy: None };
    let vault = test_vault();
    let result = save_memory(&ctx, &vault, "content".into(), "test-agent".into());
    assert!(matches!(result, Err(BrainApiError::PermissionDenied)));
}
#[test]
fn save_memory_creates_a_pending_proposal_with_propose_write_permission() {
    let ctx = RequestContext { schema_version: 1, correlation_id: uuid::Uuid::new_v4(), write_policy: Some(WritePolicy::mcp_agent()) };
    let vault = test_vault();
    let id = save_memory(&ctx, &vault, "content".into(), "test-agent".into()).unwrap();
    let loaded = proposal_store::load_proposal(&vault, &id).unwrap();
    assert!(matches!(loaded.status, ProposalStatus::Pending));
}
```

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement** `save_memory`/`update_memory`/`link_entities`, each: check permission, build a `PendingWriteProposal` with a fresh `ProposalId` (random UUID-based, per Task 9), `ProposalProvenance { source: originating_identity.clone(), created_at: now }`, `status: Pending`, call `proposal_store::save_proposal`, return the id.

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Write and pass the MCP-denial-shaped test explicitly**

```rust
#[test]
fn an_mcp_agent_policy_can_propose_but_the_resulting_proposal_cannot_self_approve() {
    // mcp_agent() policy creates the proposal fine (ProposeWrite allowed);
    // separately assert WritePolicy::mcp_agent().allows(WriteOperation::ApproveProposal) is false,
    // tying this test explicitly to the anti-goal rather than leaving it implicit.
}
```

- [ ] **Step 6: Commit**

```bash
git add crates/brain-api/src/memory.rs crates/brain-api/src/graph.rs crates/brain-api/src/lib.rs
git commit -m "feat(brain-api): save_memory/update_memory/link_entities as proposal-creating operations"
```

## Task 14: `approve_proposal`/`reject_proposal`/`list_proposals` (CLI-only approval execution)

**Files:**
- Create: `crates/brain-api/src/approval.rs`
- Modify: `crates/brain-api/src/lib.rs`

**Interfaces:**
- Consumes: Task 9's `valid_proposal_transition`, Task 10's `WritePolicy::allows`, Task 12's `load_proposal`/`save_proposal`, `storage::propose_and_commit` (the real signature — re-confirm its exact parameter list by reading `crates/storage/src/propose.rs` in full before writing this task's implementation, since Tasks 1–13 did not need its full signature and the design doc only cites its existence, not its parameters).
- Produces: `pub fn approve_proposal(ctx: &RequestContext, vault: &storage::Vault, id: &ProposalId, now: &str) -> Result<storage::CommitOutcome, BrainApiError>`, `pub fn reject_proposal(ctx: &RequestContext, vault: &storage::Vault, id: &ProposalId, reason: String, now: &str) -> Result<(), BrainApiError>`, `pub fn list_proposals(ctx: &RequestContext, vault: &storage::Vault) -> Result<Vec<PendingWriteProposal>, BrainApiError>` (listing itself needs no write permission, but should it require read permission at minimum? Decide: require `ctx` to at least be a recognized caller, i.e. keep it simple and require the same `approve_write_allowed` as approve/reject, since only the trusted approval surface has reason to see pending proposals in B5 — the B6 UI's "human-initiated write" path is a different, not-yet-built operation).

- [ ] **Step 1: Write failing tests for the full lifecycle and every denial path**

```rust
#[test]
fn approve_proposal_denied_for_mcp_originated_context() {
    let ctx = RequestContext { schema_version: 1, correlation_id: uuid::Uuid::new_v4(), write_policy: Some(WritePolicy::mcp_agent()) };
    // ... create a pending proposal first via trusted_cli or direct store ...
    let result = approve_proposal(&ctx, &vault, &id, "2026-11-15T12:00:00Z");
    assert!(matches!(result, Err(BrainApiError::PermissionDenied)));
}
#[test]
fn approve_proposal_succeeds_for_trusted_cli_context_and_commits_the_note() {
    let ctx = RequestContext { schema_version: 1, correlation_id: uuid::Uuid::new_v4(), write_policy: Some(WritePolicy::trusted_cli()) };
    // create proposal, approve it, assert storage::Vault::read_note now returns the committed content
}
#[test]
fn approve_proposal_rejects_an_expired_proposal_even_if_status_still_says_pending() {
    // construct a proposal with expires_at in the past, status still Pending
    // (simulating "never listed since expiry"); approve_proposal must refuse
    // with a Conflict or InvalidInput (decide which; InvalidInput fits "the
    // request is no longer valid" better than Conflict's version-mismatch
    // connotation — use BrainApiError::InvalidInput("proposal expired")).
}
#[test]
fn approving_an_already_approved_proposal_is_rejected_not_double_executed() {
    // approve once (succeeds), approve again (must fail via valid_proposal_transition, not re-commit)
}
#[test]
fn reject_proposal_moves_pending_to_rejected_and_approve_proposal_then_fails() {
    // reject, then attempt approve; must fail via valid_proposal_transition
}
#[test]
fn permission_check_happens_before_proposal_existence_is_revealed() {
    // approve_proposal with mcp_agent() policy on a NONEXISTENT proposal id
    // must still return PermissionDenied, not NotFound — proving the
    // permission check runs before the load.
}
```

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement**

Order inside `approve_proposal`/`reject_proposal`: (1) check `ctx.write_policy` allows the operation — return `PermissionDenied` immediately if not, without touching storage; (2) load the proposal via `proposal_store::load_proposal`; (3) check expiry against `now` regardless of persisted status; (4) check `valid_proposal_transition` from the loaded status to the target status; (5) for approve specifically: call `storage::propose_and_commit` with the proposal's operation payload, using its real parameter list (confirmed in this task's prep step); (6) on successful commit, update the proposal's persisted status to `Approved { approved_at: now }` via `proposal_store::save_proposal`; (7) return the commit outcome.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p brain-api approval::tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/brain-api/src/approval.rs crates/brain-api/src/lib.rs
git commit -m "feat(brain-api): approve_proposal/reject_proposal/list_proposals with expiry and transition enforcement"
```

## Task 15: Minimal CLI approval surface (for E2E testing, per design doc scope)

**Files:**
- Create: `crates/brain-api/examples/approve_cli.rs` (a `cargo run --example` binary, the simplest way to get a real CLI entry point without inventing a new crate for it, consistent with "NON costruire la UI in B5" and "CLI approval minima se necessaria per verificare E2E")
- Test: `crates/brain-api/tests/cli_approval_e2e.rs` (integration test driving the example's logic as a library call, not by spawning the binary — extract the example's core logic into a `pub fn run_cli(args: &[String], vault: &storage::Vault) -> Result<String, BrainApiError>` in `approval.rs` or a new `cli.rs` so it's testable without process spawning)

**Interfaces:**
- Consumes: Task 14's `approve_proposal`/`reject_proposal`/`list_proposals`.
- Produces: a working `list` / `approve <id>` / `reject <id> <reason>` command set, always constructing `WritePolicy::trusted_cli()`.

- [ ] **Step 1: Write the failing integration test**

```rust
#[test]
fn cli_list_shows_a_pending_proposal_and_approve_commits_it() {
    // create a proposal via brain_api::save_memory with an mcp_agent() context,
    // then drive run_cli(&["list"], &vault) and assert the id appears in output,
    // then run_cli(&["approve", &id.to_string()], &vault) and assert the note is now readable.
}
```

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement `run_cli` and the thin example binary wrapping it**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/brain-api/examples/approve_cli.rs crates/brain-api/src/approval.rs crates/brain-api/tests/cli_approval_e2e.rs
git commit -m "feat(brain-api): minimal CLI approval surface for E2E verification"
```

## Task 16: Read delegation — `search`, `retrieve`, `get_code_context` via `brain-api`

**Files:**
- Create: `crates/brain-api/src/read.rs`
- Modify: `crates/brain-api/src/lib.rs`

**Interfaces:**
- Consumes: `api::{search, retrieve, SearchRequest, RetrieveRequest, RetrieveResponse}` (existing, real), `policy::{ReadPolicy, ReadOperation}` (existing, extend `ReadOperation` with a `GetCodeContext` variant in this task — confirm whether `get_code_context` is already implemented anywhere in `codegraph-index` before assuming it needs building from scratch here; if it exists as a function there, this task wraps it the same way it wraps `api::search`/`api::retrieve`; if not, this task's scope grows to include a minimal implementation and that growth should be flagged, not silently absorbed).
- Produces: `pub fn search(ctx: &RequestContext, vault: &storage::Vault, query: &str) -> Result<Vec<index::search::SearchHit>, BrainApiError>`, `pub fn retrieve(...) -> Result<domain::NoteMetadata, BrainApiError>` (exact return types match whatever `api::search`/`api::retrieve` already return — do not invent new ones), `pub fn get_code_context(...) -> Result<..., BrainApiError>`.

- [ ] **Step 1: Check whether `get_code_context`-equivalent functionality already exists in `codegraph-index`**

Grep `crates/codegraph-index/src` for anything matching "context," "snippet," or similar. Read whatever is found.

- [ ] **Step 2: Write failing tests for each of the 3 delegating wrappers, happy path + empty query**

```rust
#[test]
fn search_delegates_to_api_search_and_wraps_its_result() { /* same assertions api::search's own tests make, through brain_api::search */ }
#[test]
fn search_with_empty_query_returns_invalid_input() { /* confirm api::search's own empty-query behavior first; wrap consistently */ }
#[test]
fn retrieve_not_found_note_returns_not_found() { }
#[test]
fn get_code_context_for_unknown_symbol_returns_not_found() { }
#[test]
fn a_hash_mismatch_on_the_underlying_read_never_falls_back_silently_through_brain_api() {
    // Pins constraint d9mm through the new wrapper layer: if api::retrieve
    // (or storage underneath it) returns an Inconsistent-shaped error for a
    // tampered/corrupt record, brain_api::retrieve must propagate that as
    // BrainApiError::CorruptData, never substitute a before-image or any
    // other silent fallback value.
}
```

- [ ] **Step 3: Run to verify failure**

- [ ] **Step 4: Implement** each as a thin wrapper converting `api::ApiError`/`codegraph-index`'s error type into `BrainApiError` via an explicit match (every `api::ApiError` variant maps to exactly one `BrainApiError` variant — write this mapping out explicitly, don't use a catch-all `_ => BrainApiError::Internal` since `BrainApiError` has no such variant by design).

- [ ] **Step 5: Run to verify pass**

- [ ] **Step 6: Commit**

```bash
git add crates/brain-api/src/read.rs crates/brain-api/src/lib.rs crates/policy/src/read.rs
git commit -m "feat(brain-api): delegate search/retrieve/get_code_context reads to crates/api"
```

## Task 17: `graph_query`, `get_entity` (reads)

**Files:**
- Modify: `crates/brain-api/src/graph.rs`
- Modify: `crates/policy/src/read.rs` (add `GraphQuery`, `GetEntity` variants to `ReadOperation`)

**Interfaces:**
- Consumes: whatever `codegraph-index`/`codegraph-domain` already expose for graph traversal and entity lookup — read those crates' public APIs before writing this task's steps in full (follow the same "check what's real first" discipline as every other task).
- Produces: `pub fn graph_query(ctx: &RequestContext, ..., query: GraphQuerySpec, limit: usize) -> Result<Vec<...>, BrainApiError>` with a bounded traversal depth and result count (exact numbers from Task 34); `pub fn get_entity(ctx: &RequestContext, ..., entity_id: &str) -> Result<..., BrainApiError>`.

- [ ] **Step 1: Read `codegraph-index`'s and `codegraph-domain`'s public surface** (grep `pub fn`/`pub struct`/`pub enum`) to ground this task's exact signatures before writing tests.

- [ ] **Step 2: Write failing tests**: valid query within bounds; query exceeding the traversal-depth bound returns `BrainApiError::InvalidInput`; unknown entity id returns `NotFound`; a cyclic graph structure does not infinite-loop (bounded traversal terminates).

- [ ] **Step 3: Run to verify failure**

- [ ] **Step 4: Implement**, delegating to the real `codegraph-index`/`codegraph-domain` functions found in Step 1; add the bounded-traversal check in `brain-api` if the underlying crate doesn't already enforce one (do not assume it does — verify).

- [ ] **Step 5: Run to verify pass**

- [ ] **Step 6: Commit**

```bash
git add crates/brain-api/src/graph.rs crates/policy/src/read.rs
git commit -m "feat(brain-api): add graph_query and get_entity with bounded traversal"
```

## Task 18: `get_previous_solution`

**Files:**
- Create or modify: `crates/brain-api/src/memory.rs` (add read function alongside Task 13's proposal-creating ones)
- Modify: `crates/policy/src/read.rs` (add `GetPreviousSolution` variant)

**Interfaces:**
- Produces: `pub fn get_previous_solution(ctx: &RequestContext, vault: &storage::Vault, query: &str) -> Result<Vec<domain::NoteMetadata>, BrainApiError>` — per the design doc, "supporta bug/fix/test/decision provenance; collega il risultato alle entità/note quando possibile." Ground this against whatever `domain::NoteType` variants already exist (`Bug` is confirmed real from `crates/domain/src/note.rs`'s test fixtures — check `schema.rs` for the full `NoteType` enum before implementing the filter).

- [ ] **Step 1: Read `domain::schema::NoteType`'s full variant list.**

- [ ] **Step 2: Write failing tests**: a query matching a `Bug`-typed note with a linked fix returns it; an empty result when nothing matches returns `Ok(vec![])`, not an error (per the design doc's empty/not-found semantics discipline — confirm this against Task 7's frozen schema for this capability).

- [ ] **Step 3: Run to verify failure**

- [ ] **Step 4: Implement**, likely delegating to `api::search` with a type filter plus `domain::classify_links`/`Relations` for provenance linking.

- [ ] **Step 5: Run to verify pass**

- [ ] **Step 6: Commit**

```bash
git add crates/brain-api/src/memory.rs crates/policy/src/read.rs
git commit -m "feat(brain-api): add get_previous_solution with provenance linking"
```

## Task 19: `get_rules`, `get_regulations`, `get_canon`

**Files:**
- Create: `crates/brain-api/src/knowledge.rs`
- Modify: `crates/policy/src/read.rs` (add 3 variants)

**Interfaces:**
- Consumes: Task 7's grounding of what these three terms mean in this project.
- Produces: `pub fn get_rules(ctx: &RequestContext, vault: &storage::Vault, scope: Option<String>) -> Result<Vec<domain::NoteMetadata>, BrainApiError>` and the equivalent two, per Task 7's frozen scope/precedence definition.

- [ ] **Step 1: Re-read Task 7's grounding section before implementing — this task cannot proceed without it being settled first.**

- [ ] **Step 2: Write failing tests pinning the precedence/scope behavior Task 7 defined** (e.g. project-scope rules override global-scope rules when both exist for the same topic, if that's what Task 7 settled; or the three are fully independent sources with no precedence interaction, if that's what was found/decided — the test must match Task 7's actual written definition, not a guess).

- [ ] **Step 3: Run to verify failure**

- [ ] **Step 4: Implement**

- [ ] **Step 5: Run to verify pass**

- [ ] **Step 6: Commit**

```bash
git add crates/brain-api/src/knowledge.rs crates/policy/src/read.rs
git commit -m "feat(brain-api): add get_rules/get_regulations/get_canon per B5.0 scope definitions"
```

## Task 20: Layering tests — no bypass of `brain-api` to `storage`

**Files:**
- Create: `crates/brain-api/tests/layering.rs`

**Interfaces:**
- Consumes: everything from Tasks 8–19.

- [ ] **Step 1: Write the failing architecture tests from the design doc's "Required tests for this layering" list**

```rust
#[test]
fn read_delegation_behaves_identically_to_calling_api_directly() { /* same query through api::search and brain_api::search, assert equal results */ }
#[test]
fn approval_is_denied_for_a_remote_mcp_originated_context() { /* duplicate/confirm Task 14's unit test at the integration level */ }
#[test]
fn an_approved_proposal_executes_exactly_once() { /* approve twice concurrently (two threads), assert exactly one commit happened */ }
#[test]
fn a_rejected_proposal_cannot_later_execute() { }
#[test]
fn an_expired_proposal_cannot_execute() { }
```

- [ ] **Step 2: Run to verify current state** (most should already pass from earlier tasks' unit tests; this task's value is having them at the integration level, across real `Vault`/`brain-api` boundaries, not re-deriving new behavior)

- [ ] **Step 3: Fix any gap found** (if a test fails here that passed as a unit test, there's a wiring bug between modules — fix it, don't weaken the test)

- [ ] **Step 4: Commit**

```bash
git add crates/brain-api/tests/layering.rs
git commit -m "test(brain-api): add cross-module layering/bypass-prevention integration tests"
```

---

## Task 21: Choose and scaffold the MCP protocol crate dependency

**Files:**
- Create: `crates/mcp-server/Cargo.toml`
- Create: `crates/mcp-server/src/lib.rs`
- Modify: workspace `Cargo.toml`

**Interfaces:**
- Produces: an empty-but-building `mcp-server` crate depending on `brain-api` and whichever real, maintained Rust MCP protocol crate is chosen (check crates.io for the current state of `rmcp`/`mcp-sdk`-equivalent crates at implementation time — this is an implementation-time lookup, not a design decision, since crate availability/maturity changes; pick the one with an actively maintained server-mode implementation matching the MCP spec version SuperBrain targets).

- [ ] **Step 1: Research available Rust MCP crates** (check crates.io, confirm server-mode support, license compatibility with this private/personal project, and whether it's offline-capable after initial fetch per the project's offline-safe constraint).

- [ ] **Step 2: Write the crate-builds test, scaffold `Cargo.toml`/`lib.rs`**

- [ ] **Step 3: Run `cargo test -p mcp-server`, expect PASS on the trivial test**

- [ ] **Step 4: Commit**

```bash
git add crates/mcp-server/Cargo.toml crates/mcp-server/src/lib.rs Cargo.toml
git commit -m "feat(mcp-server): scaffold crate, select MCP protocol dependency"
```

## Task 22: MCP server — search/retrieve/get_code_context tools

**Files:**
- Create: `crates/mcp-server/src/tools/search.rs`

**Interfaces:**
- Consumes: `brain_api::read::{search, retrieve, get_code_context}` (Task 16), the chosen MCP crate's tool-registration API (Task 21).
- Produces: 3 registered MCP tools, each a thin schema-mapping wrapper: MCP request → `brain_api` call → MCP response, with `BrainApiError` mapped to the MCP crate's own error/result shape.

- [ ] **Step 1: Write failing tests**: valid query; empty query; invalid `k` (negative or zero, per the design doc's B5.3 test list); filters; a sensitive-classified result with/without `allow_sensitive`; dense runtime unavailable (feature-gated, simulate via the existing `dense-retrieval` feature flag's absence) falls back per B4's frozen fallback semantics; deterministic ordering across repeated identical calls.

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement the 3 tool wrappers**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/mcp-server/src/tools/search.rs
git commit -m "feat(mcp-server): wire search/retrieve/get_code_context as MCP tools"
```

## Task 23: MCP server — graph/knowledge/memory/rules tools (read-shaped + proposal-creating)

**Files:**
- Create: `crates/mcp-server/src/tools/graph.rs`, `crates/mcp-server/src/tools/memory.rs`, `crates/mcp-server/src/tools/knowledge.rs`

**Interfaces:**
- Consumes: `brain_api::{graph_query, get_entity, link_entities, get_previous_solution, save_memory, update_memory, get_rules, get_regulations, get_canon}` (Tasks 13, 17, 18, 19). Every MCP-originated call constructs its `RequestContext` with `write_policy: Some(WritePolicy::mcp_agent())` for the 3 write-shaped tools, and `write_policy: None` for the 7 pure-read tools — this construction happens in `mcp-server`, never trusting anything from the incoming MCP request to set it.

- [ ] **Step 1: Write failing tests**: each of the 9 remaining capabilities' happy path; `save_memory`/`update_memory`/`link_entities` via MCP return a `ProposalId`, never a committed result; a direct attempt to call `approve_proposal` through the MCP transport (if such a tool were mistakenly registered) is the one thing this task's tests must confirm is **not** registered at all — i.e., assert the MCP server's tool list does not contain `approve_proposal`/`reject_proposal`.

```rust
#[test]
fn mcp_server_never_registers_approve_or_reject_proposal_as_tools() {
    let server = build_test_server();
    let tool_names: Vec<_> = server.list_tools().iter().map(|t| t.name.clone()).collect();
    assert!(!tool_names.contains(&"approve_proposal".to_string()));
    assert!(!tool_names.contains(&"reject_proposal".to_string()));
}

#[test]
fn every_registered_mcp_tool_maps_1to1_to_a_named_brain_api_operation() {
    // Anti-goal: "Silent capability creep: any MCP tool whose effect isn't
    // 1:1 (or documented N:1) traceable to a named Brain API operation."
    // Enumerate the server's registered tools and assert each name is one
    // of exactly the 12 Brain API capability names (or their proposal-
    // creating equivalents) — no tool exists that isn't a direct,
    // documented wrapper over a brain_api function from Tasks 13/16-19.
    let server = build_test_server();
    let known_capabilities = [
        "search", "retrieve", "get_code_context", "graph_query", "get_entity",
        "get_previous_solution", "get_rules", "get_regulations", "get_canon",
        "save_memory", "update_memory", "link_entities",
    ];
    for tool in server.list_tools() {
        assert!(known_capabilities.contains(&tool.name.as_str()), "undocumented tool: {}", tool.name);
    }
}
```

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement the 9 tool wrappers; do not register approve/reject**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/mcp-server/src/tools/graph.rs crates/mcp-server/src/tools/memory.rs crates/mcp-server/src/tools/knowledge.rs
git commit -m "feat(mcp-server): wire graph/memory/knowledge tools; confirm approve/reject never exposed via MCP"
```

## Task 24: MCP server integration tests (B5.2's own acceptance)

**Files:**
- Create: `crates/mcp-server/tests/server_integration.rs`

- [ ] **Step 1: Write failing integration tests covering the full real MCP protocol round-trip** (a real client connecting over whatever transport the chosen crate provides — stdio is the typical MCP default — sending real protocol messages, not just calling Rust functions directly): list tools, call `search`, call `save_memory` and confirm a `ProposalId` comes back, malformed JSON-RPC input returns a structured protocol error not a panic.

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement whatever server bootstrap wiring is still missing to make these pass**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/mcp-server/tests/server_integration.rs
git commit -m "test(mcp-server): add real-protocol integration tests"
```

---

## Task 25: Permission model hardening (B5.7) — deny-by-default audit

**Files:**
- Create: `crates/brain-api/tests/permission_model.rs`

- [ ] **Step 1: Write failing tests enumerating every capability × every policy state, asserting deny-by-default**: for every one of the 12 capabilities, a `RequestContext` with no policy / the most restrictive policy must deny, not silently allow. This is a matrix test, not 12 separate ad-hoc tests — write it as a table-driven test iterating capability+policy combinations.

- [ ] **Step 2: Run; fix any capability found to allow-by-default** (this is the task's real purpose: catching anything Tasks 13–19 got wrong)

- [ ] **Step 3: Commit**

```bash
git add crates/brain-api/tests/permission_model.rs
git commit -m "test(brain-api): deny-by-default permission matrix across all 12 capabilities"
```

## Task 26: MCP client foundation — connection lifecycle, server registration (B5.8)

**Files:**
- Create: `crates/mcp-client/Cargo.toml`, `crates/mcp-client/src/lib.rs`, `crates/mcp-client/src/registration.rs`
- Modify: workspace `Cargo.toml`

**Interfaces:**
- Produces: `pub struct McpServerRegistration { pub name: String, pub transport: TransportConfig, pub enabled: bool }`, `pub fn connect(registration: &McpServerRegistration) -> Result<McpConnection, McpClientError>` with timeout, `pub fn global_client_enabled() -> bool` (a single kill-switch config read, per the design doc's "client deve poter essere disabilitato globalmente").

- [ ] **Step 1: Write failing tests**: a registration with `enabled: false` never attempts connection; a connection attempt that times out returns `McpClientError::Timeout`, not a hang; the global disable switch short-circuits every connection attempt regardless of individual registrations.

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement** using the same MCP crate chosen in Task 21, in client mode.

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/mcp-client/Cargo.toml crates/mcp-client/src/lib.rs crates/mcp-client/src/registration.rs Cargo.toml
git commit -m "feat(mcp-client): connection lifecycle, server registration, global disable switch"
```

## Task 27: MCP client — capability discovery, tool metadata cache

**Files:**
- Create: `crates/mcp-client/src/discovery.rs`

- [ ] **Step 1: Write failing tests**: discovering tools from a mock external MCP server populates a cache; a server returning a malformed tool list is rejected, not cached; cache entries carry the metadata B5.9 needs (read/write/execute classification placeholder, filled by Task 28).

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/mcp-client/src/discovery.rs
git commit -m "feat(mcp-client): capability discovery and tool metadata cache"
```

## Task 28: External tool authorization model (B5.9)

**Files:**
- Create: `crates/mcp-client/src/permission.rs`

**Interfaces:**
- Produces: `pub struct ExternalToolMetadata { pub server: String, pub tool_name: String, pub classification: ToolClassification, pub network_required: bool, pub filesystem_scope: Option<String>, pub sensitivity: Sensitivity }`, `pub enum ToolClassification { Read, Write, Execute }`, `pub enum ApprovalPolicy { Deny, AllowReadOnly, AskApprovalRequired, AllowScopedExecution(ScopeLimits) }`, `pub fn authorize(metadata: &ExternalToolMetadata, policy: &ApprovalPolicy) -> Result<(), McpClientError>`.

- [ ] **Step 1: Write failing tests**: default policy for an unregistered tool is `Deny`, never implicit allow; `AllowReadOnly` policy rejects a `Write`/`Execute`-classified tool; `AskApprovalRequired` always requires an explicit approval call before `authorize` succeeds (i.e. `authorize` alone, with no prior approval record, fails for this policy); no policy variant exists that means "allow everything" (this is a type-system-level test — confirm `ApprovalPolicy` has no such variant by exhaustively matching all variants in the test and asserting the match is exhaustive at compile time, i.e. no `_ =>` arm).

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/mcp-client/src/permission.rs
git commit -m "feat(mcp-client): external tool authorization model, deny-by-default"
```

## Task 29: MCP client — invocation, timeout, audit record

**Files:**
- Create: `crates/mcp-client/src/invoke.rs`
- Modify: `crates/mcp-client/src/lib.rs` (add `audit.rs` per the file structure — can fold into this task since they're tightly coupled: every invocation produces an audit record)

**Interfaces:**
- Consumes: Task 28's `authorize`, Task 26's `McpConnection`.
- Produces: `pub fn invoke_external_tool(conn: &McpConnection, metadata: &ExternalToolMetadata, policy: &ApprovalPolicy, args: serde_json::Value) -> Result<serde_json::Value, McpClientError>`, emitting an audit record (Task 32's schema) on every call regardless of outcome.

- [ ] **Step 1: Write failing tests**: a denied tool never reaches the network call (assert via a mock connection that records whether `send` was ever invoked); a timeout produces `McpClientError::Timeout` and still emits an audit record; a malformed response envelope (wrong shape, unexpected fields) is rejected as `McpClientError::MalformedResponse`, never passed through as if valid; a response that claims to be from a different tool than the one invoked (spoofing/envelope mismatch) is rejected.

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/mcp-client/src/invoke.rs
git commit -m "feat(mcp-client): tool invocation with timeout, response validation, audit"
```

## Task 30: Server/client separation architecture test (B5.10)

**Files:**
- Create: `crates/mcp-client/tests/separation.rs` (or a workspace-level test if that fits better — decide based on whether the assertion needs both crates' types in scope, which it does, so this likely needs a new small test crate or lives in whichever of `mcp-server`/`mcp-client` doesn't create a circular dev-dependency; check `Cargo.toml` dependency direction before deciding placement)

- [ ] **Step 1: Write the failing architecture test**: `mcp-server` does not depend on `mcp-client` in its `Cargo.toml` (and vice versa) — assert this by parsing both crates' `Cargo.toml` files in the test rather than relying on compilation alone, since a transitive dependency could sneak in without a direct one; a capability registered on the server is never automatically callable by the client (no shared registry between the two); a client-side tool invocation can never reach `brain-api`'s `storage::propose_and_commit` path directly (the client only ever calls out to *external* servers, never loops back into SuperBrain's own write path — assert `mcp-client` has no dependency on `storage` or `brain-api` at all in its `Cargo.toml`).

- [ ] **Step 2: Run to verify failure/pass** (this should mostly pass already if Tasks 21–29 were scoped correctly; its value is catching an accidental coupling)

- [ ] **Step 3: Fix any coupling found**

- [ ] **Step 4: Commit**

```bash
git add crates/mcp-client/tests/separation.rs
git commit -m "test: verify MCP server/client architectural separation"
```

---

## Task 31: E2E flow 1 — external agent → MCP server → Brain API → DenseV2 search

**Files:**
- Create: `crates/mcp-server/tests/e2e_agent_search.rs`

- [ ] **Step 1: Write the failing E2E test**: a simulated external agent (a real MCP client, even a minimal throwaway one built from the same crate as `mcp-client` or the raw protocol crate) connects to the real `mcp-server`, calls `search` with a query known to hit DenseV2 (requires the `dense-retrieval` feature enabled for this test run), and receives real, correctly-ranked results from the real index — not mocked data anywhere in the chain.

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Wire whatever's missing to make it pass**

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p mcp-server --features dense-retrieval e2e_agent_search`

- [ ] **Step 5: Commit**

```bash
git add crates/mcp-server/tests/e2e_agent_search.rs
git commit -m "test: E2E external-agent-to-DenseV2-search flow"
```

## Task 32: E2E flow 2 — Brain service → MCP client → mock external tool

**Files:**
- Create: `crates/mcp-client/tests/e2e_external_tool.rs`

- [ ] **Step 1: Write the failing E2E test**: a mock external MCP server (spun up in-test) exposes one tool; SuperBrain's `mcp-client` registers it, discovers it, and invokes it under an `AllowScopedExecution` policy, receiving a real result over the real transport.

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement/wire**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Test denied/unavailable/timeout/malformed/revoked-permission variants in the same file**

```rust
#[test] fn denied_tool_is_never_invoked() { }
#[test] fn unavailable_server_returns_a_typed_error_not_a_hang() { }
#[test] fn timeout_is_enforced() { }
#[test] fn malformed_response_is_rejected() { }
#[test] fn revoking_permission_mid_session_denies_the_next_call() { }
```

- [ ] **Step 6: Run to verify all pass**

- [ ] **Step 7: Commit**

```bash
git add crates/mcp-client/tests/e2e_external_tool.rs
git commit -m "test: E2E Brain-to-external-tool flow, plus denial/timeout/malformed/revocation cases"
```

---

## Task 33: Audit/observability events (B5.12)

**Files:**
- Create: `crates/brain-api/src/audit.rs`
- Modify: `crates/mcp-client/src/invoke.rs` (wire in, if not already done as part of Task 29 — check before duplicating)

**Interfaces:**
- Produces: `pub struct AuditEvent { pub kind: AuditEventKind, pub timestamp: String, pub correlation_id: uuid::Uuid }`, `pub enum AuditEventKind { McpConnection { server: String }, ToolCall { tool: String, allowed: bool }, PermissionDecision { operation: String, allowed: bool }, DeniedAction { operation: String, reason: String }, WriteMemory { proposal_id: String }, ExternalExecution { server: String, tool: String }, FallbackOrError { context: String } }` — no field anywhere carries note content or query text, only identifiers/booleans/reasons-as-category-not-full-text.

- [ ] **Step 1: Write failing tests**: constructing any `AuditEventKind` variant with an attempted content-bearing field doesn't compile (this is enforced by the type definition itself having no such field — the test instead confirms serialization of each variant produces no field matching note body/query text patterns, as a regression guard); every one of Tasks 13/14/22/23/29's operations emits exactly one audit event per call.

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement, wiring emission into every call site identified**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/brain-api/src/audit.rs crates/mcp-client/src/invoke.rs
git commit -m "feat(brain-api,mcp-client): structured, content-free audit events"
```

## Task 34: Security/adversarial test suite (B5.13)

**Files:**
- Create: `crates/mcp-server/tests/adversarial.rs`
- Create: `crates/mcp-client/tests/adversarial.rs`

- [ ] **Step 1: Write failing tests for every item in the design doc's/user brief's adversarial list not already covered by Tasks 6/9/12/23/28/29/30/32**: malformed JSON-RPC input (oversized, truncated, wrong types); unknown tool name; schema mismatch on a known tool; path traversal in any tool accepting a path-like string (confirm which tools, if any, accept path-like input — likely none of the 12, since all operate on `NoteId`/`ProposalId`, not raw filesystem paths; if genuinely none do, write a test asserting no tool schema accepts a raw path string, rather than skipping this item silently); a prompt/tool-injection-shaped string in a note body or query treated as plain data by every tool (i.e., a note body containing text that looks like a tool-call instruction must never cause the server to execute anything — assert the server's behavior is identical whether the string is injection-shaped or not); repeated/replayed request (does replaying an identical `approve_proposal` call twice double-execute? — tie this to Task 14's existing `approving_an_already_approved_proposal_is_rejected` test, confirm it covers replay specifically, not just double-approval-in-sequence); sensitive-data exfiltration attempt (a `search`/`retrieve` call for a `Sensitive`-classified note without `allow_sensitive` must never leak content in the error path either — check the error variant returned carries no content).

- [ ] **Step 2: Run to verify failure where new, pass where already covered (confirm, don't just assume)**

- [ ] **Step 3: Fix any real gap found**

- [ ] **Step 4: Commit**

```bash
git add crates/mcp-server/tests/adversarial.rs crates/mcp-client/tests/adversarial.rs
git commit -m "test: security/adversarial suite for MCP server and client"
```

## Task 35: Resource limits (B5.16)

**Files:**
- Modify: `crates/brain-api/src/lib.rs` or a new `crates/brain-api/src/limits.rs`
- Modify: `crates/mcp-server/src/lib.rs`, `crates/mcp-client/src/lib.rs`

**Interfaces:**
- Produces: concrete constants — e.g. `pub const MAX_QUERY_LEN: usize = 4096;`, `pub const MAX_RESULT_COUNT: usize = 500;`, `pub const MAX_GRAPH_TRAVERSAL_DEPTH: usize = 10;`, `pub const MCP_MAX_PAYLOAD_BYTES: usize = 1_048_576;`, `pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;`, `pub const MAX_CONCURRENT_REQUESTS: usize = 16;` — exact values are this task's own call (per the design doc: "non serve tuning scientifico, serve impedire resource exhaustion accidentale"), document each with a one-line rationale comment.

- [ ] **Step 1: Write failing tests**: a query exceeding `MAX_QUERY_LEN` returns `InvalidInput`, not a panic or silent truncation; a graph traversal exceeding `MAX_GRAPH_TRAVERSAL_DEPTH` terminates with `InvalidInput`, not an unbounded walk; an oversized MCP payload is rejected at the transport layer before reaching `brain-api`; more than `MAX_CONCURRENT_REQUESTS` simultaneous calls queue or reject rather than exhausting resources (test via a bounded semaphore, not literally spawning thousands of threads).

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement the constants and enforce each at its natural boundary**

- [ ] **Step 4: Run to verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/brain-api/src/limits.rs crates/mcp-server/src/lib.rs crates/mcp-client/src/lib.rs
git commit -m "feat: enforce query/result/traversal/payload/concurrency limits"
```

## Task 36: Mutation testing pass (B5.17)

**Files:** none new — this task runs `cargo mutants` (or whatever mutation tool this workspace already uses elsewhere; check prior B-phase closure docs for the exact tool/invocation this project standardized on, e.g. `cargo-mutants`, and reuse it) scoped to:
- `crates/storage/src/handle_safety.rs`
- `crates/brain-api/src/{policy_write,proposal,approval,proposal_store}.rs`
- `crates/mcp-client/src/permission.rs`

- [ ] **Step 1: Run the mutation tool scoped to these files**

- [ ] **Step 2: For every surviving mutant that represents a real gap (not an equivalent mutant), write the test that kills it**

- [ ] **Step 3: Re-run until the gate this project uses elsewhere is met (check what "closed" meant for prior phases' mutation gates, e.g. B2's "mutation gate closed" language, and match that bar)**

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "test: close mutation-testing gaps in permission gates, proposal lifecycle, tool mapping"
```

## Task 37: Final verification sweep (B5.18)

- [ ] **Step 1:** `cargo fmt --check` — fix any formatting drift, commit separately if needed.
- [ ] **Step 2:** `cargo clippy --workspace --all-targets --all-features -- -D warnings` — fix every new warning; if a pre-existing warning outside this plan's touched files shows up, note it as pre-existing, do not fix unrelated code.
- [ ] **Step 3:** `cargo test --workspace --all-features` — all green.
- [ ] **Step 4:** Re-run Tasks 24, 30, 31, 32, 34's integration/E2E/adversarial suites explicitly by name, confirm each still passes in a full-workspace build (not just crate-isolated).
- [ ] **Step 5:** Commit any final fixes.

```bash
git add -A
git commit -m "chore(B5): final verification sweep — fmt, clippy, full workspace test pass"
```

## Task 38: Closure document

**Files:**
- Create: `docs/research/B5-BRAIN-API-MCP-CLOSURE.md`

- [ ] **Step 1: Write the closure document** covering, per the user's brief: architecture; Brain API contract (link Task 7's freeze doc); MCP server tools; MCP client; permission model; E2E flows (Tasks 31–32); error model (Task 11); tests (summarize counts/coverage, don't re-paste); mutation results (Task 36); security findings (Task 34, plus B5.SEC's residual-risk note from Task 6 if any); limits (Task 35's exact constants); deviations from the original design doc (if any emerged during implementation — list them honestly); known technical debt; an exact PASS/FAIL table against every one of the 20 acceptance criteria in the design doc's source brief; a merge recommendation (not a merge action).

- [ ] **Step 2: Commit**

```bash
git add docs/research/B5-BRAIN-API-MCP-CLOSURE.md
git commit -m "docs(B5): closure document with PASS/FAIL acceptance table and merge recommendation"
```

**Do not merge, tag, or push.** Stop here and report to the user per the brief's required STOP report format: branch, HEAD, commit list, diff/stat, test results, clippy/fmt, mutation results, security/adversarial results, E2E results, technical debt, exact B5 PASS/FAIL, merge recommendation.
