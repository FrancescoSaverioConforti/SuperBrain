//! Minimal CLI approval surface (B5.1 Task 15): `cargo run --example
//! approve_cli -- <vault-root> <journal-path> list|approve <id>|reject <id> <reason>`.
//!
//! A thin wrapper over `brain_api::run_cli_durable` — all the real trust
//! and dispatch logic (command parsing, trusted-context construction,
//! authorization, `approve_proposal`/`reject_proposal`/`list_proposals`)
//! lives in `brain-api` itself, tested in `tests/cli_approval_e2e.rs`
//! without spawning this binary.
//!
//! **M5 Task 7: real durable approval/commit audit wiring.** This binary
//! is the one production composition root authorized to construct
//! `brain_api_audit_worker::spawn_worker` for the CLI approval surface,
//! exactly mirroring `mcp-server/examples/serve_stdio.rs`'s own M5 Task 6
//! role for the MCP surface. `brain-api-audit-worker` is a `dev-
//! dependency` of this crate, used only by this `[[example]]` target —
//! `brain-api`'s own library code (`run_cli`/`run_cli_with_audit`/
//! `run_cli_durable`) stays completely unaware the worker crate exists,
//! and gains no `tokio` dependency of its own; the async bridging below
//! is entirely this binary's own concern.
//!
//! **Fail-closed, before any command runs**: if the audit store cannot be
//! opened/verified, this binary exits non-zero before ever calling
//! `run_cli_durable` — there is no fallback to a best-effort-only or
//! `NullAuditSink`-equivalent path for `approve`/`reject` in this file.
//!
//! **Sync/async bridge**: `run_cli_durable`'s own `DurableApprovalHooks`/
//! `DurableRejectionHooks` (M5 Task 8B's rejection-specific mirror) are
//! plain synchronous `FnMut` closures (`brain-api` has no `tokio`
//! dependency and never will). This binary runs the ENTIRE synchronous
//! `run_cli_durable` call on `tokio::task::spawn_blocking` (the same
//! proven-safe sync/async boundary `brain-api-audit-worker::worker.rs`
//! itself uses for its own `AuditWriterHandle::append` calls) and gives
//! it hooks whose bodies send one request over a bounded
//! `tokio::sync::mpsc` channel to a small, separately spawned async
//! bridge task — which alone ever calls the real, async
//! `AuditWorkerHandle::record_durable(...).await` — and then block
//! (via `std::sync::mpsc`, a PLAIN OS-thread blocking channel, never
//! `Handle::block_on`) on that task's own reply. This never nests a
//! runtime inside a runtime: the blocking happens on a `spawn_blocking`
//! thread, exactly the kind of thread `spawn_blocking` exists to
//! tolerate blocking on. If the bridge task is ever gone (channel
//! closed) a hook returns `Err` immediately — never hangs, never
//! silently treats "no reply" as "it happened".

use brain_api::{DurableApprovalHooks, DurableRejectionHooks};

/// One request a blocking-side hook sends to the async bridge task —
/// `event` is the already-built `AuditEvent` (ApprovalAttempted or
/// CommitOutcome; this binary's own call sites decide which), `reply` is
/// a plain OS-thread channel the hook blocks on for the real outcome.
struct BridgeRequest {
    event: brain_api::AuditEvent,
    reply: std::sync::mpsc::Sender<Result<(), String>>,
}

/// Sends `event` to the bridge task and blocks (plain OS thread, never
/// `Handle::block_on`) for its reply — the one shape both hooks below
/// share. `req_tx.blocking_send` is the correct call here specifically
/// because this always runs INSIDE `tokio::task::spawn_blocking` (never
/// directly on a runtime worker thread), which is the one context
/// `blocking_send` itself requires.
fn send_and_await_durable_ack(req_tx: &tokio::sync::mpsc::Sender<BridgeRequest>, event: brain_api::AuditEvent) -> Result<(), String> {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    if req_tx.blocking_send(BridgeRequest { event, reply: reply_tx }).is_err() {
        return Err("audit bridge task is gone".to_string());
    }
    reply_rx.recv().unwrap_or_else(|_| Err("audit bridge task dropped the reply channel without responding".to_string()))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: approve_cli <vault-root> <journal-path> list|approve <id>|reject <id> <reason>");
        std::process::exit(2);
    }
    let vault_root = std::path::PathBuf::from(&args[1]);
    let journal_path = std::path::PathBuf::from(&args[2]);
    let command_args: Vec<String> = args[3..].to_vec();

    let vault = match storage::Vault::open_with_default_indexer(vault_root.clone(), &journal_path) {
        Ok(vault) => vault,
        Err(err) => {
            eprintln!("failed to open vault: {err:?}");
            std::process::exit(1);
        }
    };
    // A second, independent `Vault` handle against the SAME root/journal,
    // for the audit worker alone -- mirrors serve_stdio.rs's own M5 Task
    // 6 pattern exactly; never shared with, or substituted for, the
    // handle `run_cli_durable` itself operates on above.
    let audit_vault = match storage::Vault::open_with_default_indexer(vault_root, &journal_path) {
        Ok(vault) => vault,
        Err(err) => {
            eprintln!("failed to open vault for the audit worker: {err:?}");
            std::process::exit(1);
        }
    };

    let now = chrono::Utc::now().to_rfc3339();

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_time().build() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start the async runtime: {err}");
            std::process::exit(1);
        }
    };

    let exit_code = rt.block_on(async move {
        // Fail closed, BEFORE ever running any command: spawn_worker runs
        // its own startup recovery scan and opens the real writer lock
        // synchronously here.
        let audit_worker = match brain_api_audit_worker::spawn_worker(audit_vault, brain_api_audit_worker::WorkerConfig::default()) {
            Ok(handle) => handle,
            Err(err) => {
                eprintln!("failed to start the audit worker: {err}");
                return 1;
            }
        };

        let (req_tx, mut req_rx) = tokio::sync::mpsc::channel::<BridgeRequest>(1);
        let worker_for_bridge = audit_worker.clone();
        let bridge = tokio::spawn(async move {
            while let Some(req) = req_rx.recv().await {
                let outcome = worker_for_bridge.record_durable(req.event).await.map(|_ack| ()).map_err(|err| err.to_string());
                let _ = req.reply.send(outcome);
            }
        });

        let req_tx_for_pre_register = req_tx.clone();
        let req_tx_for_outcome = req_tx.clone();
        let req_tx_for_rejection_pre_register = req_tx.clone();
        let req_tx_for_rejection_outcome = req_tx.clone();
        let now_for_pre_register = now.clone();
        let now_for_outcome = now.clone();
        let now_for_rejection_pre_register = now.clone();
        let now_for_rejection_outcome = now.clone();
        let command_args_for_blocking = command_args.clone();
        let cli_outcome = tokio::task::spawn_blocking(move || {
            let mut pre_register = move |attempt_id: uuid::Uuid, proposal_id: &str| -> Result<(), String> {
                let event = brain_api::event(
                    brain_api::AuditEventKind::ApprovalAttempted { attempt_id: attempt_id.to_string(), proposal_id: proposal_id.to_string() },
                    &now_for_pre_register,
                    uuid::Uuid::new_v4(),
                );
                send_and_await_durable_ack(&req_tx_for_pre_register, event)
            };
            let mut record_outcome = move |attempt_id: uuid::Uuid, result: &Result<storage::CommitResult, brain_api::BrainApiError>| -> Result<(), String> {
                let outcome = commit_outcome_kind_from(result);
                let event = brain_api::event(
                    brain_api::AuditEventKind::CommitOutcome { attempt_id: attempt_id.to_string(), outcome },
                    &now_for_outcome,
                    uuid::Uuid::new_v4(),
                );
                send_and_await_durable_ack(&req_tx_for_outcome, event)
            };
            let mut rejection_pre_register = move |attempt_id: uuid::Uuid, proposal_id: &str| -> Result<(), String> {
                let event = brain_api::event(
                    brain_api::AuditEventKind::RejectionAttempted { attempt_id: attempt_id.to_string(), proposal_id: proposal_id.to_string() },
                    &now_for_rejection_pre_register,
                    uuid::Uuid::new_v4(),
                );
                send_and_await_durable_ack(&req_tx_for_rejection_pre_register, event)
            };
            let mut rejection_record_outcome = move |attempt_id: uuid::Uuid, result: &Result<(), brain_api::BrainApiError>| -> Result<(), String> {
                let outcome = rejection_outcome_kind_from(result);
                let event = brain_api::event(
                    brain_api::AuditEventKind::RejectionOutcome { attempt_id: attempt_id.to_string(), outcome },
                    &now_for_rejection_outcome,
                    uuid::Uuid::new_v4(),
                );
                send_and_await_durable_ack(&req_tx_for_rejection_outcome, event)
            };
            let sink = brain_api::NullAuditSink;
            let mut approval_hooks = DurableApprovalHooks { pre_register: &mut pre_register, record_outcome: &mut record_outcome };
            let mut rejection_hooks = DurableRejectionHooks { pre_register: &mut rejection_pre_register, record_outcome: &mut rejection_record_outcome };
            brain_api::run_cli_durable(&command_args_for_blocking, &vault, &now, &sink, &mut approval_hooks, &mut rejection_hooks)
        })
        .await
        .expect("the blocking CLI closure itself never panics");

        drop(req_tx); // closes the channel once the blocking call is done, letting `bridge`'s own recv() loop end
        bridge.await.expect("the bridge task itself never panics");

        let exit_code = match cli_outcome.result {
            Ok(output) => {
                println!("{output}");
                0
            }
            Err(err) => {
                eprintln!("error: {err}");
                1
            }
        };
        if let Err(err) = cli_outcome.audit_outcome_write {
            eprintln!("warning: the durable audit outcome write itself failed (the real command result above is unaffected and truthful): {err}");
        }

        let shutdown_bound = std::time::Duration::from_millis(brain_api::limits::DEFAULT_TIMEOUT_MS);
        match tokio::time::timeout(shutdown_bound, audit_worker.shutdown()).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => eprintln!("audit worker was already gone at shutdown time (nothing to drain): {err}"),
            Err(_) => eprintln!("audit worker shutdown did not complete within {shutdown_bound:?} -- its drain status on exit is UNKNOWN, not confirmed graceful"),
        }

        exit_code
    });

    std::process::exit(exit_code);
}

/// Maps a real `approve_proposal` result onto the closed, content-free
/// `CommitOutcomeKind` schema (M5 Task 5) -- never the raw `BrainApiError`
/// `Display` text, which could carry caller-supplied detail
/// (`InvalidInput(String)`'s own inner string) this schema's content-free
/// discipline forbids. `PermissionDenied` has no arm here because
/// `run_cli_durable`'s own `record_outcome` hook is only ever called
/// AFTER a pre-registered attempt reaches `approve_proposal` -- a denied
/// attempt never gets this far (see `cli.rs`'s own `run_approve_core`).
fn commit_outcome_kind_from(result: &Result<storage::CommitResult, brain_api::BrainApiError>) -> brain_api::CommitOutcomeKind {
    use brain_api::{BrainApiError, CommitFailureCategory, CommitOutcomeKind};
    match result {
        Ok(storage::CommitResult::Committed(_)) => CommitOutcomeKind::Committed,
        Ok(storage::CommitResult::GitPending { .. }) => CommitOutcomeKind::GitPending,
        Err(BrainApiError::NotFound) => CommitOutcomeKind::Failed { category: CommitFailureCategory::NotFound },
        Err(BrainApiError::InvalidInput(_)) => CommitOutcomeKind::Failed { category: CommitFailureCategory::InvalidInput },
        Err(BrainApiError::Conflict) => CommitOutcomeKind::Failed { category: CommitFailureCategory::Conflict },
        Err(BrainApiError::RuntimeUnavailable) => CommitOutcomeKind::Failed { category: CommitFailureCategory::RuntimeUnavailable },
        Err(BrainApiError::CorruptData) => CommitOutcomeKind::Failed { category: CommitFailureCategory::CorruptData },
        // PermissionDenied/DenseUnavailable/ExternalToolUnavailable/
        // Timeout/UnsupportedOperation: none of these can actually be
        // returned by approve_proposal (confirmed by reading its own
        // body in crates/brain-api/src/approval.rs) -- mapped to the
        // closest existing category rather than panicking on an
        // impossible-today variant, since BrainApiError's own variant
        // set could grow in a future, separately authorized task.
        Err(_) => CommitOutcomeKind::Failed { category: CommitFailureCategory::RuntimeUnavailable },
    }
}

/// Maps a real `reject_proposal` result onto the closed, content-free
/// `RejectionOutcomeKind` schema (M5 Task 8B) -- the rejection-specific
/// mirror of `commit_outcome_kind_from` above. `PermissionDenied` has no
/// arm here for the same reason it has none above: `run_cli_durable`'s
/// own `record_outcome` hook is only ever called AFTER a pre-registered
/// attempt reaches `reject_proposal` -- a denied attempt never gets this
/// far (see `cli.rs`'s own `run_reject_core`).
fn rejection_outcome_kind_from(result: &Result<(), brain_api::BrainApiError>) -> brain_api::RejectionOutcomeKind {
    use brain_api::{BrainApiError, CommitFailureCategory, RejectionOutcomeKind};
    match result {
        Ok(()) => RejectionOutcomeKind::Rejected,
        Err(BrainApiError::NotFound) => RejectionOutcomeKind::Failed { category: CommitFailureCategory::NotFound },
        Err(BrainApiError::InvalidInput(_)) => RejectionOutcomeKind::Failed { category: CommitFailureCategory::InvalidInput },
        Err(BrainApiError::Conflict) => RejectionOutcomeKind::Failed { category: CommitFailureCategory::Conflict },
        Err(BrainApiError::RuntimeUnavailable) => RejectionOutcomeKind::Failed { category: CommitFailureCategory::RuntimeUnavailable },
        Err(BrainApiError::CorruptData) => RejectionOutcomeKind::Failed { category: CommitFailureCategory::CorruptData },
        // PermissionDenied/DenseUnavailable/ExternalToolUnavailable/
        // Timeout/UnsupportedOperation: same reasoning as
        // commit_outcome_kind_from's own identical catch-all above.
        Err(_) => RejectionOutcomeKind::Failed { category: CommitFailureCategory::RuntimeUnavailable },
    }
}
