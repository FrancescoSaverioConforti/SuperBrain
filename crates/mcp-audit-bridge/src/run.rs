//! M7 / M7.7: one batch/offline bridge run — ties `crate::identity`/
//! `crate::registration`/`crate::checkpoint`/`crate::dedup`/
//! `crate::mapping` together against a real source `storage::Vault`
//! (read-only) and a [`crate::destination::DestinationAccess`]
//! implementation (M7.7: generalized from a single fixed
//! `storage::Vault` so a sibling composition crate can supply a
//! segmented, multi-Vault view without this crate ever depending on
//! it or knowing the word "segment").
//!
//! **`run_with_destination` is the ONLY owner of [`bridge_lock`]**
//! (the frozen M7.7 correction): [`run_once`] (the legacy, single-
//! vault wrapper) never acquires it itself — it constructs a
//! [`crate::destination::SingleVaultDestination`] and delegates
//! entirely to `run_with_destination`, which is also what a segmented
//! composition layer calls. There is exactly one lock-acquisition
//! site in this entire crate, so there is no way for two different
//! callers to disagree about lock order.
//!
//! **Registration vs. checkpoint — two separate files, two separate
//! lifecycles**: the durable `SourceLogId` lives in
//! `crate::registration`'s own file, never in the checkpoint. Losing
//! the checkpoint is always harmless (rebuilt from a full destination
//! scan under the SAME identity). Losing the registration is a
//! DIFFERENT, more serious case — and, as of M7.7, [`run_with_destination`]
//! NEVER mints a fresh `SourceLogId` automatically. See
//! [`resolve_registration`]'s own doc comment for the full, now-
//! explicit policy.
//!
//! **Per-record algorithm** (frozen design, with the dedup correction
//! applied): for every new source record, in order —
//! 1. build its `SourceRecordIdentity`;
//! 2. check membership in a FULL scan of the destination history
//!    (never the checkpoint alone, never a tail-only search);
//! 3. if already imported: skip the append, advance the checkpoint only;
//! 4. if not: translate, append durably, THEN persist the checkpoint.

use crate::bridge_lock::{self, BridgeLockError};
use crate::checkpoint::{self, BridgeCheckpoint, CheckpointError};
use crate::dedup::{scan_destination_for_imported_identities, DestinationAnomaly};
use crate::destination::{scan_chain_bytes, DestinationAccess, DestinationAccessError, HistoryCompleteness, SingleVaultDestination};
use crate::identity::{verify_relocation, RelocationError, SourceLogId, SourceLogRegistration, SourceRecordIdentity};
use crate::registration::{self, RegistrationError, RegistrationFile, RegistrationRecoveryError};

/// Everything the legacy, single-vault `run_once` path needs.
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
    /// file from `checkpoint_path`, different lifecycle.
    pub registration_path: std::path::PathBuf,
    /// Where this bridge's mutable progress state lives.
    pub checkpoint_path: std::path::PathBuf,
    /// Test/fault-injection hook only: when `Some`, the run returns a
    /// failure immediately after this many successful durable
    /// destination appends in THIS call, before the checkpoint for
    /// that append is saved — the real "destination append succeeded,
    /// then the process died before the checkpoint write" crash
    /// window, exercised through the actual production code path.
    /// `None` in every real composition root; acceptance tests are the
    /// only caller that ever sets this.
    pub crash_after_n_destination_appends: Option<usize>,
}

impl RunConfig {
    pub fn new(source_vault_root: std::path::PathBuf, source_journal_path: std::path::PathBuf, destination_vault_root: std::path::PathBuf, destination_journal_path: std::path::PathBuf, registration_path: std::path::PathBuf, checkpoint_path: std::path::PathBuf) -> Self {
        Self { source_vault_root, source_journal_path, destination_vault_root, destination_journal_path, registration_path, checkpoint_path, crash_after_n_destination_appends: None }
    }
}

/// The source side's own config — unchanged in shape from before
/// M7.7, pulled out as its own type so `run_with_destination` does not
/// need the destination-specific fields `RunConfig` still carries for
/// the legacy wrapper's own convenience.
pub struct SourceConfig {
    pub root: std::path::PathBuf,
    pub journal_path: std::path::PathBuf,
}

/// What happened during one run — enough detail for the acceptance
/// tests (and a real operator) to verify exactly what was imported,
/// skipped, or flagged, without re-deriving it from logs.
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
    /// M7.7: registration is missing, and destination evidence alone
    /// is never sufficient to decide anything automatically anymore —
    /// see [`resolve_registration`]'s own doc comment. This is now a
    /// terminal, typed outcome a caller must handle explicitly (by
    /// calling [`register_source`]), never a condition `run_once`
    /// silently resolves by minting a fresh identity.
    #[error("no registration exists for this source, and this crate never mints one automatically -- call register_source explicitly after confirming this is a genuine first-ever run")]
    RegistrationMissing,
    /// M7.7: registration is missing, destination evidence identifies
    /// exactly one candidate `SourceLogId`, and the destination history
    /// available is `Complete` — the ONE case recovery may proceed
    /// automatically, by reconstructing (never minting) the existing
    /// identity.
    #[error("failed to save recovered registration: {0}")]
    RegistrationRecoverySaveFailed(RegistrationError),
    #[error("registration recovery from destination evidence failed: {0}")]
    RegistrationRecoveryFailed(RegistrationRecoveryError),
    /// M7.7: the destination history available to this call is
    /// `Incomplete` (see [`HistoryCompleteness`]) at a point where
    /// either dedup or registration recovery would otherwise need to
    /// rely on it — fail closed rather than risk a duplicate import or
    /// a silently-wrong registration. Safety over availability,
    /// deliberately: this crate never tries to prove a retained
    /// segment was irrelevant.
    #[error("destination history required for a safe decision is no longer completely available (some destination segment has been retained/retired) -- failing closed rather than risking a duplicate import or an incorrect registration")]
    HistoricalEvidenceUnavailable,
    #[error("failed to load checkpoint: {0}")]
    CheckpointLoadFailed(#[from] CheckpointError),
    #[error("failed to save checkpoint: {0}")]
    CheckpointSaveFailed(CheckpointError),
    #[error("destination access failed: {0}")]
    Destination(#[from] DestinationAccessError),
    #[error("source record at sequence {sequence} passed chain verification but its content did not decode as a valid McpClientEvent: {detail}")]
    SourceEventMalformed { sequence: u64, detail: String },
    /// Test/fault-injection only — see [`RunConfig::crash_after_n_destination_appends`].
    #[error("injected fault: simulated crash after {appends_so_far} destination append(s), before the checkpoint for the most recent one was saved")]
    InjectedFault { appends_so_far: usize },
    /// Another run already holds the cross-process bridge lock for
    /// this state directory — refused immediately, never waited out.
    #[error(transparent)]
    AlreadyRunning(#[from] BridgeLockError),
    /// M7.7 anti-bypass tripwire — see `crate::anti_bypass`'s own doc
    /// comment for why this is defense-in-depth only, never a trust
    /// boundary.
    #[error(transparent)]
    SegmentedDestinationGuard(#[from] crate::anti_bypass::SegmentedDestinationGuardError),
}

/// M7.7.6: carries, alongside the terminal error, whether at least one
/// destination append was durably confirmed before this run failed —
/// the ONE fact a caller (e.g. a segmented composition layer deciding
/// whether to release a `RetentionPin`) needs, and the only thing that
/// fact may ever be derived from: the exact point where
/// `DestinationAccess::append_durable` returns `Ok`, never from
/// `RunOutcome`'s own variant, never from `imported.len()` on a
/// partially-constructed report (a failure can occur before any report
/// is even returned).
#[derive(Debug, thiserror::Error)]
#[error("{outcome}")]
pub struct RunFailure {
    #[source]
    pub outcome: RunOutcome,
    pub at_least_one_durable_append_occurred: bool,
}

impl<E: Into<RunOutcome>> From<E> for RunFailure {
    fn from(e: E) -> Self {
        RunFailure { outcome: e.into(), at_least_one_durable_append_occurred: false }
    }
}

/// Legacy, single-vault wrapper — delegates entirely to
/// [`run_with_destination`], never acquires [`bridge_lock`] itself
/// (the frozen M7.7 invariant: exactly one lock-acquisition site in
/// this crate).
///
/// **M7.7 anti-bypass tripwire (defense-in-depth only, never a trust
/// boundary)**: if a `segment-manifest.json` file exists as a sibling
/// of `destination_vault_root`, this function refuses immediately
/// with [`RunOutcome::SegmentedDestinationGuard`] rather than silently
/// operating on a single segment of what is actually a segmented
/// destination. This is a best-effort tripwire against *accidental*
/// misuse, not a security boundary — see `crate::anti_bypass`'s own
/// doc comment for exactly what it does and does not guarantee.
pub async fn run_once(config: &RunConfig) -> Result<RunReport, RunFailure> {
    crate::anti_bypass::guard_against_segmented_destination(&config.destination_vault_root)?;
    let destination = SingleVaultDestination { root: config.destination_vault_root.clone(), journal_path: config.destination_journal_path.clone() };
    let source = SourceConfig { root: config.source_vault_root.clone(), journal_path: config.source_journal_path.clone() };
    run_with_destination(&destination, &source, &config.registration_path, &config.checkpoint_path, config.crash_after_n_destination_appends).await
}

/// The bridge's ONE real implementation of mapping/dedup/registration/
/// checkpoint logic — generic over `D: DestinationAccess`, so neither
/// `run_once` nor a future segmented composition layer duplicates any
/// of it. Acquires [`bridge_lock`] as its very first action (the sole
/// acquisition site in this crate) and holds it for this function's
/// entire remaining body.
pub async fn run_with_destination<D: DestinationAccess>(destination: &D, source: &SourceConfig, registration_path: &std::path::Path, checkpoint_path: &std::path::Path, crash_after_n_destination_appends: Option<usize>) -> Result<RunReport, RunFailure> {
    let _lock = bridge_lock::acquire(checkpoint_path)?;

    // --- source side: open read-only, verify the full chain ---
    let source_vault = storage::Vault::open_with_default_indexer(source.root.clone(), &source.journal_path).map_err(RunOutcome::SourceVaultOpenFailed)?;
    let source_raw = source_vault.read_audit_log_for_recovery_scan().map_err(RunOutcome::SourceReadFailed)?;
    let source_scan = scan_chain_bytes(&source_raw);
    if let Some(stop_reason) = &source_scan.stop_reason {
        if !audit_chain_core::may_resume_without_recovery(&source_scan) {
            return Err(RunOutcome::SourceChainInvalid(stop_reason.clone()).into());
        }
    }

    // --- destination side: read history once up front, for registration recovery only ---
    let history_for_recovery = destination.read_verified_history().await?;

    // --- registration: load, recover (only if history is Complete), or fail closed -- never mint automatically ---
    let registration = resolve_registration(registration_path, source, &source_scan, &history_for_recovery)?;
    let source_log_id = registration.source_log_id;

    // --- checkpoint: progress only, its own absence is always harmless once registration is settled ---
    let loaded_checkpoint = checkpoint::load(checkpoint_path)?;
    let last_imported_sequence = loaded_checkpoint.as_ref().and_then(|cp| cp.last_imported.as_ref()).map(|i| i.sequence);

    let ctx = ProcessCandidatesContext { source, registration_path, checkpoint_path, source_log_id, last_imported_sequence, crash_after_n_destination_appends };
    process_candidates(destination, &ctx, &source_scan).await
}

/// Private grouping of `process_candidates`' own fixed-for-the-call
/// parameters — pure parameter-count reduction, never a new public
/// shape; `run_with_destination` is still the only caller.
struct ProcessCandidatesContext<'a> {
    source: &'a SourceConfig,
    registration_path: &'a std::path::Path,
    checkpoint_path: &'a std::path::Path,
    source_log_id: SourceLogId,
    last_imported_sequence: Option<u64>,
    crash_after_n_destination_appends: Option<usize>,
}

async fn process_candidates<D: DestinationAccess>(destination: &D, ctx: &ProcessCandidatesContext<'_>, source_scan: &audit_chain_core::ScanResult) -> Result<RunReport, RunFailure> {
    let ProcessCandidatesContext { source, registration_path, checkpoint_path, source_log_id, last_imported_sequence, crash_after_n_destination_appends } = *ctx;
    let mut report = RunReport::default();
    let mut appends_so_far = 0usize;

    // Candidate records: every source record not yet BELOW the
    // checkpoint's own last imported sequence -- an optimization over
    // always starting from sequence 0 (the full destination scan below
    // is still what actually decides import-or-skip for each one).
    let candidates: Vec<&audit_chain_core::ChainRecord> = source_scan.verified_records.iter().filter(|r| last_imported_sequence.map(|last| r.sequence > last).unwrap_or(true)).collect();

    for record in &candidates {
        let identity = SourceRecordIdentity { source_log_id, sequence: record.sequence, record_hash: record.record_hash.clone() };

        // Full destination scan, re-derived before THIS append -- never
        // trusting the checkpoint alone, never a tail-only search.
        let destination_history = destination.read_verified_history().await.map_err(|e| RunFailure { outcome: e.into(), at_least_one_durable_append_occurred: appends_so_far > 0 })?;
        if destination_history.completeness == HistoryCompleteness::Incomplete {
            return Err(RunFailure { outcome: RunOutcome::HistoricalEvidenceUnavailable, at_least_one_durable_append_occurred: appends_so_far > 0 });
        }
        let destination_state = scan_destination_for_imported_identities(&destination_history.verified_records);
        report.anomalies.extend(destination_state.anomalies.iter().cloned());

        if destination_state.imported.contains(&identity) {
            report.skipped_already_imported.push(identity.clone());
        } else {
            let source_event: mcp_client_audit_worker::McpClientEvent = audit_chain_core::parse_event(&record.content_bytes)
                .map_err(|e| RunFailure { outcome: RunOutcome::SourceEventMalformed { sequence: record.sequence, detail: e.to_string() }, at_least_one_durable_append_occurred: appends_so_far > 0 })?;
            let translated_kind = crate::mapping::translate(&source_event.kind, &identity);
            let event = brain_api::event(translated_kind, &source_event.timestamp, uuid::Uuid::new_v4());

            // The exact, single point where "at least one durable
            // append occurred" becomes true -- set only on a
            // confirmed Ok from append_durable, never inferred from
            // anything else (the M7.7.6 frozen rule).
            destination.append_durable(event).await.map_err(|e| RunFailure { outcome: e.into(), at_least_one_durable_append_occurred: appends_so_far > 0 })?;
            appends_so_far += 1;
            report.imported.push(identity.clone());

            if crash_after_n_destination_appends == Some(appends_so_far) {
                // Simulated crash: the destination append above already
                // committed durably -- this return happens BEFORE the
                // checkpoint save below, faithfully modeling "process
                // died in that exact window" through the real code
                // path, not a hand-rolled bypass. at_least_one_durable_append_occurred
                // is true here: an append genuinely just succeeded.
                return Err(RunFailure { outcome: RunOutcome::InjectedFault { appends_so_far }, at_least_one_durable_append_occurred: true });
            }
        }

        // Update the REGISTRATION's own last_known_tail (needed for
        // future relocation verification) and the CHECKPOINT's
        // last_imported -- two separate files, both updated after this
        // record either way (skip or import).
        let updated_registration = SourceLogRegistration { source_log_id, current_locator: source.root.clone(), last_known_tail: Some((record.sequence, record.record_hash.clone())) };
        registration::save(registration_path, &RegistrationFile::new(updated_registration)).map_err(|e| RunFailure { outcome: RunOutcome::RegistrationSaveFailed(e), at_least_one_durable_append_occurred: appends_so_far > 0 })?;

        let updated_checkpoint = BridgeCheckpoint::new(Some(identity));
        checkpoint::save(checkpoint_path, &updated_checkpoint).map_err(|e| RunFailure { outcome: RunOutcome::CheckpointSaveFailed(e), at_least_one_durable_append_occurred: appends_so_far > 0 })?;
    }

    Ok(report)
}

/// M7.7.2: resolves this run's `SourceLogRegistration` under the new,
/// explicit policy — this function NEVER mints a fresh `SourceLogId`
/// automatically, in any branch. The prior (pre-M7.7) behavior of
/// treating "registration absent, zero destination evidence found" as
/// a license to call `SourceLogId::generate()` is gone: that ambiguity
/// (genuine first run vs. registration lost vs. evidence legitimately
/// retired) is exactly what a prior integration review found unsafe.
///
/// - registration file present and loads cleanly: verify relocation
///   compatibility against the current source chain (fail closed on
///   mismatch), then use it. Unchanged from before M7.7.
/// - registration file present but malformed/wrong version: fail
///   closed immediately. Unchanged.
/// - registration file absent, destination history `Incomplete`:
///   [`RunOutcome::HistoricalEvidenceUnavailable`] — never attempts
///   `recover_from_destination_evidence` at all, since "zero evidence
///   found" would be indistinguishable from "evidence existed but was
///   retired."
/// - registration file absent, destination history `Complete`,
///   evidence identifies exactly one candidate: recovery proceeds,
///   reconstructing (never minting) that identity.
/// - registration file absent, destination history `Complete`, zero
///   evidence found anywhere: [`RunOutcome::RegistrationMissing`] —
///   this IS the genuine first-ever-run case, but this function no
///   longer resolves it by itself; a caller must call
///   [`register_source`] explicitly, after whatever external
///   confirmation it requires that this really is a first run.
/// - registration file absent, destination history `Complete`,
///   evidence found but ambiguous (more than one candidate): fail
///   closed, unchanged from before M7.7.
fn resolve_registration(registration_path: &std::path::Path, source: &SourceConfig, source_scan: &audit_chain_core::ScanResult, destination_history: &crate::destination::DestinationHistory) -> Result<SourceLogRegistration, RunFailure> {
    match registration::load(registration_path) {
        Ok(Some(file)) => {
            verify_relocation(&file.registration, source_scan)?;
            Ok(SourceLogRegistration { source_log_id: file.registration.source_log_id, current_locator: source.root.clone(), last_known_tail: file.registration.last_known_tail })
        }
        Ok(None) => {
            if destination_history.completeness == HistoryCompleteness::Incomplete {
                return Err(RunOutcome::HistoricalEvidenceUnavailable.into());
            }
            match registration::recover_from_destination_evidence(&destination_history.verified_records, &source_scan.verified_records, &source.root) {
                Ok(Some(recovered)) => {
                    registration::save(registration_path, &RegistrationFile::new(recovered.clone())).map_err(RunOutcome::RegistrationRecoverySaveFailed)?;
                    Ok(recovered)
                }
                Ok(None) => Err(RunOutcome::RegistrationMissing.into()),
                Err(e) => Err(RunOutcome::RegistrationRecoveryFailed(e).into()),
            }
        }
        Err(e) => Err(RunOutcome::RegistrationLoadFailed(e).into()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterSourceError {
    #[error("a registration already exists at this path -- register_source only runs once, on a genuinely unregistered source")]
    AlreadyRegistered,
    #[error("failed to check for an existing registration: {0}")]
    LoadFailed(#[from] RegistrationError),
    #[error("failed to save the new registration: {0}")]
    SaveFailed(RegistrationError),
}

/// M7.7.2: the ONE explicit, caller-initiated action that may ever mint
/// a fresh `SourceLogId` for a source that has no registration yet.
/// Never called automatically by [`run_with_destination`]/[`run_once`]
/// — a caller (an operator, or a composition layer with its own policy
/// for what "confirmed first run" means) invokes this deliberately,
/// once, before the first real `run_once`/`run_with_destination` call
/// for that source.
///
/// Fails if a registration already exists at `registration_path` —
/// this is a first-registration operation, never an implicit
/// overwrite.
pub fn register_source(registration_path: &std::path::Path, source_vault_root: &std::path::Path) -> Result<SourceLogRegistration, RegisterSourceError> {
    if registration::load(registration_path)?.is_some() {
        return Err(RegisterSourceError::AlreadyRegistered);
    }
    let registration = SourceLogRegistration { source_log_id: SourceLogId::generate(), current_locator: source_vault_root.to_path_buf(), last_known_tail: None };
    registration::save(registration_path, &RegistrationFile::new(registration.clone())).map_err(RegisterSourceError::SaveFailed)?;
    Ok(registration)
}
