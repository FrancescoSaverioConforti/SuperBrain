//! SuperBrain Brain API orchestration layer (B5).
//!
//! Sits between the MCP server (and, later, the B6 UI) and the existing
//! read-only `crates/api` plus `storage`/`policy`/`domain`/`index`. Delegates
//! reads to `api` unchanged, owns `PendingWriteProposal` persistence and
//! lifecycle, and is the only caller of `storage::propose_and_commit` for
//! approved proposals. `crates/api` itself stays exactly what it already is
//! ("B1 callable core: local single-user read-only functions, no
//! transport") — nothing here modifies it.
//!
//! See `docs/superpowers/specs/2026-11-15-b5-0-brain-api-contract-freeze.md`
//! for the frozen contract this crate implements.

mod approval;
mod cli;
mod error;
mod graph;
mod memory;
mod policy_write;
mod proposal;
mod proposal_store;
pub use approval::{approve_proposal, list_proposals, reject_proposal};
pub use cli::run_cli;
pub use error::BrainApiError;
pub use graph::link_entities;
pub use memory::{save_memory, update_memory, ProposedOperation, RelationKind};
pub use policy_write::{WriteOperation, WritePolicy};
pub use proposal_store::{load_proposal, save_proposal};
pub use proposal::{
    InvalidProposalTransition, PendingWriteProposal, ProposalId, ProposalIdError, ProposalProvenance, ProposalStatus,
    valid_proposal_transition,
};

/// Who/what a Brain API call originated from, as asserted by the TRUSTED
/// BOUNDARY that constructed the `RequestContext` carrying it — never a
/// value parsed or forwarded from an agent-facing request payload. This is
/// deliberately minimal (no general Actor/Identity system, per the user's
/// explicit instruction): just enough to let `save_memory`/`update_memory`/
/// `link_entities` record a `PendingWriteProposal`'s `originating_identity`
/// without ever exposing that string as a function parameter a caller
/// could set directly.
///
/// - [`CallOrigin::McpSession`] carries whatever opaque, non-sensitive
///   label the (future, B5.8) MCP connection/session-binding layer assigns
///   to a connection — never a field the connected agent's own request
///   supplied. Until that transport exists, test/internal code may
///   construct this directly (there is no transport yet to compromise),
///   but once it exists, only that transport's own connection-handling
///   code may construct one, exactly as `policy::ReadPolicy`'s module doc
///   states the same rule for read permissions.
/// - [`CallOrigin::TrustedLocal`] is the trusted local caller (the CLI
///   approval surface today; later, in B6, the UI acting on its own
///   initiative) — carries no identity string at all, since "trusted
///   local" IS the identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallOrigin {
    McpSession(String),
    TrustedLocal,
}

impl CallOrigin {
    /// The opaque label to record as a proposal's `originating_identity`:
    /// the session label for `McpSession`, or a fixed, non-identity-leaking
    /// constant for `TrustedLocal` (there is no per-session string to carry
    /// for the trusted local surface, and none is needed — "trusted local"
    /// is itself the full identity this system cares about recording).
    pub fn as_identity_label(&self) -> String {
        match self {
            CallOrigin::McpSession(label) => label.clone(),
            CallOrigin::TrustedLocal => "trusted-local".to_string(),
        }
    }
}

/// Identifies one Brain API call: schema version for forward-compat
/// rejection, an opaque correlation id for audit/tracing, and the trusted
/// [`CallOrigin`] this call came from. Mirrors `api::RequestContext`'s
/// shape deliberately for the first two fields — this is a NEW struct, not
/// a modification of `api::RequestContext`, per the frozen constraint that
/// `crates/api` is untouched by B5.
///
/// `write_policy: None` is the fail-closed default: no write-shaped
/// operation (`ProposeWrite`/`ApproveProposal`/`RejectProposal`) is
/// permitted unless a `Some(WritePolicy)` is present, and `WritePolicy`
/// itself has no public constructor that can set `approve_write_allowed`
/// from caller-supplied data (see `policy_write`'s module doc) — so there
/// is no way to build a `RequestContext` with approval rights other than
/// explicitly calling `WritePolicy::trusted_cli()` from trusted boundary
/// code.
///
/// `origin` is likewise never settable from an agent-facing payload: the
/// boundary code that builds a `RequestContext` (a CLI entry point today;
/// the MCP transport's connection handling in B5.8) is the only code that
/// may choose this field's value, and `save_memory`/`update_memory`/
/// `link_entities` (Task 13) read `originating_identity` from THIS field,
/// never from a separate function parameter — there is no such parameter
/// on any of those three functions' public signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestContext {
    pub schema_version: u32,
    pub correlation_id: uuid::Uuid,
    pub write_policy: Option<WritePolicy>,
    pub origin: CallOrigin,
}

pub const SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {}
}
