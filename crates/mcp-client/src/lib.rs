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
//! B5.9 (Task 27): capability discovery and a tool metadata cache
//! (`discover_tools`, `ToolCache`, `DiscoveredTool`) — calls a connected
//! server's real `tools/list` and validates the reply before trusting any
//! of it. `DiscoveredTool::classification` is an inert `Option<()>`
//! placeholder, always `None`, never read by anything in this crate.
//!
//! B5.9 (Task 28): the external tool authorization model (`permission`
//! module) — `ExternalToolMetadata`, `ToolClassification`, `Sensitivity`,
//! `ApprovalPolicy`, `ScopeLimits`, and the one decision function
//! `authorize`. Deliberately NOT connected to `discovery`: `authorize`
//! never reads a `DiscoveredTool`, and nothing in this module derives
//! `ExternalToolMetadata` from server-supplied data automatically —
//! external tool metadata is untrusted input (see `permission`'s own
//! module doc comment).
//!
//! B5.9 (this task, Task 29): invocation (`invoke` module) —
//! `invoke_external_tool`, the one path this crate has to actually call
//! a tool on an already-connected external server. Always runs
//! `authorize` first (a denial never touches the transport), always
//! times out rather than hanging, and always validates a successful
//! reply before trusting it. Also defines a deliberately minimal,
//! LOCAL, provisional invocation log (`InvocationLogEntry`) — NOT Task
//! 33's real `brain_api::AuditEvent` schema, which this crate has no
//! dependency on and does not anticipate; see `invoke`'s own module doc
//! comment for why.

mod discovery;
mod error;
mod invoke;
mod permission;
mod registration;

pub use discovery::{DiscoveredTool, ToolCache, discover_tools};
pub use error::McpClientError;
pub use invoke::{InvocationLogEntry, InvocationLogSink, InvocationOutcome, NullSink, invoke_external_tool, invoke_external_tool_logged, invoke_external_tool_with_timeout};
pub use permission::{ApprovalPolicy, ExternalToolMetadata, ScopeLimits, Sensitivity, ToolClassification, authorize};
pub use registration::{McpConnection, McpServerRegistration, TransportConfig, connect, connect_with_timeout, global_client_enabled};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {}

    /// B5.16 Task 35: `registration::DEFAULT_CONNECT_TIMEOUT` and
    /// `invoke::DEFAULT_INVOKE_TIMEOUT` are this crate's own,
    /// independently-defined timeout defaults — kept at the SAME VALUE
    /// as `brain_api::limits::DEFAULT_TIMEOUT_MS` (30 seconds) by
    /// convention, never by a shared import, because this crate must
    /// never depend on `brain-api` at all (the frozen B5.10 trust-
    /// boundary separation — see `tests/separation.rs`). This test
    /// cannot reference `brain_api::limits::DEFAULT_TIMEOUT_MS` directly
    /// for that same reason; it instead pins the literal value
    /// `brain-api`'s own constant is documented to hold, so a future
    /// change to either side's literal — without updating the other —
    /// fails this test rather than silently drifting the two crates'
    /// "same default" claim apart.
    #[test]
    fn mcp_clients_default_timeouts_still_match_brain_apis_canonical_value() {
        const BRAIN_API_DEFAULT_TIMEOUT_MS: u64 = 30_000; // brain_api::limits::DEFAULT_TIMEOUT_MS's documented value
        let brain_api_default = std::time::Duration::from_millis(BRAIN_API_DEFAULT_TIMEOUT_MS);
        assert_eq!(crate::registration::DEFAULT_CONNECT_TIMEOUT, brain_api_default, "mcp-client's connect timeout default has drifted from brain-api's canonical default");
        assert_eq!(crate::invoke::DEFAULT_INVOKE_TIMEOUT, brain_api_default, "mcp-client's invoke timeout default has drifted from brain-api's canonical default");
    }
}
