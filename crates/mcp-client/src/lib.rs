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
//! dependency list is the proof: `rmcp` + `tokio` only.
//!
//! B5.8 (this task, Task 26): connection lifecycle and server
//! registration — `McpServerRegistration`, `connect()`, and the one global
//! kill-switch (`global_client_enabled()`). Discovery (Task 27),
//! authorization (Task 28), and invocation (Task 29) are later, separate
//! modules this crate does not yet have.

mod error;
mod registration;

pub use error::McpClientError;
pub use registration::{McpConnection, McpServerRegistration, TransportConfig, connect, connect_with_timeout, global_client_enabled};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {}
}
