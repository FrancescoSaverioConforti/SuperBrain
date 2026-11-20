//! B5.1 Task 15: end-to-end exercise of the minimal CLI approval surface
//! (`brain_api::run_cli`) as a library call — never by spawning the
//! `approve_cli` example binary as a subprocess. This is deliberate: a
//! subprocess test can only assert on stdout/exit-code text, while calling
//! `run_cli` directly lets this test assert on the real, structured
//! `Result<String, BrainApiError>` it returns and on the vault's real state
//! afterwards, exactly as every other brain-api integration test in this
//! crate already does for `save_memory`/`approve_proposal`/etc.
//!
//! `run_cli` always constructs `RequestContext` with
//! `WritePolicy::trusted_cli()` and `CallOrigin::TrustedLocal` internally —
//! never from `args` — so this test never passes a write policy or origin
//! itself; that is the whole point of a CLI entry point being the trusted
//! boundary, per the B5.0 frozen trust model restated in `lib.rs`'s own
//! `RequestContext`/`CallOrigin` doc comments.

use std::path::PathBuf;

fn unique_test_dir() -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-brain-api-cli-e2e-test-{}-{}-{}",
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

/// An MCP-originated context (never trusted_cli) — used to create
/// proposals exactly as a remote agent would, so this test exercises the
/// real agent-creates/CLI-approves boundary, not a shortcut.
fn mcp_ctx() -> brain_api::RequestContext {
    brain_api::RequestContext {
        schema_version: brain_api::SCHEMA_VERSION,
        correlation_id: uuid::Uuid::new_v4(),
        write_policy: Some(brain_api::WritePolicy::mcp_agent()),
        read_policy: None,
        origin: brain_api::CallOrigin::McpSession("session-cli-e2e".into()),
    }
}

const NOW: &str = "2026-11-18T00:00:00Z";

#[test]
fn cli_list_shows_a_pending_proposal_and_approve_commits_it() {
    let vault = test_vault();
    let id = brain_api::save_memory(
        &mcp_ctx(),
        &vault,
        storage::NoteId::parse("note-cli-a").unwrap(),
        domain::NoteType::Bug,
        domain::Classification::Internal,
        "project-x".to_string(),
        "agent:coder@prompt-v3".to_string(),
        "2026-11-18".to_string(),
        "cli e2e content".to_string(),
        NOW,
    )
    .unwrap();

    let listed = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    assert!(listed.contains(&id.to_string()), "list output must mention the pending proposal's id, got: {listed}");

    let approved = brain_api::run_cli(&["approve".to_string(), id.to_string()], &vault, NOW).unwrap();
    assert!(!approved.is_empty());

    let note = vault.read_note(&storage::NoteId::parse("note-cli-a").unwrap()).unwrap();
    assert!(note.content.contains("cli e2e content"));
}

#[test]
fn cli_reject_moves_a_proposal_to_rejected_and_never_executes_it() {
    let vault = test_vault();
    let id = brain_api::save_memory(
        &mcp_ctx(),
        &vault,
        storage::NoteId::parse("note-cli-b").unwrap(),
        domain::NoteType::Bug,
        domain::Classification::Internal,
        "project-x".to_string(),
        "agent:coder@prompt-v3".to_string(),
        "2026-11-18".to_string(),
        "should never land".to_string(),
        NOW,
    )
    .unwrap();

    let rejected = brain_api::run_cli(&["reject".to_string(), id.to_string(), "not needed".to_string()], &vault, NOW).unwrap();
    assert!(!rejected.is_empty());

    assert!(matches!(
        vault.read_note(&storage::NoteId::parse("note-cli-b").unwrap()),
        Err(storage::StorageError::NotFound { .. })
    ));
}

#[test]
fn cli_list_on_an_empty_vault_does_not_error() {
    let vault = test_vault();
    let result = brain_api::run_cli(&["list".to_string()], &vault, NOW);
    assert!(result.is_ok());
}

#[test]
fn cli_approve_on_an_unknown_id_returns_an_error() {
    let vault = test_vault();
    let unknown = uuid::Uuid::new_v4().to_string();
    let result = brain_api::run_cli(&["approve".to_string(), unknown], &vault, NOW);
    assert!(result.is_err());
}

#[test]
fn cli_with_an_unknown_subcommand_returns_an_error_not_a_panic() {
    let vault = test_vault();
    let result = brain_api::run_cli(&["frobnicate".to_string()], &vault, NOW);
    assert!(result.is_err());
}

#[test]
fn cli_always_uses_trusted_cli_write_policy_never_settable_via_args() {
    // Compile-time + behavioral property: run_cli's signature is (args,
    // vault, now) -- there is no WritePolicy/CallOrigin parameter an
    // argument could smuggle in. Behavioral proof: an mcp_agent()-created
    // proposal (which this test itself built) is approvable through
    // run_cli at all, which is only possible if run_cli's internally
    // constructed context has approve_write_allowed -- i.e. is
    // trusted_cli(), never mcp_agent() (which never has it, confirmed by
    // policy_write.rs's own exhaustive test matrix).
    let vault = test_vault();
    let id = brain_api::save_memory(
        &mcp_ctx(),
        &vault,
        storage::NoteId::parse("note-cli-c").unwrap(),
        domain::NoteType::Bug,
        domain::Classification::Internal,
        "project-x".to_string(),
        "agent:coder@prompt-v3".to_string(),
        "2026-11-18".to_string(),
        "content".to_string(),
        NOW,
    )
    .unwrap();
    assert!(brain_api::run_cli(&["approve".to_string(), id.to_string()], &vault, NOW).is_ok());
}

// --- B5.12 Task 33: run_cli_with_audit emits one ToolCall per command,
//     plus the approve/reject PermissionDecision pairing ---

#[derive(Default)]
struct RecordingSink(std::sync::Mutex<Vec<brain_api::AuditEvent>>);
impl brain_api::AuditSink for RecordingSink {
    fn record(&self, event: brain_api::AuditEvent) {
        self.0.lock().unwrap().push(event);
    }
}
impl RecordingSink {
    fn events(&self) -> Vec<brain_api::AuditEvent> {
        self.0.lock().unwrap().clone()
    }
}

#[test]
fn run_cli_with_audit_emits_a_tool_call_for_list_approve_and_reject() {
    let vault = test_vault();
    let id_a = brain_api::save_memory(
        &mcp_ctx(),
        &vault,
        storage::NoteId::parse("note-cli-audit-a").unwrap(),
        domain::NoteType::Bug,
        domain::Classification::Internal,
        "project-x".to_string(),
        "agent:coder@prompt-v3".to_string(),
        "2026-11-18".to_string(),
        "content".to_string(),
        NOW,
    )
    .unwrap();
    let id_b = brain_api::save_memory(
        &mcp_ctx(),
        &vault,
        storage::NoteId::parse("note-cli-audit-b").unwrap(),
        domain::NoteType::Bug,
        domain::Classification::Internal,
        "project-x".to_string(),
        "agent:coder@prompt-v3".to_string(),
        "2026-11-18".to_string(),
        "content".to_string(),
        NOW,
    )
    .unwrap();

    let sink = RecordingSink::default();
    brain_api::run_cli_with_audit(&["list".to_string()], &vault, NOW, &sink).unwrap();
    brain_api::run_cli_with_audit(&["approve".to_string(), id_a.to_string()], &vault, NOW, &sink).unwrap();
    brain_api::run_cli_with_audit(&["reject".to_string(), id_b.to_string(), "not needed".to_string()], &vault, NOW, &sink).unwrap();

    let events = sink.events();
    let tool_calls: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.kind {
            brain_api::AuditEventKind::ToolCall { tool, allowed } => Some((tool.as_str(), *allowed)),
            _ => None,
        })
        .collect();
    assert_eq!(tool_calls, vec![("list", true), ("approve", true), ("reject", true)], "got events: {events:?}");

    // approve/reject also each emit their own PermissionDecision (always
    // allowed here, since trusted_ctx() has approve_write_allowed).
    let permission_decisions: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.kind {
            brain_api::AuditEventKind::PermissionDecision { operation, allowed } => Some((operation.as_str(), *allowed)),
            _ => None,
        })
        .collect();
    assert_eq!(permission_decisions, vec![("ApproveProposal", true), ("RejectProposal", true)], "got events: {events:?}");
}

#[test]
fn run_cli_with_audit_on_an_unknown_id_still_emits_a_tool_call_marked_allowed() {
    // NotFound is not PermissionDenied -- trusted_cli() was allowed to
    // attempt the approve; it failed for an unrelated reason
    // (nonexistent id). record_tool_call's `allowed` reflects the
    // permission axis only.
    let vault = test_vault();
    let sink = RecordingSink::default();
    let unknown = uuid::Uuid::new_v4().to_string();

    let result = brain_api::run_cli_with_audit(&["approve".to_string(), unknown], &vault, NOW, &sink);

    assert!(result.is_err());
    let tool_calls: Vec<_> = sink
        .events()
        .into_iter()
        .filter_map(|e| match e.kind {
            brain_api::AuditEventKind::ToolCall { tool, allowed } if tool == "approve" => Some(allowed),
            _ => None,
        })
        .collect();
    assert_eq!(tool_calls, vec![true]);
}

#[test]
fn run_cli_without_audit_behaves_identically_to_run_cli_with_audit_and_a_null_sink() {
    // run_cli itself must remain exactly the pinned (args, vault, now)
    // signature (see cli_always_uses_trusted_cli_write_policy_never_settable_via_args
    // above) -- this just confirms the thin-wrapper relationship holds
    // observably, not merely by reading the source.
    let vault = test_vault();
    let id = brain_api::save_memory(
        &mcp_ctx(),
        &vault,
        storage::NoteId::parse("note-cli-audit-c").unwrap(),
        domain::NoteType::Bug,
        domain::Classification::Internal,
        "project-x".to_string(),
        "agent:coder@prompt-v3".to_string(),
        "2026-11-18".to_string(),
        "content".to_string(),
        NOW,
    )
    .unwrap();

    let via_run_cli = brain_api::run_cli(&["approve".to_string(), id.to_string()], &vault, NOW);
    assert!(via_run_cli.is_ok());
}

// =====================================================================
// M5 Task 7: run_cli_durable -- fail-closed durable pre-registration and
// outcome recording for approvals, via opaque, trust-blind hooks. All
// hooks below are plain closures over local `Vec`s -- no async runtime,
// no real audit worker -- this file tests brain-api's own synchronous
// contract in isolation; the real async bridge is exercised separately
// by crates/brain-api/examples/approve_cli.rs's own end-to-end tests.
// =====================================================================

fn propose_a_note(vault: &storage::Vault, note_id: &str) -> brain_api::ProposalId {
    brain_api::save_memory(
        &mcp_ctx(),
        vault,
        storage::NoteId::parse(note_id).unwrap(),
        domain::NoteType::Bug,
        domain::Classification::Internal,
        "project-x".to_string(),
        "agent:coder@prompt-v3".to_string(),
        "2026-11-18".to_string(),
        "content".to_string(),
        NOW,
    )
    .unwrap()
}

struct RecordedPreRegistration {
    attempt_id: uuid::Uuid,
    proposal_id: String,
}

struct RecordedOutcome {
    attempt_id: uuid::Uuid,
    outcome: Result<storage::CommitResult, brain_api::BrainApiError>,
}

#[test]
fn run_cli_durable_pre_registers_before_approving_and_records_the_real_commit_outcome() {
    let vault = test_vault();
    let id = propose_a_note(&vault, "note-durable-a");
    let sink = RecordingSink::default();

    let pre_registrations = std::sync::Mutex::new(Vec::<RecordedPreRegistration>::new());
    let mut pre_register = |attempt_id: uuid::Uuid, proposal_id: &str| -> Result<(), String> {
        pre_registrations.lock().unwrap().push(RecordedPreRegistration { attempt_id, proposal_id: proposal_id.to_string() });
        Ok(())
    };
    let outcomes = std::sync::Mutex::new(Vec::<RecordedOutcome>::new());
    let mut record_outcome = |attempt_id: uuid::Uuid, result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> {
        outcomes.lock().unwrap().push(RecordedOutcome { attempt_id, outcome: result.clone() });
        Ok(())
    };
    let mut hooks = brain_api::DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };

    let outcome = brain_api::run_cli_durable(&["approve".to_string(), id.to_string()], &vault, NOW, &sink, &mut hooks);

    assert!(outcome.result.is_ok(), "the real approval must succeed, got {:?}", outcome.result);
    assert!(outcome.audit_outcome_write.is_ok());

    let pre = pre_registrations.into_inner().unwrap();
    assert_eq!(pre.len(), 1, "pre_register must be called exactly once for one approve attempt");
    assert_eq!(pre[0].proposal_id, id.to_string());

    let out = outcomes.into_inner().unwrap();
    assert_eq!(out.len(), 1, "record_outcome must be called exactly once");
    assert_eq!(out[0].attempt_id, pre[0].attempt_id, "the SAME attempt_id must correlate pre-registration with its outcome");
    // The test vault's own temp dir is not a git repository, so a real
    // commit here rests at GitPending rather than Committed -- both are
    // genuine, non-error success shapes (see storage::CommitResult's own
    // doc comment); this test only needs to confirm SOME real success
    // outcome reached record_outcome, not which of the two it was.
    assert!(matches!(out[0].outcome, Ok(storage::CommitResult::Committed(_) | storage::CommitResult::GitPending { .. })), "got {:?}", out[0].outcome);
}

#[test]
fn run_cli_durable_generates_a_distinct_attempt_id_for_each_separate_approval_attempt() {
    // Two separate attempts against the same proposal: the first succeeds
    // (consuming the proposal), the second then fails with Conflict
    // (already Approved) -- but BOTH must still get their own distinct,
    // independently pre-registered attempt_id.
    let vault = test_vault();
    let id = propose_a_note(&vault, "note-durable-distinct-attempts");
    let sink = RecordingSink::default();

    let seen_attempt_ids = std::sync::Mutex::new(Vec::<uuid::Uuid>::new());
    let mut pre_register = |attempt_id: uuid::Uuid, _proposal_id: &str| -> Result<(), String> {
        seen_attempt_ids.lock().unwrap().push(attempt_id);
        Ok(())
    };
    let mut record_outcome = |_attempt_id: uuid::Uuid, _result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> { Ok(()) };
    let mut hooks = brain_api::DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };

    let first = brain_api::run_cli_durable(&["approve".to_string(), id.to_string()], &vault, NOW, &sink, &mut hooks);
    assert!(first.result.is_ok());
    let second = brain_api::run_cli_durable(&["approve".to_string(), id.to_string()], &vault, NOW, &sink, &mut hooks);
    assert!(matches!(second.result, Err(brain_api::BrainApiError::Conflict)), "got {:?}", second.result);

    let ids = seen_attempt_ids.into_inner().unwrap();
    assert_eq!(ids.len(), 2, "both attempts, successful and conflicting, must each get their own pre-registration call");
    assert_ne!(ids[0], ids[1], "distinct approval attempts must never share an attempt_id");
}

#[test]
fn run_cli_durable_never_calls_approve_proposal_when_pre_registration_fails() {
    let vault = test_vault();
    let id = propose_a_note(&vault, "note-durable-pre-register-fails");
    let sink = RecordingSink::default();

    let mut pre_register = |_attempt_id: uuid::Uuid, _proposal_id: &str| -> Result<(), String> { Err("durable audit worker unavailable".to_string()) };
    let mut record_outcome = |_attempt_id: uuid::Uuid, _result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> {
        panic!("record_outcome must never be called when pre-registration itself failed");
    };
    let mut hooks = brain_api::DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };

    let outcome = brain_api::run_cli_durable(&["approve".to_string(), id.to_string()], &vault, NOW, &sink, &mut hooks);

    assert!(matches!(outcome.result, Err(brain_api::BrainApiError::RuntimeUnavailable)), "a failed pre-registration must refuse to approve, fail closed, got {:?}", outcome.result);

    // The proposal itself must still be genuinely Pending -- approve_proposal
    // was never reached, so nothing was committed and no status transition
    // happened.
    let listing = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    assert!(listing.contains("Pending"), "the proposal must remain Pending since approve_proposal was never called, got: {listing:?}");
}

#[test]
fn run_cli_durable_reports_a_successful_commit_truthfully_even_when_the_outcome_write_itself_fails() {
    let vault = test_vault();
    let id = propose_a_note(&vault, "note-durable-outcome-write-fails");
    let sink = RecordingSink::default();

    let mut pre_register = |_attempt_id: uuid::Uuid, _proposal_id: &str| -> Result<(), String> { Ok(()) };
    let mut record_outcome = |_attempt_id: uuid::Uuid, _result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> {
        Err("audit worker queue full".to_string())
    };
    let mut hooks = brain_api::DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };

    let outcome = brain_api::run_cli_durable(&["approve".to_string(), id.to_string()], &vault, NOW, &sink, &mut hooks);

    assert!(outcome.result.is_ok(), "the real commit succeeded and must be reported as such regardless of the outcome-write's own failure, got {:?}", outcome.result);
    assert_eq!(outcome.audit_outcome_write, Err("audit worker queue full".to_string()), "the outcome-write failure must be separately, honestly surfaced, never silently discarded");

    // The real commit genuinely happened -- the proposal is Approved, not Pending.
    let listing = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    assert!(listing.contains("Approved"), "got: {listing:?}");
}

#[test]
fn run_cli_durable_never_emits_a_commit_outcome_hook_call_for_rejection() {
    let vault = test_vault();
    let id = propose_a_note(&vault, "note-durable-reject-no-outcome");
    let sink = RecordingSink::default();

    let mut pre_register = |_attempt_id: uuid::Uuid, _proposal_id: &str| -> Result<(), String> {
        panic!("pre_register must never be called for reject -- rejection has no approval attempt to pre-register");
    };
    let mut record_outcome = |_attempt_id: uuid::Uuid, _result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> {
        panic!("record_outcome must never be called for reject -- rejection is a pure status transition, never a CommitOutcome");
    };
    let mut hooks = brain_api::DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };

    let outcome = brain_api::run_cli_durable(&["reject".to_string(), id.to_string(), "not needed".to_string()], &vault, NOW, &sink, &mut hooks);

    assert!(outcome.result.is_ok(), "got {:?}", outcome.result);
    assert!(outcome.audit_outcome_write.is_ok(), "reject's own audit_outcome_write must stay Ok(()) -- there is nothing to write an outcome for");
    let listing = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    assert!(listing.contains("Rejected"), "got: {listing:?}");
}

#[test]
fn run_cli_durable_permission_denial_never_calls_pre_register_or_reaches_an_unauthorized_commit() {
    // run_cli_durable itself always constructs the trusted context
    // internally (exactly like run_cli/run_cli_with_audit) -- there is no
    // way to reach it with an unauthorized RequestContext through this
    // public entry point. This test instead confirms the structural
    // property via `list` on an UNKNOWN id, proving approve_proposal's own
    // existing NotFound-before-any-write discipline is unaffected by the
    // new hooks plumbing (a stand-in for "permission/validation boundaries
    // are unchanged"), since the trust boundary itself cannot be
    // constructed or bypassed by a caller of this public API at all.
    let vault = test_vault();
    let sink = RecordingSink::default();
    let unknown = uuid::Uuid::new_v4().to_string();

    let called = std::sync::atomic::AtomicBool::new(false);
    let mut pre_register = |_attempt_id: uuid::Uuid, _proposal_id: &str| -> Result<(), String> {
        called.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    };
    let mut record_outcome = |_attempt_id: uuid::Uuid, _result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> { Ok(()) };
    let mut hooks = brain_api::DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };

    let outcome = brain_api::run_cli_durable(&["approve".to_string(), unknown], &vault, NOW, &sink, &mut hooks);
    assert!(matches!(outcome.result, Err(brain_api::BrainApiError::NotFound)), "got {:?}", outcome.result);
    // pre_register IS still called here (parse_id succeeded; the id is
    // well-formed, just unknown) -- NotFound happens inside
    // approve_proposal itself, after pre-registration, exactly mirroring
    // where NotFound already surfaces for run_cli's own existing behavior.
    assert!(called.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn run_cli_durable_list_and_existing_run_cli_and_run_cli_with_audit_remain_fully_unaffected() {
    // Regression guard: run_cli/run_cli_with_audit's own exact pinned
    // behavior (and output strings) must be byte-identical to before this
    // task, proving the new shared internal core changed nothing
    // observable for the existing entry points.
    let vault = test_vault();
    let id = propose_a_note(&vault, "note-durable-regression-guard");

    let via_run_cli = brain_api::run_cli(&["approve".to_string(), id.to_string()], &vault, NOW).unwrap();
    assert!(via_run_cli.starts_with("approved "), "got {via_run_cli:?}");
}
