//! M7.6.2: the full recovery matrix — a pure classification function
//! over the REAL state (manifest entry + Active segment's own verified
//! records + ledger's own verified records), never a guess, never
//! trusting the manifest alone. `crate::retention::retire_segment`
//! already performs the RESUME behavior for the two "already durably
//! in progress" cases inline (resuming from `RetentionAuthorized`, and
//! skipping an already-appended `RetentionIntentAttempted`); this
//! module's own job is to make that same classification independently
//! inspectable and testable — the exact mapping the frozen recovery
//! matrix names, as a value a caller (or `crate::verifier`) can query
//! without having to run a real retention step.

use crate::domain::SegmentId;

/// One target segment's recovery classification — mirrors the frozen
/// recovery matrix's own rows exactly, each a possible real, observed
/// combination of (intent record, authorized record, ledger receipt,
/// payload presence, manifest's own cached `retained_payload_present`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryState {
    /// No intent, no authorization, no receipt -- nothing in progress.
    NoRetentionInProgress,
    /// Phase 1 durable, phase 2/3 not yet observed, payload still
    /// present -- resume from phase 2.
    IntentWithoutReceipt,
    /// Phases 1-3 all durable, payload still present -- resume from
    /// phase 4 (delete).
    AuthorizedPayloadPresent { retention_ledger_sequence: u64 },
    /// Phases 1-3 all durable, payload already gone, manifest not yet
    /// updated -- resume from phase 5 (projection only).
    AuthorizedPayloadAbsentManifestStale { retention_ledger_sequence: u64 },
    /// Phases 1-5 all consistent -- terminal, correct state.
    RetirementComplete { retention_ledger_sequence: u64 },
    /// Payload is absent, but NO `RetentionAuthorized` record exists
    /// anywhere in the Active segment's own verified chain for this
    /// target -- tampering or malicious deletion. Never auto-repaired.
    PayloadMissingWithoutAuthorization,
    /// A `RetentionAuthorized` record exists, but its own
    /// `retention_ledger_sequence`/`retention_ledger_record_hash` does
    /// not match a real record at that exact position in the ledger's
    /// own independently verified chain -- tampering on the ledger
    /// side, the audit side, or both. Never auto-repaired.
    LedgerMismatch,
}

/// Classifies `target_segment_id`'s current recovery state from REAL,
/// independently-verified evidence — never the manifest's own cached
/// `retained_payload_present` alone (that field is consulted only as a
/// secondary signal for the "stale projection" case, never as the
/// authority for whether a retention actually happened).
pub fn classify(active_verified_records: &[audit_chain_core::ChainRecord], ledger_verified_records: &[audit_chain_core::ChainRecord], target_segment_id: &SegmentId, payload_exists: bool) -> RecoveryState {
    let intent = find_intent(active_verified_records, target_segment_id);
    let authorized = find_authorized(active_verified_records, target_segment_id);

    match (intent, authorized, payload_exists) {
        (None, None, true) => RecoveryState::NoRetentionInProgress,
        (None, None, false) => RecoveryState::PayloadMissingWithoutAuthorization,
        (Some(_), None, true) => RecoveryState::IntentWithoutReceipt,
        (Some(_), None, false) => RecoveryState::PayloadMissingWithoutAuthorization,
        (_, Some(authorized), payload_exists) => {
            if !ledger_commits_match(ledger_verified_records, authorized.retention_ledger_sequence, &authorized.retention_ledger_record_hash) {
                return RecoveryState::LedgerMismatch;
            }
            if payload_exists {
                RecoveryState::AuthorizedPayloadPresent { retention_ledger_sequence: authorized.retention_ledger_sequence }
            } else {
                RecoveryState::AuthorizedPayloadAbsentManifestStale { retention_ledger_sequence: authorized.retention_ledger_sequence }
            }
        }
    }
}

/// Distinguishes the terminal `RetirementComplete` case from
/// `AuthorizedPayloadAbsentManifestStale` using the manifest's own
/// cached `retained_payload_present` -- the ONE place this module
/// consults that field, and only as a secondary signal AFTER the real
/// chain evidence has already confirmed authorization and ledger
/// agreement; never as the sole basis for any other classification.
pub fn refine_with_manifest_projection(state: RecoveryState, manifest_says_retained_payload_present: bool) -> RecoveryState {
    match state {
        RecoveryState::AuthorizedPayloadAbsentManifestStale { retention_ledger_sequence } if !manifest_says_retained_payload_present => RecoveryState::RetirementComplete { retention_ledger_sequence },
        other => other,
    }
}

struct DecodedAuthorized {
    retention_ledger_sequence: u64,
    retention_ledger_record_hash: String,
}

fn find_intent(records: &[audit_chain_core::ChainRecord], target_segment_id: &SegmentId) -> Option<()> {
    records.iter().find_map(|r| {
        let event: brain_api::AuditEvent = audit_chain_core::parse_event(&r.content_bytes).ok()?;
        match event.kind {
            brain_api::AuditEventKind::RetentionIntentAttempted { target_segment_id: t, .. } if t == target_segment_id.as_str() => Some(()),
            _ => None,
        }
    })
}

fn find_authorized(records: &[audit_chain_core::ChainRecord], target_segment_id: &SegmentId) -> Option<DecodedAuthorized> {
    records.iter().rev().find_map(|r| {
        let event: brain_api::AuditEvent = audit_chain_core::parse_event(&r.content_bytes).ok()?;
        match event.kind {
            brain_api::AuditEventKind::RetentionAuthorized { target_segment_id: t, retention_ledger_sequence, retention_ledger_record_hash } if t == target_segment_id.as_str() => Some(DecodedAuthorized { retention_ledger_sequence, retention_ledger_record_hash }),
            _ => None,
        }
    })
}

fn ledger_commits_match(ledger_records: &[audit_chain_core::ChainRecord], expected_sequence: u64, expected_record_hash: &str) -> bool {
    ledger_records.iter().any(|r| r.sequence == expected_sequence && r.record_hash == expected_record_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_chain_core::{serialize_record, GENESIS_HASH};

    fn build_chain(events: &[brain_api::AuditEventKind]) -> Vec<audit_chain_core::ChainRecord> {
        let mut records = Vec::new();
        let mut prev = GENESIS_HASH.to_string();
        for (i, kind) in events.iter().enumerate() {
            let event = brain_api::event(kind.clone(), "2026-01-01T00:00:00Z", uuid::Uuid::new_v4());
            let (_, hash) = serialize_record(&event, i as u64, &prev);
            let content_bytes = serde_json::to_vec(&event).unwrap();
            records.push(audit_chain_core::ChainRecord { sequence: i as u64, prev_record_hash: prev.clone(), record_hash: hash.clone(), content_bytes });
            prev = hash;
        }
        records
    }

    fn target() -> SegmentId {
        SegmentId::from_index(1)
    }

    #[test]
    fn no_evidence_and_payload_present_is_no_retention_in_progress() {
        let active = build_chain(&[]);
        let ledger = build_chain(&[]);
        assert_eq!(classify(&active, &ledger, &target(), true), RecoveryState::NoRetentionInProgress);
    }

    #[test]
    fn intent_without_authorized_and_payload_present_resumes_from_phase_2() {
        let active = build_chain(&[brain_api::AuditEventKind::RetentionIntentAttempted { target_segment_id: target().to_string(), target_segment_final_sequence: 0, target_segment_final_record_hash: "a".repeat(64) }]);
        let ledger = build_chain(&[]);
        assert_eq!(classify(&active, &ledger, &target(), true), RecoveryState::IntentWithoutReceipt);
    }

    #[test]
    fn authorized_with_matching_ledger_and_payload_present_resumes_from_phase_4() {
        let receipt = crate::domain::RetentionReceipt { retired_segment_id: target(), retired_segment_final_sequence: 0, retired_segment_final_record_hash: "a".repeat(64), retirement_authorized_by: "p".into(), retired_at: "2026-01-01T00:00:00Z".into() };
        let ledger_event = brain_api::event(brain_api::AuditEventKind::McpConnection { server: "unused".into() }, "t", uuid::Uuid::new_v4());
        let _ = ledger_event; // ledger stores RetentionReceipt, not AuditEvent -- build it directly below
        let (_, ledger_hash) = serialize_record(&receipt, 0, GENESIS_HASH);
        let ledger = vec![audit_chain_core::ChainRecord { sequence: 0, prev_record_hash: GENESIS_HASH.to_string(), record_hash: ledger_hash.clone(), content_bytes: serde_json::to_vec(&receipt).unwrap() }];

        let active = build_chain(&[brain_api::AuditEventKind::RetentionAuthorized { target_segment_id: target().to_string(), retention_ledger_sequence: 0, retention_ledger_record_hash: ledger_hash }]);
        let state = classify(&active, &ledger, &target(), true);
        assert!(matches!(state, RecoveryState::AuthorizedPayloadPresent { retention_ledger_sequence: 0 }), "{state:?}");
    }

    #[test]
    fn authorized_with_matching_ledger_and_payload_absent_is_stale_manifest_until_refined() {
        let receipt = crate::domain::RetentionReceipt { retired_segment_id: target(), retired_segment_final_sequence: 0, retired_segment_final_record_hash: "a".repeat(64), retirement_authorized_by: "p".into(), retired_at: "2026-01-01T00:00:00Z".into() };
        let (_, ledger_hash) = serialize_record(&receipt, 0, GENESIS_HASH);
        let ledger = vec![audit_chain_core::ChainRecord { sequence: 0, prev_record_hash: GENESIS_HASH.to_string(), record_hash: ledger_hash.clone(), content_bytes: serde_json::to_vec(&receipt).unwrap() }];
        let active = build_chain(&[brain_api::AuditEventKind::RetentionAuthorized { target_segment_id: target().to_string(), retention_ledger_sequence: 0, retention_ledger_record_hash: ledger_hash }]);

        let state = classify(&active, &ledger, &target(), false);
        assert!(matches!(state, RecoveryState::AuthorizedPayloadAbsentManifestStale { .. }), "{state:?}");

        let refined_still_stale = refine_with_manifest_projection(state.clone(), true);
        assert_eq!(refined_still_stale, state, "manifest says retained_payload_present: true -- not yet refined to Complete");

        let refined_complete = refine_with_manifest_projection(state, false);
        assert!(matches!(refined_complete, RecoveryState::RetirementComplete { .. }));
    }

    #[test]
    fn payload_missing_with_no_authorization_at_all_is_tampering() {
        let active = build_chain(&[]);
        let ledger = build_chain(&[]);
        assert_eq!(classify(&active, &ledger, &target(), false), RecoveryState::PayloadMissingWithoutAuthorization);
    }

    #[test]
    fn an_authorized_record_whose_ledger_commitment_does_not_match_any_real_ledger_record_is_a_mismatch() {
        let active = build_chain(&[brain_api::AuditEventKind::RetentionAuthorized { target_segment_id: target().to_string(), retention_ledger_sequence: 0, retention_ledger_record_hash: "deadbeef".repeat(8) }]);
        let ledger = build_chain(&[]); // empty -- no record at sequence 0 at all
        assert_eq!(classify(&active, &ledger, &target(), true), RecoveryState::LedgerMismatch);
    }

    #[test]
    fn duplicate_receipt_anomaly_is_representable_in_the_ledger_evidence() {
        // This module itself does not DETECT duplicates (that is
        // crate::verifier's own job, mirroring mcp_audit_bridge::dedup's
        // full-chain anomaly detection) -- this test only confirms the
        // recovery classification does not choke on a ledger containing
        // two receipts for the same target, which a tampered/buggy
        // ledger could produce.
        let receipt = crate::domain::RetentionReceipt { retired_segment_id: target(), retired_segment_final_sequence: 0, retired_segment_final_record_hash: "a".repeat(64), retirement_authorized_by: "p".into(), retired_at: "2026-01-01T00:00:00Z".into() };
        let (_, hash0) = serialize_record(&receipt, 0, GENESIS_HASH);
        let (_, hash1) = serialize_record(&receipt, 1, &hash0);
        let ledger = vec![
            audit_chain_core::ChainRecord { sequence: 0, prev_record_hash: GENESIS_HASH.to_string(), record_hash: hash0.clone(), content_bytes: serde_json::to_vec(&receipt).unwrap() },
            audit_chain_core::ChainRecord { sequence: 1, prev_record_hash: hash0, record_hash: hash1.clone(), content_bytes: serde_json::to_vec(&receipt).unwrap() },
        ];
        let active = build_chain(&[brain_api::AuditEventKind::RetentionAuthorized { target_segment_id: target().to_string(), retention_ledger_sequence: 1, retention_ledger_record_hash: hash1 }]);
        let state = classify(&active, &ledger, &target(), true);
        assert!(matches!(state, RecoveryState::AuthorizedPayloadPresent { .. }));
    }
}
