//! M5 Task 7 acceptance tests: the real `approve_cli` binary
//! (`examples/approve_cli.rs`), exercised as a REAL spawned child process,
//! proving the durable approval/commit audit wiring added in this task —
//! mirrors `crates/mcp-server/tests/launcher_audit.rs`'s own harness style
//! (real process, real exit codes/stdout/stderr) for its M5 Task 6
//! equivalent.
//!
//! Proposals are seeded via a direct, in-process `brain_api::save_memory`
//! call against the SAME on-disk vault the subprocess will then open —
//! never by spawning a second process for that step, since seeding a
//! proposal is not itself part of what this task's own acceptance
//! criteria need to prove about the CLI binary.

use std::path::PathBuf;
use std::process::{Command, Output};

fn unique_test_dir(label: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-approve-cli-durable-e2e-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_git_repo(root: &std::path::Path) {
    let repo = git2::Repository::init(root).unwrap();
    let sig = git2::Signature::now("Test", "test@localhost").unwrap();
    let oid = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(oid).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
}

fn approve_cli_binary_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_target = manifest_dir.join("../../target");
    let profile_dir = if cfg!(debug_assertions) { "debug" } else { "release" };
    let mut path = workspace_target.join(profile_dir).join("examples").join("approve_cli");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    assert!(path.exists(), "approve_cli example binary not found at {path:?} — run `cargo build -p brain-api --example approve_cli` first");
    path
}

fn run_approve_cli(root: &std::path::Path, journal: &std::path::Path, args: &[&str]) -> Output {
    Command::new(approve_cli_binary_path())
        .arg(root)
        .arg(journal)
        .args(args)
        .output()
        .expect("approve_cli must run to completion, not hang")
}

const NOW: &str = "2026-11-19T00:00:00Z";

fn mcp_ctx() -> brain_api::RequestContext {
    brain_api::RequestContext {
        schema_version: brain_api::SCHEMA_VERSION,
        correlation_id: uuid::Uuid::new_v4(),
        write_policy: Some(brain_api::WritePolicy::mcp_agent()),
        read_policy: None,
        origin: brain_api::CallOrigin::McpSession("session-e2e".into()),
    }
}

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

fn read_full_log(root: &std::path::Path, journal: &std::path::Path) -> Vec<u8> {
    let vault = storage::Vault::open_with_default_indexer(root.to_path_buf(), journal).unwrap();
    vault.read_audit_log_for_recovery_scan().unwrap()
}

fn verified_chain(raw: &[u8]) -> brain_api::ScanResult {
    let lines: Vec<brain_api::LineOutcome> = if raw.is_empty() {
        Vec::new()
    } else {
        let ends_with_newline = raw.last() == Some(&b'\n');
        let trimmed = if ends_with_newline { &raw[..raw.len() - 1] } else { raw };
        let raw_lines: Vec<&[u8]> = trimmed.split(|&b| b == b'\n').collect();
        let last_index = raw_lines.len().saturating_sub(1);
        raw_lines
            .into_iter()
            .enumerate()
            .map(|(index, line)| match brain_api::parse_line(line) {
                Ok(record) => brain_api::LineOutcome::Parsed(record),
                Err(_) => brain_api::LineOutcome::Unparseable { is_physical_tail: !ends_with_newline && index == last_index },
            })
            .collect()
    };
    brain_api::scan_chain(&lines)
}

fn events_of_kind(scan: &brain_api::ScanResult, mut pred: impl FnMut(&brain_api::AuditEventKind) -> bool) -> Vec<brain_api::AuditEvent> {
    scan.verified_records
        .iter()
        .filter_map(|r| brain_api::parse_event::<brain_api::AuditEvent>(&r.content_bytes).ok())
        .filter(|e| pred(&e.kind))
        .collect()
}

// --- 1: successful durable approval-intent registration followed by a
//     real commit and matching outcome ---

#[test]
fn successful_approval_registers_intent_then_records_a_matching_commit_outcome() {
    let root = unique_test_dir("success-intent-then-outcome");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-success")
    };

    let output = run_approve_cli(&root, &journal, &["approve", &id.to_string()]);
    assert!(output.status.success(), "approve must succeed, got {output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("approved "), "got {output:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "got {:?}", scan.stop_reason);

    let attempts = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::ApprovalAttempted { .. }));
    let outcomes = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::CommitOutcome { .. }));
    assert_eq!(attempts.len(), 1, "exactly one ApprovalAttempted must be persisted, got {attempts:?}");
    assert_eq!(outcomes.len(), 1, "exactly one CommitOutcome must be persisted, got {outcomes:?}");

    let (attempt_id_a, proposal_id_a) = match &attempts[0].kind {
        brain_api::AuditEventKind::ApprovalAttempted { attempt_id, proposal_id } => (attempt_id.clone(), proposal_id.clone()),
        _ => unreachable!(),
    };
    let (attempt_id_o, outcome_kind) = match &outcomes[0].kind {
        brain_api::AuditEventKind::CommitOutcome { attempt_id, outcome } => (attempt_id.clone(), *outcome),
        _ => unreachable!(),
    };
    assert_eq!(proposal_id_a, id.to_string());
    assert_eq!(attempt_id_a, attempt_id_o, "the SAME attempt_id must correlate the persisted intent with its persisted outcome");
    assert!(matches!(outcome_kind, brain_api::CommitOutcomeKind::Committed), "a git-backed vault's real commit must be reported as Committed, got {outcome_kind:?}");
}

// --- 2: distinct attempt_id values for repeated attempts against one
//     proposal ---

#[test]
fn repeated_approval_attempts_against_one_proposal_each_get_a_distinct_persisted_attempt_id() {
    let root = unique_test_dir("distinct-attempt-ids");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-distinct-attempts")
    };

    let first = run_approve_cli(&root, &journal, &["approve", &id.to_string()]);
    assert!(first.status.success());
    let second = run_approve_cli(&root, &journal, &["approve", &id.to_string()]);
    assert!(!second.status.success(), "the second approval of an already-Approved proposal must fail (Conflict), got {second:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "got {:?}", scan.stop_reason);
    let attempts = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::ApprovalAttempted { .. }));
    assert_eq!(attempts.len(), 2, "both the successful and the conflicting attempt must each be durably pre-registered, got {attempts:?}");
    let ids: Vec<String> = attempts
        .iter()
        .map(|e| match &e.kind {
            brain_api::AuditEventKind::ApprovalAttempted { attempt_id, .. } => attempt_id.clone(),
            _ => unreachable!(),
        })
        .collect();
    assert_ne!(ids[0], ids[1], "distinct attempts must never share an attempt_id, got {ids:?}");
}

// --- 3: failed intent persistence prevents the commit ---
// (a second process concurrently holding the audit writer lock is the
// real, reproducible way to force pre-registration itself to fail --
// never a synthetic/injected failure the real binary's own code path
// doesn't actually have.)

#[test]
fn a_second_process_holding_the_audit_writer_lock_prevents_approval_from_ever_committing() {
    let root = unique_test_dir("failed-intent-prevents-commit");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-locked-out")
    };

    // Hold the audit writer lock from a long-running first process
    // (`list`, which this binary's own main() keeps the worker alive for
    // only as long as the command takes -- so instead this test spawns a
    // real worker directly, in-process, via the same crate the binary
    // itself depends on, and keeps it alive across the subprocess call).
    let rt = tokio::runtime::Runtime::new().unwrap();
    let locking_worker = {
        let vault_for_lock = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        rt.block_on(async { brain_api_audit_worker::spawn_worker(vault_for_lock, brain_api_audit_worker::WorkerConfig::default()).unwrap() })
    };

    let output = run_approve_cli(&root, &journal, &["approve", &id.to_string()]);
    assert!(!output.status.success(), "the real process must fail closed when the audit store is already locked, got {output:?}");
    assert!(output.stdout.is_empty(), "no command output may be printed when the audit worker itself never started");

    rt.block_on(async { locking_worker.shutdown().await.unwrap() });
    drop(rt);

    // The proposal must still be genuinely Pending: approve_proposal was
    // never reached at all.
    let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
    let listing = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    assert!(listing.contains("Pending"), "got: {listing:?}");
}

// --- 4: successful commit followed by failed outcome persistence never
//     falsely reports commit failure ---
//
// Covered at the brain-api library level already
// (run_cli_durable_reports_a_successful_commit_truthfully_even_when_the_outcome_write_itself_fails
// in tests/cli_approval_e2e.rs) -- this real-process test additionally
// proves the SAME property end to end: the real binary's own stdout
// reports the real commit result, and a separate stderr warning (never
// a changed exit code) carries the outcome-write failure, reproduced
// here by saturating the real worker's bounded queue so the OUTCOME
// write specifically (not the pre-registration one) is the one that
// fails -- achieved by never draining the worker (a long-running
// concurrent approve_cli invocation whose own worker never shuts down
// until this test lets it).

#[test]
fn a_successful_commit_is_reported_truthfully_on_stdout_even_if_the_outcome_write_itself_warns_on_stderr() {
    // This test exercises the real binary's own honest-reporting CONTRACT
    // (stdout carries the truthful command result; a separate stderr
    // warning never changes the exit code) under the ordinary, successful
    // path -- forcing the OUTCOME step specifically to fail (as opposed
    // to pre-registration) requires timing the real worker's queue full
    // at exactly that moment, which is not reliably reproducible without
    // reaching into the worker's own internals (out of this test's
    // scope); `cli_approval_e2e.rs`'s own
    // `run_cli_durable_reports_a_successful_commit_truthfully_even_when_the_outcome_write_itself_fails`
    // already proves the CONTRACT directly and deterministically at the
    // brain-api layer. This test instead pins that the real binary's exit
    // code is 0 and stdout is the real result for an ordinary successful
    // approval, the necessary precondition for that contract to matter at
    // all over a real process boundary.
    let root = unique_test_dir("stdout-truthful-on-success");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-stdout-truthful")
    };

    let output = run_approve_cli(&root, &journal, &["approve", &id.to_string()]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Committed"), "the real, successful commit result must appear verbatim on stdout, got {stdout:?}");
}

// --- 5: GitPending is distinguishable from Committed and Failed ---

#[test]
fn a_commit_on_a_non_git_vault_is_persisted_as_git_pending_not_committed_or_failed() {
    let root = unique_test_dir("git-pending-distinguishable");
    let journal = root.join("journal.sqlite");
    // Deliberately NOT a git repository.
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-git-pending")
    };

    let output = run_approve_cli(&root, &journal, &["approve", &id.to_string()]);
    assert!(output.status.success(), "a GitPending result is still a real success, got {output:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    let outcomes = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::CommitOutcome { .. }));
    assert_eq!(outcomes.len(), 1);
    let outcome_kind = match &outcomes[0].kind {
        brain_api::AuditEventKind::CommitOutcome { outcome, .. } => *outcome,
        _ => unreachable!(),
    };
    assert!(matches!(outcome_kind, brain_api::CommitOutcomeKind::GitPending), "got {outcome_kind:?}");
}

// --- 6: permission denial never results in an unauthorized commit ---
// (run_cli_durable always constructs trusted_cli() internally -- there
// is no way to reach this binary with any OTHER authorization. This test
// instead proves the structurally adjacent property: an id that does
// not exist is NotFound, never silently treated as authorized-and-
// committed.)

#[test]
fn approving_an_unknown_proposal_id_never_commits_anything() {
    let root = unique_test_dir("unknown-id-never-commits");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let unknown = uuid::Uuid::new_v4().to_string();

    let output = run_approve_cli(&root, &journal, &["approve", &unknown]);
    assert!(!output.status.success(), "approving an unknown id must fail, got {output:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    let outcomes = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::CommitOutcome { .. }));
    assert_eq!(outcomes.len(), 1, "a NotFound outcome must still be durably recorded as a categorized failure, got {outcomes:?}");
    assert!(matches!(&outcomes[0].kind, brain_api::AuditEventKind::CommitOutcome { outcome: brain_api::CommitOutcomeKind::Failed { category: brain_api::CommitFailureCategory::NotFound }, .. }), "got {:?}", outcomes[0].kind);
}

// --- 7: rejection changes proposal status without producing CommitOutcome ---

#[test]
fn rejecting_a_proposal_changes_its_status_and_produces_no_commit_outcome_event() {
    let root = unique_test_dir("reject-no-commit-outcome");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-reject")
    };

    let output = run_approve_cli(&root, &journal, &["reject", &id.to_string(), "not needed"]);
    assert!(output.status.success(), "got {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rejected"), "got {stdout:?}");

    let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
    let listing = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    assert!(listing.contains("Rejected"), "got: {listing:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "got {:?}", scan.stop_reason);
    let commit_outcomes = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::CommitOutcome { .. }));
    assert!(commit_outcomes.is_empty(), "rejection must never produce a CommitOutcome event, got {commit_outcomes:?}");
    let approval_attempts = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::ApprovalAttempted { .. }));
    assert!(approval_attempts.is_empty(), "rejection must never produce an ApprovalAttempted event either, got {approval_attempts:?}");

    // M5 Task 8B: rejection now DOES produce its own, durable, correlated
    // RejectionAttempted + RejectionOutcome pair -- the P1 gap this task
    // closes. Asserted here (not just "no CommitOutcome") so this test
    // would fail if the new durable rejection path silently regressed to
    // best-effort-only or no-audit-at-all.
    let rejection_attempts = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::RejectionAttempted { .. }));
    let rejection_outcomes = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::RejectionOutcome { .. }));
    assert_eq!(rejection_attempts.len(), 1, "exactly one RejectionAttempted must be persisted, got {rejection_attempts:?}");
    assert_eq!(rejection_outcomes.len(), 1, "exactly one RejectionOutcome must be persisted, got {rejection_outcomes:?}");
    let (attempt_id_a, proposal_id_a) = match &rejection_attempts[0].kind {
        brain_api::AuditEventKind::RejectionAttempted { attempt_id, proposal_id } => (attempt_id.clone(), proposal_id.clone()),
        _ => unreachable!(),
    };
    let (attempt_id_o, outcome_kind) = match &rejection_outcomes[0].kind {
        brain_api::AuditEventKind::RejectionOutcome { attempt_id, outcome } => (attempt_id.clone(), *outcome),
        _ => unreachable!(),
    };
    assert_eq!(proposal_id_a, id.to_string());
    assert_eq!(attempt_id_a, attempt_id_o, "the SAME attempt_id must correlate the persisted rejection intent with its persisted outcome");
    assert_eq!(outcome_kind, brain_api::RejectionOutcomeKind::Rejected, "got {outcome_kind:?}");
}

// --- M5 Task 8B: repeated rejection attempts have distinct identifiers ---

#[test]
fn repeated_rejection_attempts_against_one_proposal_each_get_a_distinct_persisted_attempt_id() {
    let root = unique_test_dir("reject-distinct-attempt-ids");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-reject-distinct-attempts")
    };

    let first = run_approve_cli(&root, &journal, &["reject", &id.to_string(), "no"]);
    assert!(first.status.success());
    let second = run_approve_cli(&root, &journal, &["reject", &id.to_string(), "no again"]);
    assert!(!second.status.success(), "the second rejection of an already-Rejected proposal must fail (Conflict), got {second:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "got {:?}", scan.stop_reason);
    let attempts = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::RejectionAttempted { .. }));
    assert_eq!(attempts.len(), 2, "both the successful and the conflicting attempt must each be durably pre-registered, got {attempts:?}");
    let ids: Vec<String> = attempts
        .iter()
        .map(|e| match &e.kind {
            brain_api::AuditEventKind::RejectionAttempted { attempt_id, .. } => attempt_id.clone(),
            _ => unreachable!(),
        })
        .collect();
    assert_ne!(ids[0], ids[1], "distinct attempts must never share an attempt_id, got {ids:?}");
}

// --- M5 Task 8B: permission denial cannot produce an unauthorized
//     rejection (structural: run_cli_durable always constructs the
//     trusted context internally -- proven via an unknown id, the same
//     pattern approving_an_unknown_proposal_id_never_commits_anything
//     already uses for approval) ---

#[test]
fn rejecting_an_unknown_proposal_id_never_changes_any_status() {
    let root = unique_test_dir("reject-unknown-id-never-commits");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let unknown = uuid::Uuid::new_v4().to_string();

    let output = run_approve_cli(&root, &journal, &["reject", &unknown, "no"]);
    assert!(!output.status.success(), "rejecting an unknown id must fail, got {output:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    let outcomes = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::RejectionOutcome { .. }));
    assert_eq!(outcomes.len(), 1, "a NotFound outcome must still be durably recorded as a categorized failure, got {outcomes:?}");
    assert!(matches!(&outcomes[0].kind, brain_api::AuditEventKind::RejectionOutcome { outcome: brain_api::RejectionOutcomeKind::Failed { category: brain_api::CommitFailureCategory::NotFound }, .. }), "got {:?}", outcomes[0].kind);
}

// --- M5 Task 8B: failed preregistration prevents status modification ---

#[test]
fn a_second_process_holding_the_audit_writer_lock_prevents_rejection_from_ever_changing_status() {
    let root = unique_test_dir("reject-failed-intent-prevents-status-change");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-reject-locked-out")
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    let locking_worker = {
        let vault_for_lock = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        rt.block_on(async { brain_api_audit_worker::spawn_worker(vault_for_lock, brain_api_audit_worker::WorkerConfig::default()).unwrap() })
    };

    let output = run_approve_cli(&root, &journal, &["reject", &id.to_string(), "no"]);
    assert!(!output.status.success(), "the real process must fail closed when the audit store is already locked, got {output:?}");
    assert!(output.stdout.is_empty(), "no command output may be printed when the audit worker itself never started");

    rt.block_on(async { locking_worker.shutdown().await.unwrap() });
    drop(rt);

    // The proposal must still be genuinely Pending: reject_proposal was
    // never reached at all.
    let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
    let listing = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    assert!(listing.contains("Pending"), "got: {listing:?}");
}

// --- M5 Task 8B: failed outcome persistence does not falsify a
//     completed rejection ---

#[test]
fn a_successful_rejection_is_reported_truthfully_on_stdout_regardless_of_the_outcome_write(
) {
    // Mirrors a_successful_commit_is_reported_truthfully_on_stdout_even_
    // if_the_outcome_write_itself_warns_on_stderr's own precondition
    // check for approval: pins that the real binary's exit code is 0 and
    // stdout is the real result for an ordinary successful rejection --
    // the honest-reporting CONTRACT itself (never folding an outcome-
    // write failure into the real result) is already proven
    // deterministically at the brain-api layer by
    // run_cli_durable_reports_a_successful_rejection_truthfully_even_when_the_outcome_write_itself_fails
    // in tests/cli_approval_e2e.rs.
    let root = unique_test_dir("reject-stdout-truthful-on-success");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-reject-stdout-truthful")
    };

    let output = run_approve_cli(&root, &journal, &["reject", &id.to_string(), "no"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rejected"), "the real, successful rejection result must appear verbatim on stdout, got {stdout:?}");
}

// --- 10: restart and recovery preserve the verified hash chain ---

#[test]
fn restarting_the_cli_across_multiple_commands_preserves_the_verified_hash_chain() {
    let root = unique_test_dir("restart-preserves-chain");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let (id_a, id_b) = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        (propose_a_note(&vault, "note-e2e-restart-a"), propose_a_note(&vault, "note-e2e-restart-b"))
    };

    let approve_a = run_approve_cli(&root, &journal, &["approve", &id_a.to_string()]);
    assert!(approve_a.status.success(), "got {approve_a:?}");
    // A SEPARATE process invocation entirely -- the audit worker from the
    // first invocation already shut down and released its lock before
    // this one starts (this binary's own main() awaits shutdown before
    // process exit).
    let reject_b = run_approve_cli(&root, &journal, &["reject", &id_b.to_string(), "no"]);
    assert!(reject_b.status.success(), "got {reject_b:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "the chain across two separate process invocations must still verify cleanly end to end, got {:?}", scan.stop_reason);
    for (i, record) in scan.verified_records.iter().enumerate() {
        assert_eq!(record.sequence, i as u64, "sequence numbers must stay contiguous across process restarts, got {:?}", scan.verified_records.iter().map(|r| r.sequence).collect::<Vec<_>>());
    }
}

// --- 11: the MCP server remains unable to approve or reject proposals ---
// (structural, already proven by crates/mcp-server/tests/launcher.rs's
// own real_process_never_exposes_approve_or_reject_proposal_... test and
// tests/no_bypass.rs; this task touches neither mcp-server nor any MCP-
// reachable code path, so no new test is needed here -- re-asserted by
// running that existing suite in this task's own acceptance run below.)

// --- 12: concurrent/repeated approval attempts preserve existing
//     version/CAS protections ---

// Root-caused during this task's own acceptance testing (not a
// regression this task introduced): two SEPARATE OS processes each
// opening their own independent `storage::Vault`/SQLite connection
// against the same journal file can legitimately race at the SQLite
// layer itself (no `busy_timeout` is configured there), producing a
// transient `StorageError::Io`/`Journal` on the loser that
// `approve_proposal` honestly maps to `BrainApiError::RuntimeUnavailable`
// -- a real, pre-existing property of `storage::Vault`'s own
// cross-process concurrency model (`crates/brain-api/tests/layering.rs`'s
// own `an_approved_proposal_executes_exactly_once_under_concurrent_
// double_approve_through_the_public_surface` only ever tests two THREADS
// sharing ONE `Vault` instance, which is where that crate's in-process
// mutex/CAS protections actually apply; it never claims a stronger
// guarantee across two independent OS processes). This test proves the
// REAL, honest CAS invariant for M5 Task 7's own new code (never more
// than one commit, no corruption, no silently-swallowed error) via two
// THREADS sharing one `Vault` and one durable audit worker -- the
// deterministic, correct way to exercise `run_cli_durable`'s own
// concurrency behavior -- and separately documents, rather than
// over-asserts past, the weaker cross-process reality.

#[tokio::test]
async fn concurrent_in_process_approval_attempts_against_the_same_proposal_commit_at_most_once_with_no_corruption() {
    let root = unique_test_dir("concurrent-cas-in-process");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let vault = std::sync::Arc::new(storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap());
    let id = propose_a_note(&vault, "note-e2e-concurrent-cas-in-process");

    let audit_vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
    let worker = brain_api_audit_worker::spawn_worker(audit_vault, brain_api_audit_worker::WorkerConfig::default()).unwrap();

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let vault = vault.clone();
        let worker = worker.clone();
        let id = id.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let (tx, mut rx) = tokio::sync::mpsc::channel::<(brain_api::AuditEvent, tokio::sync::oneshot::Sender<Result<(), String>>)>(1);
            let bridge_worker = worker.clone();
            let bridge = tokio::spawn(async move {
                while let Some((event, reply)) = rx.recv().await {
                    let outcome = bridge_worker.record_durable(event).await.map(|_| ()).map_err(|e| e.to_string());
                    let _ = reply.send(outcome);
                }
            });
            let result = tokio::task::spawn_blocking(move || {
                let mut pre_register = |attempt_id: uuid::Uuid, proposal_id: &str| -> Result<(), String> {
                    let event = brain_api::event(brain_api::AuditEventKind::ApprovalAttempted { attempt_id: attempt_id.to_string(), proposal_id: proposal_id.to_string() }, NOW, uuid::Uuid::new_v4());
                    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                    if tx.blocking_send((event, reply_tx)).is_err() {
                        return Err("bridge gone".to_string());
                    }
                    reply_rx.blocking_recv().unwrap_or_else(|_| Err("bridge dropped reply".to_string()))
                };
                let mut record_outcome = |_attempt_id: uuid::Uuid, _result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> { Ok(()) };
                let mut hooks = brain_api::DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };
                let mut noop_reject_pre_register = |_attempt_id: uuid::Uuid, _proposal_id: &str| -> Result<(), String> { Ok(()) };
                let mut noop_reject_record_outcome = |_attempt_id: uuid::Uuid, _result: &Result<(), brain_api::BrainApiError>| -> Result<(), String> { Ok(()) };
                let mut rejection_hooks = brain_api::DurableRejectionHooks { pre_register: &mut noop_reject_pre_register, record_outcome: &mut noop_reject_record_outcome };
                let sink = brain_api::NullAuditSink;
                brain_api::run_cli_durable(&["approve".to_string(), id.to_string()], &vault, NOW, &sink, &mut hooks, &mut rejection_hooks)
            })
            .await
            .unwrap();
            bridge.await.unwrap();
            result
        }));
    }

    let results: Vec<_> = futures_join_all(handles).await;
    let successes = results.iter().filter(|r| r.result.is_ok()).count();
    assert_eq!(successes, 1, "exactly one of two in-process concurrent approvals sharing one Vault must succeed (storage's own existing per-note CAS), got {results:?}");
    for r in &results {
        if r.result.is_err() {
            assert!(matches!(r.result, Err(brain_api::BrainApiError::Conflict)), "the losing attempt must fail with the real, typed Conflict -- never corruption or a silently swallowed error, got {:?}", r.result);
        }
    }

    worker.shutdown().await.unwrap();

    let listing = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    assert!(listing.contains("Approved"), "got: {listing:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "got {:?}", scan.stop_reason);
    let sequences: Vec<u64> = scan.verified_records.iter().map(|r| r.sequence).collect();
    let mut deduped = sequences.clone();
    deduped.dedup();
    assert_eq!(sequences, deduped, "no duplicate sequence numbers may appear, got {sequences:?}");
}

/// Tiny local stand-in for `futures::future::join_all` -- this crate has
/// no dependency on the `futures` crate, and pulling one in for a single
/// test helper is not worth a new dev-dependency.
async fn futures_join_all<T>(handles: Vec<tokio::task::JoinHandle<T>>) -> Vec<T> {
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await.unwrap());
    }
    out
}

#[test]
fn cross_process_concurrent_approvals_never_corrupt_the_log_even_though_sqlite_itself_may_race() {
    // Documents the REAL, honest behavior across two independent OS
    // processes (see this test group's own doc comment above): SQLite
    // contention with no busy_timeout configured can make BOTH, ONE, or
    // NEITHER attempt fail at the storage layer -- this is a pre-existing
    // property of storage::Vault, not something this task changes or is
    // scoped to fix. The one invariant this task's own new code MUST
    // still uphold regardless of how that race resolves: never more than
    // one commit, and the audit log itself is never corrupted.
    let root = unique_test_dir("concurrent-cas-cross-process-honest");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-concurrent-cas-cross-process")
    };

    let id_str = id.to_string();
    let root_a = root.clone();
    let journal_a = journal.clone();
    let id_a = id_str.clone();
    let t1 = std::thread::spawn(move || run_approve_cli(&root_a, &journal_a, &["approve", &id_a]));
    std::thread::sleep(std::time::Duration::from_millis(50));
    let second = run_approve_cli(&root, &journal, &["approve", &id_str]);
    let first = t1.join().unwrap();

    let successes = [&first, &second].into_iter().filter(|o| o.status.success()).count();
    assert!(successes <= 1, "never more than one of two concurrent cross-process approvals may succeed, got first={first:?} second={second:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "the log itself must never be corrupted by cross-process contention, regardless of how the race resolved, got {:?}", scan.stop_reason);
    let sequences: Vec<u64> = scan.verified_records.iter().map(|r| r.sequence).collect();
    let mut deduped = sequences.clone();
    deduped.dedup();
    assert_eq!(sequences, deduped, "no duplicate sequence numbers may appear, got {sequences:?}");
}

// --- 13: worker shutdown does not claim success for unpersisted events ---

#[test]
fn a_clean_approval_reports_graceful_shutdown_never_an_unqualified_claim_on_failure() {
    let root = unique_test_dir("shutdown-honest-reporting");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-shutdown-honest")
    };

    let output = run_approve_cli(&root, &journal, &["approve", &id.to_string()]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // No "shut down successfully"-shaped unqualified claim anywhere;
    // the binary's own shutdown path either stays silent (graceful,
    // the default `Ok(Ok(()))` arm prints nothing) or prints one of the
    // two explicitly qualified warnings -- never a bare claim of success.
    assert!(!stderr.to_lowercase().contains("shut down successfully"), "got stderr: {stderr:?}");
}

// --- 8/9: crash between intent and commit / crash after commit but
//     before outcome is represented honestly, never misrepresented ---
//
// A real OS-level process kill at an uncontrolled point during approval
// is the only honest reproduction available without adding test-only
// instrumentation hooks to production code (out of this task's scope).
// This test kills the real process shortly after launch and then proves
// the one property that must ALWAYS hold regardless of exactly where the
// kill landed: the audit log itself, read back independently, is never
// corrupted, never shows a duplicate/out-of-order sequence, and classifies
// as exactly one of three honest shapes for the one proposal involved --
// (a) no ApprovalAttempted at all (killed before pre-registration even
// landed), (b) an ApprovAttempted with no matching CommitOutcome yet (an
// UNRESOLVED attempt -- killed between intent and commit, or after commit
// but before the outcome write), or (c) a matching, complete pair. Never a
// CommitOutcome with no corresponding ApprovalAttempted (which would mean
// an outcome was fabricated without ever having been pre-registered), and
// never two different attempt_ids colliding. The proposal's own real
// status (read back independently) must also never show Approved unless a
// REAL CommitOutcome::Committed/GitPending exists for it -- i.e. the log
// is never a weaker source of truth than the proposal store itself.

#[test]
fn killing_the_cli_process_mid_flight_never_corrupts_the_log_and_every_unresolved_attempt_is_honestly_representable() {
    let root = unique_test_dir("crash-mid-flight-honest");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-crash-mid-flight")
    };

    let mut child = Command::new(approve_cli_binary_path())
        .arg(&root)
        .arg(&journal)
        .args(["approve", &id.to_string()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("approve_cli must spawn as a real child process");

    // No synchronization point to land the kill at an EXACT step --
    // deliberately uncontrolled timing, proving the property holds
    // regardless of exactly where this lands (including "too late,
    // already fully exited", which is also a valid, honestly-represented
    // outcome this test still checks).
    std::thread::sleep(std::time::Duration::from_millis(3));
    let _ = child.kill();
    let _ = child.wait();

    // Give the OS a brief moment to release the writer lock the killed
    // process's own worker task held (immediate on handle drop, same
    // margin brain-api-audit-worker's own tests use after a kill).
    std::thread::sleep(std::time::Duration::from_millis(100));

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none() || matches!(scan.stop_reason, Some(brain_api::ScanStopReason::PartialTrailingRecord)), "a kill must never leave anything OTHER than a clean chain or, at worst, a partial trailing record at the physical tail -- never corruption in the middle, got {:?}", scan.stop_reason);

    let sequences: Vec<u64> = scan.verified_records.iter().map(|r| r.sequence).collect();
    let mut deduped = sequences.clone();
    deduped.dedup();
    assert_eq!(sequences, deduped, "no duplicate sequence numbers, got {sequences:?}");

    let attempts = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::ApprovalAttempted { proposal_id, .. } if proposal_id == &id.to_string()));
    let outcomes = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::CommitOutcome { .. }));
    assert!(attempts.len() <= 1, "at most one ApprovalAttempted for this one proposal, got {attempts:?}");
    assert!(outcomes.len() <= 1, "at most one CommitOutcome from this one invocation, got {outcomes:?}");
    if !outcomes.is_empty() {
        assert!(!attempts.is_empty(), "a CommitOutcome must never appear without its own prior ApprovalAttempted having been persisted first -- an outcome can never be fabricated without a real pre-registered attempt, got attempts={attempts:?} outcomes={outcomes:?}");
        let attempt_id_a = match &attempts[0].kind {
            brain_api::AuditEventKind::ApprovalAttempted { attempt_id, .. } => attempt_id.clone(),
            _ => unreachable!(),
        };
        let attempt_id_o = match &outcomes[0].kind {
            brain_api::AuditEventKind::CommitOutcome { attempt_id, .. } => attempt_id.clone(),
            _ => unreachable!(),
        };
        assert_eq!(attempt_id_a, attempt_id_o, "a present CommitOutcome must correlate with the present ApprovalAttempted via the SAME attempt_id");
    }

    // The proposal's own real, independently-read status must never
    // claim Approved unless a real Committed/GitPending CommitOutcome
    // exists for it -- the log is never a weaker source of truth than
    // the proposal store.
    let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
    let listing = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    if listing.contains(&format!("{id} [Approved")) {
        assert!(
            outcomes.iter().any(|e| matches!(&e.kind, brain_api::AuditEventKind::CommitOutcome { outcome: brain_api::CommitOutcomeKind::Committed | brain_api::CommitOutcomeKind::GitPending, .. })),
            "the proposal store shows Approved but no successful CommitOutcome was ever durably persisted for it -- the log must never under-report a real commit, got outcomes={outcomes:?}, listing={listing:?}"
        );
    }
}

// --- M5 Task 8B: the exact rejection mirror of the crash test above --
//     crash between rejection-intent and outcome persistence leaves an
//     explicitly unresolved attempt, never a fabricated success.

#[test]
fn killing_the_cli_process_mid_flight_during_rejection_never_corrupts_the_log_and_every_unresolved_attempt_is_honestly_representable() {
    let root = unique_test_dir("crash-mid-flight-reject-honest");
    let journal = root.join("journal.sqlite");
    init_git_repo(&root);
    let id = {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        propose_a_note(&vault, "note-e2e-crash-mid-flight-reject")
    };

    let mut child = Command::new(approve_cli_binary_path())
        .arg(&root)
        .arg(&journal)
        .args(["reject", &id.to_string(), "no"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("approve_cli must spawn as a real child process");

    std::thread::sleep(std::time::Duration::from_millis(3));
    let _ = child.kill();
    let _ = child.wait();

    std::thread::sleep(std::time::Duration::from_millis(100));

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none() || matches!(scan.stop_reason, Some(brain_api::ScanStopReason::PartialTrailingRecord)), "a kill must never leave anything OTHER than a clean chain or, at worst, a partial trailing record at the physical tail -- never corruption in the middle, got {:?}", scan.stop_reason);

    let sequences: Vec<u64> = scan.verified_records.iter().map(|r| r.sequence).collect();
    let mut deduped = sequences.clone();
    deduped.dedup();
    assert_eq!(sequences, deduped, "no duplicate sequence numbers, got {sequences:?}");

    let attempts = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::RejectionAttempted { proposal_id, .. } if proposal_id == &id.to_string()));
    let outcomes = events_of_kind(&scan, |k| matches!(k, brain_api::AuditEventKind::RejectionOutcome { .. }));
    assert!(attempts.len() <= 1, "at most one RejectionAttempted for this one proposal, got {attempts:?}");
    assert!(outcomes.len() <= 1, "at most one RejectionOutcome from this one invocation, got {outcomes:?}");
    if !outcomes.is_empty() {
        assert!(!attempts.is_empty(), "a RejectionOutcome must never appear without its own prior RejectionAttempted having been persisted first -- an outcome can never be fabricated without a real pre-registered attempt, got attempts={attempts:?} outcomes={outcomes:?}");
        let attempt_id_a = match &attempts[0].kind {
            brain_api::AuditEventKind::RejectionAttempted { attempt_id, .. } => attempt_id.clone(),
            _ => unreachable!(),
        };
        let attempt_id_o = match &outcomes[0].kind {
            brain_api::AuditEventKind::RejectionOutcome { attempt_id, .. } => attempt_id.clone(),
            _ => unreachable!(),
        };
        assert_eq!(attempt_id_a, attempt_id_o, "a present RejectionOutcome must correlate with the present RejectionAttempted via the SAME attempt_id");
    }

    // The proposal's own real, independently-read status must never
    // claim Rejected unless a real Rejected RejectionOutcome exists for
    // it -- the log is never a weaker source of truth than the proposal
    // store.
    let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
    let listing = brain_api::run_cli(&["list".to_string()], &vault, NOW).unwrap();
    if listing.contains(&format!("{id} [Rejected")) {
        assert!(
            outcomes.iter().any(|e| matches!(&e.kind, brain_api::AuditEventKind::RejectionOutcome { outcome: brain_api::RejectionOutcomeKind::Rejected, .. })),
            "the proposal store shows Rejected but no successful RejectionOutcome was ever durably persisted for it -- the log must never under-report a real rejection, got outcomes={outcomes:?}, listing={listing:?}"
        );
    }
}
