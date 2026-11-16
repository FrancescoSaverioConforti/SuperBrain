//! `.proposals/` persistence (B5.1 Task 12): serializes/deserializes a
//! [`PendingWriteProposal`] and persists it through `storage::Vault`'s own
//! confined, validated proposal API (`save_proposal`/`load_proposal`/
//! `list_proposal_ids`) — never a general filesystem write primitive.
//! `storage` owns every filesystem-safety guarantee (B5.SEC Approach A);
//! this module never touches a raw handle, a path, or any
//! `handle_safety`-aware type directly.

use crate::{BrainApiError, PendingWriteProposal, ProposalId, ProposalProvenance, ProposalStatus};

/// The on-disk shape of a persisted proposal. A deliberately separate,
/// fully-public "shadow" struct — not `PendingWriteProposal` itself, whose
/// `status` field is private by design (its only mutation path is
/// `transition_to`, never a public setter, never serde's derive-generated
/// field-by-field construction). Serializing/deserializing through this
/// shadow type means persistence can read and write `status` without the
/// runtime `PendingWriteProposal` type ever gaining a public way to set it
/// directly.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedProposal {
    schema_version: u32,
    id: String,
    originating_identity: String,
    source: String,
    created_at: String,
    expires_at: Option<String>,
    status: PersistedStatus,
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
/// read. A future incompatible change bumps `CURRENT` and adds an explicit
/// migration or rejection for older versions — never a silent
/// reinterpretation of old bytes under a new shape.
const PROPOSAL_SCHEMA_VERSION: u32 = 1;

fn to_persisted(proposal: &PendingWriteProposal) -> PersistedProposal {
    PersistedProposal {
        schema_version: PROPOSAL_SCHEMA_VERSION,
        id: proposal.id.to_string(),
        originating_identity: proposal.originating_identity.clone(),
        source: proposal.provenance.source.clone(),
        created_at: proposal.provenance.created_at.clone(),
        expires_at: proposal.expires_at.clone(),
        status: PersistedStatus::from(proposal.status()),
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
fn from_persisted(persisted: PersistedProposal) -> Result<PendingWriteProposal, BrainApiError> {
    let id = ProposalId::parse(&persisted.id).map_err(|_| BrainApiError::CorruptData)?;
    let mut proposal = PendingWriteProposal::new(
        id,
        persisted.originating_identity,
        ProposalProvenance { source: persisted.source, created_at: persisted.created_at },
        persisted.expires_at,
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
pub fn save_proposal(vault: &storage::Vault, proposal: &PendingWriteProposal) -> Result<(), BrainApiError> {
    let persisted = to_persisted(proposal);
    let bytes = serde_yaml::to_string(&persisted).map_err(|_| BrainApiError::RuntimeUnavailable)?;
    vault.save_proposal(&proposal.id.to_string(), bytes.as_bytes()).map_err(map_storage_error)
}

/// Loads and reconstructs the proposal persisted under `id`.
pub fn load_proposal(vault: &storage::Vault, id: &ProposalId) -> Result<PendingWriteProposal, BrainApiError> {
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
pub fn list_proposals(vault: &storage::Vault) -> Result<Vec<PendingWriteProposal>, BrainApiError> {
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

    fn sample_proposal() -> PendingWriteProposal {
        PendingWriteProposal::new(
            ProposalId::generate(),
            "mcp-session-abc".into(),
            ProposalProvenance { source: "mcp-session-abc".into(), created_at: "2026-11-16T00:00:00Z".into() },
            Some("2026-11-17T00:00:00Z".into()),
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
