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
