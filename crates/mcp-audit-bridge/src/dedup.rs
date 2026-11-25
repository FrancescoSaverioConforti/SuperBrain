//! M7.3 correction: before re-appending any `SourceRecordIdentity`, the
//! bridge must determine reliably whether it has already been
//! materialized in the destination (Brain) audit chain. The frozen
//! correction explicitly rejects a tail-only/"recent records" search —
//! the destination chain can contain later, interleaved events (ordinary
//! Brain activity unrelated to this bridge) that push an already-
//! imported bridge fact away from the tail. So this module scans the
//! ENTIRE verified destination chain, every time, building a complete
//! set of already-imported identities — no second persisted index, no
//! optimization, correctness first (explicitly deferred by the same
//! decision).
//!
//! The destination audit chain is the ONE authoritative source of truth
//! for "was this source record already imported" — the bridge
//! checkpoint (`crate::checkpoint`) is an accelerator only, never trusted
//! on its own for this question.

use crate::identity::SourceRecordIdentity;

/// A `SourceRecordIdentity` found more than once in the destination chain
/// — never normalized away silently; this is itself evidence that
/// something is wrong (a prior non-idempotent append, external tampering,
/// or a bug in this bridge), and must be surfaced to the caller rather
/// than quietly collapsed into one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationAnomaly {
    pub identity: SourceRecordIdentity,
    pub occurrence_count: usize,
}

/// The result of one full destination-chain scan: every distinct
/// `SourceRecordIdentity` already present (for O(1) membership checks by
/// the caller), plus any anomalies found. An anomaly does not stop the
/// scan early — the full set is still built and returned alongside the
/// anomaly list, so a caller can decide how to proceed with complete
/// information rather than a truncated view.
#[derive(Debug, Clone, Default)]
pub struct DestinationScanResult {
    pub imported: std::collections::HashSet<SourceRecordIdentity>,
    pub anomalies: Vec<DestinationAnomaly>,
}

/// Scans every verified record of the destination chain
/// (`verified_records`, as produced by `audit_chain_core::scan_chain`
/// over the destination's own raw bytes — this function performs no I/O
/// itself; a caller reads the destination vault's raw bytes and runs
/// `scan_chain` before calling this), decodes each one via
/// `brain_api::parse_event`, and collects the `SourceRecordIdentity` of
/// every `McpBridgeEvent`/`McpBridgeExternalSideEffectUnknown` found.
///
/// Non-bridge destination events (the other 11 `AuditEventKind`
/// variants — ordinary Brain activity) are skipped, not an error: this
/// is precisely the "interleaved events" case the frozen correction
/// requires this function to tolerate correctly.
///
/// A record that fails to decode via `parse_event` (should not happen for
/// a chain that already passed `scan_chain`, since the envelope's own
/// `event_json` is validated as well-formed JSON at that stage, but the
/// TYPED decode here is a separate, later step) is treated the same as a
/// non-bridge event: skipped, never treated as an anomaly by itself — it
/// is simply not one of this bridge's own events.
pub fn scan_destination_for_imported_identities(verified_records: &[audit_chain_core::ChainRecord]) -> DestinationScanResult {
    let mut counts: std::collections::HashMap<SourceRecordIdentity, usize> = std::collections::HashMap::new();

    for record in verified_records {
        let Ok(event) = audit_chain_core::parse_event::<brain_api::AuditEvent>(&record.content_bytes) else {
            continue;
        };
        if let Some(identity) = identity_of(&event.kind) {
            *counts.entry(identity).or_insert(0) += 1;
        }
    }

    let mut imported = std::collections::HashSet::new();
    let mut anomalies = Vec::new();
    for (identity, occurrence_count) in counts {
        if occurrence_count > 1 {
            anomalies.push(DestinationAnomaly { identity: identity.clone(), occurrence_count });
        }
        imported.insert(identity);
    }

    DestinationScanResult { imported, anomalies }
}

/// Extracts a `SourceRecordIdentity` from a destination `AuditEventKind`,
/// if (and only if) it is one of the two M7 bridge variants — every other
/// variant has no such provenance and returns `None`.
fn identity_of(kind: &brain_api::AuditEventKind) -> Option<SourceRecordIdentity> {
    use brain_api::AuditEventKind as K;
    match kind {
        K::McpBridgeEvent { source_log_id, source_sequence, source_record_hash, .. } | K::McpBridgeExternalSideEffectUnknown { source_log_id, source_sequence, source_record_hash, .. } => {
            Some(SourceRecordIdentity { source_log_id: parse_source_log_id(source_log_id), sequence: *source_sequence, record_hash: source_record_hash.clone() })
        }
        _ => None,
    }
}

fn parse_source_log_id(raw: &str) -> crate::identity::SourceLogId {
    // A destination record's own `source_log_id` string was produced by
    // this same bridge's `translate` (via `SourceLogId::to_string`) at
    // append time, so it is always a valid UUID in practice; a malformed
    // value here would mean the destination chain itself was tampered
    // with or corrupted in a way `scan_chain`'s hash verification did not
    // already catch, which this function has no separate recovery for —
    // it is deliberately NOT silently substituted with a fabricated
    // identity (that would risk masking a real anomaly as an ordinary
    // miss), so this falls back to a fixed nil UUID, which can never
    // equal a genuinely minted `SourceLogId::generate()` value and so can
    // only ever manifest as "not found" to a caller, never as a false
    // match.
    uuid::Uuid::parse_str(raw).map(crate::identity::SourceLogId).unwrap_or_else(|_| crate::identity::SourceLogId(uuid::Uuid::nil()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::SourceLogId;
    use audit_chain_core::{serialize_record, GENESIS_HASH};
    use brain_api::{AuditEventKind, McpBridgePhase};

    fn bridge_event(log_id: SourceLogId, sequence: u64, record_hash: &str) -> AuditEventKind {
        AuditEventKind::McpBridgeEvent {
            source_phase: McpBridgePhase::Invocation,
            source_kind: "invocation_attempted".into(),
            server: Some("s".into()),
            tool_name: Some("t".into()),
            invocation_id: Some("i1".into()),
            outcome: None,
            source_log_id: log_id.to_string(),
            source_sequence: sequence,
            source_record_hash: record_hash.to_string(),
        }
    }

    fn build_destination_chain(events: &[brain_api::AuditEvent]) -> Vec<audit_chain_core::ChainRecord> {
        let mut records = Vec::new();
        let mut prev_hash = GENESIS_HASH.to_string();
        for (i, event) in events.iter().enumerate() {
            let (_, record_hash) = serialize_record(event, i as u64, &prev_hash);
            let content_bytes = serde_json::to_vec(event).unwrap();
            records.push(audit_chain_core::ChainRecord { sequence: i as u64, prev_record_hash: prev_hash.clone(), record_hash: record_hash.clone(), content_bytes });
            prev_hash = record_hash;
        }
        records
    }

    fn event(kind: AuditEventKind) -> brain_api::AuditEvent {
        brain_api::event(kind, "2026-01-01T00:00:00Z", uuid::Uuid::new_v4())
    }

    #[test]
    fn an_identity_present_once_in_the_destination_is_reported_imported_with_no_anomaly() {
        let log_id = SourceLogId::generate();
        let chain = build_destination_chain(&[event(bridge_event(log_id, 0, &"a".repeat(64)))]);
        let result = scan_destination_for_imported_identities(&chain);
        assert_eq!(result.imported.len(), 1);
        assert!(result.anomalies.is_empty());
        assert!(result.imported.contains(&SourceRecordIdentity { source_log_id: log_id, sequence: 0, record_hash: "a".repeat(64) }));
    }

    #[test]
    fn interleaved_non_bridge_events_do_not_hide_an_already_imported_identity_even_when_appended_after_it() {
        let log_id = SourceLogId::generate();
        let identity_event = event(bridge_event(log_id, 0, &"a".repeat(64)));
        let unrelated_1 = event(AuditEventKind::WriteMemory { proposal_id: "p1".into() });
        let unrelated_2 = event(AuditEventKind::ToolCall { tool: "search".into(), allowed: true });
        // The imported fact sits FAR from the tail -- exactly the case a
        // tail-only scan would miss and this full-chain scan must not.
        let chain = build_destination_chain(&[identity_event, unrelated_1, unrelated_2]);
        let result = scan_destination_for_imported_identities(&chain);
        assert!(result.imported.contains(&SourceRecordIdentity { source_log_id: log_id, sequence: 0, record_hash: "a".repeat(64) }));
        assert!(result.anomalies.is_empty());
    }

    #[test]
    fn the_same_identity_appended_twice_is_reported_as_an_anomaly_not_silently_normalized() {
        let log_id = SourceLogId::generate();
        let dup = bridge_event(log_id, 0, &"a".repeat(64));
        let chain = build_destination_chain(&[event(dup.clone()), event(dup)]);
        let result = scan_destination_for_imported_identities(&chain);
        assert_eq!(result.anomalies.len(), 1);
        assert_eq!(result.anomalies[0].occurrence_count, 2);
        assert_eq!(result.anomalies[0].identity, SourceRecordIdentity { source_log_id: log_id, sequence: 0, record_hash: "a".repeat(64) });
    }

    #[test]
    fn an_identity_never_imported_is_absent_from_the_result() {
        let log_id = SourceLogId::generate();
        let chain = build_destination_chain(&[event(bridge_event(log_id, 0, &"a".repeat(64)))]);
        let result = scan_destination_for_imported_identities(&chain);
        assert!(!result.imported.contains(&SourceRecordIdentity { source_log_id: log_id, sequence: 1, record_hash: "z".repeat(64) }));
    }

    #[test]
    fn an_empty_destination_chain_yields_no_imports_and_no_anomalies() {
        let result = scan_destination_for_imported_identities(&[]);
        assert!(result.imported.is_empty());
        assert!(result.anomalies.is_empty());
    }
}
