---
id: bug-2026-10-27-sec002-fix-incomplete
schema_version: 1
type: bug
status: reviewed
trust: internal
classification: internal
project: eval-b3
source: docs/technical-debt.md#SEC-002 (quarta revisione)
created: 2026-10-27
updated: 2026-10-27
event_period: 2026-10-27
relations: {}
---
An initial fix proposed for SEC-002 — reject any symlink at the candidate .record path, same mechanism as SEC-001, claimed to cost nothing extra — was checked and found incomplete: a hard link (not a symlink) to the same path produces the identical aliasing effect, both outside the vault (SEC-003) and between two notes inside it, and a check that only rejects symlinks does not catch it, because canonicalization does not see hard links. The correct fix, shared with SEC-001 and SEC-003, is verifying the open handle, not adding another path-level check.
