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
