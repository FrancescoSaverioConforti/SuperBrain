---
id: code-propose-propose-and-commit
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: crates/storage/src/propose.rs:propose_and_commit
created: 2026-10-26
updated: 2026-10-26
event_period: 2026-10-26
relations: {}
---
repository: superbrain
commit: e92dc781bcd32fcca5445295221f36aea2fe791e
path: crates/storage/src/propose.rs
symbol: propose_and_commit

pub fn propose_and_commit(vault: &Vault, note_text: &str, expected_version: Version, operation_id: Uuid, known_ids: &HashSet<NoteId>, pending_ids: &HashSet<String>, gate: &policy::Gate, pii_store: Option<&PiiStore>) -> Result<CommitResult, ProposeError>

The one entry point that parses a note's frontmatter, validates its relations, and commits it through the gate.
