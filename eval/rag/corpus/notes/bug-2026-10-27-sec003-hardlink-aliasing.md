---
id: bug-2026-10-27-sec003-hardlink-aliasing
schema_version: 1
type: bug
status: reviewed
trust: internal
classification: internal
project: eval-b3
source: docs/technical-debt.md#SEC-003
created: 2026-10-27
updated: 2026-10-27
event_period: 2026-10-27
relations: {}
---
Reproduced empirically on Windows, 2026-10-27: a hard link at root/note-a.record pointing to a file outside the vault on the same volume makes read_note(note-a) return the external file's content, and propose_and_commit at the external file's version number succeeds. Canonicalization cannot detect this because a hard link is not a symlink pointing elsewhere — it is a second directory entry pointing directly at the same inode, a genuine regular file whose path genuinely resolves inside the vault root.
