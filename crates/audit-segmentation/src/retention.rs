//! M7.6.1: the 5-phase retention protocol — the proof is written to
//! the audit chain and the retention ledger BEFORE any payload is
//! deleted, never after, closing the crash window a prior design
//! review found (payload-deleted-then-crash must never be
//! indistinguishable from tampering).
//!
//! **Phases, in order, each durable before the next begins**:
//! 1. `RetentionIntentAttempted` appended to the CURRENT Active
//!    segment (never the target — the target may already be Closed).
//! 2. `RetentionReceipt` appended to the retention ledger
//!    (`crate::ledger::append_receipt`).
//! 3. `RetentionAuthorized` appended to the current Active segment,
//!    committing to the receipt's own `(sequence, record_hash)` — from
//!    this point on, deleting the target's payload is authorized and
//!    PROVABLY so, independent of whether the delete itself ever
//!    completes.
//! 4. delete the target segment's own retirable payload — ONLY ever
//!    the dedicated segment directory a rotation created, NEVER the
//!    legacy vault root (see `retention_eligible`'s own enforcement
//!    below, checked before phase 1 even begins).
//! 5. mark `retained_payload_present: false` in the manifest — a pure
//!    projection update; the manifest was never the authority.
//!
//! **`retention_eligible == true` is checked BEFORE phase 1** — the
//! frozen requirement: no `RetentionIntentAttempted` is ever appended
//! for a segment that is not eligible, so the legacy segment's audit
//! chain is never even touched by an attempted retention.

use crate::domain::{RetentionReceipt, SegmentId, SegmentManifestEntry};

#[derive(Debug, thiserror::Error)]
pub enum RetentionError {
    #[error("another rotation/retention/manifest mutation is already in progress: {0}")]
    LockHeld(#[from] crate::lifecycle_lock::LifecycleLockError),
    #[error("failed to load the manifest: {0}")]
    ManifestLoadFailed(crate::manifest_store::ManifestStoreError),
    #[error("failed to save the manifest: {0}")]
    ManifestSaveFailed(crate::manifest_store::ManifestStoreError),
    #[error("no manifest exists yet")]
    NoManifest,
    #[error("target segment {0} is not present in the manifest")]
    TargetSegmentNotFound(SegmentId),
    #[error("target segment {0} is not retention_eligible -- refused before any audit mutation")]
    NotRetentionEligible(SegmentId),
    #[error("target segment {0} is not Closed -- only a Closed segment may be retired")]
    TargetSegmentNotClosed(SegmentId),
    #[error("no segment is currently Active (needed to record RetentionIntentAttempted/RetentionAuthorized)")]
    NoActiveSegment,
    #[error("failed to open the current Active segment's vault: {0:?}")]
    ActiveSegmentVaultOpenFailed(storage::StorageError),
    #[error("failed to start the current Active segment's durable writer: {0}")]
    ActiveSegmentWriterStartupFailed(brain_api_audit_worker::WorkerStartupError),
    #[error("durable append to the current Active segment failed: {0}")]
    ActiveSegmentAppendFailed(brain_api_audit_worker::DurableRecordError),
    #[error("failed to append the retention receipt to the ledger: {0}")]
    LedgerAppendFailed(#[from] crate::ledger::LedgerError),
    #[error("failed to delete the target segment's retirable payload: {0}")]
    PayloadDeleteFailed(std::io::Error),
    /// M7.7.3: at least one `RetentionPin` is still Active on the
    /// target segment — denied BEFORE phase 1 (no `RetentionIntentAttempted`
    /// is ever appended), a structural property of this function, never
    /// a composition-root convention.
    #[error("segment {0} has at least one Active RetentionPin -- denied before any audit mutation: {1:?}")]
    SegmentHasActivePins(SegmentId, Vec<uuid::Uuid>),
    #[error("failed to check for Active pins: {0}")]
    PinStoreCheckFailed(#[from] crate::pin_store::PinStoreError),
}

pub struct RetentionConfig {
    pub manifest_path: std::path::PathBuf,
    pub retention_ledger_journal_path: std::path::PathBuf,
    pub active_segment_journal_path: std::path::PathBuf,
    pub retirement_authorized_by: String,
    /// M7.7.3: the pin store's own locator/journal path — ALWAYS
    /// supplied explicitly by the caller (e.g. derived from a trusted
    /// segmentation root), NEVER read from `manifest.retention_ledger_locator`
    /// or any other manifest field. See `crate::pin_store`'s own doc
    /// comment for why this crate never lets the manifest determine
    /// the pin store's own authority.
    pub pin_store_locator: std::path::PathBuf,
    pub pin_store_journal_path: std::path::PathBuf,
}

#[derive(Debug)]
pub struct RetentionReport {
    pub retired_segment_id: SegmentId,
    pub retention_ledger_sequence: u64,
}

/// Runs the full 5-phase retention protocol for `target_segment_id`.
/// Idempotent and crash-safe: calling this again after a crash at any
/// phase boundary resumes from exactly where the durable proof left
/// off (see `crate::recovery` for the full matrix); this function
/// itself always re-derives its own state from the real manifest/chain/
/// ledger rather than assuming a clean start.
pub async fn retire_segment(config: &RetentionConfig, target_segment_id: &SegmentId) -> Result<RetentionReport, RetentionError> {
    let _lock = crate::lifecycle_lock::acquire(&config.manifest_path)?;

    let manifest = crate::manifest_store::load(&config.manifest_path).map_err(RetentionError::ManifestLoadFailed)?.ok_or(RetentionError::NoManifest)?;
    let target = manifest.find(target_segment_id).ok_or_else(|| RetentionError::TargetSegmentNotFound(target_segment_id.clone()))?.clone();

    // The frozen gate: checked BEFORE phase 1, so an ineligible segment
    // (the legacy one, permanently) never gets even a
    // RetentionIntentAttempted record appended for it.
    if !target.retention_eligible {
        return Err(RetentionError::NotRetentionEligible(target_segment_id.clone()));
    }
    if target.status != crate::domain::SegmentStatus::Closed {
        return Err(RetentionError::TargetSegmentNotClosed(target_segment_id.clone()));
    }

    // M7.7.3 frozen requirement: zero Active pins, checked BEFORE
    // phase 1 -- a structural gate, never a composition-root
    // convention. No audit mutation (not even RetentionIntentAttempted)
    // has happened yet at this point.
    let (_pin_store_vault, pin_store_state) = crate::pin_store::verify_and_scan(&config.pin_store_locator, &config.pin_store_journal_path)?;
    let active_pins = pin_store_state.active_pins_for(target_segment_id)?;
    if !active_pins.is_empty() {
        return Err(RetentionError::SegmentHasActivePins(target_segment_id.clone(), active_pins.iter().map(|p| p.pin_id).collect()));
    }

    let target_final_sequence = target.final_sequence.expect("SegmentManifest::validate guarantees a Closed segment always carries final_sequence");
    let target_final_record_hash = target.final_record_hash.clone().expect("SegmentManifest::validate guarantees a Closed segment always carries final_record_hash");

    let active = manifest.active_segment().ok_or(RetentionError::NoActiveSegment)?.clone();

    // Resume-aware: check what has already durably happened before
    // attempting any phase again.
    let active_state = scan_active_segment(&active, &config.active_segment_journal_path)?;
    let already_authorized = find_authorized_record(&active_state, target_segment_id);

    if let Some(authorized) = already_authorized {
        // Phases 1-3 already durably completed in a prior run. Resume
        // at phase 4.
        return finish_from_authorized(config, &manifest, &target, authorized.retention_ledger_sequence).await;
    }

    let already_intended = find_intent_record(&active_state, target_segment_id).is_some();

    // Phase 1: RetentionIntentAttempted (skip if a prior run already
    // appended it durably -- never re-append, which would itself be a
    // detectable anomaly down the line).
    if !already_intended {
        append_to_active(
            config,
            &active,
            brain_api::AuditEventKind::RetentionIntentAttempted { target_segment_id: target_segment_id.to_string(), target_segment_final_sequence: target_final_sequence, target_segment_final_record_hash: target_final_record_hash.clone() },
        )
        .await?;
    }

    // Phase 2: durable RetentionReceipt in the ledger.
    let receipt = RetentionReceipt { retired_segment_id: target_segment_id.clone(), retired_segment_final_sequence: target_final_sequence, retired_segment_final_record_hash: target_final_record_hash.clone(), retirement_authorized_by: config.retirement_authorized_by.clone(), retired_at: chrono::Utc::now().to_rfc3339() };
    let (ledger_sequence, ledger_record_hash) = crate::ledger::append_receipt(&manifest.retention_ledger_locator, &config.retention_ledger_journal_path, &receipt)?;

    // Phase 3: RetentionAuthorized, committing to the receipt's own position.
    append_to_active(config, &active, brain_api::AuditEventKind::RetentionAuthorized { target_segment_id: target_segment_id.to_string(), retention_ledger_sequence: ledger_sequence, retention_ledger_record_hash: ledger_record_hash }).await?;

    finish_from_authorized(config, &manifest, &target, ledger_sequence).await
}

async fn finish_from_authorized(config: &RetentionConfig, manifest: &crate::domain::SegmentManifest, target: &SegmentManifestEntry, retention_ledger_sequence: u64) -> Result<RetentionReport, RetentionError> {
    // Phase 4: delete ONLY the dedicated segment directory -- never the
    // legacy vault root. `retention_eligible` already gates this at
    // entry, and this is the ONE place this crate ever calls
    // `remove_dir_all` on a segment's own `locator` -- never
    // generalized into a helper a non-retention code path could reuse
    // by accident.
    if target.locator.exists() {
        std::fs::remove_dir_all(&target.locator).map_err(RetentionError::PayloadDeleteFailed)?;
    }

    // Phase 5: manifest projection only.
    let mut updated = manifest.clone();
    for entry in updated.segments.iter_mut() {
        if entry.segment_id == target.segment_id {
            entry.retained_payload_present = false;
        }
    }
    crate::manifest_store::save(&config.manifest_path, &updated).map_err(RetentionError::ManifestSaveFailed)?;

    Ok(RetentionReport { retired_segment_id: target.segment_id.clone(), retention_ledger_sequence })
}

async fn append_to_active(config: &RetentionConfig, active: &SegmentManifestEntry, kind: brain_api::AuditEventKind) -> Result<(), RetentionError> {
    let vault = storage::Vault::open_with_default_indexer(active.locator.clone(), &config.active_segment_journal_path).map_err(RetentionError::ActiveSegmentVaultOpenFailed)?;
    let worker = brain_api_audit_worker::spawn_worker(vault, brain_api_audit_worker::WorkerConfig::default()).map_err(RetentionError::ActiveSegmentWriterStartupFailed)?;
    let event = brain_api::event(kind, &chrono::Utc::now().to_rfc3339(), uuid::Uuid::new_v4());
    worker.record_durable(event).await.map_err(RetentionError::ActiveSegmentAppendFailed)?;
    let _ = worker.shutdown().await;
    Ok(())
}

struct ActiveSegmentState {
    verified_records: Vec<audit_chain_core::ChainRecord>,
}

fn scan_active_segment(active: &SegmentManifestEntry, journal_path: &std::path::Path) -> Result<ActiveSegmentState, RetentionError> {
    let vault = storage::Vault::open_with_default_indexer(active.locator.clone(), journal_path).map_err(RetentionError::ActiveSegmentVaultOpenFailed)?;
    let raw = vault.read_audit_log_for_recovery_scan().map_err(RetentionError::ActiveSegmentVaultOpenFailed)?;
    let scan = scan_chain_bytes(&raw);
    Ok(ActiveSegmentState { verified_records: scan.verified_records })
}

struct DecodedAuthorized {
    retention_ledger_sequence: u64,
}

fn find_authorized_record(state: &ActiveSegmentState, target_segment_id: &SegmentId) -> Option<DecodedAuthorized> {
    state.verified_records.iter().rev().find_map(|r| {
        let event: brain_api::AuditEvent = audit_chain_core::parse_event(&r.content_bytes).ok()?;
        match event.kind {
            brain_api::AuditEventKind::RetentionAuthorized { target_segment_id: t, retention_ledger_sequence, .. } if t == target_segment_id.as_str() => Some(DecodedAuthorized { retention_ledger_sequence }),
            _ => None,
        }
    })
}

fn find_intent_record(state: &ActiveSegmentState, target_segment_id: &SegmentId) -> Option<()> {
    state.verified_records.iter().find_map(|r| {
        let event: brain_api::AuditEvent = audit_chain_core::parse_event(&r.content_bytes).ok()?;
        match event.kind {
            brain_api::AuditEventKind::RetentionIntentAttempted { target_segment_id: t, .. } if t == target_segment_id.as_str() => Some(()),
            _ => None,
        }
    })
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
    use crate::rotation::{rotate, RotationConfig};

    struct Harness {
        dir: tempfile::TempDir,
        manifest_path: std::path::PathBuf,
        legacy_locator: std::path::PathBuf,
        retention_ledger_locator: std::path::PathBuf,
        pin_store_locator: std::path::PathBuf,
        segment_1_locator: std::path::PathBuf,
    }

    impl Harness {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let manifest_path = dir.path().join("segment-manifest.json");
            let legacy_locator = dir.path().join("segment-0");
            let retention_ledger_locator = dir.path().join("retention-ledger");
            let pin_store_locator = dir.path().join("pin-store");
            let segment_1_locator = dir.path().join("segment-1");
            std::fs::create_dir_all(&legacy_locator).unwrap();
            std::fs::create_dir_all(&retention_ledger_locator).unwrap();
            std::fs::create_dir_all(&pin_store_locator).unwrap();
            crate::adoption::adopt_existing_vault(&manifest_path, &legacy_locator, &retention_ledger_locator).unwrap();
            Self { dir, manifest_path, legacy_locator, retention_ledger_locator, pin_store_locator, segment_1_locator }
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

        fn rotation_config(&self) -> RotationConfig {
            RotationConfig { manifest_path: self.manifest_path.clone(), retention_ledger_journal_path: self.retention_ledger_locator.join("journal.sqlite"), next_segment_locator: self.segment_1_locator.clone(), next_segment_journal_path: self.segment_1_locator.join("journal.sqlite") }
        }

        fn retention_config(&self) -> RetentionConfig {
            RetentionConfig { manifest_path: self.manifest_path.clone(), retention_ledger_journal_path: self.retention_ledger_locator.join("journal.sqlite"), active_segment_journal_path: self.segment_1_locator.join("journal.sqlite"), retirement_authorized_by: "policy:test".into(), pin_store_locator: self.pin_store_locator.clone(), pin_store_journal_path: self.pin_store_locator.join("journal.sqlite") }
        }

        /// Rotates once (closing segment 0, opening segment 1 Active),
        /// so there is a Closed, retention_eligible-by-default segment
        /// 0... except segment 0 is the LEGACY one, permanently
        /// ineligible -- for tests that need a genuinely retirable
        /// Closed segment, this harness rotates TWICE, so segment 1
        /// (created by rotation, eligible by default) becomes Closed.
        async fn rotate_twice(&self) {
            self.append(&self.legacy_locator, brain_api::AuditEventKind::McpConnection { server: "s0".into() });
            rotate(&self.rotation_config()).await.unwrap();
            self.append(&self.segment_1_locator, brain_api::AuditEventKind::McpConnection { server: "s1".into() });
            let segment_2_locator = self.dir.path().join("segment-2");
            let config = RotationConfig { manifest_path: self.manifest_path.clone(), retention_ledger_journal_path: self.retention_ledger_locator.join("journal.sqlite"), next_segment_locator: segment_2_locator.clone(), next_segment_journal_path: segment_2_locator.join("journal.sqlite") };
            rotate(&config).await.unwrap();
        }
    }

    fn active_segment_journal_after_second_rotation(h: &Harness) -> std::path::PathBuf {
        h.dir.path().join("segment-2").join("journal.sqlite")
    }

    // --- the explicitly required test: legacy segment retention is
    //     refused BEFORE any audit mutation ---

    #[tokio::test]
    async fn legacy_segment_retention_is_denied_before_any_audit_mutation() {
        let h = Harness::new();
        h.append(&h.legacy_locator, brain_api::AuditEventKind::McpConnection { server: "s".into() });
        rotate(&h.rotation_config()).await.unwrap(); // closes segment 0 (legacy), opens segment 1 (Active)

        let before = {
            let journal = h.legacy_locator.join("journal.sqlite");
            let vault = storage::Vault::open_with_default_indexer(h.legacy_locator.clone(), &journal).unwrap();
            vault.read_audit_log_for_recovery_scan().unwrap()
        };

        let result = retire_segment(&h.retention_config(), &SegmentId::from_index(0)).await;
        assert!(matches!(result, Err(RetentionError::NotRetentionEligible(_))), "got legacy retention result");

        // The legacy segment's own chain must be byte-for-byte
        // unchanged -- no RetentionIntentAttempted, no anything.
        let after = {
            let journal = h.legacy_locator.join("journal.sqlite");
            let vault = storage::Vault::open_with_default_indexer(h.legacy_locator.clone(), &journal).unwrap();
            vault.read_audit_log_for_recovery_scan().unwrap()
        };
        assert_eq!(before, after, "the legacy segment's own chain must never be touched by a rejected retention attempt");

        // The Active segment (segment 1) must ALSO be untouched -- the
        // rejection happens before phase 1, which would otherwise
        // append to segment 1, not segment 0.
        let active_journal = h.segment_1_locator.join("journal.sqlite");
        let active_vault = storage::Vault::open_with_default_indexer(h.segment_1_locator.clone(), &active_journal).unwrap();
        let active_raw = active_vault.read_audit_log_for_recovery_scan().unwrap();
        let active_scan = scan_chain_bytes(&active_raw);
        assert_eq!(active_scan.verified_records.len(), 1, "only SegmentLinkage, from rotation itself -- no RetentionIntentAttempted was ever appended");
    }

    #[tokio::test]
    async fn normal_retention_of_an_eligible_closed_segment_succeeds() {
        let h = Harness::new();
        h.rotate_twice().await;

        let mut config = h.retention_config();
        config.active_segment_journal_path = active_segment_journal_after_second_rotation(&h);
        let report = retire_segment(&config, &SegmentId::from_index(1)).await.unwrap();
        assert_eq!(report.retired_segment_id, SegmentId::from_index(1));

        assert!(!h.segment_1_locator.exists(), "the retired segment's own payload directory must be deleted");

        let manifest = crate::manifest_store::load(&h.manifest_path).unwrap().unwrap();
        assert!(!manifest.find(&SegmentId::from_index(1)).unwrap().retained_payload_present);
    }

    #[tokio::test]
    async fn retiring_a_segment_that_is_still_active_is_refused() {
        let h = Harness::new();
        h.append(&h.legacy_locator, brain_api::AuditEventKind::McpConnection { server: "s".into() });
        rotate(&h.rotation_config()).await.unwrap();

        // Segment 1 is now Active, not Closed.
        let result = retire_segment(&h.retention_config(), &SegmentId::from_index(1)).await;
        assert!(matches!(result, Err(RetentionError::TargetSegmentNotClosed(_))));
    }

    #[tokio::test]
    async fn a_target_segment_not_present_in_the_manifest_is_refused() {
        let h = Harness::new();
        let result = retire_segment(&h.retention_config(), &SegmentId::from_index(99)).await;
        assert!(matches!(result, Err(RetentionError::TargetSegmentNotFound(_))));
    }

    #[tokio::test]
    async fn resuming_retention_after_authorized_was_written_but_before_delete_completes_the_delete() {
        let h = Harness::new();
        h.rotate_twice().await;
        let mut config = h.retention_config();
        config.active_segment_journal_path = active_segment_journal_after_second_rotation(&h);

        // Drive phases 1-3 manually (mirrors what retire_segment itself
        // would do), WITHOUT calling finish_from_authorized -- modeling
        // "crashed after phase 3, before phase 4."
        let manifest = crate::manifest_store::load(&h.manifest_path).unwrap().unwrap();
        let target = manifest.find(&SegmentId::from_index(1)).unwrap().clone();
        let active = manifest.active_segment().unwrap().clone();
        append_to_active(&config, &active, brain_api::AuditEventKind::RetentionIntentAttempted { target_segment_id: target.segment_id.to_string(), target_segment_final_sequence: target.final_sequence.unwrap(), target_segment_final_record_hash: target.final_record_hash.clone().unwrap() }).await.unwrap();
        let receipt = RetentionReceipt { retired_segment_id: target.segment_id.clone(), retired_segment_final_sequence: target.final_sequence.unwrap(), retired_segment_final_record_hash: target.final_record_hash.clone().unwrap(), retirement_authorized_by: "policy:test".into(), retired_at: "2026-01-01T00:00:00Z".into() };
        let (ledger_seq, ledger_hash) = crate::ledger::append_receipt(&manifest.retention_ledger_locator, &config.retention_ledger_journal_path, &receipt).unwrap();
        append_to_active(&config, &active, brain_api::AuditEventKind::RetentionAuthorized { target_segment_id: target.segment_id.to_string(), retention_ledger_sequence: ledger_seq, retention_ledger_record_hash: ledger_hash }).await.unwrap();

        assert!(h.segment_1_locator.exists(), "payload must still exist at this crash point");

        // Resume: must detect the already-written RetentionAuthorized
        // and complete only phases 4-5, never re-append phases 1-3.
        let report = retire_segment(&config, &SegmentId::from_index(1)).await.unwrap();
        assert_eq!(report.retired_segment_id, SegmentId::from_index(1));
        assert!(!h.segment_1_locator.exists());

        // Confirm no duplicate RetentionIntentAttempted/RetentionAuthorized exist.
        let active_journal = active_segment_journal_after_second_rotation(&h);
        let active_vault = storage::Vault::open_with_default_indexer(active.locator.clone(), &active_journal).unwrap();
        let raw = active_vault.read_audit_log_for_recovery_scan().unwrap();
        let scan = scan_chain_bytes(&raw);
        let intent_count = scan.verified_records.iter().filter(|r| matches!(audit_chain_core::parse_event::<brain_api::AuditEvent>(&r.content_bytes).map(|e| e.kind), Ok(brain_api::AuditEventKind::RetentionIntentAttempted { .. }))).count();
        let authorized_count = scan.verified_records.iter().filter(|r| matches!(audit_chain_core::parse_event::<brain_api::AuditEvent>(&r.content_bytes).map(|e| e.kind), Ok(brain_api::AuditEventKind::RetentionAuthorized { .. }))).count();
        assert_eq!(intent_count, 1, "resuming must never duplicate RetentionIntentAttempted");
        assert_eq!(authorized_count, 1, "resuming must never duplicate RetentionAuthorized");
    }

    // --- M7.7.3: retention denied by an Active pin, before any audit mutation ---

    #[tokio::test]
    async fn retention_is_denied_before_any_audit_mutation_when_the_target_segment_has_an_active_pin() {
        let h = Harness::new();
        h.rotate_twice().await;
        let active_journal = active_segment_journal_after_second_rotation(&h);
        let mut config = h.retention_config();
        config.active_segment_journal_path = active_journal.clone();

        let pin_id = crate::pin_store::acquire(&h.pin_store_locator, &config.pin_store_journal_path, &SegmentId::from_index(1), crate::domain::OwnerClass::new("mcp-audit-bridge").unwrap(), crate::domain::PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();

        let active_locator = h.dir.path().join("segment-2");
        let before_count = {
            let vault = storage::Vault::open_with_default_indexer(active_locator.clone(), &active_journal).unwrap();
            scan_chain_bytes(&vault.read_audit_log_for_recovery_scan().unwrap()).verified_records.len()
        };

        let result = retire_segment(&config, &SegmentId::from_index(1)).await;
        assert!(matches!(result, Err(RetentionError::SegmentHasActivePins(ref id, ref pins)) if id == &SegmentId::from_index(1) && pins.contains(&pin_id)), "got {result:?}");

        // No audit mutation at all -- not even RetentionIntentAttempted.
        let after_count = {
            let vault = storage::Vault::open_with_default_indexer(active_locator, &active_journal).unwrap();
            scan_chain_bytes(&vault.read_audit_log_for_recovery_scan().unwrap()).verified_records.len()
        };
        assert_eq!(before_count, after_count, "a pin-denied retention must append zero records to the Active segment");
        assert!(h.segment_1_locator.exists(), "the target segment's payload must remain untouched");
    }

    #[tokio::test]
    async fn retention_succeeds_once_the_active_pin_is_released() {
        let h = Harness::new();
        h.rotate_twice().await;
        let mut config = h.retention_config();
        config.active_segment_journal_path = active_segment_journal_after_second_rotation(&h);

        let pin_id = crate::pin_store::acquire(&h.pin_store_locator, &config.pin_store_journal_path, &SegmentId::from_index(1), crate::domain::OwnerClass::new("mcp-audit-bridge").unwrap(), crate::domain::PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();

        let denied = retire_segment(&config, &SegmentId::from_index(1)).await;
        assert!(matches!(denied, Err(RetentionError::SegmentHasActivePins(_, _))));

        crate::pin_store::release(&h.pin_store_locator, &config.pin_store_journal_path, pin_id, "2026-01-01T00:05:00Z").unwrap();

        let report = retire_segment(&config, &SegmentId::from_index(1)).await.unwrap();
        assert_eq!(report.retired_segment_id, SegmentId::from_index(1));
        assert!(!h.segment_1_locator.exists());
    }

    #[tokio::test]
    async fn the_full_load_bearing_crash_scenario_pin_append_crash_rotation_retention_denied_then_recovery() {
        // The scenario from the M7.7 design memo, step by step:
        // 1. "bridge" acquires a durable pin on segment N (here: segment 1, the Active segment at the time);
        // 2. an event is appended to N (modeling the bridge's own append);
        // 3. (crash before checkpoint -- modeled by simply not releasing the pin yet);
        // 4. N is rotated and becomes Closed;
        // 5. retention on N is attempted;
        // 6. retention is denied because the pin is still Active;
        // 7. "bridge restart": the pin is still there, the append is still there;
        // 8. a verified scan confirms the appended fact is still present;
        // 9/10. the pin is released (simulating successful recovery+checkpoint);
        // 11. retention can now proceed.
        let h = Harness::new();
        h.append(&h.legacy_locator, brain_api::AuditEventKind::McpConnection { server: "s0".into() });
        rotate(&h.rotation_config()).await.unwrap();

        let pin_store_journal = h.pin_store_locator.join("journal.sqlite");
        let segment_1_journal = h.segment_1_locator.join("journal.sqlite");

        // 1. Pin acquired on segment 1 (the current Active segment).
        let pin_id = crate::pin_store::acquire(&h.pin_store_locator, &pin_store_journal, &SegmentId::from_index(1), crate::domain::OwnerClass::new("mcp-audit-bridge").unwrap(), crate::domain::PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();

        // 2. Append X into segment 1 (modeling the bridge's own durable append).
        h.append(&h.segment_1_locator, brain_api::AuditEventKind::McpConnection { server: "X".into() });

        // 3. Crash before checkpoint -- nothing to do, the pin is already durable and X is already durable.

        // 4. Rotate segment 1 to Closed, segment 2 becomes Active.
        let segment_2_locator = h.dir.path().join("segment-2");
        let rotate_config = RotationConfig { manifest_path: h.manifest_path.clone(), retention_ledger_journal_path: h.retention_ledger_locator.join("journal.sqlite"), next_segment_locator: segment_2_locator.clone(), next_segment_journal_path: segment_2_locator.join("journal.sqlite") };
        rotate(&rotate_config).await.unwrap();

        // 5/6. Retention on segment 1 is attempted and denied.
        let mut config = h.retention_config();
        config.active_segment_journal_path = segment_2_locator.join("journal.sqlite");
        let denied = retire_segment(&config, &SegmentId::from_index(1)).await;
        assert!(matches!(denied, Err(RetentionError::SegmentHasActivePins(ref id, ref pins)) if id == &SegmentId::from_index(1) && pins.contains(&pin_id)), "got {denied:?}");
        assert!(h.segment_1_locator.exists(), "segment 1's payload, containing X, must still exist");

        // 7/8. "Restart": scan segment 1 and confirm X is still there.
        // The vault handle is dropped immediately after reading (never
        // held across the later retire_segment call below), since
        // retire_segment's own Phase 4 deletes this exact directory --
        // an open handle on Windows would otherwise block that delete,
        // which would itself be a real bug in a real composition root
        // too (never read a segment's vault and hold it open across a
        // retention attempt on that same segment).
        let found_x = {
            let vault = storage::Vault::open_with_default_indexer(h.segment_1_locator.clone(), &segment_1_journal).unwrap();
            let raw = vault.read_audit_log_for_recovery_scan().unwrap();
            let scan = scan_chain_bytes(&raw);
            scan.verified_records.iter().any(|r| matches!(audit_chain_core::parse_event::<brain_api::AuditEvent>(&r.content_bytes), Ok(e) if matches!(e.kind, brain_api::AuditEventKind::McpConnection { ref server } if server == "X")))
        };
        assert!(found_x, "X must still be recoverable from segment 1 after the simulated crash and failed retention attempt");

        // 9/10. Release the pin (simulating successful recovery + checkpoint save).
        crate::pin_store::release(&h.pin_store_locator, &pin_store_journal, pin_id, "2026-01-01T00:10:00Z").unwrap();

        // 11. Retention can now proceed.
        let report = retire_segment(&config, &SegmentId::from_index(1)).await.unwrap();
        assert_eq!(report.retired_segment_id, SegmentId::from_index(1));
        assert!(!h.segment_1_locator.exists(), "segment 1's payload may now be deleted");
    }
}
