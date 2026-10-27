# A2 — Recovery: closing evidence

**Date:** 2026-10-27

**Git commit hash at the start of this task's verification run:** `28ae8ffdeffa860bf1d75a71bff93ae0cb8dc303`
(this is Task 9's closing commit, `fix(storage): close two scope gaps in the SEC-001/002/003 adversarial
evaluation`; this task adds no production or test code — only this evidence file,
`eval/results/A2/dependency-exceptions.md`, and a closing commit on top of it).

**Plan:** `docs/superpowers/plans/2026-10-27-a2-recovery.md`
**Task brief:** `.superpowers/sdd/2026-10-27-a2-recovery/task-10-brief.md`

---

## 1. Scope of this file

This is Task 10, the final task of the A2 ("Recovery") plan. It closes A2 by re-running the
full acceptance checklist the spec's Fase A table requires (the same table A1 closed against,
now re-run against A2's journal/git/recovery additions) and recording the artifact. **No
production code and no existing test was modified to produce this evidence** — every result
below is the actual output of the commands run against the tree as Task 9 left it, including
one finding (Section 2.2) that this task deliberately did **not** remediate because doing so
would have required a dependency-version change, which is outside a documentation task's
license to make.

---

## 2. Commands run and results

All commands were run from the repository root in the `a2-recovery` worktree.

### 2.1 `cargo test --workspace`

```
cargo test --workspace
```

**Result: 199 passed, 0 failed.**

| Suite | Passed |
|---|---|
| `domain` unit tests | 32 |
| `policy` unit tests | 12 |
| `storage` unit tests (`src/lib.rs`, inline `#[cfg(test)]` modules across `vault.rs`, `journal.rs`, `git.rs`, `fault.rs`, `atomic.rs`, etc.) | 127 |
| `storage` integration tests (`tests/integration.rs`) | 23 |
| `storage` doc-tests (incl. 4 `compile_fail` tests pinning the public write surface) | 5 |
| `domain`/`policy` doc-tests | 0 / 0 |
| **Total** | **199** |

Representative test names from the 127 `storage` unit tests, confirming every C1 fault-matrix
row and the retry/idempotency/concurrency guarantees are exercised (full list in
`crates/storage/src/vault.rs`, `#[cfg(test)] mod tests`):

- `crash_after_intent_recorded_before_any_write_results_in_failed_no_file_touched`
- `crash_after_rename_with_an_invalid_hash_restores_the_exact_before_image_on_reopen`
- `crash_during_before_image_restore_restarts_idempotently_on_the_next_open`
- `crash_after_journal_committed_before_git_rolls_the_git_commit_forward_on_reopen`
- `crash_after_journal_committed_without_git_rests_as_git_pending_then_commits_once_git_exists`
- `crash_after_git_committed_before_index_current_reaches_index_current_on_reopen_and_is_stale_meanwhile`
- `second_crash_during_recovery_of_the_same_operation_still_converges` (double-crash on one `operation_id`)
- `retry_of_a_failed_operation_id_returns_failed_again_without_reapplying`
- `retry_after_a_restart_returns_the_outcome_recovery_reached`
- `reusing_an_in_flight_operation_id_for_different_content_is_refused_as_reused_not_in_progress`
- `reusing_an_operation_id_for_a_different_request_is_refused_not_reported_as_the_original_outcome`
- `a_read_concurrent_with_each_write_fault_point_returns_only_a_committed_version`
- `read_note_blocks_deterministically_while_another_thread_holds_the_notes_lock`
- `read_note_returns_inconsistent_on_hash_mismatch_never_a_before_image_substitution`
- `bounded_scan_misses_a_real_commit_beyond_the_depth_cap_but_unbounded_finds_it` (`git::tests`)
- `sec_001_toctou_window_between_path_check_and_write_still_exists`
- `sec_001_toctou_read_side_leaks_an_outside_records_version_and_bytes_into_the_before_image`
- `sec_002_read_path_rejects_both_aliasing_shapes_but_write_path_pre_read_does_not`
- `sec_003_hard_link_aliasing_is_invisible_to_canonicalization_after_a2`

No test was added, removed, or edited by this task. These counts are the output of Task 9's
closing tree, re-run verbatim.

### 2.2 `cargo deny check` — **FAILED on `advisories`** (new finding, not remediated by this task)

```
cargo deny check
```

**Result: `advisories FAILED, bans ok, licenses ok, sources ok`.**

- **`licenses ok`**: the three dependencies A2 added — `rusqlite` 0.31.0 (MIT, via
  `features = ["bundled"]`), `git2` 0.19.0 (MIT OR Apache-2.0), `sha2` 0.10.9 (MIT OR
  Apache-2.0) — and every one of their transitive dependencies pass the existing
  `deny.toml` license allowlist (`Apache-2.0`, `MIT`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`,
  `Zlib`, `Unicode-3.0`) with zero violations. This confirms the plan's Tech Stack section's
  license claim against `cargo deny`'s actual metadata, not against the plan's own assumption,
  per the brief's explicit instruction. **No new `dependency-exceptions.md` entry was needed
  for licensing** — see `eval/results/A2/dependency-exceptions.md`, which states this
  explicitly.
- **`bans ok`, `sources ok`**: clean except for two pre-existing informational `duplicate`
  warnings (`hashbrown` 0.14.5/0.17.1 via `rusqlite`→`hashlink` vs. `domain`→`serde_yaml`→
  `indexmap`; `syn` 2.0.119/3.0.9 via `thiserror` vs. `git2`'s own `url`→`idna`→`icu_*` chain)
  and three `license-not-encountered` informational warnings carried over unchanged from A1.
  None of these fail the check; they are the same class of informational noise A1's evidence
  file already recorded as non-blocking.
- **`advisories FAILED`**: three new RustSec advisories were found against `git2` 0.19.0,
  the exact version the plan pins (`crates/storage/Cargo.toml`: `git2 = "0.19"`). None of these
  existed, or at least were not yet published, when the plan was written or when Tasks 1–9 ran
  their own `cargo deny check` passes — this is a genuinely new finding surfaced only by this
  task's final, from-scratch re-run, not something earlier tasks missed.

  | Advisory | Issue | Fixed in |
  |---|---|---|
  | RUSTSEC-2026-0008 | UB dereferencing a `Buf` struct right after `Buf::new()`/`default()` (null pointer into `slice::from_raw_parts`) | `>=0.20.4` |
  | RUSTSEC-2026-0183 | UB in `Remote::list()` when the remote advertises no references (null pointer into `slice::from_raw_parts`) | `>=0.21.0` |
  | RUSTSEC-2026-0184 | UB constructing a `Signature` from a buffer-created `BlameHunk` with null author/committer pointers | `>=0.21.0` |

  **Applicability check (performed, not assumed):** `crates/storage/src/git.rs` — the only
  file in this workspace that uses `git2` — was grepped for every API surface named in the
  three advisories. It calls `git2::Repository::open`/`init`, `git2::TreeBuilder` (`insert`),
  `git2::Signature::now`, `git2::Oid::from_str`/`from_bytes`, and basic blob/commit creation.
  **It never calls `Remote::list()`, never constructs a `Blame`/`BlameHunk`, and never calls
  `Buf::new()`/`default()` directly** (`cargo deny`'s advisory is about `git2`'s own internal
  use of `Buf` reachable from certain call paths that this project's code does not take). This
  does not prove the running binary is unaffected under every possible code path inside
  `git2`/`libgit2` itself, but it does establish that none of the three advisories' own stated
  trigger conditions are reachable through code this project wrote.

  **Why this task did not fix it by upgrading `git2`:** a fix requires `git2 >= 0.20.4` or
  `>= 0.21.0`, both outside the `^0.19` range the plan's Cargo.toml pins
  (`crates/storage/Cargo.toml` line 17: `git2 = "0.19"`). `cargo update -p git2 --dry-run`
  confirms no compatible newer version exists under the current constraint; bumping the
  constraint itself is a dependency-version change to a production manifest, which this task's
  brief explicitly scopes out ("No code changes — this is a documentation/evidence task"). This
  task's own tool-use permissions independently flagged an attempted `deny.toml` advisory-ignore
  edit as a security-relevant action requiring explicit human approval (see Section 2.2.1) — so
  **no exception was added to `deny.toml` either**, and `cargo deny check` is left genuinely
  **red** at the close of this task, rather than silently suppressed.

  **This is recorded as an open, unresolved finding, not absorbed:** see
  `eval/results/A2/dependency-exceptions.md` for the full writeup, and Section 6 below ("What
  A2 does NOT guarantee") for its restatement as a limitation. The plan owner must decide,
  before merge, whether to (a) bump `git2` to `>=0.21` (a real dependency change, needing its
  own review since `git2-rs` 0.20/0.21 may carry other API or behavior changes), or (b)
  explicitly approve a time-bound `deny.toml` exception with the applicability analysis above
  as its justification, or (c) accept `cargo deny check` staying red until (a) is done. This
  task does not make that call unilaterally.

#### 2.2.1 Note on the blocked edit attempt

During this task, an `Edit` to `deny.toml` (adding `ignore` entries for the three new
advisories, mirroring the existing `RUSTSEC-2024-0449` exception's format) was **blocked by
the coding agent's own permission system**, which classified it as a "Security Weaken" action
requiring explicit user approval. This task respected that block and left `deny.toml`
byte-for-byte unchanged (confirmed via `git status --short deny.toml` showing no diff). This is
reported here as a procedural fact, not a recommendation either way — only the user/plan owner
can grant that exception.

### 2.3 `cargo clippy --workspace -- -D warnings`

```
cargo clippy --workspace -- -D warnings
```

**Result: clean. `Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.29s` with
zero warnings and zero errors emitted** (confirmed by grepping the full output for
`error|warning`, which matched nothing beyond the build-step summary line). This satisfies
Continuity [c11mp]'s standing requirement that clippy must pass clean before any task is
reported done, re-run one final time across the whole branch as the brief's Step 3 requires.

### 2.4 Mutation testing (`cargo mutants`)

```
cargo mutants --package domain --package storage --package policy
```

`cargo-mutants 27.1.0` was already installed in this environment (confirmed via
`cargo mutants --version` before concluding it was unavailable, per the brief's instruction).
196 mutants were found across the three crates; the unmutated baseline built and passed in
19s build + 6s test.

Full results are in Section 2.4.1, filled in after the run actually completed (11 minutes
wall-clock).

#### 2.4.1 Mutation testing results

**Final tally: 196 mutants tested in 11m — 131 caught, 50 unviable (did not compile), 1
timeout, 14 missed.**

`domain` and `policy` together had **34 mutants, 0 survivors** (27 caught + 7 unviable). All 14
missed mutants and the 1 timeout are in `storage`.

Every one of the 14 missed mutants is examined below against the actual source, per the
brief's acceptance threshold ("Ogni mutante sopravvissuto esaminato e giustificato o coperto da
un nuovo test"). No new test was added — per this task's "no code changes" scope, every
survivor below is **justified** as a documented test-coverage gap rather than closed, and
recorded here so it is not silently absorbed. If this task's scope had permitted new tests, the
IO-error-path cluster (items 1–13 below) is the concrete, actionable work a follow-up task
should do.

**Cluster A — un-exercised non-`NotFound` I/O error branches (13 of 14 survivors).** Every
item in this cluster shares one root cause: the test suite only ever induces a *missing file*
(`NotFound`) at these call sites — never a different I/O failure (permission denied, a locked
file, a directory where a file was expected, disk-full, etc.). Each of these call sites has a
`match`/`if` arm that specifically distinguishes `NotFound` (treated as an expected, recoverable
case) from any other `io::Error` (propagated as a hard failure). Mutating the guard
(`==`→`!=`) or collapsing it to a constant (`true`/`false`) changes which branch handles
`NotFound` versus "every other error," and no test fails, because no test ever produces the
"every other error" input in the first place.

1. `crates/storage/src/git.rs:221` — `open_repo`: `e.code() == git2::ErrorCode::NotFound` → `true`.
   Collapsing the guard to always-true would make *every* `git2::Repository::open` failure
   (not just "no repository here") map to `GitError::NoRepository`/`git_pending`, silently
   hiding a real git error (corrupt `.git`, permissions) behind the ordinary "git not
   initialized yet" path. No test opens a git repository in a state that fails for a reason
   other than "doesn't exist." **Justified as a real, if narrow, coverage gap**: a test that
   corrupts `.git` (e.g. truncates `.git/HEAD`) and asserts `GitError::Git2` rather than
   `NoRepository` would close it; not added here per this task's scope.
2. `crates/storage/src/recovery.rs:272` (×2: the guard-to-`true` mutant and the `==`→`!=`
   mutant) — `fix_content_persisted`'s on-disk-read: same pattern, deciding whether a missing
   record (`None`, to be recreated from before-image) or a different I/O error
   (`ReconcileError::Io`, hard-fails reconcile) is reported. No test makes this specific read
   fail with anything but `NotFound`. **Justified**, same gap shape as item 1.
3. `crates/storage/src/recovery.rs:290` — `on_disk_hash.is_some() && journal.last_committed_for_note(...)?.is_none()` →
   `&&`→`||`. This guards whether reconcile deletes an unverified record with no
   before-image when no committed row exists for the note. Flipping to `||` would delete the
   record even when `on_disk_hash` is `None` (nothing to delete) or even when a committed row
   *does* exist for the note (which the comment explicitly says must be preserved: "the record
   is left alone rather than deleting content another row may describe"). No test constructs
   the specific combination needed to observe the behavioral difference (a `ContentPersisted`
   row with no before-image, no on-disk file, on a note that nonetheless already has a
   different committed row). **Justified as a real gap worth a dedicated test**, not added
   here.
4. `crates/storage/src/recovery.rs:293` (×3: guard-to-`true`, guard-to-`false`, `==`→`!=`) —
   the `remove_file` cleanup inside that same branch: distinguishes "file was already gone"
   (fine) from "removal failed for another reason" (`ReconcileError::Io`). Same
   un-exercised-non-`NotFound`-error pattern as items 1–2. **Justified**, same gap shape.
5. `crates/storage/src/recovery.rs:423` (×3: guard-to-`true`, guard-to-`false`, `==`→`!=`) —
   `own_content`'s on-disk read, deciding whether a missing file (fall through to checking
   successor before-images) or another I/O error (hard-fails reconcile) is reported. Same
   pattern again. **Justified**, same gap shape.
6. `crates/storage/src/vault.rs:468` — `read_note_locked`'s read: `e.kind() ==
   std::io::ErrorKind::NotFound`, `==`→`!=`. Same pattern on the live read path: distinguishes
   `StorageError::NotFound` from `StorageError::Io`. No test makes this specific read fail with
   a non-`NotFound` error. **Justified**, same gap shape.
7. `crates/storage/src/vault.rs:1088` — `delete_before_image`: guard-to-`true` on the
   `remove_file`'s `NotFound` arm. Same pattern; a mutant that always treats removal failure as
   "already gone" would mask a genuine before-image deletion failure. **Justified**, same gap
   shape.
8. `crates/storage/src/vault.rs:1101` — `read_before_image`: `==`→`!=` on the same `NotFound`
   distinction for reading a before-image back. **Justified**, same gap shape.

   **Why this cluster is accepted as justified rather than closed in this task:** closing it
   properly means inducing a *real*, non-`NotFound` I/O failure at eight distinct call sites —
   on Windows, the practical options are limited (no `chmod` semantics; a file locked by another
   handle, or a directory substituted for a file, are the realistic levers) and doing this
   properly for all eight would mean either new test infrastructure (an injectable I/O layer)
   or OS-specific test code, which is new test-writing work, not evidence-gathering. It is
   recorded here as the single concrete, actionable follow-up this mutation-testing pass
   surfaced, per the spec's "justified or covered by a new test" standard — justified, not
   covered, within this task's own scope.

**Cluster B — a semantically equivalent mutant (1 of 14 survivors).**

9. `crates/storage/src/journal.rs:79` — `AdvanceFields::none() -> Self` body:
   `Self::default()` → `Default::default()`. `Self` and `AdvanceFields` are the same type at
   this point, so `Self::default()` and `Default::default()` compile to the identical call —
   this is a **mutant-equivalence false positive**, not a real coverage gap. No test can ever
   distinguish these two expressions because they produce bit-for-bit identical values by
   construction. **Justified as inherently unkillable**, not a defect in test coverage.

**The 1 timeout**, `crates/storage/src/fault.rs:115` (`pause.point == point` → `!=`, inside
`FaultInjector::check`'s `#[cfg(test)]`-only pause-handling branch): this line decides whether
the *current* call site matches the point a test asked to pause at. Flipping it to `!=` makes
every pipeline invocation pause at every point **except** the one requested, which hangs any
test using `FaultInjector::pause_at` (there are several — e.g.
`read_blocks_behind_a_paused_writer_holding_the_same_note_lock_and_then_sees_the_fully_committed_result`)
until `cargo-mutants`' timeout kills it. This is correctly counted as mutant-testing *working
as intended* (a real behavioral difference the suite does detect, just via a hang rather than
a clean assertion failure) against test-harness-internal scaffolding code, not production C1
logic — `FaultInjector`'s pause mechanism exists only under `#[cfg(test)]` and is not reachable
in a production build. **Not a coverage gap**: the mutant is effectively caught (the hang is a
test failure under `cargo-mutants`' own accounting it just classifies separately from a clean
`MISSED`/`CAUGHT` outcome), and no production guarantee is affected.

**Summary disposition:** 0 of the 14 survivors represent a production correctness defect. 13
are a single recurring, real test-coverage gap (non-`NotFound` I/O error branches, documented
above as the one concrete follow-up this pass surfaced) and 1 is a mutant-equivalence false
positive. The 1 timeout is scaffolding-only and not a production-code gap. Raw tool output
(`missed.txt`, `timeout.txt`, `caught.txt`, `unviable.txt`) was produced by
`cargo mutants --package domain --package storage --package policy` and is not committed to
the repository (it is a local, regenerable tool artifact, consistent with A1's own practice of
not committing `mutants.out/` directories).

---

## 3. What A2 now guarantees

Per Continuity [c5ja]'s extension to A2, stated explicitly and only as far as the plan actually
implements:

- **Durable crash recovery per the C1 state machine, across all 8 fault-matrix rows.** Every
  write drives `domain::WriteState` through `IntentRecorded → ContentPersisted →
  JournalCommitted → GitCommitted → IndexCurrent`, persisting each transition to a SQLite
  journal (`rusqlite`, bundled) before performing that step's filesystem/git effect, with
  `Recovering`/`Failed` as the two side states. `domain::WriteState::valid_transition`
  (`crates/domain/src/write_state.rs`) is tightened to exactly this table, including the two
  edges added for correctness (`ContentPersisted -> Failed` for a rolled-back write, and the
  narrowed `Recovering`-outgoing set matching C1's actual recovery table rather than A1's
  broader placeholder). A crash at any of the 8 named fault-matrix points is recovered
  deterministically by `Vault::recover()`/`reconcile`, run at every `Vault::open`, before any
  other `storage` call is permitted to proceed — proven by the six `crash_*` tests in
  `crates/storage/src/vault.rs` (Section 2.1) plus the dedicated double-crash test
  (`second_crash_during_recovery_of_the_same_operation_still_converges`).
- **Idempotent retry, including argument-mismatch detection.** Retrying the same
  `operation_id` against a terminal `Failed` operation returns `Failed` again without
  reapplying anything (`retry_of_a_failed_operation_id_returns_failed_again_without_reapplying`).
  Retrying a non-terminal operation continues with that same id and returns the outcome already
  reached, never double-applying
  (`retry_after_a_restart_returns_the_outcome_recovery_reached`,
  `retry_of_a_git_pending_operation_commits_its_own_content_once_git_exists`,
  `retry_of_a_git_committed_operation_completes_the_index_step_exactly_once`). Reusing an
  in-flight `operation_id` with *different* request content is refused as reused, not treated
  as "in progress" or silently merged
  (`reusing_an_in_flight_operation_id_for_different_content_is_refused_as_reused_not_in_progress`),
  surfaced via `StorageError::OperationIdReused` and backed by validating a retry's request
  against the originally recorded request, not just against the `operation_id` alone (Task 5's
  fix, `d758d5e`).
- **Before-image rollback.** A new version's content is durably persisted as a before-image
  before the rename that replaces it, and deleted only after `GitCommitted` — never before (C1
  "Before-image"), proven by the crash-matrix tests that interrupt mid-write and confirm the
  before-image is what gets restored on reopen. A rolled-back write (hash verification failure)
  ends at `Failed`, with its `new_version` field preserved as a truthful record of what it
  failed to commit, never reinterpreted as success and never committed to git under its
  `operation_id` (plan's Global Constraints, Task 7 correction).
- **Idempotent git commit with repo-wide serialization.** Each operation's git commit is built
  via `git2::TreeBuilder` seeded from `HEAD` plus that operation's own blob — never through the
  shared `.git/index` — so concurrent commits cannot interleave staged-but-uncommitted content
  from different operations. The journal's own `git_commit_oid` column is the primary O(1)
  idempotency source; a bounded history scan backs the live write path, while `reconcile` uses
  an unbounded scan (Section 6 discusses its residual risk). Commits are serialized via a
  repo-wide lock, closing the git-commit-concurrency gap found during Task 5's review (subject
  to the residual `HEAD`-ordering limitation in Section 5, item 5).
- **Reads resolve through the journal, verified by hash, and share the writer's lock.**
  `read_note`/`current_version` resolve exclusively through
  `Journal::last_committed_for_note`, keyed by note id, verified by content hash against the
  on-disk file — never by opening "whatever file currently sits at the record path" — and share
  the writer's per-note lock so a read can never observe a file mid-rename before its writer's
  journal advance makes the new version official
  (`read_note_resolves_through_the_journal_not_the_raw_file`,
  `read_blocks_behind_a_paused_writer_holding_the_same_note_lock_and_then_sees_the_fully_committed_result`,
  `read_note_returns_inconsistent_on_hash_mismatch_never_a_before_image_substitution`).

## 4. What A2 still does NOT guarantee

Restated per the plan's Global Constraints, verbatim in substance:

- **No encryption, no deletion registry, no backup/restore.** These are A3's scope (spec item
  5–6 of Fase A consegne, Continuity [d1ec]). A2 implements none of them.
- **No real search/graph indexing.** `domain::IndexState { Current, Stale }` is a
  reported-state stub only (`crates/domain/src/index_state.rs`). **No A2 code ever returns
  `IndexState::Current`**: `WriteState::IndexCurrent` is a write-pipeline completion marker,
  strictly distinct from the honestly-`Stale`-throughout `domain::IndexState`, per the plan's
  explicit architectural correction. Real indexing (trigrams, BM25, tree-sitter) is B1's scope.
- **SEC-001/002/003 are carried forward, not closed.** Per Task 9's adversarial
  re-evaluation (`eval/results/A2/sec-debt-reevaluation.md`), summarized here and not
  duplicated in full:
  - **SEC-001 (TOCTOU/symlink race):** `OPEN, UNCHANGED`. Both a write-side outcome (an
    attacker's symlink planted mid-race is destroyed by `atomic_write`'s rename rather than
    written through) and a read-side outcome (an outside record's version and content bytes
    can be pulled into the vault's own before-image store via the pre-write version-check
    read, which does follow a symlink) were demonstrated with concrete, passing adversarial
    tests.
  - **SEC-002 (intra-vault symlink aliasing):** `NARROWED (read path only); write-path
    pre-read unchanged and open`. Two read-side sub-cases are incidentally caught (a
    never-written aliased note id is rejected `NotFound` by the per-note-id journal lookup; an
    already-committed note later aliased to different content is rejected `Inconsistent` by the
    unconditional hash check) — neither is an anti-aliasing check by design, and both have real
    limits (DoS-only; defeated by identical-content aliasing; no defense against direct journal
    tampering). The write path's pre-read is a structurally separate code path with no
    journal-identity gating at all, and leaks cross-note content into the attacker's note's
    before-image with no race required.
  - **SEC-003 (hard-link aliasing):** `NARROWED (read path only); write-path pre-read and
    outward hard-link exposure unchanged`. Same two read-side mechanisms and limits as SEC-002
    for the inward direction; the outward direction (a hard link created outside the vault
    exposing the vault's own content with zero `Vault` code involved) is untouched by A2 and is
    not a code path this crate owns at all.

  **Mandatory restatement (Continuity [d5b9]):** all three debts require re-evaluation again
  before Fase D. None of A2's hash verification should be read as proof that any of the three
  is resolved — a hash check only detects aliasing/TOCTOU after the fact; it does not prevent
  the race. Full detail, reproduction steps, and the independent-review findings that corrected
  the brief's own original predictions are in
  `eval/results/A2/sec-debt-reevaluation.md`.
- **`git2` 0.19.0 carries three open, unresolved RustSec unsoundness advisories** as of this
  task's close (Section 2.2): RUSTSEC-2026-0008, -0183, -0184. This project's own use of
  `git2` does not call any of the three advisories' stated trigger APIs
  (`Remote::list()`, `Blame`, direct `Buf` dereference), but `cargo deny check` is left
  genuinely failing (`advisories FAILED`) at A2's close — neither upgraded nor exempted — per
  Section 2.2's reasoning. This is a real, unresolved gap in the dependency/license acceptance
  criterion, not a pass with an asterisk.

## 5. Known limitations accepted during this plan's execution

These were surfaced and explicitly deferred across Tasks 5–9's independent review rounds.
Flagged here per Step 5's own requirement, not silently absorbed.

1. **(Task 5, deferred Minor)** An unborn `HEAD` (user ran `git init` but has no commits yet)
   makes `repo.head()` fail, surfacing as a hard `StorageError::Git` on every write rather than
   being handled gracefully.
2. **(Task 5, deferred Minor)** Hot-path cost: a new operation's git-commit lookup always
   walks the full `Bounded(500)` history on both `find_commit_for_operation` and
   `commit_operation`'s own re-entry check — up to 1000 commit reads per write once history is
   long, and the lookup holds the global journal mutex for the duration, blocking all journal
   access (including reads) during the walk.
3. **(Task 5, deferred Minor)** `record_path.strip_prefix(repo_path).unwrap_or(record_path)`
   silently falls back to an absolute path on failure, which would then fail inside
   `TreeBuilder` with an unclear error (currently unreachable given note-id constraints, but
   worth a clearer error if the constraints ever change).
4. **(Task 5, deferred design observation)** `commit_operation` never updates the shared
   `.git/index`, so the on-disk index falls further behind `HEAD` with every automated write.
   If the vault's human operator later runs a plain `git add`/`git commit` on unrelated
   changes, the stale index could cause that manual commit to appear to delete every note
   `storage` has committed since. Accepted as within the project's "trusted single operator"
   threat model, but flagged as a real operational gotcha.
5. **(Task 5, fix round, new Minor)** After the git-commit-concurrency fix (serializing
   commits via a repo-wide lock), a specific mixed-ordering scenario (an older operation's git
   commit recovers successfully via reconcile while a newer operation on the same note already
   committed live) can leave git's `HEAD` tree showing the OLDER operation's content for that
   note, even though the journal/filesystem/`read_note` all correctly show the newer content.
   No data loss, no misattribution of content to the wrong operation — but `HEAD` can
   transiently disagree with the vault's true current state until the next write to that note.
   Self-heals on the next write. Narrow trigger: needs a specific
   transient-git-failure-then-recovery-then-live-success ordering.
6. **(Task 7, fix round, deferred Minor)** Tautological git-existence assertions in some tests
   check only that the journal's recorded `git_commit_oid` is set, not that a real commit
   object with that Oid actually exists in the repository — a real but narrow test-rigor gap,
   not a production defect.
7. **(Task 7, deferred Minor)** `open_reconciled` discards the diagnostic `reason` string from
   `ReconcileError::Inconsistent` when surfacing it as a top-level `StorageError`, losing
   diagnostic detail a human operator would want when a vault refuses to open.
8. **(Task 7, deferred Minor)** A before-image can be orphaned (never deleted) if a crash
   happens between `reconcile`'s `GitCommitted` advance and its subsequent
   `delete_before_image` call — a pre-existing pattern already present in the live write path
   (not new to recovery), and a storage leak only, not a correctness issue.
9. **(Task 7, deferred Minor)** `reconcile`'s row 3 treats "the record file is unreadable" the
   same as "git itself is unavailable," both resolving to a `git_pending`-style resting state —
   reasonable behavior, but worth a code comment since the two causes are different.
10. **(Task 8, deferred Important — explicitly NOT fixed, intentionally deferred by the plan
    owner, not a Task 10 action item, but documented here as required)** Out-of-order retries
    can move git's `HEAD` backward: if a caller retries two git-pending operations on the same
    note in the wrong order (the newer operation's retry call happening before the older
    operation's retry call), `HEAD`'s tree can end up holding the OLDER operation's content
    under its own correct `operation_id` marker, while the journal/filesystem continue to
    correctly serve the newer content. This is caller-triggerable (not just
    crash-timing-dependent, unlike item 5 above), though it requires a caller to deliberately
    retry out of order. Reads are unaffected. No guard was implemented; the plan owner (not
    Task 10) explicitly deferred this.
11. **(Task 9, the task's own primary deliverable)** SEC-001/002/003 — see Section 4 above for
    the restated verdicts and `eval/results/A2/sec-debt-reevaluation.md` for full detail. The
    mandatory Fase-D re-evaluation obligation (Continuity [d5b9]) applies to all three.
12. **(Per the plan's own Global Constraints, required explicitly)** `reconcile`'s
    `ScanDepth::Unbounded` git-history scan has no stated worst-case latency bound on a vault
    with very large git history. This is accepted deliberately (correctness over speed in the
    rare recovery case; the scan only triggers for a row sitting at `JournalCommitted` with no
    `git_commit_oid` yet — not on every ordinary `Vault::open`). No timeout or bounded
    first-pass was implemented; flagged here for revisit if it proves too slow in practice, not
    silently absorbed.

    **Fifth-review correction, recorded verbatim (per the plan's explicit instruction):** an
    `Unbounded` scan proves non-existence only among commits reachable from `HEAD`, not
    absolute non-existence. If the vault's git history is ever rewritten out-of-band (a manual
    `git reset`/rebase/force-push by the vault's own user, per this repo's one-user/one-laptop
    threat model) between an operation's original commit and a later recovery pass, a commit
    `reconcile` is looking for can become unreachable from `HEAD` without A2 ever touching it.
    `reconcile`'s row-3 fix-up then cannot distinguish "this was genuinely never committed"
    from "this was committed but the history was later rewritten out from under it," and on an
    unbounded miss it calls `commit_operation` regardless, accepting the small residual risk of
    creating a duplicate commit under that operation's `operation_id` marker in the rare
    rewritten-history case. This is accepted as outside this plan's fault model — A2 itself
    never rewrites history; only external, out-of-band action triggers it — and the plan
    deliberately does not invent a new "fail closed on scan miss" behavior for row 3, because
    doing so would make every ordinary, legitimate "git was never committed yet" case (the
    overwhelming majority of row-3 hits) indistinguishable from the rare rewritten-history case
    and would block `Vault::open` on the common case to guard against the rare one. Flagged
    here for revisit if ever observed in practice, not silently absorbed.
13. **(New design judgment calls from Tasks 6/7, resolved-not-defects, noted for completeness)**
    `Vault::open`'s 3-argument form (explicit indexer) is `pub(crate)`, not `pub`, because the
    `Indexer` trait itself is `pub(crate)` per the plan's own spec —
    `Vault::open_with_default_indexer` is the real public constructor. Judged sound by two
    independent reviews (Tasks 6 and 7): nothing outside `storage` needs to supply a custom
    indexer in this phase, and `storage`'s only truly intended public write entry point is
    `propose_and_commit` regardless (Continuity [c1p2]).
14. **(Task 10, new finding, not a deferred item from an earlier task — see Section 2.2)**
    `git2` 0.19.0's three RustSec unsoundness advisories (RUSTSEC-2026-0008/0183/0184), left
    unresolved at this task's close pending an explicit decision from the plan owner between
    upgrading `git2` (a dependency-version change outside this task's scope) or granting a
    time-bound `deny.toml` exception (an action this task's own permission system flagged as
    requiring explicit human approval, and which was therefore not performed unilaterally).
15. **(Task 10, new finding from this task's own mutation-testing pass, Section 2.4.1)** 13 of
    `storage`'s 14 surviving mutants share one recurring, real test-coverage gap: no test
    induces a non-`NotFound` I/O error (permission denied, locked file, etc.) at eight call
    sites across `git.rs`, `recovery.rs`, and `vault.rs` that each explicitly branch on
    `NotFound` vs. "any other I/O error." Each survivor is individually justified in Section
    2.4.1, not closed — closing it would require new test infrastructure (an injectable I/O
    layer, since Windows offers no `chmod`-style lever) or OS-specific test code, which is new
    test-writing work this documentation task does not perform. The 14th survivor
    (`journal.rs:79`) is a mutant-equivalence false positive (`Self::default()` vs.
    `Default::default()` on the same type), not a coverage gap at all.

---

## 6. Acceptance-criteria mapping (spec Fase A table, A2's additions)

| Acceptance criterion | Status | Evidence |
|---|---|---|
| Test suite green (workspace) | **PASS** | Section 2.1: 199/199 passed, 0 failed. |
| Every C1 fault-matrix row covered by a dedicated crash test | **PASS** | Six `crash_*` tests plus the double-crash test, `crates/storage/src/vault.rs`. |
| Retry idempotency (terminal and non-terminal states) | **PASS** | `retry_of_a_failed_operation_id_returns_failed_again_without_reapplying`, `retry_after_a_restart_returns_the_outcome_recovery_reached`, and the full `retry_*` family in Section 2.1. |
| Argument-mismatch detection on retry (`OperationIdReused`) | **PASS** | `reusing_an_in_flight_operation_id_for_different_content_is_refused_as_reused_not_in_progress`, `reusing_an_operation_id_for_a_different_request_is_refused_not_reported_as_the_original_outcome`. |
| Before-image rollback, never indistinguishable from success | **PASS** | Crash-matrix restore tests; `new_version_with_an_invalid_hash_is_detected_by_the_writer_and_restored_from_the_before_image`. |
| Reads resolve through journal, verified by hash, sharing writer's lock | **PASS** | `read_note_resolves_through_the_journal_not_the_raw_file`, `read_blocks_behind_a_paused_writer_holding_the_same_note_lock_and_then_sees_the_fully_committed_result`. |
| Idempotent git commit, isolated staging, repo-wide serialization | **PASS** (subject to limitation 5) | `retry_of_a_git_pending_operation_commits_its_own_content_once_git_exists`, `retry_of_a_git_pending_operation_after_a_successor_overwrote_it_commits_its_own_content_not_the_successors`. |
| `domain::WriteState::valid_transition` tightened to C1's exact table | **PASS** | `crates/domain/src/write_state.rs`; see Section 3. |
| `domain::IndexState` never reports `Current` in A2 | **PASS** | `crates/domain/src/index_state.rs`; `write_record_unchecked_reaches_index_current_pipeline_state_but_index_state_for_stays_stale`. |
| `storage`'s only `pub` write entry point is `propose_and_commit` | **PASS** | `storage_exposes_exactly_one_public_write_function` (unchanged from A1, re-verified running). |
| `cargo clippy --workspace -- -D warnings` clean | **PASS** | Section 2.3. |
| Dependency/license allowlist check (`cargo deny check`) | **FAIL (advisories only)** | Section 2.2: `licenses ok`, `bans ok`, `sources ok`; `advisories FAILED` on three new `git2` RustSec entries, left unresolved — see Section 2.2 and `eval/results/A2/dependency-exceptions.md`. |
| Mutation testing examined (`domain`, `policy`, `storage`) | **PASS** (every survivor examined and justified, per spec threshold) | Section 2.4.1: 196 mutants, 131 caught, 50 unviable, 1 timeout, 14 missed — all 14 individually examined; 13 share one documented I/O-error-path coverage gap, 1 is a mutant-equivalence false positive; 0 are production defects. `domain`/`policy`: 34 mutants, 0 survivors. |
| SEC-001/002/003 re-evaluated against the actual A2 design | **DONE (all three remain open/narrowed, not closed)** | Section 4; `eval/results/A2/sec-debt-reevaluation.md`. |

---

## 7. Boundary restatement (Continuity [c5ja] extended to A2)

A2 does not provide anything beyond what this plan implements. In particular: no encryption,
no deletion registry, no backup/restore (A3); no real index (B1); SEC-001/002/003 remain open
or only narrowed, not closed, and require re-evaluation before Fase D (Continuity [d5b9]); and,
as of this task's close, `cargo deny check` does not pass cleanly — the `git2` advisory
findings in Section 2.2 are a genuine, unresolved gap in A2's acceptance evidence, reported
here rather than concealed.

Per the brief's Step 7 and the pattern established in A1 (Continuity [c12av]): **this branch is
not to be merged to main without explicit user approval**, even though the test suite and
clippy are green — the `cargo deny` failure and the SEC-debt verdicts above must be presented
to the user alongside this evidence before any merge decision.
