//! Read delegation (B5.1 Task 16): thin wrappers over the existing,
//! real `api::search`/`api::retrieve` and a minimal `get_code_context`
//! built directly on `codegraph_index::graph::CodeGraph`'s own query
//! primitives (`CodeGraph::snapshot`/`outgoing_edges`/`incoming_edges`).
//!
//! Per the B5.0 frozen contract's architecture boundary, these three
//! capabilities execute DIRECTLY once the caller's read permission check
//! passes — never a proposal, never approval-gated, never touching
//! `storage::propose_and_commit` or any write path. `crates/api` itself
//! stays exactly what it already is; this module only adds a thin
//! permission-checking + error-mapping layer in front of it.
//!
//! **Known lifecycle gap, documented explicitly, not resolved here**:
//! B5 Task 16 implements query semantics for `search`/`get_code_context`,
//! not their underlying index/graph's LIFECYCLE. `search` takes an
//! already-built `&index::Index<'_>` (which itself needs a `RealIndexer`
//! kept in sync with the vault's writes over its whole lifetime — this
//! crate does not construct, persist, or load one); `get_code_context`
//! takes an already-built `&codegraph_index::graph::CodeGraph` (there is
//! no repository-level CodeGraph persistence/loading/orchestration
//! anywhere in this codebase yet — `CodeGraph::new()` is always empty,
//! `build_graph` only assembles one from snapshots/edges a caller already
//! extracted). The caller is responsible for building and supplying both;
//! that responsibility is explicitly out of this task's scope, per the
//! user's own framing.
//!
//! `retrieve` has no such gap — `api::retrieve` depends only on
//! `&storage::Vault`, which every other capability in this crate already
//! takes directly.

use crate::{BrainApiError, RequestContext};

/// `brain-api`'s own `RequestContext` has no `read_policy` field before
/// this task (Task 8 deliberately deferred it — "add the field when the
/// type/need it depends on is real, not before," the same ruling already
/// applied to `write_policy` at Task 10 and `operation` at Task 13/14).
/// Task 16 is the first capability in this crate that needs one, so this
/// is where it is added: an `Option<policy::ReadPolicy>`, the exact same
/// fail-closed shape `write_policy` already has (`None` denies every read
/// operation, mirroring `api::check`'s own behavior for a caller with no
/// policy at all).
fn require_read_policy(ctx: &RequestContext) -> Result<&policy::ReadPolicy, BrainApiError> {
    ctx.read_policy.as_ref().ok_or(BrainApiError::PermissionDenied)
}

fn check_read(ctx: &RequestContext, op: policy::ReadOperation) -> Result<(), BrainApiError> {
    let read_policy = require_read_policy(ctx)?;
    if !read_policy.allows(op) {
        return Err(BrainApiError::PermissionDenied);
    }
    Ok(())
}

fn api_context(ctx: &RequestContext) -> api::RequestContext {
    api::RequestContext { schema_version: api::SCHEMA_VERSION, correlation_id: ctx.correlation_id }
}

fn map_api_error(err: api::ApiError) -> BrainApiError {
    match err {
        api::ApiError::UnsupportedSchema => BrainApiError::UnsupportedOperation,
        api::ApiError::PermissionDenied => BrainApiError::PermissionDenied,
        api::ApiError::UnsupportedChunk => BrainApiError::InvalidInput("chunk retrieval is not supported".to_string()),
        api::ApiError::NotFound | api::ApiError::Deleted => BrainApiError::NotFound,
        api::ApiError::Inconsistent => BrainApiError::CorruptData,
        api::ApiError::ReadUnavailable => BrainApiError::RuntimeUnavailable,
        api::ApiError::Search(search_err) => map_search_error(search_err),
    }
}

fn map_search_error(err: index::search::SearchError) -> BrainApiError {
    match err {
        index::search::SearchError::EmptyQuery
        | index::search::SearchError::InvalidLimit
        | index::search::SearchError::EmptyProject => BrainApiError::InvalidInput(err.to_string()),
        index::search::SearchError::TimeResolution(_) => BrainApiError::InvalidInput(err.to_string()),
        index::search::SearchError::ReadUnavailable => BrainApiError::RuntimeUnavailable,
    }
}

/// Delegates to `api::search`. `ctx.read_policy` must allow
/// `ReadOperation::Search` or this fails closed with `PermissionDenied`
/// before `api::search`/the underlying index is ever touched.
pub fn search(ctx: &RequestContext, index: &index::Index<'_>, request: api::SearchRequest) -> Result<index::search::SearchResponse, BrainApiError> {
    check_read(ctx, policy::ReadOperation::Search)?;
    // api::search's own ReadPolicy check is still real and still runs
    // (api::check consults the SAME read_policy this function already
    // checked above) -- this is intentional double enforcement, not
    // redundant risk: api::search is itself callable directly by any
    // other crate, so it must keep its own check regardless of what any
    // particular caller (this one included) already verified.
    let read_policy = require_read_policy(ctx)?;
    let response = api::search(index, read_policy, api_context(ctx), request);
    response.result.map_err(map_api_error)
}

/// Delegates to `api::retrieve`. `ctx.read_policy` must allow
/// `ReadOperation::Retrieve` or this fails closed with `PermissionDenied`
/// before `api::retrieve`/`storage::Vault::read_note` is ever touched.
/// Never substitutes a before-image or any other fallback value on a
/// hash-mismatch/corruption signal — `api::ApiError::Inconsistent` maps
/// exactly onto `BrainApiError::CorruptData`, propagated as-is.
pub fn retrieve(ctx: &RequestContext, vault: &storage::Vault, request: api::RetrieveRequest) -> Result<api::RetrieveResponse, BrainApiError> {
    check_read(ctx, policy::ReadOperation::Retrieve)?;
    let read_policy = require_read_policy(ctx)?;
    let response = api::retrieve(vault, read_policy, api_context(ctx), request);
    response.result.map_err(map_api_error)
}

/// Minimal, structured code context for one symbol: exactly the fields
/// `codegraph_index::snapshot::SymbolSnapshot` already exposes, plus its
/// immediate graph neighbors (`CodeGraph::outgoing_edges`/`incoming_edges`,
/// both already deterministic and already ordered). No source-text
/// excerpt: this crate never reads a file off disk to manufacture one —
/// if `codegraph_index` does not already carry source text on the
/// snapshot, `get_code_context` returns structured metadata only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeContext {
    pub id: codegraph_domain::SymbolSnapshotId,
    pub kind: codegraph_domain::SymbolKind,
    pub name: String,
    pub signature: String,
    pub repository: codegraph_domain::RepositoryId,
    pub commit: codegraph_domain::CommitHash,
    pub relative_path: String,
    pub span_start: usize,
    pub span_end: usize,
    pub outgoing: Vec<CodeContextEdge>,
    pub incoming: Vec<CodeContextEdge>,
}

/// One immediate graph neighbor: just enough to identify it and the
/// relationship, without recursing into ITS own context (this is a
/// single-hop neighbor list, never a transitive expansion — the user's
/// own scope limit: "no call-chain expansion").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeContextEdge {
    pub other: codegraph_domain::SymbolSnapshotId,
    pub kind: codegraph_domain::EdgeKind,
}

/// Looks up `id` in `graph` and returns its minimal structured context.
/// `ctx.read_policy` must allow `ReadOperation::GetCodeContext` or this
/// fails closed with `PermissionDenied` before `graph` is ever queried.
/// `NotFound` if `id` is not present in `graph` — never a fuzzy/partial
/// match, never a guess.
///
/// `graph` is the caller's responsibility to have already built for the
/// target repository (this module's own doc comment explains why — no
/// CodeGraph lifecycle exists in this crate or anywhere else yet).
pub fn get_code_context(ctx: &RequestContext, graph: &codegraph_index::graph::CodeGraph, id: codegraph_domain::SymbolSnapshotId) -> Result<CodeContext, BrainApiError> {
    check_read(ctx, policy::ReadOperation::GetCodeContext)?;

    let snapshot = graph.snapshot(id).ok_or(BrainApiError::NotFound)?;
    let span = snapshot.span();
    let file = snapshot.file();

    let to_edge = |edge: &codegraph_index::edges::Edge, other_of: fn(&codegraph_index::edges::Edge) -> codegraph_domain::SymbolSnapshotId| {
        CodeContextEdge { other: other_of(edge), kind: edge.kind }
    };

    Ok(CodeContext {
        id: snapshot.id(),
        kind: snapshot.kind(),
        name: snapshot.name().as_str().to_string(),
        signature: snapshot.signature().as_str().to_string(),
        repository: file.repository(),
        commit: file.commit().clone(),
        relative_path: file.relative_path().to_string(),
        span_start: span.start,
        span_end: span.end,
        outgoing: graph.outgoing_edges(id, None).into_iter().map(|e| to_edge(e, |e| e.to)).collect(),
        incoming: graph.incoming_edges(id, None).into_iter().map(|e| to_edge(e, |e| e.from)).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallOrigin, WritePolicy};
    use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot};
    use codegraph_domain::EdgeKind;
    use codegraph_index::edges::Edge as GraphEdge;
    use codegraph_index::extract::{ByteSpan, RawSymbolCandidate};
    use codegraph_index::graph::CodeGraph;
    use codegraph_index::snapshot::promote;
    use index::indexer_impl::{rebuild_index, RealIndexer};
    use index::Index;
    use std::collections::HashSet;
    use std::sync::Arc;
    use storage::{NoteId, Vault, Version};
    use uuid::Uuid;

    fn unique_test_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-brain-api-read-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mcp_ctx_with_read(read_policy: policy::ReadPolicy) -> RequestContext {
        RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: Uuid::new_v4(),
            write_policy: Some(WritePolicy::mcp_agent()),
            read_policy: Some(read_policy),
            origin: CallOrigin::McpSession("session-read-test".into()),
        }
    }

    fn mcp_ctx_no_read_policy() -> RequestContext {
        RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: Uuid::new_v4(),
            write_policy: Some(WritePolicy::mcp_agent()),
            read_policy: None,
            origin: CallOrigin::McpSession("session-read-test".into()),
        }
    }

    // --- search / retrieve fixture, mirroring crates/api/tests/b1_api.rs's own ---

    struct Fixture {
        vault: Vault,
        indexer: Arc<RealIndexer>,
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = unique_test_dir();
            {
                let repo = git2::Repository::init(&root).unwrap();
                let sig = git2::Signature::now("Test", "test@localhost").unwrap();
                let oid = repo.index().unwrap().write_tree().unwrap();
                let tree = repo.find_tree(oid).unwrap();
                repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
            }
            let vault = Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();
            Self { vault, indexer: Arc::new(RealIndexer::new()), root }
        }

        fn write(&self, id: &str, body: &str) {
            let text = format!(
                "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: reviewed\ntrust: official\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-18\nupdated: 2026-11-18\nevent_period: 2026-11-18\nrelations: {{}}\n---\n{body}"
            );
            let result = storage::propose_and_commit(
                &self.vault,
                &text,
                Version(0),
                Uuid::new_v4(),
                &HashSet::new(),
                &HashSet::new(),
                &policy::Gate::new(policy::SecretPolicy::new(1)),
                None,
            )
            .unwrap();
            assert!(matches!(result, storage::CommitResult::Committed(_) | storage::CommitResult::GitPending { .. }));
            rebuild_index(&self.vault, &self.indexer).unwrap();
        }

        fn index(&self) -> Index<'_> {
            Index::new(&self.vault, &self.indexer, "project-x", chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap())
        }
    }

    fn allow_search_and_retrieve() -> policy::ReadPolicy {
        policy::ReadPolicy::new(true, true)
    }

    fn search_request(text: &str) -> api::SearchRequest {
        api::SearchRequest { text: text.into(), filters: Default::default(), k: 10 }
    }

    // --- search ---

    #[test]
    fn search_delegates_to_api_search_and_wraps_its_result() {
        let fixture = Fixture::new();
        fixture.write("note-a", "the unique searchable phrase");
        let index = fixture.index();
        let ctx = mcp_ctx_with_read(allow_search_and_retrieve());

        let response = search(&ctx, &index, search_request("unique searchable phrase")).unwrap();
        assert!(matches!(response.outcome, index::search::SearchOutcome::Matches { .. }), "expected at least one hit, got {:?}", response.outcome);
    }

    #[test]
    fn search_with_empty_query_returns_invalid_input() {
        let fixture = Fixture::new();
        let index = fixture.index();
        let ctx = mcp_ctx_with_read(allow_search_and_retrieve());

        let result = search(&ctx, &index, search_request(""));
        assert!(matches!(result, Err(BrainApiError::InvalidInput(_))), "expected InvalidInput, got {result:?}");
    }

    #[test]
    fn search_denied_without_read_policy() {
        let fixture = Fixture::new();
        let index = fixture.index();
        let ctx = mcp_ctx_no_read_policy();

        let result = search(&ctx, &index, search_request("anything"));
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn search_denied_when_read_policy_does_not_allow_search() {
        let fixture = Fixture::new();
        let index = fixture.index();
        let ctx = mcp_ctx_with_read(policy::ReadPolicy::new(false, true));

        let result = search(&ctx, &index, search_request("anything"));
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    // --- retrieve ---

    #[test]
    fn retrieve_not_found_note_returns_not_found() {
        let fixture = Fixture::new();
        let ctx = mcp_ctx_with_read(allow_search_and_retrieve());

        let request = api::RetrieveRequest { id: api::RetrieveTarget::Note(NoteId::parse("note-missing").unwrap()) };
        let result = retrieve(&ctx, &fixture.vault, request);
        assert_eq!(result, Err(BrainApiError::NotFound));
    }

    #[test]
    fn retrieve_delegates_to_api_retrieve_and_wraps_its_result() {
        let fixture = Fixture::new();
        fixture.write("note-b", "content");
        let ctx = mcp_ctx_with_read(allow_search_and_retrieve());

        let request = api::RetrieveRequest { id: api::RetrieveTarget::Note(NoteId::parse("note-b").unwrap()) };
        let response = retrieve(&ctx, &fixture.vault, request).unwrap();
        assert!(response.content.contains("content"));
    }

    #[test]
    fn retrieve_denied_without_read_policy() {
        let fixture = Fixture::new();
        let ctx = mcp_ctx_no_read_policy();
        let request = api::RetrieveRequest { id: api::RetrieveTarget::Note(NoteId::parse("note-a").unwrap()) };
        let result = retrieve(&ctx, &fixture.vault, request);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn a_hash_mismatch_on_the_underlying_read_never_falls_back_silently_through_brain_api() {
        // Pins constraint d9mm through the new wrapper layer: a tampered
        // record (bytes on disk no longer matching the journal's committed
        // hash) must surface as CorruptData, never a before-image or any
        // other silent substitute.
        let fixture = Fixture::new();
        fixture.write("note-c", "original content");
        let record_path = fixture.root.join("note-c.record");
        std::fs::write(&record_path, b"tampered bytes, does not match the committed hash").unwrap();

        let ctx = mcp_ctx_with_read(allow_search_and_retrieve());
        let request = api::RetrieveRequest { id: api::RetrieveTarget::Note(NoteId::parse("note-c").unwrap()) };
        let result = retrieve(&ctx, &fixture.vault, request);
        assert_eq!(result, Err(BrainApiError::CorruptData));
    }

    // --- get_code_context ---

    fn sample_graph_with_one_edge() -> (CodeGraph, codegraph_domain::SymbolSnapshotId, codegraph_domain::SymbolSnapshotId) {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, "src/main.rs").unwrap();

        let caller_candidate = RawSymbolCandidate {
            name: "caller".to_string(),
            kind_hint: "function_item".to_string(),
            span: ByteSpan { start: 0, end: 10 },
            signature_text: "fn caller()".to_string(),
        };
        let callee_candidate = RawSymbolCandidate {
            name: "callee".to_string(),
            kind_hint: "function_item".to_string(),
            span: ByteSpan { start: 20, end: 30 },
            signature_text: "fn callee()".to_string(),
        };
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
        (graph, caller_id, callee_id)
    }

    #[test]
    fn get_code_context_for_a_valid_symbol_returns_its_metadata_and_neighbors() {
        let (graph, caller_id, callee_id) = sample_graph_with_one_edge();
        let ctx = mcp_ctx_with_read(policy::ReadPolicy::new(false, false).with_get_code_context(true));

        let context = get_code_context(&ctx, &graph, caller_id).unwrap();
        assert_eq!(context.id, caller_id);
        assert_eq!(context.kind, codegraph_domain::SymbolKind::Function);
        assert_eq!(context.name, "caller");
        assert_eq!(context.signature, "fn caller()");
        assert_eq!(context.relative_path, "src/main.rs");
        assert_eq!(context.span_start, 0);
        assert_eq!(context.span_end, 10);
        assert_eq!(context.outgoing, vec![CodeContextEdge { other: callee_id, kind: EdgeKind::Calls }]);
        assert!(context.incoming.is_empty());
    }

    #[test]
    fn get_code_context_for_unknown_symbol_returns_not_found() {
        let (graph, _caller_id, _callee_id) = sample_graph_with_one_edge();
        let ctx = mcp_ctx_with_read(policy::ReadPolicy::new(false, false).with_get_code_context(true));

        let unknown = codegraph_domain::SymbolSnapshotId::derive(
            &FileIdentity::parse(
                RepositoryRegistry::new().register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap()),
                CommitHash::parse(&"b".repeat(40)).unwrap(),
                "src/other.rs",
            )
            .unwrap(),
            codegraph_domain::SymbolKind::Function,
            "nonexistent",
            "fn nonexistent()",
            0,
            10,
        );
        let result = get_code_context(&ctx, &graph, unknown);
        assert_eq!(result, Err(BrainApiError::NotFound));
    }

    #[test]
    fn get_code_context_denied_without_read_policy() {
        let (graph, caller_id, _callee_id) = sample_graph_with_one_edge();
        let ctx = mcp_ctx_no_read_policy();
        let result = get_code_context(&ctx, &graph, caller_id);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn get_code_context_denied_when_read_policy_does_not_allow_it() {
        let (graph, caller_id, _callee_id) = sample_graph_with_one_edge();
        // search/retrieve allowed, but NOT get_code_context -- proving the
        // three read operations are gated independently, not as one bloc.
        let ctx = mcp_ctx_with_read(policy::ReadPolicy::new(true, true));
        let result = get_code_context(&ctx, &graph, caller_id);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn get_code_context_is_deterministic() {
        let (graph, caller_id, _callee_id) = sample_graph_with_one_edge();
        let ctx = mcp_ctx_with_read(policy::ReadPolicy::new(false, false).with_get_code_context(true));
        let first = get_code_context(&ctx, &graph, caller_id).unwrap();
        let second = get_code_context(&ctx, &graph, caller_id).unwrap();
        assert_eq!(first, second);
    }
}
