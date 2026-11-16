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

/// Identifies one Brain API call: schema version for forward-compat
/// rejection, and an opaque correlation id for audit/tracing. Mirrors
/// `api::RequestContext`'s shape deliberately (same two fields) — this is a
/// NEW struct, not a modification of `api::RequestContext`, per the frozen
/// constraint that `crates/api` is untouched by B5.
///
/// Does not yet carry a `write_policy` field: `policy::WritePolicy` doesn't
/// exist until Task 10 adds it. Added then, not now, so this struct is never
/// defined against a type that doesn't exist yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestContext {
    pub schema_version: u32,
    pub correlation_id: uuid::Uuid,
}

pub const SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {
        assert!(true);
    }
}
