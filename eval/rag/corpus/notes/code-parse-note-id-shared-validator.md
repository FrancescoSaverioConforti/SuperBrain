---
id: code-parse-note-id-shared-validator
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: crates/storage/src/propose.rs:parse_note_id
created: 2026-10-26
updated: 2026-10-26
event_period: 2026-10-26
relations: {}
---
repository: superbrain
commit: see corpus-manifest.json build_commit
path: crates/storage/src/propose.rs
symbol: parse_note_id

The internal function parse_note_id(field: &str, raw: &str) -> Result<NoteId, ProposeError> validates a candidate id string against NoteId's syntax, reused by propose_and_commit both for a note's own id and for every non-pending relations.* target.
