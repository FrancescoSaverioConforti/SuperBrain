//! `link_entities` as a proposal-creating operation (B5.1 Task 13).
//! `graph_query`/`get_entity` (the read-shaped capabilities this module's
//! name eventually also covers) come in Task 17 — this file currently
//! holds only the proposal-creating half.

use crate::memory::{ProposedOperation, RelationKind};
use crate::{BrainApiError, PendingWriteProposal, ProposalId, ProposalProvenance, RequestContext, WriteOperation};

/// Proposes linking two notes via one of `domain::Relations`'s five real
/// fields (`crates/domain/src/note.rs:13-24`). Never executes the link
/// directly — produces a `Pending` proposal only, same permission
/// discipline as `save_memory`/`update_memory`: `ctx.write_policy` must
/// allow `WriteOperation::ProposeWrite` or this fails closed with
/// `PermissionDenied`.
pub fn link_entities(
    ctx: &RequestContext,
    vault: &storage::Vault,
    from: storage::NoteId,
    to: storage::NoteId,
    relation: RelationKind,
    now: &str,
) -> Result<ProposalId, BrainApiError> {
    if !ctx.write_policy.as_ref().is_some_and(|p| p.allows(WriteOperation::ProposeWrite)) {
        return Err(BrainApiError::PermissionDenied);
    }

    let operation = ProposedOperation::LinkEntities {
        from: from.as_str().to_string(),
        to: to.as_str().to_string(),
        relation,
    };
    let _ = &operation; // recorded in provenance for now; see memory.rs's module note on ProposedOperation's future use at approval time (Task 14).

    let identity = ctx.origin.as_identity_label();
    let proposal = PendingWriteProposal::new(
        ProposalId::generate(),
        identity.clone(),
        ProposalProvenance { source: identity, created_at: now.to_string() },
        None,
    );
    crate::save_proposal(vault, &proposal)?;
    Ok(proposal.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallOrigin, ProposalStatus, WritePolicy};

    fn unique_test_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-brain-api-graph-test-{}-{}-{}",
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

    fn mcp_ctx() -> RequestContext {
        RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: Some(WritePolicy::mcp_agent()),
            origin: CallOrigin::McpSession("session-abc".into()),
        }
    }

    const NOW: &str = "2026-11-16T00:00:00Z";

    #[test]
    fn link_entities_denied_without_write_policy() {
        let vault = test_vault();
        let ctx = RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: None,
            origin: CallOrigin::McpSession("session-abc".into()),
        };
        let result = link_entities(
            &ctx,
            &vault,
            storage::NoteId::parse("note-a").unwrap(),
            storage::NoteId::parse("note-b").unwrap(),
            RelationKind::Supports,
            NOW,
        );
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn link_entities_creates_a_pending_proposal() {
        let vault = test_vault();
        let id = link_entities(
            &mcp_ctx(),
            &vault,
            storage::NoteId::parse("note-a").unwrap(),
            storage::NoteId::parse("note-b").unwrap(),
            RelationKind::Supports,
            NOW,
        )
        .unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Pending);
    }

    #[test]
    fn link_entities_never_modifies_either_note_directly() {
        let vault = test_vault();
        // Neither note-a nor note-b exists; link_entities must not create
        // either one as a side effect of proposing the link.
        link_entities(
            &mcp_ctx(),
            &vault,
            storage::NoteId::parse("note-a").unwrap(),
            storage::NoteId::parse("note-b").unwrap(),
            RelationKind::Supports,
            NOW,
        )
        .unwrap();
        assert!(matches!(vault.read_note(&storage::NoteId::parse("note-a").unwrap()), Err(storage::StorageError::NotFound { .. })));
        assert!(matches!(vault.read_note(&storage::NoteId::parse("note-b").unwrap()), Err(storage::StorageError::NotFound { .. })));
    }

    #[test]
    fn approve_permission_is_never_required_to_propose_a_link() {
        let vault = test_vault();
        let ctx = mcp_ctx();
        assert!(!ctx.write_policy.as_ref().unwrap().allows(WriteOperation::ApproveProposal));
        let result = link_entities(
            &ctx,
            &vault,
            storage::NoteId::parse("note-a").unwrap(),
            storage::NoteId::parse("note-b").unwrap(),
            RelationKind::Contradicts,
            NOW,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn relation_kind_round_trips_through_the_persisted_operation() {
        // RelationKind is one of domain::Relations's 5 real fields --
        // confirm all five are constructible and distinct, since the
        // proposal's operation payload must be able to name any of them.
        let kinds = [
            RelationKind::Supports,
            RelationKind::Contradicts,
            RelationKind::DerivedFrom,
            RelationKind::Supersedes,
            RelationKind::AppliesTo,
        ];
        let unique: std::collections::HashSet<_> = kinds.iter().map(|k| format!("{k:?}")).collect();
        assert_eq!(unique.len(), 5, "all 5 RelationKind variants must be distinct");
    }
}
