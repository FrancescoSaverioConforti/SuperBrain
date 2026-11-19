//! B5 remediation M1: the composition-root launcher that was missing at
//! closure (see `docs/research/B5-BRAIN-API-MCP-CLOSURE.md` Section 11,
//! item 2, and the follow-up Data Governance/Security/Trust audit's risk
//! R1). Every piece this binary wires together already exists and is
//! already tested in isolation (`SuperBrainMcpServer::new`,
//! `mcp_server::limited_transport`, `index::rebuild_index`,
//! `storage::Vault::open_with_default_indexer`) — this file's only job is
//! to compose them against real stdio, the same "thin wrapper, all real
//! logic lives in library code" discipline `brain-api/examples/
//! approve_cli.rs` already established for the CLI approval surface.
//!
//! `cargo run -p mcp-server --example serve_stdio -- <vault-root>
//! <journal-path> <active-project>`
//!
//! What this binary deliberately does NOT do: it does not construct a
//! `WritePolicy::trusted_cli()` or any other elevated context (every MCP
//! request already builds `WritePolicy::mcp_agent()` internally, inside
//! `SuperBrainMcpServer`'s own tool handlers — this binary has no code
//! path that could override that); it does not open a network socket
//! (stdio only, matching `rmcp`'s `transport-io` feature this crate
//! already depends on); it does not wire a dense scorer (that remains
//! the same safe, documented default `SuperBrainMcpServer::new` already
//! gives every caller — opting in is a separate, later decision, not
//! this remediation's job).
//!
//! **M5 Task 6: real durable audit wiring.** This binary IS the one
//! production composition root authorized to construct
//! `brain_api_audit_worker::spawn_worker` and inject the resulting
//! `AuditWorkerHandle` via `SuperBrainMcpServer::with_audit_sink(...)` —
//! `mcp-server`'s own library code stays completely unaware the worker
//! crate exists (a `dev-dependency` of `mcp-server`, used only by this
//! `[[example]]` target; see that crate's own `Cargo.toml` comment).
//! Fails closed, before ever touching stdio, if the audit store cannot
//! be opened/verified (`spawn_worker`'s own `WorkerStartupError`) — there
//! is no fallback to `brain_api::NullAuditSink` anywhere in this file;
//! the only way this binary ever serves a request is with the real
//! worker already running. A SECOND, independent `storage::Vault` handle
//! is opened against the SAME root/journal purely to hand to
//! `spawn_worker` (never shared with, or substituted for, the server's
//! own vault instance) — the same two-handles-one-root pattern
//! `brain-api-audit-worker`'s own test suite already uses; `Vault`'s own
//! cross-process exclusion lives in the audit writer's OS-level lock,
//! not in how many in-process `Vault` values exist.
//!
//! On a clean MCP service exit, this binary calls
//! `AuditWorkerHandle::shutdown()` — a BOUNDED wait
//! (`brain_api::limits::DEFAULT_TIMEOUT_MS`, this crate's own existing
//! canonical bounded-operation default; see that constant's own doc
//! comment), never an unbounded block — so every durable command already
//! enqueued drains before the process exits, and logs explicitly whether
//! that drain completed, timed out, or found the worker already gone,
//! rather than reporting an unqualified "shut down successfully".

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: serve_stdio <vault-root> <journal-path> <active-project>");
        std::process::exit(2);
    }
    let vault_root = std::path::PathBuf::from(&args[1]);
    let journal_path = std::path::PathBuf::from(&args[2]);
    let active_project = args[3].clone();

    let vault = match storage::Vault::open_with_default_indexer(vault_root.clone(), &journal_path) {
        Ok(vault) => vault,
        Err(err) => {
            eprintln!("failed to open vault: {err:?}");
            std::process::exit(1);
        }
    };

    // A second, independent `Vault` handle against the SAME root/journal,
    // for the audit worker alone -- see this file's own top-level doc
    // comment for why two handles (never shared, never substituted) is
    // the correct shape here.
    let audit_vault = match storage::Vault::open_with_default_indexer(vault_root, &journal_path) {
        Ok(vault) => vault,
        Err(err) => {
            eprintln!("failed to open vault for the audit worker: {err:?}");
            std::process::exit(1);
        }
    };

    // Cold-start indexing: the same real, already-tested function the
    // library itself provides for populating a fresh `RealIndexer` from
    // whatever notes already exist in the vault (`index::rebuild_index`,
    // `crates/index/src/indexer_impl.rs:467`) — never a placeholder/empty
    // index silently served as if it were current.
    let indexer = std::sync::Arc::new(index::indexer_impl::RealIndexer::new());
    if let Err(err) = index::indexer_impl::rebuild_index(&vault, &indexer) {
        eprintln!("failed to build the note index: {err:?}");
        std::process::exit(1);
    }

    // The code graph has no persisted-build/load mechanism in this
    // codebase today (confirmed: `CodeGraph::new()` is `Self::default()`,
    // the same empty-graph construction `tests/server_integration.rs`'s
    // own fixture uses) — an empty graph is a real, correctly-behaving
    // state per the B5.0 frozen contract (`get_entity`/`get_code_context`/
    // `graph_query` return `NotFound`/empty results, never a fabricated
    // fact), not a stand-in for a missing feature this binary should
    // paper over.
    let graph = std::sync::Arc::new(codegraph_index::graph::CodeGraph::new());

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start the async runtime: {err}");
            std::process::exit(1);
        }
    };

    let exit_code = rt.block_on(async move {
        // Fail closed, BEFORE ever touching stdio or the MCP handshake:
        // `spawn_worker` runs its own startup recovery scan and opens the
        // real writer lock synchronously here. There is no fallback to
        // `brain_api::NullAuditSink` on any `Err` branch in this file.
        let audit_worker = match brain_api_audit_worker::spawn_worker(audit_vault, brain_api_audit_worker::WorkerConfig::default()) {
            Ok(handle) => handle,
            Err(err) => {
                eprintln!("failed to start the audit worker: {err}");
                return 1;
            }
        };

        let server = mcp_server::SuperBrainMcpServer::new(vault, indexer, graph, active_project)
            .with_audit_sink(std::sync::Arc::new(audit_worker.clone()));

        let (stdin, stdout) = rmcp::transport::io::stdio();
        let transport = mcp_server::limited_transport(stdin, stdout);

        let service = match rmcp::ServiceExt::serve(server, transport).await {
            Ok(service) => service,
            Err(err) => {
                eprintln!("failed to start the MCP server (handshake/transport error): {err}");
                return shut_down_audit_worker_and_exit(&audit_worker, 1).await;
            }
        };

        let exit_code = match service.waiting().await {
            Ok(_) => 0,
            Err(err) => {
                eprintln!("MCP server exited with an error: {err}");
                1
            }
        };

        shut_down_audit_worker_and_exit(&audit_worker, exit_code).await
    });

    std::process::exit(exit_code);
}

/// Drains and stops the audit worker with a BOUNDED wait
/// (`brain_api::limits::DEFAULT_TIMEOUT_MS`) before this process exits,
/// then returns `exit_code` unchanged — this function never upgrades or
/// downgrades the MCP service's own exit code based on how shutdown
/// went; it only logs, honestly, which of three distinct outcomes
/// actually happened, never reporting an unqualified "shut down
/// successfully":
/// - graceful drain completed (every already-enqueued durable command
///   was processed before the worker task exited);
/// - the bounded wait elapsed first (`UNKNOWN`: the worker may still be
///   draining in the background when this process exits — never
///   reported as either success or failure);
/// - the worker was already gone (nothing to drain).
async fn shut_down_audit_worker_and_exit(audit_worker: &brain_api_audit_worker::AuditWorkerHandle, exit_code: i32) -> i32 {
    let bound = std::time::Duration::from_millis(brain_api::limits::DEFAULT_TIMEOUT_MS);
    match tokio::time::timeout(bound, audit_worker.shutdown()).await {
        Ok(Ok(())) => eprintln!("audit worker drained and shut down gracefully"),
        Ok(Err(err)) => eprintln!("audit worker was already gone at shutdown time (nothing to drain): {err}"),
        Err(_) => eprintln!("audit worker shutdown did not complete within {bound:?} -- its drain status on exit is UNKNOWN, not confirmed graceful"),
    }
    exit_code
}
