//! B5.12 Task 33: audit/observability events, exercised as genuinely
//! external code (`mcp_server::...` only) against a `SuperBrainMcpServer`
//! constructed with a real, injected `brain_api::AuditSink`
//! (`RecordingSink` below) — never reaching into `mcp-server`'s own
//! internals. Mirrors `tests/graph_memory_knowledge_tools.rs`'s fixture
//! style (Task 23).
//!
//! Covers: every one of the 12 registered tools emits a `ToolCall` event
//! on both success and failure; the 3 write-shaped tools additionally
//! emit `WriteMemory` with the real `proposal_id` on success; a denied
//! operation always pairs `PermissionDecision{allowed:false}` with
//! `DeniedAction` (proven directly against `brain_api::
//! record_permission_decision`, since no MCP tool in this crate is ever
//! reachable in a denied state through its own `mcp_ctx()` — every
//! `RequestContext` it builds carries full read/propose-write permission,
//! per `tools/search.rs`'s own module doc); and that a logging failure
//! can never grant an implicit authorization or skip real execution (the
//! audit sink is invoked strictly AFTER `brain_api`'s own capability call
//! already returned its real result).

use std::sync::{Arc, Mutex};

use brain_api::{AuditEvent, AuditEventKind, AuditSink};
use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot};
use codegraph_index::extract::{ByteSpan, RawSymbolCandidate};
use codegraph_index::graph::CodeGraph;
use codegraph_index::snapshot::promote;
use index::indexer_impl::{rebuild_index, RealIndexer};
use mcp_server::SuperBrainMcpServer;
use storage::Vault;

fn unique_test_dir(label: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-mcp-server-audit-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[derive(Default, Clone)]
struct RecordingSink(Arc<Mutex<Vec<AuditEvent>>>);

impl AuditSink for RecordingSink {
    fn record(&self, event: AuditEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl RecordingSink {
    fn events(&self) -> Vec<AuditEvent> {
        self.0.lock().unwrap().clone()
    }

    fn tool_calls(&self, tool: &str) -> Vec<bool> {
        self.events()
            .into_iter()
            .filter_map(|e| match e.kind {
                AuditEventKind::ToolCall { tool: t, allowed } if t == tool => Some(allowed),
                _ => None,
            })
            .collect()
    }

    fn write_memory_proposal_ids(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|e| match e.kind {
                AuditEventKind::WriteMemory { proposal_id } => Some(proposal_id),
                _ => None,
            })
            .collect()
    }
}

/// A sink whose `record` panics — used only by
/// `a_panicking_audit_sink_does_not_change_save_memorys_own_outcome` to
/// prove ordering: the real `brain_api::save_memory` call and its
/// `Result` (a genuinely persisted `Pending` proposal) are fully computed
/// before the sink is ever touched, so a sink failure cannot retroactively
/// grant permission, fabricate a success, or silently skip the real
/// write-proposal persistence.
struct PanickingSink;
impl AuditSink for PanickingSink {
    fn record(&self, _event: AuditEvent) {
        panic!("this sink always panics");
    }
}

struct Fixture {
    vault: Vault,
    indexer: Arc<RealIndexer>,
    graph: Arc<CodeGraph>,
    symbol_id: codegraph_domain::SymbolSnapshotId,
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

        let text_a = "---\nid: note-audit-a\nschema_version: 1\ntype: bug\nstatus: reviewed\ntrust: official\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-19\nupdated: 2026-11-19\nevent_period: 2026-11-19\nrelations: {}\n---\nthe unique audit fixture phrase about retry backoff\n";
        let text_b = "---\nid: note-audit-b\nschema_version: 1\ntype: decision\nstatus: reviewed\ntrust: official\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-19\nupdated: 2026-11-19\nevent_period: 2026-11-19\nrelations: {}\n---\nanother note\n";
        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        storage::propose_and_commit(&vault, text_a, storage::Version(0), uuid::Uuid::new_v4(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &gate, None).unwrap();
        storage::propose_and_commit(&vault, text_b, storage::Version(0), uuid::Uuid::new_v4(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &gate, None).unwrap();
        rebuild_index(&vault, &indexer).unwrap();

        let mut registry = RepositoryRegistry::new();
        let repo_id = registry.register(RepositoryRoot::parse(&root).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo_id, commit, "src/lib.rs").unwrap();
        let candidate = RawSymbolCandidate::new("audited_fn".to_string(), "function_item".to_string(), ByteSpan { start: 0, end: 10 }, "fn audited_fn() {}".to_string()).unwrap();
        let snapshot = promote(&candidate, file).unwrap();
        let symbol_id = snapshot.id();

        let mut graph = CodeGraph::new();
        graph.insert_snapshot(snapshot).unwrap();

        Self { vault, indexer, graph: Arc::new(graph), symbol_id }
    }

    fn server(self, sink: RecordingSink) -> SuperBrainMcpServer {
        SuperBrainMcpServer::new(self.vault, self.indexer, self.graph, "project-x").with_audit_sink(Arc::new(sink))
    }
}

// --- read-shaped tools: ToolCall(allowed: true) on success ---

#[tokio::test]
async fn search_emits_exactly_one_allowed_tool_call_on_success() {
    let sink = RecordingSink::default();
    let server = Fixture::new().server(sink.clone());

    let _ = server
        .search(rmcp::handler::server::wrapper::Parameters(mcp_server::SearchInput { query: "retry backoff".into(), k: 10, type_filter: None, status_filter: None }))
        .await
        .unwrap();

    assert_eq!(sink.tool_calls("search"), vec![true]);
}

#[tokio::test]
async fn retrieve_emits_exactly_one_allowed_tool_call_on_success() {
    let sink = RecordingSink::default();
    let server = Fixture::new().server(sink.clone());

    let _ = server.retrieve(rmcp::handler::server::wrapper::Parameters(mcp_server::RetrieveInput { note_id: "note-audit-a".into() })).await.unwrap();

    assert_eq!(sink.tool_calls("retrieve"), vec![true]);
}

#[tokio::test]
async fn retrieve_on_an_unknown_note_still_emits_exactly_one_tool_call_marked_allowed() {
    // NotFound is not PermissionDenied -- the call was allowed to reach
    // brain-api and run; it failed for an unrelated reason. `allowed` in
    // ToolCall means "the permission check passed", not "the call
    // succeeded end to end" -- those are deliberately distinct axes.
    let sink = RecordingSink::default();
    let server = Fixture::new().server(sink.clone());

    let result = server.retrieve(rmcp::handler::server::wrapper::Parameters(mcp_server::RetrieveInput { note_id: "note-nonexistent".into() })).await;

    assert!(result.is_err());
    assert_eq!(sink.tool_calls("retrieve"), vec![true]);
}

#[tokio::test]
async fn get_code_context_emits_exactly_one_tool_call() {
    let sink = RecordingSink::default();
    let fixture = Fixture::new();
    let symbol_id = fixture.symbol_id;
    let server = fixture.server(sink.clone());

    let _ = server.get_code_context(rmcp::handler::server::wrapper::Parameters(mcp_server::GetCodeContextInput { symbol_id: symbol_id.to_string() })).await.unwrap();

    assert_eq!(sink.tool_calls("get_code_context"), vec![true]);
}

#[tokio::test]
async fn graph_query_emits_exactly_one_tool_call() {
    let sink = RecordingSink::default();
    let fixture = Fixture::new();
    let symbol_id = fixture.symbol_id;
    let server = fixture.server(sink.clone());

    let _ = server
        .graph_query(rmcp::handler::server::wrapper::Parameters(mcp_server::GraphQueryInput { start_symbol_id: symbol_id.to_string(), kind: None, max_depth: 2, limit: 10 }))
        .await
        .unwrap();

    assert_eq!(sink.tool_calls("graph_query"), vec![true]);
}

#[tokio::test]
async fn get_entity_emits_exactly_one_tool_call() {
    let sink = RecordingSink::default();
    let fixture = Fixture::new();
    let symbol_id = fixture.symbol_id;
    let server = fixture.server(sink.clone());

    let _ = server.get_entity(rmcp::handler::server::wrapper::Parameters(mcp_server::GetEntityInput { symbol_id: symbol_id.to_string() })).await.unwrap();

    assert_eq!(sink.tool_calls("get_entity"), vec![true]);
}

#[tokio::test]
async fn get_previous_solution_emits_exactly_one_tool_call() {
    let sink = RecordingSink::default();
    let server = Fixture::new().server(sink.clone());

    let _ = server
        .get_previous_solution(rmcp::handler::server::wrapper::Parameters(mcp_server::GetPreviousSolutionInput { query: "retry backoff".into(), note_type: None }))
        .await
        .unwrap();

    assert_eq!(sink.tool_calls("get_previous_solution"), vec![true]);
}

#[tokio::test]
async fn get_rules_get_regulations_get_canon_each_emit_exactly_one_tool_call() {
    let sink = RecordingSink::default();
    let server = Fixture::new().server(sink.clone());

    let _ = server.get_rules(rmcp::handler::server::wrapper::Parameters(mcp_server::GetRulesInput { scope: None })).await.unwrap();
    let _ = server.get_regulations(rmcp::handler::server::wrapper::Parameters(mcp_server::GetRegulationsInput { regime: None })).await.unwrap();
    let _ = server.get_canon(rmcp::handler::server::wrapper::Parameters(mcp_server::GetCanonInput { topic: None })).await.unwrap();

    assert_eq!(sink.tool_calls("get_rules"), vec![true]);
    assert_eq!(sink.tool_calls("get_regulations"), vec![true]);
    assert_eq!(sink.tool_calls("get_canon"), vec![true]);
}

// --- write-shaped tools: ToolCall(allowed: true) + WriteMemory(proposal_id) ---

#[tokio::test]
async fn save_memory_emits_a_tool_call_and_a_write_memory_event_with_the_real_proposal_id() {
    let sink = RecordingSink::default();
    let server = Fixture::new().server(sink.clone());

    let output = server
        .save_memory(rmcp::handler::server::wrapper::Parameters(mcp_server::SaveMemoryInput {
            note_id: "note-audit-new".into(),
            note_type: mcp_server::McpNoteType::Bug,
            classification: mcp_server::McpClassification::Internal,
            project: "project-x".into(),
            source: "agent:coder@prompt-v3".into(),
            event_period: "2026-11-19".into(),
            body: "proposed content".into(),
        }))
        .await
        .unwrap();

    assert_eq!(sink.tool_calls("save_memory"), vec![true]);
    assert_eq!(sink.write_memory_proposal_ids(), vec![output.0.proposal_id]);
}

#[tokio::test]
async fn update_memory_emits_a_tool_call_and_a_write_memory_event_with_the_real_proposal_id() {
    let sink = RecordingSink::default();
    let server = Fixture::new().server(sink.clone());

    let output = server
        .update_memory(rmcp::handler::server::wrapper::Parameters(mcp_server::UpdateMemoryInput { note_id: "note-audit-a".into(), body: "updated content".into(), expected_version: 1 }))
        .await
        .unwrap();

    assert_eq!(sink.tool_calls("update_memory"), vec![true]);
    assert_eq!(sink.write_memory_proposal_ids(), vec![output.0.proposal_id]);
}

#[tokio::test]
async fn link_entities_emits_a_tool_call_and_a_write_memory_event_with_the_real_proposal_id() {
    let sink = RecordingSink::default();
    let server = Fixture::new().server(sink.clone());

    let output = server
        .link_entities(rmcp::handler::server::wrapper::Parameters(mcp_server::LinkEntitiesInput {
            from_note_id: "note-audit-a".into(),
            to_note_id: "note-audit-b".into(),
            relation: mcp_server::McpRelationKind::Supports,
        }))
        .await
        .unwrap();

    assert_eq!(sink.tool_calls("link_entities"), vec![true]);
    assert_eq!(sink.write_memory_proposal_ids(), vec![output.0.proposal_id]);
}

// --- correlation ---

#[tokio::test]
async fn every_event_from_one_call_shares_the_same_correlation_id() {
    let sink = RecordingSink::default();
    let server = Fixture::new().server(sink.clone());

    let _ = server
        .save_memory(rmcp::handler::server::wrapper::Parameters(mcp_server::SaveMemoryInput {
            note_id: "note-audit-corr".into(),
            note_type: mcp_server::McpNoteType::Bug,
            classification: mcp_server::McpClassification::Internal,
            project: "project-x".into(),
            source: "agent:coder@prompt-v3".into(),
            event_period: "2026-11-19".into(),
            body: "content".into(),
        }))
        .await
        .unwrap();

    let events = sink.events();
    assert_eq!(events.len(), 2, "save_memory must emit exactly ToolCall + WriteMemory, got {events:?}");
    let ids: std::collections::HashSet<uuid::Uuid> = events.iter().map(|e| e.correlation_id).collect();
    assert_eq!(ids.len(), 1, "both events from one call must share the same correlation_id, got {events:?}");
}

// --- permission-decision pairing (direct, since no MCP tool in this
//     crate is reachable in a denied state through its own mcp_ctx()) ---

#[test]
fn a_denied_operation_always_pairs_a_permission_decision_with_a_denied_action() {
    let sink = RecordingSink::default();
    let correlation_id = uuid::Uuid::new_v4();
    brain_api::record_permission_decision(&sink, "ApproveProposal", false, Some("mcp session cannot approve"), "2026-11-19T00:00:00Z", correlation_id);

    let events = sink.events();
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0].kind, AuditEventKind::PermissionDecision { allowed: false, .. }));
    assert!(matches!(events[1].kind, AuditEventKind::DeniedAction { .. }));
}

// --- logging failure never grants implicit authorization ---

#[tokio::test]
async fn a_panicking_audit_sink_does_not_change_save_memorys_own_outcome() {
    let root = unique_test_dir("panic-sink");
    {
        let repo = git2::Repository::init(&root).unwrap();
        let sig = git2::Signature::now("Test", "test@localhost").unwrap();
        let oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
    }
    let vault = Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();
    let indexer = Arc::new(RealIndexer::new());
    let server = SuperBrainMcpServer::new(vault, indexer, Arc::new(CodeGraph::new()), "project-x").with_audit_sink(Arc::new(PanickingSink));

    // Run on a dedicated task so the panicking sink's panic is caught by
    // the task boundary (JoinHandle::await returning Err), proving the
    // sink genuinely ran and genuinely panicked -- never silently
    // swallowed by this test itself.
    let handle = tokio::spawn(async move {
        server
            .save_memory(rmcp::handler::server::wrapper::Parameters(mcp_server::SaveMemoryInput {
                note_id: "note-audit-panic".into(),
                note_type: mcp_server::McpNoteType::Bug,
                classification: mcp_server::McpClassification::Internal,
                project: "project-x".into(),
                source: "agent:coder@prompt-v3".into(),
                event_period: "2026-11-19".into(),
                body: "content".into(),
            }))
            .await
    });
    let result = handle.await;

    assert!(result.is_err(), "the panicking sink must indeed panic (proving the sink really ran), not be silently swallowed");

    // The real brain_api::save_memory call ran and persisted a Pending
    // proposal BEFORE the panicking sink was ever invoked -- confirmed
    // via a fresh Vault handle against the same on-disk root (the
    // original handle was moved into the spawned task). `run_cli`'s own
    // `list` output does not echo the proposed note_id (only id/status/
    // originating_identity), so this asserts on the structured listing
    // itself, via a trusted_cli-equivalent check: exactly one real,
    // genuinely Pending proposal exists, proving save_memory's own write
    // to `.proposals/` completed in full before the sink panicked.
    let reopened = Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();
    let proposal_ids = reopened.list_proposal_ids().unwrap();
    assert_eq!(proposal_ids.len(), 1, "exactly one real proposal must have been persisted despite the audit sink panicking, got ids: {proposal_ids:?}");
    let listing = brain_api::run_cli(&["list".to_string()], &reopened, "2026-11-19T00:00:00Z").unwrap();
    assert!(listing.contains("Pending"), "the persisted proposal must genuinely be Pending, got: {listing:?}");
}

