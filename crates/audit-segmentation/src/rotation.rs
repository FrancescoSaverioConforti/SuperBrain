//! M7.5.6: segment rotation — closes the current Active segment and
//! opens a new, dedicated Active segment, writing `SegmentLinkage` as
//! its first record. Crash-safe and idempotent by construction: every
//! step re-derives its own precondition from the REAL chains (never
//! trusts the manifest alone — the frozen "manifest is an accelerator,
//! never an authority" rule applies here as much as everywhere else in
//! this crate), so resuming after a crash at any point reproduces
//! exactly the same end state a clean run would have.
//!
//! **Steps, in order** (the frozen design's own numbering):
//! 1. verify current segment (real `scan_chain`, never the manifest's
//!    cached `final_sequence`/`final_record_hash` alone);
//! 2. derive final sequence/hash from that real verification;
//! 3. mark the current segment `Closed` in the manifest (projection
//!    only — the real authority is step 5's own `SegmentLinkage`);
//! 4. create the next segment's own dedicated Vault;
//! 5. write `SegmentLinkage` as that Vault's FIRST record;
//! 6. verify the linkage (re-open, re-scan, decode, compare against
//!    step 2's own values — never assumed correct just because the
//!    write call returned `Ok`);
//! 7. mark the next segment `Active` in the manifest.
//!
//! The WHOLE sequence runs under `crate::lifecycle_lock`, held for its
//! entire duration — two concurrent rotations racing on the same
//! manifest is exactly the failure mode that lock exists to prevent.

use crate::domain::{SegmentId, SegmentManifest, SegmentManifestEntry, SegmentStatus};

#[derive(Debug, thiserror::Error)]
pub enum RotationError {
    #[error("another rotation/retention/manifest mutation is already in progress: {0}")]
    LockHeld(#[from] crate::lifecycle_lock::LifecycleLockError),
    #[error("failed to load the manifest: {0}")]
    ManifestLoadFailed(crate::manifest_store::ManifestStoreError),
    #[error("failed to save the manifest: {0}")]
    ManifestSaveFailed(crate::manifest_store::ManifestStoreError),
    #[error("no manifest exists yet -- adopt a legacy segment first")]
    NoManifest,
    #[error("no segment is currently Active")]
    NoActiveSegment,
    #[error("failed to open the current segment's vault: {0:?}")]
    CurrentSegmentVaultOpenFailed(storage::StorageError),
    #[error("failed to read the current segment's audit log: {0:?}")]
    CurrentSegmentReadFailed(storage::StorageError),
    #[error("current segment's chain failed verification, refusing to rotate: {0:?}")]
    CurrentSegmentChainInvalid(audit_chain_core::ScanStopReason),
    #[error("current segment has no verified records at all -- rotating an empty segment is refused (nothing to close)")]
    CurrentSegmentEmpty,
    #[error("failed to open the retention ledger: {0}")]
    LedgerOpenFailed(crate::ledger::LedgerError),
    #[error("failed to open the next segment's vault: {0:?}")]
    NextSegmentVaultOpenFailed(storage::StorageError),
    #[error("failed to start the next segment's durable writer: {0}")]
    NextSegmentWriterStartupFailed(brain_api_audit_worker::WorkerStartupError),
    #[error("durable append of SegmentLinkage to the next segment failed: {0}")]
    LinkageAppendFailed(brain_api_audit_worker::DurableRecordError),
    #[error("failed to verify the just-written SegmentLinkage record: {0}")]
    LinkageVerificationFailed(String),
}

/// Everything one rotation needs: where the manifest/retention ledger
/// live, and the locator+journal path the NEXT segment should be
/// created at (the caller decides this — this module never invents a
/// path convention on its own, same "locator is explicit, never a fixed
/// convention baked into code" discipline `crate::adoption` already
/// established for the legacy segment).
pub struct RotationConfig {
    pub manifest_path: std::path::PathBuf,
    pub retention_ledger_journal_path: std::path::PathBuf,
    pub next_segment_locator: std::path::PathBuf,
    pub next_segment_journal_path: std::path::PathBuf,
}

#[derive(Debug)]
pub struct RotationReport {
    pub closed_segment_id: SegmentId,
    pub closed_final_sequence: u64,
    pub closed_final_record_hash: String,
    pub new_active_segment_id: SegmentId,
}

/// Runs one full rotation. Idempotent: if a prior call already
/// completed steps 1-6 but crashed before step 7 (manifest not yet
/// reflecting the new Active segment), calling this again detects that
/// the next segment's `SegmentLinkage` already exists and verified,
/// and simply completes step 7 — never re-creates a second next
/// segment, never double-closes the current one.
pub async fn rotate(config: &RotationConfig) -> Result<RotationReport, RotationError> {
    let _lock = crate::lifecycle_lock::acquire(&config.manifest_path)?;

    let manifest = crate::manifest_store::load(&config.manifest_path).map_err(RotationError::ManifestLoadFailed)?.ok_or(RotationError::NoManifest)?;
    let active = manifest.active_segment().ok_or(RotationError::NoActiveSegment)?.clone();

    // Step 1 + 2: verify the current segment for REAL, never trust the
    // manifest's own cached fields (it has none for an Active segment
    // anyway -- SegmentManifest::validate enforces that).
    let (current_sequence, current_hash, current_record_count) = verify_current_segment(&active)?;

    // Idempotency check: has a next segment with a verified
    // SegmentLinkage matching THIS exact closure already been created
    // by a prior, interrupted run? If so, skip straight to step 7.
    if let Some(existing_linkage) = try_read_existing_linkage(config)? {
        if existing_linkage.prev_segment_id == active.segment_id.as_str() && existing_linkage.prev_final_sequence == current_sequence && existing_linkage.prev_final_record_hash == current_hash {
            return finalize_manifest(config, &manifest, &active, current_sequence, &current_hash, &active.segment_id.next().map_err(|_| RotationError::CurrentSegmentEmpty)?);
        }
    }

    // Step 3 (projection only) happens together with step 7 inside
    // finalize_manifest -- a single manifest write that marks the OLD
    // segment Closed and the NEW segment Active atomically, so there is
    // no intermediate manifest state with zero Active segments (which
    // SegmentManifest::validate would itself reject).

    // Step 4: open/create the next segment's own dedicated Vault.
    let next_segment_id = active.segment_id.next().map_err(|_| RotationError::CurrentSegmentEmpty)?;
    std::fs::create_dir_all(&config.next_segment_locator).map_err(|e| RotationError::NextSegmentVaultOpenFailed(storage::StorageError::Io(e)))?;
    let next_vault = storage::Vault::open_with_default_indexer(config.next_segment_locator.clone(), &config.next_segment_journal_path).map_err(RotationError::NextSegmentVaultOpenFailed)?;

    // Determine the retention ledger's own current tail (read-only,
    // real verification -- never the manifest's `retention_ledger_locator`
    // trusted blindly; this crate's own ledger module already performs
    // the real scan_chain/head.ref verification).
    let ledger_journal = config.retention_ledger_journal_path.clone();
    let (_ledger_vault, ledger_state) = crate::ledger::verify_and_scan(&manifest.retention_ledger_locator, &ledger_journal).map_err(RotationError::LedgerOpenFailed)?;
    let (retention_ledger_sequence, retention_ledger_record_hash) = ledger_state.last_verified.clone().unwrap_or((0, audit_chain_core::GENESIS_HASH.to_string()));

    // Step 5: write SegmentLinkage as the FIRST record of the next
    // segment, through the real, unmodified brain_api_audit_worker
    // machinery (same pattern mcp-audit-bridge already uses for its own
    // destination writes).
    let linkage_worker = brain_api_audit_worker::spawn_worker(next_vault, brain_api_audit_worker::WorkerConfig::default()).map_err(RotationError::NextSegmentWriterStartupFailed)?;
    let linkage_event = brain_api::event(
        brain_api::AuditEventKind::SegmentLinkage {
            prev_segment_id: active.segment_id.to_string(),
            prev_final_sequence: current_sequence,
            prev_final_record_hash: current_hash.clone(),
            prev_segment_record_count: current_record_count,
            retention_ledger_sequence,
            retention_ledger_record_hash: retention_ledger_record_hash.clone(),
        },
        &current_timestamp(),
        uuid::Uuid::new_v4(),
    );
    linkage_worker.record_durable(linkage_event).await.map_err(RotationError::LinkageAppendFailed)?;
    let _ = linkage_worker.shutdown().await;

    // Step 6: verify the linkage for REAL -- re-open, re-scan, decode,
    // compare against step 2's own values. Never trust the append
    // call's own Ok() alone as proof.
    let verified_linkage = try_read_existing_linkage(config)?.ok_or_else(|| RotationError::LinkageVerificationFailed("SegmentLinkage not found after a supposedly successful append".into()))?;
    if verified_linkage.prev_segment_id != active.segment_id.as_str() || verified_linkage.prev_final_sequence != current_sequence || verified_linkage.prev_final_record_hash != current_hash {
        return Err(RotationError::LinkageVerificationFailed("re-verified SegmentLinkage does not match the expected prior segment's final state".into()));
    }

    // Step 7 (+ step 3's projection): mark old Closed, new Active, in one atomic manifest write.
    finalize_manifest(config, &manifest, &active, current_sequence, &current_hash, &next_segment_id)
}

fn finalize_manifest(config: &RotationConfig, manifest: &SegmentManifest, closed: &SegmentManifestEntry, closed_final_sequence: u64, closed_final_record_hash: &str, next_segment_id: &SegmentId) -> Result<RotationReport, RotationError> {
    let mut updated = manifest.clone();
    for entry in updated.segments.iter_mut() {
        if entry.segment_id == closed.segment_id {
            entry.status = SegmentStatus::Closed;
            entry.final_sequence = Some(closed_final_sequence);
            entry.final_record_hash = Some(closed_final_record_hash.to_string());
        }
    }
    if updated.find(next_segment_id).is_none() {
        updated.segments.push(SegmentManifestEntry {
            segment_id: next_segment_id.clone(),
            locator: config.next_segment_locator.clone(),
            status: SegmentStatus::Active,
            final_sequence: None,
            final_record_hash: None,
            retained_payload_present: true,
            retention_eligible: true,
        });
    }
    crate::manifest_store::save(&config.manifest_path, &updated).map_err(RotationError::ManifestSaveFailed)?;
    Ok(RotationReport { closed_segment_id: closed.segment_id.clone(), closed_final_sequence, closed_final_record_hash: closed_final_record_hash.to_string(), new_active_segment_id: next_segment_id.clone() })
}

fn verify_current_segment(active: &SegmentManifestEntry) -> Result<(u64, String, u64), RotationError> {
    let journal = active.locator.join("journal.sqlite");
    let vault = storage::Vault::open_with_default_indexer(active.locator.clone(), &journal).map_err(RotationError::CurrentSegmentVaultOpenFailed)?;
    let raw = vault.read_audit_log_for_recovery_scan().map_err(RotationError::CurrentSegmentReadFailed)?;
    let scan = scan_chain_bytes(&raw);
    if let Some(stop_reason) = &scan.stop_reason {
        if !audit_chain_core::may_resume_without_recovery(&scan) {
            return Err(RotationError::CurrentSegmentChainInvalid(stop_reason.clone()));
        }
    }
    let (seq, hash) = scan.last_verified.ok_or(RotationError::CurrentSegmentEmpty)?;
    Ok((seq, hash, scan.verified_records.len() as u64))
}

struct DecodedLinkage {
    prev_segment_id: String,
    prev_final_sequence: u64,
    prev_final_record_hash: String,
}

fn try_read_existing_linkage(config: &RotationConfig) -> Result<Option<DecodedLinkage>, RotationError> {
    if !config.next_segment_locator.exists() {
        return Ok(None);
    }
    let vault = match storage::Vault::open_with_default_indexer(config.next_segment_locator.clone(), &config.next_segment_journal_path) {
        Ok(v) => v,
        Err(e) => return Err(RotationError::NextSegmentVaultOpenFailed(e)),
    };
    let raw = vault.read_audit_log_for_recovery_scan().map_err(RotationError::NextSegmentVaultOpenFailed)?;
    if raw.is_empty() {
        return Ok(None);
    }
    let scan = scan_chain_bytes(&raw);
    let Some(first) = scan.verified_records.first() else { return Ok(None) };
    let event: brain_api::AuditEvent = audit_chain_core::parse_event(&first.content_bytes).map_err(|e| RotationError::LinkageVerificationFailed(e.to_string()))?;
    match event.kind {
        brain_api::AuditEventKind::SegmentLinkage { prev_segment_id, prev_final_sequence, prev_final_record_hash, .. } => Ok(Some(DecodedLinkage { prev_segment_id, prev_final_sequence, prev_final_record_hash })),
        _ => Ok(None),
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

fn current_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Harness {
        dir: tempfile::TempDir,
        manifest_path: std::path::PathBuf,
        legacy_locator: std::path::PathBuf,
        retention_ledger_locator: std::path::PathBuf,
    }

    impl Harness {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let manifest_path = dir.path().join("segment-manifest.json");
            let legacy_locator = dir.path().join("segment-0");
            let retention_ledger_locator = dir.path().join("retention-ledger");
            std::fs::create_dir_all(&legacy_locator).unwrap();
            std::fs::create_dir_all(&retention_ledger_locator).unwrap();
            crate::adoption::adopt_existing_vault(&manifest_path, &legacy_locator, &retention_ledger_locator).unwrap();
            Self { dir, manifest_path, legacy_locator, retention_ledger_locator }
        }

        fn append_to_legacy(&self, kind: brain_api::AuditEventKind) {
            let journal = self.legacy_locator.join("journal.sqlite");
            let vault = storage::Vault::open_with_default_indexer(self.legacy_locator.clone(), &journal).unwrap();
            let raw = vault.read_audit_log_for_recovery_scan().unwrap();
            let scan = scan_chain_bytes(&raw);
            let sequence = scan.last_verified.as_ref().map(|(s, _)| s + 1).unwrap_or(0);
            let prev_hash = scan.last_verified.as_ref().map(|(_, h)| h.clone()).unwrap_or_else(|| audit_chain_core::GENESIS_HASH.to_string());
            let event = brain_api::event(kind, "2026-01-01T00:00:00Z", uuid::Uuid::new_v4());
            let (bytes, hash) = audit_chain_core::serialize_record(&event, sequence, &prev_hash);
            let mut writer = vault.open_audit_writer(scan.last_verified.as_ref().map(|(s, h)| (*s, h.as_str()))).unwrap();
            writer.append(&bytes, sequence, &hash).unwrap();
        }

        fn config(&self) -> RotationConfig {
            RotationConfig {
                manifest_path: self.manifest_path.clone(),
                retention_ledger_journal_path: self.retention_ledger_locator.join("journal.sqlite"),
                next_segment_locator: self.dir.path().join("segment-1"),
                next_segment_journal_path: self.dir.path().join("segment-1").join("journal.sqlite"),
            }
        }
    }

    fn sample_event(server: &str) -> brain_api::AuditEventKind {
        brain_api::AuditEventKind::McpConnection { server: server.into() }
    }

    #[tokio::test]
    async fn normal_rotation_closes_current_and_opens_a_new_active_segment() {
        let h = Harness::new();
        h.append_to_legacy(sample_event("s"));

        let report = rotate(&h.config()).await.unwrap();
        assert_eq!(report.closed_segment_id, SegmentId::from_index(0));
        assert_eq!(report.new_active_segment_id, SegmentId::from_index(1));

        let manifest = crate::manifest_store::load(&h.manifest_path).unwrap().unwrap();
        assert_eq!(manifest.find(&SegmentId::from_index(0)).unwrap().status, crate::domain::SegmentStatus::Closed);
        assert_eq!(manifest.find(&SegmentId::from_index(1)).unwrap().status, crate::domain::SegmentStatus::Active);
        assert!(manifest.find(&SegmentId::from_index(1)).unwrap().retention_eligible, "a segment created by rotation must default to retention_eligible: true");
    }

    #[tokio::test]
    async fn rotating_an_empty_current_segment_is_refused() {
        let h = Harness::new(); // no events appended to the legacy segment
        let result = rotate(&h.config()).await;
        assert!(matches!(result, Err(RotationError::CurrentSegmentEmpty)));
    }

    #[tokio::test]
    async fn the_new_segments_first_record_is_exactly_segment_linkage_committing_to_the_closed_segment() {
        let h = Harness::new();
        h.append_to_legacy(sample_event("s"));
        h.append_to_legacy(sample_event("s2"));

        rotate(&h.config()).await.unwrap();

        let config = h.config();
        let vault = storage::Vault::open_with_default_indexer(config.next_segment_locator.clone(), &config.next_segment_journal_path).unwrap();
        let raw = vault.read_audit_log_for_recovery_scan().unwrap();
        let scan = scan_chain_bytes(&raw);
        assert_eq!(scan.verified_records.len(), 1, "SegmentLinkage must be the ONLY record so far, and specifically the first");
        let event: brain_api::AuditEvent = audit_chain_core::parse_event(&scan.verified_records[0].content_bytes).unwrap();
        match event.kind {
            brain_api::AuditEventKind::SegmentLinkage { prev_segment_id, prev_final_sequence, prev_segment_record_count, .. } => {
                assert_eq!(prev_segment_id, "00000000000000000000");
                assert_eq!(prev_final_sequence, 1);
                assert_eq!(prev_segment_record_count, 2);
            }
            other => panic!("expected SegmentLinkage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn two_concurrent_rotations_only_one_succeeds() {
        let h = Harness::new();
        h.append_to_legacy(sample_event("s"));

        let config_a = h.config();
        let config_b = h.config();
        let task_a = tokio::spawn(async move { rotate(&config_a).await });
        let task_b = tokio::spawn(async move { rotate(&config_b).await });
        let (result_a, result_b) = tokio::join!(task_a, task_b);
        let (result_a, result_b) = (result_a.unwrap(), result_b.unwrap());

        let outcomes = [result_a, result_b];
        let successes = outcomes.iter().filter(|r| r.is_ok()).count();
        let lock_failures = outcomes.iter().filter(|r| matches!(r, Err(RotationError::LockHeld(_)))).count();
        assert_eq!(successes, 1, "exactly one concurrent rotation must succeed");
        assert_eq!(lock_failures, 1, "the other must fail specifically because the lifecycle lock was already held");
    }

    #[tokio::test]
    async fn rerunning_rotation_after_linkage_was_written_but_manifest_not_yet_updated_completes_idempotently() {
        let h = Harness::new();
        h.append_to_legacy(sample_event("s"));

        // First rotation completes fully and normally.
        rotate(&h.config()).await.unwrap();

        // Manually revert ONLY the manifest to look like step 7 never
        // happened (segment 0 still Active, segment 1 absent from the
        // manifest) -- modeling "crashed after step 6 (linkage verified)
        // but before step 7 (manifest write)". The real SegmentLinkage
        // record on disk is left untouched.
        let mut stale_manifest = crate::manifest_store::load(&h.manifest_path).unwrap().unwrap();
        stale_manifest.segments.retain(|s| s.segment_id != SegmentId::from_index(1));
        stale_manifest.segments[0].status = crate::domain::SegmentStatus::Active;
        stale_manifest.segments[0].final_sequence = None;
        stale_manifest.segments[0].final_record_hash = None;
        crate::manifest_store::save(&h.manifest_path, &stale_manifest).unwrap();

        // Re-running rotation must detect the already-written, already-
        // verifiable SegmentLinkage and simply complete step 7 -- never
        // attempt to create a THIRD segment or double-write linkage.
        let report = rotate(&h.config()).await.unwrap();
        assert_eq!(report.new_active_segment_id, SegmentId::from_index(1));

        let config = h.config();
        let vault = storage::Vault::open_with_default_indexer(config.next_segment_locator.clone(), &config.next_segment_journal_path).unwrap();
        let raw = vault.read_audit_log_for_recovery_scan().unwrap();
        let scan = scan_chain_bytes(&raw);
        assert_eq!(scan.verified_records.len(), 1, "resuming rotation must never duplicate the SegmentLinkage record");
    }
}
