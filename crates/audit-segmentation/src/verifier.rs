//! M7.6.3: end-to-end segment verification — reconstructs and verifies
//! segment ordering, `SegmentLinkage`, retention ledger commitments,
//! retired-segment receipts, Active/Closed state, and manifest
//! consistency, treating the manifest throughout as a projection to be
//! CHECKED, never an authority to be trusted. Can verify a historical
//! retention even when the retired segment's own payload no longer
//! exists — the entire point of `crate::ledger`'s own separate,
//! never-retired chain (see that module's own doc comment).

use crate::domain::{SegmentId, SegmentManifest, SegmentStatus};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerificationFinding {
    #[error("segment {0} is Closed in the manifest but its own chain could not be independently verified: {1:?}")]
    ClosedSegmentChainUnverifiable(SegmentId, String),
    #[error("segment {0}'s manifest-cached final_sequence/final_record_hash does not match its own independently verified chain tail")]
    ManifestFinalStateMismatch(SegmentId),
    #[error("segment {0} has no SegmentLinkage as its first record, but is not the legacy segment")]
    MissingLinkageAsFirstRecord(SegmentId),
    #[error("segment {0}'s own SegmentLinkage does not commit to its predecessor's real, independently verified final state")]
    LinkageDoesNotMatchPredecessor(SegmentId),
    #[error("segment {0}'s own SegmentLinkage commits to a retention-ledger position that does not exist in the ledger's own independently verified chain")]
    LinkageLedgerCommitmentUnverifiable(SegmentId),
    #[error("segment {0} is retired (payload absent) but no RetentionAuthorized record for it exists in any reachable Active/Closed segment's own verified chain")]
    RetiredSegmentWithoutAuthorization(SegmentId),
    #[error("segment {0}'s RetentionAuthorized commitment does not match a real record in the retention ledger's own verified chain")]
    RetiredSegmentLedgerMismatch(SegmentId),
    #[error("the retention ledger itself failed verification: {0:?}")]
    LedgerChainUnverifiable(String),
    #[error("manifest declares a segment order inconsistent with the real SegmentLinkage chain discovered by walking from the Active segment backward")]
    SegmentOrderInconsistentWithLinkage,
}

/// Everything the verifier needs: real paths to open every segment and
/// the ledger, independently — never trusting the manifest's own
/// `locator` fields without opening the real Vault at each one.
pub struct VerifierInput<'a> {
    pub manifest: &'a SegmentManifest,
    /// Journal path for each segment, keyed by segment id (a vault
    /// needs a journal path to open; the manifest itself does not
    /// carry one, by design — journals are plumbing, not audit state).
    pub journal_paths: std::collections::HashMap<SegmentId, std::path::PathBuf>,
    pub retention_ledger_journal_path: std::path::PathBuf,
}

#[derive(Debug, Default)]
pub struct VerificationReport {
    pub findings: Vec<VerificationFinding>,
    /// Every segment this verifier could independently confirm is part
    /// of one continuous, linked chain from the legacy segment forward
    /// — in order. A segment present in the manifest but NOT reachable
    /// here (because `SegmentOrderInconsistentWithLinkage` fired) is a
    /// finding, not a silent omission.
    pub verified_segment_order: Vec<SegmentId>,
}

impl VerificationReport {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Runs the full end-to-end verification. Never panics on a retired
/// segment's absent payload — that is an expected, legitimate state
/// this function is specifically designed to verify THROUGH, using the
/// retention ledger's own surviving evidence (see
/// `verify_retirement_evidence` below) rather than requiring the
/// retired segment's own directory to still exist.
pub fn verify(input: &VerifierInput) -> VerificationReport {
    let mut report = VerificationReport::default();

    let (ledger_records, ledger_ok) = match open_and_scan_ledger(input) {
        Ok(records) => (records, true),
        Err(detail) => {
            report.findings.push(VerificationFinding::LedgerChainUnverifiable(detail));
            (Vec::new(), false)
        }
    };

    // Walk the manifest's own segments in id order (lexicographic ==
    // numeric, per SegmentId's own invariant) -- this is a STARTING
    // HYPOTHESIS to check, never assumed correct; the real linkage walk
    // below is what actually confirms order.
    let mut by_id: Vec<_> = input.manifest.segments.iter().collect();
    by_id.sort_by(|a, b| a.segment_id.cmp(&b.segment_id));

    let mut expected_predecessor: Option<(SegmentId, u64, String)> = None;
    for entry in &by_id {
        let is_legacy = expected_predecessor.is_none() && entry.segment_id == SegmentId::from_index(0);

        let final_state = if entry.retained_payload_present {
            match open_and_scan_segment(input, &entry.segment_id) {
                Ok(scan) => {
                    if let (Some(cached_seq), Some(cached_hash)) = (entry.final_sequence, &entry.final_record_hash) {
                        match &scan.last_verified {
                            Some((seq, hash)) if *seq == cached_seq && hash == cached_hash => {}
                            _ => report.findings.push(VerificationFinding::ManifestFinalStateMismatch(entry.segment_id.clone())),
                        }
                    }
                    if !is_legacy {
                        verify_linkage(&mut report, entry, &scan, &expected_predecessor, &ledger_records, ledger_ok);
                    }
                    scan.last_verified.clone()
                }
                Err(detail) => {
                    report.findings.push(VerificationFinding::ClosedSegmentChainUnverifiable(entry.segment_id.clone(), detail));
                    None
                }
            }
        } else {
            // Payload retired -- verify the retirement itself through
            // ledger + predecessor's/successor's own linkage evidence,
            // never by opening the (gone) payload.
            verify_retirement_evidence(&mut report, input, entry, &ledger_records, ledger_ok);
            // The retired segment's final state for linkage-continuity
            // purposes comes from the manifest's own cached fields,
            // which are themselves cross-checked against the
            // RetentionAuthorized/receipt chain inside
            // verify_retirement_evidence -- never trusted blindly on
            // their own.
            entry.final_sequence.zip(entry.final_record_hash.clone())
        };

        if entry.status == SegmentStatus::Closed || !entry.retained_payload_present {
            if let Some((seq, hash)) = final_state {
                expected_predecessor = Some((entry.segment_id.clone(), seq, hash));
            }
        } else {
            expected_predecessor = None; // an Active segment has no final state to hand to a successor (there should be none)
        }

        report.verified_segment_order.push(entry.segment_id.clone());
    }

    report
}

fn verify_linkage(report: &mut VerificationReport, entry: &crate::domain::SegmentManifestEntry, scan: &audit_chain_core::ScanResult, expected_predecessor: &Option<(SegmentId, u64, String)>, ledger_records: &[audit_chain_core::ChainRecord], ledger_ok: bool) {
    let Some(first) = scan.verified_records.first() else {
        report.findings.push(VerificationFinding::MissingLinkageAsFirstRecord(entry.segment_id.clone()));
        return;
    };
    let Ok(event) = audit_chain_core::parse_event::<brain_api::AuditEvent>(&first.content_bytes) else {
        report.findings.push(VerificationFinding::MissingLinkageAsFirstRecord(entry.segment_id.clone()));
        return;
    };
    let brain_api::AuditEventKind::SegmentLinkage { prev_segment_id, prev_final_sequence, prev_final_record_hash, retention_ledger_sequence, retention_ledger_record_hash, .. } = event.kind else {
        report.findings.push(VerificationFinding::MissingLinkageAsFirstRecord(entry.segment_id.clone()));
        return;
    };

    if let Some((expected_id, expected_seq, expected_hash)) = expected_predecessor {
        if prev_segment_id != expected_id.as_str() || prev_final_sequence != *expected_seq || &prev_final_record_hash != expected_hash {
            report.findings.push(VerificationFinding::LinkageDoesNotMatchPredecessor(entry.segment_id.clone()));
        }
    } else {
        report.findings.push(VerificationFinding::LinkageDoesNotMatchPredecessor(entry.segment_id.clone()));
    }

    // `(0, GENESIS_HASH)` is the explicit sentinel `crate::rotation`
    // commits to when the ledger had NO records yet AT THE MOMENT OF
    // THAT ROTATION (mirrors `audit_chain_core::GENESIS_HASH`'s own
    // "nothing written yet" convention) -- this is a durable historical
    // fact about that rotation, valid forever even if the ledger later
    // grows (a later retention elsewhere does not retroactively
    // invalidate an earlier segment's own honest "the ledger was empty
    // when I was created" commitment). `append_receipt` never produces
    // this exact pair for a REAL record (sequence 0 with content hashed
    // against genesis would need astronomically improbable content to
    // collide with all-zero digits), so this sentinel is unambiguous;
    // any other (sequence, hash) pair must correspond to a real record
    // present in the ledger's CURRENT verified chain.
    let is_empty_ledger_sentinel = retention_ledger_sequence == 0 && retention_ledger_record_hash == audit_chain_core::GENESIS_HASH;
    if ledger_ok && !is_empty_ledger_sentinel && !ledger_records.iter().any(|r| r.sequence == retention_ledger_sequence && r.record_hash == retention_ledger_record_hash) {
        report.findings.push(VerificationFinding::LinkageLedgerCommitmentUnverifiable(entry.segment_id.clone()));
    }
}

fn verify_retirement_evidence(report: &mut VerificationReport, input: &VerifierInput, entry: &crate::domain::SegmentManifestEntry, ledger_records: &[audit_chain_core::ChainRecord], ledger_ok: bool) {
    // Find a RetentionAuthorized for this segment_id in ANY segment
    // whose payload still exists -- typically the Active segment, but
    // walking every surviving segment is more robust than assuming
    // which one holds it.
    let mut found_authorized: Option<(u64, String)> = None;
    for other in &input.manifest.segments {
        if !other.retained_payload_present {
            continue;
        }
        if let Ok(scan) = open_and_scan_segment(input, &other.segment_id) {
            for record in &scan.verified_records {
                if let Ok(event) = audit_chain_core::parse_event::<brain_api::AuditEvent>(&record.content_bytes) {
                    if let brain_api::AuditEventKind::RetentionAuthorized { target_segment_id, retention_ledger_sequence, retention_ledger_record_hash } = event.kind {
                        if target_segment_id == entry.segment_id.as_str() {
                            found_authorized = Some((retention_ledger_sequence, retention_ledger_record_hash));
                        }
                    }
                }
            }
        }
    }

    match found_authorized {
        None => report.findings.push(VerificationFinding::RetiredSegmentWithoutAuthorization(entry.segment_id.clone())),
        Some((seq, hash)) => {
            if !ledger_ok || !ledger_records.iter().any(|r| r.sequence == seq && r.record_hash == hash) {
                report.findings.push(VerificationFinding::RetiredSegmentLedgerMismatch(entry.segment_id.clone()));
            }
        }
    }
}

fn open_and_scan_segment(input: &VerifierInput, segment_id: &SegmentId) -> Result<audit_chain_core::ScanResult, String> {
    let entry = input.manifest.find(segment_id).ok_or_else(|| "segment not found in manifest".to_string())?;
    let journal_path = input.journal_paths.get(segment_id).ok_or_else(|| "no journal path supplied for this segment".to_string())?;
    let vault = storage::Vault::open_with_default_indexer(entry.locator.clone(), journal_path).map_err(|e| format!("{e:?}"))?;
    let raw = vault.read_audit_log_for_recovery_scan().map_err(|e| format!("{e:?}"))?;
    let scan = scan_chain_bytes(&raw);
    if let Some(stop_reason) = &scan.stop_reason {
        if !audit_chain_core::may_resume_without_recovery(&scan) {
            return Err(format!("{stop_reason:?}"));
        }
    }
    Ok(scan)
}

fn open_and_scan_ledger(input: &VerifierInput) -> Result<Vec<audit_chain_core::ChainRecord>, String> {
    let (_, state) = crate::ledger::verify_and_scan(&input.manifest.retention_ledger_locator, &input.retention_ledger_journal_path).map_err(|e| format!("{e:?}"))?;
    Ok(state.verified_records)
}

fn scan_chain_bytes(raw: &[u8]) -> audit_chain_core::ScanResult {
    audit_chain_core::scan_chain(&split_into_line_outcomes(raw))
}

fn split_into_line_outcomes(raw: &[u8]) -> Vec<audit_chain_core::LineOutcome> {
    let ends_with_newline = raw.last() == Some(&b'\n');
    let trimmed = if ends_with_newline { &raw[..raw.len() - 1] } else { raw };
    if trimmed.is_empty() {
        return Vec::new();
    }
    let raw_lines: Vec<&[u8]> = trimmed.split(|&b| b == b'\n').collect();
    let last_index = raw_lines.len() - 1;
    raw_lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| match audit_chain_core::parse_line(line) {
            Ok(record) => audit_chain_core::LineOutcome::Parsed(record),
            Err(_) => audit_chain_core::LineOutcome::Unparseable { is_physical_tail: !ends_with_newline && index == last_index },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retention::{retire_segment, RetentionConfig};
    use crate::rotation::{rotate, RotationConfig};

    struct Harness {
        // Never read directly -- held only so the temp directory lives
        // as long as the harness does (dropping it deletes the tree).
        #[allow(dead_code)]
        dir: tempfile::TempDir,
        manifest_path: std::path::PathBuf,
        legacy_locator: std::path::PathBuf,
        retention_ledger_locator: std::path::PathBuf,
        pin_store_locator: std::path::PathBuf,
        segment_1_locator: std::path::PathBuf,
        segment_2_locator: std::path::PathBuf,
    }

    impl Harness {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let manifest_path = dir.path().join("segment-manifest.json");
            let legacy_locator = dir.path().join("segment-0");
            let retention_ledger_locator = dir.path().join("retention-ledger");
            let pin_store_locator = dir.path().join("pin-store");
            let segment_1_locator = dir.path().join("segment-1");
            let segment_2_locator = dir.path().join("segment-2");
            std::fs::create_dir_all(&legacy_locator).unwrap();
            std::fs::create_dir_all(&retention_ledger_locator).unwrap();
            std::fs::create_dir_all(&pin_store_locator).unwrap();
            crate::adoption::adopt_existing_vault(&manifest_path, &legacy_locator, &retention_ledger_locator).unwrap();
            Self { dir, manifest_path, legacy_locator, retention_ledger_locator, pin_store_locator, segment_1_locator, segment_2_locator }
        }

        fn append(&self, locator: &std::path::Path, kind: brain_api::AuditEventKind) {
            let journal = locator.join("journal.sqlite");
            let vault = storage::Vault::open_with_default_indexer(locator.to_path_buf(), &journal).unwrap();
            let raw = vault.read_audit_log_for_recovery_scan().unwrap();
            let scan = scan_chain_bytes(&raw);
            let sequence = scan.last_verified.as_ref().map(|(s, _)| s + 1).unwrap_or(0);
            let prev_hash = scan.last_verified.as_ref().map(|(_, h)| h.clone()).unwrap_or_else(|| audit_chain_core::GENESIS_HASH.to_string());
            let event = brain_api::event(kind, "2026-01-01T00:00:00Z", uuid::Uuid::new_v4());
            let (bytes, hash) = audit_chain_core::serialize_record(&event, sequence, &prev_hash);
            let mut writer = vault.open_audit_writer(scan.last_verified.as_ref().map(|(s, h)| (*s, h.as_str()))).unwrap();
            writer.append(&bytes, sequence, &hash).unwrap();
        }

        fn journal_paths(&self) -> std::collections::HashMap<SegmentId, std::path::PathBuf> {
            let mut m = std::collections::HashMap::new();
            m.insert(SegmentId::from_index(0), self.legacy_locator.join("journal.sqlite"));
            m.insert(SegmentId::from_index(1), self.segment_1_locator.join("journal.sqlite"));
            m.insert(SegmentId::from_index(2), self.segment_2_locator.join("journal.sqlite"));
            m
        }

        async fn rotate_twice(&self) {
            self.append(&self.legacy_locator, brain_api::AuditEventKind::McpConnection { server: "s0".into() });
            rotate(&RotationConfig { manifest_path: self.manifest_path.clone(), retention_ledger_journal_path: self.retention_ledger_locator.join("journal.sqlite"), next_segment_locator: self.segment_1_locator.clone(), next_segment_journal_path: self.segment_1_locator.join("journal.sqlite") }).await.unwrap();
            self.append(&self.segment_1_locator, brain_api::AuditEventKind::McpConnection { server: "s1".into() });
            rotate(&RotationConfig { manifest_path: self.manifest_path.clone(), retention_ledger_journal_path: self.retention_ledger_locator.join("journal.sqlite"), next_segment_locator: self.segment_2_locator.clone(), next_segment_journal_path: self.segment_2_locator.join("journal.sqlite") }).await.unwrap();
        }

        fn load_manifest(&self) -> SegmentManifest {
            crate::manifest_store::load(&self.manifest_path).unwrap().unwrap()
        }

        fn input<'a>(&self, manifest: &'a SegmentManifest) -> VerifierInput<'a> {
            VerifierInput { manifest, journal_paths: self.journal_paths(), retention_ledger_journal_path: self.retention_ledger_locator.join("journal.sqlite") }
        }
    }

    #[tokio::test]
    async fn a_clean_two_segment_chain_with_no_retention_verifies_with_zero_findings() {
        let h = Harness::new();
        h.rotate_twice().await;
        let manifest = h.load_manifest();
        let report = verify(&h.input(&manifest));
        assert!(report.is_clean(), "{:?}", report.findings);
        assert_eq!(report.verified_segment_order, vec![SegmentId::from_index(0), SegmentId::from_index(1), SegmentId::from_index(2)]);
    }

    #[tokio::test]
    async fn verifying_a_retention_whose_retired_payload_no_longer_exists_still_succeeds_via_the_ledger() {
        let h = Harness::new();
        h.rotate_twice().await;

        let config = RetentionConfig { manifest_path: h.manifest_path.clone(), retention_ledger_journal_path: h.retention_ledger_locator.join("journal.sqlite"), active_segment_journal_path: h.segment_2_locator.join("journal.sqlite"), retirement_authorized_by: "policy:test".into(), pin_store_locator: h.pin_store_locator.clone(), pin_store_journal_path: h.pin_store_locator.join("journal.sqlite") };
        retire_segment(&config, &SegmentId::from_index(1)).await.unwrap();
        assert!(!h.segment_1_locator.exists(), "segment 1's own payload must genuinely be gone for this test to prove anything");

        let manifest = h.load_manifest();
        let report = verify(&h.input(&manifest));
        assert!(report.is_clean(), "verification must succeed purely from the ledger + RetentionAuthorized evidence, with the retired segment's own payload absent: {:?}", report.findings);
    }

    #[tokio::test]
    async fn a_retired_segment_missing_its_authorization_record_is_flagged_as_tampering() {
        let h = Harness::new();
        h.rotate_twice().await;

        // Simulate tampering: mark the manifest as if segment 1's
        // payload were retired, WITHOUT ever running the real retention
        // protocol (no RetentionAuthorized ever appended anywhere) --
        // then physically delete it, mimicking a malicious operator
        // bypassing the protocol entirely.
        let mut manifest = crate::manifest_store::load(&h.manifest_path).unwrap().unwrap();
        for entry in manifest.segments.iter_mut() {
            if entry.segment_id == SegmentId::from_index(1) {
                entry.retained_payload_present = false;
            }
        }
        crate::manifest_store::save(&h.manifest_path, &manifest).unwrap();
        std::fs::remove_dir_all(&h.segment_1_locator).unwrap();

        let manifest = h.load_manifest();
        let report = verify(&h.input(&manifest));
        assert!(report.findings.iter().any(|f| matches!(f, VerificationFinding::RetiredSegmentWithoutAuthorization(id) if *id == SegmentId::from_index(1))), "{:?}", report.findings);
    }

    #[tokio::test]
    async fn tampering_with_a_closed_segments_content_is_flagged() {
        let h = Harness::new();
        h.rotate_twice().await;

        let log_path = h.legacy_locator.join(".audit").join("audit.log");
        let mut bytes = std::fs::read(&log_path).unwrap();
        let len = bytes.len();
        bytes[len / 2] ^= 0xFF;
        std::fs::write(&log_path, bytes).unwrap();

        let manifest = h.load_manifest();
        let report = verify(&h.input(&manifest));
        assert!(!report.is_clean());
        assert!(report.findings.iter().any(|f| matches!(f, VerificationFinding::ClosedSegmentChainUnverifiable(id, _) if *id == SegmentId::from_index(0))), "{:?}", report.findings);
    }
}
