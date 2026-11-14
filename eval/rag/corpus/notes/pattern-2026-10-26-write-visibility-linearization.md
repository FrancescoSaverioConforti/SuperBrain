---
id: pattern-2026-10-26-write-visibility-linearization
schema_version: 1
type: pattern
status: verified
trust: internal
classification: internal
project: eval-b3
source: docs/Spec_mvp.md#C1
created: 2026-10-26
updated: 2026-10-26
event_period: 2026-10-26
relations: {}
---
A write only becomes visible through the API once it reaches journal_committed, the logical linearization point. Every internal component — indexer, backup, MCP server — reads only committed versions through the core; none ever does a free scan of vault paths during a write.
