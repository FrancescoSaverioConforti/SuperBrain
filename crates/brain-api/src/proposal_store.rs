//! `.proposals/` persistence (B5.1 Task 12): serializes/deserializes a
//! [`PendingWriteProposal`] and persists it through `storage::Vault`'s own
//! confined, validated proposal API (`save_proposal`/`load_proposal`/
//! `list_proposal_ids`) — never a general filesystem write primitive.
//! `storage` owns every filesystem-safety guarantee (B5.SEC Approach A);
//! this module never touches a raw handle, a path, or any
//! `handle_safety`-aware type directly.

use crate::{BrainApiError, PendingWriteProposal, ProposalId, ProposalProvenance, ProposalStatus, ProposedOperation, RelationKind};

/// The on-disk shape of a persisted proposal. A deliberately separate,
/// fully-public "shadow" struct — not `PendingWriteProposal` itself, whose
/// `status` field is private by design (its only mutation path is
/// `transition_to`, never a public setter, never serde's derive-generated
/// field-by-field construction). Serializing/deserializing through this
/// shadow type means persistence can read and write `status` without the
/// runtime `PendingWriteProposal` type ever gaining a public way to set it
/// directly.
///
/// `operation` is `Option` purely as a parse-time staging area: a persisted
/// file written under schema version 1 (before this field existed) has no
/// `operation` key at all, and `serde_yaml` would otherwise reject the
/// whole document as a missing-field error rather than letting
/// `from_persisted` turn that specific condition into a clean,
/// fail-closed `UnsupportedOperation`. It is never `None` for anything
/// this module itself writes (schema version 2+).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedProposal {
    schema_version: u32,
    id: String,
    originating_identity: String,
    source: String,
    created_at: String,
    expires_at: Option<String>,
    status: PersistedStatus,
    #[serde(default)]
    operation: Option<PersistedOperation>,
}

/// Mirrors [`ProposedOperation`]'s shape for serialization, exactly as
/// [`PersistedStatus`] mirrors [`ProposalStatus`] — a separate, explicit
/// type rather than deriving `Serialize` on the domain enum directly.
///
/// `note_type`/`classification` serialize through `domain::NoteType`'s/
/// `domain::Classification`'s own `Serialize`/`Deserialize` impls directly
/// (no further shadow type for them): both are already stable, public,
/// `#[serde(rename_all = "kebab-case")]` enums owned by `domain`, the exact
/// same type `render_frontmatter`/`parse_frontmatter` use — mirroring them
/// a second time here would duplicate, not protect, that format.
/// `expected_version` serializes through `storage::Version`'s own public
/// `u64` field (`pub struct Version(pub u64)`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
enum PersistedOperation {
    SaveMemory {
        note_id: String,
        note_type: domain::NoteType,
        classification: domain::Classification,
        project: String,
        source: String,
        event_period: String,
        body: String,
    },
    UpdateMemory {
        note_id: String,
        body: String,
        expected_version: u64,
    },
    LinkEntities {
        from: String,
        to: String,
        relation: PersistedRelationKind,
    },
}

/// Mirrors [`RelationKind`]'s exactly 5 variants for serialization.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
enum PersistedRelationKind {
    Supports,
    Contradicts,
    DerivedFrom,
    Supersedes,
    AppliesTo,
}

impl From<RelationKind> for PersistedRelationKind {
    fn from(relation: RelationKind) -> Self {
        match relation {
            RelationKind::Supports => PersistedRelationKind::Supports,
            RelationKind::Contradicts => PersistedRelationKind::Contradicts,
            RelationKind::DerivedFrom => PersistedRelationKind::DerivedFrom,
            RelationKind::Supersedes => PersistedRelationKind::Supersedes,
            RelationKind::AppliesTo => PersistedRelationKind::AppliesTo,
        }
    }
}

impl From<PersistedRelationKind> for RelationKind {
    fn from(relation: PersistedRelationKind) -> Self {
        match relation {
            PersistedRelationKind::Supports => RelationKind::Supports,
            PersistedRelationKind::Contradicts => RelationKind::Contradicts,
            PersistedRelationKind::DerivedFrom => RelationKind::DerivedFrom,
            PersistedRelationKind::Supersedes => RelationKind::Supersedes,
            PersistedRelationKind::AppliesTo => RelationKind::AppliesTo,
        }
    }
}

impl From<&ProposedOperation> for PersistedOperation {
    fn from(operation: &ProposedOperation) -> Self {
        match operation {
            ProposedOperation::SaveMemory { note_id, note_type, classification, project, source, event_period, body } => {
                PersistedOperation::SaveMemory {
                    note_id: note_id.clone(),
                    note_type: *note_type,
                    classification: *classification,
                    project: project.clone(),
                    source: source.clone(),
                    event_period: event_period.clone(),
                    body: body.clone(),
                }
            }
            ProposedOperation::UpdateMemory { note_id, body, expected_version } => {
                PersistedOperation::UpdateMemory { note_id: note_id.clone(), body: body.clone(), expected_version: expected_version.0 }
            }
            ProposedOperation::LinkEntities { from, to, relation } => {
                PersistedOperation::LinkEntities { from: from.clone(), to: to.clone(), relation: (*relation).into() }
            }
        }
    }
}

impl From<PersistedOperation> for ProposedOperation {
    fn from(operation: PersistedOperation) -> Self {
        match operation {
            PersistedOperation::SaveMemory { note_id, note_type, classification, project, source, event_period, body } => {
                ProposedOperation::SaveMemory { note_id, note_type, classification, project, source, event_period, body }
            }
            PersistedOperation::UpdateMemory { note_id, body, expected_version } => {
                ProposedOperation::UpdateMemory { note_id, body, expected_version: storage::Version(expected_version) }
            }
            PersistedOperation::LinkEntities { from, to, relation } => {
                ProposedOperation::LinkEntities { from, to, relation: relation.into() }
            }
        }
    }
}

/// Mirrors [`ProposalStatus`]'s shape for serialization. A separate type
/// (not `#[derive(Serialize)]` on `ProposalStatus` itself) so this format
/// is explicit and reviewable independent of the domain type's own
/// evolution.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state")]
enum PersistedStatus {
    Pending,
    Approved { approved_at: String },
    Rejected { reason: String, rejected_at: String },
    Expired,
}

impl From<&ProposalStatus> for PersistedStatus {
    fn from(status: &ProposalStatus) -> Self {
        match status {
            ProposalStatus::Pending => PersistedStatus::Pending,
            ProposalStatus::Approved { approved_at } => PersistedStatus::Approved { approved_at: approved_at.clone() },
            ProposalStatus::Rejected { reason, rejected_at } => {
                PersistedStatus::Rejected { reason: reason.clone(), rejected_at: rejected_at.clone() }
            }
            ProposalStatus::Expired => PersistedStatus::Expired,
        }
    }
}

impl From<PersistedStatus> for ProposalStatus {
    fn from(status: PersistedStatus) -> Self {
        match status {
            PersistedStatus::Pending => ProposalStatus::Pending,
            PersistedStatus::Approved { approved_at } => ProposalStatus::Approved { approved_at },
            PersistedStatus::Rejected { reason, rejected_at } => ProposalStatus::Rejected { reason, rejected_at },
            PersistedStatus::Expired => ProposalStatus::Expired,
        }
    }
}

/// The schema version this module currently writes and the minimum it can
/// read.
///
/// - Bumped from 1 to 2 when `operation` was added (Task 14 prerequisite
///   #1): a pre-existing version-1 file has no `operation` field at all,
///   and there is no inferred/default operation that would be safe to
///   assume for a proposal that might be approved and executed — version 1
///   is deliberately UNSUPPORTED, not migrated.
/// - Bumped from 2 to 3 when `ProposedOperation::SaveMemory`/`UpdateMemory`
///   gained their B5.0-contract-required fields (Task 14 prerequisite #2:
///   `note_id`/`note_type`/`classification`/`project`/`source`/
///   `event_period` for `SaveMemory`; `expected_version` for
///   `UpdateMemory`). A version-2 file's `SaveMemory`/`UpdateMemory`
///   payload is missing these required fields entirely (no
///   `#[serde(default)]` on any of them — a proposal missing its
///   `expected_version` or its note's intended `NoteType`/`classification`
///   has no safe default either), so version 2 is likewise UNSUPPORTED,
///   not migrated: a version-2 file's `operation` key now fails to parse
///   as a `PersistedOperation` at all (missing required fields inside the
///   tagged variant), which `load_proposal` already maps to
///   `BrainApiError::CorruptData` — the same fail-closed outcome a missing
///   `operation` key gets, just via a different point in the same parse.
///
/// In both cases, a persisted proposal is a promise about what will be
/// executed on approval, and an incomplete promise must fail closed, never
/// be silently interpreted via a guessed default.
const PROPOSAL_SCHEMA_VERSION: u32 = 3;

fn to_persisted(proposal: &PendingWriteProposal) -> PersistedProposal {
    PersistedProposal {
        schema_version: PROPOSAL_SCHEMA_VERSION,
        id: proposal.id.to_string(),
        originating_identity: proposal.originating_identity.clone(),
        source: proposal.provenance.source.clone(),
        created_at: proposal.provenance.created_at.clone(),
        expires_at: proposal.expires_at.clone(),
        status: PersistedStatus::from(proposal.status()),
        operation: Some(PersistedOperation::from(&proposal.operation)),
    }
}

/// Reconstructs a `PendingWriteProposal` from its persisted shape. Uses
/// `PendingWriteProposal::new` (always `Pending`) then, if the persisted
/// status is not `Pending`, moves it forward via the one legitimate
/// mutation path (`transition_to`) — so a persisted proposal can never be
/// reconstructed into a state `valid_proposal_transition` would not have
/// allowed as a real transition from `Pending`. A persisted file somehow
/// holding a status `transition_to` rejects (which cannot happen through
/// this module's own `to_persisted`, but could from a hand-edited or
/// corrupted file) surfaces as `BrainApiError::CorruptData`, not a silent
/// construction of an impossible state.
///
/// `persisted.operation: None` (a version-1 file, or a hand-edited/
/// corrupted one missing the key) fails closed as `CorruptData` — never
/// defaults to any `ProposedOperation` variant. The schema-version check in
/// `load_proposal` already rejects a real version-1 file before this
/// function runs; this is the second, defense-in-depth check for a file
/// that claims the current version but is missing the field anyway.
fn from_persisted(persisted: PersistedProposal) -> Result<PendingWriteProposal, BrainApiError> {
    let id = ProposalId::parse(&persisted.id).map_err(|_| BrainApiError::CorruptData)?;
    let operation: ProposedOperation = persisted.operation.ok_or(BrainApiError::CorruptData)?.into();
    let mut proposal = PendingWriteProposal::new(
        id,
        persisted.originating_identity,
        ProposalProvenance { source: persisted.source, created_at: persisted.created_at },
        persisted.expires_at,
        operation,
    );
    let target_status: ProposalStatus = persisted.status.into();
    if target_status != ProposalStatus::Pending {
        proposal.transition_to(target_status).map_err(|_| BrainApiError::CorruptData)?;
    }
    Ok(proposal)
}

/// Persists `proposal` at `.proposals/{proposal.id}.proposal`, through
/// `storage::Vault::save_proposal` (which validates the id itself and
/// confines the write to `.proposals/` — see that method's own doc
/// comment). Maps every `storage::StorageError` onto the frozen
/// `BrainApiError` vocabulary at this boundary — no internal detail
/// (filesystem path, I/O error text) crosses into the public error.
///
/// `pub(crate)`, not `pub`: raw persistence with no permission check at
/// all — restricted during B5's layering review for the same reason
/// `list_proposals` below already was at Task 14 (its own doc comment
/// explains the precedent). `approve_proposal`/`reject_proposal`
/// (`approval.rs`, Task 14) and `save_memory`/`update_memory`/
/// `link_entities` (`memory.rs`/`graph.rs`, Task 13) are the crate's real,
/// `WritePolicy`-gated public write/propose surface; none of them are
/// bypassed by this restriction, since all of them already call this
/// through `crate::proposal_store::save_proposal`, not through a crate-root
/// re-export. No crate-external consumer depended on the old `pub`
/// re-export (confirmed: `crates/brain-api/tests/*.rs` never names
/// `brain_api::save_proposal`).
pub(crate) fn save_proposal(vault: &storage::Vault, proposal: &PendingWriteProposal) -> Result<(), BrainApiError> {
    let persisted = to_persisted(proposal);
    let bytes = serde_yaml::to_string(&persisted).map_err(|_| BrainApiError::RuntimeUnavailable)?;
    vault.save_proposal(&proposal.id.to_string(), bytes.as_bytes()).map_err(map_storage_error)
}

/// Loads and reconstructs the proposal persisted under `id`.
///
/// `pub(crate)`, not `pub`: same restriction and rationale as
/// [`save_proposal`] above — raw, unauthenticated-by-itself persistence
/// read with no permission check; `approve_proposal`/`reject_proposal`/
/// `list_proposals` are the gated public surface that may read a persisted
/// proposal.
pub(crate) fn load_proposal(vault: &storage::Vault, id: &ProposalId) -> Result<PendingWriteProposal, BrainApiError> {
    let bytes = vault.load_proposal(&id.to_string()).map_err(map_storage_error)?;
    let text = String::from_utf8(bytes).map_err(|_| BrainApiError::CorruptData)?;
    let persisted: PersistedProposal = serde_yaml::from_str(&text).map_err(|_| BrainApiError::CorruptData)?;
    if persisted.schema_version != PROPOSAL_SCHEMA_VERSION {
        return Err(BrainApiError::UnsupportedOperation);
    }
    from_persisted(persisted)
}

/// Lists every currently-persisted proposal. Skips (does not fail the
/// whole call for) any individual id that fails to load or parse —
/// mirrors `storage::Vault::list_proposal_ids`'s own "discovery, not a
/// guarantee every entry is valid" discipline; a single corrupted
/// proposal file must never prevent listing the rest.
///
/// `pub(crate)`, not `pub`: raw, unauthenticated persistence enumeration
/// with no permission check and no expiry re-check — the opposite of
/// what a public "list proposals" API should guarantee. `crate::approval`
/// (Task 14) is the actual public, `WritePolicy`-gated, expiry-aware
/// `list_proposals` this crate exposes; `save_memory`/`update_memory`/
/// `link_entities` never call this (they only ever write new proposals,
/// never enumerate existing ones). Corrected from a premature `pub` this
/// module briefly had (Task 12, before Task 14's own gated version
/// existed to need disambiguating from) — no external consumer depended
/// on the old public surface.
pub(crate) fn list_proposals(vault: &storage::Vault) -> Result<Vec<PendingWriteProposal>, BrainApiError> {
    let ids = vault.list_proposal_ids().map_err(map_storage_error)?;
    let mut proposals = Vec::new();
    for id_str in ids {
        let Ok(id) = ProposalId::parse(&id_str) else { continue };
        if let Ok(proposal) = load_proposal(vault, &id) {
            proposals.push(proposal);
        }
    }
    Ok(proposals)
}

/// The one place `storage::StorageError` crosses into `BrainApiError`,
/// for this module's two call sites. Explicit per-variant mapping — no
/// blanket `From` impl, so a future `StorageError` variant added upstream
/// fails to compile here rather than silently collapsing into a default.
fn map_storage_error(err: storage::StorageError) -> BrainApiError {
    match err {
        storage::StorageError::NotFound { .. } => BrainApiError::NotFound,
        storage::StorageError::PathEscapesVaultRoot { .. } => BrainApiError::InvalidInput("invalid proposal id".into()),
        storage::StorageError::Conflict { .. } => BrainApiError::Conflict,
        storage::StorageError::Io(_) | storage::StorageError::Journal(_) => BrainApiError::RuntimeUnavailable,
        _ => BrainApiError::CorruptData,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-brain-api-proposal-store-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_vault() -> storage::Vault {
        let dir = unique_test_dir();
        storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap()
    }

    fn sample_save_memory_operation(body: &str) -> ProposedOperation {
        ProposedOperation::SaveMemory {
            note_id: "note-sample".into(),
            note_type: domain::NoteType::Bug,
            classification: domain::Classification::Internal,
            project: "project-x".into(),
            source: "agent:coder@prompt-v3".into(),
            event_period: "2026-11-16".into(),
            body: body.into(),
        }
    }

    fn sample_proposal() -> PendingWriteProposal {
        PendingWriteProposal::new(
            ProposalId::generate(),
            "mcp-session-abc".into(),
            ProposalProvenance { source: "mcp-session-abc".into(), created_at: "2026-11-16T00:00:00Z".into() },
            Some("2026-11-17T00:00:00Z".into()),
            sample_save_memory_operation("sample body"),
        )
    }

    fn proposal_with_operation(operation: ProposedOperation) -> PendingWriteProposal {
        PendingWriteProposal::new(
            ProposalId::generate(),
            "mcp-session-abc".into(),
            ProposalProvenance { source: "mcp-session-abc".into(), created_at: "2026-11-16T00:00:00Z".into() },
            Some("2026-11-17T00:00:00Z".into()),
            operation,
        )
    }

    #[test]
    fn save_then_load_proposal_round_trips_exactly() {
        let vault = test_vault();
        let proposal = sample_proposal();
        save_proposal(&vault, &proposal).unwrap();
        let loaded = load_proposal(&vault, &proposal.id).unwrap();
        assert_eq!(loaded, proposal);
    }

    #[test]
    fn save_then_load_preserves_a_non_pending_status() {
        let vault = test_vault();
        let mut proposal = sample_proposal();
        proposal.transition_to(ProposalStatus::Approved { approved_at: "2026-11-16T01:00:00Z".into() }).unwrap();
        save_proposal(&vault, &proposal).unwrap();
        let loaded = load_proposal(&vault, &proposal.id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Approved { approved_at: "2026-11-16T01:00:00Z".into() });
    }

    #[test]
    fn load_proposal_for_an_unknown_id_returns_not_found() {
        let vault = test_vault();
        let unknown_id = ProposalId::generate();
        let result = load_proposal(&vault, &unknown_id);
        assert_eq!(result, Err(BrainApiError::NotFound));
    }

    #[test]
    fn load_proposal_on_malformed_yaml_returns_corrupt_data() {
        let vault = test_vault();
        let id = ProposalId::generate();
        vault.save_proposal(&id.to_string(), b"not: valid: yaml: at: all: [").unwrap();
        let result = load_proposal(&vault, &id);
        assert_eq!(result, Err(BrainApiError::CorruptData));
    }

    #[test]
    fn load_proposal_on_a_schema_version_mismatch_returns_unsupported_operation() {
        let vault = test_vault();
        let id = ProposalId::generate();
        let future_schema = format!(
            "schema_version: 999\nid: {id}\noriginating_identity: x\nsource: x\ncreated_at: '2026-11-16T00:00:00Z'\nexpires_at: null\nstatus:\n  state: Pending\n"
        );
        vault.save_proposal(&id.to_string(), future_schema.as_bytes()).unwrap();
        let result = load_proposal(&vault, &id);
        assert_eq!(result, Err(BrainApiError::UnsupportedOperation));
    }

    #[test]
    fn load_proposal_on_a_pre_task_14_schema_version_1_file_returns_unsupported_operation() {
        // A real version-1 file (written before `operation` existed) has
        // no `operation` key at all. Schema version 1 is deliberately
        // UNSUPPORTED, not migrated -- there is no safe inferred operation
        // for a file that predates this field.
        let vault = test_vault();
        let id = ProposalId::generate();
        let old_format = format!(
            "schema_version: 1\nid: {id}\noriginating_identity: x\nsource: x\ncreated_at: '2026-11-16T00:00:00Z'\nexpires_at: null\nstatus:\n  state: Pending\n"
        );
        vault.save_proposal(&id.to_string(), old_format.as_bytes()).unwrap();
        let result = load_proposal(&vault, &id);
        assert_eq!(result, Err(BrainApiError::UnsupportedOperation));
    }

    #[test]
    fn load_proposal_with_current_schema_version_but_missing_operation_field_returns_corrupt_data() {
        // Defense in depth: even if a hand-edited/corrupted file claims
        // the CURRENT schema version, a missing `operation` key must still
        // fail closed rather than default to any operation.
        let vault = test_vault();
        let id = ProposalId::generate();
        let missing_operation = format!(
            "schema_version: {PROPOSAL_SCHEMA_VERSION}\nid: {id}\noriginating_identity: x\nsource: x\ncreated_at: '2026-11-16T00:00:00Z'\nexpires_at: null\nstatus:\n  state: Pending\n"
        );
        vault.save_proposal(&id.to_string(), missing_operation.as_bytes()).unwrap();
        let result = load_proposal(&vault, &id);
        assert_eq!(result, Err(BrainApiError::CorruptData));
    }

    #[test]
    fn load_proposal_with_malformed_operation_payload_returns_corrupt_data() {
        let vault = test_vault();
        let id = ProposalId::generate();
        let malformed_operation = format!(
            "schema_version: {PROPOSAL_SCHEMA_VERSION}\nid: {id}\noriginating_identity: x\nsource: x\ncreated_at: '2026-11-16T00:00:00Z'\nexpires_at: null\nstatus:\n  state: Pending\noperation:\n  kind: NotARealOperationKind\n"
        );
        vault.save_proposal(&id.to_string(), malformed_operation.as_bytes()).unwrap();
        let result = load_proposal(&vault, &id);
        assert_eq!(result, Err(BrainApiError::CorruptData));
    }

    // --- operation round-trips: all 3 ProposedOperation variants must
    //     survive save -> load exactly ---

    #[test]
    fn save_memory_operation_round_trips_exactly() {
        let vault = test_vault();
        let operation = sample_save_memory_operation("hello world");
        let proposal = proposal_with_operation(operation.clone());
        save_proposal(&vault, &proposal).unwrap();
        let loaded = load_proposal(&vault, &proposal.id).unwrap();
        assert_eq!(loaded.operation, operation);
    }

    #[test]
    fn update_memory_operation_round_trips_exactly() {
        let vault = test_vault();
        let operation = ProposedOperation::UpdateMemory {
            note_id: "note-a".into(),
            body: "new body".into(),
            expected_version: storage::Version(3),
        };
        let proposal = proposal_with_operation(operation.clone());
        save_proposal(&vault, &proposal).unwrap();
        let loaded = load_proposal(&vault, &proposal.id).unwrap();
        assert_eq!(loaded.operation, operation);
    }

    #[test]
    fn load_proposal_on_a_pre_task_14_schema_version_2_file_returns_corrupt_data() {
        // A real version-2 file's `operation.SaveMemory` payload predates
        // note_id/note_type/classification/project/source/event_period --
        // `serde_yaml::from_str::<PersistedProposal>` decodes the WHOLE
        // document (including `operation`) in one call, so this fails at
        // that single deserialize step itself (missing required fields
        // inside the tagged variant) before load_proposal's own explicit
        // schema_version check ever runs. Either way the outcome is the
        // same fail-closed CorruptData a missing `operation` key gets --
        // there is no path through which an incomplete operation payload
        // is silently accepted or defaulted.
        let vault = test_vault();
        let id = ProposalId::generate();
        let v2_format = format!(
            "schema_version: 2\nid: {id}\noriginating_identity: x\nsource: x\ncreated_at: '2026-11-16T00:00:00Z'\nexpires_at: null\nstatus:\n  state: Pending\noperation:\n  kind: SaveMemory\n  body: old-shape-body\n"
        );
        vault.save_proposal(&id.to_string(), v2_format.as_bytes()).unwrap();
        let result = load_proposal(&vault, &id);
        assert_eq!(result, Err(BrainApiError::CorruptData));
    }

    #[test]
    fn link_entities_operation_round_trips_exactly_for_every_relation_kind() {
        let vault = test_vault();
        for relation in [
            RelationKind::Supports,
            RelationKind::Contradicts,
            RelationKind::DerivedFrom,
            RelationKind::Supersedes,
            RelationKind::AppliesTo,
        ] {
            let proposal =
                proposal_with_operation(ProposedOperation::LinkEntities { from: "note-a".into(), to: "note-b".into(), relation });
            save_proposal(&vault, &proposal).unwrap();
            let loaded = load_proposal(&vault, &proposal.id).unwrap();
            assert_eq!(loaded.operation, ProposedOperation::LinkEntities { from: "note-a".into(), to: "note-b".into(), relation });
        }
    }

    #[test]
    fn list_proposals_on_an_empty_vault_returns_empty_not_an_error() {
        let vault = test_vault();
        let proposals = list_proposals(&vault).unwrap();
        assert!(proposals.is_empty());
    }

    #[test]
    fn list_proposals_returns_every_saved_proposal() {
        let vault = test_vault();
        let p1 = sample_proposal();
        let p2 = sample_proposal();
        save_proposal(&vault, &p1).unwrap();
        save_proposal(&vault, &p2).unwrap();
        let mut proposals = list_proposals(&vault).unwrap();
        proposals.sort_by_key(|a| a.id.to_string());
        let mut expected = vec![p1, p2];
        expected.sort_by_key(|a| a.id.to_string());
        assert_eq!(proposals, expected);
    }

    #[test]
    fn list_proposals_skips_a_corrupted_entry_without_failing_the_whole_call() {
        let vault = test_vault();
        let good = sample_proposal();
        save_proposal(&vault, &good).unwrap();
        let corrupted_id = ProposalId::generate();
        vault.save_proposal(&corrupted_id.to_string(), b"not valid yaml [[[").unwrap();
        let proposals = list_proposals(&vault).unwrap();
        assert_eq!(proposals, vec![good]);
    }

    // --- concurrent/duplicate-create semantics: saving twice under the
    //     same id overwrites atomically (matches Vault::save_proposal's
    //     own replace-cleanly semantics), it does not error or corrupt ---

    #[test]
    fn saving_the_same_proposal_id_twice_overwrites_cleanly() {
        let vault = test_vault();
        let mut proposal = sample_proposal();
        save_proposal(&vault, &proposal).unwrap();
        proposal.transition_to(ProposalStatus::Rejected { reason: "r".into(), rejected_at: "2026-11-16T02:00:00Z".into() }).unwrap();
        save_proposal(&vault, &proposal).unwrap();
        let loaded = load_proposal(&vault, &proposal.id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Rejected { reason: "r".into(), rejected_at: "2026-11-16T02:00:00Z".into() });
    }
}
