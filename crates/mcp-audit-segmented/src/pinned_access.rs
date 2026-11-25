//! M7.7.4/M7.7.5/M7.7.6: the pinned write-target model — the ONLY way
//! a segmented bridge run may ever come into existence, enforcing the
//! correct lock order and the correct pin-release semantics at the
//! type level, not by convention.
//!
//! **Construction invariant** (the frozen M7.7 correction): neither
//! [`PinnedWriteTarget`] nor [`PinnedDestinationAccess`] is
//! constructible outside this module — [`SegmentedBridgeRun::begin`]
//! is the one entry point, and it only ever succeeds after acquiring,
//! in order, the `audit-segmentation` lifecycle lock and a durable
//! `RetentionPin` on the resolved Active segment. There is no
//! constructor that accepts a raw path/`SegmentId` and skips either
//! step.
//!
//! **Lock order, exactly as frozen**:
//! ```text
//! lifecycle_lock
//!   -> pin-store writer.lock (acquire_pin, brief, released immediately after)
//!   -> bridge_lock (inside mcp_audit_bridge::run_with_destination)
//!     -> destination segment writer.lock (inside run_with_destination, per append)
//! ```
//! [`SegmentedBridgeRun::begin`] acquires the lifecycle lock and the
//! pin (steps 1-2); the caller then invokes
//! `mcp_audit_bridge::run_with_destination` against
//! [`SegmentedBridgeRun::destination`] (step 3, acquiring `bridge_lock`
//! internally); [`SegmentedBridgeRun::complete`] decides the pin's fate
//! and only then drops the lifecycle guard.
//!
//! **Write target frozen for the run's entire duration**: the Active
//! segment is resolved exactly once, inside `begin`, while the
//! lifecycle lock is held — `rotate()` (in `audit-segmentation`) also
//! acquires that same lock, so no rotation can complete while a
//! `SegmentedBridgeRun` is alive. [`PinnedDestinationAccess::append_durable`]
//! never re-resolves "the current Active segment" — it always writes
//! to the one locator captured at construction time.

use audit_segmentation::{OwnerClass, PinReason};

use crate::destination::SegmentedDestination;

#[derive(Debug, thiserror::Error)]
pub enum SegmentedRunError {
    #[error("segmented destination error: {0}")]
    Destination(#[from] crate::destination::SegmentedDestinationError),
    #[error("failed to acquire the segmentation lifecycle lock: {0}")]
    LifecycleLock(#[from] audit_segmentation::LifecycleLockError),
    #[error("failed to acquire a retention pin: {0}")]
    PinAcquireFailed(audit_segmentation::PinStoreError),
    #[error("failed to release a retention pin: {0}")]
    PinReleaseFailed(audit_segmentation::PinStoreError),
}

/// The Active segment resolved and frozen for one run's entire
/// duration — never re-resolved after construction. Not publicly
/// constructible (see this module's own top doc comment).
struct PinnedWriteTarget {
    locator: std::path::PathBuf,
    journal_path: std::path::PathBuf,
}

/// Implements [`mcp_audit_bridge::DestinationAccess`] against a single,
/// frozen [`PinnedWriteTarget`] — never re-resolves "the current
/// Active segment." Reads route through the [`SegmentedDestination`]
/// resolved alongside that same write target in
/// [`SegmentedBridgeRun::begin`], giving the frozen multi-segment
/// completeness rule. Not publicly constructible; only
/// [`SegmentedBridgeRun::begin`] ever builds one, and only after the
/// lifecycle lock and pin are already held.
pub struct PinnedDestinationAccess {
    write_target: PinnedWriteTarget,
    seg_dest: SegmentedDestination,
}

impl mcp_audit_bridge::DestinationAccess for PinnedDestinationAccess {
    async fn read_verified_history(&self) -> Result<mcp_audit_bridge::DestinationHistory, mcp_audit_bridge::DestinationAccessError> {
        let completeness = self.seg_dest.history_completeness().map_err(|e| mcp_audit_bridge::DestinationAccessError::VaultOpenFailed(storage::StorageError::Io(std::io::Error::other(e.to_string()))))?;
        let verified_records = match completeness {
            mcp_audit_bridge::HistoryCompleteness::Incomplete => Vec::new(),
            mcp_audit_bridge::HistoryCompleteness::Complete => self.seg_dest.verified_history().map_err(|e| mcp_audit_bridge::DestinationAccessError::VaultOpenFailed(storage::StorageError::Io(std::io::Error::other(e.to_string()))))?,
        };
        Ok(mcp_audit_bridge::DestinationHistory { verified_records, completeness })
    }

    async fn append_durable(&self, event: brain_api::AuditEvent) -> Result<(), mcp_audit_bridge::DestinationAccessError> {
        let vault = storage::Vault::open_with_default_indexer(self.write_target.locator.clone(), &self.write_target.journal_path).map_err(mcp_audit_bridge::DestinationAccessError::VaultOpenFailed)?;
        let worker = brain_api_audit_worker::spawn_worker(vault, brain_api_audit_worker::WorkerConfig::default()).map_err(mcp_audit_bridge::DestinationAccessError::WorkerStartupFailed)?;
        let result = worker.record_durable(event).await.map(|_ack| ()).map_err(mcp_audit_bridge::DestinationAccessError::AppendFailed);
        let _ = worker.shutdown().await;
        result
    }
}

/// The public guard: the ONLY way to obtain a [`PinnedDestinationAccess`].
/// Holding one IS holding the segmentation lifecycle lock and the
/// retention pin — both released only via [`complete`](Self::complete),
/// never by dropping this value silently (dropping without calling
/// `complete` is still safe — the pin simply stays Active, which is the
/// fail-safe direction — but a caller should always call `complete`).
pub struct SegmentedBridgeRun {
    _lifecycle_guard: audit_segmentation::LifecycleLock,
    pin_id: uuid::Uuid,
    pin_store_locator: std::path::PathBuf,
    pin_store_journal_path: std::path::PathBuf,
    destination_access: PinnedDestinationAccess,
}

impl SegmentedBridgeRun {
    /// The one public entry point. Acquires, strictly in order: the
    /// `audit-segmentation` lifecycle lock (step 1), then resolves the
    /// Active segment (stable for the rest of this run, since `rotate()`
    /// cannot complete without the same lock), then acquires a durable
    /// pin on it (step 2, briefly taking the pin store's own
    /// `writer.lock`, released immediately after). Fails closed at any
    /// step without leaking a partially-acquired resource (the lifecycle
    /// guard is simply dropped on an early return, which releases it
    /// immediately).
    pub fn begin(seg_dest: SegmentedDestination, owner_class: OwnerClass, reason: PinReason) -> Result<Self, SegmentedRunError> {
        let lifecycle_guard = audit_segmentation::lifecycle_lock::acquire(&seg_dest.manifest_path())?;

        let active = seg_dest.resolve_active_destination()?;
        let pin_store_locator = seg_dest.pin_store_locator();
        let pin_store_journal_path = seg_dest.pin_store_journal_path();
        let pin_id = audit_segmentation::pin_store::acquire(&pin_store_locator, &pin_store_journal_path, &active.segment_id, owner_class, reason, &chrono::Utc::now().to_rfc3339()).map_err(SegmentedRunError::PinAcquireFailed)?;

        let write_target = PinnedWriteTarget { locator: active.locator, journal_path: active.journal_path };
        let destination_access = PinnedDestinationAccess { write_target, seg_dest };

        Ok(Self { _lifecycle_guard: lifecycle_guard, pin_id, pin_store_locator, pin_store_journal_path, destination_access })
    }

    /// The `DestinationAccess` implementation to pass to
    /// `mcp_audit_bridge::run_with_destination` — never re-resolves
    /// "the current Active segment," always writes to the one target
    /// frozen in [`begin`](Self::begin).
    pub fn destination(&self) -> &PinnedDestinationAccess {
        &self.destination_access
    }

    /// M7.7.6: the pin-release state machine, driven directly by the
    /// bridge's own `run_with_destination` result — this is the ONLY
    /// public completion API, and it never asks a caller to extract and
    /// re-pass a bare bool. The structural fact
    /// `RunFailure::at_least_one_durable_append_occurred` is read from
    /// `result` internally, exactly as `run_with_destination` itself set
    /// it (at the one point `DestinationAccess::append_durable` returns
    /// `Ok`, never re-derived from `is_ok()`/`imported.len()`/the error
    /// variant):
    ///
    /// - `Ok(_)`: full success — release immediately.
    /// - `Err(failure)` with `at_least_one_durable_append_occurred == false`:
    ///   nothing durable happened yet — release immediately.
    /// - `Err(failure)` with `at_least_one_durable_append_occurred == true`:
    ///   keep the pin Active. A caller must retry/reconcile and call
    ///   `complete` again later with a result that resolves one of the
    ///   first two cases.
    ///
    /// Returns the original `result` back to the caller unchanged (this
    /// function's own job is pin/lock bookkeeping, never swallowing the
    /// run's outcome) alongside the pin decision's own success/failure.
    ///
    /// **Release failure is fail-safe**: if releasing the pin itself
    /// fails, this reports that failure and the pin remains considered
    /// Active (this function never claims success it cannot prove) — a
    /// caller must retry the release, never assume it happened.
    pub fn complete(self, result: Result<mcp_audit_bridge::RunReport, mcp_audit_bridge::RunFailure>) -> (Result<mcp_audit_bridge::RunReport, mcp_audit_bridge::RunFailure>, Result<(), SegmentedRunError>) {
        let at_least_one_durable_append_occurred = match &result {
            Ok(_) => false,
            Err(failure) => failure.at_least_one_durable_append_occurred,
        };

        let pin_outcome = if at_least_one_durable_append_occurred {
            Ok(())
        } else {
            audit_segmentation::pin_store::release(&self.pin_store_locator, &self.pin_store_journal_path, self.pin_id, &chrono::Utc::now().to_rfc3339()).map_err(SegmentedRunError::PinReleaseFailed)
        };

        // self._lifecycle_guard is dropped here, releasing the
        // lifecycle lock -- always, regardless of the pin decision
        // above, since the lifecycle lock protects only THIS run's own
        // critical section, never the pin's own longer-lived duration.
        (result, pin_outcome)
    }

    /// Releases the pin unconditionally — the caller's own explicit
    /// choice after a confirmed-successful reconciliation outside the
    /// normal `complete` path (never inferred automatically from a
    /// `Result` type by this module).
    pub fn release_pin(self) -> Result<(), SegmentedRunError> {
        audit_segmentation::pin_store::release(&self.pin_store_locator, &self.pin_store_journal_path, self.pin_id, &chrono::Utc::now().to_rfc3339()).map_err(SegmentedRunError::PinReleaseFailed)
    }
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

        fn seg_dest(&self) -> SegmentedDestination {
            SegmentedDestination::open(self.root.clone())
        }
    }

    fn owner() -> OwnerClass {
        OwnerClass::new("mcp-audit-bridge").unwrap()
    }

    #[test]
    fn begin_acquires_a_durable_pin_on_the_resolved_active_segment() {
        let h = Harness::new();
        let run = SegmentedBridgeRun::begin(h.seg_dest(), owner(), PinReason::PendingCheckpointWrite).unwrap();

        let (_, state) = audit_segmentation::pin_store::verify_and_scan(&h.root.expected_pin_store_locator(), &h.root.expected_pin_store_locator().join("journal.sqlite")).unwrap();
        let active_pins = state.active_pins_for(&audit_segmentation::SegmentId::from_index(0)).unwrap();
        assert_eq!(active_pins.len(), 1);
        assert_eq!(active_pins[0].pin_id, run.pin_id);
    }

    fn ok_report() -> Result<mcp_audit_bridge::RunReport, mcp_audit_bridge::RunFailure> {
        Ok(mcp_audit_bridge::RunReport::default())
    }

    fn failure_without_append() -> Result<mcp_audit_bridge::RunReport, mcp_audit_bridge::RunFailure> {
        Err(mcp_audit_bridge::RunOutcome::RegistrationMissing.into())
    }

    fn failure_with_append() -> Result<mcp_audit_bridge::RunReport, mcp_audit_bridge::RunFailure> {
        Err(mcp_audit_bridge::RunFailure { outcome: mcp_audit_bridge::RunOutcome::RegistrationMissing, at_least_one_durable_append_occurred: true })
    }

    #[tokio::test]
    async fn complete_with_full_success_releases_the_pin() {
        let h = Harness::new();
        let run = SegmentedBridgeRun::begin(h.seg_dest(), owner(), PinReason::PendingCheckpointWrite).unwrap();
        let pin_id = run.pin_id;
        let (result, pin_outcome) = run.complete(ok_report());
        assert!(result.is_ok());
        pin_outcome.unwrap();

        let (_, state) = audit_segmentation::pin_store::verify_and_scan(&h.root.expected_pin_store_locator(), &h.root.expected_pin_store_locator().join("journal.sqlite")).unwrap();
        let active_pins = state.active_pins_for(&audit_segmentation::SegmentId::from_index(0)).unwrap();
        assert!(active_pins.is_empty(), "pin {pin_id} must be released");
    }

    #[tokio::test]
    async fn complete_with_no_durable_append_releases_the_pin() {
        let h = Harness::new();
        let run = SegmentedBridgeRun::begin(h.seg_dest(), owner(), PinReason::PendingCheckpointWrite).unwrap();
        let pin_id = run.pin_id;
        let (_, pin_outcome) = run.complete(failure_without_append());
        pin_outcome.unwrap();

        let (_, state) = audit_segmentation::pin_store::verify_and_scan(&h.root.expected_pin_store_locator(), &h.root.expected_pin_store_locator().join("journal.sqlite")).unwrap();
        let active_pins = state.active_pins_for(&audit_segmentation::SegmentId::from_index(0)).unwrap();
        assert!(active_pins.is_empty(), "pin {pin_id} must be released");
    }

    #[tokio::test]
    async fn complete_with_a_durable_append_keeps_the_pin_active() {
        let h = Harness::new();
        let run = SegmentedBridgeRun::begin(h.seg_dest(), owner(), PinReason::PendingCheckpointWrite).unwrap();
        let pin_id = run.pin_id;
        let (_, pin_outcome) = run.complete(failure_with_append());
        pin_outcome.unwrap();

        let (_, state) = audit_segmentation::pin_store::verify_and_scan(&h.root.expected_pin_store_locator(), &h.root.expected_pin_store_locator().join("journal.sqlite")).unwrap();
        let active_pins = state.active_pins_for(&audit_segmentation::SegmentId::from_index(0)).unwrap();
        assert_eq!(active_pins.len(), 1);
        assert_eq!(active_pins[0].pin_id, pin_id);
    }

    /// M7.7.6: pin release failure is fail-safe -- if the release
    /// itself fails, `complete` reports that failure rather than
    /// silently claiming success, so a caller never treats the pin as
    /// released without proof. Modeled here by tampering with the pin
    /// store's own chain bytes after acquisition (the real, verifiable
    /// way `audit_segmentation::pin_store::release`'s internal
    /// `verify_and_scan` can fail), rather than any kind of mocked
    /// failure.
    #[tokio::test]
    async fn complete_reports_a_release_failure_and_the_pin_must_still_be_treated_as_active() {
        let h = Harness::new();
        let run = SegmentedBridgeRun::begin(h.seg_dest(), owner(), PinReason::PendingCheckpointWrite).unwrap();

        // Tamper with the pin store's own on-disk chain bytes -- the
        // next verify_and_scan (which release() performs internally)
        // must detect this and fail closed.
        let log_path = h.root.expected_pin_store_locator().join(".audit").join("audit.log");
        let mut bytes = std::fs::read(&log_path).unwrap();
        let len = bytes.len();
        bytes[len - 5] ^= 0xFF;
        std::fs::write(&log_path, bytes).unwrap();

        let (_, pin_outcome) = run.complete(failure_without_append());
        assert!(matches!(pin_outcome, Err(SegmentedRunError::PinReleaseFailed(_))), "got {pin_outcome:?}");
    }

    #[test]
    fn a_second_begin_on_the_same_root_fails_while_the_first_holds_the_lifecycle_lock() {
        let h = Harness::new();
        let _first = SegmentedBridgeRun::begin(h.seg_dest(), owner(), PinReason::PendingCheckpointWrite).unwrap();
        let second = SegmentedBridgeRun::begin(h.seg_dest(), owner(), PinReason::PendingCheckpointWrite);
        assert!(matches!(second, Err(SegmentedRunError::LifecycleLock(_))), "expected LifecycleLock error on the second begin");
    }

    #[test]
    fn rotation_is_refused_while_a_segmented_bridge_run_holds_the_lifecycle_lock() {
        let h = Harness::new();
        let _run = SegmentedBridgeRun::begin(h.seg_dest(), owner(), PinReason::PendingCheckpointWrite).unwrap();

        // Append something to the legacy segment first so rotation
        // would otherwise be eligible to proceed (an empty segment is
        // refused for an unrelated reason, which would mask the real
        // assertion here).
        // (append bypassing the run's own lock -- this models a SEPARATE process/call attempting rotation concurrently)
        let journal = h.legacy_locator.join("journal.sqlite");
        let vault = storage::Vault::open_with_default_indexer(h.legacy_locator.clone(), &journal).unwrap();
        let event = brain_api::event(brain_api::AuditEventKind::McpConnection { server: "s".into() }, "2026-01-01T00:00:00Z", uuid::Uuid::new_v4());
        let (bytes, hash) = audit_chain_core::serialize_record(&event, 0, audit_chain_core::GENESIS_HASH);
        let mut writer = vault.open_audit_writer(None).unwrap();
        writer.append(&bytes, 0, &hash).unwrap();
        drop(writer);

        let next_locator = h.dir.path().join("segment-1");
        let rotation_config = audit_segmentation::RotationConfig { manifest_path: h.root.manifest_path(), retention_ledger_journal_path: h.root.expected_retention_ledger_locator().join("journal.sqlite"), next_segment_locator: next_locator.clone(), next_segment_journal_path: next_locator.join("journal.sqlite") };

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = rt.block_on(audit_segmentation::rotate(&rotation_config));
        assert!(matches!(result, Err(audit_segmentation::RotationError::LockHeld(_))), "got {result:?}");
    }
}
