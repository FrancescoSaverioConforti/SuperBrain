# SEC-001/002/003 re-evaluation against the A2 journal design

**Task:** A2 Task 9. **Purpose:** re-evaluate three previously-accepted security
debts from A1 — SEC-001 (TOCTOU/symlink race), SEC-002 (intra-vault record
aliasing via symlink), SEC-003 (hard-link aliasing) — against the
journal/hash-verification machinery A2 just built, via adversarial tests added
in `crates/storage/src/vault.rs` (`vault::tests::sec_001_*`, `sec_002_*`,
`sec_003_*`). This task does **not** attempt to close any of these debts; it
only measures what A2's new code actually does when the original exploit
scenarios are run against it.

**Mandatory restatement (per the task's Interfaces spec):** whatever remains
open below requires re-evaluation again before Fase D (Continuity [d5b9]).

**Important note on method:** this task's brief supplied exact test code and
*predicted* verdicts for each debt (all three predicted `OPEN, UNCHANGED`).
Running that code verbatim against the actual A2 vault surfaced concrete,
reproducible divergences from those predictions in all three cases. Per this
task's explicit instructions, those divergences are reported here as findings,
not silently corrected to match the prediction. An independent review of the
first pass of this evaluation (see "Fix round" note in the task report) found
that even the corrected version still understated the live attack surface in
two specific ways — this file has been revised to cover both. The adversarial
tests committed assert the *measured* behavior, each with an inline comment
explaining any deviation from the brief's literal assertion and why.

---

## SEC-001 — TOCTOU / symlink race on the write path (Continuity [d5b9])

**Original exploit scenario (A1):** a check-then-use race in
`record_path_checked`: the containment check canonicalizes the record path and
judges it safe, but nothing is atomic between that check and the later I/O
(`std::fs::read`, or `atomic_write`'s create/rename). A concurrent process that
swaps the path for a symlink *after* the check but *before* the write can, in
principle, cause the write to land bytes somewhere the check never approved.

The race window covers **two separate pieces of I/O** inside
`write_record_unchecked`, both reached through the exact same checked-but-now-
stale `record_path`:

1. **Step 2's version-check read** (`std::fs::read(&record_path)`, run after
   the per-note lock is acquired) — reads whatever is actually at the path
   right now, to determine the current version. `std::fs::read` **follows**
   a symlink.
2. **`atomic_write`'s rename** (`crates/storage/src/atomic.rs`) — writes a
   temp file, then calls `std::fs::rename(&tmp_path, record_path)`. A rename
   whose destination is a symlink **replaces the symlink itself** rather
   than following it to its target — true under both Windows and POSIX
   `rename(2)` semantics, not a platform-specific quirk.

Because these two have opposite "follow vs. replace" behavior, the race
produces **two distinct, independently-demonstrated consequences** depending
on what the attacker's swapped-in symlink points at.

### Test 1 — write-side: `vault::tests::sec_001_toctou_window_between_path_check_and_write_still_exists`

Symlink target: `outside/victim`, a path that **does not exist**. Mechanism:
a new fault point, `FaultPoint::AfterPathCheckedBeforeAtomicWrite` (added to
`crates/storage/src/fault.rs`), is checked in
`write_record_unchecked_with_injector` immediately after
`record_path_checked` returns and before the per-note lock / step 2 / write.
The test arms `FaultInjector::pause_at` on that point, starts a writer thread,
waits for the pause signal, deletes the checked-safe `note-a.record` path and
replaces it with a symlink pointing at the nonexistent `outside/victim`, then
resumes the writer.

**Actual result (measured):**

```
write_result = Ok(Version(1))
link_is_symlink (after) = false
link_contents (after) = "1\n---\ncontent"
outside directory listing = []
```

The write **succeeds** despite the mid-flight swap. Because the symlink's
target does not exist, step 2's read sees `NotFound` and falls back to
`Version(0)` (the normal "note doesn't exist yet" path) — the race is
invisible to the version check in this specific setup. `atomic_write`'s
rename then replaces the symlink (not the nonexistent target) with the
vault's own record. `outside/victim` is never created.

**Verdict contribution:** the race window is confirmed still open (the write
proceeds through a path an attacker swapped after the check approved it,
violating the "write what you checked" invariant), but *this specific test's*
consequence is destruction of the attacker's symlink, not exfiltration — a
narrower consequence than the brief's literal prediction
(`outside/victim.exists()`), because it only exercises `atomic_write`'s
rename-replaces-symlink behavior and happens to use a target that doesn't
exist, so step 2 never has anything real to read through the link.

### Test 2 — read-side: `vault::tests::sec_001_toctou_read_side_leaks_an_outside_records_version_and_bytes_into_the_before_image`

**Independent-review finding I-1:** Test 1 alone is incomplete evidence for
SEC-001, because it only exercises the write side (the rename) and does so
against a target that doesn't exist, so it can't show what happens when the
race's OTHER I/O point — step 2's read, which DOES follow a symlink — hits a
real, readable file. This test closes that gap: the symlink target is now a
genuine, parseable, readable record (`outside/victim.record`, content
`"OUTSIDE CONTENT"`, `Version(7)`), and the writer's `expected_version` is set
to `Version(7)` to match it, so step 2's read-through-the-symlink succeeds
and the pipeline proceeds rather than failing on a version mismatch.

(The root for this test is deliberately **not** git-initialized:
`write_record_unchecked` deletes a note's before-image once `GitCommitted` is
reached, and this test needs to inspect the before-image the write produced,
so the write must stop at `JournalCommitted`/`GitPending` rather than running
to completion and cleaning up after itself.)

**Actual result (measured):**

```
write_result = Err(StorageError::GitPending { .. })   // expected: non-git root
before_image_bytes (note-a, this operation_id) == serialize_record("OUTSIDE CONTENT", Version(7))
```

Step 2 read `Version(7)` through the attacker's symlink, matched
`expected_version`, and the write proceeded to `persist_before_image` —
which copied the OUTSIDE file's own bytes into the vault's **own**
before-image store, filed under this note's id and this operation's id. This
is a genuine, concrete, if narrow, exfiltration-ADJACENT consequence that
Test 1 cannot show: external content pulled **into** the vault's own
storage. (The symmetric consequence — a version **mismatch** surfacing
`Conflict` with an attacker-chosen "actual" version, usable as a denial-of-
service lever against legitimate writers racing to use the real version — is
the ordinary `Conflict` path already covered elsewhere in this module by
`stale_expected_version_is_rejected_with_explicit_conflict`; the only new
fact here is *where* the "actual" version came from, which needed no
separate demonstration.)

### SEC-001 verdict: `OPEN, UNCHANGED`

Both tests confirm the race window itself is completely untouched by A2: step
2's read and `atomic_write`'s rename are the same two operations A1 identified,
reached through the same unprotected `record_path`. The journal's hash
verification (step 5 of `write_record_unchecked`) runs *after* the write,
over whatever bytes are now at the resolved path — it has no bearing on what
path resolution the race already completed, and no bearing on what step 2
already read before that. A2 neither closes the race nor meaningfully
narrows it; it adds no check anywhere in this window. The two concrete
consequences measured — (a) the attacker's symlink is silently replaced
rather than written through, and (b) an outside file's version and bytes can
be pulled into the vault's own before-image store via step 2 — are both still
fully live. Closing this properly still requires handle-relative
(`openat`/`O_NOFOLLOW`-style) I/O for both step 2's read and the write, out of
scope for A2 per the plan's Global Constraints.

---

## SEC-002 — intra-vault record aliasing via symlink (Continuity [c16c6])

**Original exploit scenario (A1):** a symlink at one note's record path
pointing at another note's record path, both inside the vault root (so
SEC-001's containment check, which only rejects paths resolving *outside* the
root, does not reject it). Reading or writing through the aliased note id
transparently operates on the other note's bytes.

**Test:** `vault::tests::sec_002_read_path_rejects_both_aliasing_shapes_but_write_path_pre_read_does_not`
(renamed from `sec_002_journal_hash_check_does_not_detect_aliasing_present_before_the_write`
— independent-review finding M-2: the old name asserted the opposite of what
Scenario A below actually proves). Three scenarios, two on the read path, one
on the write path.

### Scenario A (read path) — alias a note id that was never itself written

`note-b` is written for real (its own committed journal row and hash exist).
`note-a.record` is then symlinked to `note-b.record` *before* `note-a` has
ever been written. `read_note(note-a)` is called directly, as the brief's
verbatim code specified.

**Brief's prediction:** `read_note(note-a)` succeeds and returns note-b's
content — the alias is live and undetected.

**Actual result (measured):** `read_note(note-a)` returns
`Err(StorageError::NotFound { note_id: "note-a" })`.

**This diverges from the brief's prediction.** `read_note_locked`
(`crates/storage/src/vault.rs`) resolves a note *exclusively* through
`Journal::last_committed_for_note(note_id)` — keyed strictly by the **logical
note id being read**, never by "whatever bytes live at the resolved path."
No journal row exists under `"note-a"`, so the journal lookup fails before
the read ever opens the aliased file. A2's journal-mediated read
architecture — not its hash verification — accidentally narrows exactly this
read-side shape.

### Scenario B (read path) — alias a note id that already has its own committed history

`note-a-v2` is written for real first (its own committed row, its own
content-hash, over its own original content). Its file is then removed and
replaced with a symlink to `note-b.record` (different content, different
hash). `read_note(note-a-v2)` is called.

**Actual result (measured):** `Err(StorageError::Inconsistent { note_id: "note-a-v2" })`.

Caught by `read_note`'s unconditional read-time hash check: the resolved
row's `content_hash` (from note-a-v2's own original write) does not match the
swapped-in content's hash.

### Scenario C (write path) — **independent-review finding I-2**: alias a never-written note id, then write through it

**Scenarios A and B are both scoped to the READ path (`read_note`) only.**
`write_record_unchecked`'s own step 2 — the pre-write version-check read — is
a *completely separate* code path from `read_note_locked`'s journal-first
resolution: step 2 calls `std::fs::read(&record_path)` directly, with no
per-note-id journal lookup at all, and falls back to `Version(0)` only on
`NotFound`. Aliasing a never-written note id to an already-committed
*different* note's file, then **writing** through the aliased id, needs no
race, no timing, nothing beyond an ordinary in-root symlink
(`record_path_checked` explicitly permits symlinks that resolve inside the
root — see `record_symlink_resolving_inside_the_vault_root_is_not_an_escape`
elsewhere in this module):

`note-c.record` is symlinked to `note-b.record` (note-b is already committed
at `Version(1)`, content `"note-b real content"`) before `note-c` has ever
been written. `vault.write_record_unchecked(&id_c, "note-c's new content",
Version(1), op_c)` is then called — `Version(1)` matches note-b's *actual*
version, read through the alias.

**Actual result (measured, interrupted at `AfterRenameBeforeJournalCommitted`
so the before-image can be inspected before the later git-commit step would
delete it):**

```
write_through_alias = Err(StorageError::Interrupted(AfterRenameBeforeJournalCommitted))  // by design, to inspect state
before_image_bytes (note-c, this operation_id) == serialize_record("note-b real content", Version(1))
read_note(note-b).content == "note-b real content"   // note-b itself untouched
```

The write **succeeds** using note-b's own version as note-c's version-check
baseline, and step 2's read persists note-b's own current content as
note-c's before-image — a cross-note content leak through the write path,
completely ungated by anything that protects `read_note`. The symlink at
`note-c.record` is then replaced (not written through) by the final rename,
exactly as SEC-001's write-side test showed, so note-b's own file is left
untouched — the leak is one-directional (note-b's content into note-c's
before-image store), not a mutation of note-b.

### SEC-002 verdict: `NARROWED (read path only); write-path pre-read is unchanged and open`

Two read-side sub-cases are caught today, both by machinery that was not
built to detect aliasing as such:

- A note id with no prior write of its own, aliased from the start: rejected
  by the per-note-id journal lookup (Scenario A) — an architectural
  coincidence of A2's read design, not an anti-aliasing check.
- A note id with its own prior committed write, later aliased to *different*
  content: rejected by the unconditional read-time hash check (Scenario B) —
  a general integrity check, not an anti-aliasing check.

Neither mechanism inspects the resolved path's link structure, and both have
real limits even within their own scope:

- The hash check is a **fail-closed integrity gate, not a security
  boundary**: a successful alias just becomes a denial of service
  (`Inconsistent`), not a prevented or cryptographically disproven attack —
  and it offers **no** defense against an attacker who can also write to the
  journal directly (the resolved row's `content_hash` is a column such an
  attacker could simply set to match whatever bytes they substitute).
- Aliasing to content that happens to **hash-identically** to what the row
  expects passes the hash check completely undetected — the check cannot
  distinguish "the original file" from "a different file with the same
  bytes," by construction.

And the **write path is a distinct, unprotected surface**: Scenario C shows
that `write_record_unchecked`'s step 2 pre-read follows an in-root symlink
exactly like SEC-001's step 2 does, with no journal-identity gating at all,
and produces a real cross-note content leak into the vault's own before-image
store with no race required. This is not narrowed by anything A2 added — it
is fully open, using the ordinary write path, every time.

The underlying structural gap — `record_path_checked` accepts any symlink
target that resolves inside the root, with no check that the resolved file
is the one record it was asked for (no inode/link check, no "this symlink
points at a *different* note's record" rejection) — is completely unchanged
on both the read and write sides.

---

## SEC-003 — hard-link aliasing (Continuity [c17e8])

**Original exploit scenario (A1):** a hard link at a note's record path to a
file outside the vault (or to another note's record inside it).
Canonicalization cannot see a hard link at all — it is a second directory
entry to the same inode, not a redirect — so the containment check cannot
reject it by construction, regardless of any fix applied to symlink handling.
SEC-003 has two directions, both out of canonicalization's reach:

- **Inward:** a hard link planted *inside* the vault, at a note's record
  path, pointing at content outside the vault (or at another note's file) —
  the vault reads through it.
- **Outward:** a hard link created *outside* the vault, pointing at the same
  inode as one of the vault's own record files — an external reader reads
  the vault's content with **no vault code involved at all**.

**Test:** `vault::tests::sec_003_hard_link_aliasing_is_invisible_to_canonicalization_after_a2`.
Covers the inward direction with the same two read-path scenarios as
SEC-002, using `std::fs::hard_link` instead of a symlink.

### Scenario A (read path, inward) — hard-link a note id that was never itself written

`victim.record` (outside the vault, content `"OUTSIDE"`, `Version(7)`) is
hard-linked to `note-a.record` before `note-a` has ever been written.
`read_note(note-a)` is called directly, as the brief's verbatim code
specified.

**Actual result (measured):** `Err(StorageError::NotFound { note_id: "note-a" })`
— identical reasoning to SEC-002's Scenario A: the journal lookup fails
before canonicalization's blind spot is ever reached.

### Scenario B (read path, inward) — hard-link over a note id that already has its own committed history

`note-a-v2` is written for real first; its file is then replaced with a hard
link to `victim.record`. `read_note(note-a-v2)` is called.

**Actual result (measured):** `Err(StorageError::Inconsistent { note_id:
"note-a-v2" })` — identical mechanism to SEC-002's Scenario B.

**Write-path scenario, inward:** not re-demonstrated with a separate test.
`write_record_unchecked`'s step 2 reads `record_path` with `std::fs::read`
regardless of whether the entry there is a plain file, a symlink, or a hard
link — the code path is identical either way, so SEC-002's Scenario C (which
demonstrates the leak via a symlink) establishes the mechanism; a hard link
at the same spot would behave identically, since by the time `std::fs::read`
opens the path, a hard link and a symlink-to-that-target are indistinguishable
to that call. This is noted here rather than duplicated as a full second test
(per this fix round's guidance), since the underlying code path being
exercised is the same one SEC-002's test already covers directly.

### Outward direction — **independent-review finding I-2, point 3** — completely untouched, not a vault code path at all

A hard link created **outside** the vault root, pointing at the same inode as
one of the vault's own `.record` files, lets any reader with filesystem
access to that outside location read the vault's content directly — no call
into `Vault`, no `record_path_checked`, no journal lookup, no hash check, is
ever involved, because nothing in the vault's own code runs when a *different*
directory entry to the same inode is opened. This direction is not
demonstrated with a new test here, deliberately: it is a fact about how hard
links work at the filesystem/OS level, not a code path this project owns or
could instrument via a `FaultPoint` or a `Vault` method call. It is also
**completely outside A2's scope and unaffected by anything A2 built** — the
journal, the hash check, and the per-note-id read gating all live inside
`Vault`'s own call surface, and none of them execute when the vault's data is
read by a process that never calls into `Vault` at all. A2 provides exactly
zero narrowing here, and nothing short of eliminating hard-link-creatable
inodes for record files (e.g. one file per inode, enforced some other way)
would touch it — well outside this task's and this plan's scope.

### SEC-003 verdict: `NARROWED (read path only); write-path pre-read and outward hard-link exposure unchanged`

Exactly the same two read-side sub-cases as SEC-002, caught by exactly the
same two incidental mechanisms, with the same limits (fail-closed-not-secure
hash check; identical-bytes aliasing passes undetected). The inward write
path is open via the same mechanism SEC-002's Scenario C demonstrates
directly (not re-tested here since the code path is identical). The outward
direction is not addressed by A2 at all, is not a `Vault` code path, and is
not expected to ever be addressed by anything inside this crate's read/write
surface — closing it would require a different architecture entirely (for
example, content-addressed storage where record files are never the sole
location of truth, or periodic inode/link-count auditing), out of scope here.

---

## Summary

| Debt | Original scenario | Measured against A2 | Verdict |
|------|-------------------|----------------------|---------|
| SEC-001 | TOCTOU race between path-check and write | Write-side: race still proceeds through a swapped path; `atomic_write`'s `rename` replaces a destination symlink rather than following it (true on Windows and POSIX alike), so bytes don't land outside the root via THIS test's nonexistent-target setup. Read-side: step 2's pre-write read DOES follow a symlink to a real file, pulling that file's version and bytes into the vault's own before-image store. | `OPEN, UNCHANGED` |
| SEC-002 | Symlink-alias one note's record to another's | Read path: a never-written alias is rejected `NotFound` (per-note-id journal lookup); an already-committed note swapped afterward is rejected `Inconsistent` (unconditional hash check) — both incidental, both with real limits (DoS-only protection; identical-bytes aliasing undetected). Write path: a never-written alias, written through, succeeds and leaks the aliased note's content into the attacker's note's before-image — completely open, no race needed. | `NARROWED (read path only); write-path pre-read unchanged and open` |
| SEC-003 | Hard-link-alias a note's record to outside/other content | Inward, read path: identical two sub-cases and mechanisms as SEC-002. Inward, write path: open via the same step-2 mechanism SEC-002's write-path test demonstrates (identical code path, not duplicated here). Outward (hard link created outside the vault, exposing vault content with no vault code involved): completely untouched, not a `Vault` code path, out of A2's and this plan's scope entirely. | `NARROWED (read path only); write-path pre-read and outward exposure unchanged` |

**Why the predicted `OPEN, UNCHANGED` for all three did not hold, in full, for
any of them:** A2's central architectural change — every `read_note` call
resolves through `Journal::last_committed_for_note(note_id)`, keyed strictly
by note id, before any filesystem content is trusted — means a note id with
no write history of its own cannot be *read* at all, aliased or not; and the
unconditional post-read hash check means a committed note's file, once
swapped to different content, is rejected as `Inconsistent` rather than
served. Both are real, if incidental, read-side narrowings. But
`write_record_unchecked`'s own step 2 pre-write read is a structurally
separate code path that the journal lookup does **not** gate at all, and it
remains exactly as exposed as A1 left it — plus SEC-001's race window
(distinct from SEC-002/003's static aliasing, but sharing the exact same
step-2 read) is completely open on both its read-through and write-replace
sides. SEC-003's outward direction was never a `Vault` code path to begin
with and A2 could not have touched it.

**What is NOT closed, for any of the three — restated precisely:**

- SEC-001: the actual check-then-use race window (the gap between
  `record_path_checked` and both step 2's read and the final write) is
  completely open on both sides measured.
- SEC-002/SEC-003 (inward): the structural root cause —
  `record_path_checked` has no way to verify the resolved path is a genuine,
  un-aliased entry for the exact note it was asked about (no inode/link
  check, no "this points at a *different* note's record" rejection) — is
  completely unaddressed on the write path, and only incidentally caught on
  the read path by two mechanisms with documented limits (DoS-only; defeated
  by identical-content aliasing; no defense against direct journal
  tampering).
- SEC-003 (outward): entirely untouched, not a code path inside this crate,
  not addressable within A2's or this plan's scope.

**Closure mechanism proposed in A1 remains the correct fix for the inward
cases of all three:** verify the *opened handle* itself (regular file, link
count 1, path inside root) at the point of I/O — for BOTH step 2's read and
the final write — rather than relying on checks before or incidental side
effects after. A2 does not implement this, and per the plan's Global
Constraints this task does not attempt to. SEC-003's outward direction has no
analogous in-crate fix; it needs a different storage architecture entirely,
also out of scope.

**Restatement (mandatory, per the Interfaces spec):** all three debts —
SEC-001 `OPEN, UNCHANGED`; SEC-002 `NARROWED (read path only); write-path
pre-read unchanged and open`; SEC-003 `NARROWED (read path only); write-path
pre-read and outward exposure unchanged` — require re-evaluation again before
Fase D (Continuity [d5b9]).

---

## How to reproduce

```
cargo test -p storage sec_00 -- --nocapture
```

Actual output at the time of this evaluation (four test functions: two for
SEC-001 — write-side and read-side — and one each for SEC-002/003, each of
which internally runs multiple scenarios under one `#[test]` function with
multiple assertions):

```
running 4 tests
test vault::tests::sec_001_toctou_read_side_leaks_an_outside_records_version_and_bytes_into_the_before_image ... ok
test vault::tests::sec_003_hard_link_aliasing_is_invisible_to_canonicalization_after_a2 ... ok
test vault::tests::sec_002_read_path_rejects_both_aliasing_shapes_but_write_path_pre_read_does_not ... ok
test vault::tests::sec_001_toctou_window_between_path_check_and_write_still_exists ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 0.19s
```

All passing is the expected, by-design outcome for this task: each assertion
encodes the exploit's or the narrowing's *actual measured* outcome, not an
unconditional print. A future change to any of these tests failing is a
signal to re-derive the verdict above, not to adjust the test to pass again.
