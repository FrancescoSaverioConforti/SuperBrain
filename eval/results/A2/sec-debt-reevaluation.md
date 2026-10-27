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
not silently corrected to match the prediction. The adversarial tests
ultimately committed assert the *measured* behavior, each with an inline
comment explaining the deviation from the brief's literal assertion and why.

---

## SEC-001 — TOCTOU / symlink race on the write path (Continuity [d5b9])

**Original exploit scenario (A1):** a check-then-use race in
`record_path_checked`: the containment check canonicalizes the record path and
judges it safe, but nothing is atomic between that check and the later I/O
(`std::fs::read`, or `atomic_write`'s create/rename). A concurrent process that
swaps the path for a symlink *after* the check but *before* the write can, in
principle, cause the write to land bytes somewhere the check never approved.

**Test added:**
`vault::tests::sec_001_toctou_window_between_path_check_and_write_still_exists`.

Mechanism: a new fault point, `FaultPoint::AfterPathCheckedBeforeAtomicWrite`
(added to `crates/storage/src/fault.rs`), is checked in
`write_record_unchecked_with_injector` immediately after
`record_path_checked` returns and before the per-note lock / `atomic_write`
call (`crates/storage/src/vault.rs`). The test arms a `FaultInjector::pause_at`
on that point, starts a writer thread, waits for the pause signal, deletes the
checked-safe `note-a.record` path and replaces it with a symlink pointing at
`outside/victim` (outside the vault root), then resumes the writer.

**Actual result (measured, diagnostic run included below):**

```
DIAG write_result=Ok(Version(1))
DIAG link_is_symlink=Ok(false)
DIAG link_contents=Ok("1\n---\ncontent")
DIAG outside_dir_listing=Ok([])
```

The write **succeeds** (`Ok(Version(1))`) despite the mid-flight swap — the
race window the original finding identified is still open; nothing in A2's
containment check, journal, or hash verification closes it. But the
*consequence* diverges from the brief's literal prediction
(`outside/victim.exists()` was predicted `true`): `atomic_write`
(`crates/storage/src/atomic.rs`) writes to a sibling temp file and then calls
`std::fs::rename(&tmp_path, path)`. Measured directly (both via this test and
an isolated minimal probe compiled outside the test harness), a rename whose
destination is an existing symlink **replaces the symlink itself** — it does
not follow the link to write through to its target. So:

- the attacker's symlink at `note-a.record` is silently destroyed and
  replaced by the vault's own record content (confirmed:
  `link_is_symlink` is `false` afterward, and the file's contents are the
  vault's own serialized record);
- `outside/victim` is never created (confirmed: the `outside` directory is
  empty afterward).

**This is a genuine, reproducible finding that diverges from the brief's
prediction**, not a test bug: the race still silently proceeds through a path
an attacker swapped after the check approved it (violating the "write what you
checked" invariant — SEC-001's core defect), but on this platform's
`atomic_write` implementation (`std::fs::rename` onto the final path, which
replaces a destination symlink rather than following it), the concrete
consequence is destruction of the attacker's symlink, not exfiltration of
vault content outside the root. This is a property of `atomic_write`'s
rename-based design, present unchanged since A1 — A2 added neither the
exposure nor this particular mitigating side-effect.

The committed test asserts all three observed facts: the write succeeds
despite the swap; the symlink is replaced (not left in place, not causing an
I/O error); and bytes do **not** land in `outside`. If `atomic_write`'s
semantics ever change (e.g. a future platform/filesystem combination where
rename *does* follow a destination symlink), the third assertion would fail
loudly, flagging a strictly worse variant of SEC-001.

**Verdict: `OPEN, UNCHANGED`** (narrowed only in the sense that *this specific
write primitive's* consequence is not exfiltration — the check-then-use race
itself, and the general class of risk it represents, is untouched by A2). The
journal's hash verification (Step 5 of `write_record_unchecked`) happens
*after* the write, over whatever bytes are now at the resolved path — it has
no bearing on what path resolution the race already completed. Closing this
properly still requires handle-relative (`openat`/`O_NOFOLLOW`-style) I/O, out
of scope for A2 per the plan's Global Constraints.

---

## SEC-002 — intra-vault record aliasing via symlink (Continuity [c16c6])

**Original exploit scenario (A1):** a symlink at one note's record path
pointing at another note's record path, both inside the vault root (so
SEC-001's containment check, which only rejects paths resolving *outside* the
root, does not reject it). Reading or writing through the aliased note id
transparently operates on the other note's bytes.

**Tests added (two scenarios, both under**
`vault::tests::sec_002_journal_hash_check_does_not_detect_aliasing_present_before_the_write`**):**

### Scenario A — brief's literal scenario: alias a note id that was never itself written

`note-b` is written for real (its own committed journal row and hash exist).
`note-a.record` is then symlinked to `note-b.record` *before* `note-a` has
ever been written. `read_note(note-a)` is called directly, as the brief's
verbatim code specifies.

**Brief's prediction:** `read_note(note-a)` succeeds and returns note-b's
content (`"note-b real content"`) — the alias is live and undetected.

**Actual result (measured):** `read_note(note-a)` returns
`Err(StorageError::NotFound { note_id: "note-a" })`.

**This diverges from the brief's prediction and is a genuine finding.**
`read_note_locked` (`crates/storage/src/vault.rs`) resolves a note
*exclusively* through `Journal::last_committed_for_note(note_id)` — a lookup
keyed strictly by the **logical note id being read**, never by "whatever bytes
happen to live at the resolved path." Since no journal row has ever been
created under the key `"note-a"` (only `"note-b"` has a committed row), the
journal lookup itself fails before the read ever gets far enough to open the
aliased file and hash its content. A2's journal-mediated read architecture —
not its hash verification specifically — accidentally narrows exactly this
sub-case: **aliasing a note id with no write history of its own is now
rejected outright, not silently served.**

### Scenario B — a more realistic variant: alias a note id that already has its own committed history

`note-a-v2` is written for real first (its own committed row, its own
content-hash, over its own original content). Its on-disk file is then
removed and replaced with a symlink to `note-b.record` (different content,
different hash). `read_note(note-a-v2)` is called.

**Actual result (measured):** `read_note(note-a-v2)` returns
`Err(StorageError::Inconsistent { note_id: "note-a-v2" })`.

This sub-case **is** caught, and for the predictable reason: `read_note`'s
hash check (`crates/storage/src/vault.rs`) is unconditional on every read —
it compares the resolved journal row's `content_hash` (computed at write time,
over note-a-v2's *own* original bytes) against the hash of whatever bytes are
actually at the path now. Swapping in note-b's differently-hashing content
after the fact is caught and fails closed as `Inconsistent`, not silently
aliased.

**Verdict: `NARROWED`.** Two real sub-cases exist and they resolve
differently:

- A note id with its own prior committed write, later aliased to a
  *different* note's content: **closed** by A2's unconditional read-time hash
  check (Scenario B, confirmed above). This is the one sub-case where A2's
  hash verification demonstrably earns its keep against SEC-002, though this
  is a side effect of a general integrity check, not a purpose-built
  anti-aliasing defense.
- A note id with **no** prior write of its own, aliased from the start:
  **closed too**, but for an unrelated, architectural reason (the journal
  lookup is keyed per-note-id and finds nothing) — not because anything
  detects or rejects the alias as such.

Neither sub-case the committed tests exercise demonstrates a *live, silent*
alias surviving a `read_note` call today. This is a real narrowing from A1
(where no journal existed and the raw filesystem was trusted directly), but it
is **not** a purpose-built fix for SEC-002, and the debt is not fully closed:
the underlying structural gap — `record_path_checked` accepts any symlink
target that resolves inside the root, with no check that the resolved file is
the one record it was asked for (same inode/no symlink at all) — is completely
unchanged. A variant this task's tests do not cover (for example, an alias
whose target bytes happen to hash-collide with the expected row, or an
in-process cache/other read path that does not route through
`last_committed_for_note`) could still be exposed; this evaluation checked the
two concrete, realistic shapes available through `Vault`'s public read
surface, not every conceivable one.

---

## SEC-003 — hard-link aliasing (Continuity [c17e8])

**Original exploit scenario (A1):** a hard link at a note's record path to a
file outside the vault (or to another note's record inside it). Canonicalization
cannot see a hard link at all — it is a second directory entry to the same
inode, not a redirect — so the containment check cannot reject it by
construction, regardless of any fix applied to symlink handling.

**Tests added (two scenarios, both under**
`vault::tests::sec_003_hard_link_aliasing_is_invisible_to_canonicalization_after_a2`**):**

### Scenario A — brief's literal scenario: hard-link a note id that was never itself written

An outside file `victim.record` holds a well-formed record (`"OUTSIDE"`,
version 7). `note-a.record` is hard-linked to it before `note-a` has ever been
written. `read_note(note-a)` is called directly, as the brief's verbatim code
specifies.

**Brief's prediction:** `read_note(note-a)` succeeds and returns `"OUTSIDE"` —
canonicalization's blind spot lets the hard link through undetected.

**Actual result (measured):** `read_note(note-a)` returns
`Err(StorageError::NotFound { note_id: "note-a" })`.

**This diverges from the brief's prediction, for the identical reason as
SEC-002's Scenario A:** the journal lookup by note id fails before
canonicalization's blind spot is ever reached, because `note-a` has no
committed row at all.

### Scenario B — a more realistic variant: hard-link over a note id that already has its own committed history

`note-a-v2` is written for real first. Its file is then removed and replaced
with a hard link to the same outside `victim.record`. `read_note(note-a-v2)`
is called.

**Actual result (measured):** `read_note(note-a-v2)` returns
`Err(StorageError::Inconsistent { note_id: "note-a-v2" })`.

Same mechanism as SEC-002's Scenario B: the committed row's hash, from
note-a-v2's own original write, does not match the hard-linked victim content's
hash, so the read fails closed.

**Verdict: `NARROWED`.** Exactly the same shape and the same caveats as
SEC-002's verdict above: canonicalization genuinely still cannot see a hard
link — that fact is completely unchanged by A2, confirmed structurally (a hard
link has no separate link-count/symlink metadata canonicalization inspects)
and is not retested further here since A1 already established it and nothing
in A2 touches `record_path_checked`'s canonicalization logic. What changed is
that **every read now also goes through a per-note-id journal lookup before
the file is ever opened**, and that lookup — plus, for an already-committed
note, the unconditional post-read hash check — closes both concrete scenarios
tested here. The structural blind spot (canonicalization cannot distinguish a
hard link from an ordinary file) is untouched; only the specific path *through*
`Vault::read_note` that would have exploited it is narrowed.

---

## Summary

| Debt | Original scenario | Measured against A2 | Verdict |
|------|-------------------|----------------------|---------|
| SEC-001 | TOCTOU race between path-check and write | Race still proceeds through a swapped path; `atomic_write`'s `rename` replaces the destination symlink rather than following it, so bytes do not land outside the root via this specific primitive (divergence from the brief's prediction) | `OPEN, UNCHANGED` |
| SEC-002 | Symlink-alias one note's record to another's | Both a never-written alias (rejected as `NotFound` by the per-note-id journal lookup) and a post-commit swap of an already-written note's file to different content (rejected as `Inconsistent` by the unconditional read-time hash check) are caught by the concrete tests run (divergence from the brief's prediction of a silent, successful alias) | `NARROWED` |
| SEC-003 | Hard-link-alias a note's record to outside/other content | Identical outcome and identical two sub-cases as SEC-002, for the same two reasons; canonicalization's blind spot is structurally unchanged but the read path's journal gating and hash check close both tested scenarios | `NARROWED` |

**Why the predicted `OPEN, UNCHANGED` for all three did not hold for SEC-002
and SEC-003 as literally specified:** the brief's test code read a note that
was *never itself written through `write_record_unchecked`*. A2's central
architectural change — every read resolves through
`Journal::last_committed_for_note(note_id)`, not through the raw filesystem —
means a note id with no write history of its own cannot be read at all,
aliased or not. This is a real, if incidental, narrowing: it was not designed
as an anti-aliasing defense, but it has that effect for this specific shape of
attack. The more realistic variant this task added (alias a note that *does*
have its own commit history) is also caught, but by a different mechanism
(the unconditional content-hash check on every read) and for a different
reason (the hashes simply do not match once different content is substituted).
Neither mechanism inspects the resolved path's link structure; both are
general-purpose integrity checks that happen to also catch these two
aliasing shapes.

**What is NOT closed, for any of the three:**

- SEC-001's actual check-then-use race window (the gap between
  `record_path_checked` and the write touching the path) is completely open;
  only the specific consequence this platform's `atomic_write` produces
  differs from what was predicted.
- SEC-002/SEC-003's structural root cause — `record_path_checked` has no way
  to verify the resolved path is a genuine, un-aliased regular file for the
  exact note it was asked about (no inode/link-count check, no "this symlink
  points at a *different* note's record" rejection) — is completely
  unaddressed. The two sub-cases tested here happen to be caught by
  *unrelated* machinery (per-note-id journal gating; unconditional hash
  verification), not by anything that recognizes or rejects aliasing as such.
  A shape not tested here that bypasses both of those incidental checks (for
  instance, content engineered to hash-collide, or any future read path that
  does not route through `last_committed_for_note` and the hash check) could
  still be exposed.

**Closure mechanism proposed in A1 remains the correct fix for all three:**
verify the *opened handle* itself (regular file, link count 1, path inside
root) at the point of I/O, rather than relying on checks before or
incidental side effects after. A2 does not implement this, and per the plan's
Global Constraints this task does not attempt to.

**Restatement (mandatory, per the Interfaces spec):** all three debts —
SEC-001 `OPEN, UNCHANGED`, SEC-002 `NARROWED`, SEC-003 `NARROWED` — require
re-evaluation again before Fase D (Continuity [d5b9]).

---

## How to reproduce

```
cargo test -p storage sec_00 -- --nocapture
```

Actual output at the time of this evaluation:

```
running 3 tests
test vault::tests::sec_003_hard_link_aliasing_is_invisible_to_canonicalization_after_a2 ... ok
test vault::tests::sec_001_toctou_window_between_path_check_and_write_still_exists ... ok
test vault::tests::sec_002_journal_hash_check_does_not_detect_aliasing_present_before_the_write ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 0.15s
```

All three passing is the expected, by-design outcome for this task: each
assertion encodes the exploit's or the narrowing's *actual measured* outcome,
not an unconditional print. A future change to any of these tests failing is
a signal to re-derive the verdict above, not to adjust the test to pass again.
