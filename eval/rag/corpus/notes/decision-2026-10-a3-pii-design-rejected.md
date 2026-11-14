---
id: decision-2026-10-a3-pii-design-rejected
schema_version: 1
type: decision
status: reviewed
trust: internal
classification: internal
project: eval-b3
source: continuity:x1ks
created: 2026-10-20
updated: 2026-10-20
event_period: 2026-10-20
relations: {}
---
An early A3 draft design was rejected before A3 closed: it would have unconditionally rejected Sensitive notes forever, treated the PiiStore and the Markdown pointer as two separately-recovered transactions, added an unauthenticated deletion-log reference file, required a Task 0 test that SEC-00x stay vulnerable, and added a new propose_delete public entry point.
