use std::collections::HashSet;

use uuid::Uuid;

use crate::note_id::{NoteId, NoteIdError};
use crate::vault::{StorageError, Vault, Version};

/// Every way `propose_and_commit` can fail to commit a note, in the exact
/// short-circuit order the function evaluates them: schema parse (surfaced
/// via the gate as `Rejected`), note-id validation, link classification,
/// gate rejection, then storage.
#[derive(Debug)]
pub enum ProposeError {
    Schema(domain::ValidationError),
    /// A note id in the frontmatter — the note's own `id`, or a relation
    /// target — is not a canonical [`NoteId`]. `field` names where it came
    /// from (`"id"` or `"relations.<name>"`), `value` is the raw string.
    /// Raised before the vault is ever touched: in particular a case
    /// variant of an existing id (e.g. `NOTE-A` vs `note-a`), which would
    /// alias the same record file on a case-insensitive filesystem, ends
    /// here as `NoteIdError::MustBeLowercase` and never reaches storage.
    InvalidNoteId {
        field: String,
        value: String,
        error: NoteIdError,
    },
    Link(domain::LinkError),
    Rejected(policy::GateRejection),
    Storage(StorageError),
}

/// The single public write path out of `storage`, and therefore out of the
/// whole workspace (the raw primitives `atomic_write` and
/// `Vault::write_record_unchecked` are both `pub(crate)`): parses `yaml` as
/// frontmatter, validates the note's id and every relation target as a
/// canonical [`NoteId`], classifies its relations against
/// `known_ids`/`pending_ids`, runs `gate.evaluate` (or
/// `gate.evaluate_schema_error` if parsing already failed), and — only on
/// `policy::GateDecision::Pass` — calls the crate-private
/// `Vault::write_record_unchecked`. No `ProposeError` variant is returned
/// after a write has happened; every rejection path returns before the vault
/// is ever touched, so a rejected note's record file is never created.
///
/// Identity: `known_ids`/`pending_ids` are sets of [`NoteId`], and relation
/// targets are themselves validated as `NoteId`s before lookup, so every
/// identity comparison here (self-reference, resolved/pending lookup) and in
/// the vault (lock key, record path) uses the same canonical value — there
/// is no separately-derived string that could disagree about identity.
pub fn propose_and_commit(
    vault: &Vault,
    yaml: &str,
    expected_version: Version,
    operation_id: Uuid,
    known_ids: &HashSet<NoteId>,
    pending_ids: &HashSet<NoteId>,
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

    let note_id = parse_note_id("id", &note.id)?;
    let relations = &note.relations;
    for (relation, targets) in [
        ("supports", &relations.supports),
        ("contradicts", &relations.contradicts),
        ("derived_from", &relations.derived_from),
        ("supersedes", &relations.supersedes),
        ("applies_to", &relations.applies_to),
    ] {
        for target in targets {
            parse_note_id(&format!("relations.{relation}"), target)?;
        }
    }

    // Every id compared below is now a validated canonical NoteId string
    // (NoteId::parse never normalizes, so `as_str()` of a parsed id equals
    // the raw input), which makes `domain::classify_links`' plain string
    // comparison an exact canonical-identity comparison.
    let known: HashSet<String> = known_ids.iter().map(|id| id.as_str().to_string()).collect();
    let pending: HashSet<String> = pending_ids.iter().map(|id| id.as_str().to_string()).collect();

    // propose_and_commit's contract exposes broken/self-referential links as
    // their own domain::LinkError directly (ProposeError::Link), not wrapped
    // in a GateRejection, so callers can match on the precise domain error
    // without unwrapping a policy-level rejection first. This check happens
    // before the gate call so a link error never reaches
    // GateRejection::Link by a different path.
    let link_state = match domain::classify_links(&note, &known, &pending) {
        Ok(state) => state,
        Err(link_err) => return Err(ProposeError::Link(link_err)),
    };

    match gate.evaluate(&note, yaml, Ok(link_state)) {
        policy::GateDecision::Reject(rejection) => Err(ProposeError::Rejected(rejection)),
        policy::GateDecision::Pass => vault
            .write_record_unchecked(&note_id, yaml, expected_version, operation_id)
            .map_err(ProposeError::Storage),
    }
}

fn parse_note_id(field: &str, raw: &str) -> Result<NoteId, ProposeError> {
    NoteId::parse(raw).map_err(|error| ProposeError::InvalidNoteId {
        field: field.to_string(),
        value: raw.to_string(),
        error,
    })
}
