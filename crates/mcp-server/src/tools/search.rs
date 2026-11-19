//! B5.2 Task 22: `search`/`retrieve`/`get_code_context` as MCP tools — thin
//! schema-mapping wrappers over `brain_api::read::{search, retrieve,
//! get_code_context}` (Task 16). Each tool: deserialize typed MCP input →
//! build a `brain_api::RequestContext` server-side → call the matching
//! `brain_api` function → map its result/error onto typed MCP output. No
//! tool here duplicates `brain-api`'s own permission logic, falls back to
//! a core primitive directly on a `brain_api` error, or builds/rebuilds
//! `Index`/`CodeGraph` itself — those are owned by [`SuperBrainMcpServer`]'s
//! caller (its composition root), injected at construction time.
//!
//! **Trust boundary (unchanged from Task 20/21):** every `RequestContext`
//! built in this file uses `CallOrigin::McpSession` and
//! `WritePolicy::mcp_agent()` — the only two values of each this crate can
//! construct at all (`brain_api::WritePolicy::trusted_cli` and
//! `brain_api::TrustedLocalToken::new` are both `pub(crate)` to
//! `brain-api`, unreachable from here). None of this file's three tools is
//! write-shaped, so `write_policy` is set purely for the `RequestContext`
//! struct's own completeness, never consulted by `search`/`retrieve`/
//! `get_code_context` (confirmed: `brain_api::read`'s `check_read` only
//! ever reads `ctx.read_policy`).
//!
//! **Session identity**: B5.8's real MCP connection/session-binding layer
//! does not exist yet (same deferral `brain-api`'s own `CallOrigin`
//! doc comment already states). `rmcp::service::RequestContext<RoleServer>`
//! carries no per-connection identity string useful as one either (its
//! `id` is a per-REQUEST id, not a per-connection one). Per the user's own
//! instruction, this crate does not invent that mechanism now: every
//! session uses the fixed, non-identity-leaking placeholder label below,
//! clearly distinct from `brain_api::CallOrigin::TrustedLocal`'s
//! `"trusted-local"` label so the two can never be confused, and
//! explicitly documented as a placeholder for B5.8, not a real identity.
//! This is inert for all 3 tools in this file today (none of them read
//! `ctx.origin`), but kept structurally honest rather than quietly
//! pretending a real session binding exists.
//!
//! **Dependency injection, not lifecycle ownership**: [`SuperBrainMcpServer`]
//! owns `storage::Vault` (not `Clone`, held directly), `Arc<RealIndexer>`,
//! and `Arc<CodeGraph>` — all three already built and handed to
//! [`SuperBrainMcpServer::new`] by its caller. This crate never calls
//! `index::indexer_impl::rebuild_index`, never scans a repository, and
//! never constructs a `CodeGraph` from source. Repository-level
//! construction, synchronization, and lifecycle ownership of `Index`/
//! `CodeGraph` remain a separate composition/runtime concern, out of this
//! task's scope by the user's explicit instruction — this is a documented
//! limitation, not a blocker for the tool wiring itself: every tool below
//! is a real, working adapter the moment it is constructed with live data.

use std::sync::Arc;

use brain_api::{CallOrigin, RequestContext, WritePolicy};
use codegraph_index::graph::CodeGraph;
use index::indexer_impl::RealIndexer;
use rmcp::{
    ErrorData as McpError,
    handler::server::{router::tool::ToolRouter, wrapper::{Json, Parameters}},
    schemars,
};
use serde::{Deserialize, Serialize};

use crate::error::map_brain_api_error;

/// Placeholder MCP-session identity label, pending B5.8's real
/// connection/session-binding layer (see this module's own doc comment).
/// Never derived from, or settable by, any tool call's own arguments —
/// every `RequestContext` this crate builds uses this same fixed constant.
const MCP_SESSION_PLACEHOLDER: &str = "mcp-session-unbound";

/// SuperBrain's MCP server (B5.2): holds the already-built runtime
/// dependencies its tools query through `brain-api`, injected by this
/// struct's caller — never constructed, rebuilt, or persisted by this
/// crate itself.
#[derive(Clone)]
pub struct SuperBrainMcpServer {
    vault: Arc<storage::Vault>,
    indexer: Arc<RealIndexer>,
    graph: Arc<CodeGraph>,
    active_project: String,
    /// B5.2 Task 31 (E2E flow 1): an optional real `DenseScorer`, applied
    /// to every `Index` this server builds for `search`/
    /// `get_previous_solution` when present. `None` by default —
    /// EVERY existing call site/test that constructs a
    /// `SuperBrainMcpServer` via [`Self::new`] is completely unaffected
    /// (identical `RetrievalMode::Lexical` behavior, matching
    /// `index::Index::new`'s own default); only
    /// [`Self::with_dense_retrieval`] opts a server instance into
    /// `RetrievalMode::DenseV2`. This crate still never builds, loads, or
    /// rebuilds a dense index itself — the caller (this crate's own
    /// `tests/e2e_agent_search.rs`, today) supplies an already-built
    /// `Arc<dyn DenseScorer>`, the same "dependency injection, not
    /// lifecycle ownership" rule this struct's own module doc comment
    /// already states for `vault`/`indexer`/`graph`.
    dense_scorer: Option<Arc<dyn index::search::DenseScorer>>,
    /// B5.12 Task 33: the durable audit sink every tool call below records
    /// a `ToolCall`/`PermissionDecision`/`WriteMemory` event to, via
    /// `brain_api::audit`'s content-free schema. Defaults to
    /// `brain_api::NullAuditSink` (see [`Self::new`]) — exactly the same
    /// "additive, defaults to a no-op, every existing call site/test
    /// unaffected" discipline `dense_scorer`'s own doc comment above
    /// already established for Task 31; only [`Self::with_audit_sink`]
    /// opts a server instance into a real destination.
    audit: Arc<dyn brain_api::AuditSink>,
    /// B5.16 Task 35: bounds how many `#[tool]` handlers run at once
    /// across this server instance. `Arc`-shared (not a plain
    /// `tokio::sync::Semaphore` field) specifically so every `Clone` of
    /// `SuperBrainMcpServer` shares the SAME permit pool, never an
    /// independent one per clone — `rmcp` dispatches each incoming
    /// `tools/call` request as its own `tokio::spawn`'d task running
    /// against a cloned `SuperBrainMcpServer` (confirmed by reading
    /// `rmcp::service`'s own request-dispatch loop: `shared_service =
    /// Arc::new(service)`, then `service.clone()` per request), so a
    /// semaphore that were re-created on every clone would impose no
    /// real limit at all — every concurrent call would get its own,
    /// always-available permit pool of 1. One `Arc<Semaphore>`,
    /// constructed once in [`Self::new`] and carried by reference
    /// through every clone, is the only shape that actually bounds
    /// concurrency across the whole server instance. See
    /// [`Self::acquire_concurrency_permit`] for where it's acquired.
    concurrency_limit: Arc<tokio::sync::Semaphore>,
    /// The full, fixed tool surface (B5.2 Task 24): Task 22's `read_tools`
    /// plus Task 23's `graph_tools`/`memory_tools`/`knowledge_tools`,
    /// combined once at construction time via `ToolRouter`'s own `+`
    /// (`std::ops::Add`), never rebuilt per-request and never mutated after
    /// construction. This is the single source `#[tool_handler(router =
    /// self.tool_router)]` dispatches every real MCP `tools/call`/
    /// `tools/list` request against — the exact same 12 tools this crate's
    /// own `tests/*.rs` files already exercise by calling the `#[tool]`
    /// methods directly, now also reachable over a real transport.
    pub(crate) tool_router: ToolRouter<Self>,
}

impl SuperBrainMcpServer {
    /// Fail-fast construction: every dependency must already be built and
    /// supplied by the caller (this crate's composition root — a real
    /// binary launcher is a separate, future concern; today's caller is
    /// this crate's own tests, injecting real fixtures). There is no
    /// default/empty fallback: a caller with no real `Vault`/`RealIndexer`/
    /// `CodeGraph` yet simply cannot construct a `SuperBrainMcpServer` at
    /// all, rather than silently getting one that always returns empty
    /// results while looking like it works.
    pub fn new(vault: storage::Vault, indexer: Arc<RealIndexer>, graph: Arc<CodeGraph>, active_project: impl Into<String>) -> Self {
        let tool_router = Self::read_tools() + Self::graph_tools() + Self::memory_tools() + Self::knowledge_tools();
        Self {
            vault: Arc::new(vault),
            indexer,
            graph,
            active_project: active_project.into(),
            tool_router,
            dense_scorer: None,
            audit: Arc::new(brain_api::NullAuditSink),
            concurrency_limit: Arc::new(tokio::sync::Semaphore::new(brain_api::limits::MAX_CONCURRENT_REQUESTS)),
        }
    }

    /// Acquires one of this server's [`brain_api::limits::MAX_CONCURRENT_REQUESTS`]
    /// permits, waiting (queueing) if none are free rather than rejecting
    /// outright (the plan's own "queue or reject" choice, resolved to
    /// queue — see [`Self::concurrency_limit`]'s own doc comment). Every
    /// `#[tool]` handler calls this as its very first line and holds the
    /// returned guard for its entire body (bound to a local variable,
    /// never discarded with `let _ =`) so the permit is held across
    /// every `.await` point the handler itself makes, released
    /// automatically (RAII, `Drop`) the moment the handler returns —
    /// on success, on an error return, or if the handler's own task is
    /// cancelled/dropped while still waiting.
    ///
    /// `Semaphore::acquire_owned` on an `Arc<Semaphore>` only ever errors
    /// if the semaphore itself has been closed (`Semaphore::close`,
    /// never called anywhere in this codebase) — this server never
    /// closes its own semaphore, so `.expect(...)` here can only panic
    /// if that invariant is later violated, never under ordinary
    /// concurrent load.
    pub(crate) async fn acquire_concurrency_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.concurrency_limit.clone().acquire_owned().await.expect("this server's own concurrency semaphore is never closed")
    }

    /// Opts this server instance into a real [`brain_api::AuditSink`]
    /// destination for every tool call's audit events (B5.12 Task 33).
    /// `sink` must already be built and ready to receive events — this
    /// method never constructs, persists, or owns the destination itself,
    /// the same "caller supplies it already-built" rule [`Self::new`]'s own
    /// doc comment states for `indexer`/`graph`/[`Self::with_dense_retrieval`]'s
    /// `scorer`.
    pub fn with_audit_sink(mut self, sink: Arc<dyn brain_api::AuditSink>) -> Self {
        self.audit = sink;
        self
    }

    pub(crate) fn audit_sink(&self) -> &dyn brain_api::AuditSink {
        self.audit.as_ref()
    }

    /// Records exactly one `ToolCall` event for `tool`, plus (only on a
    /// `PermissionDenied` result) one `PermissionDecision`+`DeniedAction`
    /// pair — every MCP tool wrapper in this crate calls this immediately
    /// after its own `brain_api::*` call, before mapping the result onto
    /// an MCP response. `now`/`ctx.correlation_id` tie every event from one
    /// call together and back to the originating `RequestContext`.
    pub(crate) fn record_tool_call<T>(&self, tool: &str, result: &Result<T, brain_api::BrainApiError>, ctx: &brain_api::RequestContext) {
        let now = chrono::Utc::now().to_rfc3339();
        let allowed = brain_api::allowed_from_result(result);
        brain_api::record_tool_call(self.audit_sink(), tool, allowed, &now, ctx.correlation_id);
        if !allowed {
            brain_api::record_permission_decision(self.audit_sink(), tool, false, Some("permission denied"), &now, ctx.correlation_id);
        }
    }

    /// Same as [`Self::record_tool_call`], plus a `WriteMemory` event
    /// carrying `proposal_id` on success — for the 3 proposal-creating
    /// tools (`save_memory`/`update_memory`/`link_entities`), never for a
    /// read capability (which has no proposal id to record).
    pub(crate) fn record_write_tool_call(&self, tool: &str, result: &Result<brain_api::ProposalId, brain_api::BrainApiError>, ctx: &brain_api::RequestContext) {
        self.record_tool_call(tool, result, ctx);
        if let Ok(proposal_id) = result {
            let now = chrono::Utc::now().to_rfc3339();
            brain_api::record_write_memory(self.audit_sink(), &proposal_id.to_string(), &now, ctx.correlation_id);
        }
    }

    /// Opts this server instance into `RetrievalMode::DenseV2` for
    /// `search`/`get_previous_solution`, using `scorer` as the real
    /// `DenseScorer` implementation. `scorer` must already be built and
    /// current (the same "caller supplies it already-built" rule
    /// `Self::new`'s own doc comment states for `indexer`/`graph`) —
    /// this method never rebuilds, refreshes, or validates it.
    pub fn with_dense_retrieval(mut self, scorer: Arc<dyn index::search::DenseScorer>) -> Self {
        self.dense_scorer = Some(scorer);
        self
    }

    pub(crate) fn mcp_ctx(&self) -> RequestContext {
        RequestContext {
            schema_version: brain_api::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: Some(WritePolicy::mcp_agent()),
            read_policy: Some(
                policy::ReadPolicy::new(true, true)
                    .with_get_code_context(true)
                    .with_graph_query(true)
                    .with_get_entity(true)
                    .with_get_previous_solution(true)
                    .with_get_rules(true)
                    .with_get_regulations(true)
                    .with_get_canon(true),
            ),
            origin: CallOrigin::McpSession(MCP_SESSION_PLACEHOLDER.to_string()),
        }
    }

    pub(crate) fn index(&self, reference_date: chrono::NaiveDate) -> index::Index<'_> {
        let idx = index::Index::new(&self.vault, &self.indexer, &self.active_project, reference_date);
        match &self.dense_scorer {
            Some(scorer) => idx.with_dense_scorer(scorer.as_ref()).with_retrieval_mode(index::search::RetrievalMode::DenseV2),
            None => idx,
        }
    }

    /// Read-only accessor for tests to assert on the injected `Vault`'s
    /// real on-disk state independently of any tool call (e.g. "this tool
    /// never wrote anything") — never used by this crate's own tool
    /// handlers, which always go through `brain_api`, never direct
    /// `Vault` reads/writes of their own.
    pub fn vault_ref(&self) -> &storage::Vault {
        &self.vault
    }

    /// Read-only accessor for this server's injected `CodeGraph`, used by
    /// `tools/graph.rs`'s `graph_query`/`get_entity` wrappers (the same
    /// already-built-and-injected dependency `get_code_context` above
    /// already uses via `self.graph`) — never built, rebuilt, or persisted
    /// by this crate itself.
    pub(crate) fn graph_ref(&self) -> &codegraph_index::graph::CodeGraph {
        &self.graph
    }
}

// --- search ---

/// The two low-complexity, closed-enum filters this MCP tool exposes.
/// `trust_filter`/`date_filter` are deliberately NOT exposed here:
/// `index::search::TimeExpression` is a 7-variant grammar with resolution
/// logic this task's "thin wrapper" scope does not invent an MCP-facing
/// encoding for — a documented, deliberate omission, not a silent drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum McpNoteType {
    Code,
    Bug,
    Decision,
    Pattern,
    Research,
    Session,
    Summary,
}

impl From<McpNoteType> for domain::NoteType {
    fn from(value: McpNoteType) -> Self {
        match value {
            McpNoteType::Code => domain::NoteType::Code,
            McpNoteType::Bug => domain::NoteType::Bug,
            McpNoteType::Decision => domain::NoteType::Decision,
            McpNoteType::Pattern => domain::NoteType::Pattern,
            McpNoteType::Research => domain::NoteType::Research,
            McpNoteType::Session => domain::NoteType::Session,
            McpNoteType::Summary => domain::NoteType::Summary,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum McpNoteStatus {
    Draft,
    TestPassed,
    Reviewed,
    Verified,
    Superseded,
    Contradicted,
}

impl From<McpNoteStatus> for domain::NoteStatus {
    fn from(value: McpNoteStatus) -> Self {
        match value {
            McpNoteStatus::Draft => domain::NoteStatus::Draft,
            McpNoteStatus::TestPassed => domain::NoteStatus::TestPassed,
            McpNoteStatus::Reviewed => domain::NoteStatus::Reviewed,
            McpNoteStatus::Verified => domain::NoteStatus::Verified,
            McpNoteStatus::Superseded => domain::NoteStatus::Superseded,
            McpNoteStatus::Contradicted => domain::NoteStatus::Contradicted,
        }
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SearchInput {
    pub query: String,
    /// Maximum number of results. Must be a positive integer; `0` is
    /// rejected as `InvalidInput`, never silently treated as "unlimited"
    /// or "zero results, no error" (`index::search`'s own
    /// `SearchError::InvalidLimit`).
    pub k: usize,
    #[serde(default)]
    pub type_filter: Option<McpNoteType>,
    #[serde(default)]
    pub status_filter: Option<McpNoteStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct SearchResultDto {
    pub note_id: String,
    pub version: u64,
    pub excerpt: String,
    pub score: f64,
    pub match_kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct SearchOutput {
    pub results: Vec<SearchResultDto>,
}

fn match_kind_str(kind: index::search::MatchKind) -> String {
    match kind {
        index::search::MatchKind::ExactSubstring => "exact-substring".to_string(),
        index::search::MatchKind::SemanticMatch => "semantic-match".to_string(),
        other => format!("{other:?}"),
    }
}

// --- retrieve ---

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RetrieveInput {
    pub note_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct RetrieveOutput {
    pub note_id: String,
    pub content: String,
    pub version: u64,
    pub status: String,
}

// --- get_code_context ---

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GetCodeContextInput {
    /// The symbol's canonical 64-character lowercase-hex
    /// `SymbolSnapshotId` (see `codegraph_domain::SymbolSnapshotId`'s own
    /// `Display`/`FromStr` doc comment — a Task 22 prerequisite added for
    /// exactly this round-trip). Never a raw byte array over the wire.
    pub symbol_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct CodeContextEdgeDto {
    pub other: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct GetCodeContextOutput {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub signature: String,
    pub relative_path: String,
    pub span_start: usize,
    pub span_end: usize,
    pub outgoing: Vec<CodeContextEdgeDto>,
    pub incoming: Vec<CodeContextEdgeDto>,
}

fn edge_dto(edge: brain_api::CodeContextEdge) -> CodeContextEdgeDto {
    CodeContextEdgeDto { other: edge.other.to_string(), kind: format!("{:?}", edge.kind) }
}

#[rmcp::tool_router(router = read_tools, vis = "pub")]
impl SuperBrainMcpServer {
    /// `search`: delegates to `brain_api::search`, which itself delegates
    /// to `api::search` unchanged. Deterministic ordering across repeated
    /// identical calls is `index::search`'s own guarantee, inherited
    /// through both layers without this wrapper re-sorting anything.
    #[rmcp::tool(description = "Search notes by text, optionally filtered by type/status.")]
    pub async fn search(&self, Parameters(input): Parameters<SearchInput>) -> Result<Json<SearchOutput>, McpError> {
        let _permit = self.acquire_concurrency_permit().await;
        let filters = index::search::SearchFilters {
            type_filter: input.type_filter.map(Into::into),
            status_filter: input.status_filter.map(Into::into),
            ..Default::default()
        };
        let request = api::SearchRequest { text: input.query, filters, k: input.k };
        let reference_date = chrono::Utc::now().date_naive();
        let idx = self.index(reference_date);
        let ctx = self.mcp_ctx();
        let result = brain_api::search(&ctx, &idx, request);
        self.record_tool_call("search", &result, &ctx);
        let response = result.map_err(map_brain_api_error)?;

        let results: Vec<SearchResultDto> = match response.outcome {
            index::search::SearchOutcome::NotPresent => Vec::new(),
            index::search::SearchOutcome::Matches { first, rest } => std::iter::once(first)
                .chain(rest)
                .map(|hit| SearchResultDto {
                    note_id: hit.note_id.as_str().to_string(),
                    version: hit.version.0,
                    excerpt: hit.excerpt,
                    score: hit.score,
                    match_kind: match_kind_str(hit.match_kind),
                })
                .collect(),
        };
        Ok(Json(SearchOutput { results }))
    }

    /// `retrieve`: delegates to `brain_api::retrieve`. An invalid/
    /// malformed `note_id` string is rejected as `InvalidInput` by
    /// `storage::NoteId::parse` before `brain_api::retrieve` is ever
    /// called — never silently treated as "not found".
    #[rmcp::tool(description = "Retrieve one note's full content by id.")]
    pub async fn retrieve(&self, Parameters(input): Parameters<RetrieveInput>) -> Result<Json<RetrieveOutput>, McpError> {
        let _permit = self.acquire_concurrency_permit().await;
        let note_id = storage::NoteId::parse(&input.note_id).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let request = api::RetrieveRequest { id: api::RetrieveTarget::Note(note_id) };
        let ctx = self.mcp_ctx();
        let result = brain_api::retrieve(&ctx, &self.vault, request);
        self.record_tool_call("retrieve", &result, &ctx);
        let response = result.map_err(map_brain_api_error)?;
        Ok(Json(RetrieveOutput {
            note_id: response.id.as_str().to_string(),
            content: response.content,
            version: response.version.0,
            status: format!("{:?}", response.status),
        }))
    }

    /// `get_code_context`: delegates to `brain_api::get_code_context`
    /// against this server's already-injected `CodeGraph` — never a
    /// `CodeGraph` this wrapper builds itself. A `symbol_id` that is not
    /// valid 64-character hex is rejected as `InvalidInput` before
    /// `brain_api::get_code_context` is ever called; a well-formed id that
    /// names no real snapshot in `self.graph` still reaches `brain_api`'s
    /// own `NotFound`, exactly as it would for a direct `brain_api` caller.
    #[rmcp::tool(description = "Get one code symbol's intrinsic facts and immediate graph neighbors.")]
    pub async fn get_code_context(&self, Parameters(input): Parameters<GetCodeContextInput>) -> Result<Json<GetCodeContextOutput>, McpError> {
        let _permit = self.acquire_concurrency_permit().await;
        let symbol_id: codegraph_domain::SymbolSnapshotId =
            input.symbol_id.parse().map_err(|e: codegraph_domain::SymbolSnapshotIdParseError| McpError::invalid_params(e.to_string(), None))?;
        let ctx = self.mcp_ctx();
        let result = brain_api::get_code_context(&ctx, &self.graph, symbol_id);
        self.record_tool_call("get_code_context", &result, &ctx);
        let context = result.map_err(map_brain_api_error)?;
        Ok(Json(GetCodeContextOutput {
            id: context.id.to_string(),
            kind: format!("{:?}", context.kind),
            name: context.name,
            signature: context.signature,
            relative_path: context.relative_path,
            span_start: context.span_start,
            span_end: context.span_end,
            outgoing: context.outgoing.into_iter().map(edge_dto).collect(),
            incoming: context.incoming.into_iter().map(edge_dto).collect(),
        }))
    }
}

#[cfg(test)]
mod concurrency_limit_tests {
    //! B5.16 Task 35: direct, white-box tests of
    //! [`SuperBrainMcpServer::acquire_concurrency_permit`] and its
    //! backing `Arc<Semaphore>` — `pub(crate)`, so only reachable from
    //! inside this crate (this module), never from the external
    //! integration tests in `tests/*.rs`, which instead prove the same
    //! mechanism's REAL, end-to-end effect over the wire
    //! (`tests/limits.rs`). Both are needed: this module proves the
    //! mechanism's own shape (shared across clones, bounded exactly at
    //! the configured limit, cancel-safe); `tests/limits.rs` proves it
    //! is actually wired into every real `#[tool]` handler.

    use super::*;

    fn unique_test_dir(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-mcp-server-concurrency-{label}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn new_server() -> SuperBrainMcpServer {
        let root = unique_test_dir("fixture");
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();
        SuperBrainMcpServer::new(vault, Arc::new(RealIndexer::new()), Arc::new(CodeGraph::new()), "project-x")
    }

    #[tokio::test]
    async fn exactly_max_concurrent_requests_permits_can_be_held_at_once() {
        let server = new_server();
        let mut held = Vec::with_capacity(brain_api::limits::MAX_CONCURRENT_REQUESTS);
        for _ in 0..brain_api::limits::MAX_CONCURRENT_REQUESTS {
            held.push(server.acquire_concurrency_permit().await);
        }
        assert_eq!(held.len(), brain_api::limits::MAX_CONCURRENT_REQUESTS);
    }

    #[tokio::test]
    async fn a_permit_beyond_the_limit_queues_until_one_is_released_never_exceeding_the_cap() {
        let server = new_server();
        let mut held = Vec::with_capacity(brain_api::limits::MAX_CONCURRENT_REQUESTS);
        for _ in 0..brain_api::limits::MAX_CONCURRENT_REQUESTS {
            held.push(server.acquire_concurrency_permit().await);
        }

        // One more acquire, at the cap: must NOT resolve yet.
        let extra_server = server.clone();
        let extra = tokio::spawn(async move { extra_server.acquire_concurrency_permit().await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!extra.is_finished(), "an acquire beyond the configured cap must queue, not succeed immediately");

        // Releasing exactly one permit must let the queued acquire
        // proceed -- never granting a permit beyond the configured cap
        // (there is no way for two permits to be "free" at once here:
        // releasing one and having the queued acquire succeed is itself
        // the proof the cap was never exceeded).
        held.pop();
        let released_permit = tokio::time::timeout(std::time::Duration::from_secs(5), extra).await.expect("the queued acquire must resume once a permit frees up").expect("the spawned task must not panic");
        drop(released_permit);
        drop(held);
    }

    #[tokio::test]
    async fn every_clone_of_the_server_shares_the_same_permit_pool_not_an_independent_one() {
        let server = new_server();
        let clone = server.clone();

        let mut held = Vec::with_capacity(brain_api::limits::MAX_CONCURRENT_REQUESTS);
        // Acquire half through the original, half through the clone --
        // if each clone had its OWN semaphore, this would never
        // saturate either one; because they share one Arc<Semaphore>,
        // the combined total still hits the single real cap below.
        for i in 0..brain_api::limits::MAX_CONCURRENT_REQUESTS {
            if i % 2 == 0 {
                held.push(server.acquire_concurrency_permit().await);
            } else {
                held.push(clone.acquire_concurrency_permit().await);
            }
        }

        let one_more_server = server.clone();
        let one_more = tokio::spawn(async move { one_more_server.acquire_concurrency_permit().await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!one_more.is_finished(), "clones must share one real permit pool -- the combined total from both handles must still be capped");

        held.pop();
        let permit = tokio::time::timeout(std::time::Duration::from_secs(5), one_more).await.expect("shared pool: releasing one permit from either handle must free a slot for the other").expect("spawned task must not panic");
        drop(permit);
        drop(held);
    }

    #[tokio::test]
    async fn dropping_a_task_still_waiting_for_a_permit_never_consumes_one() {
        let server = new_server();
        let mut held = Vec::with_capacity(brain_api::limits::MAX_CONCURRENT_REQUESTS);
        for _ in 0..brain_api::limits::MAX_CONCURRENT_REQUESTS {
            held.push(server.acquire_concurrency_permit().await);
        }

        // A queued acquire, then abort the task waiting on it before it
        // ever resolves -- tokio::sync::Semaphore's own documented
        // cancel-safety: a dropped Acquire future is removed from the
        // wait queue without taking a permit.
        let waiting_server = server.clone();
        let waiting = tokio::spawn(async move { waiting_server.acquire_concurrency_permit().await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        waiting.abort();
        let _ = waiting.await;

        // Release one permit. If the aborted waiter had somehow already
        // consumed it, this fresh acquire would now queue instead of
        // resolving immediately.
        held.pop();
        let fresh = tokio::time::timeout(std::time::Duration::from_millis(500), server.acquire_concurrency_permit()).await;
        assert!(fresh.is_ok(), "a permit released after an aborted waiter must be available to a fresh acquire, not silently consumed by the cancelled task");
    }
}
