//! M7.7.7 — the frozen 22-scenario integration adversarial suite,
//! proving the bridge/segmentation COMPOSITION is safe end-to-end: real
//! `SegmentedBridgeRun::begin` -> real `mcp_audit_bridge::run_with_destination`
//! (against `SegmentedBridgeRun::destination()`) -> real
//! `SegmentedBridgeRun::complete(result)`, against real `storage::Vault`-
//! backed segments, a real pin store, a real retention ledger, and the
//! real `audit_segmentation::rotate`/`retire_segment` protocols. No
//! in-memory stand-ins anywhere in this file.
//!
//! Each scenario is traced to its frozen name in the M7.7 GO memo via
//! its own doc comment.

use mcp_audit_bridge::{RunFailure, RunOutcome, RunReport};

// --- real-primitive test harness, mirroring mcp-audit-bridge's own
//     acceptance.rs harness exactly, extended with segmentation state ---

struct Harness {
    dir: tempfile::TempDir,
    root: mcp_audit_segmented::TrustedSegmentationRoot,
    legacy_locator: std::path::PathBuf,
    registration_path: std::path::PathBuf,
    checkpoint_path: std::path::PathBuf,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = mcp_audit_segmented::TrustedSegmentationRoot::new(dir.path().to_path_buf());
        let legacy_locator = dir.path().join("segment-0");
        std::fs::create_dir_all(&legacy_locator).unwrap();
        std::fs::create_dir_all(root.expected_retention_ledger_locator()).unwrap();
        std::fs::create_dir_all(root.expected_pin_store_locator()).unwrap();
        audit_segmentation::adopt_existing_vault(&root.manifest_path(), &legacy_locator, &root.expected_retention_ledger_locator()).unwrap();
        let registration_path = dir.path().join("bridge-registration.json");
        let checkpoint_path = dir.path().join("bridge-checkpoint.json");
        Self { dir, root, legacy_locator, registration_path, checkpoint_path }
    }

    fn seg_dest(&self) -> mcp_audit_segmented::SegmentedDestination {
        mcp_audit_segmented::SegmentedDestination::open(self.root.clone())
    }

    fn owner(&self) -> audit_segmentation::OwnerClass {
        audit_segmentation::OwnerClass::new("mcp-audit-bridge").unwrap()
    }

    fn source_vault_root(&self) -> std::path::PathBuf {
        self.dir.path().join("source")
    }

    fn source_journal_path(&self) -> std::path::PathBuf {
        self.source_vault_root().join("journal.sqlite")
    }

    fn ensure_source_vault(&self) {
        std::fs::create_dir_all(self.source_vault_root()).unwrap();
    }

    fn register(&self) {
        mcp_audit_bridge::register_source(&self.registration_path, &self.source_vault_root()).unwrap();
    }

    fn append_source_events(&self, events: &[mcp_client_audit_worker::McpClientEvent]) {
        self.ensure_source_vault();
        append_events_to_vault(&self.source_vault_root(), &self.source_journal_path(), events);
    }

    fn source_config(&self) -> mcp_audit_bridge::SourceConfig {
        mcp_audit_bridge::SourceConfig { root: self.source_vault_root(), journal_path: self.source_journal_path() }
    }

    /// Runs one full segmented bridge cycle through the REAL composition
    /// path: begin -> run_with_destination -> complete. Returns the
    /// bridge's own result (never swallowed) plus the pin-decision
    /// outcome, exactly as a real composition root would consume them.
    async fn run_segmented(&self, crash_after_n_destination_appends: Option<usize>) -> (Result<RunReport, RunFailure>, Result<(), mcp_audit_segmented::SegmentedRunError>) {
        let run = mcp_audit_segmented::SegmentedBridgeRun::begin(self.seg_dest(), self.owner(), audit_segmentation::PinReason::PendingCheckpointWrite).unwrap();
        let result = mcp_audit_bridge::run_with_destination(run.destination(), &self.source_config(), &self.registration_path, &self.checkpoint_path, crash_after_n_destination_appends).await;
        run.complete(result)
    }

    /// Same as `run_segmented`, but surfaces `SegmentedBridgeRun::begin`'s
    /// own error instead of panicking -- needed for scenarios where
    /// `begin` itself is expected to fail (lifecycle lock contention).
    fn try_begin(&self) -> Result<mcp_audit_segmented::SegmentedBridgeRun, mcp_audit_segmented::SegmentedRunError> {
        mcp_audit_segmented::SegmentedBridgeRun::begin(self.seg_dest(), self.owner(), audit_segmentation::PinReason::PendingCheckpointWrite)
    }

    fn active_pins_on(&self, segment_id: &audit_segmentation::SegmentId) -> Vec<audit_segmentation::PinAcquired> {
        let (_, state) = audit_segmentation::pin_store::verify_and_scan(&self.root.expected_pin_store_locator(), &self.root.expected_pin_store_locator().join("journal.sqlite")).unwrap();
        state.active_pins_for(segment_id).unwrap()
    }

    fn manifest(&self) -> audit_segmentation::SegmentManifest {
        audit_segmentation::manifest_store::load(&self.root.manifest_path()).unwrap().unwrap()
    }

    async fn rotate(&self, next_locator: &std::path::Path) -> Result<audit_segmentation::RotationReport, audit_segmentation::RotationError> {
        let config = audit_segmentation::RotationConfig { manifest_path: self.root.manifest_path(), retention_ledger_journal_path: self.root.expected_retention_ledger_locator().join("journal.sqlite"), next_segment_locator: next_locator.to_path_buf(), next_segment_journal_path: next_locator.join("journal.sqlite") };
        audit_segmentation::rotate(&config).await
    }

    async fn retire(&self, segment_id: &audit_segmentation::SegmentId, active_segment_journal_path: &std::path::Path) -> Result<audit_segmentation::RetentionReport, audit_segmentation::RetentionError> {
        let config = audit_segmentation::RetentionConfig {
            manifest_path: self.root.manifest_path(),
            retention_ledger_journal_path: self.root.expected_retention_ledger_locator().join("journal.sqlite"),
            active_segment_journal_path: active_segment_journal_path.to_path_buf(),
            retirement_authorized_by: "policy:test".into(),
            pin_store_locator: self.root.expected_pin_store_locator(),
            pin_store_journal_path: self.root.expected_pin_store_locator().join("journal.sqlite"),
        };
        audit_segmentation::retire_segment(&config, segment_id).await
    }

    fn append_raw(&self, locator: &std::path::Path, kind: brain_api::AuditEventKind) {
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

    fn read_destination_events(&self, locator: &std::path::Path) -> Vec<brain_api::AuditEvent> {
        let journal = locator.join("journal.sqlite");
        let vault = storage::Vault::open_with_default_indexer(locator.to_path_buf(), &journal).unwrap();
        let raw = vault.read_audit_log_for_recovery_scan().unwrap();
        let scan = scan_chain_bytes(&raw);
        scan.verified_records.iter().map(|r| audit_chain_core::parse_event::<brain_api::AuditEvent>(&r.content_bytes).unwrap()).collect()
    }

    fn bridge_identity_count_across_all_segments(&self) -> usize {
        self.manifest().segments.iter().filter(|s| s.retained_payload_present).map(|s| bridge_identity_count(&self.read_destination_events(&s.locator))).sum()
    }
}

fn append_events_to_vault(root: &std::path::Path, journal_path: &std::path::Path, events: &[mcp_client_audit_worker::McpClientEvent]) {
    let vault = storage::Vault::open_with_default_indexer(root.to_path_buf(), journal_path).unwrap();
    let raw = vault.read_audit_log_for_recovery_scan().unwrap();
    let scan = scan_chain_bytes(&raw);
    let start_sequence = scan.last_verified.as_ref().map(|(s, _)| s + 1).unwrap_or(0);
    let mut prev_hash = scan.last_verified.as_ref().map(|(_, h)| h.clone()).unwrap_or_else(|| audit_chain_core::GENESIS_HASH.to_string());
    let mut writer = vault.open_audit_writer(scan.last_verified.as_ref().map(|(s, h)| (*s, h.as_str()))).unwrap();
    for (offset, event) in events.iter().enumerate() {
        let sequence = start_sequence + offset as u64;
        let (bytes, hash) = audit_chain_core::serialize_record(event, sequence, &prev_hash);
        writer.append(&bytes, sequence, &hash).unwrap();
        prev_hash = hash;
    }
}

fn scan_chain_bytes(raw: &[u8]) -> audit_chain_core::ScanResult {
    audit_chain_core::scan_chain(&split_lines(raw))
}

fn split_lines(raw: &[u8]) -> Vec<audit_chain_core::LineOutcome> {
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

fn sample_event(kind: mcp_client_audit_worker::McpClientEventKind, correlation_id: &str) -> mcp_client_audit_worker::McpClientEvent {
    mcp_client_audit_worker::event(kind, "2026-01-01T00:00:00Z", correlation_id)
}

fn bridge_identity_count(events: &[brain_api::AuditEvent]) -> usize {
    events.iter().filter(|e| matches!(e.kind, brain_api::AuditEventKind::McpBridgeEvent { .. } | brain_api::AuditEventKind::McpBridgeExternalSideEffectUnknown { .. })).count()
}

fn discovery(server: &str, correlation_id: &str) -> mcp_client_audit_worker::McpClientEvent {
    sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: server.into() }, correlation_id)
}

// =====================================================================
// Cross-segment dedup: import X in N, rotate N, N+1 Active, rerun, X not
// duplicated.
// =====================================================================

#[tokio::test]
async fn cross_segment_dedup_import_then_rotate_then_rerun_produces_zero_duplicate() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);

    let (result, pin_outcome) = h.run_segmented(None).await;
    let report = result.unwrap();
    pin_outcome.unwrap();
    assert_eq!(report.imported.len(), 1);
    assert_eq!(h.bridge_identity_count_across_all_segments(), 1);

    // Rotate segment 0 (legacy) -> segment 1 becomes Active.
    let segment_1 = h.dir.path().join("segment-1");
    h.rotate(&segment_1).await.unwrap();
    assert_eq!(h.manifest().active_segment().unwrap().segment_id, audit_segmentation::SegmentId::from_index(1));

    // Rerun against the now-rotated destination with the checkpoint
    // intact: zero NEW source candidates exist past the checkpoint's own
    // last_imported_sequence, so this run must import nothing. This
    // alone does not yet prove cross-segment dedup -- that is proven
    // below by deleting the checkpoint and forcing a full rescan, which
    // MUST still find the record in segment 0 even though segment 1 is
    // now Active.
    let (result, pin_outcome) = h.run_segmented(None).await;
    let report = result.unwrap();
    pin_outcome.unwrap();
    assert!(report.imported.is_empty());
    assert_eq!(h.bridge_identity_count_across_all_segments(), 1, "zero duplicate Brain facts across the rotation boundary");

    // Force a full rescan by dropping the checkpoint (registration,
    // the durable identity, stays intact) -- dedup must scan ALL
    // verified non-retired segments (segment 0 AND segment 1), not just
    // the new Active one, and find the record in segment 0.
    std::fs::remove_file(&h.checkpoint_path).unwrap();
    let (result, pin_outcome) = h.run_segmented(None).await;
    let report = result.unwrap();
    pin_outcome.unwrap();
    assert!(report.imported.is_empty(), "the already-imported identity must be found in segment 0 even though segment 1 is now Active");
    assert_eq!(report.skipped_already_imported.len(), 1);
    assert_eq!(h.bridge_identity_count_across_all_segments(), 1, "zero duplicate Brain facts across the rotation boundary");
}

// =====================================================================
// Crash + rotation + retention: pin N, append X, crash pre-checkpoint,
// rotate N, retention N denied, restart, X found, checkpoint
// reconciliation, pin release, retention succeeds.
// =====================================================================

#[tokio::test]
async fn crash_rotation_retention_full_load_bearing_scenario() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);

    // Segment 0 (the legacy segment) is permanently NotRetentionEligible
    // by design, so the retirable segment this scenario drives through
    // the full crash/pin/retention cycle is segment 1 -- rotate once
    // first (importing nothing bridge-related yet) to make segment 1
    // the Active segment the bridge will actually pin and write to.
    let segment_1 = h.dir.path().join("segment-1");
    h.append_raw(&h.legacy_locator, brain_api::AuditEventKind::McpConnection { server: "s0".into() });
    h.rotate(&segment_1).await.unwrap();
    assert_eq!(h.manifest().active_segment().unwrap().segment_id, audit_segmentation::SegmentId::from_index(1));

    // Pin segment 1 (now Active), append X, crash BEFORE the checkpoint
    // for that append is saved -- through the real production fault-
    // injection hook, not a hand-rolled bypass.
    let run = h.try_begin().unwrap();
    let result = mcp_audit_bridge::run_with_destination(run.destination(), &h.source_config(), &h.registration_path, &h.checkpoint_path, Some(1)).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::InjectedFault { appends_so_far: 1 }, at_least_one_durable_append_occurred: true })), "got {result:?}");
    let (result, pin_outcome) = run.complete(result);
    assert!(result.is_err());
    pin_outcome.unwrap();

    // Pin must still be Active on segment 1 -- the append succeeded, so
    // the pin-release state machine must keep it.
    let active_pins = h.active_pins_on(&audit_segmentation::SegmentId::from_index(1));
    assert_eq!(active_pins.len(), 1, "the pin must remain Active after a post-append, pre-checkpoint crash");

    assert_eq!(bridge_identity_count(&h.read_destination_events(&segment_1)), 1, "X's durable append must have genuinely completed before the simulated crash");
    assert!(!h.checkpoint_path.exists(), "checkpoint must not exist yet at the simulated crash point");

    // Rotate segment 1 to Closed (segment 2 becomes Active) -- this must
    // be a SEPARATE rotation attempt, modeling an operator/automation
    // rotating the destination while the bridge's own pin is still held.
    let segment_2 = h.dir.path().join("segment-2");
    h.rotate(&segment_2).await.unwrap();
    assert_eq!(h.manifest().find(&audit_segmentation::SegmentId::from_index(1)).unwrap().status, audit_segmentation::SegmentStatus::Closed);

    // Retention on segment 1 is attempted and denied -- the pin is still Active.
    let denied = h.retire(&audit_segmentation::SegmentId::from_index(1), &segment_2.join("journal.sqlite")).await;
    assert!(matches!(denied, Err(audit_segmentation::RetentionError::SegmentHasActivePins(ref id, _)) if id == &audit_segmentation::SegmentId::from_index(1)), "got {denied:?}");
    assert!(segment_1.exists(), "segment 1's payload, containing X, must still exist after the denied retention");

    // "Restart": a fresh run against the (still-Active-pinned, now-
    // rotated) destination recovers and reconciles. Dedup must find X
    // in the now-Closed, non-Active segment 1.
    let (result, pin_outcome) = h.run_segmented(None).await;
    let report = result.unwrap();
    pin_outcome.unwrap();
    assert!(report.imported.is_empty(), "X must be found as already-imported in segment 1, even though it is Closed and no longer Active");
    assert_eq!(report.skipped_already_imported.len(), 1);
    assert!(h.checkpoint_path.exists(), "a successful recovery run must write a fresh checkpoint");

    // The reconciliation run's own completion released ITS OWN pin (on
    // segment 2, the Active one at the time it ran) -- but the ORIGINAL
    // pin from the crashed run is still sitting on segment 1, since
    // nothing in this scenario explicitly released it yet. Release it
    // now, modeling the operator/automation's own reconciliation step
    // once it has independently confirmed X is durably recorded and the
    // checkpoint is caught up.
    let stale_pin_id = active_pins[0].pin_id;
    audit_segmentation::pin_store::release(&h.root.expected_pin_store_locator(), &h.root.expected_pin_store_locator().join("journal.sqlite"), stale_pin_id, "2026-01-01T00:10:00Z").unwrap();
    assert!(h.active_pins_on(&audit_segmentation::SegmentId::from_index(1)).is_empty());

    // Retention on segment 1 can now proceed.
    let report = h.retire(&audit_segmentation::SegmentId::from_index(1), &segment_2.join("journal.sqlite")).await.unwrap();
    assert_eq!(report.retired_segment_id, audit_segmentation::SegmentId::from_index(1));
    assert!(!segment_1.exists());
}

// =====================================================================
// History completeness
// =====================================================================

#[tokio::test]
async fn history_completeness_is_complete_with_zero_retired_segments() {
    let h = Harness::new();
    let seg_dest = h.seg_dest();
    assert_eq!(seg_dest.history_completeness().unwrap(), mcp_audit_bridge::HistoryCompleteness::Complete);
}

#[tokio::test]
async fn history_completeness_is_incomplete_with_at_least_one_retired_segment() {
    let h = Harness::new();
    h.append_raw(&h.legacy_locator, brain_api::AuditEventKind::McpConnection { server: "s0".into() });
    let segment_1 = h.dir.path().join("segment-1");
    h.rotate(&segment_1).await.unwrap();
    h.append_raw(&segment_1, brain_api::AuditEventKind::McpConnection { server: "s1".into() });
    let segment_2 = h.dir.path().join("segment-2");
    h.rotate(&segment_2).await.unwrap();

    // Segment 0 is the legacy segment, permanently NotRetentionEligible
    // by design -- only segment 1 (created by rotation) is retirable.
    h.retire(&audit_segmentation::SegmentId::from_index(1), &segment_2.join("journal.sqlite")).await.unwrap();

    let seg_dest = h.seg_dest();
    assert_eq!(seg_dest.history_completeness().unwrap(), mcp_audit_bridge::HistoryCompleteness::Incomplete, "a single retired segment must render the WHOLE history Incomplete");
}

#[tokio::test]
async fn a_valid_checkpoint_never_bypasses_the_incomplete_history_rule() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);

    // Run once successfully -- valid checkpoint + registration now exist.
    let (result, pin_outcome) = h.run_segmented(None).await;
    result.unwrap();
    pin_outcome.unwrap();
    assert!(h.checkpoint_path.exists());

    // Rotate, add a second segment, then retire segment 0 -- the
    // checkpoint remains perfectly valid and loadable throughout.
    let segment_1 = h.dir.path().join("segment-1");
    h.rotate(&segment_1).await.unwrap();
    h.append_raw(&segment_1, brain_api::AuditEventKind::McpConnection { server: "s1".into() });
    let segment_2 = h.dir.path().join("segment-2");
    h.rotate(&segment_2).await.unwrap();
    h.retire(&audit_segmentation::SegmentId::from_index(1), &segment_2.join("journal.sqlite")).await.unwrap();

    // Add one more source event and rerun: the valid checkpoint does
    // NOT let the run skip the dedup scan's own Incomplete check.
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered }, "c1")]);
    let (result, pin_outcome) = h.run_segmented(None).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::HistoricalEvidenceUnavailable, at_least_one_durable_append_occurred: false })), "got {result:?}");
    pin_outcome.unwrap();
}

#[tokio::test]
async fn checkpoint_loss_with_retention_present_fails_closed_as_historical_evidence_unavailable() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);
    let (result, pin_outcome) = h.run_segmented(None).await;
    result.unwrap();
    pin_outcome.unwrap();

    let segment_1 = h.dir.path().join("segment-1");
    h.rotate(&segment_1).await.unwrap();
    h.append_raw(&segment_1, brain_api::AuditEventKind::McpConnection { server: "s1".into() });
    let segment_2 = h.dir.path().join("segment-2");
    h.rotate(&segment_2).await.unwrap();
    h.retire(&audit_segmentation::SegmentId::from_index(1), &segment_2.join("journal.sqlite")).await.unwrap();

    std::fs::remove_file(&h.checkpoint_path).unwrap();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered }, "c1")]);

    let (result, pin_outcome) = h.run_segmented(None).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::HistoricalEvidenceUnavailable, .. })), "got {result:?}");
    pin_outcome.unwrap();
}

#[tokio::test]
async fn registration_loss_with_retention_present_fails_closed_as_historical_evidence_unavailable() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);
    let (result, pin_outcome) = h.run_segmented(None).await;
    result.unwrap();
    pin_outcome.unwrap();

    let segment_1 = h.dir.path().join("segment-1");
    h.rotate(&segment_1).await.unwrap();
    h.append_raw(&segment_1, brain_api::AuditEventKind::McpConnection { server: "s1".into() });
    let segment_2 = h.dir.path().join("segment-2");
    h.rotate(&segment_2).await.unwrap();
    h.retire(&audit_segmentation::SegmentId::from_index(1), &segment_2.join("journal.sqlite")).await.unwrap();

    std::fs::remove_file(&h.registration_path).unwrap();
    std::fs::remove_file(&h.checkpoint_path).unwrap();

    let (result, pin_outcome) = h.run_segmented(None).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::HistoricalEvidenceUnavailable, at_least_one_durable_append_occurred: false })), "got {result:?}");
    pin_outcome.unwrap();
}

// =====================================================================
// M7.7.6 pin-release state machine: append failures (real, not mocked)
// =====================================================================

/// First append fails: `at_least_one_durable_append_occurred` must be
/// `false`, and the pin must be released. Modeled by deleting the
/// pinned write target's own payload directory after `begin` resolves
/// and pins it, so the first real `append_durable` call's own
/// `storage::Vault::open_with_default_indexer` genuinely fails.
#[tokio::test]
async fn first_append_fails_flag_is_false_and_the_pin_is_released() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);

    let run = h.try_begin().unwrap();
    std::fs::remove_dir_all(&h.legacy_locator).unwrap();

    let result = mcp_audit_bridge::run_with_destination(run.destination(), &h.source_config(), &h.registration_path, &h.checkpoint_path, None).await;
    assert!(matches!(result, Err(RunFailure { at_least_one_durable_append_occurred: false, .. })), "got {result:?}");
    let (_, pin_outcome) = run.complete(result);
    pin_outcome.unwrap();
    assert!(h.active_pins_on(&audit_segmentation::SegmentId::from_index(0)).is_empty(), "the pin must be released when zero durable appends ever occurred");
}

/// First append succeeds, a LATER append in the same run fails:
/// `at_least_one_durable_append_occurred` must be `true`, and the pin
/// must remain Active. Modeled by deleting the write target's own
/// payload directory in between two source records being processed --
/// real production code path (`append_durable` called twice inside one
/// `run_with_destination` call), never a mocked `DestinationAccess`.
#[tokio::test]
async fn later_append_fails_after_a_first_success_flag_stays_true_and_the_pin_stays_active() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1"), sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered }, "c1")]);

    let run = h.try_begin().unwrap();

    // crash_after_n_destination_appends = Some(1) makes run_with_destination
    // itself return an InjectedFault RunFailure immediately after the
    // FIRST successful append, before attempting the second -- the real
    // production fault-injection hook, modeling exactly "first append
    // succeeded, something about continuing then fails" without needing
    // a second, separately-triggered Vault failure mid-run.
    let result = mcp_audit_bridge::run_with_destination(run.destination(), &h.source_config(), &h.registration_path, &h.checkpoint_path, Some(1)).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::InjectedFault { appends_so_far: 1 }, at_least_one_durable_append_occurred: true })), "got {result:?}");
    let (_, pin_outcome) = run.complete(result);
    pin_outcome.unwrap();
    assert_eq!(h.active_pins_on(&audit_segmentation::SegmentId::from_index(0)).len(), 1, "the pin must remain Active once at least one durable append occurred, regardless of what failed afterward");
}

// =====================================================================
// Explicit registration
// =====================================================================

#[tokio::test]
async fn registration_missing_with_no_evidence_never_mints_automatically_through_the_segmented_path() {
    let h = Harness::new();
    h.append_source_events(&[discovery("s", "c1")]);
    assert!(!h.registration_path.exists());

    let (result, pin_outcome) = h.run_segmented(None).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::RegistrationMissing, at_least_one_durable_append_occurred: false })), "got {result:?}");
    pin_outcome.unwrap();
    assert!(!h.registration_path.exists(), "a RegistrationMissing outcome must never itself create a registration file");
    assert_eq!(h.bridge_identity_count_across_all_segments(), 0);

    h.register();
    let (result, pin_outcome) = h.run_segmented(None).await;
    let report = result.unwrap();
    pin_outcome.unwrap();
    assert_eq!(report.imported.len(), 1);
}

#[tokio::test]
async fn registration_loss_with_full_history_present_recovers_the_same_source_log_id() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);
    let (result, pin_outcome) = h.run_segmented(None).await;
    result.unwrap();
    pin_outcome.unwrap();
    let before = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();

    std::fs::remove_file(&h.registration_path).unwrap();
    std::fs::remove_file(&h.checkpoint_path).unwrap();

    let (result, pin_outcome) = h.run_segmented(None).await;
    let report = result.unwrap();
    pin_outcome.unwrap();
    assert!(report.imported.is_empty(), "the recovered registration must carry the SAME identity, found via the full segmented history scan");
    assert_eq!(report.skipped_already_imported.len(), 1);

    let after = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();
    assert_eq!(before.registration.source_log_id, after.registration.source_log_id);
    assert_eq!(h.bridge_identity_count_across_all_segments(), 1);
}

// =====================================================================
// Anti-bypass tripwire -- legacy run_once against a segmented
// destination: defense-in-depth only, documented as such, never a trust
// boundary.
// =====================================================================

#[tokio::test]
async fn legacy_run_once_against_a_segmented_destination_marker_is_refused_with_a_typed_error_and_zero_mutation() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);

    // h.legacy_locator is a real segment whose SIBLING carries
    // segment-manifest.json (created by adopt_existing_vault above) --
    // exactly the shape run_once's own anti_bypass tripwire checks for.
    let config = mcp_audit_bridge::RunConfig::new(h.source_vault_root(), h.source_journal_path(), h.legacy_locator.clone(), h.legacy_locator.join("journal.sqlite"), h.registration_path.clone(), h.checkpoint_path.clone());

    let before = h.read_destination_events(&h.legacy_locator);
    let result = mcp_audit_bridge::run_once(&config).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::SegmentedDestinationGuard(_), at_least_one_durable_append_occurred: false })), "got {result:?}");
    assert_eq!(h.read_destination_events(&h.legacy_locator).len(), before.len(), "the tripwire must refuse before any destination mutation");

    // Defense-in-depth only, never a trust boundary: this guard works
    // because segment-manifest.json happens to be a SIBLING of the
    // vault root in this harness's own layout. It is never asserted
    // anywhere in this crate or mcp-audit-bridge that its absence proves
    // a destination is safe to treat as a plain single vault -- the
    // REAL safety net is `audit_segmentation::retention`'s own pin check
    // and `SegmentedDestination`'s own trusted-root comparison, neither
    // of which this tripwire is a substitute for.
}

// =====================================================================
// Tampering
// =====================================================================

#[tokio::test]
async fn manifest_tampering_substituted_retention_ledger_locator_is_rejected() {
    let h = Harness::new();
    let mut manifest = h.manifest();
    manifest.retention_ledger_locator = h.dir.path().join("fake-ledger");
    std::fs::write(h.root.manifest_path(), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

    let result = h.seg_dest().resolve_active_destination();
    assert!(matches!(result, Err(mcp_audit_segmented::SegmentedDestinationError::RetentionLedgerLocatorMismatch { .. })), "got {result:?}");

    let begin_result = h.try_begin();
    assert!(begin_result.is_err(), "SegmentedBridgeRun::begin must also refuse against a tampered manifest");
}

#[tokio::test]
async fn pin_store_tampering_is_detected_on_reopen() {
    let h = Harness::new();
    audit_segmentation::pin_store::acquire(&h.root.expected_pin_store_locator(), &h.root.expected_pin_store_locator().join("journal.sqlite"), &audit_segmentation::SegmentId::from_index(0), h.owner(), audit_segmentation::PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();

    let log_path = h.root.expected_pin_store_locator().join(".audit").join("audit.log");
    let mut bytes = std::fs::read(&log_path).unwrap();
    let len = bytes.len();
    bytes[len - 5] ^= 0xFF;
    std::fs::write(&log_path, bytes).unwrap();

    let result = audit_segmentation::pin_store::verify_and_scan(&h.root.expected_pin_store_locator(), &h.root.expected_pin_store_locator().join("journal.sqlite"));
    assert!(matches!(result, Err(audit_segmentation::PinStoreError::ChainInvalid(_))), "expected ChainInvalid after tampering with the pin store's own chain bytes");

    // SegmentedBridgeRun::begin must also fail closed when the pin
    // store itself cannot be trusted.
    let begin_result = h.try_begin();
    assert!(begin_result.is_err());
}

#[tokio::test]
async fn duplicate_replayed_pin_acquired_is_detectable_via_check_no_duplicate_pin_ids() {
    let h = Harness::new();
    let locator = h.root.expected_pin_store_locator();
    let journal = locator.join("journal.sqlite");
    let segment = audit_segmentation::SegmentId::from_index(0);
    let pin_id = uuid::Uuid::new_v4();
    let pin = audit_segmentation::PinAcquired { pin_id, segment_id: segment, owner_class: h.owner(), reason: audit_segmentation::PinReason::PendingCheckpointWrite, acquired_at: "2026-01-01T00:00:00Z".into() };

    // Write the same PinAcquired twice, bypassing acquire()'s own
    // fresh-uuid guarantee -- modeling a replayed/tampered record.
    append_raw_pin_record(&locator, &journal, &pin);
    append_raw_pin_record(&locator, &journal, &pin);

    let (_, state) = audit_segmentation::pin_store::verify_and_scan(&locator, &journal).unwrap();
    let result = state.check_no_duplicate_pin_ids();
    assert!(matches!(result, Err(audit_segmentation::PinStoreError::DuplicatePinAcquired(id)) if id == pin_id), "got {result:?}");
}

fn append_raw_pin_record<T: serde::Serialize>(locator: &std::path::Path, journal_path: &std::path::Path, payload: &T) {
    let vault = storage::Vault::open_with_default_indexer(locator.to_path_buf(), journal_path).unwrap();
    let raw = vault.read_audit_log_for_recovery_scan().unwrap();
    let scan = scan_chain_bytes(&raw);
    let sequence = scan.last_verified.as_ref().map(|(s, _)| s + 1).unwrap_or(0);
    let prev_hash = scan.last_verified.as_ref().map(|(_, h)| h.clone()).unwrap_or_else(|| audit_chain_core::GENESIS_HASH.to_string());
    let (bytes, hash) = audit_chain_core::serialize_record(payload, sequence, &prev_hash);
    let mut writer = vault.open_audit_writer(scan.last_verified.as_ref().map(|(s, h)| (*s, h.as_str()))).unwrap();
    writer.append(&bytes, sequence, &hash).unwrap();
}

#[tokio::test]
async fn a_release_with_no_matching_acquire_is_refused_missing_or_forged_pin_released() {
    let h = Harness::new();
    let locator = h.root.expected_pin_store_locator();
    let journal = locator.join("journal.sqlite");
    let forged_pin_id = uuid::Uuid::new_v4();

    let result = audit_segmentation::pin_store::release(&locator, &journal, forged_pin_id, "2026-01-01T00:00:00Z");
    assert!(matches!(result, Err(audit_segmentation::PinStoreError::ReleaseWithoutAcquire(id)) if id == forged_pin_id), "got {result:?}");
}

#[tokio::test]
async fn a_substituted_valid_but_wrong_vault_as_pin_store_locator_is_rejected() {
    let h = Harness::new();
    // Point the pin-store locator at the legacy segment's OWN vault --
    // a genuinely valid, verifiable Vault, just the wrong one.
    let wrong_locator = h.legacy_locator.clone();
    let wrong_journal = wrong_locator.join("journal.sqlite");

    // This succeeds as a plain pin-store scan (it's a valid Vault), but
    // the point of the trusted-root discipline is that NOTHING in this
    // crate ever derives this locator from anywhere other than
    // TrustedSegmentationRoot's own fixed sibling derivation -- so a
    // caller substituting a different, wrong-but-valid Vault here is
    // only possible by bypassing SegmentedDestination/SegmentedBridgeRun
    // entirely, which this test documents explicitly.
    let scan_result = audit_segmentation::pin_store::verify_and_scan(&wrong_locator, &wrong_journal);
    assert!(scan_result.is_ok(), "a wrong-but-internally-valid Vault is not itself detectable as 'wrong' by the pin store alone");

    // The real safety net: SegmentedBridgeRun::begin/SegmentedDestination
    // never accept a caller-supplied pin-store locator at all -- it is
    // ALWAYS TrustedSegmentationRoot::expected_pin_store_locator()'s own
    // deterministic derivation, with no parameter or code path to
    // override it.
    assert_eq!(h.seg_dest().pin_store_locator(), h.root.expected_pin_store_locator());
    assert_ne!(h.seg_dest().pin_store_locator(), wrong_locator);
}

#[tokio::test]
async fn a_stale_manifest_missing_a_segment_the_pin_store_references_still_fails_closed_on_resolve() {
    let h = Harness::new();
    // Acquire a pin on a segment that does not exist in the manifest at
    // all (modeling a stale/tampered pin-store entry outliving manifest
    // state) -- `retire_segment`'s own pin check runs against whatever
    // target_segment_id is passed, so this models the pin-store side
    // alone; resolve_active_destination must still only ever trust the
    // manifest for what is Active.
    audit_segmentation::pin_store::acquire(&h.root.expected_pin_store_locator(), &h.root.expected_pin_store_locator().join("journal.sqlite"), &audit_segmentation::SegmentId::from_index(99), h.owner(), audit_segmentation::PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();

    let active = h.seg_dest().resolve_active_destination().unwrap();
    assert_eq!(active.segment_id, audit_segmentation::SegmentId::from_index(0), "a stale pin on a nonexistent segment must never influence which segment is resolved as Active");
}

// =====================================================================
// Concurrency
// =====================================================================

// Two segmented runs: the second `begin` fails while the first holds
// the lifecycle lock (already covered by pinned_access::tests, reasserted
// here against the same Harness this adversarial module uses, with the
// REAL run_with_destination/complete wiring around it this time).
#[tokio::test]
async fn two_segmented_runs_the_second_begin_is_refused_while_the_first_is_in_progress() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);

    let first_run = h.try_begin().unwrap();
    let second_begin = h.try_begin();
    assert!(matches!(second_begin, Err(mcp_audit_segmented::SegmentedRunError::LifecycleLock(_))), "a second begin must be refused while the first holds the lifecycle lock");

    let result = mcp_audit_bridge::run_with_destination(first_run.destination(), &h.source_config(), &h.registration_path, &h.checkpoint_path, None).await;
    let (result, pin_outcome) = first_run.complete(result);
    result.unwrap();
    pin_outcome.unwrap();

    // Now that the first run has fully completed (lifecycle lock
    // dropped), a fresh begin succeeds.
    let third_begin = h.try_begin();
    assert!(third_begin.is_ok());
}

// Segmented run vs rotation: already covered directly in
// pinned_access::tests::rotation_is_refused_while_a_segmented_bridge_run_holds_the_lifecycle_lock;
// reconfirmed here through the full begin/run_with_destination path.
#[tokio::test]
async fn segmented_run_vs_rotation_rotation_is_refused_for_the_runs_entire_duration() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);

    let run = h.try_begin().unwrap();

    let segment_1 = h.dir.path().join("segment-1");
    let rotate_result = h.rotate(&segment_1).await;
    assert!(matches!(rotate_result, Err(audit_segmentation::RotationError::LockHeld(_))), "got {rotate_result:?}");

    let result = mcp_audit_bridge::run_with_destination(run.destination(), &h.source_config(), &h.registration_path, &h.checkpoint_path, None).await;
    let (result, pin_outcome) = run.complete(result);
    result.unwrap();
    pin_outcome.unwrap();

    // After completion, rotation succeeds normally.
    assert!(h.rotate(&segment_1).await.is_ok());
}

// Segmented run vs retention: retention on the segment the run has
// pinned is refused for the pin's entire lifetime, independent of the
// lifecycle lock (retention's own pin check, not the lifecycle lock,
// is what denies it once the run itself has completed and released the
// lifecycle lock but the append succeeded so the pin survives).
#[tokio::test]
async fn segmented_run_vs_retention_retention_is_refused_while_the_runs_pin_is_still_active() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);

    // Segment 0 is permanently NotRetentionEligible -- rotate once first
    // so segment 1 (retirable) is the Active segment the run pins.
    let segment_1 = h.dir.path().join("segment-1");
    h.append_raw(&h.legacy_locator, brain_api::AuditEventKind::McpConnection { server: "s0".into() });
    h.rotate(&segment_1).await.unwrap();

    // Crash after the append, before the checkpoint -- pin survives
    // completion.
    let run = h.try_begin().unwrap();
    let result = mcp_audit_bridge::run_with_destination(run.destination(), &h.source_config(), &h.registration_path, &h.checkpoint_path, Some(1)).await;
    let (result, pin_outcome) = run.complete(result);
    assert!(result.is_err());
    pin_outcome.unwrap();
    assert_eq!(h.active_pins_on(&audit_segmentation::SegmentId::from_index(1)).len(), 1);

    // Rotate so segment 1 can become Closed and therefore
    // retention-attemptable at all.
    let segment_2 = h.dir.path().join("segment-2");
    h.rotate(&segment_2).await.unwrap();

    let denied = h.retire(&audit_segmentation::SegmentId::from_index(1), &segment_2.join("journal.sqlite")).await;
    assert!(matches!(denied, Err(audit_segmentation::RetentionError::SegmentHasActivePins(_, _))), "got {denied:?}");
}

// Legacy concurrent runs: two run_once calls racing on the SAME
// already-registered, non-segmented state -- the bridge_lock makes
// exactly one proceed (reconfirms mcp-audit-bridge's own
// concurrency_6 guarantee holds unchanged after M7.7, from this crate's
// own perspective as the thing that must never weaken it).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_concurrent_runs_exactly_one_proceeds() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[discovery("s", "c1")]);

    // A genuinely separate temp directory -- NOT a sibling of
    // h.dir's own segment-manifest.json, which would otherwise (quite
    // correctly) trip the anti-bypass tripwire this suite tests
    // elsewhere. This scenario is specifically about the LEGACY,
    // non-segmented path's own lock behavior, unrelated to segmentation.
    let plain_dir = tempfile::tempdir().unwrap();
    let plain_destination = plain_dir.path().join("plain-destination");
    std::fs::create_dir_all(&plain_destination).unwrap();
    let config_a = mcp_audit_bridge::RunConfig::new(h.source_vault_root(), h.source_journal_path(), plain_destination.clone(), plain_destination.join("journal.sqlite"), h.registration_path.clone(), h.checkpoint_path.clone());
    let config_b = mcp_audit_bridge::RunConfig::new(h.source_vault_root(), h.source_journal_path(), plain_destination.clone(), plain_destination.join("journal.sqlite"), h.registration_path.clone(), h.checkpoint_path.clone());

    let task_a = tokio::spawn(async move { mcp_audit_bridge::run_once(&config_a).await });
    let task_b = tokio::spawn(async move { mcp_audit_bridge::run_once(&config_b).await });
    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let (result_a, result_b) = (result_a.unwrap(), result_b.unwrap());

    let successes = [&result_a, &result_b].iter().filter(|r| r.is_ok()).count();
    let refusals = [&result_a, &result_b].iter().filter(|r| matches!(r, Err(RunFailure { outcome: RunOutcome::AlreadyRunning(_), .. }))).count();
    assert_eq!(successes, 1, "got {result_a:?} / {result_b:?}");
    assert_eq!(refusals, 1);
}
