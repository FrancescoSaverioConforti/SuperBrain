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
mod audit;
/// B5 remediation M5 Task 1: pure hash-chain/recovery-scan primitives.
/// Kept private (not `pub mod`): only the specific items
/// `brain-api-audit-worker` (M5 Task 4) needs are re-exported below,
/// through this crate's own top-level facade, never the whole module --
/// see that `pub use` block's own doc comment for why each item is
/// there.
mod audit_chain;
/// B5 remediation M5 Task 4: the concrete, versioned log line format
/// (serialize/parse) -- kept separate from `audit_chain`'s own
/// format-agnostic pure primitives. See the module's own doc comment.
mod audit_record;
mod cli;
mod error;
mod graph;
mod knowledge;
pub mod limits;
mod memory;
mod policy_write;
mod proposal;
mod proposal_store;
mod read;
pub use approval::{approve_proposal, list_proposals, reject_proposal};
pub use audit::{allowed_from_result, event, record_permission_decision, record_tool_call, record_write_memory, AuditEvent, AuditEventKind, AuditSink, CommitFailureCategory, CommitOutcomeKind, NullAuditSink};
pub use audit_record::{parse_event, parse_line, serialize_record, AuditRecordParseError, AUDIT_RECORD_FORMAT_VERSION};
pub use cli::{run_cli, run_cli_durable, run_cli_with_audit, DurableApprovalHooks, DurableCliOutcome};

/// M5 Task 4's narrow facade onto `audit_chain`'s otherwise-private pure
/// primitives — re-exported individually, not `pub use audit_chain::*`,
/// so this crate's public surface documents exactly what an external
/// worker crate is authorized to depend on and why:
///
/// - [`audit_chain::GENESIS_HASH`]: the worker's in-memory chain-tail
///   state starts here before any record exists; must be the exact
///   constant, not an independently-duplicated literal.
/// - [`audit_chain::ChainRecord`]/[`audit_chain::LineOutcome`]: plain
///   data types the worker's startup recovery scan constructs and holds;
///   carry no behavior of their own.
/// - [`audit_chain::scan_chain`]: the one recovery/verification entry
///   point Task 1 built for exactly this reuse — the worker must never
///   reimplement chain-link verification itself.
/// - [`audit_chain::ScanResult`]/[`audit_chain::ScanStopReason`]/
///   [`audit_chain::ChainLinkError`]: `scan_chain`'s own return/error
///   shapes; required simply to call and match on it.
/// - [`audit_chain::may_resume_without_recovery`]: the worker's startup
///   gate, reused rather than re-derived from `ScanResult` by hand.
///
/// Deliberately NOT exported: `verify_record_hash`/`verify_link`
/// individually (the worker only ever needs the aggregate `scan_chain`
/// result, never per-record verification standalone) and
/// `compute_record_hash` (used internally by `audit_record::
/// serialize_record`, which is this crate's own, not the worker's, job
/// to call when producing new records).
pub use audit_chain::{
    may_resume_without_recovery, scan_chain, ChainLinkError, ChainRecord, LineOutcome, ScanResult, ScanStopReason, GENESIS_HASH,
};
pub use error::BrainApiError;
pub use graph::link_entities;
pub use knowledge::{get_canon, get_regulations, get_rules, BindingStatus, CanonEntry, KnowledgeItem};
pub use memory::{get_previous_solution, save_memory, update_memory, ProposedOperation, RelationKind};
pub use policy_write::{WriteOperation, WritePolicy};
pub use read::{get_code_context, get_entity, graph_query, retrieve, search, CodeContext, CodeContextEdge, Entity, GraphQuerySpec};
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
///
/// **Structural correction (B5, pre-closure):** the original shape of this
/// enum made `TrustedLocal` a plain, no-field variant — constructible by
/// ANY code that can name `brain_api::CallOrigin`, including a future
/// `mcp-server` crate that merely depends on `brain-api`, with no
/// constructor call to gate it at all. That violated the trust-boundary
/// invariant this module's own doc comment already claimed ("`origin` is
/// likewise never settable from an agent-facing payload"): the claim was
/// true for `RequestContext`'s own field assignment, but not for the
/// variant's constructibility itself. `TrustedLocal` now carries a
/// [`TrustedLocalToken`], whose only constructor is `pub(crate)` — so
/// external code can still *match* on `CallOrigin::TrustedLocal(_)` (or use
/// [`CallOrigin::is_trusted_local`] below) to read which origin a context
/// carries, but cannot construct one, no matter what payload a remote
/// caller controls. This is not a public API compatibility guarantee being
/// preserved — no stable external consumer depended on the old shape (the
/// only crate-external reference, `crates/brain-api/tests/
/// cli_approval_e2e.rs`, only ever constructs `McpSession`) — it is a
/// pre-closure correction of a real gap found during B5's layering review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallOrigin {
    McpSession(String),
    TrustedLocal(TrustedLocalToken),
}

/// A zero-sized, crate-private-constructor marker: proof that whoever
/// produced a [`CallOrigin::TrustedLocal`] went through `brain-api`'s own
/// trusted boundary code (today, only `cli::trusted_ctx`), never through a
/// payload field or a bare enum-variant literal written by an external
/// crate. The type itself is `pub` (it has to be, to appear inside a `pub
/// enum` variant that external code may still pattern-match on), but
/// [`TrustedLocalToken::new`] is `pub(crate)` — there is no public way to
/// obtain a value of this type from outside `brain-api`, and therefore no
/// public way to construct `CallOrigin::TrustedLocal(_)` from outside it
/// either. Carries no data: "trusted local" IS the identity, exactly as
/// `CallOrigin`'s own doc comment already said before this correction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustedLocalToken(());

impl TrustedLocalToken {
    pub(crate) fn new() -> Self {
        Self(())
    }
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
            CallOrigin::TrustedLocal(_) => "trusted-local".to_string(),
        }
    }

    /// Read-only discriminator for external code that needs to distinguish
    /// a trusted-local origin from an MCP one without being able to
    /// construct either — e.g. a future audit/log facade deciding how to
    /// render a context's origin. Never used by this crate's own
    /// permission checks, which always match on `ctx.write_policy`/
    /// `ctx.read_policy` directly, never on `origin` itself.
    pub fn is_trusted_local(&self) -> bool {
        matches!(self, CallOrigin::TrustedLocal(_))
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
///
/// `read_policy: Option<policy::ReadPolicy>` (B5.1 Task 16) follows the
/// exact same deferral discipline `write_policy` (Task 10) and
/// `PendingWriteProposal.operation` (Task 13/14) already established:
/// added only once a real capability (`search`/`retrieve`/
/// `get_code_context`) needed it, fail-closed default `None` (denies
/// every read operation, mirroring `write_policy: None`'s own meaning).
/// It is a SEPARATE field from `write_policy`, never conflated with it —
/// a caller with read permission gains no write-shaped capability from
/// that alone, and vice versa (same "no implicit escalation between axes"
/// principle `policy_write.rs`'s own test suite already pins for
/// `WritePolicy`/`ReadPolicy`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestContext {
    pub schema_version: u32,
    pub correlation_id: uuid::Uuid,
    pub write_policy: Option<WritePolicy>,
    pub read_policy: Option<policy::ReadPolicy>,
    pub origin: CallOrigin,
}

pub const SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {}
}
