//! SuperBrain MCP client (B5.8+).
//!
//! The reverse direction of `crates/mcp-server`: where `mcp-server`
//! exposes SuperBrain's own Brain API capabilities OUTWARD over MCP, this
//! crate lets SuperBrain call OUT to *external* MCP tools, under its own
//! permission gate (B5.9, Task 28 — not yet implemented by this task).
//!
//! **Trust boundary (per the design doc's B5.10 architecture test,
//! enforced structurally here, not just by convention): this crate has no
//! dependency on `brain-api` or `storage`.** It cannot reach
//! `storage::propose_and_commit`, cannot construct a `brain_api::
//! RequestContext`/`WritePolicy`, and has no loop-back path into
//! SuperBrain's own write pipeline — calling an external tool through this
//! crate is a dead end from SuperBrain's own data's perspective until some
//! future, separate, explicitly-gated capability (not part of this plan)
//! chooses to act on an external tool's result. `Cargo.toml`'s own
//! dependency list is the proof: `rmcp`, `tokio`, and `serde_json` only —
//! none of which is `storage`, `brain-api`, or `mcp-server`.
//!
//! B5.8 (Task 26): connection lifecycle and server registration —
//! `McpServerRegistration`, `connect()`, and the one global kill-switch
//! (`global_client_enabled()`).
//!
//! B5.9 (this task, Task 27): capability discovery and a tool metadata
//! cache (`discover_tools`, `ToolCache`, `DiscoveredTool`) — calls a
//! connected server's real `tools/list` and validates the reply before
//! trusting any of it. Still does not classify, authorize, or invoke
//! anything; `DiscoveredTool::classification` is an inert `Option<()>`
//! placeholder, always `None`, never read by this crate. Authorization
//! (Task 28) and invocation (Task 29) are later, separate modules this
//! crate does not yet have.

mod discovery;
mod error;
mod registration;

pub use discovery::{DiscoveredTool, ToolCache, discover_tools};
pub use error::McpClientError;
pub use registration::{McpConnection, McpServerRegistration, TransportConfig, connect, connect_with_timeout, global_client_enabled};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {}
}
