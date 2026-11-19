//! B5.16 Task 35: end-to-end resource-limit tests for `mcp-server`.
//!
//! The concurrency semaphore's own mechanism (cap enforcement, shared
//! pool across clones, cancel-safety) is proven as a white-box test
//! inside the crate itself
//! (`src/tools/search.rs::concurrency_limit_tests`, reachable there only
//! because `acquire_concurrency_permit` is `pub(crate)`). This file
//! proves the REAL, externally-observable effects: an oversized MCP
//! payload is rejected at the transport layer, before any line reaches
//! this server's JSON-RPC/tool-dispatch logic at all (never mind
//! `brain-api`); and `graph_query`'s traversal-depth/result-count bounds
//! are enforced over a real wire connection, not just in `brain-api`'s
//! own unit tests. `search`'s `MAX_QUERY_LEN`/`MAX_RESULT_COUNT` bounds
//! are already proven at the `brain_api::read::search` unit-test level
//! (`crates/brain-api/src/read.rs`) and at the MCP-tool level the same
//! way `search_tools.rs` already proves `search`'s other `InvalidInput`
//! cases (calling the `#[tool]` method directly) — not duplicated here.

use std::sync::Arc;

use codegraph_domain::{CommitHash, EdgeKind, FileIdentity, RepositoryRegistry, RepositoryRoot};
use codegraph_index::edges::Edge as GraphEdge;
use codegraph_index::extract::{ByteSpan, RawSymbolCandidate};
use codegraph_index::graph::CodeGraph;
use codegraph_index::snapshot::promote;
use index::indexer_impl::RealIndexer;
use mcp_server::SuperBrainMcpServer;
use rmcp::{model::CallToolRequestParams, object, ServiceExt};
use storage::Vault;
use tokio::io::AsyncWriteExt;

fn unique_test_dir(label: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-mcp-server-limits-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn new_server() -> SuperBrainMcpServer {
    let root = unique_test_dir("fixture");
    let vault =
        Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();
    SuperBrainMcpServer::new(
        vault,
        Arc::new(RealIndexer::new()),
        Arc::new(CodeGraph::new()),
        "project-x",
    )
}

/// Same chain-graph fixture shape as `brain-api`'s own `read.rs` tests
/// (`a -> b -> c`), built here against this crate's real server so
/// `graph_query`'s depth/result bounds can be exercised over a real
/// wire connection.
fn server_with_chain_graph() -> (SuperBrainMcpServer, String) {
    let root = unique_test_dir("graph-fixture");
    let vault =
        Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();

    let mut registry = RepositoryRegistry::new();
    let repo = registry.register(RepositoryRoot::parse(&root).unwrap());
    let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
    let file = FileIdentity::parse(repo, commit, "src/lib.rs").unwrap();

    let a_candidate = RawSymbolCandidate::new(
        "a".to_string(),
        "function_item".to_string(),
        ByteSpan { start: 0, end: 10 },
        "fn a() { b(); }".to_string(),
    )
    .unwrap();
    let b_candidate = RawSymbolCandidate::new(
        "b".to_string(),
        "function_item".to_string(),
        ByteSpan { start: 20, end: 30 },
        "fn b() { c(); }".to_string(),
    )
    .unwrap();
    let c_candidate = RawSymbolCandidate::new(
        "c".to_string(),
        "function_item".to_string(),
        ByteSpan { start: 40, end: 50 },
        "fn c() {}".to_string(),
    )
    .unwrap();
    let a = promote(&a_candidate, file.clone()).unwrap();
    let b = promote(&b_candidate, file.clone()).unwrap();
    let c = promote(&c_candidate, file).unwrap();
    let (a_id, b_id, c_id) = (a.id(), b.id(), c.id());

    let mut graph = CodeGraph::new();
    graph.insert_snapshot(a).unwrap();
    graph.insert_snapshot(b).unwrap();
    graph.insert_snapshot(c).unwrap();
    graph
        .insert_edge(GraphEdge {
            from: a_id,
            to: b_id,
            kind: EdgeKind::Calls,
            evidence: codegraph_domain::Evidence::new(
                codegraph_domain::ResolutionMethod::StaticAnalysis,
                None,
                "test edge",
            )
            .unwrap(),
        })
        .unwrap();
    graph
        .insert_edge(GraphEdge {
            from: b_id,
            to: c_id,
            kind: EdgeKind::Calls,
            evidence: codegraph_domain::Evidence::new(
                codegraph_domain::ResolutionMethod::StaticAnalysis,
                None,
                "test edge",
            )
            .unwrap(),
        })
        .unwrap();

    let server = SuperBrainMcpServer::new(
        vault,
        Arc::new(RealIndexer::new()),
        Arc::new(graph),
        "project-x",
    );
    (server, a_id.to_string())
}

async fn connected_client(
    server: SuperBrainMcpServer,
) -> (
    rmcp::service::RunningService<rmcp::RoleClient, ()>,
    rmcp::service::RunningService<rmcp::RoleServer, SuperBrainMcpServer>,
) {
    let (server_transport, client_transport) = tokio::io::duplex(1 << 20);
    let (server_result, client_result) =
        tokio::join!(server.serve(server_transport), ().serve(client_transport));
    (
        client_result.expect("client should initialize"),
        server_result.expect("server should initialize"),
    )
}

// --- graph_query depth/result bounds, enforced over a real wire ---

#[tokio::test]
async fn graph_query_over_the_wire_with_max_depth_exceeding_the_bound_is_a_tool_level_error() {
    let (server, start_id) = server_with_chain_graph();
    let (client, server_handle) = connected_client(server).await;

    let params = CallToolRequestParams::new("graph_query").with_arguments(object!({
        "start_symbol_id": start_id,
        "max_depth": brain_api::limits::MAX_GRAPH_TRAVERSAL_DEPTH + 1,
        "limit": 10,
    }));
    // brain_api::graph_query's InvalidInput maps to a protocol-level
    // McpError (map_brain_api_error), not a tool-level is_error result --
    // the same shape retrieve's NotFound uses. Either way, the real
    // subject here is that the bound violation is rejected, never
    // silently allowed through to an unbounded real traversal.
    let result = client.call_tool(params).await;
    assert!(
        result.is_err(),
        "max_depth beyond the bound must be rejected, got {result:?}"
    );

    client.cancel().await.expect("client should cancel cleanly");
    server_handle
        .cancel()
        .await
        .expect("server should cancel cleanly");
}

#[tokio::test]
async fn graph_query_over_the_wire_with_limit_exceeding_the_bound_is_a_tool_level_error() {
    let (server, start_id) = server_with_chain_graph();
    let (client, server_handle) = connected_client(server).await;

    let params = CallToolRequestParams::new("graph_query").with_arguments(object!({
        "start_symbol_id": start_id,
        "max_depth": 1,
        "limit": brain_api::limits::MAX_GRAPH_QUERY_RESULTS + 1,
    }));
    let result = client.call_tool(params).await;
    assert!(
        result.is_err(),
        "limit beyond the bound must be rejected, got {result:?}"
    );

    client.cancel().await.expect("client should cancel cleanly");
    server_handle
        .cancel()
        .await
        .expect("server should cancel cleanly");
}

#[tokio::test]
async fn graph_query_over_the_wire_within_bounds_still_succeeds() {
    let (server, start_id) = server_with_chain_graph();
    let (client, server_handle) = connected_client(server).await;

    let params = CallToolRequestParams::new("graph_query").with_arguments(object!({
        "start_symbol_id": start_id,
        "max_depth": brain_api::limits::MAX_GRAPH_TRAVERSAL_DEPTH,
        "limit": brain_api::limits::MAX_GRAPH_QUERY_RESULTS,
    }));
    let result = client
        .call_tool(params)
        .await
        .expect("tools/call must succeed");
    assert_ne!(
        result.is_error,
        Some(true),
        "a call exactly at the bound must still succeed, got {result:?}"
    );
    let structured = result
        .structured_content
        .expect("graph_query must produce structured_content");
    let ids = structured["symbol_ids"]
        .as_array()
        .expect("symbol_ids must be an array");
    assert_eq!(
        ids.len(),
        3,
        "expected the full 3-node chain to be reached, got {structured:?}"
    );

    client.cancel().await.expect("client should cancel cleanly");
    server_handle
        .cancel()
        .await
        .expect("server should cancel cleanly");
}

// --- oversized MCP payload: rejected at the transport layer, before
//     reaching this server's own JSON-RPC/tool-dispatch logic ---

#[tokio::test]
async fn an_oversized_jsonrpc_line_is_dropped_at_the_transport_layer_and_the_connection_survives() {
    let (server_io, peer_io) = tokio::io::duplex(8 * 1024 * 1024);
    let (reader, writer) = tokio::io::split(server_io);
    let transport = mcp_server::limited_transport(reader, writer);
    let server_handle = tokio::spawn(new_server().serve(transport));

    let (peer_read, mut peer_write) = tokio::io::split(peer_io);
    let mut reader = tokio::io::BufReader::new(peer_read);

    async fn next_line(
        reader: &mut tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    ) -> String {
        use tokio::io::AsyncBufReadExt;
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            reader.read_line(&mut line),
        )
        .await
        .expect("server must reply promptly, not hang")
        .expect("read should not error");
        line
    }

    peer_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test-client\",\"version\":\"0.0.1\"}}}\n").await.unwrap();
    let init_reply: serde_json::Value =
        serde_json::from_str(next_line(&mut reader).await.trim()).unwrap();
    assert!(
        init_reply.get("result").is_some(),
        "initialize must succeed before this test's real subject, got {init_reply}"
    );
    peer_write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .unwrap();

    // A single JSON-RPC line, well-formed, carrying a `body` field far
    // larger than MCP_MAX_PAYLOAD_BYTES -- this crosses the transport's
    // own configured max_line_length BEFORE the JSON is ever parsed into
    // a request this server's tool-dispatch logic would see.
    let oversized_body = "y".repeat(brain_api::limits::MCP_MAX_PAYLOAD_BYTES + 1024);
    let mut oversized_line = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"save_memory\",\"arguments\":{{\"note_id\":\"note-oversized\",\"note_type\":\"decision\",\"classification\":\"internal\",\"project\":\"project-x\",\"source\":\"adversarial\",\"event_period\":\"2026-11-18\",\"body\":\"{oversized_body}\"}}}}}}"
    );
    oversized_line.push('\n');
    assert!(
        oversized_line.len() > brain_api::limits::MCP_MAX_PAYLOAD_BYTES,
        "the test line itself must exceed the configured limit to be a real test of it"
    );
    peer_write
        .write_all(oversized_line.as_bytes())
        .await
        .unwrap();

    // The oversized line is dropped (never buffered unboundedly, never
    // causing a panic) -- rmcp's own AsyncRwTransport documented
    // behavior. The connection stays alive: a real, well-formed request
    // sent right after still gets a real reply.
    peer_write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let list_reply: serde_json::Value = serde_json::from_str(next_line(&mut reader).await.trim()).expect("the server must send a valid JSON reply to the request after the oversized line, proving it survived rather than hanging or crashing");
    assert!(
        list_reply.get("result").is_some(),
        "tools/list after the oversized line must still succeed, got {list_reply}"
    );

    drop(peer_write);
    drop(reader);
    server_handle
        .await
        .unwrap()
        .expect("server task must not have panicked or errored")
        .cancel()
        .await
        .expect("server should still be cancellable after surviving the oversized line");
}

#[tokio::test]
async fn a_well_formed_line_within_the_payload_limit_is_not_affected_by_the_limited_transport() {
    let (server_io, peer_io) = tokio::io::duplex(1 << 20);
    let (reader, writer) = tokio::io::split(server_io);
    let transport = mcp_server::limited_transport(reader, writer);
    let server_handle = tokio::spawn(new_server().serve(transport));

    let (peer_read, mut peer_write) = tokio::io::split(peer_io);
    let mut reader = tokio::io::BufReader::new(peer_read);

    async fn next_line(
        reader: &mut tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    ) -> String {
        use tokio::io::AsyncBufReadExt;
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            reader.read_line(&mut line),
        )
        .await
        .expect("server must reply promptly")
        .expect("read should not error");
        line
    }

    peer_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test-client\",\"version\":\"0.0.1\"}}}\n").await.unwrap();
    let init_reply: serde_json::Value =
        serde_json::from_str(next_line(&mut reader).await.trim()).unwrap();
    assert!(init_reply.get("result").is_some());
    peer_write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .unwrap();

    peer_write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let list_reply: serde_json::Value =
        serde_json::from_str(next_line(&mut reader).await.trim()).unwrap();
    assert!(list_reply.get("result").is_some(), "an ordinary, well-within-limit request must succeed exactly as it would over an unconfigured transport, got {list_reply}");

    drop(peer_write);
    drop(reader);
    server_handle
        .await
        .unwrap()
        .unwrap()
        .cancel()
        .await
        .unwrap();
}
