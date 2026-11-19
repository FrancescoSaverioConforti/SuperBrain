//! B5.2 Task 23: integration tests for `graph_query`/`get_entity`/
//! `link_entities`/`get_previous_solution`/`save_memory`/`update_memory`/
//! `get_rules`/`get_regulations`/`get_canon` as MCP tools, exercised as
//! genuinely external code (`mcp_server::...` only) against a
//! `SuperBrainMcpServer` constructed with real, injected fixtures.
//! Mirrors `tests/search_tools.rs`'s own fixture/assertion style (Task 22).
//!
//! The two anti-capability-creep assertions this task owns:
//! `mcp_server_never_registers_approve_or_reject_proposal_as_tools` and
//! `every_registered_mcp_tool_maps_1to1_to_a_named_brain_api_operation`.

use std::sync::Arc;

use codegraph_domain::{CommitHash, EdgeKind, FileIdentity, RepositoryRegistry, RepositoryRoot};
use codegraph_index::extract::{ByteSpan, RawSymbolCandidate};
use codegraph_index::graph::CodeGraph;
use codegraph_index::snapshot::promote;
use index::indexer_impl::{rebuild_index, RealIndexer};
use mcp_server::SuperBrainMcpServer;
use storage::{NoteId, Vault};

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

struct Fixture {
    vault: Vault,
    indexer: Arc<RealIndexer>,
    graph: Arc<CodeGraph>,
    a_id: codegraph_domain::SymbolSnapshotId,
    b_id: codegraph_domain::SymbolSnapshotId,
}

impl Fixture {
    fn new() -> Self {
        let root = unique_test_dir("fixture-t23");
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("Test", "test@localhost").unwrap();
            let oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
        }
        let vault = Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();
        let indexer = Arc::new(RealIndexer::new());

        let text = "---\nid: note-bug-a\nschema_version: 1\ntype: bug\nstatus: reviewed\ntrust: official\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-18\nupdated: 2026-11-18\nevent_period: 2026-11-18\nrelations: {}\n---\nthe unique previous-solution phrase\n";
        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        storage::propose_and_commit(&vault, text, storage::Version(0), uuid::Uuid::new_v4(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &gate, None).unwrap();
        rebuild_index(&vault, &indexer).unwrap();

        let mut registry = RepositoryRegistry::new();
        let repo_id = registry.register(RepositoryRoot::parse(&root).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo_id, commit, "src/lib.rs").unwrap();

        let a_candidate = RawSymbolCandidate::new("a".to_string(), "function_item".to_string(), ByteSpan { start: 0, end: 10 }, "fn a() { b(); }".to_string()).unwrap();
        let b_candidate = RawSymbolCandidate::new("b".to_string(), "function_item".to_string(), ByteSpan { start: 20, end: 30 }, "fn b() {}".to_string()).unwrap();
        let a = promote(&a_candidate, file.clone()).unwrap();
        let b = promote(&b_candidate, file).unwrap();
        let a_id = a.id();
        let b_id = b.id();

        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a).unwrap();
        graph.insert_snapshot(b).unwrap();
        graph
            .insert_edge(codegraph_index::edges::Edge {
                from: a_id,
                to: b_id,
                kind: EdgeKind::Calls,
                evidence: codegraph_domain::Evidence::new(codegraph_domain::ResolutionMethod::StaticAnalysis, None, "test edge").unwrap(),
            })
            .unwrap();

        Self { vault, indexer, graph: Arc::new(graph), a_id, b_id }
    }

    fn server(self) -> SuperBrainMcpServer {
        SuperBrainMcpServer::new(self.vault, self.indexer, self.graph, "project-x")
    }
}

// --- registration / anti-capability-creep (this task's own required assertions) ---

#[test]
fn mcp_server_never_registers_approve_or_reject_proposal_as_tools() {
    let router = SuperBrainMcpServer::read_tools() + SuperBrainMcpServer::graph_tools() + SuperBrainMcpServer::memory_tools() + SuperBrainMcpServer::knowledge_tools();
    assert!(!router.has_route("approve_proposal"), "approve_proposal must never be registered as an MCP tool");
    assert!(!router.has_route("reject_proposal"), "reject_proposal must never be registered as an MCP tool");
    assert!(!router.has_route("list_proposals"), "list_proposals (CLI-only approval surface) must never be registered as an MCP tool");
}

#[test]
fn every_registered_mcp_tool_maps_1to1_to_a_named_brain_api_operation() {
    // Anti-goal: "Silent capability creep: any MCP tool whose effect isn't
    // 1:1 (or documented N:1) traceable to a named Brain API operation."
    let router = SuperBrainMcpServer::read_tools() + SuperBrainMcpServer::graph_tools() + SuperBrainMcpServer::memory_tools() + SuperBrainMcpServer::knowledge_tools();
    let known_capabilities = [
        "search",
        "retrieve",
        "get_code_context",
        "graph_query",
        "get_entity",
        "get_previous_solution",
        "get_rules",
        "get_regulations",
        "get_canon",
        "save_memory",
        "update_memory",
        "link_entities",
    ];
    let names: Vec<String> = router.list_all().iter().map(|t| t.name.to_string()).collect();
    for name in &names {
        assert!(known_capabilities.contains(&name.as_str()), "undocumented tool: {name}");
    }
    let mut sorted = names.clone();
    sorted.sort();
    let mut expected: Vec<String> = known_capabilities.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(sorted, expected, "exactly the 12 Brain API capability names must be registered across Task 22 + Task 23, no more, no fewer");
}

#[test]
fn exactly_the_9_task_23_tools_are_registered_no_capability_creep() {
    let router = SuperBrainMcpServer::graph_tools() + SuperBrainMcpServer::memory_tools() + SuperBrainMcpServer::knowledge_tools();
    let mut names: Vec<String> = router.list_all().iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "get_canon",
            "get_entity",
            "get_previous_solution",
            "get_regulations",
            "get_rules",
            "graph_query",
            "link_entities",
            "save_memory",
            "update_memory",
        ],
        "Task 23 registers exactly these 9 tools, no more"
    );
}

// --- graph_query ---

#[tokio::test]
async fn mcp_graph_query_delegates_to_brain_api_and_returns_the_reachable_chain() {
    let fixture = Fixture::new();
    let a_id = fixture.a_id;
    let b_id = fixture.b_id;
    let server = fixture.server();

    let input = mcp_server::GraphQueryInput { start_symbol_id: a_id.to_string(), kind: Some(mcp_server::McpEdgeKind::Calls), max_depth: 10, limit: 100 };
    let output = server.graph_query(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert_eq!(output.0.symbol_ids, vec![a_id.to_string(), b_id.to_string()]);
}

#[tokio::test]
async fn mcp_graph_query_with_a_malformed_start_symbol_id_is_rejected_as_invalid_params() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::GraphQueryInput { start_symbol_id: "not-a-real-hex-id".to_string(), kind: None, max_depth: 10, limit: 100 };
    let result = server.graph_query(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    let err = expect_mcp_error(result, "a malformed start_symbol_id must be rejected");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn mcp_graph_query_exceeding_the_depth_bound_is_a_typed_invalid_params_error() {
    let fixture = Fixture::new();
    let a_id = fixture.a_id;
    let server = fixture.server();

    let input = mcp_server::GraphQueryInput { start_symbol_id: a_id.to_string(), kind: None, max_depth: 10_000, limit: 100 };
    let result = server.graph_query(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    let err = expect_mcp_error(result, "max_depth exceeding the bound must be a typed error");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

// --- get_entity ---

#[tokio::test]
async fn mcp_get_entity_delegates_to_brain_api_and_returns_intrinsic_facts_only() {
    let fixture = Fixture::new();
    let a_id = fixture.a_id;
    let server = fixture.server();

    let input = mcp_server::GetEntityInput { symbol_id: a_id.to_string() };
    let output = server.get_entity(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert_eq!(output.0.id, a_id.to_string());
    assert_eq!(output.0.name, "a");
    assert_eq!(output.0.signature, "fn a() { b(); }");
}

#[tokio::test]
async fn mcp_get_entity_with_a_well_formed_but_unknown_symbol_id_is_a_typed_not_found_error() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let unknown = "0".repeat(64);
    let input = mcp_server::GetEntityInput { symbol_id: unknown };
    let result = server.get_entity(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    let err = expect_mcp_error(result, "an unknown but well-formed symbol_id must still be a typed error");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

// --- link_entities ---

#[tokio::test]
async fn mcp_link_entities_returns_a_proposal_id_never_a_committed_result() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::LinkEntitiesInput { from_note_id: "note-a".to_string(), to_note_id: "note-b".to_string(), relation: mcp_server::McpRelationKind::Supports };
    let output = server.link_entities(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(uuid::Uuid::parse_str(&output.0.proposal_id).is_ok(), "proposal_id must be a real UUID, got {:?}", output.0.proposal_id);
}

#[tokio::test]
async fn mcp_link_entities_never_mutates_either_note_directly() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::LinkEntitiesInput { from_note_id: "note-never-created-a".to_string(), to_note_id: "note-never-created-b".to_string(), relation: mcp_server::McpRelationKind::Contradicts };
    server.link_entities(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(matches!(server.vault_ref().read_note(&NoteId::parse("note-never-created-a").unwrap()), Err(storage::StorageError::NotFound { .. })));
    assert!(matches!(server.vault_ref().read_note(&NoteId::parse("note-never-created-b").unwrap()), Err(storage::StorageError::NotFound { .. })));
}

#[tokio::test]
async fn mcp_link_entities_with_a_malformed_note_id_is_rejected_as_invalid_params_before_calling_brain_api() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::LinkEntitiesInput { from_note_id: "../../etc/passwd".to_string(), to_note_id: "note-b".to_string(), relation: mcp_server::McpRelationKind::Supports };
    let result = server.link_entities(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    assert!(result.is_err(), "a path-traversal-shaped note id must be rejected as invalid input");
}

// --- save_memory ---

#[tokio::test]
async fn mcp_save_memory_returns_a_proposal_id_never_a_committed_note() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::SaveMemoryInput {
        note_id: "note-new-via-mcp".to_string(),
        note_type: mcp_server::McpNoteType::Decision,
        classification: mcp_server::McpClassification::Internal,
        project: "project-x".to_string(),
        source: "agent:mcp-test".to_string(),
        event_period: "2026-11-18".to_string(),
        body: "proposed via mcp".to_string(),
    };
    let output = server.save_memory(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(uuid::Uuid::parse_str(&output.0.proposal_id).is_ok());
    // The proposed note must not exist as a real, committed note yet.
    assert!(matches!(server.vault_ref().read_note(&NoteId::parse("note-new-via-mcp").unwrap()), Err(storage::StorageError::NotFound { .. })));
}

#[tokio::test]
async fn mcp_save_memory_with_a_malformed_note_id_is_rejected_as_invalid_params_before_calling_brain_api() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::SaveMemoryInput {
        note_id: "../../etc/passwd".to_string(),
        note_type: mcp_server::McpNoteType::Decision,
        classification: mcp_server::McpClassification::Internal,
        project: "project-x".to_string(),
        source: "agent:mcp-test".to_string(),
        event_period: "2026-11-18".to_string(),
        body: "x".to_string(),
    };
    let result = server.save_memory(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    assert!(result.is_err(), "a path-traversal-shaped note id must be rejected as invalid input");
}

// --- update_memory ---

#[tokio::test]
async fn mcp_update_memory_returns_a_proposal_id_never_a_committed_note() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::UpdateMemoryInput { note_id: "note-bug-a".to_string(), body: "updated body via mcp".to_string(), expected_version: 1 };
    let output = server.update_memory(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(uuid::Uuid::parse_str(&output.0.proposal_id).is_ok());
    // The real note's content must be unchanged by a mere proposal.
    let real = server.vault_ref().read_note(&NoteId::parse("note-bug-a").unwrap()).unwrap();
    assert!(real.content.contains("the unique previous-solution phrase"));
    assert!(!real.content.contains("updated body via mcp"));
}

// --- get_previous_solution ---

#[tokio::test]
async fn mcp_get_previous_solution_delegates_to_brain_api_and_returns_real_results() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::GetPreviousSolutionInput { query: "unique previous-solution phrase".to_string(), note_type: None };
    let output = server.get_previous_solution(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert_eq!(output.0.results.len(), 1, "expected exactly one real result, got {:?}", output.0.results);
    assert_eq!(output.0.results[0].note_id, "note-bug-a");
}

#[tokio::test]
async fn mcp_get_previous_solution_with_no_matches_returns_an_empty_result_list_not_an_error() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::GetPreviousSolutionInput { query: "zzyzx qqrrss nonoverlap".to_string(), note_type: None };
    let output = server.get_previous_solution(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(output.0.results.is_empty());
}

// --- get_rules / get_regulations / get_canon ---

#[tokio::test]
async fn mcp_get_rules_returns_real_registry_entries() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::GetRulesInput { scope: None };
    let output = server.get_rules(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(!output.0.items.is_empty(), "the known rules registry must not be empty");
}

#[tokio::test]
async fn mcp_get_rules_with_an_empty_scope_string_is_a_typed_invalid_params_error() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::GetRulesInput { scope: Some(String::new()) };
    let result = server.get_rules(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    let err = expect_mcp_error(result, "an empty scope filter must be a typed error, not a silent empty success");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn mcp_get_regulations_with_no_regime_filter_succeeds() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::GetRegulationsInput { regime: None };
    let output = server.get_regulations(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(output.0.items.is_empty(), "no regulatory regime is registered yet, per brain_api::get_regulations's own documented behavior");
}

#[tokio::test]
async fn mcp_get_canon_always_returns_an_empty_list_today_but_still_succeeds() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::GetCanonInput { topic: None };
    let output = server.get_canon(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await.unwrap();

    assert!(output.0.entries.is_empty(), "no canonization mechanism exists yet, per brain_api::get_canon's own documented behavior");
}

#[tokio::test]
async fn mcp_get_canon_with_an_empty_topic_string_is_a_typed_invalid_params_error() {
    let fixture = Fixture::new();
    let server = fixture.server();

    let input = mcp_server::GetCanonInput { topic: Some(String::new()) };
    let result = server.get_canon(rmcp::handler::server::wrapper::Parameters(input), tokio_util::sync::CancellationToken::new()).await;

    let err = expect_mcp_error(result, "an empty topic filter must be a typed error");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}
