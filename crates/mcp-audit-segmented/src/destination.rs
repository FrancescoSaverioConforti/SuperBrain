//! M7.7.4: `SegmentedDestination` — the real, verified view over a
//! segmented audit state directory, built entirely on `audit-segmentation`'s
//! own public API. Never trusts the manifest's raw fields as authority
//! (per the standing rule); the retention-ledger locator it reads from
//! the manifest is always compared against
//! [`crate::trusted_root::TrustedSegmentationRoot`]'s own derived
//! value before being used for anything.

use crate::trusted_root::TrustedSegmentationRoot;

#[derive(Debug, thiserror::Error)]
pub enum SegmentedDestinationError {
    #[error("failed to load the manifest: {0}")]
    ManifestLoadFailed(#[from] audit_segmentation::ManifestStoreError),
    #[error("no manifest exists at this trusted root yet -- adopt a legacy segment first")]
    NoManifest,
    #[error("no segment is currently Active")]
    NoActiveSegment,
    #[error("manifest's retention_ledger_locator ({found:?}) does not match the trusted root's own expected locator ({expected:?}) -- refusing to trust it")]
    RetentionLedgerLocatorMismatch { expected: std::path::PathBuf, found: std::path::PathBuf },
    #[error("failed to open/scan a segment's own audit log: {0:?}")]
    SegmentReadFailed(storage::StorageError),
    #[error("a segment's chain failed verification: {0:?}")]
    SegmentChainInvalid(audit_chain_core::ScanStopReason),
    #[error("end-to-end segmentation verification failed: {0:?}")]
    VerificationFailed(Vec<audit_segmentation::VerificationFinding>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSegment {
    pub segment_id: audit_segmentation::SegmentId,
    pub locator: std::path::PathBuf,
    pub journal_path: std::path::PathBuf,
}

/// Opened, verified handle onto one segmented audit state directory.
/// Every method re-derives its own answer from the real manifest/
/// chains at call time — never caches a stale view across calls (same
/// discipline `mcp_audit_bridge::run_with_destination` already applies
/// to its own single-vault case).
pub struct SegmentedDestination {
    trusted_root: TrustedSegmentationRoot,
}

impl SegmentedDestination {
    pub fn open(trusted_root: TrustedSegmentationRoot) -> Self {
        Self { trusted_root }
    }

    pub fn manifest_path(&self) -> std::path::PathBuf {
        self.trusted_root.manifest_path()
    }

    pub fn pin_store_locator(&self) -> std::path::PathBuf {
        self.trusted_root.expected_pin_store_locator()
    }

    pub fn pin_store_journal_path(&self) -> std::path::PathBuf {
        self.pin_store_locator().join("journal.sqlite")
    }

    fn load_and_verify_manifest(&self) -> Result<audit_segmentation::SegmentManifest, SegmentedDestinationError> {
        let manifest = audit_segmentation::manifest_store::load(&self.trusted_root.manifest_path())?.ok_or(SegmentedDestinationError::NoManifest)?;
        let expected_ledger_locator = self.trusted_root.expected_retention_ledger_locator();
        if manifest.retention_ledger_locator != expected_ledger_locator {
            return Err(SegmentedDestinationError::RetentionLedgerLocatorMismatch { expected: expected_ledger_locator, found: manifest.retention_ledger_locator.clone() });
        }
        Ok(manifest)
    }

    /// The segment currently Active for writing — the ONE segment any
    /// append this call resolves to may ever land in for the lifetime
    /// of whatever write window a caller builds on top of this.
    pub fn resolve_active_destination(&self) -> Result<ResolvedSegment, SegmentedDestinationError> {
        let manifest = self.load_and_verify_manifest()?;
        let active = manifest.active_segment().ok_or(SegmentedDestinationError::NoActiveSegment)?;
        Ok(ResolvedSegment { segment_id: active.segment_id.clone(), locator: active.locator.clone(), journal_path: active.locator.join("journal.sqlite") })
    }

    /// Every segment whose payload still physically exists
    /// (`retained_payload_present: true`) — a caller enumerating
    /// history for dedup/registration-recovery purposes must still run
    /// `history_completeness` separately to know whether this set is
    /// SUFFICIENT (see that method's own doc comment for the
    /// conservative, frozen M7.7 semantics).
    pub fn enumerate_retained_segments(&self) -> Result<Vec<ResolvedSegment>, SegmentedDestinationError> {
        let manifest = self.load_and_verify_manifest()?;
        Ok(manifest.segments.iter().filter(|s| s.retained_payload_present).map(|s| ResolvedSegment { segment_id: s.segment_id.clone(), locator: s.locator.clone(), journal_path: s.locator.join("journal.sqlite") }).collect())
    }

    /// M7.7, frozen correction: `Complete` if and only if EVERY segment
    /// in the manifest still has its payload present — the simplest,
    /// safest rule, deliberately never attempting to prove a retained
    /// segment was "irrelevant" via any sequence-based heuristic (the
    /// two numeric domains — source MCP chain sequence and destination
    /// segment chain sequence — are never comparable, and no durable
    /// record today links a specific `SourceRecordIdentity` to the
    /// segment that once held it). Safety over availability: a SINGLE
    /// retained segment anywhere renders the WHOLE history `Incomplete`
    /// for dedup/registration-recovery purposes, for as long as no
    /// provenance-summary mechanism exists (explicitly out of scope
    /// here).
    pub fn history_completeness(&self) -> Result<mcp_audit_bridge::HistoryCompleteness, SegmentedDestinationError> {
        let manifest = self.load_and_verify_manifest()?;
        let any_retired = manifest.segments.iter().any(|s| !s.retained_payload_present);
        Ok(if any_retired { mcp_audit_bridge::HistoryCompleteness::Incomplete } else { mcp_audit_bridge::HistoryCompleteness::Complete })
    }

    /// Verified, chain-linked history across every retained segment —
    /// the real `audit_segmentation::verify` is run first (never
    /// skipped): a segment reachable only through a BROKEN linkage is
    /// never silently included. Returns the concatenation of every
    /// retained segment's own verified records, in linkage order.
    pub fn verified_history(&self) -> Result<Vec<audit_chain_core::ChainRecord>, SegmentedDestinationError> {
        let manifest = self.load_and_verify_manifest()?;
        let retained = self.enumerate_retained_segments()?;

        let mut journal_paths = std::collections::HashMap::new();
        for segment in &retained {
            journal_paths.insert(segment.segment_id.clone(), segment.journal_path.clone());
        }

        let verifier_input = audit_segmentation::VerifierInput { manifest: &manifest, journal_paths, retention_ledger_journal_path: self.trusted_root.expected_retention_ledger_locator().join("journal.sqlite") };
        let report = audit_segmentation::verify(&verifier_input);
        if !report.is_clean() {
            return Err(SegmentedDestinationError::VerificationFailed(report.findings));
        }

        let mut all_records = Vec::new();
        for segment_id in &report.verified_segment_order {
            if let Some(segment) = retained.iter().find(|s| &s.segment_id == segment_id) {
                let vault = storage::Vault::open_with_default_indexer(segment.locator.clone(), &segment.journal_path).map_err(SegmentedDestinationError::SegmentReadFailed)?;
                let raw = vault.read_audit_log_for_recovery_scan().map_err(SegmentedDestinationError::SegmentReadFailed)?;
                let scan = scan_chain_bytes(&raw);
                if let Some(stop_reason) = &scan.stop_reason {
                    if !audit_chain_core::may_resume_without_recovery(&scan) {
                        return Err(SegmentedDestinationError::SegmentChainInvalid(stop_reason.clone()));
                    }
                }
                all_records.extend(scan.verified_records);
            }
        }
        Ok(all_records)
    }
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
    use crate::trusted_root::TrustedSegmentationRoot;

    struct Harness {
        dir: tempfile::TempDir,
        root: TrustedSegmentationRoot,
        legacy_locator: std::path::PathBuf,
    }

    impl Harness {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = TrustedSegmentationRoot::new(dir.path().to_path_buf());
            let legacy_locator = dir.path().join("segment-0");
            std::fs::create_dir_all(&legacy_locator).unwrap();
            std::fs::create_dir_all(root.expected_retention_ledger_locator()).unwrap();
            std::fs::create_dir_all(root.expected_pin_store_locator()).unwrap();
            audit_segmentation::adopt_existing_vault(&root.manifest_path(), &legacy_locator, &root.expected_retention_ledger_locator()).unwrap();
            Self { dir, root, legacy_locator }
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
    }

    #[test]
    fn resolve_active_destination_finds_the_legacy_segment_on_a_fresh_state() {
        let h = Harness::new();
        let seg_dest = SegmentedDestination::open(h.root.clone());
        let active = seg_dest.resolve_active_destination().unwrap();
        assert_eq!(active.segment_id, audit_segmentation::SegmentId::from_index(0));
        assert_eq!(active.locator, h.legacy_locator);
    }

    #[test]
    fn history_completeness_is_complete_when_nothing_has_been_retained() {
        let h = Harness::new();
        let seg_dest = SegmentedDestination::open(h.root.clone());
        assert_eq!(seg_dest.history_completeness().unwrap(), mcp_audit_bridge::HistoryCompleteness::Complete);
    }

    #[test]
    fn verified_history_includes_the_legacy_segments_own_records() {
        let h = Harness::new();
        h.append(&h.legacy_locator, brain_api::AuditEventKind::McpConnection { server: "s".into() });
        let seg_dest = SegmentedDestination::open(h.root.clone());
        let history = seg_dest.verified_history().unwrap();
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn a_tampered_retention_ledger_locator_in_the_manifest_is_rejected() {
        let h = Harness::new();
        let mut manifest = audit_segmentation::manifest_store::load(&h.root.manifest_path()).unwrap().unwrap();
        manifest.retention_ledger_locator = h.dir.path().join("fake-ledger");
        std::fs::write(h.root.manifest_path(), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

        let seg_dest = SegmentedDestination::open(h.root.clone());
        let result = seg_dest.resolve_active_destination();
        assert!(matches!(result, Err(SegmentedDestinationError::RetentionLedgerLocatorMismatch { .. })), "got {result:?}");
    }
}
