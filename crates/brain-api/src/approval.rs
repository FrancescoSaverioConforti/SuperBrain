//! `approve_proposal`/`reject_proposal`/`list_proposals` (B5.1 Task 14):
//! the only place a `PendingWriteProposal` can ever actually be executed.
//! Per the B5.0 frozen trust model, this is CLI-only today
//! (`WritePolicy::trusted_cli()`-gated) — no MCP-originated `RequestContext`
//! can ever reach `WriteOperation::ApproveProposal`/`RejectProposal`
//! (`policy_write.rs`'s module doc: there is no public constructor that can
//! produce `approve_write_allowed: true` other than `trusted_cli()`).
//!
//! Core trust invariant: `approve_proposal` executes EXACTLY the
//! `ProposedOperation` persisted on the proposal at creation time — never a
//! caller-supplied override. Its own public signature carries no
//! operation/payload parameter for that reason; the only inputs are the
//! proposal's id and the current time.

use crate::{BrainApiError, PendingWriteProposal, ProposalId, ProposedOperation, RequestContext, WriteOperation};

/// Authorizes `op` (fail-closed on a missing/insufficient `WritePolicy`),
/// then loads the proposal and re-checks expiry against `now` regardless
/// of its persisted status — the defense-in-depth rule the design doc
/// requires: a proposal's `status` field alone is never trusted to reflect
/// expiry; `now >= expires_at` always re-fails closed even if nothing
/// ever ran `expire` on it.
///
/// Permission is checked BEFORE the load (not after), so an unauthorized
/// caller gets `PermissionDenied` without ever learning whether `id`
/// exists — `load_proposal`'s own `NotFound` is only reachable once
/// authorization already passed.
fn authorize_and_load(
    ctx: &RequestContext,
    vault: &storage::Vault,
    id: &ProposalId,
    op: WriteOperation,
    now: &str,
) -> Result<PendingWriteProposal, BrainApiError> {
    if !ctx.write_policy.as_ref().is_some_and(|p| p.allows(op)) {
        return Err(BrainApiError::PermissionDenied);
    }
    let proposal = crate::proposal_store::load_proposal(vault, id)?;
    if is_expired(&proposal, now) {
        return Err(BrainApiError::InvalidInput("proposal expired".to_string()));
    }
    Ok(proposal)
}

/// String-lexicographic comparison is correct here because every
/// `expires_at`/`now` this crate ever persists or is called with is an
/// RFC 3339 / ISO 8601 UTC timestamp with a fixed-width date-time
/// representation (`crate::SCHEMA_VERSION`'s whole timestamp convention,
/// already used identically by `ProposalProvenance::created_at` and every
/// `now: &str` parameter in this crate) — lexicographic and chronological
/// order coincide for that format. `expires_at: None` means "never
/// expires".
fn is_expired(proposal: &PendingWriteProposal, now: &str) -> bool {
    proposal.expires_at.as_deref().is_some_and(|expires_at| now >= expires_at)
}

/// Executes exactly the proposal's persisted `operation`, through the
/// storage-layer typed facades (`storage::propose_and_commit` directly for
/// `SaveMemory`, `storage::update_note_body`/`storage::add_relation` for
/// `UpdateMemory`/`LinkEntities`) — never a raw write primitive, never a
/// second path around A1's one real write authority. `now` supplies the
/// system-generated `created`/`updated` fields for a brand-new `SaveMemory`
/// note (`updated = created` at creation, per the B5.0 freeze's own
/// documented convention) — the ONLY place those two fields are decided;
/// they are never part of the persisted `operation` itself.
fn execute(vault: &storage::Vault, operation: &ProposedOperation, now: &str) -> Result<storage::CommitResult, BrainApiError> {
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    match operation {
        ProposedOperation::SaveMemory { note_id, note_type, classification, project, source, event_period, body } => {
            let note_text = build_new_note_text(note_id, *note_type, *classification, project, source, event_period, now, body);
            storage::propose_and_commit(
                vault,
                &note_text,
                storage::Version(0),
                uuid::Uuid::new_v4(),
                &std::collections::HashSet::new(),
                &std::collections::HashSet::new(),
                &gate,
                None,
            )
            .map_err(map_propose_error)
        }
        ProposedOperation::UpdateMemory { note_id, body, expected_version } => {
            let id = storage::NoteId::parse(note_id).map_err(|_| BrainApiError::CorruptData)?;
            storage::update_note_body(
                vault,
                &id,
                *expected_version,
                Some(body.as_str()),
                uuid::Uuid::new_v4(),
                &std::collections::HashSet::new(),
                &std::collections::HashSet::new(),
                &gate,
                None,
            )
            .map_err(map_propose_error)
        }
        ProposedOperation::LinkEntities { from, to, relation } => {
            let from_id = storage::NoteId::parse(from).map_err(|_| BrainApiError::CorruptData)?;
            let relation_field = match relation {
                crate::RelationKind::Supports => storage::RelationField::Supports,
                crate::RelationKind::Contradicts => storage::RelationField::Contradicts,
                crate::RelationKind::DerivedFrom => storage::RelationField::DerivedFrom,
                crate::RelationKind::Supersedes => storage::RelationField::Supersedes,
                crate::RelationKind::AppliesTo => storage::RelationField::AppliesTo,
            };
            // link_entities carries no expected_version (B5.0's own input
            // list for it never included one); Version(0) here would be
            // wrong for an existing note, so this reads the note's REAL
            // current version immediately before applying the relation --
            // append-only, so a concurrent unrelated edit landing between
            // this read and add_relation's own internal write is still
            // caught by add_relation's real optimistic-concurrency check
            // (same note, same lock), just never silently widened into a
            // version this caller never observed.
            let current = vault.read_note(&from_id).map_err(map_storage_error_for_read)?;
            storage::add_relation(
                vault,
                &from_id,
                current.version,
                relation_field,
                to,
                uuid::Uuid::new_v4(),
                &std::collections::HashSet::new(),
                &[to.clone()].into_iter().collect(),
                &gate,
                None,
            )
            .map_err(map_propose_error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_new_note_text(
    note_id: &str,
    note_type: domain::NoteType,
    classification: domain::Classification,
    project: &str,
    source: &str,
    event_period: &str,
    now: &str,
    body: &str,
) -> String {
    let metadata = domain::NoteMetadata {
        id: note_id.to_string(),
        schema_version: 1,
        r#type: note_type,
        status: domain::NoteStatus::Draft,
        trust: domain::TrustLevel::Internal,
        classification,
        project: project.to_string(),
        source: source.to_string(),
        created: now.to_string(),
        updated: now.to_string(),
        event_period: event_period.to_string(),
        relations: domain::Relations::default(),
    };
    let frontmatter = domain::render_frontmatter(&metadata);
    format!("---\n{frontmatter}---\n{body}")
}

fn map_propose_error(err: storage::ProposeError) -> BrainApiError {
    match err {
        storage::ProposeError::Storage(storage_err) => map_storage_error_for_write(storage_err),
        storage::ProposeError::Schema(_) | storage::ProposeError::InvalidNoteId { .. } | storage::ProposeError::Link(_) => {
            BrainApiError::InvalidInput("the persisted operation no longer produces a valid note".to_string())
        }
        storage::ProposeError::Rejected(_) => BrainApiError::InvalidInput("the persisted operation was rejected by policy".to_string()),
    }
}

fn map_storage_error_for_write(err: storage::StorageError) -> BrainApiError {
    match err {
        storage::StorageError::Conflict { .. } => BrainApiError::Conflict,
        storage::StorageError::NotFound { .. } => BrainApiError::NotFound,
        storage::StorageError::PathEscapesVaultRoot { .. } => BrainApiError::InvalidInput("invalid note id".to_string()),
        storage::StorageError::Io(_) | storage::StorageError::Journal(_) => BrainApiError::RuntimeUnavailable,
        _ => BrainApiError::CorruptData,
    }
}

fn map_storage_error_for_read(err: storage::StorageError) -> BrainApiError {
    match err {
        storage::StorageError::NotFound { .. } => BrainApiError::NotFound,
        storage::StorageError::Io(_) | storage::StorageError::Journal(_) => BrainApiError::RuntimeUnavailable,
        _ => BrainApiError::CorruptData,
    }
}

/// Approves `id`: authorizes (`WriteOperation::ApproveProposal`), loads,
/// re-checks expiry, validates the `Pending -> Approved` transition via
/// `valid_proposal_transition` (an already-`Approved`/`Rejected`/`Expired`
/// proposal fails closed with `Conflict` — never re-executed), executes
/// the persisted `operation` for real, and ONLY on a successful commit
/// persists the terminal `Approved { approved_at: now }` status. If
/// execution fails, the proposal stays `Pending` (or, on the race case
/// below, whatever a concurrent caller already moved it to) — there is no
/// partial "approved but not executed" state this function can produce.
///
/// `ctx.write_policy` must allow `WriteOperation::ApproveProposal` or this
/// fails closed with `PermissionDenied` before the proposal is even
/// loaded — an MCP-originated `RequestContext` (`write_policy:
/// Some(WritePolicy::mcp_agent())`, which never has `approve_write_allowed`)
/// can never reach this far, exactly as `WritePolicy::mcp_agent`'s own
/// doc comment states.
///
/// **Exactly-once execution under concurrent double-approve**, without any
/// additional locking in this crate: two concurrent `approve_proposal`
/// calls on the SAME `id` both pass the `Pending -> Approved` transition
/// check above (both still see `Pending` — there is a real race window
/// here, between this check and either call's own `execute`), but both
/// then call `execute`, which for every one of the 3 operations reaches
/// `storage`'s per-note lock + real `expected_version` check
/// (`write_record_impl` for `SaveMemory`'s `Version(0)`/`UpdateMemory`'s
/// caller-supplied version, `add_relation`'s freshly-read-then-checked
/// version for `LinkEntities`): only ONE of the two concurrent `execute`
/// calls can win that check; the other receives a real `StorageError::Conflict`,
/// mapped to `BrainApiError::Conflict` here, and returns BEFORE ever
/// calling `transition_to`/`save_proposal`. So exactly one caller reaches
/// the `transition_to(Approved)` + `save_proposal` below, and the proposal
/// ends up `Approved` exactly once, with the loser receiving an honest
/// `Conflict` rather than a silent no-op or a double execution. This
/// relies on `storage`'s own concurrency guarantee, never re-implements
/// it — no new lock was added to this crate, per the plan's own
/// suggestion to use an existing local synchronization primitive only if
/// actually needed; it was not needed here.
pub fn approve_proposal(ctx: &RequestContext, vault: &storage::Vault, id: &ProposalId, now: &str) -> Result<storage::CommitResult, BrainApiError> {
    let mut proposal = authorize_and_load(ctx, vault, id, WriteOperation::ApproveProposal, now)?;

    let target = crate::ProposalStatus::Approved { approved_at: now.to_string() };
    if !crate::valid_proposal_transition(proposal.status(), &target) {
        return Err(BrainApiError::Conflict);
    }

    let result = execute(vault, &proposal.operation, now)?;

    proposal.transition_to(target).map_err(|_| BrainApiError::Conflict)?;
    crate::proposal_store::save_proposal(vault, &proposal)?;

    Ok(result)
}

/// Rejects `id`: authorizes (`WriteOperation::RejectProposal`), loads,
/// re-checks expiry, validates the `Pending -> Rejected` transition (same
/// idempotency/conflict discipline as `approve_proposal` — an already
/// `Approved`/`Rejected`/`Expired` proposal fails closed with `Conflict`),
/// and persists `Rejected { reason, rejected_at: now }`. Never executes
/// the proposal's operation — a rejection is a pure status transition.
pub fn reject_proposal(ctx: &RequestContext, vault: &storage::Vault, id: &ProposalId, reason: String, now: &str) -> Result<(), BrainApiError> {
    let mut proposal = authorize_and_load(ctx, vault, id, WriteOperation::RejectProposal, now)?;

    let target = crate::ProposalStatus::Rejected { reason, rejected_at: now.to_string() };
    if !crate::valid_proposal_transition(proposal.status(), &target) {
        return Err(BrainApiError::Conflict);
    }

    proposal.transition_to(target).map_err(|_| BrainApiError::Conflict)?;
    crate::proposal_store::save_proposal(vault, &proposal)?;
    Ok(())
}

/// Lists every persisted proposal. Requires the SAME `WriteOperation::ApproveProposal`
/// permission `approve_proposal`/`reject_proposal` require (not merely
/// "any recognized caller") — in B5, only the trusted approval surface has
/// any reason to see pending proposals at all; an MCP agent has no
/// legitimate use for this list (it already knows the ids of proposals it
/// itself created, and has no business seeing anyone else's). Re-checks
/// expiry against `now` for every listed proposal (same defense-in-depth
/// rule as `approve_proposal`/`reject_proposal`), but does NOT persist an
/// `Expired` transition as a side effect of listing — expiry here is a
/// presentation-time check only; the proposal's own persisted status is
/// left untouched by a mere list call.
pub fn list_proposals(ctx: &RequestContext, vault: &storage::Vault, now: &str) -> Result<Vec<PendingWriteProposal>, BrainApiError> {
    if !ctx.write_policy.as_ref().is_some_and(|p| p.allows(WriteOperation::ApproveProposal)) {
        return Err(BrainApiError::PermissionDenied);
    }
    let proposals = crate::proposal_store::list_proposals(vault)?;
    Ok(proposals
        .into_iter()
        .map(|mut proposal| {
            if is_expired(&proposal, now) && *proposal.status() == crate::ProposalStatus::Pending {
                let _ = proposal.transition_to(crate::ProposalStatus::Expired);
            }
            proposal
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallOrigin, ProposalProvenance, ProposalStatus, WritePolicy};

    fn unique_test_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-brain-api-approval-test-{}-{}-{}",
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
            read_policy: None,
            origin: CallOrigin::McpSession("session-abc".into()),
        }
    }

    fn trusted_ctx() -> RequestContext {
        RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: Some(WritePolicy::trusted_cli()),
            read_policy: None,
            origin: CallOrigin::TrustedLocal(crate::TrustedLocalToken::new()),
        }
    }

    const NOW: &str = "2026-11-16T00:00:00Z";
    const LATER: &str = "2026-11-17T00:00:00Z";

    fn save_memory_proposal(vault: &storage::Vault, note_id: &str, body: &str) -> ProposalId {
        crate::save_memory(
            &mcp_ctx(),
            vault,
            storage::NoteId::parse(note_id).unwrap(),
            domain::NoteType::Bug,
            domain::Classification::Internal,
            "project-x".to_string(),
            "agent:coder@prompt-v3".to_string(),
            "2026-11-16".to_string(),
            body.to_string(),
            NOW,
        )
        .unwrap()
    }

    // --- approve_proposal: permission ---

    #[test]
    fn approve_proposal_denied_for_mcp_originated_context() {
        let vault = test_vault();
        let id = save_memory_proposal(&vault, "note-a", "content");
        let result = approve_proposal(&mcp_ctx(), &vault, &id, NOW);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
        // and the proposal is untouched: still Pending.
        let loaded = crate::proposal_store::load_proposal(&vault, &id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Pending);
    }

    #[test]
    fn permission_check_happens_before_proposal_existence_is_revealed_on_approve() {
        let vault = test_vault();
        let nonexistent = ProposalId::generate();
        let result = approve_proposal(&mcp_ctx(), &vault, &nonexistent, NOW);
        assert_eq!(result, Err(BrainApiError::PermissionDenied), "must be PermissionDenied, not NotFound, for an unauthorized caller");
    }

    #[test]
    fn permission_check_happens_before_proposal_existence_is_revealed_on_reject() {
        let vault = test_vault();
        let nonexistent = ProposalId::generate();
        let result = reject_proposal(&mcp_ctx(), &vault, &nonexistent, "no".into(), NOW);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn approve_proposal_on_an_unknown_id_under_trusted_cli_returns_not_found() {
        let vault = test_vault();
        let nonexistent = ProposalId::generate();
        let result = approve_proposal(&trusted_ctx(), &vault, &nonexistent, NOW);
        assert_eq!(result, Err(BrainApiError::NotFound));
    }

    // --- approve_proposal: SaveMemory end-to-end ---

    #[test]
    fn approve_proposal_succeeds_for_trusted_cli_context_and_commits_the_note() {
        let vault = test_vault();
        let id = save_memory_proposal(&vault, "note-a", "the real content");
        let result = approve_proposal(&trusted_ctx(), &vault, &id, NOW).unwrap();
        assert!(matches!(result, storage::CommitResult::Committed(_) | storage::CommitResult::GitPending { .. }));

        let note = vault.read_note(&storage::NoteId::parse("note-a").unwrap()).unwrap();
        assert!(note.content.contains("the real content"));

        let loaded = crate::proposal_store::load_proposal(&vault, &id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Approved { approved_at: NOW.to_string() });
    }

    #[test]
    fn approve_proposal_uses_exactly_the_persisted_operation_not_a_caller_override() {
        // Compile-time property: approve_proposal's signature is (ctx,
        // vault, id, now) -- there is no operation/payload parameter for
        // a caller to pass a different note_id/body/classification
        // through. This test's own successful compilation with only
        // those 4 arguments is the proof; runtime half: the committed
        // note's content matches what save_memory_proposal proposed, not
        // anything this test could have substituted.
        let vault = test_vault();
        let id = save_memory_proposal(&vault, "note-b", "proposed-body-xyz");
        approve_proposal(&trusted_ctx(), &vault, &id, NOW).unwrap();
        let note = vault.read_note(&storage::NoteId::parse("note-b").unwrap()).unwrap();
        assert!(note.content.contains("proposed-body-xyz"));
    }

    // --- approve_proposal: UpdateMemory end-to-end ---

    #[test]
    fn approve_proposal_executes_update_memory_and_replaces_the_body() {
        let vault = test_vault();
        let create_id = save_memory_proposal(&vault, "note-c", "original body");
        approve_proposal(&trusted_ctx(), &vault, &create_id, NOW).unwrap();

        let update_id = crate::update_memory(
            &mcp_ctx(),
            &vault,
            storage::NoteId::parse("note-c").unwrap(),
            "updated body".to_string(),
            storage::Version(1),
            NOW,
        )
        .unwrap();
        approve_proposal(&trusted_ctx(), &vault, &update_id, LATER).unwrap();

        let (_, body) = storage::split_record(vault.read_note(&storage::NoteId::parse("note-c").unwrap()).unwrap().content.as_bytes()).unwrap();
        assert_eq!(body.as_deref(), Some("updated body"));
    }

    #[test]
    fn approve_proposal_on_update_memory_with_stale_expected_version_is_conflict() {
        let vault = test_vault();
        let create_id = save_memory_proposal(&vault, "note-d", "v1");
        approve_proposal(&trusted_ctx(), &vault, &create_id, NOW).unwrap();

        // Propose an update against version 1 (correct now), then
        // approve a SECOND, competing update also against version 1 --
        // the first approval advances the note to version 2, so the
        // second's expected_version is now stale.
        let update_a =
            crate::update_memory(&mcp_ctx(), &vault, storage::NoteId::parse("note-d").unwrap(), "v2-a".to_string(), storage::Version(1), NOW)
                .unwrap();
        let update_b =
            crate::update_memory(&mcp_ctx(), &vault, storage::NoteId::parse("note-d").unwrap(), "v2-b".to_string(), storage::Version(1), NOW)
                .unwrap();

        approve_proposal(&trusted_ctx(), &vault, &update_a, LATER).unwrap();
        let result = approve_proposal(&trusted_ctx(), &vault, &update_b, LATER);
        assert_eq!(result, Err(BrainApiError::Conflict));

        // the second proposal must still be Pending (not silently
        // consumed) -- the caller can re-propose with a fresh version.
        let loaded = crate::proposal_store::load_proposal(&vault, &update_b).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Pending);
    }

    // --- approve_proposal: LinkEntities end-to-end ---

    #[test]
    fn approve_proposal_executes_link_entities_and_appends_the_relation() {
        let vault = test_vault();
        let a = save_memory_proposal(&vault, "note-e", "a");
        let b = save_memory_proposal(&vault, "note-f", "b");
        approve_proposal(&trusted_ctx(), &vault, &a, NOW).unwrap();
        approve_proposal(&trusted_ctx(), &vault, &b, NOW).unwrap();

        let link_id = crate::link_entities(
            &mcp_ctx(),
            &vault,
            storage::NoteId::parse("note-e").unwrap(),
            storage::NoteId::parse("note-f").unwrap(),
            crate::RelationKind::Supports,
            NOW,
        )
        .unwrap();
        approve_proposal(&trusted_ctx(), &vault, &link_id, LATER).unwrap();

        let (metadata, _) = storage::split_record(vault.read_note(&storage::NoteId::parse("note-e").unwrap()).unwrap().content.as_bytes()).unwrap();
        assert_eq!(metadata.relations.supports, vec!["note-f".to_string()]);
    }

    // --- B5.17 Task 36 mutation-testing gaps: map_storage_error_for_write/
    //     map_storage_error_for_read's named StorageError arms, proven
    //     distinguishable from the `_ => CorruptData` catch-all through
    //     the real public approve_proposal surface, not a direct unit
    //     test of the private mapping function itself. A proposal's
    //     target note id is never validated at proposal-creation time
    //     (update_memory/link_entities's own doc comments: existence is
    //     checked only at approval time) -- proposing against a note id
    //     that was never actually created is a real, reachable path to
    //     StorageError::NotFound surfacing through execute()'s
    //     update_note_body/add_relation/vault.read_note calls.

    #[test]
    fn approve_proposal_on_update_memory_targeting_a_never_created_note_is_not_found_not_corrupt_data() {
        let vault = test_vault();
        let update_id =
            crate::update_memory(&mcp_ctx(), &vault, storage::NoteId::parse("note-never-created").unwrap(), "body".to_string(), storage::Version(0), NOW)
                .unwrap();

        let result = approve_proposal(&trusted_ctx(), &vault, &update_id, LATER);

        assert_eq!(result, Err(BrainApiError::NotFound), "update_note_body's real StorageError::NotFound must map to BrainApiError::NotFound, never fall through to the CorruptData catch-all");
    }

    #[test]
    fn approve_proposal_on_link_entities_whose_from_note_was_never_created_is_not_found_not_corrupt_data() {
        let vault = test_vault();
        let to = save_memory_proposal(&vault, "note-g", "b");
        approve_proposal(&trusted_ctx(), &vault, &to, NOW).unwrap();

        let link_id = crate::link_entities(
            &mcp_ctx(),
            &vault,
            storage::NoteId::parse("note-from-never-created").unwrap(),
            storage::NoteId::parse("note-g").unwrap(),
            crate::RelationKind::Supports,
            NOW,
        )
        .unwrap();

        let result = approve_proposal(&trusted_ctx(), &vault, &link_id, LATER);

        // link_entities's own execute() path calls vault.read_note on
        // `from` BEFORE add_relation -- this exercises
        // map_storage_error_for_read's NotFound arm specifically, not
        // map_storage_error_for_write's.
        assert_eq!(result, Err(BrainApiError::NotFound), "vault.read_note's real StorageError::NotFound must map to BrainApiError::NotFound through map_storage_error_for_read, never fall through to the CorruptData catch-all");
    }

    #[cfg(windows)]
    fn try_symlink_file(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }
    #[cfg(unix)]
    fn try_symlink_file(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    /// Same real attack `storage::vault`'s own
    /// `record_symlink_resolving_inside_the_vault_root_is_rejected_as_an_alias_not_followed`
    /// (SEC-002 closure) exercises directly against `Vault::read_note` --
    /// replayed here through `brain-api`'s own public `approve_proposal`
    /// surface, since `map_storage_error_for_write`'s `PathEscapesVaultRoot`
    /// arm is reachable through exactly this path (`update_memory`'s
    /// `execute()` calls `storage::update_note_body`, whose first step is
    /// `vault.read_note`) and is a real, not equivalent, mutant: deleting
    /// this arm would fall the outcome through to `CorruptData`, a
    /// materially different, less specific error for a caller to act on.
    #[test]
    fn approve_proposal_on_update_memory_whose_target_record_was_swapped_for_a_symlink_is_invalid_input_not_corrupt_data() {
        let dir = unique_test_dir();
        let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
        let create_id = save_memory_proposal(&vault, "note-h", "original");
        approve_proposal(&trusted_ctx(), &vault, &create_id, NOW).unwrap();

        let update_id =
            crate::update_memory(&mcp_ctx(), &vault, storage::NoteId::parse("note-h").unwrap(), "updated".to_string(), storage::Version(1), NOW).unwrap();

        // Swap the committed record file for a symlink pointing at an
        // equally well-formed record elsewhere -- the aliasing attack
        // this function's own on-disk identity check must reject, never
        // silently follow.
        std::fs::create_dir(dir.join("store")).unwrap();
        let outside_target = dir.join("store").join("real.record");
        std::fs::rename(dir.join("note-h.record"), &outside_target).unwrap();
        if try_symlink_file(&outside_target, &dir.join("note-h.record")).is_err() {
            eprintln!("skipping: could not create a symlink in this environment");
            return;
        }

        let result = approve_proposal(&trusted_ctx(), &vault, &update_id, LATER);

        assert!(matches!(result, Err(BrainApiError::InvalidInput(_))), "a real StorageError::PathEscapesVaultRoot must map to BrainApiError::InvalidInput, never fall through to the CorruptData catch-all, got {result:?}");
    }

    /// A directory sitting where a note's `.record` file is expected is
    /// data corruption, not an aliasing attack (`handle_safety.rs::
    /// open_verified`'s own documented distinction) — it surfaces as a
    /// real `StorageError::Io`, genuinely constructible with no special
    /// privilege (unlike the symlink case above, which some CI
    /// environments cannot create). Exercises `map_storage_error_for_write`'s
    /// `Io(_) | Journal(_)` arm through `update_memory`'s real
    /// `approve_proposal` path.
    #[test]
    fn approve_proposal_on_update_memory_whose_target_record_path_is_a_directory_is_runtime_unavailable_not_corrupt_data() {
        let dir = unique_test_dir();
        let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
        let create_id = save_memory_proposal(&vault, "note-i", "original");
        approve_proposal(&trusted_ctx(), &vault, &create_id, NOW).unwrap();

        let update_id =
            crate::update_memory(&mcp_ctx(), &vault, storage::NoteId::parse("note-i").unwrap(), "updated".to_string(), storage::Version(1), NOW).unwrap();

        std::fs::remove_file(dir.join("note-i.record")).unwrap();
        std::fs::create_dir(dir.join("note-i.record")).unwrap();

        let result = approve_proposal(&trusted_ctx(), &vault, &update_id, LATER);

        assert_eq!(result, Err(BrainApiError::RuntimeUnavailable), "a real StorageError::Io must map to BrainApiError::RuntimeUnavailable through map_storage_error_for_write, never fall through to the CorruptData catch-all, got {result:?}");
    }

    /// Same real `StorageError::Io` cause as the test above, reached
    /// through `link_entities`'s `vault.read_note(&from_id)` call instead
    /// -- exercises `map_storage_error_for_read`'s `Io(_) | Journal(_)`
    /// arm specifically.
    #[test]
    fn approve_proposal_on_link_entities_whose_from_record_path_is_a_directory_is_runtime_unavailable_not_corrupt_data() {
        let dir = unique_test_dir();
        let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
        let from_id = save_memory_proposal(&vault, "note-j", "a");
        let to_id = save_memory_proposal(&vault, "note-k", "b");
        approve_proposal(&trusted_ctx(), &vault, &from_id, NOW).unwrap();
        approve_proposal(&trusted_ctx(), &vault, &to_id, NOW).unwrap();

        let link_id = crate::link_entities(
            &mcp_ctx(),
            &vault,
            storage::NoteId::parse("note-j").unwrap(),
            storage::NoteId::parse("note-k").unwrap(),
            crate::RelationKind::Supports,
            NOW,
        )
        .unwrap();

        std::fs::remove_file(dir.join("note-j.record")).unwrap();
        std::fs::create_dir(dir.join("note-j.record")).unwrap();

        let result = approve_proposal(&trusted_ctx(), &vault, &link_id, LATER);

        assert_eq!(result, Err(BrainApiError::RuntimeUnavailable), "a real StorageError::Io must map to BrainApiError::RuntimeUnavailable through map_storage_error_for_read, never fall through to the CorruptData catch-all, got {result:?}");
    }

    // --- expiry: defense in depth ---

    #[test]
    fn approve_proposal_rejects_an_expired_proposal_even_if_status_still_says_pending() {
        let vault = test_vault();
        // Build a proposal whose expires_at is already in the past
        // relative to `now`, with status still Pending (simulating
        // "never listed since expiry") -- bypass save_memory (which
        // never sets an expiry) by constructing + persisting directly.
        let proposal = PendingWriteProposal::new(
            ProposalId::generate(),
            "trusted-local".to_string(),
            ProposalProvenance { source: "trusted-local".to_string(), created_at: "2026-11-01T00:00:00Z".to_string() },
            Some("2026-11-02T00:00:00Z".to_string()), // expired well before NOW
            ProposedOperation::SaveMemory {
                note_id: "note-g".to_string(),
                note_type: domain::NoteType::Bug,
                classification: domain::Classification::Internal,
                project: "project-x".to_string(),
                source: "agent:coder@prompt-v3".to_string(),
                event_period: "2026-11-01".to_string(),
                body: "content".to_string(),
            },
        );
        crate::proposal_store::save_proposal(&vault, &proposal).unwrap();

        let result = approve_proposal(&trusted_ctx(), &vault, &proposal.id, NOW);
        assert_eq!(result, Err(BrainApiError::InvalidInput("proposal expired".to_string())));

        // must not have executed: the note was never created.
        assert!(matches!(
            vault.read_note(&storage::NoteId::parse("note-g").unwrap()),
            Err(storage::StorageError::NotFound { .. })
        ));
        // status on disk is untouched (still Pending) -- a mere approve
        // attempt on an expired proposal does not itself mutate status.
        let loaded = crate::proposal_store::load_proposal(&vault, &proposal.id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Pending);
    }

    #[test]
    fn reject_proposal_also_rejects_an_expired_proposal_even_if_status_still_says_pending() {
        let vault = test_vault();
        let proposal = PendingWriteProposal::new(
            ProposalId::generate(),
            "trusted-local".to_string(),
            ProposalProvenance { source: "trusted-local".to_string(), created_at: "2026-11-01T00:00:00Z".to_string() },
            Some("2026-11-02T00:00:00Z".to_string()),
            ProposedOperation::SaveMemory {
                note_id: "note-h".to_string(),
                note_type: domain::NoteType::Bug,
                classification: domain::Classification::Internal,
                project: "project-x".to_string(),
                source: "agent:coder@prompt-v3".to_string(),
                event_period: "2026-11-01".to_string(),
                body: "content".to_string(),
            },
        );
        crate::proposal_store::save_proposal(&vault, &proposal).unwrap();

        let result = reject_proposal(&trusted_ctx(), &vault, &proposal.id, "too late".into(), NOW);
        assert_eq!(result, Err(BrainApiError::InvalidInput("proposal expired".to_string())));
    }

    // --- idempotency / conflict matrix ---

    #[test]
    fn approving_an_already_approved_proposal_is_rejected_not_double_executed() {
        let vault = test_vault();
        let id = save_memory_proposal(&vault, "note-i", "v1");
        approve_proposal(&trusted_ctx(), &vault, &id, NOW).unwrap();

        let second = approve_proposal(&trusted_ctx(), &vault, &id, LATER);
        assert_eq!(second, Err(BrainApiError::Conflict));

        // exactly one commit happened: the note's version is 1, not 2+
        // (a second execution would have advanced it further).
        let note = vault.read_note(&storage::NoteId::parse("note-i").unwrap()).unwrap();
        assert_eq!(note.version, storage::Version(1));
    }

    #[test]
    fn approving_a_rejected_proposal_fails_with_conflict() {
        let vault = test_vault();
        let id = save_memory_proposal(&vault, "note-j", "v1");
        reject_proposal(&trusted_ctx(), &vault, &id, "no".into(), NOW).unwrap();

        let result = approve_proposal(&trusted_ctx(), &vault, &id, LATER);
        assert_eq!(result, Err(BrainApiError::Conflict));
        assert!(matches!(
            vault.read_note(&storage::NoteId::parse("note-j").unwrap()),
            Err(storage::StorageError::NotFound { .. })
        ));
    }

    #[test]
    fn reject_proposal_moves_pending_to_rejected_and_approve_proposal_then_fails() {
        let vault = test_vault();
        let id = save_memory_proposal(&vault, "note-k", "v1");
        reject_proposal(&trusted_ctx(), &vault, &id, "bad idea".into(), NOW).unwrap();

        let loaded = crate::proposal_store::load_proposal(&vault, &id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Rejected { reason: "bad idea".to_string(), rejected_at: NOW.to_string() });

        let result = approve_proposal(&trusted_ctx(), &vault, &id, LATER);
        assert_eq!(result, Err(BrainApiError::Conflict));
    }

    #[test]
    fn rejecting_an_already_rejected_proposal_fails_with_conflict() {
        let vault = test_vault();
        let id = save_memory_proposal(&vault, "note-l", "v1");
        reject_proposal(&trusted_ctx(), &vault, &id, "first".into(), NOW).unwrap();
        let result = reject_proposal(&trusted_ctx(), &vault, &id, "second".into(), LATER);
        assert_eq!(result, Err(BrainApiError::Conflict));
    }

    #[test]
    fn rejecting_an_already_approved_proposal_fails_with_conflict() {
        let vault = test_vault();
        let id = save_memory_proposal(&vault, "note-m", "v1");
        approve_proposal(&trusted_ctx(), &vault, &id, NOW).unwrap();
        let result = reject_proposal(&trusted_ctx(), &vault, &id, "too late".into(), LATER);
        assert_eq!(result, Err(BrainApiError::Conflict));
    }

    // --- list_proposals ---

    #[test]
    fn list_proposals_denied_for_mcp_originated_context() {
        let vault = test_vault();
        save_memory_proposal(&vault, "note-n", "v1");
        let result = list_proposals(&mcp_ctx(), &vault, NOW);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn list_proposals_returns_every_pending_proposal_for_trusted_cli() {
        let vault = test_vault();
        save_memory_proposal(&vault, "note-o", "v1");
        save_memory_proposal(&vault, "note-p", "v1");
        let proposals = list_proposals(&trusted_ctx(), &vault, NOW).unwrap();
        assert_eq!(proposals.len(), 2);
    }

    #[test]
    fn list_proposals_marks_an_expired_pending_proposal_as_expired_in_its_output_without_persisting_it() {
        let vault = test_vault();
        let proposal = PendingWriteProposal::new(
            ProposalId::generate(),
            "trusted-local".to_string(),
            ProposalProvenance { source: "trusted-local".to_string(), created_at: "2026-11-01T00:00:00Z".to_string() },
            Some("2026-11-02T00:00:00Z".to_string()),
            ProposedOperation::SaveMemory {
                note_id: "note-q".to_string(),
                note_type: domain::NoteType::Bug,
                classification: domain::Classification::Internal,
                project: "project-x".to_string(),
                source: "agent:coder@prompt-v3".to_string(),
                event_period: "2026-11-01".to_string(),
                body: "content".to_string(),
            },
        );
        crate::proposal_store::save_proposal(&vault, &proposal).unwrap();

        let proposals = list_proposals(&trusted_ctx(), &vault, NOW).unwrap();
        assert_eq!(proposals.len(), 1);
        assert_eq!(*proposals[0].status(), ProposalStatus::Expired);

        // the ON-DISK status is untouched by a mere list call.
        let reloaded = crate::proposal_store::load_proposal(&vault, &proposal.id).unwrap();
        assert_eq!(*reloaded.status(), ProposalStatus::Pending);
    }

    #[test]
    fn list_proposals_on_an_empty_vault_returns_empty_not_an_error() {
        let vault = test_vault();
        let proposals = list_proposals(&trusted_ctx(), &vault, NOW).unwrap();
        assert!(proposals.is_empty());
    }

    // --- exactly-once execution under real concurrent double-approve ---

    #[test]
    fn concurrent_double_approve_of_the_same_proposal_executes_exactly_once() {
        let dir = unique_test_dir();
        let vault = std::sync::Arc::new(storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap());
        let id = save_memory_proposal(&vault, "note-r", "content");
        let id = std::sync::Arc::new(id);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let vault = std::sync::Arc::clone(&vault);
                let id = std::sync::Arc::clone(&id);
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    approve_proposal(&trusted_ctx(), &vault, &id, NOW)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        let conflict_count = results.iter().filter(|r| matches!(r, Err(BrainApiError::Conflict))).count();
        assert_eq!(ok_count, 1, "exactly one concurrent approve must succeed, got results: {results:?}");
        assert_eq!(conflict_count, 1, "the other concurrent approve must fail with Conflict, got results: {results:?}");

        // the note itself reflects exactly one real commit (version 1,
        // never 2+ -- a double execution would have advanced it further).
        let note = vault.read_note(&storage::NoteId::parse("note-r").unwrap()).unwrap();
        assert_eq!(note.version, storage::Version(1));

        // the proposal itself ends up Approved exactly once, not stuck
        // Pending and not corrupted by two concurrent writers.
        let loaded = crate::proposal_store::load_proposal(&vault, &id).unwrap();
        assert_eq!(*loaded.status(), ProposalStatus::Approved { approved_at: NOW.to_string() });
    }
}
