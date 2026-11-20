//! M6.3: a dedicated, versioned, content-free audit event schema for
//! external MCP client activity — independent of `brain_api::AuditEvent`
//! (that schema stays brain-api's own; this module has no dependency on
//! `brain-api` at all, matching this crate's own `Cargo.toml`).
//!
//! **Why a separate schema, not a shared one**: `mcp-client` is, and
//! must remain, dependency-isolated from `brain-api`/`storage`/
//! `mcp-server` (B5.10, `crates/mcp-client/tests/separation.rs`). This
//! crate (`mcp-client-audit-worker`) depends on `mcp-client` (to name its
//! types in doc comments/tests) and `audit-chain-core` (the same pure
//! hash-chain/record-envelope primitives `brain-api-audit-worker` reuses)
//! but never on `brain-api` itself — so this crate cannot construct a
//! `brain_api::AuditEventKind` even if it wanted to. A future, separate,
//! trusted boundary (not part of this task) is responsible for ever
//! translating one of these events into a `brain_api::AuditEvent`, if
//! that translation is ever built at all; this module makes no such
//! claim.
//!
//! **Relationship to `mcp_client::InvocationLogEntry`**: that type (B5.9
//! Task 29) is UNCHANGED by this task and stays exactly what its own doc
//! comment already says: a minimal, non-durable, in-process diagnostic
//! record with no correlation id, no invocation id, and no timestamp.
//! This module's own events are never constructed BY retroactively
//! reading an `InvocationLogEntry` after the fact — a correlation id and
//! invocation id are minted by the caller at the moment of the real
//! action (registration attempt, authorization check, invocation
//! attempt), not inferred from an existing log entry after execution (an
//! explicit task constraint). The two types serve different purposes and
//! are never converted into one another by any function in this crate.
//!
//! **Content-free discipline** (the same rule `brain_api::audit`'s own
//! module doc comment states for its schema, applied here for the same
//! reason): every field below is an identifier, a boolean, or a short
//! category/reason label — never a tool argument, a tool response body,
//! a secret, an environment variable, or any filesystem content. There is
//! no field anywhere in [`McpClientEventKind`] shaped to hold such a
//! value, by construction — enforced by a regression test in this
//! module's own `tests` submodule, mirroring `brain_api::audit`'s own
//! `no_variant_serializes_a_note_body_or_query_text_shaped_string`-style
//! guard.
//!
//! **Format version**: [`MCP_CLIENT_EVENT_FORMAT_VERSION`] is this
//! schema's own version marker (distinct from, and never confused with,
//! `audit_chain_core::AUDIT_RECORD_FORMAT_VERSION`, which versions the
//! on-disk record ENVELOPE — the line format — not the event schema
//! carried inside it; the two numbers evolve independently). Embedded
//! inside every [`McpClientEvent`] as `schema_version`, so a reader can
//! tell which version of THIS schema a stored event used even after the
//! envelope format itself has stayed at version 1.

use serde::{Deserialize, Serialize};

/// This schema's own version marker — bumped only when
/// [`McpClientEventKind`]'s variants or fields change in a way that
/// breaks a prior reader's ability to parse an older event. Distinct
/// from `audit_chain_core::AUDIT_RECORD_FORMAT_VERSION` (see this
/// module's own top doc comment).
pub const MCP_CLIENT_EVENT_FORMAT_VERSION: u32 = 1;

/// One durable MCP client audit record: a categorized event, this
/// schema's own version, when it happened, and the correlation id tying
/// it back to whatever caller-side request produced it. Mirrors
/// `brain_api::AuditEvent`'s own shape deliberately (the same proven
/// pattern, reused for consistency across this codebase's two audit
/// schemas) without sharing the type itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpClientEvent {
    pub schema_version: u32,
    pub kind: McpClientEventKind,
    pub timestamp: String,
    pub correlation_id: String,
}

/// Builds one [`McpClientEvent`] of `kind`, stamped with `now`,
/// `correlation_id`, and this module's own current
/// [`MCP_CLIENT_EVENT_FORMAT_VERSION`] — the one constructor every call
/// site goes through, so every event's shape is produced identically.
pub fn event(kind: McpClientEventKind, now: &str, correlation_id: &str) -> McpClientEvent {
    McpClientEvent { schema_version: MCP_CLIENT_EVENT_FORMAT_VERSION, kind, timestamp: now.to_string(), correlation_id: correlation_id.to_string() }
}

/// The closed set of MCP client audit event categories. Every field is
/// an identifier, a boolean, or a short category/reason label — see this
/// module's own top doc comment's "Content-free discipline" section.
/// `invocation_id` (where present) is a fresh identifier minted by the
/// caller at the moment of the real invocation attempt, independent of
/// `correlation_id` (the enclosing request) and independent of
/// `mcp_client::InvocationLogEntry` (never inferred from it after the
/// fact) — the same "mint at the moment of the attempt" discipline
/// `brain_api::AuditEventKind::ApprovalAttempted`'s own doc comment
/// already establishes for `attempt_id`, reused here for consistency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum McpClientEventKind {
    /// A server registration was attempted (`McpServerRegistration`
    /// constructed and a `connect()` call was about to be made, or was
    /// just denied before any transport was touched) — `server` is the
    /// registration's own `name`, never a full transport command line
    /// (which could carry caller-supplied content).
    RegistrationAttempted { server: String },
    /// The outcome of exactly one registration attempt (linked to its
    /// own `RegistrationAttempted` by their shared `correlation_id`, the
    /// same "no separate attempt_id needed because exactly one
    /// registration event pair ever shares a correlation_id" shape this
    /// module uses for registration/discovery, distinct from invocation
    /// events below, which DO need their own `invocation_id` since many
    /// invocations can share one correlation_id).
    RegistrationOutcome { server: String, outcome: RegistrationOutcomeKind },
    /// A `tools/list` discovery call was attempted against `server`.
    DiscoveryAttempted { server: String },
    /// The outcome of exactly one discovery attempt.
    DiscoveryOutcome { server: String, outcome: DiscoveryOutcomeKind },
    /// A tool was classified by the caller's own judgment (never
    /// inferred from server-supplied metadata — see `mcp_client::
    /// permission`'s own module doc comment on untrusted metadata) as
    /// part of constructing its `ExternalToolMetadata`. Recorded so a
    /// reader can see WHEN a classification decision was made, without
    /// this event itself carrying the tool's real description/schema.
    ToolClassified { server: String, tool_name: String, classification: ToolClassificationLabel, sensitivity: SensitivityLabel },
    /// An authorization check (`mcp_client::authorize`) was performed and
    /// its result — mirrors `brain_api::AuditEventKind::
    /// PermissionDecision`'s own shape deliberately.
    AuthorizationDecision { server: String, tool_name: String, allowed: bool },
    /// The specific reason an authorization check denied a call — mirrors
    /// `brain_api::AuditEventKind::DeniedAction`'s own shape; emitted
    /// alongside `AuthorizationDecision { allowed: false, .. }`, same
    /// pairing convention `brain_api::audit::record_permission_decision`
    /// already uses.
    AuthorizationDenied { server: String, tool_name: String, reason: DenialReason },
    /// An invocation attempt was MADE against `server`/`tool_name` —
    /// distinct from authorization (a tool can be authorized and never
    /// actually invoked, or invocation can be attempted and THEN denied
    /// at this layer too if authorization is re-checked at the call
    /// site). `invocation_id` is minted fresh per attempt, independent of
    /// `proposal_id`-shaped identifiers elsewhere in this codebase — this
    /// crate has no concept of a proposal at all.
    InvocationAttempted { invocation_id: String, server: String, tool_name: String },
    /// The terminal protocol-level outcome of exactly one
    /// `InvocationAttempted` (linked by `invocation_id`).
    InvocationOutcome { invocation_id: String, outcome: InvocationOutcomeKind },
    /// The external side effect of a `Write`/`Execute`-classified
    /// invocation could not be confirmed either way — the explicit,
    /// named "I do not know" state this schema requires (never silently
    /// folded into `InvocationOutcomeKind::ProtocolError` or omitted):
    /// the local process observed the FAILURE MODE named by `reason`
    /// (e.g. the connection dropped mid-call, or the process was
    /// killed) but has no way to know whether the external server's own
    /// side effect (if any) actually completed, partially completed, or
    /// never started. A reader must never interpret an ABSENCE of this
    /// event as "no doubt exists" — only its PRESENCE means doubt was
    /// recorded; this schema makes no claim about unobserved failure
    /// modes this task's own code never detects.
    ExternalSideEffectUnknown { invocation_id: String, server: String, tool_name: String, reason: UnknownOutcomeReason },
}

/// Mirrors `brain_api::CommitOutcomeKind`'s own closed-category-not-raw-
/// error-text discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationOutcomeKind {
    Connected,
    Disabled,
    Timeout,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryOutcomeKind {
    Discovered,
    Timeout,
    Failed,
}

/// A content-free mirror of `mcp_client::ToolClassification` — this
/// crate does not depend on `mcp-client`'s own enum directly inside the
/// schema (keeping the persisted wire format independent of that crate's
/// internal type so a schema reader never needs `mcp-client` itself to
/// parse a stored event), but every variant name matches it exactly and
/// a `From<mcp_client::ToolClassification>` conversion is provided
/// in this crate's own worker-facing code (not this module) for callers
/// that do have `mcp_client::ToolClassification` in hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolClassificationLabel {
    Read,
    Write,
    Execute,
}

/// A content-free mirror of `mcp_client::Sensitivity`, same rationale as
/// [`ToolClassificationLabel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityLabel {
    Public,
    Internal,
    Sensitive,
}

/// A coarse, content-free bucket for why an authorization check denied a
/// call — mirrors `brain_api::CommitFailureCategory`'s own "one variant
/// per real decision path, never a raw error message" discipline, here
/// mapped against `mcp_client::ApprovalPolicy`'s own 4 variants' real
/// denial reasons (see `mcp_client::permission::authorize`): `Deny` is
/// unconditional; `AskApprovalRequired` always denies through
/// `authorize` alone; `AllowReadOnly` denies a non-`Read` classification;
/// `AllowScopedExecution` denies on a network or filesystem scope
/// mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenialReason {
    UnconditionalDeny,
    ApprovalRequiredButNotGranted,
    ClassificationNotPermitted,
    NetworkScopeExceeded,
    FilesystemScopeExceeded,
}

/// The terminal classification of one invocation attempt's protocol-level
/// result — mirrors `mcp_client::InvocationOutcome`'s own categories
/// (this crate does not import that enum into the schema itself, same
/// "content-free, schema-reader-independent" rationale as
/// [`ToolClassificationLabel`]) plus [`InvocationOutcomeKind::Cancelled`],
/// which `mcp_client::InvocationOutcome` has no equivalent for today
/// (this worker's own cancellation-safety requirement, Phase 3 item 4,
/// is what introduces a genuine "the caller's own future was dropped
/// before the protocol call finished" case that `invoke.rs`'s own
/// synchronous return-path enum never needed to represent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationOutcomeKind {
    Success,
    Denied,
    Timeout,
    MalformedResponse,
    ConnectionFailed,
    /// The invocation's own durable pre-registration/attempt was
    /// accepted, but the caller's own future was dropped (cancelled)
    /// before the protocol call could resolve to any of the other
    /// outcomes above — this is a LOCAL fact (this process stopped
    /// waiting) and makes no claim about the external side effect; see
    /// [`McpClientEventKind::ExternalSideEffectUnknown`] for that
    /// separate, explicit concern.
    Cancelled,
}

/// Why an external side effect's outcome is unknown — a closed,
/// content-free bucket, never a raw error message (same discipline as
/// [`DenialReason`]/`CommitFailureCategory`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownOutcomeReason {
    /// The transport/connection failed while a call was in flight, after
    /// the request may already have reached the external server.
    ConnectionLostMidCall,
    /// The local process observed a timeout while a call was in flight —
    /// the external server may have already started, completed, or never
    /// received the request; this local process cannot distinguish
    /// those cases from a bare timeout alone.
    TimeoutMidCall,
    /// The calling future was cancelled (dropped) while a call was in
    /// flight, for the same reason as [`InvocationOutcomeKind::Cancelled`]
    /// but specifically for a `Write`/`Execute`-classified tool, where
    /// "the local wait stopped" also means "the external side effect's
    /// completion can never be confirmed by this process."
    CancelledMidCall,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_variants() -> Vec<McpClientEventKind> {
        vec![
            McpClientEventKind::RegistrationAttempted { server: "s".into() },
            McpClientEventKind::RegistrationOutcome { server: "s".into(), outcome: RegistrationOutcomeKind::Connected },
            McpClientEventKind::RegistrationOutcome { server: "s".into(), outcome: RegistrationOutcomeKind::Disabled },
            McpClientEventKind::RegistrationOutcome { server: "s".into(), outcome: RegistrationOutcomeKind::Timeout },
            McpClientEventKind::RegistrationOutcome { server: "s".into(), outcome: RegistrationOutcomeKind::Failed },
            McpClientEventKind::DiscoveryAttempted { server: "s".into() },
            McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: DiscoveryOutcomeKind::Discovered },
            McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: DiscoveryOutcomeKind::Timeout },
            McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: DiscoveryOutcomeKind::Failed },
            McpClientEventKind::ToolClassified { server: "s".into(), tool_name: "t".into(), classification: ToolClassificationLabel::Execute, sensitivity: SensitivityLabel::Sensitive },
            McpClientEventKind::AuthorizationDecision { server: "s".into(), tool_name: "t".into(), allowed: true },
            McpClientEventKind::AuthorizationDecision { server: "s".into(), tool_name: "t".into(), allowed: false },
            McpClientEventKind::AuthorizationDenied { server: "s".into(), tool_name: "t".into(), reason: DenialReason::UnconditionalDeny },
            McpClientEventKind::AuthorizationDenied { server: "s".into(), tool_name: "t".into(), reason: DenialReason::ApprovalRequiredButNotGranted },
            McpClientEventKind::AuthorizationDenied { server: "s".into(), tool_name: "t".into(), reason: DenialReason::ClassificationNotPermitted },
            McpClientEventKind::AuthorizationDenied { server: "s".into(), tool_name: "t".into(), reason: DenialReason::NetworkScopeExceeded },
            McpClientEventKind::AuthorizationDenied { server: "s".into(), tool_name: "t".into(), reason: DenialReason::FilesystemScopeExceeded },
            McpClientEventKind::InvocationAttempted { invocation_id: "i1".into(), server: "s".into(), tool_name: "t".into() },
            McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: InvocationOutcomeKind::Success },
            McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: InvocationOutcomeKind::Denied },
            McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: InvocationOutcomeKind::Timeout },
            McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: InvocationOutcomeKind::MalformedResponse },
            McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: InvocationOutcomeKind::ConnectionFailed },
            McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: InvocationOutcomeKind::Cancelled },
            McpClientEventKind::ExternalSideEffectUnknown { invocation_id: "i1".into(), server: "s".into(), tool_name: "t".into(), reason: UnknownOutcomeReason::ConnectionLostMidCall },
            McpClientEventKind::ExternalSideEffectUnknown { invocation_id: "i1".into(), server: "s".into(), tool_name: "t".into(), reason: UnknownOutcomeReason::TimeoutMidCall },
            McpClientEventKind::ExternalSideEffectUnknown { invocation_id: "i1".into(), server: "s".into(), tool_name: "t".into(), reason: UnknownOutcomeReason::CancelledMidCall },
        ]
    }

    // --- content-free regression guard, mirroring brain_api::audit's own ---

    #[test]
    fn no_variant_serializes_a_note_body_or_query_text_shaped_field() {
        for kind in all_variants() {
            let json = serde_json::to_string(&kind).unwrap();
            assert!(json.len() < 300, "variant serialized unexpectedly large, suggesting a content-bearing field crept in: {json}");
            assert!(!json.contains('\n'), "a multi-line value suggests prose/content leaked into an audit event: {json}");
        }
    }

    #[test]
    fn every_variant_round_trips_through_serde_json_exactly() {
        for kind in all_variants() {
            let json = serde_json::to_string(&kind).unwrap();
            let decoded: McpClientEventKind = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, kind, "round trip through JSON must reproduce the exact original variant, got {json}");
        }
    }

    #[test]
    fn event_constructor_stamps_the_current_schema_version_timestamp_and_correlation_id() {
        let e = event(McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "2026-01-01T00:00:00Z", "corr-1");
        assert_eq!(e.schema_version, MCP_CLIENT_EVENT_FORMAT_VERSION);
        assert_eq!(e.timestamp, "2026-01-01T00:00:00Z");
        assert_eq!(e.correlation_id, "corr-1");
    }

    #[test]
    fn invocation_attempted_and_invocation_outcome_are_correlated_by_invocation_id_not_server_or_tool_name_alone() {
        // Mirrors brain_api::audit's own
        // approval_attempted_and_commit_outcome_are_correlated_by_attempt_id_not_proposal_id
        // test: two separate invocations of the SAME tool on the SAME
        // server must be distinguishable by invocation_id alone.
        let a1 = McpClientEventKind::InvocationAttempted { invocation_id: "inv-1".into(), server: "s".into(), tool_name: "t".into() };
        let a2 = McpClientEventKind::InvocationAttempted { invocation_id: "inv-2".into(), server: "s".into(), tool_name: "t".into() };
        assert_ne!(a1, a2);

        let o1 = McpClientEventKind::InvocationOutcome { invocation_id: "inv-1".into(), outcome: InvocationOutcomeKind::Success };
        let o2 = McpClientEventKind::InvocationOutcome { invocation_id: "inv-2".into(), outcome: InvocationOutcomeKind::Timeout };
        if let (McpClientEventKind::InvocationAttempted { invocation_id: i1, .. }, McpClientEventKind::InvocationOutcome { invocation_id: o1id, .. }) = (&a1, &o1) {
            assert_eq!(i1, o1id);
        } else {
            panic!("pattern must match");
        }
        if let (McpClientEventKind::InvocationAttempted { invocation_id: i2, .. }, McpClientEventKind::InvocationOutcome { invocation_id: o2id, .. }) = (&a2, &o2) {
            assert_eq!(i2, o2id);
        } else {
            panic!("pattern must match");
        }
    }

    #[test]
    fn invocation_outcome_never_carries_a_server_or_tool_name_field_only_invocation_id() {
        // Structural guard mirroring brain_api::audit's own
        // commit_outcome_never_carries_a_proposal_id_field_only_attempt_id:
        // a reader must never mistake "this invocation_id exists" for
        // "this specific outcome happened to this specific tool" without
        // first finding the matching InvocationAttempted record.
        let json = serde_json::to_string(&McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: InvocationOutcomeKind::Success }).unwrap();
        assert!(!json.contains("server"), "InvocationOutcome's own serialized shape must never carry a server field, got {json}");
        assert!(!json.contains("tool_name"), "InvocationOutcome's own serialized shape must never carry a tool_name field, got {json}");
    }

    #[test]
    fn external_side_effect_unknown_is_a_distinct_variant_never_collapsed_into_invocation_outcome() {
        // The explicit "unknown outcome" requirement: this variant must
        // be representable and distinguishable from every InvocationOutcomeKind
        // case, never silently mapped onto one of them.
        let unknown = McpClientEventKind::ExternalSideEffectUnknown { invocation_id: "i1".into(), server: "s".into(), tool_name: "t".into(), reason: UnknownOutcomeReason::ConnectionLostMidCall };
        let json = serde_json::to_string(&unknown).unwrap();
        assert!(json.contains("external_side_effect_unknown"));
        for outcome_kind in [
            InvocationOutcomeKind::Success,
            InvocationOutcomeKind::Denied,
            InvocationOutcomeKind::Timeout,
            InvocationOutcomeKind::MalformedResponse,
            InvocationOutcomeKind::ConnectionFailed,
            InvocationOutcomeKind::Cancelled,
        ] {
            let as_outcome = McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: outcome_kind };
            assert_ne!(unknown, as_outcome, "ExternalSideEffectUnknown must never equal any InvocationOutcomeKind-shaped event");
        }
    }

    #[test]
    fn registration_and_discovery_events_identify_the_server_but_never_a_transport_command_line() {
        // Content-free discipline, specific to registration: `server` is
        // always a short name, never the full ChildProcess{command, args}
        // shape (which could carry caller-supplied content, e.g. a
        // filesystem path argument).
        let json = serde_json::to_string(&McpClientEventKind::RegistrationAttempted { server: "my-server".into() }).unwrap();
        assert!(!json.contains("command"), "RegistrationAttempted must never carry a transport command field, got {json}");
        assert!(!json.contains("args"), "RegistrationAttempted must never carry a transport args field, got {json}");
    }
}
