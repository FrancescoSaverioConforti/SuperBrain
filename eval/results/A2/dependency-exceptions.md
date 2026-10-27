# A2 dependency exceptions

Recorded per the project spec: every exception is justified, time-bound, and recorded here.
Mirrors `eval/results/A1/dependency-exceptions.md`'s format and discipline.

## Licensing: no new exception needed

A2 adds three direct dependencies to `crates/storage/Cargo.toml`: `rusqlite` 0.31.0
(`features = ["bundled"]`), `git2` 0.19.0, and `sha2` 0.10.9 (promoted from an existing
transitive dependency via `policy`'s secret hashing to a direct `storage` dependency, per the
plan's Tech Stack section).

`cargo deny check licenses`, run as part of Task 10's closing verification
(`eval/results/A2/evidence.md`, Section 2.2), returned **`licenses ok`** with zero violations
against the project's existing `deny.toml` allowlist (`Apache-2.0`, `MIT`, `BSD-2-Clause`,
`BSD-3-Clause`, `ISC`, `Zlib`, `Unicode-3.0`) for all three new dependencies and every one of
their transitive dependencies. This was verified against `cargo deny`'s actual crates.io
metadata at check time, not against the plan's own stated assumption that these three crates
are "MIT or MIT/Apache-2.0 dual-licensed" — the plan's assumption held, and `cargo deny` is
recorded here as the authority that confirmed it, per the brief's explicit instruction.

**No `deny.toml` or `dependency-exceptions.md` entry was required for licensing.** The only
`deny.toml` entry pre-dating A2 (`RUSTSEC-2024-0449`, `unsafe-libyaml`) is unrelated to A2's new
dependencies and is unchanged; see `eval/results/A1/dependency-exceptions.md`.

## Advisories: one new finding, left OPEN — not resolved by this task

`cargo deny check advisories`, run in the same pass, **FAILED**. Three RustSec advisories were
found against `git2` 0.19.0, the exact version pinned by `crates/storage/Cargo.toml`
(`git2 = "0.19"`):

### Finding: RUSTSEC-2026-0008

- **Advisory:** Potential undefined behavior when dereferencing a `Buf` struct immediately
  after calling `Buf::new()` or `Buf::default()` — a null pointer is passed to the unsafe
  `slice::from_raw_parts`, which requires a non-null, aligned pointer even for zero-length
  slices.
- **Affected crate:** `git2` 0.19.0.
- **Dependency chain:** `superbrain` crates → `storage` → `git2` 0.19.0. Declared in
  `crates/storage/Cargo.toml`.
- **Fixed upstream in:** `git2 >= 0.20.4`.
- **Applicability to this project:** `crates/storage/src/git.rs` (the sole consumer of `git2`
  in this workspace) was checked against this advisory's trigger path. It never directly
  constructs or dereferences a `git2::Buf`. This project's own code does not take the trigger
  path, though this does not rule out the advisory's root cause being reachable through some
  other `git2`/`libgit2` internal call this project does make.
- **Status:** **OPEN, not exempted, not fixed.** See "Why this was not resolved" below.

### Finding: RUSTSEC-2026-0183

- **Advisory:** Potential undefined behavior calling `Remote::list()` when the remote
  advertises no references — `git2` passes a null pointer to `slice::from_raw_parts`.
- **Affected crate:** `git2` 0.19.0.
- **Dependency chain:** same as above.
- **Fixed upstream in:** `git2 >= 0.21.0`.
- **Applicability to this project:** `crates/storage/src/git.rs` never calls
  `Remote::list()` — this project has no remote-related git functionality at all (A2 commits
  only to the local repository; no push/fetch/remote operations exist anywhere in `storage`).
- **Status:** **OPEN, not exempted, not fixed.**

### Finding: RUSTSEC-2026-0184

- **Advisory:** Potential undefined behavior constructing a `Signature` from a
  buffer-created `BlameHunk` whose author/committer pointers may be null.
- **Affected crate:** `git2` 0.19.0.
- **Dependency chain:** same as above.
- **Fixed upstream in:** `git2 >= 0.21.0`.
- **Applicability to this project:** `crates/storage/src/git.rs` never uses `Blame` or
  `BlameHunk` — this project has no blame/history-attribution functionality.
- **Status:** **OPEN, not exempted, not fixed.**

### Why this was not resolved

Two remediation paths exist, and this task performed neither:

1. **Upgrade `git2` to `>= 0.21`.** This requires changing the version constraint in
   `crates/storage/Cargo.toml` (currently `git2 = "0.19"`, which `cargo update -p git2
   --dry-run` confirms has no compatible newer version available under that constraint — the
   fix versions are a minor/major bump away). This is a production dependency-manifest change.
   Task 10's brief is explicit that this task makes **no code changes** ("this is a
   documentation/evidence task"); a version bump, even without touching any `.rs` file, is a
   change to a committed production artifact (`Cargo.toml`/`Cargo.lock`) that could alter
   `git2`'s behavior or API surface in ways this task is not chartered to review or test.
2. **Add a time-bound `deny.toml` exception**, following the exact pattern of the existing
   `RUSTSEC-2024-0449` entry, backed by the applicability analysis above (none of the three
   advisories' trigger APIs are called by this project's code). A draft of this exception was
   prepared during this task but the `Edit` to `deny.toml` was **blocked by the coding agent's
   own permission system**, which classified adding advisory-ignore entries to a
   security-relevant configuration file as a "Security Weaken" action requiring explicit human
   approval before proceeding. This task respected that block rather than finding a workaround,
   since granting an exception to unsoundness advisories is exactly the kind of decision that
   should require explicit, visible human sign-off rather than being made unilaterally by an
   agent — even one executing a brief that anticipated exceptions might be needed.

**Net result:** `cargo deny check` is left genuinely failing (`advisories FAILED`) at A2's
close. This is recorded as an open item for the plan owner / user to resolve before merge, by
explicitly choosing one of the two paths above (or a third option, such as pinning to a patched
`git2` fork or vendoring a fix) — not by this task silently picking one.

**Recorded in:** `eval/results/A2/evidence.md` Section 2.2 and Section 4 ("What A2 does NOT
guarantee"). `deny.toml` itself is **unchanged** by this task — confirmed via
`git status --short deny.toml` showing no diff at the time this file was written.

**Re-review:** whichever path the plan owner chooses, it should happen before this branch
merges to `main`, since `cargo deny check` is part of this project's standing acceptance
criteria and is not currently green.
