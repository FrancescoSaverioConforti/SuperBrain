//! M7.7.1: the one seam between the bridge's own domain logic
//! (mapping/dedup/registration/checkpoint, all invariant) and WHERE
//! destination bytes actually live. [`DestinationAccess`] is the only
//! abstraction this crate exposes for that seam — the legacy
//! single-vault path and a future segmented composition layer
//! (`mcp-audit-segmented`, a sibling crate, never a dependency of this
//! one) both implement it, and [`crate::run::run_with_destination`] is
//! the ONLY place that contains the bridge's real algorithm. Neither
//! implementation duplicates mapping/dedup/registration logic — they
//! only answer "where are the bytes" and "where does an append land."
//!
//! **Native `async fn` in traits** (stable since Rust 1.75, this
//! workspace's toolchain is 1.99.1): no `async-trait` dependency is
//! introduced. [`DestinationAccess`] is used only as a generic bound
//! (`D: DestinationAccess`), never as a trait object, so the one real
//! limitation of native async trait methods (no `dyn` support without
//! extra ceremony) never applies here.

use audit_chain_core::ChainRecord;

/// Whether the destination history available to THIS call is
/// sufficient to answer dedup/registration-recovery questions safely.
/// Conservative by design (the frozen M7.7 correction): there is no
/// comparison between source-chain sequence and destination-segment
/// sequence anywhere in this crate — those are two independent numeric
/// domains, never comparable, and this crate never pretends otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryCompleteness {
    /// Every destination record potentially relevant to dedup/
    /// registration recovery is still physically present.
    Complete,
    /// At least one destination segment that could have held bridge
    /// provenance is no longer available (retained/retired) — safety
    /// over availability: this crate never tries to prove such a
    /// segment was "irrelevant" from its mere absence or from any
    /// sequence-based heuristic. A caller seeing this must fail closed
    /// ([`crate::run::RunOutcome::HistoricalEvidenceUnavailable`]),
    /// never guess.
    Incomplete,
}

/// One destination read's full result — verified records plus the
/// completeness judgment a [`DestinationAccess`] implementation is
/// responsible for making about ITSELF (a single, unsegmented vault is
/// always `Complete`; a segmented composition layer may report
/// `Incomplete` when retention has occurred).
pub struct DestinationHistory {
    pub verified_records: Vec<ChainRecord>,
    pub completeness: HistoryCompleteness,
}

#[derive(Debug, thiserror::Error)]
pub enum DestinationAccessError {
    #[error("failed to open destination vault: {0:?}")]
    VaultOpenFailed(storage::StorageError),
    #[error("failed to read destination audit log: {0:?}")]
    ReadFailed(storage::StorageError),
    #[error("destination chain failed verification: {0:?}")]
    ChainInvalid(audit_chain_core::ScanStopReason),
    #[error("failed to start the destination audit worker: {0}")]
    WorkerStartupFailed(brain_api_audit_worker::WorkerStartupError),
    #[error("durable append to the destination chain failed: {0}")]
    AppendFailed(brain_api_audit_worker::DurableRecordError),
}

/// The only seam through which [`crate::run::run_with_destination`]
/// ever touches "the destination" — never `storage::Vault` directly.
/// An implementation decides what "destination" even means (one vault,
/// many verified segments, ...); the bridge's own algorithm never
/// knows or cares which.
pub trait DestinationAccess {
    /// Every verified destination record relevant to dedup/
    /// registration recovery, re-derived fresh on every call — never
    /// cached by an implementation across calls within one run (the
    /// same "never trust a stale snapshot" discipline the pre-M7.7
    /// `run_once` already enforced by re-scanning before every single
    /// append).
    ///
    /// Desugared to `-> impl Future + Send` (rather than `async fn`
    /// directly) so this trait's futures are usable across
    /// `tokio::spawn`/multi-thread-runtime boundaries — exactly the
    /// context this crate's own acceptance tests already exercise
    /// `run_once`/`run_with_destination` under.
    fn read_verified_history(&self) -> impl std::future::Future<Output = Result<DestinationHistory, DestinationAccessError>> + Send;

    /// Appends one event durably to whatever this implementation
    /// considers its current write target. For a legacy single-vault
    /// implementation this is the one configured vault; for a
    /// segmented implementation this MUST be the exact segment a
    /// caller's own pin protects — never re-resolved dynamically mid-
    /// call (an `mcp-audit-segmented` concern, not this trait's own
    /// job to enforce, since this crate has no concept of "segment" or
    /// "pin" at all).
    fn append_durable(&self, event: brain_api::AuditEvent) -> impl std::future::Future<Output = Result<(), DestinationAccessError>> + Send;
}

/// The legacy implementation: exactly one fixed `storage::Vault`,
/// always `HistoryCompleteness::Complete` (a single, unsegmented vault
/// has no concept of retention at all, so there is never a reason to
/// report anything else — this is the correct, not an approximated,
/// answer for this case).
pub struct SingleVaultDestination {
    pub root: std::path::PathBuf,
    pub journal_path: std::path::PathBuf,
}

impl DestinationAccess for SingleVaultDestination {
    async fn read_verified_history(&self) -> Result<DestinationHistory, DestinationAccessError> {
        let raw = destination_vault_raw_bytes(&self.root, &self.journal_path).map_err(DestinationAccessError::ReadFailed)?;
        let scan = scan_chain_bytes(&raw);
        if let Some(stop_reason) = &scan.stop_reason {
            if !audit_chain_core::may_resume_without_recovery(&scan) {
                return Err(DestinationAccessError::ChainInvalid(stop_reason.clone()));
            }
        }
        Ok(DestinationHistory { verified_records: scan.verified_records, completeness: HistoryCompleteness::Complete })
    }

    async fn append_durable(&self, event: brain_api::AuditEvent) -> Result<(), DestinationAccessError> {
        let vault = storage::Vault::open_with_default_indexer(self.root.clone(), &self.journal_path).map_err(DestinationAccessError::VaultOpenFailed)?;
        let worker = brain_api_audit_worker::spawn_worker(vault, brain_api_audit_worker::WorkerConfig::default()).map_err(DestinationAccessError::WorkerStartupFailed)?;
        let result = worker.record_durable(event).await.map(|_ack| ()).map_err(DestinationAccessError::AppendFailed);
        let _ = worker.shutdown().await;
        result
    }
}

pub(crate) fn scan_chain_bytes(raw: &[u8]) -> audit_chain_core::ScanResult {
    audit_chain_core::scan_chain(&split_into_line_outcomes(raw))
}

fn destination_vault_raw_bytes(root: &std::path::Path, journal_path: &std::path::Path) -> Result<Vec<u8>, storage::StorageError> {
    let vault = storage::Vault::open_with_default_indexer(root.to_path_buf(), journal_path)?;
    vault.read_audit_log_for_recovery_scan()
}

/// Mirrors `brain_api_audit_worker::recovery`'s own private line-
/// splitting logic (not public, and this crate must not depend on
/// `brain-api-audit-worker`'s internals beyond its public surface).
pub(crate) fn split_into_line_outcomes(raw: &[u8]) -> Vec<audit_chain_core::LineOutcome> {
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
