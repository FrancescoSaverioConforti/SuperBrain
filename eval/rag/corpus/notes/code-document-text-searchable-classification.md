---
id: code-document-text-searchable-classification
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
symbol: build_text_searchable

text_searchable is built differently by classification: Sensitive notes include only the seven non-temporal allowlisted structured fields, never body, source, relations, or any temporal string; Internal and Public notes additionally include body, source, and every relations.* target.
