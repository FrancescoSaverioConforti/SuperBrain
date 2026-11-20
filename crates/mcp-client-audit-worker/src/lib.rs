//! SuperBrain M6.3: MCP Client durable audit worker and event model.
//!
//! **Scope discipline (this task's own explicit authorization)**: this
//! crate implements the independent audit event model
//! ([`event::McpClientEvent`]/[`event::McpClientEventKind`]) and the
//! durable, cancellation-safe writer task ([`spawn_worker`]) for MCP
//! client activity. It does NOT connect external MCP execution to any
//! production CLI, does NOT create an MCP Server tool for external
//! execution, does NOT create a production external-tool launcher, does
//! NOT implement autonomous agent orchestration, and does NOT implement
//! log rotation or retention. None of those are in this task's scope.
//!
//! **Dependency boundary (verified by `tests/dependency_boundary.rs`)**:
//! this crate may depend on `mcp-client` (to name its types in its own
//! doc comments/conversions) and `audit-chain-core` (the same pure
//! hash-chain/record-envelope primitives `brain-api-audit-worker`
//! reuses), plus `tokio` and minimal serialization/error dependencies.
//! It must NEVER depend on `brain-api`, `storage`, or `mcp-server` --
//! this crate's own `Cargo.toml` is the proof (parsed as text by that
//! test, the same pattern `crates/mcp-client/tests/separation.rs`
//! already established for `mcp-client` itself).
//!
//! **The durable writer remains owned by a future composition-root
//! binary** (not part of this task), via [`writer::DurableAuditWriter`]
//! -- a narrow injected interface a composition root implements by
//! adapting a real `storage::Vault::open_audit_writer`-backed handle
//! (Option C, approved at the M6.1 security gate). This crate introduces
//! no independent filesystem writer of its own (an explicit scope
//! prohibition) -- see `writer.rs`'s own module doc comment for exactly
//! why this interface has the shape it has.
//!
//! **No claim of exactly-once execution** (an explicit Phase 4
//! requirement): this crate's own durability guarantee covers the AUDIT
//! RECORD ITSELF -- once `record_durable` resolves `Ok`, that record is
//! genuinely on disk. It makes no claim whatsoever about whether an
//! external MCP tool's own side effect happened exactly once, at all, or
//! not at all; see [`event::McpClientEventKind::ExternalSideEffectUnknown`]
//! for the explicit, named representation of that unresolvable ambiguity.
//! An audit failure (this crate's own write not completing) must never
//! be mistaken for a confirmed external execution failure or success --
//! the two are independent facts this crate's own types never conflate.

mod commands;
mod error;
mod event;
mod handle;
mod recovery;
mod writer;
mod worker;

pub use commands::DurableAck;
pub use error::{DurableRecordError, EnqueueError, RecoveryError, WorkerStartupError};
pub use event::{event, McpClientEvent, McpClientEventKind, DenialReason, DiscoveryOutcomeKind, InvocationOutcomeKind, RegistrationOutcomeKind, SensitivityLabel, ToolClassificationLabel, UnknownOutcomeReason, MCP_CLIENT_EVENT_FORMAT_VERSION};
pub use handle::AuditWorkerHandle;
pub use worker::{spawn_worker, WorkerConfig, QUEUE_CAPACITY};
pub use writer::{DurableAuditWriter, WriterAppendError};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {}
}
