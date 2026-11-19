//! B5.2 Task 23 (memory half): `get_previous_solution`/`save_memory`/
//! `update_memory` as MCP tools — thin schema-mapping wrappers over
//! `brain_api::{get_previous_solution, save_memory, update_memory}`
//! (Tasks 13/18). Same trust-boundary discipline as `tools/search.rs`
//! (Task 22) and `tools/graph.rs`: every `RequestContext` is built
//! server-side via `SuperBrainMcpServer::mcp_ctx`.
//!
//! `save_memory`/`update_memory` are write-shaped: each returns a
//! `ProposalId`, never a committed note. Approval/rejection
//! (`brain_api::approve_proposal`/`reject_proposal`) are not, and must
//! never be, registered as MCP tools (see `tests/mcp_tools_task23.rs`).

use rmcp::{
    ErrorData as McpError,
    handler::server::wrapper::{Json, Parameters},
    schemars,
};
use serde::{Deserialize, Serialize};

use super::search::{McpNoteType, SuperBrainMcpServer};
use crate::error::map_brain_api_error;

// --- get_previous_solution ---

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GetPreviousSolutionInput {
    pub query: String,
    #[serde(default)]
    pub note_type: Option<McpNoteType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct PreviousSolutionDto {
    pub note_id: String,
    pub note_type: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct GetPreviousSolutionOutput {
    pub results: Vec<PreviousSolutionDto>,
}

// --- save_memory ---

/// Mirrors `domain::Classification`'s real 3 variants over the wire (never
/// `Secret`, which is not a constructible variant in `domain` itself —
/// see `domain::schema::Classification`'s own doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum McpClassification {
    Public,
    Internal,
    Sensitive,
}

impl From<McpClassification> for domain::Classification {
    fn from(value: McpClassification) -> Self {
        match value {
            McpClassification::Public => domain::Classification::Public,
            McpClassification::Internal => domain::Classification::Internal,
            McpClassification::Sensitive => domain::Classification::Sensitive,
        }
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SaveMemoryInput {
    pub note_id: String,
    pub note_type: McpNoteType,
    pub classification: McpClassification,
    pub project: String,
    pub source: String,
    pub event_period: String,
    pub body: String,
}

/// Always a `ProposalId`, never a committed note — `save_memory` only ever
/// creates a `Pending` proposal (see this file's module doc comment).
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct SaveMemoryOutput {
    pub proposal_id: String,
}

// --- update_memory ---

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct UpdateMemoryInput {
    pub note_id: String,
    pub body: String,
    /// The optimistic-concurrency version the caller observed when
    /// proposing this update. `approve_proposal` re-checks it at approval
    /// time — never trusted as still-current by this tool.
    pub expected_version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct UpdateMemoryOutput {
    pub proposal_id: String,
}

#[rmcp::tool_router(router = memory_tools, vis = "pub")]
impl SuperBrainMcpServer {
    /// `get_previous_solution`: delegates to `brain_api::get_previous_solution`
    /// against this server's already-injected `Vault`/`Index`. An empty
    /// query is rejected as `InvalidInput` before `brain_api` is ever
    /// called; no match is `Ok(vec![])`, never an error.
    #[rmcp::tool(description = "Find previously-recorded bug/decision notes matching a query, for provenance tracing.")]
    pub async fn get_previous_solution(&self, Parameters(input): Parameters<GetPreviousSolutionInput>, cancellation_token: tokio_util::sync::CancellationToken) -> Result<Json<GetPreviousSolutionOutput>, McpError> {
        let Ok(_permit) = self.acquire_concurrency_permit(&cancellation_token).await else {
            return Err(McpError::internal_error("request cancelled while waiting for a concurrency permit", None));
        };
        let reference_date = chrono::Utc::now().date_naive();
        let idx = self.index(reference_date);
        let ctx = self.mcp_ctx();
        let result = brain_api::get_previous_solution(&ctx, self.vault_ref(), &idx, &input.query, input.note_type.map(Into::into));
        self.record_tool_call("get_previous_solution", &result, &ctx);
        let results = result.map_err(map_brain_api_error)?;
        Ok(Json(GetPreviousSolutionOutput {
            results: results
                .into_iter()
                .map(|meta| PreviousSolutionDto { note_id: meta.id, note_type: format!("{:?}", meta.r#type), status: format!("{:?}", meta.status) })
                .collect(),
        }))
    }

    /// `save_memory`: delegates to `brain_api::save_memory`, which never
    /// executes the save directly — it creates a `Pending`
    /// `PendingWriteProposal` and returns its id. This tool's
    /// `RequestContext` always carries `WritePolicy::mcp_agent()`
    /// (`ApproveProposal`/`RejectProposal` denied), so the proposal this
    /// tool creates can never be self-approved through MCP.
    #[rmcp::tool(description = "Propose saving new content as a fresh note. Creates a pending proposal; does not execute the save.")]
    pub async fn save_memory(&self, Parameters(input): Parameters<SaveMemoryInput>, cancellation_token: tokio_util::sync::CancellationToken) -> Result<Json<SaveMemoryOutput>, McpError> {
        let Ok(_permit) = self.acquire_concurrency_permit(&cancellation_token).await else {
            return Err(McpError::internal_error("request cancelled while waiting for a concurrency permit", None));
        };
        let note_id = storage::NoteId::parse(&input.note_id).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let now = chrono::Utc::now().to_rfc3339();
        let ctx = self.mcp_ctx();
        let result = brain_api::save_memory(
            &ctx,
            self.vault_ref(),
            note_id,
            input.note_type.into(),
            input.classification.into(),
            input.project,
            input.source,
            input.event_period,
            input.body,
            &now,
        );
        self.record_write_tool_call("save_memory", &result, &ctx);
        let proposal_id = result.map_err(map_brain_api_error)?;
        Ok(Json(SaveMemoryOutput { proposal_id: proposal_id.to_string() }))
    }

    /// `update_memory`: delegates to `brain_api::update_memory`, same
    /// proposal-only discipline as `save_memory`.
    #[rmcp::tool(description = "Propose replacing an existing note's content. Creates a pending proposal; does not execute the update.")]
    pub async fn update_memory(&self, Parameters(input): Parameters<UpdateMemoryInput>, cancellation_token: tokio_util::sync::CancellationToken) -> Result<Json<UpdateMemoryOutput>, McpError> {
        let Ok(_permit) = self.acquire_concurrency_permit(&cancellation_token).await else {
            return Err(McpError::internal_error("request cancelled while waiting for a concurrency permit", None));
        };
        let note_id = storage::NoteId::parse(&input.note_id).map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let now = chrono::Utc::now().to_rfc3339();
        let ctx = self.mcp_ctx();
        let result = brain_api::update_memory(&ctx, self.vault_ref(), note_id, input.body, storage::Version(input.expected_version), &now);
        self.record_write_tool_call("update_memory", &result, &ctx);
        let proposal_id = result.map_err(map_brain_api_error)?;
        Ok(Json(UpdateMemoryOutput { proposal_id: proposal_id.to_string() }))
    }
}
