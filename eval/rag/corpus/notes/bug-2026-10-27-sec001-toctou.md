---
id: bug-2026-10-27-sec001-toctou
schema_version: 1
type: bug
status: reviewed
trust: internal
classification: internal
project: eval-b3
source: docs/technical-debt.md#SEC-001
created: 2026-10-27
updated: 2026-10-27
event_period: 2026-10-27
relations: {}
---
A1 introduced anti-symlink checks in crates/storage/src/vault.rs (record_path_checked / ensure_resolves_inside_root) that canonicalize the candidate .record path before use. This is a check-then-act pattern: it blocks symlinks already present at check time, but the filesystem can change between the check and the actual I/O (read, write, rename), a TOCTOU race. Deferred to A2; the correct fix is to open the file without following links and verify the already-open handle itself (regular file, link count 1, path inside root), not the path before opening it.
