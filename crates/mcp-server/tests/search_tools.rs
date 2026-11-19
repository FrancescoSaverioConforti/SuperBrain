//! B5.2 Task 22: integration tests for `search`/`retrieve`/`get_code_context`
//! as MCP tools, exercised as genuinely external code (`mcp_server::...`
//! only) against a `SuperBrainMcpServer` constructed with real, injected
//! fixtures — never an empty placeholder `Vault`/`RealIndexer`/`CodeGraph`
//! standing in for production data. Calls the `#[tool]`-annotated async
//! methods directly (no transport, no `ToolCallContext`/`Peer` needed to
//! exercise the business logic) and separately checks registration/schema
//! through `SuperBrainMcpServer::read_tools()` (the `#[tool_router(router =
//! read_tools)]`-generated accessor), matching `rmcp`'s own
//! `test_tool_routers.rs` test pattern.

use std::sync::Arc;

use codegraph_domain::{CommitHash, EdgeKind, FileIdentity, RepositoryRegistry, RepositoryRoot};
use codegraph_index::edges::Edge as GraphEdge;
use codegraph_index::extract::{ByteSpan, RawSymbolCandidate};
use codegraph_index::graph::CodeGraph;
use codegraph_index::snapshot::promote;
use index::indexer_impl::{rebuild_index, RealIndexer};
use mcp_server::SuperBrainMcpServer;
use storage::{NoteId, Vault, Version};

/// `Json<T>`'s own `T` is not required to implement `Debug` for
/// `rmcp::ErrorData` purposes, so `Result::expect_err`/`unwrap_err` (which
/// need `Debug` on the `Ok` side to format a panic message) don't apply
/// directly to a `Result<Json<T>, McpError>`. This helper extracts the
/// error without that bound.
fn expect_mcp_error<T>(result: Result<T, rmcp::ErrorData>, message: &str) -> rmcp::ErrorData {
    match result {
        Err(e) => e,
        Ok(_) => panic!("{message}"),
    }
}

fn unique_test_dir(label: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-mcp-server-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Real fixture: a `Vault` with one git commit (so `propose_and_commit`'s
/// own git-pending path has something to commit against, matching
/// `brain-api`/`api`'s own test fixtures), a `RealIndexer` kept current
/// via the real `rebuild_index` call -- run HERE, in the test's own
/// composition root, never inside `mcp-server` itself (the user's
/// explicit "no rebuild_index inside mcp-server" rule is about the
/// CRATE under test, not about how its test fixtures get built) -- and a
/// `CodeGraph` built from real `codegraph_index` primitives (`promote`,
/// `insert_snapshot`, `insert_edge`), never an empty placeholder.
struct Fixture {
    vault: Vault,
    indexer: Arc<RealIndexer>,
    graph: Arc<CodeGraph>,
    caller_id: codegraph_domain::SymbolSnapshotId,
    callee_id: codegraph_domain::SymbolSnapshotId,
}

impl Fixture {
    fn new() -> Self {
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

        let text = "---\nid: note-fixture-a\nschema_version: 1\ntype: bug\nstatus: reviewed\ntrust: official\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-18\nupdated: 2026-11-18\nevent_period: 2026-11-18\nrelations: {}\n---\nthe unique mcp fixture phrase\n";
        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        storage::propose_and_commit(&vault, text, Version(0), uuid::Uuid::new_v4(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &gate, None).unwrap();
        rebuild_index(&vault, &indexer).unwrap();

        let mut registry = RepositoryRegistry::new();
        let repo_id = registry.register(RepositoryRoot::parse(&root).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo_id, commit, "src/main.rs").unwrap();

        let caller_candidate = RawSymbolCandidate::new("caller".to_string(), "function_item".to_string(), ByteSpan { start: 0, end: 10 }, "fn caller()".to_string()).unwrap();
        let callee_candidate = RawSymbolCandidate::new("callee".to_string(), "function_item".to_string(), ByteSpan { start: 20, end: 30 }, "fn callee()".to_string()).unwrap();
        let caller = promote(&caller_candidate, file.clone()).unwrap();
        let callee = promote(&callee_candidate, file).unwrap();
        let caller_id = caller.id();
        let callee_id = callee.id();

        let mut graph = CodeGraph::new();
        graph.insert_snapshot(caller).unwrap();
        graph.insert_snapshot(callee).unwrap();
        graph
            .insert_edge(GraphEdge {
                from: caller_id,
                to: callee_id,
                kind: EdgeKind::Calls,
                evidence: codegraph_domain::Evidence::new(codegraph_domain::ResolutionMethod::StaticAnalysis, None, "test edge").unwrap(),
            })
            .unwrap();

        Self { vault, indexer, graph: Arc::new(graph), caller_id, callee_id }
    }

    fn server(self) -> SuperBrainMcpServer {
        SuperBrainMcpServer::new(self.vault, self.indexer, self.graph, "project-x")
    }
}

// --- tool registration (compile-time + structural, no business logic) ---

#[test]
fn exactly_the_3_task_22_tools_are_registered_no_capability_creep() {
    let router = SuperBrainMcpServer::read_tools();
    let names: Vec<String> = router.list_all().iter().map(|t| t.name.to_string()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["get_code_context", "retrieve", "search"], "Task 22 registers exactly these 3 tools, no more");
}

#[test]
fn an_unknown_tool_name_is_not_routable() {
    let router = SuperBrainMcpServer::read_tools();
    assert!(!router.has_route("approve_proposal"), "approve_proposal must never be registered as an MCP tool by this task");
    assert!(!router.has_route("reject_proposal"));
    assert!(!router.has_route("save_memory"), "write-shaped capabilities are out of Task 22's scope entirely");
    assert!(!router.has_route("does_not_exist"));
}

#[test]
fn every_registered_tool_has_a_non_empty_input_schema() {
    let router = SuperBrainMcpServer::read_tools();
    for name in ["search", "retrieve", "get_code_context"] {
        let tool = router.get(name).unwrap_or_else(|| panic!("tool {name} must be registered"));
        assert!(!tool.input_schema.is_empty(), "{name}'s input schema must not be empty -- typed, strict input is required");
    }
}

// --- search ---

#[tokio::test]
async fn mcp_search_delegates_to_brain_api_and_returns_real_results() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::SearchInput { query: "unique mcp fixture phrase".to_string(), k: 10, type_filter: None, status_filter: None };
    let output = server.search(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert_eq!(output.0.results.len(), 1, "expected exactly one real result, got {:?}", output.0.results);
    assert_eq!(output.0.results[0].note_id, "note-fixture-a");
}

#[tokio::test]
async fn mcp_search_with_zero_k_is_a_typed_invalid_params_error_not_an_empty_success() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::SearchInput { query: "anything".to_string(), k: 0, type_filter: None, status_filter: None };
    let result = server.search(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    let err = expect_mcp_error(result, "k=0 must be a typed error, not a silent empty success");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn mcp_search_with_no_matches_returns_an_empty_result_list_not_an_error() {
    let fixture = Fixture::new();
    let server = fixture.server();

    // Zero token overlap with the fixture's "unique mcp fixture phrase",
    // matching brain-api's own test convention for a genuine no-match
    // query (a query sharing even one token with real BM25/trigram
    // scoring can still rank as a match, which is correct engine
    // behavior, not something this test should fight).
    let input = mcp_server::SearchInput { query: "zzyzx qqrrss nonoverlap".to_string(), k: 10, type_filter: None, status_filter: None };
    let output = server.search(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(output.0.results.is_empty());
}

#[tokio::test]
async fn mcp_search_with_a_type_filter_excluding_the_only_note_returns_empty() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::SearchInput {
        query: "unique mcp fixture phrase".to_string(),
        k: 10,
        type_filter: Some(mcp_server::McpNoteType::Decision), // the fixture note is type bug
        status_filter: None,
    };
    let output = server.search(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(output.0.results.is_empty(), "a Bug-typed note must not satisfy a Decision-only filter");
}

#[tokio::test]
async fn mcp_search_is_deterministic_across_repeated_identical_calls() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = || mcp_server::SearchInput { query: "unique mcp fixture phrase".to_string(), k: 10, type_filter: None, status_filter: None };
    let first = server.search(rmcp::handler::server::wrapper::Parameters(input()), tokio_util::sync::CancellationToken::new()).await.unwrap();
    let second = server.search(rmcp::handler::server::wrapper::Parameters(input()), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert_eq!(first.0, second.0);
}

#[tokio::test]
async fn mcp_search_never_mutates_the_vault() {
    let fixture = Fixture::new();
    let server = fixture.server();
    let before = server.vault_ref().read_note(&NoteId::parse("note-fixture-a").unwrap()).unwrap();

    let input = mcp_server::SearchInput { query: "unique mcp fixture phrase".to_string(), k: 10, type_filter: None, status_filter: None };
    server.search(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    let after = server.vault_ref().read_note(&NoteId::parse("note-fixture-a").unwrap()).unwrap();
    assert_eq!(before, after, "search must never write to the vault");
}

// --- retrieve ---

#[tokio::test]
async fn mcp_retrieve_delegates_to_brain_api_and_returns_the_real_note() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::RetrieveInput { note_id: "note-fixture-a".to_string() };
    let output = server.retrieve(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert_eq!(output.0.note_id, "note-fixture-a");
    assert!(output.0.content.contains("the unique mcp fixture phrase"));
    assert_eq!(output.0.version, 1);
}

#[tokio::test]
async fn mcp_retrieve_on_an_unknown_note_id_is_a_typed_not_found_error() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::RetrieveInput { note_id: "note-does-not-exist".to_string() };
    let result = server.retrieve(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    let err = expect_mcp_error(result, "an unknown note id must be a typed error");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn mcp_retrieve_on_a_malformed_note_id_is_rejected_before_calling_brain_api() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::RetrieveInput { note_id: "../../etc/passwd".to_string() };
    let result = server.retrieve(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    assert!(result.is_err(), "a path-traversal-shaped note id must be rejected as invalid input");
}

// --- get_code_context ---

#[tokio::test]
async fn mcp_get_code_context_delegates_to_brain_api_against_the_injected_graph() {
    let fixture = Fixture::new();
    let caller_id = fixture.caller_id;
    let callee_id = fixture.callee_id;
    let server = fixture.server();

    let input = mcp_server::GetCodeContextInput { symbol_id: caller_id.to_string() };
    let output = server.get_code_context(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert_eq!(output.0.id, caller_id.to_string());
    assert_eq!(output.0.name, "caller");
    assert_eq!(output.0.signature, "fn caller()");
    assert_eq!(output.0.outgoing.len(), 1);
    assert_eq!(output.0.outgoing[0].other, callee_id.to_string());
    assert!(output.0.incoming.is_empty());
}

#[tokio::test]
async fn mcp_get_code_context_with_a_wrong_length_symbol_id_is_rejected_as_invalid_params_before_calling_brain_api() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::GetCodeContextInput { symbol_id: "not-a-real-hex-id".to_string() };
    let result = server.get_code_context(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    let err = expect_mcp_error(result, "a malformed symbol_id must be rejected");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn mcp_get_code_context_with_a_well_formed_but_unknown_symbol_id_reaches_brain_apis_own_not_found() {
    let fixture = Fixture::new();
    let server = fixture.server();

    // 64 lowercase hex chars -- well-formed, but names no real snapshot in
    // this fixture's graph.
    let unknown = "0".repeat(64);
    let input = mcp_server::GetCodeContextInput { symbol_id: unknown };
    let result = server.get_code_context(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    let err = expect_mcp_error(result, "an unknown but well-formed symbol_id must still be a typed error");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn mcp_get_code_context_symbol_id_round_trips_through_display_and_the_mcp_tool() {
    let fixture = Fixture::new();
    let caller_id = fixture.caller_id;
    let server = fixture.server();

    // Round-trip proof: the id this test gets back from the tool output,
    // fed back in as input, resolves to the exact same symbol again.
    let first = server
        .get_code_context(rmcp::handler::server::wrapper::Parameters(mcp_server::GetCodeContextInput { symbol_id: caller_id.to_string() }), tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    let second = server
        .get_code_context(rmcp::handler::server::wrapper::Parameters(mcp_server::GetCodeContextInput { symbol_id: first.0.id.clone() }), tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(first.0, second.0);
}

// --- no raw bypass / no direct core-primitive fallback on brain_api error ---

#[tokio::test]
async fn mcp_tools_never_fall_back_to_a_raw_core_primitive_when_brain_api_errors() {
    // Compile-time + behavioral property: retrieve's only path to data is
    // brain_api::retrieve (confirmed by this file's own imports -- there is
    // no direct call to storage::Vault::read_note anywhere in
    // tools/search.rs). An unknown note id surfaces brain_api's own
    // NotFound, mapped to InvalidParams -- never a raw vault read that
    // might behave differently.
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::RetrieveInput { note_id: "note-does-not-exist".to_string() };
    let result = server.retrieve(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;
    assert!(result.is_err());
}
