//! B5.2 Task 24: real MCP protocol round-trip integration tests —
//! `SuperBrainMcpServer::get_info`/`ServerHandler` (Task 24's own addition)
//! served over a real in-memory transport (`tokio::io::duplex`, the same
//! `AsyncRead + AsyncWrite` pair `transport-io`'s stdio transport uses, just
//! not backed by a real stdin/stdout pipe) to a genuine MCP client sending
//! real JSON-RPC messages — never calling the `#[tool]` methods directly
//! as `tests/search_tools.rs`/`tests/graph_memory_knowledge_tools.rs` do.
//!
//! This is this crate's only test file that needs `rmcp`'s `client`
//! feature (dev-dependency only — see `Cargo.toml`'s own comment on that
//! entry); every other file in this crate exercises the server side only.

use std::sync::Arc;

use codegraph_index::graph::CodeGraph;
use index::indexer_impl::{RealIndexer, rebuild_index};
use mcp_server::SuperBrainMcpServer;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ErrorCode},
    object,
};
use storage::Vault;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn unique_test_dir(label: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-mcp-server-integration-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Real fixture: a `Vault` with one real committed note, a `RealIndexer`
/// kept current, and an empty-but-real `CodeGraph` — mirrors
/// `tests/search_tools.rs`'s own `Fixture`, built fresh here since this
/// file's own `server()` immediately moves it into a `serve()` call.
fn new_server() -> SuperBrainMcpServer {
    let root = unique_test_dir("fixture");
    {
        let repo = git2::Repository::init(&root).unwrap();
        let sig = git2::Signature::now("Test", "test@localhost").unwrap();
        let oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
    }
    let vault = Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();
    let indexer = Arc::new(RealIndexer::new());

    let text = "---\nid: note-integration-a\nschema_version: 1\ntype: bug\nstatus: reviewed\ntrust: official\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-18\nupdated: 2026-11-18\nevent_period: 2026-11-18\nrelations: {}\n---\nthe unique real-protocol fixture phrase\n";
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    storage::propose_and_commit(&vault, text, storage::Version(0), uuid::Uuid::new_v4(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &gate, None).unwrap();
    rebuild_index(&vault, &indexer).unwrap();

    SuperBrainMcpServer::new(vault, indexer, Arc::new(CodeGraph::new()), "project-x")
}

/// `()` is `ClientHandler` with no special capabilities — exactly what a
/// bare probe client that only sends `tools/list`/`tools/call` needs; see
/// `rmcp`'s own `test_default_list_capabilities.rs` for the same pattern.
async fn connected_client() -> (rmcp::service::RunningService<rmcp::RoleClient, ()>, rmcp::service::RunningService<rmcp::RoleServer, SuperBrainMcpServer>) {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    // `serve()` on each side blocks until it completes the `initialize`
    // handshake with its peer -- the two sides must be driven concurrently,
    // never one `.await`ed to completion before the other even starts, or
    // each side waits forever for a message the other hasn't sent yet.
    let (server_result, client_result) = tokio::join!(new_server().serve(server_transport), ().serve(client_transport));
    (client_result.expect("client should initialize"), server_result.expect("server should initialize"))
}

#[tokio::test]
async fn real_client_lists_exactly_the_12_registered_tools_over_the_wire() {
    let (client, server) = connected_client().await;

    let result = client.list_tools(None).await.expect("tools/list should succeed");
    let mut names: Vec<String> = result.tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();

    let mut expected = vec![
        "get_canon",
        "get_code_context",
        "get_entity",
        "get_previous_solution",
        "get_regulations",
        "get_rules",
        "graph_query",
        "link_entities",
        "retrieve",
        "save_memory",
        "search",
        "update_memory",
    ];
    expected.sort();
    assert_eq!(names, expected, "the real MCP tools/list response must expose exactly the 12 Brain API capabilities, no more, no fewer");

    client.cancel().await.expect("client should cancel cleanly");
    server.cancel().await.expect("server should cancel cleanly");
}

#[tokio::test]
async fn real_client_calls_search_over_the_wire_and_gets_real_structured_results() {
    let (client, server) = connected_client().await;

    let params = CallToolRequestParams::new("search").with_arguments(object!({
        "query": "unique real-protocol fixture phrase",
        "k": 10,
    }));
    let result = client.call_tool(params).await.expect("tools/call search should succeed");

    assert_ne!(result.is_error, Some(true), "search must not be reported as a tool-level error, got {result:?}");
    let structured = result.structured_content.expect("search's Json<SearchOutput> must produce structured_content");
    let results = structured["results"].as_array().expect("structured_content.results must be an array");
    assert_eq!(results.len(), 1, "expected exactly one real hit, got {structured:?}");
    assert_eq!(results[0]["note_id"], "note-integration-a");

    client.cancel().await.expect("client should cancel cleanly");
    server.cancel().await.expect("server should cancel cleanly");
}

#[tokio::test]
async fn real_client_calls_save_memory_over_the_wire_and_gets_a_proposal_id_never_a_committed_note() {
    let (client, server) = connected_client().await;

    let params = CallToolRequestParams::new("save_memory").with_arguments(object!({
        "note_id": "note-proposed-over-wire",
        "note_type": "decision",
        "classification": "internal",
        "project": "project-x",
        "source": "agent:integration-test",
        "event_period": "2026-11-18",
        "body": "proposed over the real wire",
    }));
    let result = client.call_tool(params).await.expect("tools/call save_memory should succeed");

    assert_ne!(result.is_error, Some(true), "save_memory must not be reported as a tool-level error, got {result:?}");
    let structured = result.structured_content.expect("save_memory's Json<SaveMemoryOutput> must produce structured_content");
    let proposal_id = structured["proposal_id"].as_str().expect("structured_content.proposal_id must be a string");
    assert!(uuid::Uuid::parse_str(proposal_id).is_ok(), "proposal_id must be a real UUID, got {proposal_id:?}");

    client.cancel().await.expect("client should cancel cleanly");
    server.cancel().await.expect("server should cancel cleanly");
}

#[tokio::test]
async fn calling_an_unregistered_tool_name_over_the_wire_is_a_typed_error_never_a_panic() {
    let (client, server) = connected_client().await;

    // approve_proposal/reject_proposal are never registered (Task 23);
    // confirm a real client calling them by name gets a typed protocol
    // error, never a connection drop or panic.
    for name in ["approve_proposal", "reject_proposal", "does_not_exist"] {
        let params = CallToolRequestParams::new(name);
        let result = client.call_tool(params).await;
        assert!(result.is_err(), "calling unregistered tool {name:?} must be a typed error, got {result:?}");
    }

    // The connection must still be alive after 3 rejected calls -- a real
    // request still succeeds afterwards.
    let listed = client.list_tools(None).await.expect("tools/list must still succeed after prior tool-not-found errors");
    assert!(!listed.tools.is_empty());

    client.cancel().await.expect("client should cancel cleanly");
    server.cancel().await.expect("server should cancel cleanly");
}

/// Well-formed JSON that is not a valid JSON-RPC message (`{"foo":"bar"}`)
/// is a protocol error: the transport replies with a structured
/// `Invalid Request` (-32600) JSON-RPC error and the connection survives to
/// serve the next real, valid request — never a panic, never a silently
/// dropped connection. This is `rmcp`'s own documented behavior (see its
/// `transport::async_rw::receive_responds_to_protocol_error` test); this
/// test pins that SuperBrain's own server, not just the bare transport,
/// exhibits it end to end.
#[tokio::test]
async fn malformed_json_rpc_input_gets_a_structured_protocol_error_not_a_panic_and_the_connection_survives() {
    let (server_io, peer_io) = tokio::io::duplex(8192);
    // `serve()` blocks until it completes the `initialize` handshake with
    // its peer, so it must run concurrently with this test driving that
    // handshake by hand on the other half of the duplex pair -- spawned,
    // never `.await`ed inline first (see `connected_client`'s own comment
    // for why awaiting it directly here would deadlock).
    let server_handle = tokio::spawn(new_server().serve(server_io));

    let (peer_read, mut peer_write) = tokio::io::split(peer_io);
    let mut lines = BufReader::new(peer_read).lines();

    async fn next_reply(lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>) -> serde_json::Value {
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
            .await
            .expect("server must reply promptly, not hang")
            .expect("read should not error")
            .expect("server must send a reply line, not close the connection");
        serde_json::from_str(&line).expect("the reply itself must be valid JSON")
    }

    // Real handshake first: a server with no completed `initialize` has no
    // lifecycle yet to send a protocol-error reply within, so this test's
    // real subject -- the server tolerating a malformed message mid-session
    // -- needs a genuinely initialized session, not a bare unhandshaked pipe.
    peer_write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test-client\",\"version\":\"0.0.1\"}}}\n")
        .await
        .unwrap();
    let init_reply = next_reply(&mut lines).await;
    assert!(init_reply.get("result").is_some(), "initialize must succeed before this test's real subject, got {init_reply}");
    peer_write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n").await.unwrap();

    // Well-formed JSON, but not a JSON-RPC message shape at all (no
    // "jsonrpc" field, no "method"/"result"/"error").
    peer_write.write_all(b"{\"not\":\"jsonrpc\"}\n").await.unwrap();

    let reply = next_reply(&mut lines).await;
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["error"]["code"], ErrorCode::INVALID_REQUEST.0, "a non-JSON-RPC-shaped message must be rejected as Invalid Request, got {reply}");

    // The connection must still be alive: a real request sent right after
    // the malformed one still gets a real response -- proving the server
    // process itself did not panic or exit on the malformed line.
    peer_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n").await.unwrap();
    let list_reply = next_reply(&mut lines).await;
    assert!(list_reply.get("result").is_some(), "tools/list after the malformed message must still succeed, got {list_reply}");

    drop(peer_write);
    drop(lines);
    server_handle.await.unwrap().expect("server task must not have panicked or errored").cancel().await.expect("server should still be cancellable after surviving the malformed message");
}
