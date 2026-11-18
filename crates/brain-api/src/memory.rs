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
///
/// Field sets are pinned to the B5.0 frozen contract
/// (`docs/superpowers/specs/2026-11-15-b5-0-brain-api-contract-freeze.md`,
/// "10–12. save_memory / update_memory / link_entities"), corrected to
/// match it exactly (a Task 13 draft under-specified both variants; fixed
/// as a Task 14 prerequisite, before any execution code could be written
/// against the incomplete shape):
///
/// - `SaveMemory` needs every field required to build a brand-new, valid
///   note (`domain::NoteMetadata`'s 12 frontmatter fields minus the 2 that
///   are always system-generated at execution time, `created`/`updated` —
///   see `created`/`updated`'s own doc note below). `note_id`, `note_type`,
///   `classification`, `project`, `source`, `event_period` are all
///   caller-supplied INTENT, persisted on the proposal exactly as given
///   and never altered or defaulted by `approve_proposal`.
/// - `UpdateMemory` needs `expected_version` (optimistic concurrency,
///   mirroring `storage::propose_and_commit`'s own `expected_version`
///   parameter): without it, `approve_proposal` would have no way to
///   detect that the target note changed between proposal creation and
///   approval, and could silently overwrite an intervening edit.
/// - `LinkEntities` does NOT carry `expected_version` — the B5.0 contract's
///   own input list for it names only "source/target identifiers,
///   relation kind... originating_identity", no concurrency parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposedOperation {
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
        expected_version: storage::Version,
    },
    LinkEntities {
        from: String,
        to: String,
        relation: RelationKind,
    },
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
    if !ctx.write_policy.as_ref().is_some_and(|p| p.allows(WriteOperation::ProposeWrite)) {
        return Err(BrainApiError::PermissionDenied);
    }

    let identity = ctx.origin.as_identity_label();
    let proposal = PendingWriteProposal::new(
        ProposalId::generate(),
        identity.clone(),
        ProposalProvenance { source: identity, created_at: now.to_string() },
        None, // expiry: no fixed default assigned yet — see module note below.
        operation,
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
///
/// Every parameter here is caller-supplied INTENT for the note
/// `approve_proposal` will later create — `note_id`/`note_type`/
/// `classification`/`project`/`source`/`event_period`/`body` are persisted
/// on the proposal exactly as given (see `ProposedOperation::SaveMemory`'s
/// own doc comment for why). `created`/`updated` are deliberately NOT
/// parameters: they are system-generated at execution time from whatever
/// timestamp `approve_proposal` is called with, never caller-controlled.
#[allow(clippy::too_many_arguments)]
pub fn save_memory(
    ctx: &RequestContext,
    vault: &storage::Vault,
    note_id: storage::NoteId,
    note_type: domain::NoteType,
    classification: domain::Classification,
    project: String,
    source: String,
    event_period: String,
    body: String,
    now: &str,
) -> Result<ProposalId, BrainApiError> {
    create_proposal(
        ctx,
        vault,
        ProposedOperation::SaveMemory {
            note_id: note_id.as_str().to_string(),
            note_type,
            classification,
            project,
            source,
            event_period,
            body,
        },
        now,
    )
}

/// Proposes replacing an existing note's content. Same permission/
/// proposal-only discipline as `save_memory`. `expected_version` is the
/// optimistic-concurrency version the caller observed when proposing this
/// update; `approve_proposal` re-checks it against the note's REAL current
/// version at approval time (never trusting that nothing changed in
/// between) and fails closed with `Conflict` on a mismatch, same as
/// `storage::propose_and_commit`'s own `expected_version` parameter.
pub fn update_memory(
    ctx: &RequestContext,
    vault: &storage::Vault,
    note_id: storage::NoteId,
    body: String,
    expected_version: storage::Version,
    now: &str,
) -> Result<ProposalId, BrainApiError> {
    create_proposal(ctx, vault, ProposedOperation::UpdateMemory { note_id: note_id.as_str().to_string(), body, expected_version }, now)
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

    /// Calls `save_memory` with a fixed, valid set of intent fields except
    /// `body`/`note_id`, which the caller still names — kept distinct per
    /// call in most tests to avoid colliding on the one real constraint
    /// `save_memory` doesn't check (two proposals naming the same
    /// `note_id` are not deduplicated; `approve_proposal` is what would
    /// eventually reject a collision at execution time, out of scope here).
    #[allow(clippy::too_many_arguments)]
    fn save_memory_with(
        ctx: &RequestContext,
        vault: &storage::Vault,
        note_id: &str,
        project: &str,
        source: &str,
        event_period: &str,
        body: &str,
    ) -> Result<ProposalId, BrainApiError> {
        save_memory(
            ctx,
            vault,
            storage::NoteId::parse(note_id).unwrap(),
            domain::NoteType::Bug,
            domain::Classification::Internal,
            project.to_string(),
            source.to_string(),
            event_period.to_string(),
            body.to_string(),
            NOW,
        )
    }

    fn save_memory_default(ctx: &RequestContext, vault: &storage::Vault, note_id: &str, body: &str) -> Result<ProposalId, BrainApiError> {
        save_memory_with(ctx, vault, note_id, "project-x", "agent:coder@prompt-v3", "2026-11-16", body)
    }

    // --- save_memory ---

    #[test]
    fn save_memory_denied_without_write_policy() {
        let vault = test_vault();
        let result = save_memory_default(&no_policy_ctx(), &vault, "note-a", "content");
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn save_memory_denied_with_write_policy_none() {
        let vault = test_vault();
        let mut ctx = mcp_ctx();
        ctx.write_policy = Some(WritePolicy::none());
        let result = save_memory_default(&ctx, &vault, "note-a", "content");
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn save_memory_creates_a_pending_proposal_with_propose_write_permission() {
        let vault = test_vault();
        let id = save_memory_default(&mcp_ctx(), &vault, "note-a", "content").unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Pending);
    }

    #[test]
    fn save_memory_never_writes_a_note_directly() {
        // The only way bytes land under a real note id is through
        // storage::propose_and_commit (A1's invariant: the sole real write
        // authority in the whole storage crate), which save_memory never
        // calls -- it only builds a ProposedOperation and persists it as a
        // Pending proposal. Observable proof: the exact note_id this call
        // names is not readable afterwards.
        let vault = test_vault();
        save_memory_default(&mcp_ctx(), &vault, "note-a", "content").unwrap();
        let probe_id = storage::NoteId::parse("note-a").unwrap();
        let result = vault.read_note(&probe_id);
        assert!(matches!(result, Err(storage::StorageError::NotFound { .. })), "save_memory must never create any note, got {result:?}");
    }

    #[test]
    fn save_memory_originating_identity_comes_from_context_not_a_parameter() {
        // There is no originating_identity parameter on save_memory's
        // signature for a caller to set. The identity persisted on the
        // proposal must match ctx.origin's own label.
        let vault = test_vault();
        let ctx = mcp_ctx();
        let id = save_memory_default(&ctx, &vault, "note-a", "content").unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(loaded.originating_identity, ctx.origin.as_identity_label());
        assert_eq!(loaded.originating_identity, "session-abc");
    }

    #[test]
    fn save_memory_source_field_never_influences_originating_identity() {
        // `source` (a frontmatter metadata field on the proposed note) and
        // `originating_identity` (the proposal's own trust-boundary field)
        // are deliberately independent: passing a `source` string that
        // even LOOKS like a different identity must not change
        // `originating_identity`, which only ever comes from `ctx.origin`.
        let vault = test_vault();
        let ctx = mcp_ctx();
        let id = save_memory_with(&ctx, &vault, "note-a", "project-x", "trusted-local", "2026-11-16", "content").unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(loaded.originating_identity, "session-abc");
        match loaded.operation {
            ProposedOperation::SaveMemory { source, .. } => assert_eq!(source, "trusted-local"),
            other => panic!("expected SaveMemory, got {other:?}"),
        }
    }

    #[test]
    fn two_contexts_with_different_origins_produce_different_provenance() {
        let vault = test_vault();
        let id_a = save_memory_default(&mcp_ctx(), &vault, "note-a", "content").unwrap();
        let mut other_ctx = mcp_ctx();
        other_ctx.origin = CallOrigin::McpSession("session-xyz".into());
        let id_b = save_memory_default(&other_ctx, &vault, "note-b", "content").unwrap();
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
        let id = save_memory_default(&ctx, &vault, "note-a", "content").unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(loaded.originating_identity, "trusted-local");
    }

    #[test]
    fn save_memory_proposal_has_a_server_generated_valid_proposal_id() {
        let vault = test_vault();
        let id = save_memory_default(&mcp_ctx(), &vault, "note-a", "content").unwrap();
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
        assert!(save_memory_default(&ctx, &vault, "note-a", "content").is_ok());
    }

    #[test]
    fn save_memory_persists_through_the_proposals_confined_api() {
        let vault = test_vault();
        let id = save_memory_default(&mcp_ctx(), &vault, "note-a", "content").unwrap();
        // list_proposal_ids is storage's own confined discovery API (Task
        // 12) -- confirms the proposal is reachable through it, not
        // through some separate, undiscovered path.
        let ids = vault.list_proposal_ids().unwrap();
        assert!(ids.contains(&id.to_string()));
    }

    #[test]
    fn calling_save_memory_twice_creates_two_independent_pending_proposals() {
        let vault = test_vault();
        let id1 = save_memory_default(&mcp_ctx(), &vault, "note-a", "first").unwrap();
        let id2 = save_memory_default(&mcp_ctx(), &vault, "note-b", "second").unwrap();
        assert_ne!(id1, id2);
        let ids = vault.list_proposal_ids().unwrap();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn save_memory_persists_note_id_note_type_classification_project_source_event_period_exactly() {
        let vault = test_vault();
        let id = save_memory(
            &mcp_ctx(),
            &vault,
            storage::NoteId::parse("note-z").unwrap(),
            domain::NoteType::Decision,
            domain::Classification::Public,
            "project-y".to_string(),
            "agent:other@prompt-v9".to_string(),
            "2026-11-15".to_string(),
            "the body".to_string(),
            NOW,
        )
        .unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        match loaded.operation {
            ProposedOperation::SaveMemory { note_id, note_type, classification, project, source, event_period, body } => {
                assert_eq!(note_id, "note-z");
                assert_eq!(note_type, domain::NoteType::Decision);
                assert_eq!(classification, domain::Classification::Public);
                assert_eq!(project, "project-y");
                assert_eq!(source, "agent:other@prompt-v9");
                assert_eq!(event_period, "2026-11-15");
                assert_eq!(body, "the body");
            }
            other => panic!("expected SaveMemory, got {other:?}"),
        }
    }

    // --- update_memory ---

    #[test]
    fn update_memory_denied_without_write_policy() {
        let vault = test_vault();
        let note_id = storage::NoteId::parse("note-a").unwrap();
        let result = update_memory(&no_policy_ctx(), &vault, note_id, "new content".into(), storage::Version(1), NOW);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn update_memory_creates_a_pending_proposal() {
        let vault = test_vault();
        let note_id = storage::NoteId::parse("note-a").unwrap();
        let id = update_memory(&mcp_ctx(), &vault, note_id, "new content".into(), storage::Version(1), NOW).unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Pending);
    }

    #[test]
    fn update_memory_persists_the_expected_version_exactly() {
        let vault = test_vault();
        let note_id = storage::NoteId::parse("note-a").unwrap();
        let id = update_memory(&mcp_ctx(), &vault, note_id, "new content".into(), storage::Version(7), NOW).unwrap();
        let loaded = crate::load_proposal(&vault, &id).unwrap();
        match loaded.operation {
            ProposedOperation::UpdateMemory { expected_version, .. } => assert_eq!(expected_version, storage::Version(7)),
            other => panic!("expected UpdateMemory, got {other:?}"),
        }
    }

    #[test]
    fn update_memory_never_modifies_the_target_note_directly() {
        let vault = test_vault();
        // Write a real note first via the ONE real write authority in
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
        update_memory(&mcp_ctx(), &vault, note_id.clone(), "proposed new content".into(), storage::Version(1), NOW).unwrap();
        let after = vault.read_note(&note_id).unwrap();
        assert_eq!(original, after, "update_memory must never modify the note directly -- only propose");
    }

    fn valid_note_yaml(id: &str) -> String {
        format!(
            "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-16\nupdated: 2026-11-16\nevent_period: 2026-11-16\nrelations: {{}}\n---\noriginal content\n"
        )
    }
}
