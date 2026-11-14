---
id: pattern-2026-10-27-handle-not-path
schema_version: 1
type: pattern
status: verified
trust: internal
classification: internal
project: eval-b3
source: docs/technical-debt.md
created: 2026-10-27
updated: 2026-10-27
event_period: 2026-10-27
relations: {}
---
SEC-001, SEC-002 and SEC-003 (symlink TOCTOU, internal symlink aliasing, hard link aliasing) share one root cause: a check on a path cannot see how that path will resolve when it is actually opened. All three share one proposed closing mechanism: verify the already-open handle — regular file, link count 1, path inside root — rather than adding another check on the path before opening it.
