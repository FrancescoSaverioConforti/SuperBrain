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

mod error;
mod policy_write;
mod proposal;
pub use error::BrainApiError;
pub use policy_write::{WriteOperation, WritePolicy};
pub use proposal::{
    InvalidProposalTransition, PendingWriteProposal, ProposalId, ProposalIdError, ProposalProvenance, ProposalStatus,
    valid_proposal_transition,
};

/// Identifies one Brain API call: schema version for forward-compat
/// rejection, and an opaque correlation id for audit/tracing. Mirrors
/// `api::RequestContext`'s shape deliberately for the first two fields —
/// this is a NEW struct, not a modification of `api::RequestContext`, per
/// the frozen constraint that `crates/api` is untouched by B5.
///
/// `write_policy: None` is the fail-closed default: no write-shaped
/// operation (`ProposeWrite`/`ApproveProposal`/`RejectProposal`) is
/// permitted unless a `Some(WritePolicy)` is present, and `WritePolicy`
/// itself has no public constructor that can set `approve_write_allowed`
/// from caller-supplied data (see `policy_write`'s module doc) — so there
/// is no way to build a `RequestContext` with approval rights other than
/// explicitly calling `WritePolicy::trusted_cli()` from trusted boundary
/// code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestContext {
    pub schema_version: u32,
    pub correlation_id: uuid::Uuid,
    pub write_policy: Option<WritePolicy>,
}

pub const SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {
        assert!(true);
    }
}
