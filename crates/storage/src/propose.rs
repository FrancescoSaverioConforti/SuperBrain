use std::collections::HashSet;

use uuid::Uuid;

use crate::note_id::NoteId;
use crate::vault::{StorageError, Vault, Version};

/// Every way `propose_and_commit` can fail to commit a note, in the exact
/// short-circuit order the function evaluates them: schema parse, link
/// classification, gate rejection, then storage.
#[derive(Debug)]
pub enum ProposeError {
    Schema(domain::ValidationError),
    Link(domain::LinkError),
    Rejected(policy::GateRejection),
    Storage(StorageError),
}

/// The single public write path out of `storage`, and therefore out of the
/// whole workspace: parses `yaml` as frontmatter, classifies its relations
/// against `known_ids`/`pending_ids`, runs `gate.evaluate` (or
/// `gate.evaluate_schema_error` if parsing already failed), and — only on
/// `policy::GateDecision::Pass` — calls the crate-private
/// `Vault::write_record_unchecked`. No `ProposeError` variant is returned
/// after a write has happened; every rejection path returns before the vault
/// is ever touched, so a rejected note's record file is never created.
pub fn propose_and_commit(
    vault: &Vault,
    yaml: &str,
    expected_version: Version,
    operation_id: Uuid,
    known_ids: &HashSet<String>,
    pending_ids: &HashSet<String>,
    gate: &policy::Gate,
) -> Result<Version, ProposeError> {
    let note = match domain::parse_frontmatter(yaml) {
        Ok(note) => note,
        Err(err) => {
            return match gate.evaluate_schema_error(err) {
                policy::GateDecision::Reject(rejection) => Err(ProposeError::Rejected(rejection)),
                policy::GateDecision::Pass => {
                    unreachable!("evaluate_schema_error never returns Pass")
                }
            };
        }
    };

    // propose_and_commit's contract exposes broken/self-referential links as
    // their own domain::LinkError directly (ProposeError::Link), not wrapped
    // in a GateRejection, so callers can match on the precise domain error
    // without unwrapping a policy-level rejection first. This check happens
    // before the gate call so a link error never reaches
    // GateRejection::Link by a different path.
    let link_state = match domain::classify_links(&note, known_ids, pending_ids) {
        Ok(state) => state,
        Err(link_err) => return Err(ProposeError::Link(link_err)),
    };

    match gate.evaluate(&note, yaml, Ok(link_state)) {
        policy::GateDecision::Reject(rejection) => Err(ProposeError::Rejected(rejection)),
        policy::GateDecision::Pass => {
            let note_id = NoteId::parse(&note.id).map_err(|e| {
                ProposeError::Storage(StorageError::InvalidNoteId(e))
            })?;
            vault
                .write_record_unchecked(&note_id, yaml, expected_version, operation_id)
                .map_err(ProposeError::Storage)
        }
    }
}
