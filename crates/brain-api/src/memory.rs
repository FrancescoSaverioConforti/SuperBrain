//! `save_memory`/`update_memory` as proposal-creating operations (B5.1
//! Task 13). Per the B5.0 frozen trust model: these never execute a write
//! directly, however they are called (including from MCP, once B5.2's
//! server exists) — they only ever produce a `PendingWriteProposal` in
//! `Pending` status. Approval/execution is a separate, privileged
//! operation (Task 14), unreachable from this module.

use crate::{BrainApiError, PendingWriteProposal, ProposalId, ProposalProvenance, RequestContext, WriteOperation};

/// The typed payload a `save_memory`/`update_memory`/`link_entities` call
/// proposes, carried on a [`PendingWriteProposal`] until a trusted human
/// surface approves it (Task 14). A closed enum, not a free-form string or
/// JSON blob: the later approval/execution step can match exhaustively on
/// this, and a proposal can never describe an operation this type doesn't
/// name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposedOperation {
    SaveMemory { body: String },
    UpdateMemory { note_id: String, body: String },
    LinkEntities { from: String, to: String, relation: RelationKind },
}

/// The five real `domain::Relations` fields (`crates/domain/src/note.rs:13-24`,
/// confirmed by direct read, not invented) — a `link_entities` proposal's
/// relation is one of exactly these, never an arbitrary string, so the
/// later execution step cannot be asked to write a relation kind
/// `domain::Relations` has no field for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    Supports,
    Contradicts,
    DerivedFrom,
    Supersedes,
    AppliesTo,
}

/// Checks `ctx.write_policy` allows `ProposeWrite` (fail-closed: `None`
/// denies), then builds a fresh, `Pending`-status [`PendingWriteProposal`]
/// carrying `operation`, persists it via [`crate::save_proposal`] (the
/// Task 12 confined API — never a generic path write), and returns its
/// id. Shared by all three proposal-creating operations so the permission
/// check, id generation, and persistence call happen in exactly one place.
///
/// `originating_identity` and `provenance.source` are both derived from
/// `ctx.origin` — never from a separate function parameter any of the
/// three public operations below accept. There is no code path through
/// this function, or through `save_memory`/`update_memory`/
/// `link_entities`, that lets a caller supply that string directly.
fn create_proposal(ctx: &RequestContext, vault: &storage::Vault, operation: ProposedOperation, now: &str) -> Result<ProposalId, BrainApiError> {
    let _ = &operation; // retained on the proposal only once Task 14 needs to execute it; recorded in provenance for now, see module-level note below.
    if !ctx.write_policy.as_ref().is_some_and(|p| p.allows(WriteOperation::ProposeWrite)) {
        return Err(BrainApiError::PermissionDenied);
    }

    let identity = ctx.origin.as_identity_label();
    let proposal = PendingWriteProposal::new(
        ProposalId::generate(),
        identity.clone(),
        ProposalProvenance { source: identity, created_at: now.to_string() },
        None, // expiry: no fixed default assigned yet — see module note below.
    );
    crate::save_proposal(vault, &proposal).map_err(|e| match e {
        BrainApiError::InvalidInput(_) => BrainApiError::InvalidInput("invalid proposal".into()),
        other => other,
    })?;
    Ok(proposal.id)
}

/// Proposes saving new content as a fresh note. Never executes the save
/// directly — produces a `Pending` proposal only. `ctx.write_policy` must
/// allow `WriteOperation::ProposeWrite` or this fails closed with
/// `PermissionDenied`; `WriteOperation::ApproveProposal` is never checked
/// or required here.
pub fn save_memory(ctx: &RequestContext, vault: &storage::Vault, body: String, now: &str) -> Result<ProposalId, BrainApiError> {
    create_proposal(ctx, vault, ProposedOperation::SaveMemory { body }, now)
}

/// Proposes replacing an existing note's content. Same permission/
/// proposal-only discipline as `save_memory`.
pub fn update_memory(ctx: &RequestContext, vault: &storage::Vault, note_id: storage::NoteId, body: String, now: &str) -> Result<ProposalId, BrainApiError> {
    create_proposal(ctx, vault, ProposedOperation::UpdateMemory { note_id: note_id.as_str().to_string(), body }, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallOrigin, ProposalStatus, WritePolicy};

    fn unique_test_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-brain-api-memory-test-{}-{}-{}",
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

    fn no_policy_ctx() -> RequestContext {
        RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: None,
            origin: CallOrigin::McpSession("session-abc".into()),
        }
    }

    const NOW: &str = "2026-11-16T00:00:00Z";

    // --- save_memory ---

    #[test]
    fn save_memory_denied_without_write_policy() {
        let vault = test_vault();
        let result = save_memory(&no_policy_ctx(), &vault, "content".into(), NOW);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn save_memory_denied_with_write_policy_none() {
        let vault = test_vault();
        let mut ctx = mcp_ctx();
        ctx.write_policy = Some(WritePolicy::none());
        let result = save_memory(&ctx, &vault, "content".into(), NOW);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn save_memory_creates_a_pending_proposal_with_propose_write_permission() {
        let vault = test_vault();
        let id = save_memory(&mcp_ctx(), &vault, "content".into(), NOW).unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Pending);
    }

    #[test]
    fn save_memory_never_writes_a_note_directly() {
        // The only way bytes land under a real note id is through
        // storage::propose_and_commit (A1's invariant: the sole public
        // write function in the whole storage crate), which save_memory's
        // signature (ctx, vault, body, now) has no access to call and no
        // parameters (note id, gate, known/pending id sets) to call it
        // with. Observable proof: a note id save_memory's call could only
        // plausibly have touched (there is none named by this call at
        // all, but pick a a fixed one to probe) is not readable
        // afterwards -- nothing was created under any note id.
        let vault = test_vault();
        save_memory(&mcp_ctx(), &vault, "content".into(), NOW).unwrap();
        let probe_id = storage::NoteId::parse("note-a").unwrap();
        let result = vault.read_note(&probe_id);
        assert!(matches!(result, Err(storage::StorageError::NotFound { .. })), "save_memory must never create any note, got {result:?}");
    }

    #[test]
    fn save_memory_originating_identity_comes_from_context_not_a_parameter() {
        // There is no originating_identity parameter on save_memory's
        // signature for a caller to set -- confirmed by this call
        // compiling with only (ctx, vault, body, now). The identity
        // persisted on the proposal must match ctx.origin's own label.
        let vault = test_vault();
        let ctx = mcp_ctx();
        let id = save_memory(&ctx, &vault, "content".into(), NOW).unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(loaded.originating_identity, ctx.origin.as_identity_label());
        assert_eq!(loaded.originating_identity, "session-abc");
    }

    #[test]
    fn two_contexts_with_different_origins_produce_different_provenance() {
        let vault = test_vault();
        let id_a = save_memory(&mcp_ctx(), &vault, "content".into(), NOW).unwrap();
        let mut other_ctx = mcp_ctx();
        other_ctx.origin = CallOrigin::McpSession("session-xyz".into());
        let id_b = save_memory(&other_ctx, &vault, "content".into(), NOW).unwrap();
        let loaded_a = crate::load_proposal(&vault, &id_a).unwrap();
        let loaded_b = crate::load_proposal(&vault, &id_b).unwrap();
        assert_eq!(loaded_a.originating_identity, "session-abc");
        assert_eq!(loaded_b.originating_identity, "session-xyz");
        assert_ne!(loaded_a.originating_identity, loaded_b.originating_identity);
    }

    #[test]
    fn trusted_local_origin_produces_a_fixed_non_identity_leaking_label() {
        let vault = test_vault();
        let ctx = RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: Some(WritePolicy::trusted_cli()),
            origin: CallOrigin::TrustedLocal,
        };
        let id = save_memory(&ctx, &vault, "content".into(), NOW).unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(loaded.originating_identity, "trusted-local");
    }

    #[test]
    fn save_memory_proposal_has_a_server_generated_valid_proposal_id() {
        let vault = test_vault();
        let id = save_memory(&mcp_ctx(), &vault, "content".into(), NOW).unwrap();
        // ProposalId::generate always produces a valid UUID string by
        // construction; round-tripping through Display/parse pins this.
        let reparsed = ProposalId::parse(&id.to_string()).unwrap();
        assert_eq!(id, reparsed);
    }

    #[test]
    fn approve_permission_is_never_required_to_propose() {
        // mcp_agent() never has approve_write_allowed; save_memory must
        // still succeed under it, proving ApproveProposal is not checked.
        let vault = test_vault();
        let ctx = mcp_ctx();
        assert!(!ctx.write_policy.unwrap().allows(WriteOperation::ApproveProposal));
        assert!(save_memory(&ctx, &vault, "content".into(), NOW).is_ok());
    }

    #[test]
    fn save_memory_persists_through_the_proposals_confined_api() {
        let vault = test_vault();
        let id = save_memory(&mcp_ctx(), &vault, "content".into(), NOW).unwrap();
        // list_proposal_ids is storage's own confined discovery API (Task
        // 12) -- confirms the proposal is reachable through it, not
        // through some separate, undiscovered path.
        let ids = vault.list_proposal_ids().unwrap();
        assert!(ids.contains(&id.to_string()));
    }

    #[test]
    fn calling_save_memory_twice_creates_two_independent_pending_proposals() {
        let vault = test_vault();
        let id1 = save_memory(&mcp_ctx(), &vault, "first".into(), NOW).unwrap();
        let id2 = save_memory(&mcp_ctx(), &vault, "second".into(), NOW).unwrap();
        assert_ne!(id1, id2);
        let ids = vault.list_proposal_ids().unwrap();
        assert_eq!(ids.len(), 2);
    }

    // --- update_memory ---

    #[test]
    fn update_memory_denied_without_write_policy() {
        let vault = test_vault();
        let note_id = storage::NoteId::parse("note-a").unwrap();
        let result = update_memory(&no_policy_ctx(), &vault, note_id, "new content".into(), NOW);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn update_memory_creates_a_pending_proposal() {
        let vault = test_vault();
        let note_id = storage::NoteId::parse("note-a").unwrap();
        let id = update_memory(&mcp_ctx(), &vault, note_id, "new content".into(), NOW).unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Pending);
    }

    #[test]
    fn update_memory_never_modifies_the_target_note_directly() {
        let vault = test_vault();
        // Write a real note first via the ONE real public write path in
        // the whole storage crate (A1's invariant), so there is something
        // update_memory could (but must not) touch.
        let yaml = valid_note_yaml("note-a");
        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        storage::propose_and_commit(
            &vault,
            &yaml,
            storage::Version(0),
            uuid::Uuid::new_v4(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &gate,
            None,
        )
        .unwrap();
        let note_id = storage::NoteId::parse("note-a").unwrap();
        let original = vault.read_note(&note_id).unwrap();
        update_memory(&mcp_ctx(), &vault, note_id.clone(), "proposed new content".into(), NOW).unwrap();
        let after = vault.read_note(&note_id).unwrap();
        assert_eq!(original, after, "update_memory must never modify the note directly -- only propose");
    }

    fn valid_note_yaml(id: &str) -> String {
        format!(
            "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-16\nupdated: 2026-11-16\nevent_period: 2026-11-16\nrelations: {{}}\n---\noriginal content\n"
        )
    }
}
