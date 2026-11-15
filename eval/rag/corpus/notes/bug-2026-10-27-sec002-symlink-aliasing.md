---
id: bug-2026-10-27-sec002-symlink-aliasing
schema_version: 1
type: bug
status: reviewed
trust: internal
classification: internal
project: eval-b3
source: docs/technical-debt.md#SEC-002
created: 2026-10-27
updated: 2026-10-27
event_period: 2026-10-27
relations: {}
---
Reproduced empirically on 2026-10-27: a symlink at root/note-a.record pointing to root/note-b.record makes read_note(note-a) return note-b's content and version; propose_and_commit(note-a, expected = note-b's version) succeeds and replaces the symlink with a real file, leaving note-b intact. No escape from the vault and no cross-note write corruption, but a real integrity bug: content and version get misattributed to the wrong note id, and note-a inherits note-b's version number instead of starting from zero.
