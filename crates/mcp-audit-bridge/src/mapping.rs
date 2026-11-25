//! M7.2: the exhaustive, approved mapping from
//! `mcp_client_audit_worker::McpClientEventKind` (10 real variants) to
//! `brain_api::AuditEventKind::{McpBridgeEvent,
//! McpBridgeExternalSideEffectUnknown}`.
//!
//! **Zero events dropped, zero fields fabricated** (the frozen M7
//! requirement): every one of the 10 source variants below produces
//! exactly one destination event (the `AuthorizationDecision`/
//! `AuthorizationDenied` pair -- already two separate source events --
//! produces two separate destination events, matching the source 1:1,
//! not one-to-many). A field absent from the source variant is always
//! `None` in the destination, never reconstructed from another event or
//! guessed.
//!
//! **Exhaustive match, no catch-all** (the frozen design's own
//! requirement): `translate_kind` below matches every
//! `McpClientEventKind` variant by name. Adding an 11th variant to that
//! enum in the future will fail THIS match at compile time, forcing an
//! explicit decision about how to extend this mapping -- never a silent
//! `_ => ...` that could let a new, unreviewed source variant pass
//! through unmapped or mis-mapped.
//!
//! **`ToolClassified` is not schema verification** (the frozen M7
//! correction): row 5 below maps `ToolClassified` to its own
//! `McpBridgePhase::ToolClassification` phase, carrying exactly
//! classification/sensitivity -- this module never emits, and must
//! never be changed to emit, any event implying a schema-verification
//! outcome was recorded. No such source event exists today; see this
//! crate's own design notes for why that gap is intentionally left open,
//! not papered over here.

use brain_api::AuditEventKind;
use mcp_client_audit_worker::McpClientEventKind;

use crate::identity::SourceRecordIdentity;

/// Translates one verified source record's `kind` into the `AuditEventKind`
/// the bridge will append to the destination chain, embedding `identity`
/// as this event's own provenance. This is the ONLY function in this
/// crate that constructs an `AuditEventKind::McpBridgeEvent` or
/// `McpBridgeExternalSideEffectUnknown` -- every other module calls
/// through this one, so the mapping table lives in exactly one place.
pub fn translate(kind: &McpClientEventKind, identity: &SourceRecordIdentity) -> AuditEventKind {
    let source_log_id = identity.source_log_id.to_string();
    let source_sequence = identity.sequence;
    let source_record_hash = identity.record_hash.clone();

    use brain_api::McpBridgePhase as Phase;
    use McpClientEventKind as Src;

    match kind {
        Src::RegistrationAttempted { server } => AuditEventKind::McpBridgeEvent {
            source_phase: Phase::Registration,
            source_kind: "registration_attempted".to_string(),
            server: Some(server.clone()),
            tool_name: None,
            invocation_id: None,
            outcome: None,
            source_log_id,
            source_sequence,
            source_record_hash,
        },
        Src::RegistrationOutcome { server, outcome } => AuditEventKind::McpBridgeEvent {
            source_phase: Phase::Registration,
            source_kind: "registration_outcome".to_string(),
            server: Some(server.clone()),
            tool_name: None,
            invocation_id: None,
            outcome: Some(registration_outcome_label(*outcome).to_string()),
            source_log_id,
            source_sequence,
            source_record_hash,
        },
        Src::DiscoveryAttempted { server } => AuditEventKind::McpBridgeEvent {
            source_phase: Phase::Discovery,
            source_kind: "discovery_attempted".to_string(),
            server: Some(server.clone()),
            tool_name: None,
            invocation_id: None,
            outcome: None,
            source_log_id,
            source_sequence,
            source_record_hash,
        },
        Src::DiscoveryOutcome { server, outcome } => AuditEventKind::McpBridgeEvent {
            source_phase: Phase::Discovery,
            source_kind: "discovery_outcome".to_string(),
            server: Some(server.clone()),
            tool_name: None,
            invocation_id: None,
            outcome: Some(discovery_outcome_label(*outcome).to_string()),
            source_log_id,
            source_sequence,
            source_record_hash,
        },
        // Row 5: ToolClassified is classification/sensitivity ONLY --
        // never a stand-in for schema verification (see this module's own
        // top doc comment).
        Src::ToolClassified { server, tool_name, classification, sensitivity } => AuditEventKind::McpBridgeEvent {
            source_phase: Phase::ToolClassification,
            source_kind: "tool_classified".to_string(),
            server: Some(server.clone()),
            tool_name: Some(tool_name.clone()),
            invocation_id: None,
            outcome: Some(format!("{}:{}", classification_label(*classification), sensitivity_label(*sensitivity))),
            source_log_id,
            source_sequence,
            source_record_hash,
        },
        Src::AuthorizationDecision { server, tool_name, allowed } => AuditEventKind::McpBridgeEvent {
            source_phase: Phase::Authorization,
            source_kind: "authorization_decision".to_string(),
            server: Some(server.clone()),
            tool_name: Some(tool_name.clone()),
            invocation_id: None,
            outcome: Some(if *allowed { "allowed".to_string() } else { "denied".to_string() }),
            source_log_id,
            source_sequence,
            source_record_hash,
        },
        Src::AuthorizationDenied { server, tool_name, reason } => AuditEventKind::McpBridgeEvent {
            source_phase: Phase::Authorization,
            source_kind: "authorization_denied".to_string(),
            server: Some(server.clone()),
            tool_name: Some(tool_name.clone()),
            invocation_id: None,
            outcome: Some(denial_reason_label(*reason).to_string()),
            source_log_id,
            source_sequence,
            source_record_hash,
        },
        Src::InvocationAttempted { invocation_id, server, tool_name } => AuditEventKind::McpBridgeEvent {
            source_phase: Phase::Invocation,
            source_kind: "invocation_attempted".to_string(),
            server: Some(server.clone()),
            tool_name: Some(tool_name.clone()),
            invocation_id: Some(invocation_id.clone()),
            outcome: None,
            source_log_id,
            source_sequence,
            source_record_hash,
        },
        // Row 9: the source variant itself never carries server/tool_name
        // (verified directly in mcp-client-audit-worker's own test
        // `invocation_outcome_never_carries_a_server_or_tool_name_field_only_invocation_id`)
        // -- never reconstructed here from another event.
        Src::InvocationOutcome { invocation_id, outcome } => AuditEventKind::McpBridgeEvent {
            source_phase: Phase::Invocation,
            source_kind: "invocation_outcome".to_string(),
            server: None,
            tool_name: None,
            invocation_id: Some(invocation_id.clone()),
            outcome: Some(invocation_outcome_label(*outcome).to_string()),
            source_log_id,
            source_sequence,
            source_record_hash,
        },
        // Row 10: kept as its own distinct destination variant, never
        // folded into McpBridgeEvent's generic `outcome` field -- the
        // explicit "unknown outcome" security signal must stay visible
        // without inspecting a shared field among many less sensitive
        // ones.
        Src::ExternalSideEffectUnknown { invocation_id, server, tool_name, reason } => AuditEventKind::McpBridgeExternalSideEffectUnknown {
            invocation_id: invocation_id.clone(),
            server: server.clone(),
            tool_name: tool_name.clone(),
            reason: unknown_outcome_reason_label(*reason).to_string(),
            source_log_id,
            source_sequence,
            source_record_hash,
        },
    }
}

fn registration_outcome_label(outcome: mcp_client_audit_worker::RegistrationOutcomeKind) -> &'static str {
    use mcp_client_audit_worker::RegistrationOutcomeKind as O;
    match outcome {
        O::Connected => "connected",
        O::Disabled => "disabled",
        O::Timeout => "timeout",
        O::Failed => "failed",
    }
}

fn discovery_outcome_label(outcome: mcp_client_audit_worker::DiscoveryOutcomeKind) -> &'static str {
    use mcp_client_audit_worker::DiscoveryOutcomeKind as O;
    match outcome {
        O::Discovered => "discovered",
        O::Timeout => "timeout",
        O::Failed => "failed",
    }
}

fn classification_label(classification: mcp_client_audit_worker::ToolClassificationLabel) -> &'static str {
    use mcp_client_audit_worker::ToolClassificationLabel as C;
    match classification {
        C::Read => "read",
        C::Write => "write",
        C::Execute => "execute",
    }
}

fn sensitivity_label(sensitivity: mcp_client_audit_worker::SensitivityLabel) -> &'static str {
    use mcp_client_audit_worker::SensitivityLabel as S;
    match sensitivity {
        S::Public => "public",
        S::Internal => "internal",
        S::Sensitive => "sensitive",
    }
}

fn denial_reason_label(reason: mcp_client_audit_worker::DenialReason) -> &'static str {
    use mcp_client_audit_worker::DenialReason as R;
    match reason {
        R::UnconditionalDeny => "unconditional_deny",
        R::ApprovalRequiredButNotGranted => "approval_required_but_not_granted",
        R::ClassificationNotPermitted => "classification_not_permitted",
        R::NetworkScopeExceeded => "network_scope_exceeded",
        R::FilesystemScopeExceeded => "filesystem_scope_exceeded",
    }
}

fn invocation_outcome_label(outcome: mcp_client_audit_worker::InvocationOutcomeKind) -> &'static str {
    use mcp_client_audit_worker::InvocationOutcomeKind as O;
    match outcome {
        O::Success => "success",
        O::Denied => "denied",
        O::Timeout => "timeout",
        O::MalformedResponse => "malformed_response",
        O::ConnectionFailed => "connection_failed",
        O::Cancelled => "cancelled",
    }
}

fn unknown_outcome_reason_label(reason: mcp_client_audit_worker::UnknownOutcomeReason) -> &'static str {
    use mcp_client_audit_worker::UnknownOutcomeReason as R;
    match reason {
        R::ConnectionLostMidCall => "connection_lost_mid_call",
        R::TimeoutMidCall => "timeout_mid_call",
        R::CancelledMidCall => "cancelled_mid_call",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::SourceLogId;
    use mcp_client_audit_worker::*;

    fn identity() -> SourceRecordIdentity {
        SourceRecordIdentity { source_log_id: SourceLogId::generate(), sequence: 7, record_hash: "h".repeat(64) }
    }

    fn all_source_variants() -> Vec<McpClientEventKind> {
        vec![
            McpClientEventKind::RegistrationAttempted { server: "s".into() },
            McpClientEventKind::RegistrationOutcome { server: "s".into(), outcome: RegistrationOutcomeKind::Connected },
            McpClientEventKind::DiscoveryAttempted { server: "s".into() },
            McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: DiscoveryOutcomeKind::Discovered },
            McpClientEventKind::ToolClassified { server: "s".into(), tool_name: "t".into(), classification: ToolClassificationLabel::Execute, sensitivity: SensitivityLabel::Sensitive },
            McpClientEventKind::AuthorizationDecision { server: "s".into(), tool_name: "t".into(), allowed: true },
            McpClientEventKind::AuthorizationDenied { server: "s".into(), tool_name: "t".into(), reason: DenialReason::UnconditionalDeny },
            McpClientEventKind::InvocationAttempted { invocation_id: "i1".into(), server: "s".into(), tool_name: "t".into() },
            McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: InvocationOutcomeKind::Success },
            McpClientEventKind::ExternalSideEffectUnknown { invocation_id: "i1".into(), server: "s".into(), tool_name: "t".into(), reason: UnknownOutcomeReason::ConnectionLostMidCall },
        ]
    }

    #[test]
    fn every_one_of_the_10_real_source_variants_produces_exactly_one_destination_event_with_provenance() {
        let id = identity();
        for kind in all_source_variants() {
            let translated = translate(&kind, &id);
            match &translated {
                AuditEventKind::McpBridgeEvent { source_log_id, source_sequence, source_record_hash, .. } => {
                    assert_eq!(*source_log_id, id.source_log_id.to_string());
                    assert_eq!(*source_sequence, id.sequence);
                    assert_eq!(*source_record_hash, id.record_hash);
                }
                AuditEventKind::McpBridgeExternalSideEffectUnknown { source_log_id, source_sequence, source_record_hash, .. } => {
                    assert_eq!(*source_log_id, id.source_log_id.to_string());
                    assert_eq!(*source_sequence, id.sequence);
                    assert_eq!(*source_record_hash, id.record_hash);
                }
                other => panic!("translate must only ever produce one of the two M7 bridge variants, got {other:?}"),
            }
        }
    }

    #[test]
    fn external_side_effect_unknown_is_never_translated_into_the_generic_mcp_bridge_event_shape() {
        let id = identity();
        let kind = McpClientEventKind::ExternalSideEffectUnknown { invocation_id: "i1".into(), server: "s".into(), tool_name: "t".into(), reason: UnknownOutcomeReason::TimeoutMidCall };
        let translated = translate(&kind, &id);
        assert!(matches!(translated, AuditEventKind::McpBridgeExternalSideEffectUnknown { .. }), "got {translated:?}");
    }

    #[test]
    fn tool_classified_never_produces_a_source_kind_naming_schema_verification() {
        let id = identity();
        let kind = McpClientEventKind::ToolClassified { server: "s".into(), tool_name: "t".into(), classification: ToolClassificationLabel::Read, sensitivity: SensitivityLabel::Public };
        let translated = translate(&kind, &id);
        let json = serde_json::to_string(&translated).unwrap();
        assert!(!json.to_lowercase().contains("schema"), "ToolClassified must never be translated into anything implying schema verification, got {json}");
    }

    #[test]
    fn invocation_outcome_never_carries_a_server_or_tool_name_in_its_translation() {
        let id = identity();
        let kind = McpClientEventKind::InvocationOutcome { invocation_id: "i1".into(), outcome: InvocationOutcomeKind::Timeout };
        let translated = translate(&kind, &id);
        if let AuditEventKind::McpBridgeEvent { server, tool_name, .. } = translated {
            assert_eq!(server, None);
            assert_eq!(tool_name, None);
        } else {
            panic!("expected McpBridgeEvent");
        }
    }

    #[test]
    fn authorization_decision_and_authorization_denied_map_to_two_separate_destination_events_not_one() {
        let id = identity();
        let decision = translate(&McpClientEventKind::AuthorizationDecision { server: "s".into(), tool_name: "t".into(), allowed: false }, &id);
        let denied = translate(&McpClientEventKind::AuthorizationDenied { server: "s".into(), tool_name: "t".into(), reason: DenialReason::NetworkScopeExceeded }, &id);
        assert_ne!(decision, denied, "the source pair must remain two distinct destination events, never merged into one");
    }
}
