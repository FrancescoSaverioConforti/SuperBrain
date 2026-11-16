//! Write/approval permission for B5's Brain API (B5.0 trust model). Lives
//! in `brain-api`, not `crates/policy`: its only consumer is `brain-api`
//! (per the design doc's crate-boundary decision), and `crates/policy` is a
//! shared dependency of `storage`/`api`/`brain-api` alike — a brain-api
//! -specific concept has no business living where those other crates would
//! see it. `ReadPolicy`/`ReadOperation` living in `crates/policy` makes
//! sense because both `api` and `brain-api` need read permission; write/
//! approval permission is brain-api-only.
//!
//! The MCP-boundary invariant this module exists to make structural, not
//! just documented: an MCP-originated caller can never obtain
//! `approve_write_allowed: true`. There is deliberately NO public
//! constructor that accepts `approve_write_allowed` as a caller-supplied
//! bool — only [`WritePolicy::mcp_agent`] (always `false`) and
//! [`WritePolicy::trusted_cli`] (the one place `true` can originate) exist.
//! A future MCP transport layer can call `mcp_agent()`, but has no API
//! surface through which to construct a policy with approval rights, no
//! matter what a remote payload claims — this is enforced by the type
//! system (no such constructor exists to call), not by a check a transport
//! layer could forget to run.

/// The three operations this crate's write/approval surface gates. Mirrors
/// `policy::ReadOperation`'s shape (a small, closed enum checked via
/// `.allows(op)`), scoped to brain-api's write/approval concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOperation {
    ProposeWrite,
    ApproveProposal,
    RejectProposal,
}

/// Write/approval permission for one Brain API call. Fail-closed: there is
/// no default that allows anything (see [`WritePolicy::none`], and the
/// `RequestContext.write_policy: Option<WritePolicy>` field this type backs
/// in `lib.rs`, where `None` means no write-shaped operation is permitted
/// at all).
///
/// No public field, no public generic constructor: the two booleans are
/// only ever set together by one of the two named constructors below, each
/// of which hard-codes `approve_write_allowed`'s value — never a parameter
/// a caller supplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WritePolicy {
    propose_write_allowed: bool,
    approve_write_allowed: bool,
}

impl WritePolicy {
    /// The policy an MCP-originated (or any other untrusted-transport)
    /// request gets: may propose a write, can never approve or reject one.
    /// `approve_write_allowed` is hard-coded `false` here — not a parameter,
    /// so no caller of this constructor can set it to `true` regardless of
    /// what a remote payload says.
    pub fn mcp_agent() -> Self {
        Self { propose_write_allowed: true, approve_write_allowed: false }
    }

    /// The only constructor that can produce `approve_write_allowed: true`.
    /// Intended for the trusted local CLI approval surface (B5) and, later,
    /// the B6 UI's human-initiated approval path — never for a transport
    /// that accepts remote input.
    pub fn trusted_cli() -> Self {
        Self { propose_write_allowed: true, approve_write_allowed: true }
    }

    /// The fail-closed default: permits nothing. What a `RequestContext`
    /// with `write_policy: None` is logically equivalent to; exists as an
    /// explicit value (rather than only as `None`) for code that needs a
    /// concrete `WritePolicy` to reason about rather than an `Option`.
    pub fn none() -> Self {
        Self { propose_write_allowed: false, approve_write_allowed: false }
    }

    pub fn allows(&self, op: WriteOperation) -> bool {
        match op {
            WriteOperation::ProposeWrite => self.propose_write_allowed,
            WriteOperation::ApproveProposal | WriteOperation::RejectProposal => self.approve_write_allowed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- the three named policies' exact permission matrix ---

    #[test]
    fn mcp_agent_policy_allows_propose() {
        assert!(WritePolicy::mcp_agent().allows(WriteOperation::ProposeWrite));
    }

    #[test]
    fn mcp_agent_policy_never_allows_approve_or_reject() {
        let p = WritePolicy::mcp_agent();
        assert!(!p.allows(WriteOperation::ApproveProposal));
        assert!(!p.allows(WriteOperation::RejectProposal));
    }

    #[test]
    fn trusted_cli_policy_allows_propose() {
        assert!(WritePolicy::trusted_cli().allows(WriteOperation::ProposeWrite));
    }

    #[test]
    fn trusted_cli_policy_allows_approve_and_reject() {
        let p = WritePolicy::trusted_cli();
        assert!(p.allows(WriteOperation::ApproveProposal));
        assert!(p.allows(WriteOperation::RejectProposal));
    }

    #[test]
    fn none_policy_denies_every_operation() {
        let p = WritePolicy::none();
        assert!(!p.allows(WriteOperation::ProposeWrite));
        assert!(!p.allows(WriteOperation::ApproveProposal));
        assert!(!p.allows(WriteOperation::RejectProposal));
    }

    // --- fail-closed: missing policy (None) denies everything, checked at
    //     the RequestContext level once that field exists (below) ---

    #[test]
    fn missing_write_policy_on_request_context_denies_every_write_operation() {
        let ctx = crate::RequestContext { schema_version: crate::SCHEMA_VERSION, correlation_id: uuid::Uuid::new_v4(), write_policy: None };
        let allowed = |op: WriteOperation| ctx.write_policy.as_ref().is_some_and(|p| p.allows(op));
        assert!(!allowed(WriteOperation::ProposeWrite));
        assert!(!allowed(WriteOperation::ApproveProposal));
        assert!(!allowed(WriteOperation::RejectProposal));
    }

    #[test]
    fn present_write_policy_on_request_context_is_consulted() {
        let ctx = crate::RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: Some(WritePolicy::trusted_cli()),
        };
        let allowed = |op: WriteOperation| ctx.write_policy.as_ref().is_some_and(|p| p.allows(op));
        assert!(allowed(WriteOperation::ApproveProposal));
    }

    // --- no read-shaped policy combination enables write implicitly:
    //     WritePolicy has no field, constructor, or From/Into path that
    //     derives from policy::ReadPolicy, so this is pinned by the type
    //     signature itself -- documented here as the test a reviewer
    //     should extend if that ever changes.

    #[test]
    fn write_policy_has_no_path_from_read_policy() {
        // There is no `From<policy::ReadPolicy> for WritePolicy`, no
        // constructor taking a ReadPolicy, and no field a ReadPolicy's
        // bools could flow into. This test exists so a reviewer adding
        // such a conversion later sees this comment and reconsiders: a
        // read-allow decision must never imply a write-allow one.
        let read_policy = policy::ReadPolicy::new(true, true);
        let _ = read_policy; // confirms the type exists and is constructible independently of WritePolicy
        // No assertion beyond compiling: the absence of a conversion path
        // is the property under test, proven by this file simply not
        // having one to call.
    }

    // --- external-construction cannot bypass approve_write_allowed ---

    #[test]
    fn no_public_api_can_construct_approve_write_allowed_true_except_trusted_cli() {
        // mcp_agent() and none() both hard-code approve_write_allowed to
        // false; trusted_cli() is the only constructor that can produce
        // true, and it takes no parameters at all -- there is no bool a
        // caller passes in, remote or otherwise, that reaches this field.
        assert!(!WritePolicy::mcp_agent().allows(WriteOperation::ApproveProposal));
        assert!(!WritePolicy::none().allows(WriteOperation::ApproveProposal));
        assert!(WritePolicy::trusted_cli().allows(WriteOperation::ApproveProposal));
    }

    // --- Copy/Eq round-trip (no serde in this crate yet; this is the
    //     relevant "round-trip" property until a wire format exists) ---

    #[test]
    fn write_policy_is_copy_and_compares_equal_to_itself() {
        let p = WritePolicy::trusted_cli();
        let copied = p;
        assert_eq!(p, copied);
    }

    #[test]
    fn distinct_policies_are_not_equal() {
        assert_ne!(WritePolicy::mcp_agent(), WritePolicy::trusted_cli());
        assert_ne!(WritePolicy::mcp_agent(), WritePolicy::none());
    }
}
