# A2 dependency exceptions

Recorded per the project spec: every exception is justified, time-bound, and recorded here.
Mirrors `eval/results/A1/dependency-exceptions.md`'s format and discipline.

## Licensing: no new exception needed

A2 adds three direct dependencies to `crates/storage/Cargo.toml`: `rusqlite` 0.31.0
(`features = ["bundled"]`), `git2` (originally 0.19.0, as pinned by the plan; upgraded to
0.21.0 later in this task's fix round — see "Advisories" below — still licensed MIT OR
Apache-2.0, re-confirmed by `cargo deny check licenses` after the upgrade), and `sha2` 0.10.9
(promoted from an existing transitive dependency via `policy`'s secret hashing to a direct
`storage` dependency, per the plan's Tech Stack section).

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

## Advisories: one finding, found OPEN by this task, RESOLVED by a later commit

`cargo deny check advisories`, run as part of this task's first pass, **FAILED**. Three
RustSec advisories were found against `git2` 0.19.0, the version pinned by
`crates/storage/Cargo.toml` at that time (`git2 = "0.19"`). **This finding is now resolved** —
see "Resolution" at the end of this section — but the original finding and the reasoning
behind each option considered are preserved below as the acceptance-evidence record of how it
was found and closed, not because it is still open.

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
- **Status:** **RESOLVED.** Found open at this task's first pass; fixed by the `git2` 0.21
  upgrade described in "Resolution" below.

### Finding: RUSTSEC-2026-0183

- **Advisory:** Potential undefined behavior calling `Remote::list()` when the remote
  advertises no references — `git2` passes a null pointer to `slice::from_raw_parts`.
- **Affected crate:** `git2` 0.19.0.
- **Dependency chain:** same as above.
- **Fixed upstream in:** `git2 >= 0.21.0`.
- **Applicability to this project:** `crates/storage/src/git.rs` never calls
  `Remote::list()` — this project has no remote-related git functionality at all (A2 commits
  only to the local repository; no push/fetch/remote operations exist anywhere in `storage`).
- **Status:** **RESOLVED.** Found open at this task's first pass; fixed by the `git2` 0.21
  upgrade described in "Resolution" below.

### Finding: RUSTSEC-2026-0184

- **Advisory:** Potential undefined behavior constructing a `Signature` from a
  buffer-created `BlameHunk` whose author/committer pointers may be null.
- **Affected crate:** `git2` 0.19.0.
- **Dependency chain:** same as above.
- **Fixed upstream in:** `git2 >= 0.21.0`.
- **Applicability to this project:** `crates/storage/src/git.rs` never uses `Blame` or
  `BlameHunk` — this project has no blame/history-attribution functionality.
- **Status:** **RESOLVED.** Found open at this task's first pass; fixed by the `git2` 0.21
  upgrade described in "Resolution" below.

### Two remediation paths were considered at this task's first pass; this task performed neither itself

1. **Upgrade `git2` to `>= 0.21`.** Requires changing the version constraint in
   `crates/storage/Cargo.toml` (at the time, `git2 = "0.19"`, and `cargo update -p git2
   --dry-run` confirmed no compatible newer version existed under that constraint — the fix
   versions were a minor/major bump away). This is a production dependency-manifest change.
   Task 10's brief is explicit that this documentation task makes **no code changes** itself;
   a version bump, even without touching any `.rs` file by this task's own hand, is a change to
   a committed production artifact that this task was not chartered to make or review.
2. **Add a time-bound `deny.toml` exception**, following the exact pattern of the existing
   `RUSTSEC-2024-0449` entry, backed by the applicability analysis above. A draft of this
   exception was prepared during this task's first pass but the `Edit` to `deny.toml` was
   **blocked by the coding agent's own permission system**, which classified adding
   advisory-ignore entries to a security-relevant configuration file as a "Security Weaken"
   action requiring explicit human approval before proceeding.

### Resolution: the user chose to upgrade `git2`; no `deny.toml` exception was ultimately needed

This finding, along with both options above, was presented to the user as part of this task's
`evidence.md`. **The user chose option 1** — upgrade `git2` — rather than granting an
exception or leaving the check red. The coordinator made that upgrade directly, in a separate
commit **`247df9e35d23eeec58003442a1b18bb91dc6a0e7`**
(`fix(storage): upgrade git2 to 0.21 to resolve three unsound advisories`), landed on top of
this task's own evidence commit:

- `crates/storage/Cargo.toml`: `git2 = "0.19"` → `git2 = "0.21"`.
- `crates/storage/src/git.rs` and `crates/storage/src/vault.rs`: two call sites adapted for
  `git2 0.21`'s API change (`Reference::name()` and `Commit::message()` changed from
  `Option`-returning to `Result`-returning, as part of the same unsound-pointer-dereference fix
  the three advisories describe) — `.ok_or_else(...)` → `.map_err(|_| ...)` and
  `.is_some_and(...)` → `.is_ok_and(...)`, both verified to preserve the exact prior behavior
  (a missing/non-UTF8 name or message is still treated as "doesn't match," never newly
  propagated as a hard error).
- `Cargo.lock`: the transitive dependency tree shrank (net 318 lines removed in the diff)
  because `git2`/`libgit2-sys` 0.21 drops the `openssl-sys`/`url`/ICU chain that only the old
  network-transport features needed — this project's git usage is local-filesystem-only, so
  that chain was unused dead weight the upgrade incidentally sheds.
- `deny.toml`: **unchanged** — confirmed via `git diff 9771c1e..247df9e -- deny.toml`, which is
  empty. No exception of any kind was ultimately added to it, through either this task's first
  pass or the later upgrade commit.

**Independently re-verified by this task** after the upgrade landed (not taken on faith from
the coordinator's own report):

```
cargo deny check   → advisories ok, bans ok, licenses ok, sources ok
cargo test --workspace   → 199 passed, 0 failed (same count as before the upgrade)
cargo clippy --workspace -- -D warnings   → clean
```

**Net result:** `cargo deny check` passes cleanly. No `deny.toml` or `dependency-exceptions.md`
exception entry exists for `git2`, and none was needed — the version upgrade resolved the
underlying advisories rather than suppressing them.

**Recorded in:** `eval/results/A2/evidence.md` Section 2.2 and Section 2.2.1.
