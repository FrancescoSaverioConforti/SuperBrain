//! B5.2 Task 23 (knowledge half): `get_rules`/`get_regulations`/
//! `get_canon` as MCP tools — thin schema-mapping wrappers over
//! `brain_api::{get_rules, get_regulations, get_canon}` (Task 19). Same
//! trust-boundary discipline as `tools/search.rs` (Task 22): every
//! `RequestContext` is built server-side via `SuperBrainMcpServer::mcp_ctx`.
//! All three are pure reads — none of them creates a proposal or touches
//! `storage::propose_and_commit`.

use rmcp::{
    ErrorData as McpError,
    handler::server::wrapper::{Json, Parameters},
    schemars,
};
use serde::{Deserialize, Serialize};

use super::search::SuperBrainMcpServer;
use crate::error::map_brain_api_error;

fn status_str(status: brain_api::BindingStatus) -> String {
    match status {
        brain_api::BindingStatus::Binding => "binding".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct KnowledgeItemDto {
    pub content_or_reference: String,
    pub source: String,
    pub scope: String,
    pub provenance: String,
    pub status: String,
    pub version_or_date: String,
}

impl From<brain_api::KnowledgeItem> for KnowledgeItemDto {
    fn from(item: brain_api::KnowledgeItem) -> Self {
        Self {
            content_or_reference: item.content_or_reference,
            source: item.source,
            scope: item.scope,
            provenance: item.provenance,
            status: status_str(item.status),
            version_or_date: item.version_or_date,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct CanonEntryDto {
    pub item: KnowledgeItemDto,
    pub canonized_by: String,
    pub canonized_at: String,
}

impl From<brain_api::CanonEntry> for CanonEntryDto {
    fn from(entry: brain_api::CanonEntry) -> Self {
        Self { item: entry.item.into(), canonized_by: entry.canonized_by, canonized_at: entry.canonized_at }
    }
}

// --- get_rules ---

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GetRulesInput {
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct GetRulesOutput {
    pub items: Vec<KnowledgeItemDto>,
}

// --- get_regulations ---

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GetRegulationsInput {
    #[serde(default)]
    pub regime: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct GetRegulationsOutput {
    pub items: Vec<KnowledgeItemDto>,
}

// --- get_canon ---

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GetCanonInput {
    #[serde(default)]
    pub topic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct GetCanonOutput {
    pub entries: Vec<CanonEntryDto>,
}

#[rmcp::tool_router(router = knowledge_tools, vis = "pub")]
impl SuperBrainMcpServer {
    /// `get_rules`: delegates to `brain_api::get_rules`. An empty (but
    /// present) `scope` string is rejected as `InvalidInput`; omitting
    /// `scope` entirely returns rules across every known scope.
    #[rmcp::tool(description = "List the project's internal operating/engineering rules, optionally filtered by scope.")]
    pub async fn get_rules(&self, Parameters(input): Parameters<GetRulesInput>) -> Result<Json<GetRulesOutput>, McpError> {
        let _permit = self.acquire_concurrency_permit().await;
        let ctx = self.mcp_ctx();
        let result = brain_api::get_rules(&ctx, input.scope.as_deref());
        self.record_tool_call("get_rules", &result, &ctx);
        let items = result.map_err(map_brain_api_error)?;
        Ok(Json(GetRulesOutput { items: items.into_iter().map(Into::into).collect() }))
    }

    /// `get_regulations`: delegates to `brain_api::get_regulations`. Same
    /// empty-filter discipline as `get_rules`; `regime` is a distinct
    /// namespace from `get_rules`'s `scope`, never conflated with it.
    #[rmcp::tool(description = "List applicable external regulations, optionally filtered by regime.")]
    pub async fn get_regulations(&self, Parameters(input): Parameters<GetRegulationsInput>) -> Result<Json<GetRegulationsOutput>, McpError> {
        let _permit = self.acquire_concurrency_permit().await;
        let ctx = self.mcp_ctx();
        let result = brain_api::get_regulations(&ctx, input.regime.as_deref());
        self.record_tool_call("get_regulations", &result, &ctx);
        let items = result.map_err(map_brain_api_error)?;
        Ok(Json(GetRegulationsOutput { items: items.into_iter().map(Into::into).collect() }))
    }

    /// `get_canon`: delegates to `brain_api::get_canon`. Always returns an
    /// empty list today — no canonization mechanism exists yet
    /// (`brain_api::get_canon`'s own doc comment) — but the permission
    /// check still runs before that empty answer is given.
    #[rmcp::tool(description = "List the authoritative corpus of consolidated project truths, optionally filtered by topic.")]
    pub async fn get_canon(&self, Parameters(input): Parameters<GetCanonInput>) -> Result<Json<GetCanonOutput>, McpError> {
        let _permit = self.acquire_concurrency_permit().await;
        let ctx = self.mcp_ctx();
        let result = brain_api::get_canon(&ctx, input.topic.as_deref());
        self.record_tool_call("get_canon", &result, &ctx);
        let entries = result.map_err(map_brain_api_error)?;
        Ok(Json(GetCanonOutput { entries: entries.into_iter().map(Into::into).collect() }))
    }
}
