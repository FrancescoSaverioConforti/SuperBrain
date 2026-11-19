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
//! already depends on); it does not wire a dense scorer or a non-null
//! audit sink (both remain the same safe, documented defaults
//! `SuperBrainMcpServer::new` already gives every caller — opting into
//! either is a separate, later decision, not this remediation's job).

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: serve_stdio <vault-root> <journal-path> <active-project>");
        std::process::exit(2);
    }
    let vault_root = std::path::PathBuf::from(&args[1]);
    let journal_path = std::path::PathBuf::from(&args[2]);
    let active_project = args[3].clone();

    let vault = match storage::Vault::open_with_default_indexer(vault_root, &journal_path) {
        Ok(vault) => vault,
        Err(err) => {
            eprintln!("failed to open vault: {err:?}");
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
        let server = mcp_server::SuperBrainMcpServer::new(vault, indexer, graph, active_project);

        let (stdin, stdout) = rmcp::transport::io::stdio();
        let transport = mcp_server::limited_transport(stdin, stdout);

        let service = match rmcp::ServiceExt::serve(server, transport).await {
            Ok(service) => service,
            Err(err) => {
                eprintln!("failed to start the MCP server (handshake/transport error): {err}");
                return 1;
            }
        };

        match service.waiting().await {
            Ok(_) => 0,
            Err(err) => {
                eprintln!("MCP server exited with an error: {err}");
                1
            }
        }
    });

    std::process::exit(exit_code);
}
