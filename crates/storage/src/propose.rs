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
    /// target that is not declared in `pending_ids` — is not a canonical
    /// [`NoteId`]. `field` names where it came
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
/// frontmatter, validates the note's id and every non-pending relation
/// target as a canonical [`NoteId`], classifies its relations against
/// `known_ids`/`pending_ids`, runs `gate.evaluate` (or
/// `gate.evaluate_schema_error` if parsing already failed), and — only on
/// `policy::GateDecision::Pass` — calls the crate-private
/// `Vault::write_record_unchecked`. No `ProposeError` variant is returned
/// after a write has happened; every rejection path returns before the vault
/// is ever touched, so a rejected note's record file is never created.
///
/// Identity — deliberately asymmetric:
///
/// - `known_ids` are real notes in this vault, so they are canonical
///   [`NoteId`]s, the same identity the vault uses for its lock key and
///   record path. A relation target that is not declared pending must parse
///   as a `NoteId` (else `ProposeError::InvalidNoteId`), and then either
///   resolves against `known_ids` or is a `domain::LinkError::BrokenLink`.
/// - `pending_ids` are **opaque strings**, not note ids: per spec contract
///   C6, a pending reference may be an external source (a URL, a DOI), a
///   note not yet imported, or an unresolved code symbol
///   (`src/main.rs::CorsMiddleware`). A relation target that is present
///   verbatim in `pending_ids` is accepted as pending and is never run
///   through `NoteId::parse`. Pending targets never touch the filesystem, so
///   they cannot alias a record file. Matching is exact (byte-for-byte):
///   the caller declares exactly which references are pending.
pub fn propose_and_commit(
    vault: &Vault,
    yaml: &str,
    expected_version: Version,
    operation_id: Uuid,
    known_ids: &HashSet<NoteId>,
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
            // A caller-declared pending reference is opaque (URL, DOI,
            // code symbol, ...) and is never forced through NoteId syntax.
            if !pending_ids.contains(target) {
                parse_note_id(&format!("relations.{relation}"), target)?;
            }
        }
    }

    // Every non-pending target is now a validated canonical NoteId string
    // (NoteId::parse never normalizes, so `as_str()` of a parsed id equals
    // the raw input), which makes `domain::classify_links`' plain string
    // comparison against `known` an exact canonical-identity comparison.
    // Pending targets are compared verbatim against the caller's opaque set.
    let known: HashSet<String> = known_ids.iter().map(|id| id.as_str().to_string()).collect();

    // propose_and_commit's contract exposes broken/self-referential links as
    // their own domain::LinkError directly (ProposeError::Link), not wrapped
    // in a GateRejection, so callers can match on the precise domain error
    // without unwrapping a policy-level rejection first. This check happens
    // before the gate call so a link error never reaches
    // GateRejection::Link by a different path.
    let link_state = match domain::classify_links(&note, &known, pending_ids) {
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
