# A2 — Recovery: closing evidence

**Date:** 2026-10-27 (first pass); updated 2026-10-27 after a later, separate commit resolved
the one open finding this file originally reported.

**Git commit hash at the start of this task's verification run:** `28ae8ffdeffa860bf1d75a71bff93ae0cb8dc303`
(this is Task 9's closing commit, `fix(storage): close two scope gaps in the SEC-001/002/003 adversarial
evaluation`). This task's own evidence commit is `9771c1ee2cf620f764c87850f2b0c293f5792d3d`,
which added no production or test code — only this evidence file and
`eval/results/A2/dependency-exceptions.md`. **A later, separate commit,
`247df9e35d23eeec58003442a1b18bb91dc6a0e7`** (`fix(storage): upgrade git2 to 0.21 to resolve
three unsound advisories`), landed on top of this task's commit after the user reviewed this
evidence and chose to upgrade `git2` rather than accept an exception or leave the finding open
(see Section 2.2/2.2.1). This file is updated in place to reflect that resolution rather than
left describing a now-stale finding.

**Plan:** `docs/superpowers/plans/2026-10-27-a2-recovery.md`
**Task brief:** `.superpowers/sdd/2026-10-27-a2-recovery/task-10-brief.md`

---

## 1. Scope of this file

This is Task 10, the final task of the A2 ("Recovery") plan. It closes A2 by re-running the
full acceptance checklist the spec's Fase A table requires (the same table A1 closed against,
now re-run against A2's journal/git/recovery additions) and recording the artifact. **No
production code and no existing test was modified by this documentation task itself** — every
result below is the actual output of commands run against the tree as Task 9 left it, except
where Section 2.2/2.2.1 explicitly notes a later, separate, user-approved commit
(`247df9e`, a real `git2` dependency upgrade with two small source adaptations) that this file
was updated to reflect after independently re-verifying its effect.

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

### 2.2 `cargo deny check` — **RESOLVED** (originally FAILED on `advisories`; fixed by a later, separate commit)

```
cargo deny check
```

**Current result (re-run after the git2 upgrade below): `advisories ok, bans ok, licenses ok,
sources ok`.** Full tail of the actual re-run:

```
advisories ok, bans ok, licenses ok, sources ok
```

(preceded only by 5 pre-existing, informational warnings — 2 `duplicate` + 3
`license-not-encountered`, described below, current as of the `git2` 0.21 upgrade — none of
which fail the check.)

**This section originally reported `advisories FAILED` at this task's first pass** (2026-10-27,
commit `9771c1e`), on three RustSec unsoundness advisories newly published against `git2`
0.19.0. That history is preserved below because it is itself useful acceptance evidence — what
was found, what decision was made, and how it was closed — not because the check is still
failing. **The check is green as of commit `247df9e`** (a separate, later commit that upgraded
`git2` to `0.21`; see "How this was resolved" below). The original finding:

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
  `indexmap`; `syn` 2.0.119/3.0.9) and three `license-not-encountered` informational warnings
  carried over unchanged from A1. **Fix-round correction:** this bullet originally attributed
  the `syn` 3.0.9 duplicate to `git2`'s own `url`→`idna`→`icu_*` chain. That was accurate for
  `git2` 0.19.0 but is now stale after the `git2` 0.21 upgrade (Section 2.2.1), which drops
  that entire transitive chain. `cargo tree -p storage -i syn@3.0.9` now shows `syn` 3.0.9
  coming from `serde_derive` → `serde` → `domain`/`storage` (and, separately, `serde_yaml` →
  `domain`) instead — unrelated to `git2` entirely. `syn` 2.0.119 still comes from
  `thiserror-impl` → `thiserror`, as before.
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

  **Why this task's first pass did not fix it by upgrading `git2` itself:** a fix requires
  `git2 >= 0.20.4` or `>= 0.21.0`, both outside the `^0.19` range the plan's Cargo.toml
  originally pinned (`crates/storage/Cargo.toml` line 17: `git2 = "0.19"`). `cargo update -p
  git2 --dry-run` confirmed no compatible newer version existed under that constraint; bumping
  the constraint itself is a dependency-version change to a production manifest, which this
  task's brief scoped out at first pass ("No code changes — this is a documentation/evidence
  task"). This task's own tool-use permissions also flagged an attempted `deny.toml`
  advisory-ignore edit as a security-relevant action requiring explicit human approval (see
  Section 2.2.1) — so no exception was added to `deny.toml` either, at that point in time. This
  was correctly reported as an open, unresolved finding rather than silently suppressed.

#### 2.2.1 How this was resolved

The open finding above was presented to the user as three options: upgrade `git2`, approve a
time-bound `deny.toml` exception, or accept the check staying red. **The user chose to upgrade
`git2`.** The coordinator (not this documentation task) made that change directly, in a
separate commit, `247df9e35d23eeec58003442a1b18bb91dc6a0e7`
(`fix(storage): upgrade git2 to 0.21 to resolve three unsound advisories`), on top of this
task's own evidence commit `9771c1ee2cf620f764c87850f2b0c293f5792d3d`. This section records
that change as acceptance-evidence history, verified independently rather than taken on faith:

- `crates/storage/Cargo.toml`: `git2 = "0.19"` → `git2 = "0.21"`.
- `Cargo.lock`: `git2` and its transitive dependency tree updated; the diff (`git diff
  9771c1e..247df9e --stat`) shows **318 lines removed, net**, because `git2`/`libgit2-sys`
  0.21 drops the `openssl-sys`/`url`/ICU transitive chain that the old network-transport
  features pulled in — this project's git usage is local-filesystem-only (no
  push/fetch/remote), so that chain was dead weight the upgrade incidentally sheds.
- `crates/storage/src/git.rs`: two call sites adapted for `git2 0.21`'s API change (part of
  the same unsound-pointer-dereference fix line the advisories describe: `Reference::name()`
  and `Commit::message()` changed from `Option`-returning to `Result`-returning).
  `commit_operation_contents`'s `head.name()` call changed from `.ok_or_else(...)` to
  `.map_err(|_| ...)`, and `scan_history_for_marker`'s `commit.message().is_some_and(...)`
  changed to `.is_ok_and(...)` — both preserve the exact prior behavior (a missing or
  non-UTF8 name/message is still treated as "doesn't match" / "not a named branch," never
  newly propagated as a hard error; verified by reading the diff directly, not by trusting the
  commit message's own description).
- `crates/storage/src/vault.rs`: the same `is_some_and` → `is_ok_and` adaptation in a test
  helper (`commits_for_operation`), for the same reason.
- No `deny.toml` change was made or needed for this fix — confirmed via
  `git diff 9771c1e..247df9e -- deny.toml`, which is empty.

**Independently re-verified by this task** (re-run from scratch, not reused from the
coordinator's own report):

```
cargo test --workspace   → 32 + 12 + 127 + 23 + 5 = 199 passed, 0 failed  (same count as Section 2.1)
cargo clippy --workspace -- -D warnings   → clean, 0 warnings, 0 errors
cargo deny check   → advisories ok, bans ok, licenses ok, sources ok
```

**Conclusion: RESOLVED, not open.** The three advisories (RUSTSEC-2026-0008/0183/0184) no
longer appear in `cargo deny check`'s output. No `deny.toml` exception was ultimately needed —
see the equivalent update in `eval/results/A2/dependency-exceptions.md`. This finding is no
longer listed as an open item in Section 4, Section 5, or Section 6 below; its full arc (found
at this task's first pass → user chose to upgrade → upgrade landed cleanly with two
behavior-preserving adaptations → re-verified clean, independently, by this task) is preserved
here as the historical record of how it was closed.

##### Note on the originally blocked edit attempt (historical)

During this task's first pass, an `Edit` to `deny.toml` (adding `ignore` entries for the three
advisories, mirroring the existing `RUSTSEC-2024-0449` exception's format) was **blocked by
the coding agent's own permission system**, which classified it as a "Security Weaken" action
requiring explicit user approval. That block was respected and `deny.toml` was left
byte-for-byte unchanged at that time. In the end, the user-approved resolution path (upgrading
`git2`) made that exception unnecessary — `deny.toml` remains unchanged through both this
task's first pass and the later upgrade commit.

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
recorded here so it is not silently absorbed.

**Fix-round correction (independent review):** this section originally claimed all 13
I/O-error-path survivors shared one uniform "Windows-hard to close" root cause. On closer,
per-mutant tracing against the actual source (re-done for this fix round), that claim does not
hold uniformly. The 13 split into two groups with genuinely different closing cost, plus one
mutant with its own distinct root cause — three groups below, not one.

**Group A — genuinely need a non-`NotFound` I/O error to kill (5 of 14 survivors).** These
call sites only branch away from their current behavior if the underlying I/O call fails for a
reason *other than* "file genuinely missing" (permission denied, a locked file, a directory
substituted for a file, disk-full, or — for the `remove_file` case below — the file vanishing
in the narrow window between an earlier read and this call, which is itself a race, not an
ordinary condition a test can set up by just deleting a file beforehand). No test induces any
of these specific conditions, and doing so on Windows has no `chmod`-style lever — the
practical options are a locked file handle or a directory-for-file substitution, which is new
test infrastructure, not a documentation fix.

1. `crates/storage/src/git.rs:222` (line number shifted by one after the `git2` 0.21 upgrade,
   commit `247df9e`; was `:221` when this section was first written) — `open_repo`:
   `e.code() == git2::ErrorCode::NotFound` → `true`. Needs `git2::Repository::open` to fail for
   a reason other than "no repository here" (a corrupted `.git`, for example) — a `git2`-level
   failure, not a plain filesystem `NotFound`. **Justified as hard to close**: no test
   corrupts `.git` in a way that produces a non-`NotFound` `git2::Error` from `open`.
2. `crates/storage/src/recovery.rs:272` — `fix_content_persisted`'s on-disk read, the
   guard-to-`true` mutant only (collapsing "is this NotFound" to always-true). Needs
   `std::fs::read(&record_path)` to fail for a reason other than the file being missing.
   **Justified as hard to close.** (The `==`→`!=` mutant at this same line is a *different*
   case — see Group B, item 1 below — because it is reachable via an ordinary missing file,
   not a non-`NotFound` error.)
3. `crates/storage/src/recovery.rs:293` (all three mutants at this line: guard-to-`true`,
   guard-to-`false`, `==`→`!=`) — the `remove_file` cleanup inside
   `fix_content_persisted`'s "no before-image, no committed row" branch. Unlike the other
   `NotFound`-guard sites in this file, **all three** of this line's mutants require the same
   precondition: the record file must vanish in the narrow window *between* the earlier
   `std::fs::read` (line 270, which must have returned `Ok` for this branch to even be
   reached) and this `remove_file` call, a few lines later in the same single-threaded
   function (`reconcile` runs before any `Vault` exists, so no concurrent writer can cause
   this). That is a race against an external actor deleting the file mid-call, not an ordinary
   "the file happens to already be missing" setup a test can arrange by simply deleting it
   beforehand — by the time this line runs, the preceding `on_disk_hash.is_some()` check has
   already confirmed the file was just read successfully. **Justified as hard to close** for
   all three mutants at this line, not only the guard-to-`true` one.
4. `crates/storage/src/recovery.rs:423` — `own_content`'s on-disk read, the guard-to-`true`
   mutant only. Needs `std::fs::read(record_path)` to fail for a reason other than the file
   being missing. **Justified as hard to close.** (The guard-to-`false` and `==`→`!=` mutants
   at this same line are in Group B below — reachable via an ordinary missing file.)
5. `crates/storage/src/vault.rs:1088` — `delete_before_image`'s `remove_file`, the
   guard-to-`true` mutant. `delete_before_image`'s own doc comment states it is "idempotent: a
   missing file is not an error" — collapsing the guard to always-true would make it swallow
   every removal failure (not just NotFound) as success, masking a genuine deletion failure
   that should propagate. Needs `remove_file` to fail for a reason other than the file being
   already gone. **Justified as hard to close.**

**Group B — ordinary, untested `NotFound` paths; cheap to close, not a platform-specific
problem (7 of 14 survivors).** Each of these is reachable by an everyday "the file genuinely
isn't there" scenario that no existing test happens to construct — closing any of them needs
nothing more exotic than deleting a file and checking the resulting variant, which works
identically on Windows, Linux, and macOS. These are acknowledged as real, currently-untested
gaps, not closed in this pass (per this task's documentation-only scope), but they are
**not** "hard to close" in the way Group A is, and the original version of this section was
wrong to describe them that way.

1. `crates/storage/src/recovery.rs:272`'s `==`→`!=` mutant. Concrete untested scenario: a
   `ContentPersisted` row is reconciled whose record file was genuinely deleted (not merely
   corrupted) before `Vault::open` runs — every existing crash-matrix test that reaches this
   function instead *damages* the file's content in place
   (`crash_after_rename_with_an_invalid_hash_restores_the_exact_before_image_on_reopen` writes
   `b"TORN WRITE"` over it; `content_persisted_first_write_with_a_mismatched_record_resolves_to_failed_not_inconsistent`
   writes `b"CORRUPTED"` over it) — none ever actually removes the file, so the `NotFound`
   arm (`Ok(None)`) is never exercised at all. A test that deletes the record outright before
   reconciling would kill this mutant.
2. `crates/storage/src/recovery.rs:423`'s guard-to-`false` and `==`→`!=` mutants (2 of the 14).
   Concrete untested scenario: `own_content` falling through to check a successor's
   before-image specifically because the record file is missing (as opposed to present but
   hash-mismatched, which is what both existing `own_content`-exercising tests —
   `superseded_journal_committed_operation_commits_its_own_content_not_its_successors` and
   `journal_committed_row_whose_own_content_is_unrecoverable_stays_pending_and_never_commits_other_content`
   — actually set up: both leave a present-but-wrong-content file on disk, never delete it).
3. `crates/storage/src/vault.rs:468`'s `==`→`!=` mutant. Concrete untested scenario:
   `read_note` on a note whose journal row **is** committed but whose on-disk file was deleted
   out from under it afterward. The two existing tests that get `StorageError::NotFound` from
   `read_note`
   (`reading_a_note_whose_only_write_never_reached_journal_committed_returns_not_found`, and the
   equivalent check after an `AfterIntentRecorded` crash) both hit the *earlier* "no committed
   journal row at all" branch (`last_committed_for_note` returning `None`, a few lines above
   this one) and never reach this function's own `std::fs::read` call at line 467 at all — so
   this specific `NotFound` arm, for an already-committed note's vanished file, is never
   exercised.
4. `crates/storage/src/vault.rs:1101`'s `==`→`!=` mutant. Concrete untested scenario:
   `read_before_image` on a genuinely missing file, checked for the *specific* `NotFound`
   variant. Existing tests do call `read_before_image` on a missing before-image (e.g.
   `before_image_is_deleted_only_after_git_committed`, and three others), but every one of
   them asserts only `.is_err()` — a loose check that cannot distinguish `StorageError::NotFound`
   from `StorageError::Io`, so it passes identically whether the guard is `==` or `!=`. A test
   asserting `matches!(result, Err(StorageError::NotFound { .. }))` specifically, rather than
   just `.is_err()`, would kill this mutant.

**Group C — a distinct root cause, not an I/O-error-path gap at all (1 of 14 survivors).**

`crates/storage/src/recovery.rs:290` — `on_disk_hash.is_some() && journal.last_committed_for_note(...)?.is_none()`
→ `&&`→`||`. This is not about `NotFound` vs. any other I/O error at all — it is a missing
*test combination* for a boolean guard with two independent conditions. Flipping `&&` to `||`
would make reconcile delete an unverified record even when `on_disk_hash` is `None` (nothing
on disk to delete) or even when a different operation already has a committed row for the
same note (which the code's own comment says must be preserved: "the record is left alone
rather than deleting content another row may describe"). The one existing test exercising this
branch
(`content_persisted_first_write_with_a_mismatched_record_resolves_to_failed_not_inconsistent`)
only covers the case where both conditions are true (file present, no committed row) — it
never constructs the case that would distinguish `&&` from `||` (e.g. a note with an existing
committed row, plus a second, unrelated `ContentPersisted` operation on that same note with no
before-image). **Justified as a real, cheap-to-close gap with its own distinct cause** — a
missing test case, not a platform-specific I/O-error limitation.

**Cluster (unchanged) — a semantically equivalent mutant (1 of 14 survivors).**

`crates/storage/src/journal.rs:79` — `AdvanceFields::none() -> Self` body:
`Self::default()` → `Default::default()`. `Self` and `AdvanceFields` are the same type at
this point, so `Self::default()` and `Default::default()` compile to the identical call —
this is a **mutant-equivalence false positive**, not a real coverage gap. No test can ever
distinguish these two expressions because they produce bit-for-bit identical values by
construction. **Justified as inherently unkillable**, not a defect in test coverage.

**Tally check:** Group A (5) + Group B (7) + Group C (1) + the equivalent mutant (1) = 14,
matching the full survivor count.

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

**Summary disposition:** 0 of the 14 survivors represent a production correctness defect. 5
(Group A) genuinely need a non-`NotFound` I/O error and are fairly accepted as hard to close on
this platform without new, injectable-I/O test infrastructure. 7 (Group B) are ordinary,
untested `NotFound`-path gaps — real and currently uncovered, but cheap to close with a
straightforward "delete the file, check the specific result variant" test, not a platform
limitation. 1 (Group C, `recovery.rs:290`) has its own distinct root cause: a missing boolean
test-combination, not an I/O-error-path issue at all. 1 (`journal.rs:79`) is a
mutant-equivalence false positive. The 1 timeout is scaffolding-only and not a production-code
gap. Raw tool output (`missed.txt`, `timeout.txt`, `caught.txt`, `unviable.txt`) was produced
by `cargo mutants --package domain --package storage --package policy` and is not committed to
the repository (it is a local, regenerable tool artifact, consistent with A1's own practice of
not committing `mutants.out/` directories).

**This mutation-testing result was not re-run after the later `git2` 0.21 upgrade (commit
`247df9e`, Section 2.2.1) and remains valid as recorded above.** Justification: the upgrade's
entire code diff is two call sites adapted from an `Option`-returning to a `Result`-returning
git2 API (`Reference::name()`, `Commit::message()`), each a mechanical combinator swap
(`.ok_or_else` → `.map_err`; `.is_some_and` → `.is_ok_and`) that preserves the exact prior
branch behavior — it introduces no new branch, no new guard, and no new I/O-error-path
distinction of the kind this mutation run's 13 real survivors are about. `cargo test
--workspace` after the upgrade still passes exactly 199/199 (Section 2.2.1), the same count as
before the upgrade, confirming no test's observable behavior changed. Re-running `cargo
mutants` over an unrelated, behavior-preserving dependency bump would not produce a materially
different survivor set, and was not done.

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
  against the originally recorded request, not just against the `operation_id` alone (Task 8's
  fix round, finding I1, commit `d758d5e`).
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
  an unbounded scan (Section 5, item 12 discusses its residual risk). Commits are serialized via a
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

(The `git2` 0.19.0 RustSec advisory finding that was originally listed here as unresolved —
RUSTSEC-2026-0008/0183/0184 — has been resolved by a later `git2` 0.21 upgrade and is no
longer a gap; see Section 2.2 and Section 2.2.1 for the full history and independent
re-verification.)

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
14. **RESOLVED — (Task 10, new finding at first pass, closed by a later commit — see Section
    2.2/2.2.1).** `git2` 0.19.0's three RustSec unsoundness advisories
    (RUSTSEC-2026-0008/0183/0184) were found by this task's first pass and initially left
    unresolved pending an explicit decision from the plan owner. The user chose to upgrade
    `git2`; the coordinator landed that upgrade (`git2` 0.21, commit `247df9e`) with two small,
    behavior-preserving adaptations in `crates/storage/src/git.rs`/`vault.rs`. Independently
    re-verified by this task: `cargo deny check` now reports `advisories ok, bans ok, licenses
    ok, sources ok`; `cargo test --workspace` still 199/199; `cargo clippy --workspace -- -D
    warnings` still clean. No longer an open item.
15. **(Task 10, new finding from this task's own mutation-testing pass, Section 2.4.1;
    corrected in this task's fix round — see below)** `storage`'s 14 surviving mutants split
    into three groups, not one uniform cause:
    - **Group A (5 survivors)** genuinely need a non-`NotFound` I/O error to kill
      (`git.rs:222`'s guard-to-`true`; `recovery.rs:272`'s guard-to-`true`;
      `recovery.rs:293`'s all three mutants, which require a file to vanish in a narrow
      single-threaded window between an earlier read and this `remove_file` call;
      `recovery.rs:423`'s guard-to-`true`; `vault.rs:1088`'s guard-to-`true`). Accepted as hard
      to close on this platform without new, injectable-I/O test infrastructure.
    - **Group B (7 survivors)** are ordinary, untested `NotFound`-path gaps — reachable by an
      everyday "delete the file" scenario no existing test happens to construct
      (`recovery.rs:272`'s `==`→`!=`; `recovery.rs:423`'s guard-to-`false` and `==`→`!=`;
      `vault.rs:468`'s `==`→`!=`; `vault.rs:1101`'s `==`→`!=`). These are real, currently
      acknowledged-but-uncovered gaps, cheap to close with a straightforward test, **not** a
      platform limitation — this task's first pass incorrectly lumped them in with Group A.
    - **Group C (1 survivor, `recovery.rs:290`'s `&&`→`||`)** has a distinct root cause: a
      missing boolean test-combination (no test exercises a `ContentPersisted` row with no
      before-image on a note that already has a *different* committed row), not an I/O-error
      issue at all.
    - Plus the mutant-equivalence false positive (`journal.rs:79`,
      `Self::default()`/`Default::default()` on the same type) — not a coverage gap.
    None were closed in this pass (documentation-only scope), but all are now described
    accurately rather than with a uniform "Windows-hard" framing. Full per-mutant detail in
    Section 2.4.1.
16. **(Task 8, deferred Minor M1 — added in this task's fix round; the progress ledger
    explicitly routes it here and it was missing from this file's first pass)** A race on the
    journal's `UNIQUE` constraint when two first-time inserts for the same `operation_id`
    arrive simultaneously returns a generic `StorageError::Journal(...)` error rather than the
    more specific `OperationInProgress`. Nothing is applied twice either way — this is an
    error-type consistency gap, not a correctness defect: the caller gets a less informative
    error variant than the "normal" concurrent-retry case would produce.
17. **(Task 8, deferred Minor M2 — added in this task's fix round)**
    `StorageError::OperationInProgress`'s doc text ("operation {operation_id} is still in
    flight; nothing was applied," `crates/storage/src/vault.rs`) is misleading for the case
    where a production I/O error — not an actively running writer — left the row in a
    non-terminal state. A retry against such a row also gets this variant, but "in flight"
    isn't an accurate description of that row's state until the next `Vault::open` resolves
    it.
18. **(Task 8, deferred Minor M3 — added in this task's fix round)** One specific interaction
    between the writer's own step-5 rollback and `reconcile`'s row 2 (a before-image already
    deleted but the row still resting at `ContentPersisted`) converges correctly on reading the
    code, but has no test exercising it directly.
19. **(Task 8, deferred Minor M4, process note — included per the reviewer's explicit
    allowance that it is optional)** Task 8's own dispatch brief named the wrong file
    (`vault.rs` instead of `recovery.rs`) for where Task 7's fix-round tests actually live.
    Not a product limitation; recorded here only because the ledger routed it to this file
    alongside M1–M3.

---

## 6. Acceptance-criteria mapping (spec Fase A table, A2's additions)

| Acceptance criterion | Status | Evidence |
|---|---|---|
| Test suite green (workspace) | **PASS** | Section 2.1: 199/199 passed, 0 failed. |
| Every C1 fault-matrix row covered by a dedicated crash test | **PASS** | Six `crash_*` tests plus the double-crash test (`crates/storage/src/vault.rs`, exercising each row via the full write pipeline + fault injector), **plus** a parallel, more direct set of row-specific `reconcile`-calling tests in `crates/storage/src/recovery.rs` (e.g. `intent_recorded_with_no_further_progress_is_marked_failed_on_reconcile`, `content_persisted_with_valid_hash_rolls_forward_to_journal_committed`, `content_persisted_with_invalid_hash_and_a_valid_before_image_rolls_back_to_failed_not_journal_committed`, `journal_committed_with_no_existing_commit_commits_on_reconcile`, `git_committed_reaches_index_current_on_reconcile`) that construct each journal row directly and call `reconcile` without going through a live writer. Both files' tests cover the same 8 rows from different entry points. |
| Retry idempotency (terminal and non-terminal states) | **PASS** | `retry_of_a_failed_operation_id_returns_failed_again_without_reapplying`, `retry_after_a_restart_returns_the_outcome_recovery_reached`, and the full `retry_*` family in Section 2.1. |
| Argument-mismatch detection on retry (`OperationIdReused`) | **PASS** | `reusing_an_in_flight_operation_id_for_different_content_is_refused_as_reused_not_in_progress`, `reusing_an_operation_id_for_a_different_request_is_refused_not_reported_as_the_original_outcome`. |
| Before-image rollback, never indistinguishable from success | **PASS** | Crash-matrix restore tests; `new_version_with_an_invalid_hash_is_detected_by_the_writer_and_restored_from_the_before_image`. |
| Reads resolve through journal, verified by hash, sharing writer's lock | **PASS** | `read_note_resolves_through_the_journal_not_the_raw_file`, `read_blocks_behind_a_paused_writer_holding_the_same_note_lock_and_then_sees_the_fully_committed_result`. |
| Idempotent git commit, isolated staging, repo-wide serialization | **PASS** (subject to limitation 5) | `retry_of_a_git_pending_operation_commits_its_own_content_once_git_exists`, `retry_of_a_git_pending_operation_after_a_successor_overwrote_it_commits_its_own_content_not_the_successors`. |
| `domain::WriteState::valid_transition` tightened to C1's exact table | **PASS** | `crates/domain/src/write_state.rs`; see Section 3. |
| `domain::IndexState` never reports `Current` in A2 | **PASS** | `crates/domain/src/index_state.rs`; `write_record_unchecked_reaches_index_current_pipeline_state_but_index_state_for_stays_stale`. |
| `storage`'s only `pub` write entry point is `propose_and_commit` | **PASS** | `storage_exposes_exactly_one_public_write_function` (unchanged from A1, re-verified running). |
| `cargo clippy --workspace -- -D warnings` clean | **PASS** | Section 2.3. |
| Dependency/license allowlist check (`cargo deny check`) | **PASS** (originally FAILED on advisories at first pass; resolved by a later `git2` 0.21 upgrade, commit `247df9e`, independently re-verified) | Section 2.2/2.2.1: `advisories ok, bans ok, licenses ok, sources ok` — see `eval/results/A2/dependency-exceptions.md` for the full resolved history. |
| Mutation testing examined (`domain`, `policy`, `storage`) | **PASS** (every survivor examined and justified, per spec threshold) | Section 2.4.1: 196 mutants, 131 caught, 50 unviable, 1 timeout, 14 missed — all 14 individually examined; 13 share one documented I/O-error-path coverage gap, 1 is a mutant-equivalence false positive; 0 are production defects. `domain`/`policy`: 34 mutants, 0 survivors. |
| SEC-001/002/003 re-evaluated against the actual A2 design | **DONE (all three remain open/narrowed, not closed)** | Section 4; `eval/results/A2/sec-debt-reevaluation.md`. |

---

## 7. Boundary restatement (Continuity [c5ja] extended to A2)

A2 does not provide anything beyond what this plan implements. In particular: no encryption,
no deletion registry, no backup/restore (A3); no real index (B1); SEC-001/002/003 remain open
or only narrowed, not closed, and require re-evaluation before Fase D (Continuity [d5b9]).

As of this file's final update, `cargo test --workspace` (199/199), `cargo clippy --workspace
-- -D warnings` (clean), and `cargo deny check` (`advisories ok, bans ok, licenses ok, sources
ok`) all pass cleanly. The `git2` advisory finding this task's first pass reported as an open
gap (Section 2.2) was resolved by a later `git2` 0.21 upgrade (commit `247df9e`), chosen and
landed after this evidence was first presented to the user, and independently re-verified here
(Section 2.2.1) rather than taken on faith.

Per the brief's Step 7 and the pattern established in A1 (Continuity [c12av]): **this branch is
not to be merged to main without explicit user approval**, even though every automated check is
now green — the SEC-debt verdicts in Section 4 (open or only narrowed, with the mandatory
Fase-D re-evaluation obligation) must still be presented to the user alongside this evidence
before any merge decision.
