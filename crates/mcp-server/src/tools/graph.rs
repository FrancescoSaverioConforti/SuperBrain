//! B5.2 Task 23 (graph half): `graph_query`/`get_entity`/`link_entities` as
//! MCP tools — thin schema-mapping wrappers over `brain_api::{graph_query,
//! get_entity, link_entities}` (Tasks 13/17). Same trust-boundary and
//! thin-wrapper discipline as `tools/search.rs` (Task 22): every
//! `RequestContext` is built server-side via `SuperBrainMcpServer::mcp_ctx`,
//! never trusting anything from the incoming MCP request to set
//! `write_policy`/`read_policy`/`origin`.
//!
//! `link_entities` is write-shaped: it returns a `ProposalId`, never a
//! committed result. This file contains no path to `brain_api::
//! approve_proposal`/`reject_proposal` — those are not, and must never be,
//! registered as MCP tools (see `tests/mcp_tools_task23.rs`).

use rmcp::{
    ErrorData as McpError,
    handler::server::wrapper::{Json, Parameters},
    schemars,
};
use serde::{Deserialize, Serialize};

use super::search::SuperBrainMcpServer;
use crate::error::map_brain_api_error;

// --- graph_query ---

/// Mirrors `codegraph_domain::EdgeKind`'s real 8 variants over the wire —
/// never a free-form string `brain_api::graph_query` would have to parse
/// loosely. Note `Imports` is deliberately absent: it was removed from
/// `EdgeKind` itself (Task 9.5, B2-ID-005 — see `codegraph_domain::kind::
/// EdgeKind`'s own doc comment), so there is no such variant to mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum McpEdgeKind {
    Calls,
    Implements,
    Extends,
    References,
    TestsFor,
    RenamedTo,
    Contains,
    Instantiates,
}

impl From<McpEdgeKind> for codegraph_domain::EdgeKind {
    fn from(value: McpEdgeKind) -> Self {
        match value {
            McpEdgeKind::Calls => codegraph_domain::EdgeKind::Calls,
            McpEdgeKind::Implements => codegraph_domain::EdgeKind::Implements,
            McpEdgeKind::Extends => codegraph_domain::EdgeKind::Extends,
            McpEdgeKind::References => codegraph_domain::EdgeKind::References,
            McpEdgeKind::TestsFor => codegraph_domain::EdgeKind::TestsFor,
            McpEdgeKind::RenamedTo => codegraph_domain::EdgeKind::RenamedTo,
            McpEdgeKind::Contains => codegraph_domain::EdgeKind::Contains,
            McpEdgeKind::Instantiates => codegraph_domain::EdgeKind::Instantiates,
        }
    }
}

impl From<codegraph_domain::EdgeKind> for McpEdgeKind {
    fn from(value: codegraph_domain::EdgeKind) -> Self {
        match value {
            codegraph_domain::EdgeKind::Calls => McpEdgeKind::Calls,
            codegraph_domain::EdgeKind::Implements => McpEdgeKind::Implements,
            codegraph_domain::EdgeKind::Extends => McpEdgeKind::Extends,
            codegraph_domain::EdgeKind::References => McpEdgeKind::References,
            codegraph_domain::EdgeKind::TestsFor => McpEdgeKind::TestsFor,
            codegraph_domain::EdgeKind::RenamedTo => McpEdgeKind::RenamedTo,
            codegraph_domain::EdgeKind::Contains => McpEdgeKind::Contains,
            codegraph_domain::EdgeKind::Instantiates => McpEdgeKind::Instantiates,
        }
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GraphQueryInput {
    /// The symbol's canonical 64-character lowercase-hex `SymbolSnapshotId`
    /// to start the traversal from.
    pub start_symbol_id: String,
    #[serde(default)]
    pub kind: Option<McpEdgeKind>,
    pub max_depth: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct GraphQueryOutput {
    pub symbol_ids: Vec<String>,
}

// --- get_entity ---

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GetEntityInput {
    pub symbol_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct GetEntityOutput {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub signature: String,
    pub relative_path: String,
    pub span_start: usize,
    pub span_end: usize,
}

// --- link_entities ---

/// The five real `domain::Relations` fields
/// (`crates/domain/src/note.rs:13-24`) — never an arbitrary string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum McpRelationKind {
    Supports,
    Contradicts,
    DerivedFrom,
    Supersedes,
    AppliesTo,
}

impl From<McpRelationKind> for brain_api::RelationKind {
    fn from(value: McpRelationKind) -> Self {
        match value {
            McpRelationKind::Supports => brain_api::RelationKind::Supports,
            McpRelationKind::Contradicts => brain_api::RelationKind::Contradicts,
            McpRelationKind::DerivedFrom => brain_api::RelationKind::DerivedFrom,
            McpRelationKind::Supersedes => brain_api::RelationKind::Supersedes,
            McpRelationKind::AppliesTo => brain_api::RelationKind::AppliesTo,
        }
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct LinkEntitiesInput {
    pub from_note_id: String,
    pub to_note_id: String,
    pub relation: McpRelationKind,
}

/// Always a `ProposalId`, never a committed outcome — `link_entities` only
/// ever creates a `Pending` proposal (see this file's module doc comment).
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct LinkEntitiesOutput {
    pub proposal_id: String,
}

#[rmcp::tool_router(router = graph_tools, vis = "pub")]
impl SuperBrainMcpServer {
    /// `graph_query`: delegates to `brain_api::graph_query` against this
    /// server's already-injected `CodeGraph`. An invalid `start_symbol_id`
    /// is rejected as `InvalidInput` before `brain_api::graph_query` is
    /// ever called; `max_depth`/`limit` bound-checking is `brain_api`'s own
    /// responsibility, never duplicated here.
    #[rmcp::tool(description = "Run a bounded graph traversal from one code symbol, following edges of an optional kind, up to max_depth hops.")]
    pub async fn graph_query(&self, Parameters(input): Parameters<GraphQueryInput>, cancellation_token: tokio_util::sync::CancellationToken) -> Result<Json<GraphQueryOutput>, McpError> {
        let Ok(_permit) = self.acquire_concurrency_permit(&cancellation_token).await else {
            return Err(McpError::internal_error("request cancelled while waiting for a concurrency permit", None));
        };
        let start: codegraph_domain::SymbolSnapshotId =
            input.start_symbol_id.parse().map_err(|e: codegraph_domain::SymbolSnapshotIdParseError| McpError::invalid_params(e.to_string(), None))?;
        let spec = brain_api::GraphQuerySpec { start, kind: input.kind.map(Into::into), max_depth: input.max_depth };
        let ctx = self.mcp_ctx();
        let result = brain_api::graph_query(&ctx, self.graph_ref(), spec, input.limit);
        self.record_tool_call("graph_query", &result, &ctx);
        let reached = result.map_err(map_brain_api_error)?;
        Ok(Json(GraphQueryOutput { symbol_ids: reached.into_iter().map(|id| id.to_string()).collect() }))
    }

    /// `get_entity`: delegates to `brain_api::get_entity` against the
    /// injected `CodeGraph`. Returns intrinsic facts only — no traversal,
    /// no neighboring edges (that is `get_code_context`'s job, Task 22).
    #[rmcp::tool(description = "Get one code symbol's intrinsic facts only (no graph neighbors).")]
    pub async fn get_entity(&self, Parameters(input): Parameters<GetEntityInput>, cancellation_token: tokio_util::sync::CancellationToken) -> Result<Json<GetEntityOutput>, McpError> {
        let Ok(_permit) = self.acquire_concurrency_permit(&cancellation_token).await else {
            return Err(McpError::internal_error("request cancelled while waiting for a concurrency permit", None));
        };
        let symbol_id: codegraph_domain::SymbolSnapshotId =
            input.symbol_id.parse().map_err(|e: codegraph_domain::SymbolSnapshotIdParseError| McpError::invalid_params(e.to_string(), None))?;
        let ctx = self.mcp_ctx();
        let result = brain_api::get_entity(&ctx, self.graph_ref(), symbol_id);
        self.record_tool_call("get_entity", &result, &ctx);
        let entity = result.map_err(map_brain_api_error)?;
        Ok(Json(GetEntityOutput {
            id: entity.id.to_string(),
            kind: format!("{:?}", entity.kind),
            name: entity.name,
            signature: entity.signature,
            relative_path: entity.relative_path,
            span_start: entity.span_start,
            span_end: entity.span_end,
        }))
    }

    /// `link_entities`: delegates to `brain_api::link_entities`, which
    /// never executes a link directly — it creates a `Pending`
    /// `PendingWriteProposal` and returns its id. This tool's
    /// `RequestContext` always carries `WritePolicy::mcp_agent()`
    /// (`ApproveProposal`/`RejectProposal` denied), so the proposal this
    /// tool creates can never be self-approved through MCP.
    #[rmcp::tool(description = "Propose linking two notes by a relation kind. Creates a pending proposal; does not execute the link.")]
    pub async fn link_entities(&self, Parameters(input): Parameters<LinkEntitiesInput>, cancellation_token: tokio_util::sync::CancellationToken) -> Result<Json<LinkEntitiesOutput>, McpError> {
        let Ok(_permit) = self.acquire_concurrency_permit(&cancellation_token).await else {
            return Err(McpError::internal_error("request cancelled while waiting for a concurrency permit", None));
        };
        let from = storage::NoteId::parse(&input.from_note_id).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let to = storage::NoteId::parse(&input.to_note_id).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let now = chrono::Utc::now().to_rfc3339();
        let ctx = self.mcp_ctx();
        let result = brain_api::link_entities(&ctx, self.vault_ref(), from, to, input.relation.into(), &now);
        self.record_write_tool_call("link_entities", &result, &ctx);
        let proposal_id = result.map_err(map_brain_api_error)?;
        Ok(Json(LinkEntitiesOutput { proposal_id: proposal_id.to_string() }))
    }
}
