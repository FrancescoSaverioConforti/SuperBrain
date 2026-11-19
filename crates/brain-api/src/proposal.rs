//! The `PendingWriteProposal` domain model (B5.1): a write an agent has
//! proposed, not yet executed. Per the B5.0 frozen trust model
//! (`docs/superpowers/specs/2026-11-15-b5-0-brain-api-contract-freeze.md`):
//! an agent (local or remote, over MCP) can only ever construct a `Pending`
//! proposal — approval/rejection are separate operations, gated by a
//! distinct, privileged capability (`WritePolicy::approve_write_allowed`,
//! added in Task 10), never settable through this module's own API.

/// A validated proposal identifier. Unlike `storage::NoteId` (validated
/// against a rich charset because note ids are sometimes human-typed with
/// semantic meaning), `ProposalId`s are always system-generated — the only
/// production constructor is [`ProposalId::generate`], which wraps a fresh
/// random UUID. [`ProposalId::parse`] exists for the round-trip case
/// (loading a persisted/transmitted id back into a `ProposalId`) and
/// accepts only a syntactically valid UUID string: this is deliberately
/// narrower than `NoteId`'s charset-based validation, because a UUID's
/// fixed, hyphenated hex format has no path-separator, no `..`, and no
/// filesystem-meaningful character by construction — there is no path
/// traversal surface to defend against separately.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProposalId(uuid::Uuid);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProposalIdError {
    #[error("proposal id is not a valid UUID")]
    NotAUuid,
}

impl ProposalId {
    /// The production constructor: a fresh, random (v4) proposal id. Every
    /// real `save_memory`/`update_memory`/`link_entities` call mints one of
    /// these; nothing else in this crate's public API can produce an
    /// arbitrary `ProposalId` from caller-supplied data.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    /// Parses a previously-generated id back from its string form (e.g.
    /// loading a persisted proposal, or a caller naming an id in an
    /// approve/reject request). Rejects anything that is not a
    /// syntactically valid UUID; never normalizes.
    pub fn parse(raw: &str) -> Result<Self, ProposalIdError> {
        uuid::Uuid::parse_str(raw)
            .map(Self)
            .map_err(|_| ProposalIdError::NotAUuid)
    }
}

impl std::fmt::Display for ProposalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a proposal came from, scoped narrowly to what B5 actually needs
/// (per the user's explicit instruction: no global `Actor`/`Identity`
/// system yet). Deliberately does NOT carry anything resembling a human
/// approver or a trust decision — this struct describes the proposal's
/// ORIGIN only, never its disposition.
///
/// `source` is an opaque, audit-only label. Its value is never
/// caller-supplied data trusted for a security decision: for an
/// MCP-originated proposal it is derived from the MCP connection/session
/// binding established at the transport boundary (B5.8), never from a
/// field the calling agent sets in its request payload (continuity
/// `d246ec`) — this struct's constructor does not enforce that itself
/// (it cannot know where its caller got the string from), so the
/// *calling* boundary code is responsible for only ever passing a
/// binding-derived value here, exactly as `policy::ReadPolicy`'s own
/// module doc states the same rule for read permissions ("supplied by the
/// trusted... host... never inferred from retrieved text").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalProvenance {
    pub source: String,
    pub created_at: String,
}

/// A write an agent has proposed but that has not been — and, via this
/// module's own API, cannot be — executed. `status` starts `Pending` and
/// advances only through [`valid_proposal_transition`]-checked moves made
/// by a separate, privileged approval operation (Task 14), never by
/// constructing or mutating this struct directly from agent-reachable code.
///
/// `operation: ProposedOperation` is the exact write this proposal names —
/// set once, at construction, from the payload `save_memory`/
/// `update_memory`/`link_entities` (Task 13) validated and built, and never
/// changed afterwards (no setter, no mutation path, not even `transition_to`
/// touches it). This is the ONLY data `approve_proposal` (Task 14) may
/// execute: its own signature takes no operation/payload parameter through
/// which a caller could substitute something else at approval time — the
/// persisted proposal is authoritative for what gets executed, by
/// construction, not by convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWriteProposal {
    pub id: ProposalId,
    pub originating_identity: String,
    pub provenance: ProposalProvenance,
    pub expires_at: Option<String>,
    pub operation: crate::ProposedOperation,
    status: ProposalStatus,
}

impl PendingWriteProposal {
    /// The only constructor: every `PendingWriteProposal` is born `Pending`.
    /// There is no constructor, in this module or any other, that can
    /// produce one already `Approved` — an agent calling `save_memory`/
    /// `update_memory`/`link_entities` can reach only this path.
    pub fn new(
        id: ProposalId,
        originating_identity: String,
        provenance: ProposalProvenance,
        expires_at: Option<String>,
        operation: crate::ProposedOperation,
    ) -> Self {
        Self {
            id,
            originating_identity,
            provenance,
            expires_at,
            operation,
            status: ProposalStatus::Pending,
        }
    }

    pub fn status(&self) -> &ProposalStatus {
        &self.status
    }

    /// The only way to change `status` after construction. Validates the
    /// transition via [`valid_proposal_transition`] and refuses — leaving
    /// `status` unchanged — if it is not legal. There is no public setter:
    /// this is the sole mutation path, and it is not `pub` to the crate
    /// root's re-export list by accident — callers outside this module
    /// (including the future approval operation, Task 14) must go through
    /// this method, which cannot be bypassed to assign `Approved` directly.
    #[allow(dead_code)] // wired to approve_proposal/reject_proposal in Task 14
    pub(crate) fn transition_to(&mut self, new_status: ProposalStatus) -> Result<(), InvalidProposalTransition> {
        if valid_proposal_transition(&self.status, &new_status) {
            self.status = new_status;
            Ok(())
        } else {
            Err(InvalidProposalTransition { from: self.status.clone(), to: new_status })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot transition proposal from {from:?} to {to:?}")]
pub struct InvalidProposalTransition {
    pub from: ProposalStatus,
    pub to: ProposalStatus,
}

/// A proposal's lifecycle state. Modeled on `domain::write_state::WriteState`'s
/// `valid_transition` idiom (small enum + a free function naming exactly
/// which moves are legal), not a derived/generic state-machine library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalStatus {
    Pending,
    Approved { approved_at: String },
    Rejected { reason: String, rejected_at: String },
    Expired,
}

/// Exactly the moves the B5.0 contract allows: `Pending` to any one of the
/// three terminal states, and nothing else. Every terminal state is
/// terminal — no move out of `Approved`, `Rejected`, or `Expired`, in
/// either direction, including back to `Pending` or between two terminal
/// states. No idempotent self-transition is treated as valid here either
/// (e.g. `Approved -> Approved`): the contract names only the three
/// `Pending -> X` moves as valid, so anything else — including a
/// same-state move — is false by the match's own structure, matching the
/// user's explicit "no automatic idempotent transitions absent an explicit
/// contract decision."
pub fn valid_proposal_transition(from: &ProposalStatus, to: &ProposalStatus) -> bool {
    use ProposalStatus::*;
    matches!(
        (from, to),
        (Pending, Approved { .. }) | (Pending, Rejected { .. }) | (Pending, Expired)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved() -> ProposalStatus {
        ProposalStatus::Approved { approved_at: "2026-11-16T00:00:00Z".into() }
    }
    fn rejected() -> ProposalStatus {
        ProposalStatus::Rejected { reason: "r".into(), rejected_at: "2026-11-16T00:00:00Z".into() }
    }
    fn sample_operation() -> crate::ProposedOperation {
        crate::ProposedOperation::SaveMemory {
            note_id: "note-sample".into(),
            note_type: domain::NoteType::Bug,
            classification: domain::Classification::Internal,
            project: "project-x".into(),
            source: "agent:coder@prompt-v3".into(),
            event_period: "2026-11-16".into(),
            body: "sample body".into(),
        }
    }

    // --- valid_proposal_transition: the three allowed moves ---

    #[test]
    fn pending_can_advance_to_approved() {
        assert!(valid_proposal_transition(&ProposalStatus::Pending, &approved()));
    }

    #[test]
    fn pending_can_advance_to_rejected() {
        assert!(valid_proposal_transition(&ProposalStatus::Pending, &rejected()));
    }

    #[test]
    fn pending_can_advance_to_expired() {
        assert!(valid_proposal_transition(&ProposalStatus::Pending, &ProposalStatus::Expired));
    }

    // --- every other transition is invalid ---

    #[test]
    fn approved_is_terminal() {
        let a = approved();
        assert!(!valid_proposal_transition(&a, &rejected()));
        assert!(!valid_proposal_transition(&a, &ProposalStatus::Pending));
        assert!(!valid_proposal_transition(&a, &ProposalStatus::Expired));
        assert!(!valid_proposal_transition(&a, &approved()), "no self-transition / re-approval");
    }

    #[test]
    fn rejected_is_terminal() {
        let r = rejected();
        assert!(!valid_proposal_transition(&r, &approved()));
        assert!(!valid_proposal_transition(&r, &ProposalStatus::Pending));
        assert!(!valid_proposal_transition(&r, &ProposalStatus::Expired));
        assert!(!valid_proposal_transition(&r, &rejected()), "no self-transition");
    }

    #[test]
    fn expired_is_terminal() {
        let e = ProposalStatus::Expired;
        assert!(!valid_proposal_transition(&e, &approved()));
        assert!(!valid_proposal_transition(&e, &rejected()));
        assert!(!valid_proposal_transition(&e, &ProposalStatus::Pending));
        assert!(!valid_proposal_transition(&e, &ProposalStatus::Expired), "no self-transition");
    }

    #[test]
    fn pending_cannot_self_transition() {
        assert!(!valid_proposal_transition(&ProposalStatus::Pending, &ProposalStatus::Pending));
    }

    // --- construction: a proposal is always born Pending ---

    #[test]
    fn new_proposal_is_always_pending() {
        let p = PendingWriteProposal::new(
            ProposalId::generate(),
            "mcp-session-abc".into(),
            ProposalProvenance { source: "mcp-session-abc".into(), created_at: "2026-11-16T00:00:00Z".into() },
            None,
            sample_operation(),
        );
        assert_eq!(*p.status(), ProposalStatus::Pending);
    }

    #[test]
    fn transition_to_approved_succeeds_from_pending() {
        let mut p = PendingWriteProposal::new(
            ProposalId::generate(),
            "mcp-session-abc".into(),
            ProposalProvenance { source: "mcp-session-abc".into(), created_at: "2026-11-16T00:00:00Z".into() },
            None,
            sample_operation(),
        );
        assert!(p.transition_to(approved()).is_ok());
        assert_eq!(*p.status(), approved());
    }

    #[test]
    fn transition_to_an_invalid_target_fails_and_leaves_status_unchanged() {
        let mut p = PendingWriteProposal::new(
            ProposalId::generate(),
            "mcp-session-abc".into(),
            ProposalProvenance { source: "mcp-session-abc".into(), created_at: "2026-11-16T00:00:00Z".into() },
            None,
            sample_operation(),
        );
        p.transition_to(approved()).unwrap();
        // Now Approved; attempting Approved -> Rejected must fail and leave status as Approved.
        let result = p.transition_to(rejected());
        assert!(result.is_err());
        assert_eq!(*p.status(), approved(), "status must be unchanged after a rejected transition attempt");
    }

    // --- ProposalId: valid/invalid parsing, round-trip, no path semantics ---

    #[test]
    fn generated_id_round_trips_through_display_and_parse() {
        let id = ProposalId::generate();
        let text = id.to_string();
        let reparsed = ProposalId::parse(&text).unwrap();
        assert_eq!(id, reparsed);
    }

    #[test]
    fn a_valid_uuid_string_parses() {
        assert!(ProposalId::parse("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn an_empty_string_is_rejected() {
        assert_eq!(ProposalId::parse(""), Err(ProposalIdError::NotAUuid));
    }

    #[test]
    fn a_path_traversal_shaped_string_is_rejected_as_not_a_uuid() {
        // Not a dedicated path-traversal check (unlike NoteId) -- UUID
        // parsing itself rejects anything with a '/' or '..', since those
        // are not valid UUID characters. This test pins that this rejection
        // actually happens, not just that it theoretically would.
        for bad in ["../../etc/passwd", "/etc/passwd", "a/b", "..", "."] {
            assert_eq!(ProposalId::parse(bad), Err(ProposalIdError::NotAUuid), "input {bad:?} must be rejected");
        }
    }

    #[test]
    fn a_syntactically_close_but_invalid_uuid_is_rejected() {
        assert_eq!(
            ProposalId::parse("550e8400-e29b-41d4-a716-44665544000"), // one hex digit short
            Err(ProposalIdError::NotAUuid)
        );
    }

    #[test]
    fn two_generated_ids_are_distinct() {
        assert_ne!(ProposalId::generate(), ProposalId::generate());
    }

    // --- status cannot be mutated except through transition_to ---

    #[test]
    fn status_field_is_not_directly_settable_from_outside_the_module() {
        // Compile-time property, not a runtime assertion: `status` is a
        // private field and PendingWriteProposal has no public setter other
        // than `transition_to`. This test exists as documentation of that
        // invariant and as a canary -- if `status` is ever made `pub` or a
        // `set_status` method is added, this comment is the place a reviewer
        // should notice the regression. (A genuine compile-fail doctest, per
        // storage::lib's own pattern, is deferred until this crate has a
        // public doc-test surface worth guarding this way.)
        let p = PendingWriteProposal::new(
            ProposalId::generate(),
            "x".into(),
            ProposalProvenance { source: "x".into(), created_at: "2026-11-16T00:00:00Z".into() },
            None,
            sample_operation(),
        );
        assert_eq!(*p.status(), ProposalStatus::Pending);
    }
}
