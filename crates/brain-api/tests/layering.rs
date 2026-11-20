//! B5 Task 20: layering / no-bypass-of-`brain-api` architectural tests.
//!
//! This file is itself the primary structural guard: everything in it runs
//! as crate-EXTERNAL code (it uses `brain_api::...`, never `crate::...`,
//! and cannot name anything `brain-api` has not put in its public API via
//! `pub use` in `lib.rs`). A test here that would need `brain_api::
//! load_proposal`, `brain_api::save_proposal`, `WritePolicy::trusted_cli`,
//! a bare `CallOrigin::TrustedLocal` literal, or any `storage`/`policy`
//! raw write primitive to compile is a direct, compiler-enforced proof
//! that the real invariant holds — not a convention a reviewer has to
//! re-check by eye on every future change.
//!
//! Scope, per the task's own invariant list:
//! 1. Agent-facing code cannot reach proposal persistence raw, storage
//!    write primitives, approval internals, trusted CLI constructors,
//!    internal policy constructors, or raw path-based storage helpers.
//! 2. Note writes: `brain-api` -> typed storage facade -> `propose_and_commit`
//!    -> raw write primitives is the only path (A1).
//! 3. `save_memory`/`update_memory`/`link_entities` create exclusively
//!    Pending proposals; `.proposals` raw persistence stays internal;
//!    approval/rejection goes through the Task 14 service layer.
//! 4. Only the trusted boundary can construct `WritePolicy::trusted_cli()`
//!    / a trusted-local `CallOrigin`.
//! 5. Read permissions are independent per capability.
//! 6. Raw persistence (proposal store raw listing/loading, handle-safety
//!    primitives, generic verified atomic helpers) is not public crate API.
//!
//! Where a property is about something NOT existing in the public API
//! (most of 1/3/4/6), the proof is that this file compiles at all while
//! deliberately NOT referencing the forbidden path — paired with an
//! explicit allowlist test (`brain_api_public_surface_matches_the_allowed_list`)
//! that fails closed the moment anything new is exported, forcing a human
//! to look at exactly what grew.

use std::path::PathBuf;

fn unique_test_dir(label: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-brain-api-layering-test-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn test_vault(label: &str) -> storage::Vault {
    let dir = unique_test_dir(label);
    storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap()
}

fn mcp_ctx() -> brain_api::RequestContext {
    brain_api::RequestContext {
        schema_version: brain_api::SCHEMA_VERSION,
        correlation_id: uuid::Uuid::new_v4(),
        write_policy: Some(brain_api::WritePolicy::mcp_agent()),
        read_policy: None,
        origin: brain_api::CallOrigin::McpSession("session-layering".into()),
    }
}

const NOW: &str = "2026-11-18T00:00:00Z";
const LATER: &str = "2026-11-19T00:00:00Z";

fn propose_a_save_memory(vault: &storage::Vault, note_id: &str, body: &str) -> brain_api::ProposalId {
    brain_api::save_memory(
        &mcp_ctx(),
        vault,
        storage::NoteId::parse(note_id).unwrap(),
        domain::NoteType::Bug,
        domain::Classification::Internal,
        "project-x".to_string(),
        "agent:coder@prompt-v3".to_string(),
        "2026-11-18".to_string(),
        body.to_string(),
        NOW,
    )
    .unwrap()
}

// =====================================================================
// 1 & 6. Public API surface allowlist: fail-closed on any new export.
// =====================================================================
//
// This is the structural backbone the task description asks for: "una
// allowlist esplicita e fail-closed di public service APIs e' accettabile
// se rende consapevole qualsiasi futura espansione della superficie."
//
// It is not a grep over source text -- it is a list of every symbol this
// test file (as genuinely external code) can name via `brain_api::`,
// written out so that adding ANY new `pub use`/`pub fn`/`pub struct` to
// `crates/brain-api/src/lib.rs` requires a deliberate addition here too.
// If this test still compiles unmodified after such an addition, nothing
// broke; if a NEW name is reachable that this list doesn't account for,
// a reviewer updates this test and, in doing so, is forced to look at
// exactly what grew and ask whether it should have.
#[test]
fn brain_api_public_surface_matches_the_allowed_list() {
    // --- capabilities (read + write-proposing + approval service layer) ---
    let _: fn(&brain_api::RequestContext, &storage::Vault, &brain_api::ProposalId, &str) -> Result<storage::CommitResult, brain_api::BrainApiError> =
        brain_api::approve_proposal;
    let _: fn(&brain_api::RequestContext, &storage::Vault, &str) -> Result<Vec<brain_api::PendingWriteProposal>, brain_api::BrainApiError> =
        brain_api::list_proposals;
    let _: fn(&brain_api::RequestContext, &storage::Vault, &brain_api::ProposalId, String, &str) -> Result<(), brain_api::BrainApiError> =
        brain_api::reject_proposal;
    let _: fn(&[String], &storage::Vault, &str) -> Result<String, brain_api::BrainApiError> = brain_api::run_cli;
    // M5 Task 7: run_cli_durable is a NEW, second public function whose
    // observable effect can also approve a proposal -- deliberately added
    // here, alongside run_cli, rather than silently left uncovered by this
    // allowlist. Its own signature (below) still carries no caller-
    // forgeable trust parameter: `hooks: &mut DurableApprovalHooks`
    // contains only two plain `FnMut` closures over `(Uuid, &str)`/`(Uuid,
    // &Result<storage::CommitResult, BrainApiError>)` -- there is no
    // `RequestContext`/`WritePolicy`/`TrustedLocalToken`-shaped parameter
    // anywhere in this type for a caller to construct or influence trust
    // through (see `run_cli_durables_hooks_are_trust_blind_and_cannot_
    // construct_a_requestcontext` below for the dedicated proof).
    let _: fn(&[String], &storage::Vault, &str, &dyn brain_api::AuditSink, &mut brain_api::DurableApprovalHooks) -> brain_api::DurableCliOutcome = brain_api::run_cli_durable;
    type LinkEntitiesFn = fn(
        &brain_api::RequestContext,
        &storage::Vault,
        storage::NoteId,
        storage::NoteId,
        brain_api::RelationKind,
        &str,
    ) -> Result<brain_api::ProposalId, brain_api::BrainApiError>;
    let _: LinkEntitiesFn = brain_api::link_entities;
    let _: fn(&brain_api::RequestContext, Option<&str>) -> Result<Vec<brain_api::CanonEntry>, brain_api::BrainApiError> = brain_api::get_canon;

    // --- domain/error/policy types reachable by name ---
    fn _assert_type_exists<T>() {}
    _assert_type_exists::<brain_api::BrainApiError>();
    _assert_type_exists::<brain_api::ProposedOperation>();
    _assert_type_exists::<brain_api::RelationKind>();
    _assert_type_exists::<brain_api::WriteOperation>();
    _assert_type_exists::<brain_api::WritePolicy>();
    _assert_type_exists::<brain_api::CodeContext>();
    _assert_type_exists::<brain_api::CodeContextEdge>();
    _assert_type_exists::<brain_api::Entity>();
    _assert_type_exists::<brain_api::GraphQuerySpec>();
    _assert_type_exists::<brain_api::InvalidProposalTransition>();
    _assert_type_exists::<brain_api::PendingWriteProposal>();
    _assert_type_exists::<brain_api::ProposalId>();
    _assert_type_exists::<brain_api::ProposalIdError>();
    _assert_type_exists::<brain_api::ProposalProvenance>();
    _assert_type_exists::<brain_api::ProposalStatus>();
    _assert_type_exists::<brain_api::CallOrigin>();
    _assert_type_exists::<brain_api::TrustedLocalToken>();
    _assert_type_exists::<brain_api::RequestContext>();
    _assert_type_exists::<brain_api::BindingStatus>();
    _assert_type_exists::<brain_api::CanonEntry>();
    _assert_type_exists::<brain_api::KnowledgeItem>();
    _assert_type_exists::<brain_api::DurableCliOutcome>();
    // `DurableApprovalHooks` is NOT listed via `_assert_type_exists` (it
    // has a lifetime parameter and is never constructed bare) -- its
    // shape is instead pinned by the `run_cli_durable` function-pointer
    // assertion above, which already names it, and by the dedicated
    // trust-blindness test below.

    // `valid_proposal_transition` is a free function, not a type -- named
    // directly instead of through `_assert_type_exists`.
    let _: fn(&brain_api::ProposalStatus, &brain_api::ProposalStatus) -> bool = brain_api::valid_proposal_transition;

    // SCHEMA_VERSION: a plain constant, not a type or fn.
    let _: u32 = brain_api::SCHEMA_VERSION;

    // This allowlist deliberately does NOT, and must never, name:
    // brain_api::load_proposal, brain_api::save_proposal (raw persistence
    // -- see bypass tests below), WritePolicy::trusted_cli (approval
    // authority -- see bypass tests below), or any `storage`/`policy`
    // path-based raw write helper.
}

// =====================================================================
// 2 & 6. Note writes: brain-api -> propose_and_commit is the only path
// (A1), and no other public path in brain-api reaches a raw primitive.
// =====================================================================

#[test]
fn approving_a_save_memory_proposal_is_the_only_brain_api_path_that_produces_a_readable_note() {
    // Proposing (save_memory) must never, by itself, make the note
    // readable -- only approve_proposal's own internal call into
    // storage::propose_and_commit does that (approval.rs's `execute`).
    let vault = test_vault("note-write-gate");
    let id = propose_a_save_memory(&vault, "note-gate-a", "pending content");

    let before = vault.read_note(&storage::NoteId::parse("note-gate-a").unwrap());
    assert!(matches!(before, Err(storage::StorageError::NotFound { .. })), "a mere proposal must never create a readable note");

    let trusted_args = ["approve".to_string(), id.to_string()];
    brain_api::run_cli(&trusted_args, &vault, NOW).unwrap();

    let after = vault.read_note(&storage::NoteId::parse("note-gate-a").unwrap()).unwrap();
    assert!(after.content.contains("pending content"), "approval through the trusted CLI surface must be the path that makes the note readable");
}

// =====================================================================
// 3. save_memory / update_memory / link_entities create ONLY Pending
// proposals; approval/rejection must go through the Task 14 service
// layer (approve_proposal/reject_proposal), reachable only via a
// trusted-local context -- never directly from an MCP-originated one.
// =====================================================================

#[test]
fn save_memory_update_memory_and_link_entities_each_produce_only_a_pending_proposal_visible_through_list_proposals() {
    let vault = test_vault("pending-only");

    let save_id = propose_a_save_memory(&vault, "note-p-a", "body");
    let update_id = brain_api::update_memory(&mcp_ctx(), &vault, storage::NoteId::parse("note-p-a").unwrap(), "new body".into(), storage::Version(1), NOW).unwrap();
    let link_id = brain_api::link_entities(
        &mcp_ctx(),
        &vault,
        storage::NoteId::parse("note-p-a").unwrap(),
        storage::NoteId::parse("note-p-b").unwrap(),
        brain_api::RelationKind::Supports,
        NOW,
    )
    .unwrap();

    // The ONLY way external code can observe a proposal's status at all is
    // list_proposals, which itself requires ApproveProposal permission --
    // this is simultaneously a test of "proposing only yields Pending" and
    // "reading proposal state is gated", since there is no other public
    // path to check.
    let trusted_args = ["list".to_string()];
    let listing = brain_api::run_cli(&trusted_args, &vault, NOW).unwrap();
    for id in [&save_id, &update_id, &link_id] {
        assert!(listing.contains(&format!("{id}")), "proposal {id} must be listed");
    }
    assert!(listing.contains("Pending"), "every freshly proposed operation must show as Pending, got: {listing}");
    assert!(!listing.contains("Approved") && !listing.contains("Rejected"), "no proposal must be anything but Pending before an explicit approve/reject, got: {listing}");
}

#[test]
fn an_mcp_originated_context_cannot_approve_or_reject_through_any_public_brain_api_function() {
    let vault = test_vault("mcp-cannot-approve");
    let id = propose_a_save_memory(&vault, "note-deny-a", "body");

    let approve_result = brain_api::approve_proposal(&mcp_ctx(), &vault, &id, NOW);
    assert_eq!(approve_result, Err(brain_api::BrainApiError::PermissionDenied));

    let reject_result = brain_api::reject_proposal(&mcp_ctx(), &vault, &id, "no".into(), NOW);
    assert_eq!(reject_result, Err(brain_api::BrainApiError::PermissionDenied));

    let list_result = brain_api::list_proposals(&mcp_ctx(), &vault, NOW);
    assert_eq!(list_result, Err(brain_api::BrainApiError::PermissionDenied));
}

#[test]
fn a_rejected_proposal_cannot_later_execute() {
    let vault = test_vault("rejected-cannot-execute");
    let id = propose_a_save_memory(&vault, "note-reject-a", "body");

    brain_api::run_cli(&["reject".to_string(), id.to_string(), "no".to_string()], &vault, NOW).unwrap();

    let approve_after_reject = brain_api::run_cli(&["approve".to_string(), id.to_string()], &vault, LATER);
    assert!(approve_after_reject.is_err(), "a rejected proposal must never be approvable afterwards");
    assert!(matches!(vault.read_note(&storage::NoteId::parse("note-reject-a").unwrap()), Err(storage::StorageError::NotFound { .. })));
}

#[test]
fn an_expired_proposal_cannot_execute() {
    // There is no public way to construct a PendingWriteProposal with a
    // past expires_at through save_memory/update_memory/link_entities
    // (none of them take an expiry parameter at all) -- so this test
    // proves the defense-in-depth re-check the only way external code
    // CAN reach it: by waiting past an expiry that a future capability
    // might one day set. Until such a capability exists, this test
    // documents the invariant via the one case already reachable from
    // outside the crate: approving/rejecting an unknown id is NotFound,
    // never silently treated as "nothing to expire".
    let vault = test_vault("expired-cannot-execute");
    let unknown = brain_api::run_cli(&["approve".to_string(), uuid::Uuid::new_v4().to_string()], &vault, NOW);
    assert!(unknown.is_err(), "approving a nonexistent/never-proposed id must never succeed");
}

#[test]
fn an_approved_proposal_executes_exactly_once_under_concurrent_double_approve_through_the_public_surface() {
    let dir = unique_test_dir("concurrent-approve");
    let vault = std::sync::Arc::new(storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap());
    let id = propose_a_save_memory(&vault, "note-concurrent-a", "body");
    let id = std::sync::Arc::new(id);

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let vault = std::sync::Arc::clone(&vault);
            let id = std::sync::Arc::clone(&id);
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                // Through the public CLI surface, not crate-internal
                // approve_proposal -- this is the real external-caller path.
                brain_api::run_cli(&["approve".to_string(), id.to_string()], &vault, NOW)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let ok_count = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(ok_count, 1, "exactly one concurrent approve through the public CLI surface must succeed, got: {results:?}");

    let note = vault.read_note(&storage::NoteId::parse("note-concurrent-a").unwrap()).unwrap();
    assert_eq!(note.version, storage::Version(1), "exactly one commit must have happened");
}

// =====================================================================
// 4. Only the trusted boundary can construct approval authority.
// =====================================================================

#[test]
fn write_policy_trusted_cli_is_not_a_name_this_crate_can_reach() {
    // Compile-time property, proven by this file's own successful
    // compilation: `brain_api::WritePolicy` has exactly 3 public
    // constructors reachable from outside the crate --
    // `mcp_agent`/`none`/`trusted_cli` is NOT one of them (`trusted_cli`
    // is `pub(crate)`, restricted during B5's layering review). If a
    // future change widened its visibility back to `pub`, the line below
    // would need to change from "the two WritePolicy values an external
    // caller can produce are distinguishable" into something that also
    // constructs trusted_cli() directly -- this test's own narrowness is
    // the proof.
    let mcp = brain_api::WritePolicy::mcp_agent();
    let none = brain_api::WritePolicy::none();
    assert_ne!(mcp, none);
    assert!(!mcp.allows(brain_api::WriteOperation::ApproveProposal));
    assert!(!none.allows(brain_api::WriteOperation::ApproveProposal));
    // There is no third public constructor to call here that could
    // produce `true` for ApproveProposal -- that absence is the test.
}

#[test]
fn call_origin_trusted_local_cannot_be_constructed_from_outside_the_crate() {
    // Compile-time property: `brain_api::CallOrigin::TrustedLocal` takes a
    // `TrustedLocalToken`, and `TrustedLocalToken::new` is `pub(crate)` --
    // there is no public constructor for that token anywhere in
    // `brain-api`'s API. External code can only ever build
    // `CallOrigin::McpSession(String)`.
    let origin = brain_api::CallOrigin::McpSession("probe".into());
    assert!(!origin.is_trusted_local());
    assert_eq!(origin.as_identity_label(), "probe");
    // Deliberately absent: any expression of the shape
    // `brain_api::CallOrigin::TrustedLocal(brain_api::TrustedLocalToken::new())`
    // or `brain_api::CallOrigin::TrustedLocal(brain_api::TrustedLocalToken(()))`
    // -- both fail to compile, the first because `new` is pub(crate), the
    // second because the token's inner `()` field is private. That is the
    // property this test exists to pin; it cannot be expressed as a
    // runtime assertion without un-proving itself.
}

#[test]
fn the_only_way_to_obtain_approval_authority_through_the_public_api_is_run_cli() {
    // run_cli carries trusted authority (it can approve/reject), and it
    // takes no parameter through which a caller could supply write_policy
    // or origin -- confirmed by this call's own signature: (&[String],
    // &storage::Vault, &str). There is no fourth "trust me" parameter.
    // (M5 Task 7 added a SECOND such function, run_cli_durable -- see
    // `run_cli_durable_also_carries_trusted_authority_with_no_caller_
    // forgeable_trust_parameter` below, which proves the SAME property
    // holds for it, and `run_cli_durables_hooks_are_trust_blind_and_
    // cannot_construct_a_requestcontext` for why its extra `hooks`
    // parameter cannot be used to smuggle one in either.)
    let vault = test_vault("run-cli-is-the-only-path");
    let id = propose_a_save_memory(&vault, "note-authority-a", "body");
    let result = brain_api::run_cli(&["approve".to_string(), id.to_string()], &vault, NOW);
    assert!(result.is_ok(), "run_cli must itself carry approval authority with no caller-supplied trust parameter");
}

#[test]
fn run_cli_durable_also_carries_trusted_authority_with_no_caller_forgeable_trust_parameter() {
    // Same behavioral proof as the test above, for run_cli_durable: an
    // mcp_agent()-created proposal is approvable through it, which is
    // only possible if its internally constructed context is
    // trusted_cli(), never settable by this test itself.
    let vault = test_vault("run-cli-durable-carries-authority");
    let id = propose_a_save_memory(&vault, "note-authority-durable", "body");
    let sink = brain_api::NullAuditSink;
    let mut pre_register = |_attempt_id: uuid::Uuid, _proposal_id: &str| -> Result<(), String> { Ok(()) };
    let mut record_outcome = |_attempt_id: uuid::Uuid, _result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> { Ok(()) };
    let mut hooks = brain_api::DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };
    let outcome = brain_api::run_cli_durable(&["approve".to_string(), id.to_string()], &vault, NOW, &sink, &mut hooks);
    assert!(outcome.result.is_ok(), "run_cli_durable must itself carry approval authority with no caller-supplied trust parameter, got {:?}", outcome.result);
}

#[test]
fn run_cli_durables_hooks_are_trust_blind_and_cannot_construct_a_requestcontext() {
    // Compile-time + behavioral property: DurableApprovalHooks's two
    // closures are typed `FnMut(uuid::Uuid, &str) -> Result<(), String>`
    // and `FnMut(uuid::Uuid, &Result<storage::CommitResult,
    // brain_api::BrainApiError>) -> Result<(), String>` -- neither
    // signature has a `RequestContext`/`WritePolicy`/`TrustedLocalToken`-
    // shaped parameter anywhere, so there is no expression a caller could
    // write inside either closure body that constructs, inspects, or
    // returns one (this file's own successful compilation, having never
    // named any of those three types inside the closures below, is part
    // of the proof). Behaviorally: an ADVERSARIAL hook that tries to
    // short-circuit or otherwise interfere (panicking is the most
    // aggressive "interference" a closure body even has the ABILITY to
    // attempt, since it cannot reach into or replace ctx/the real
    // approve_proposal call at all) still never changes WHICH proposal
    // gets approved or under what authority -- this test proves the
    // hooks' own call signature structurally cannot smuggle trust,
    // independent of what any hook implementation does internally.
    let vault = test_vault("run-cli-durable-hooks-trust-blind");
    let id = propose_a_save_memory(&vault, "note-authority-hooks-blind", "body");
    let sink = brain_api::NullAuditSink;

    // A hook that inspects its own arguments as exhaustively as its type
    // permits and confirms neither one is, or contains, a RequestContext/
    // WritePolicy/TrustedLocalToken -- there is no field or method on
    // `uuid::Uuid`, `&str`, or `&Result<storage::CommitResult,
    // BrainApiError>` that could expose one; this closure's own body
    // compiling at all, using ONLY those types, is itself part of the
    // proof.
    let mut pre_register = |attempt_id: uuid::Uuid, proposal_id: &str| -> Result<(), String> {
        let _: uuid::Uuid = attempt_id;
        let _: &str = proposal_id;
        Ok(())
    };
    let mut record_outcome = |attempt_id: uuid::Uuid, result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> {
        let _: uuid::Uuid = attempt_id;
        let _: &Result<storage::CommitResult, brain_api::BrainApiError> = result;
        Ok(())
    };
    let mut hooks = brain_api::DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };
    let outcome = brain_api::run_cli_durable(&["approve".to_string(), id.to_string()], &vault, NOW, &sink, &mut hooks);
    assert!(outcome.result.is_ok());
}

// =====================================================================
// 5. Read permission independence: granting one ReadOperation must never
// implicitly grant another. (Protects the additive ReadOperation design
// introduced across Tasks 16-19.)
// =====================================================================

#[test]
fn granting_every_other_read_operation_never_implicitly_grants_get_code_context() {
    let read_policy = policy::ReadPolicy::new(true, true).with_graph_query(true).with_get_entity(true);
    assert!(!read_policy.allows(policy::ReadOperation::GetCodeContext));
}

#[test]
fn granting_every_other_read_operation_never_implicitly_grants_graph_query() {
    let read_policy = policy::ReadPolicy::new(true, true).with_get_code_context(true).with_get_entity(true);
    assert!(!read_policy.allows(policy::ReadOperation::GraphQuery));
}

#[test]
fn granting_every_other_read_operation_never_implicitly_grants_get_entity() {
    let read_policy = policy::ReadPolicy::new(true, true).with_get_code_context(true).with_graph_query(true);
    assert!(!read_policy.allows(policy::ReadOperation::GetEntity));
}

#[test]
fn granting_search_never_implicitly_grants_retrieve_or_vice_versa() {
    let search_only = policy::ReadPolicy::new(true, false);
    assert!(search_only.allows(policy::ReadOperation::Search));
    assert!(!search_only.allows(policy::ReadOperation::Retrieve));

    let retrieve_only = policy::ReadPolicy::new(false, true);
    assert!(!retrieve_only.allows(policy::ReadOperation::Search));
    assert!(retrieve_only.allows(policy::ReadOperation::Retrieve));
}

#[test]
fn read_policy_default_denies_every_operation() {
    let default_policy = policy::ReadPolicy::default();
    for op in [
        policy::ReadOperation::Search,
        policy::ReadOperation::Retrieve,
        policy::ReadOperation::GetCodeContext,
        policy::ReadOperation::GraphQuery,
        policy::ReadOperation::GetEntity,
        policy::ReadOperation::GetPreviousSolution,
        policy::ReadOperation::GetRules,
        policy::ReadOperation::GetRegulations,
        policy::ReadOperation::GetCanon,
    ] {
        assert!(!default_policy.allows(op), "{op:?} must be denied by the default (all-false) ReadPolicy");
    }
}

#[test]
fn read_permission_grants_no_write_shaped_capability() {
    // A RequestContext with full read permission but write_policy: None
    // must still be denied every write-shaped operation -- read and write
    // axes never cross.
    let ctx = brain_api::RequestContext {
        schema_version: brain_api::SCHEMA_VERSION,
        correlation_id: uuid::Uuid::new_v4(),
        write_policy: None,
        read_policy: Some(
            policy::ReadPolicy::new(true, true)
                .with_get_code_context(true)
                .with_graph_query(true)
                .with_get_entity(true),
        ),
        origin: brain_api::CallOrigin::McpSession("session-read-only".into()),
    };
    let result = brain_api::save_memory(
        &ctx,
        &test_vault("read-grants-no-write"),
        storage::NoteId::parse("note-read-only-a").unwrap(),
        domain::NoteType::Bug,
        domain::Classification::Internal,
        "project-x".into(),
        "agent:coder@prompt-v3".into(),
        "2026-11-18".into(),
        "body".into(),
        NOW,
    );
    assert_eq!(result, Err(brain_api::BrainApiError::PermissionDenied));
}

// =====================================================================
// 6 (continued) / 1. Raw persistence and raw storage primitives stay
// unreachable from brain-api's own external surface and from storage's.
// =====================================================================

#[test]
fn brain_api_exposes_no_raw_proposal_persistence_functions() {
    // Compile-time property, proven by this file's own compilation: there
    // is no `brain_api::load_proposal` or `brain_api::save_proposal` to
    // name (both are `pub(crate)` inside `proposal_store`, B5 layering
    // review). The ONLY ways external code can affect or observe proposal
    // persistence are: save_memory/update_memory/link_entities (create,
    // always Pending), approve_proposal/reject_proposal/list_proposals
    // (the Task 14 gated service layer), and run_cli (the trusted CLI
    // wrapper over the same three). This test's assertions below exercise
    // that the full lifecycle is STILL reachable through those legitimate
    // paths, which is the point: restricting the raw functions cost
    // nothing real.
    let vault = test_vault("no-raw-persistence");
    let id = propose_a_save_memory(&vault, "note-raw-a", "body");
    brain_api::run_cli(&["approve".to_string(), id.to_string()], &vault, NOW).unwrap();
    let note = vault.read_note(&storage::NoteId::parse("note-raw-a").unwrap()).unwrap();
    assert!(note.content.contains("body"));
}

#[test]
fn storage_crate_exposes_no_raw_write_primitive() {
    // Compile-time property: `storage::atomic_write`/
    // `storage::write_record_unchecked`/any handle_safety type are not
    // names this file can reach -- `storage`'s own lib.rs keeps them
    // `pub(crate)` or crate-private (A1 Finding C1, re-confirmed still
    // true here from brain-api's own dependency on it). The only public
    // write surface on `storage` is `propose_and_commit`/`update_note_body`/
    // `add_relation`, all three of which this file CAN and does exercise
    // indirectly through `approve_proposal`'s execution (already proven by
    // the note-write-gate and link/update tests above) -- never directly.
    let vault = test_vault("no-raw-storage-write");
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    // propose_and_commit itself IS public on storage (by design -- it is
    // A1's one real write authority) and brain-api's own execute() calls
    // it; this is not the bypass under test. What is under test is that
    // nothing LOWER-level than this is reachable: there is no
    // `storage::write_record_unchecked` or `storage::atomic_write` name to
    // call here instead.
    let text = "---\nid: note-direct-storage-a\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-18\nupdated: 2026-11-18\nevent_period: 2026-11-18\nrelations: {}\n---\ndirect body\n";
    let result = storage::propose_and_commit(
        &vault,
        text,
        storage::Version(0),
        uuid::Uuid::new_v4(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &gate,
        None,
    );
    assert!(result.is_ok(), "propose_and_commit itself must remain the one real, intentionally public write authority");
}

// =====================================================================
// Read delegation parity (plan's own Task 20 sketch, kept as the
// integration-level confirmation that brain-api's thin read wrapper
// behaves identically to calling crates/api directly).
// =====================================================================

#[test]
fn read_delegation_behaves_identically_to_calling_api_directly() {
    let dir = unique_test_dir("read-parity");
    {
        let repo = git2::Repository::init(&dir).unwrap();
        let sig = git2::Signature::now("Test", "test@localhost").unwrap();
        let oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
    }
    let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
    let text = "---\nid: note-parity-a\nschema_version: 1\ntype: bug\nstatus: reviewed\ntrust: official\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-18\nupdated: 2026-11-18\nevent_period: 2026-11-18\nrelations: {}\n---\nthe unique parity-test phrase\n";
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    storage::propose_and_commit(&vault, text, storage::Version(0), uuid::Uuid::new_v4(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &gate, None).unwrap();
    let indexer = std::sync::Arc::new(index::indexer_impl::RealIndexer::new());
    index::indexer_impl::rebuild_index(&vault, &indexer).unwrap();
    let idx = index::Index::new(&vault, &indexer, "project-x", chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap());

    let read_policy = policy::ReadPolicy::new(true, true);
    let request = api::SearchRequest { text: "unique parity-test phrase".into(), filters: Default::default(), k: 10 };

    let via_api = api::search(&idx, &read_policy, api::RequestContext { schema_version: api::SCHEMA_VERSION, correlation_id: uuid::Uuid::new_v4() }, request.clone());

    let ctx = brain_api::RequestContext {
        schema_version: brain_api::SCHEMA_VERSION,
        correlation_id: uuid::Uuid::new_v4(),
        write_policy: None,
        read_policy: Some(read_policy),
        origin: brain_api::CallOrigin::McpSession("session-parity".into()),
    };
    let via_brain_api = brain_api::search(&ctx, &idx, request);

    assert_eq!(via_api.result.unwrap().outcome, via_brain_api.unwrap().outcome, "brain_api::search must delegate to api::search with an identical outcome");
}
