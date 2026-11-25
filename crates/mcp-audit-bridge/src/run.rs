//! M7: one batch/offline bridge run — ties `crate::identity`/
//! `crate::registration`/`crate::checkpoint`/`crate::dedup`/
//! `crate::mapping` together against a real source `storage::Vault`
//! (read-only) and a real destination `storage::Vault` (via
//! `brain_api_audit_worker`, reusing the exact same M5-reviewed writer
//! machinery every other composition root uses — this crate adds no new
//! write path).
//!
//! **Registration vs. checkpoint — two separate files, two separate
//! lifecycles** (the frozen correction): the durable `SourceLogId` lives
//! in `crate::registration`'s own file, never in the checkpoint. Losing
//! the checkpoint is always harmless (rebuilt from a full destination
//! scan under the SAME identity, see [`resolve_registration`]'s own
//! handling below). Losing the registration is a DIFFERENT, more
//! serious case, handled explicitly by [`resolve_registration`] — never
//! silently treated as "first-ever run."
//!
//! **Cross-process exclusivity** (the frozen correction's third
//! requirement): the ENTIRE critical section below — registration
//! resolve, checkpoint load, source rebind check, destination dedup
//! scan, every append, every checkpoint/registration save — runs under
//! one process-wide `crate::bridge_lock::BridgeLock`, acquired before
//! any of it and released only after the destination worker has shut
//! down. A second `run_once` call against the same state directory,
//! whether from this process or another, is refused immediately
//! ([`RunOutcome::AlreadyRunning`]) rather than allowed to race the
//! first — see `crate::bridge_lock`'s own doc comment for why this is a
//! real OS-level file lock, not an in-process mutex.
//!
//! **Per-record algorithm** (frozen design, with the dedup correction
//! applied): for every new source record, in order —
//! 1. build its `SourceRecordIdentity`;
//! 2. check membership in a FULL scan of the destination chain (never
//!    the checkpoint alone, never a tail-only search — `crate::dedup`);
//! 3. if already imported: skip the append, advance the checkpoint only;
//! 4. if not: translate, append durably, THEN persist the checkpoint.
//!
//! Re-deriving the full destination scan before every single append (not
//! just once per run) is deliberate: a prior run may have appended a
//! record and crashed before updating its checkpoint, and this run must
//! detect that exact fact for the SPECIFIC identity about to be
//! (re)appended, not rely on a stale assumption from earlier in its own
//! loop.

use crate::bridge_lock::{self, BridgeLockError};
use crate::checkpoint::{self, BridgeCheckpoint, CheckpointError};
use crate::dedup::{scan_destination_for_imported_identities, DestinationAnomaly};
use crate::identity::{verify_relocation, RelocationError, SourceLogId, SourceLogRegistration, SourceRecordIdentity};
use crate::registration::{self, RegistrationError, RegistrationFile, RegistrationRecoveryError};

/// Everything one bridge run needs to locate its source, its
/// destination, and its own durable state files.
pub struct RunConfig {
    /// The source MCP audit vault's root — read-only; this crate never
    /// opens a writer on it, never calls `open_audit_writer` on this
    /// vault.
    pub source_vault_root: std::path::PathBuf,
    pub source_journal_path: std::path::PathBuf,
    /// The destination Brain audit vault's root — the bridge opens a
    /// real durable writer on this one, via `brain_api_audit_worker`.
    pub destination_vault_root: std::path::PathBuf,
    pub destination_journal_path: std::path::PathBuf,
    /// Where this bridge's durable source-log identity lives — separate
    /// file from `checkpoint_path`, different lifecycle (see this
    /// module's own top doc comment).
    pub registration_path: std::path::PathBuf,
    /// Where this bridge's mutable progress state lives.
    pub checkpoint_path: std::path::PathBuf,
    /// Test/fault-injection hook only: when `Some`, `run_once` returns
    /// `Err(RunOutcome::InjectedFault)` immediately after this many
    /// successful durable destination appends in THIS call, before the
    /// checkpoint for that append is saved — the real "destination
    /// append succeeded, then the process died before the checkpoint
    /// write" crash window, exercised through the actual production
    /// code path rather than a hand-rolled bypass. `None` in every real
    /// composition root; acceptance tests are the only caller that ever
    /// sets this.
    pub crash_after_n_destination_appends: Option<usize>,
}

impl RunConfig {
    pub fn new(source_vault_root: std::path::PathBuf, source_journal_path: std::path::PathBuf, destination_vault_root: std::path::PathBuf, destination_journal_path: std::path::PathBuf, registration_path: std::path::PathBuf, checkpoint_path: std::path::PathBuf) -> Self {
        Self { source_vault_root, source_journal_path, destination_vault_root, destination_journal_path, registration_path, checkpoint_path, crash_after_n_destination_appends: None }
    }
}

/// What happened during one run — enough detail for the acceptance tests
/// (and a real operator) to verify exactly what was imported, skipped,
/// or flagged, without re-deriving it from logs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub imported: Vec<SourceRecordIdentity>,
    pub skipped_already_imported: Vec<SourceRecordIdentity>,
    pub anomalies: Vec<DestinationAnomaly>,
}

#[derive(Debug, thiserror::Error)]
pub enum RunOutcome {
    #[error("failed to open source vault: {0:?}")]
    SourceVaultOpenFailed(storage::StorageError),
    #[error("failed to read source audit log: {0:?}")]
    SourceReadFailed(storage::StorageError),
    #[error("source chain failed verification, refusing to import: {0:?}")]
    SourceChainInvalid(audit_chain_core::ScanStopReason),
    #[error("registration's known source chain is incompatible with the current locator's chain: {0}")]
    SourceRelocationRefused(#[from] RelocationError),
    #[error("failed to load registration: {0}")]
    RegistrationLoadFailed(RegistrationError),
    #[error("failed to save registration: {0}")]
    RegistrationSaveFailed(RegistrationError),
    #[error("registration is missing and automatic recovery from destination evidence failed: {0}")]
    RegistrationRecoveryFailed(RegistrationRecoveryError),
    #[error("failed to load checkpoint: {0}")]
    CheckpointLoadFailed(#[from] CheckpointError),
    #[error("failed to save checkpoint: {0}")]
    CheckpointSaveFailed(CheckpointError),
    #[error("failed to open destination vault: {0:?}")]
    DestinationVaultOpenFailed(storage::StorageError),
    #[error("failed to read destination audit log: {0:?}")]
    DestinationReadFailed(storage::StorageError),
    #[error("destination chain failed verification: {0:?}")]
    DestinationChainInvalid(audit_chain_core::ScanStopReason),
    #[error("failed to start the destination audit worker: {0}")]
    DestinationWorkerStartupFailed(brain_api_audit_worker::WorkerStartupError),
    #[error("durable append to the destination chain failed: {0}")]
    DestinationAppendFailed(brain_api_audit_worker::DurableRecordError),
    #[error("source record at sequence {sequence} passed chain verification but its content did not decode as a valid McpClientEvent: {detail}")]
    SourceEventMalformed { sequence: u64, detail: String },
    /// Test/fault-injection only — see [`RunConfig::crash_after_n_destination_appends`].
    #[error("injected fault: simulated crash after {appends_so_far} destination append(s), before the checkpoint for the most recent one was saved")]
    InjectedFault { appends_so_far: usize },
    /// Another `run_once` already holds the cross-process bridge lock
    /// for this state directory — refused immediately, never waited out.
    #[error(transparent)]
    AlreadyRunning(#[from] BridgeLockError),
}

/// Runs one full batch/offline import pass. Idempotent and crash-safe:
/// calling this repeatedly (including after a crash at any point during
/// a prior call) converges to the same destination state, never
/// duplicating an already-imported source record (see `crate::dedup` and
/// this module's own per-record algorithm above) — PROVIDED the
/// registration file (`crate::registration`) survives; see
/// [`resolve_registration`] for what happens when it does not.
pub async fn run_once(config: &RunConfig) -> Result<RunReport, RunOutcome> {
    // Acquire the cross-process bridge lock FIRST, before touching any
    // state file -- held for this entire function's remaining body (the
    // guard is a plain local, dropped only when this function returns,
    // by which point the destination worker has already been shut down
    // and every save has already happened). See `crate::bridge_lock`'s
    // own doc comment for why this closes the first-run registration
    // race a prior review found.
    let _lock = bridge_lock::acquire(&config.checkpoint_path)?;

    // --- source side: open read-only, verify the full chain ---
    let source_vault = storage::Vault::open_with_default_indexer(config.source_vault_root.clone(), &config.source_journal_path).map_err(RunOutcome::SourceVaultOpenFailed)?;
    let source_raw = source_vault.read_audit_log_for_recovery_scan().map_err(RunOutcome::SourceReadFailed)?;
    let source_scan = scan_chain_bytes(&source_raw);
    if let Some(stop_reason) = &source_scan.stop_reason {
        if !audit_chain_core::may_resume_without_recovery(&source_scan) {
            return Err(RunOutcome::SourceChainInvalid(stop_reason.clone()));
        }
    }

    // --- destination side: read raw bytes once up front, for registration recovery only ---
    let destination_raw_for_recovery = destination_vault_raw_bytes(&config.destination_vault_root, &config.destination_journal_path).map_err(RunOutcome::DestinationReadFailed)?;
    let destination_scan_for_recovery = scan_chain_bytes(&destination_raw_for_recovery);

    // --- registration: load, recover from destination evidence, or mint fresh -- never conflated with checkpoint loss ---
    let registration = resolve_registration(config, &source_scan, &destination_scan_for_recovery)?;
    let source_log_id = registration.source_log_id;

    // --- checkpoint: progress only, its own absence is always harmless once registration is settled ---
    let loaded_checkpoint = checkpoint::load(&config.checkpoint_path)?;
    let last_imported_sequence = loaded_checkpoint.as_ref().and_then(|cp| cp.last_imported.as_ref()).map(|i| i.sequence);

    // --- destination side: open the real durable writer (M5 machinery, unchanged) ---
    let destination_vault = storage::Vault::open_with_default_indexer(config.destination_vault_root.clone(), &config.destination_journal_path).map_err(RunOutcome::DestinationVaultOpenFailed)?;
    let destination_worker = brain_api_audit_worker::spawn_worker(destination_vault, brain_api_audit_worker::WorkerConfig::default()).map_err(RunOutcome::DestinationWorkerStartupFailed)?;

    // The record-processing loop lives in its own function so EVERY exit
    // path (success, a real failure, or the fault-injection return) goes
    // through the single `shutdown().await` below before this function
    // returns -- releasing the destination vault's own exclusive writer
    // lock deterministically rather than leaving it to an unawaited
    // background task's own eventual drop. A real crash releases the OS
    // lock immediately on process exit; a graceful early return from
    // this async function must not leave it held indefinitely instead.
    let outcome = process_candidates(config, source_log_id, &source_scan, last_imported_sequence, &destination_worker).await;
    let _ = destination_worker.shutdown().await;
    outcome
}

async fn process_candidates(config: &RunConfig, source_log_id: SourceLogId, source_scan: &audit_chain_core::ScanResult, last_imported_sequence: Option<u64>, destination_worker: &brain_api_audit_worker::AuditWorkerHandle) -> Result<RunReport, RunOutcome> {
    let mut report = RunReport::default();
    let mut appends_so_far = 0usize;

    // Candidate records: every source record not yet BELOW the
    // checkpoint's own last imported sequence -- an optimization over
    // always starting from sequence 0 (the full destination scan below
    // is still what actually decides import-or-skip for each one, per
    // the frozen dedup correction).
    let candidates: Vec<&audit_chain_core::ChainRecord> = source_scan.verified_records.iter().filter(|r| last_imported_sequence.map(|last| r.sequence > last).unwrap_or(true)).collect();

    for record in &candidates {
        let identity = SourceRecordIdentity { source_log_id, sequence: record.sequence, record_hash: record.record_hash.clone() };

        // Full destination scan, re-derived before THIS append -- never
        // trusting the checkpoint alone, never a tail-only search (the
        // frozen correction this module exists to satisfy).
        let destination_raw = destination_vault_raw_bytes(&config.destination_vault_root, &config.destination_journal_path).map_err(RunOutcome::DestinationReadFailed)?;
        let destination_scan = scan_chain_bytes(&destination_raw);
        if let Some(stop_reason) = &destination_scan.stop_reason {
            if !audit_chain_core::may_resume_without_recovery(&destination_scan) {
                return Err(RunOutcome::DestinationChainInvalid(stop_reason.clone()));
            }
        }
        let destination_state = scan_destination_for_imported_identities(&destination_scan.verified_records);
        report.anomalies.extend(destination_state.anomalies.iter().cloned());

        if destination_state.imported.contains(&identity) {
            report.skipped_already_imported.push(identity.clone());
        } else {
            let source_event: mcp_client_audit_worker::McpClientEvent =
                audit_chain_core::parse_event(&record.content_bytes).map_err(|e| RunOutcome::SourceEventMalformed { sequence: record.sequence, detail: e.to_string() })?;
            let translated_kind = crate::mapping::translate(&source_event.kind, &identity);
            let event = brain_api::event(translated_kind, &source_event.timestamp, uuid::Uuid::new_v4());
            destination_worker.record_durable(event).await.map_err(RunOutcome::DestinationAppendFailed)?;
            report.imported.push(identity.clone());
            appends_so_far += 1;

            if config.crash_after_n_destination_appends == Some(appends_so_far) {
                // Simulated crash: the destination append above already
                // committed durably (the real `record_durable` call
                // returned `Ok`) -- this return happens BEFORE the
                // checkpoint save below, faithfully modeling "process
                // died in that exact window" through the real code path,
                // not a hand-rolled bypass.
                return Err(RunOutcome::InjectedFault { appends_so_far });
            }
        }

        // Update the REGISTRATION's own last_known_tail (needed for
        // future relocation verification) and the CHECKPOINT's
        // last_imported -- two separate files, both updated after this
        // record either way (skip or import), so a subsequent run's
        // `candidates` filter and relocation check both reflect this
        // record's position.
        let updated_registration = SourceLogRegistration { source_log_id, current_locator: config.source_vault_root.clone(), last_known_tail: Some((record.sequence, record.record_hash.clone())) };
        registration::save(&config.registration_path, &RegistrationFile::new(updated_registration)).map_err(RunOutcome::RegistrationSaveFailed)?;

        let updated_checkpoint = BridgeCheckpoint::new(Some(identity));
        checkpoint::save(&config.checkpoint_path, &updated_checkpoint).map_err(RunOutcome::CheckpointSaveFailed)?;
    }

    Ok(report)
}

/// Resolves this run's `SourceLogRegistration`, distinguishing the three
/// cases the frozen correction names explicitly:
/// - registration file present and loads cleanly: verify relocation
///   compatibility against the current source chain (fail closed on
///   mismatch, per `crate::identity::verify_relocation`), then use it.
/// - registration file present but malformed/wrong version: fail closed
///   immediately ([`RunOutcome::RegistrationLoadFailed`]) — never
///   silently treated as absent.
/// - registration file absent: attempt recovery from destination
///   evidence FIRST (`crate::registration::recover_from_destination_evidence`).
///   If that finds no evidence at all, this is treated as a genuine
///   first-ever run and a fresh `SourceLogId` is minted. If it finds
///   evidence but cannot resolve a UNIQUE candidate, this fails closed
///   ([`RunOutcome::RegistrationRecoveryFailed`]) rather than minting a
///   fresh identity that would silently duplicate prior imports.
fn resolve_registration(config: &RunConfig, source_scan: &audit_chain_core::ScanResult, destination_scan: &audit_chain_core::ScanResult) -> Result<SourceLogRegistration, RunOutcome> {
    match registration::load(&config.registration_path) {
        Ok(Some(file)) => {
            verify_relocation(&file.registration, source_scan)?;
            Ok(SourceLogRegistration { source_log_id: file.registration.source_log_id, current_locator: config.source_vault_root.clone(), last_known_tail: file.registration.last_known_tail })
        }
        Ok(None) => match registration::recover_from_destination_evidence(&destination_scan.verified_records, &source_scan.verified_records, &config.source_vault_root) {
            Ok(Some(recovered)) => {
                registration::save(&config.registration_path, &RegistrationFile::new(recovered.clone())).map_err(RunOutcome::RegistrationSaveFailed)?;
                Ok(recovered)
            }
            Ok(None) => {
                // No destination evidence at all -- a genuine first-ever
                // run, never seen before under any identity.
                Ok(SourceLogRegistration { source_log_id: SourceLogId::generate(), current_locator: config.source_vault_root.clone(), last_known_tail: None })
            }
            Err(e) => Err(RunOutcome::RegistrationRecoveryFailed(e)),
        },
        Err(e) => Err(RunOutcome::RegistrationLoadFailed(e)),
    }
}

fn scan_chain_bytes(raw: &[u8]) -> audit_chain_core::ScanResult {
    let lines = split_into_line_outcomes(raw);
    audit_chain_core::scan_chain(&lines)
}

fn destination_vault_raw_bytes(root: &std::path::Path, journal_path: &std::path::Path) -> Result<Vec<u8>, storage::StorageError> {
    let vault = storage::Vault::open_with_default_indexer(root.to_path_buf(), journal_path)?;
    vault.read_audit_log_for_recovery_scan()
}

/// Mirrors `brain_api_audit_worker::recovery`'s own private line-splitting
/// logic (that function is not public, and this crate must not depend on
/// `brain-api-audit-worker`'s internals beyond its public surface) --
/// same exact rule: a line is `is_physical_tail` only when the raw bytes
/// end without a trailing newline AND it is the last line.
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
            Err(_) => {
                let is_physical_tail = !ends_with_newline && index == last_index;
                audit_chain_core::LineOutcome::Unparseable { is_physical_tail }
            }
        })
        .collect()
}
