//! SuperBrain MCP server (B5.2+).
//!
//! Exposes `brain-api`'s capabilities over the Model Context Protocol,
//! using the official Rust SDK (`rmcp`, `modelcontextprotocol/rust-sdk`)
//! with its stdio server transport (`transport-io` feature) — stdin/stdout
//! only, no network, the standard way to launch a local MCP server as a
//! child process, matching this project's offline-safe, local-single-user
//! design.
//!
//! **B5.1 Task 20's trust-boundary invariants apply unchanged to every
//! tool this crate registers** (Task 22+): this crate depends on
//! `brain-api`, never the reverse, and has no access to anything
//! `brain-api` itself keeps `pub(crate)` — there is no
//! `brain_api::load_proposal`/`save_proposal` to call, no
//! `WritePolicy::trusted_cli()` to construct, and no way to produce a
//! `CallOrigin::TrustedLocal` (its only constructor,
//! `TrustedLocalToken::new`, is `pub(crate)` to `brain-api` and therefore
//! unreachable from here). Every `RequestContext` this crate builds for an
//! incoming MCP request uses `CallOrigin::McpSession` and
//! `WritePolicy::mcp_agent()` — the only two values of each this crate CAN
//! construct — never derived from any field inside the MCP request payload
//! itself (same anti-self-assertion principle `brain-api`'s own module
//! docs already state for `originating_identity`).

mod error;
mod tools;

pub use error::map_brain_api_error;
pub use tools::graph::{
    GetEntityInput, GetEntityOutput, GraphQueryInput, GraphQueryOutput, LinkEntitiesInput, LinkEntitiesOutput, McpEdgeKind, McpRelationKind,
};
pub use tools::knowledge::{
    CanonEntryDto, GetCanonInput, GetCanonOutput, GetRegulationsInput, GetRegulationsOutput, GetRulesInput, GetRulesOutput, KnowledgeItemDto,
};
pub use tools::memory::{
    GetPreviousSolutionInput, GetPreviousSolutionOutput, McpClassification, PreviousSolutionDto, SaveMemoryInput, SaveMemoryOutput,
    UpdateMemoryInput, UpdateMemoryOutput,
};
pub use tools::search::{
    CodeContextEdgeDto, GetCodeContextInput, GetCodeContextOutput, McpNoteStatus, McpNoteType, RetrieveInput, RetrieveOutput, SearchInput,
    SearchOutput, SearchResultDto, SuperBrainMcpServer,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {}
}
