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
    handler::server::wrapper::{Json, Parameters},
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
        Self { vault: Arc::new(vault), indexer, graph, active_project: active_project.into() }
    }

    fn mcp_ctx(&self) -> RequestContext {
        RequestContext {
            schema_version: brain_api::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: Some(WritePolicy::mcp_agent()),
            read_policy: Some(policy::ReadPolicy::new(true, true).with_get_code_context(true)),
            origin: CallOrigin::McpSession(MCP_SESSION_PLACEHOLDER.to_string()),
        }
    }

    fn index(&self, reference_date: chrono::NaiveDate) -> index::Index<'_> {
        index::Index::new(&self.vault, &self.indexer, &self.active_project, reference_date)
    }

    /// Read-only accessor for tests to assert on the injected `Vault`'s
    /// real on-disk state independently of any tool call (e.g. "this tool
    /// never wrote anything") — never used by this crate's own tool
    /// handlers, which always go through `brain_api`, never direct
    /// `Vault` reads/writes of their own.
    pub fn vault_ref(&self) -> &storage::Vault {
        &self.vault
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
        let filters = index::search::SearchFilters {
            type_filter: input.type_filter.map(Into::into),
            status_filter: input.status_filter.map(Into::into),
            ..Default::default()
        };
        let request = api::SearchRequest { text: input.query, filters, k: input.k };
        let reference_date = chrono::Utc::now().date_naive();
        let idx = self.index(reference_date);
        let response = brain_api::search(&self.mcp_ctx(), &idx, request).map_err(map_brain_api_error)?;

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
        let note_id = storage::NoteId::parse(&input.note_id).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let request = api::RetrieveRequest { id: api::RetrieveTarget::Note(note_id) };
        let response = brain_api::retrieve(&self.mcp_ctx(), &self.vault, request).map_err(map_brain_api_error)?;
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
        let symbol_id: codegraph_domain::SymbolSnapshotId =
            input.symbol_id.parse().map_err(|e: codegraph_domain::SymbolSnapshotIdParseError| McpError::invalid_params(e.to_string(), None))?;
        let context = brain_api::get_code_context(&self.mcp_ctx(), &self.graph, symbol_id).map_err(map_brain_api_error)?;
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
