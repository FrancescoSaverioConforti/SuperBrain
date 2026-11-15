---
id: code-document-sensitive-vs-internal-fields
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: crates/index/src/document.rs
created: 2026-10-30
updated: 2026-10-30
event_period: 2026-10-30
relations: {}
---
repository: superbrain
commit: e92dc781bcd32fcca5445295221f36aea2fe791e
path: crates/index/src/document.rs
symbol: structured_fields_as_text

The seven non-temporal allowlisted fields kept as searchable text even for Sensitive notes are id, schema_version, type, status, trust, classification, and project — joined with a space, in that fixed order, by structured_fields_as_text.
