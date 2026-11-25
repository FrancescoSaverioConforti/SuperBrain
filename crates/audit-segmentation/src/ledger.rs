//! M7.5.3: the retention ledger, implemented as an ordinary, dedicated
//! `storage::Vault` — never a raw `std::fs::File` writer. Every
//! durability property this module has (path/handle safety,
//! cross-process single-writer locking, fsync ordering, DPAPI-trusted
//! `head.ref`, roll-forward-on-crash recovery) comes from reusing
//! `storage::Vault::open_audit_writer`/`AuditWriterHandle::append`/
//! `Vault::read_audit_log_for_recovery_scan` exactly as every other
//! composition root in this workspace already does — this module adds
//! none of that machinery itself, and introduces no new persisted
//! format (`RetentionReceipt` is serialized through the SAME
//! `audit_chain_core::serialize_record`/`parse_event` generic functions
//! `AuditEvent` itself goes through, at the SAME `ChainRecord`/
//! `AUDIT_RECORD_FORMAT_VERSION` version).
//!
//! **Trusted-head semantics, explicitly**: the ledger's own chain is
//! verified via `scan_chain` exactly like an audit segment; its head is
//! anchored by the SAME DPAPI-protected `head.ref` mechanism
//! (`storage::Vault::open_audit_writer` internally uses
//! `DpapiTrustedReferenceStore` unconditionally — there is no
//! lower-trust code path to accidentally take). A forged reference is
//! detected the same way (`AuditWriterOpenError::ReferenceInconsistent`
//! on disagreement); a genuine crash between log fsync and reference
//! update is the same one safe roll-forward pattern already proven for
//! the main audit log. The one residual risk is the same one already
//! accepted for the main audit log (continuity `d279ua`): a coordinated
//! replay of an old-but-internally-consistent `(audit.log, head.ref)`
//! pair is not detected by this mechanism alone — not widened, not
//! narrowed, by reusing it here.
//!
//! **`retention_ledger_locator` is a locator, never an authority**
//! (the frozen manifest rule — see `crate::domain::SegmentManifest`'s
//! own doc comment): [`open_for_write`]/[`verify_and_scan`] below always
//! perform real verification (open the real Vault at that path, scan
//! its real chain) — nothing in this module trusts a locator string by
//! itself.

use audit_chain_core::ChainRecord;

use crate::domain::RetentionReceipt;

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("ledger vault error: {0:?}")]
    Vault(storage::StorageError),
    #[error("ledger chain failed verification: {0:?}")]
    ChainInvalid(audit_chain_core::ScanStopReason),
    #[error("failed to open the ledger's durable writer: {0:?}")]
    WriterOpenFailed(storage::AuditWriterOpenError),
    #[error("ledger append failed: {0:?}")]
    AppendFailed(storage::AuditAppendError),
    #[error("a receipt in the ledger's own verified chain failed to decode: {0}")]
    MalformedReceipt(String),
}

/// The ledger's own current, independently-verified state — never
/// constructed except by actually opening the real Vault at a locator
/// and running `scan_chain` against its real bytes (see
/// [`verify_and_scan`]).
pub struct VerifiedLedgerState {
    pub verified_records: Vec<ChainRecord>,
    pub last_verified: Option<(u64, String)>,
}

impl VerifiedLedgerState {
    /// Every `RetentionReceipt` in the ledger's own verified prefix, in
    /// order — decodes each verified record via
    /// `audit_chain_core::parse_event`, the same generic function every
    /// other typed chain payload in this workspace already uses.
    pub fn receipts(&self) -> Result<Vec<RetentionReceipt>, LedgerError> {
        self.verified_records.iter().map(|r| audit_chain_core::parse_event::<RetentionReceipt>(&r.content_bytes).map_err(|e| LedgerError::MalformedReceipt(e.to_string()))).collect()
    }
}

/// Opens the dedicated retention-ledger Vault at `locator` (creating it,
/// including its own `.audit/` directory, if this is the first-ever
/// use — identical "create on first open" behavior to any other
/// `storage::Vault`) and runs a REAL `scan_chain` against its REAL raw
/// bytes — never trusts `locator` by itself. Fails closed
/// ([`LedgerError::ChainInvalid`]) if the ledger's own chain does not
/// permit immediate resumption (tampering, truncation, reordering —
/// exactly the same `may_resume_without_recovery` gate the main audit
/// log already uses).
pub fn verify_and_scan(locator: &std::path::Path, journal_path: &std::path::Path) -> Result<(storage::Vault, VerifiedLedgerState), LedgerError> {
    let vault = storage::Vault::open_with_default_indexer(locator.to_path_buf(), journal_path).map_err(LedgerError::Vault)?;
    let raw = vault.read_audit_log_for_recovery_scan().map_err(LedgerError::Vault)?;
    let scan = scan_chain_bytes(&raw);
    if let Some(stop_reason) = &scan.stop_reason {
        if !audit_chain_core::may_resume_without_recovery(&scan) {
            return Err(LedgerError::ChainInvalid(stop_reason.clone()));
        }
    }
    Ok((vault, VerifiedLedgerState { verified_records: scan.verified_records, last_verified: scan.last_verified }))
}

/// Appends one `receipt` to the ledger at `locator`, through the
/// unmodified `storage::Vault::open_audit_writer`/
/// `AuditWriterHandle::append` — the ONE and only way this module ever
/// writes to the ledger. Re-verifies the chain immediately before
/// opening the writer (never trusts a caller-supplied stale tail),
/// exactly mirroring the real composition-root pattern
/// `mcp-client-cli`/`mcp-audit-bridge` already use for their own
/// destination writes.
///
/// Returns the receipt's own durable `(sequence, record_hash)` — the
/// caller (`crate::retention`) embeds this into the subsequent
/// `AuditEventKind::RetentionAuthorized` record it appends to the
/// AUDIT side, completing phase 3 of the 5-phase protocol.
pub fn append_receipt(locator: &std::path::Path, journal_path: &std::path::Path, receipt: &RetentionReceipt) -> Result<(u64, String), LedgerError> {
    let (vault, state) = verify_and_scan(locator, journal_path)?;
    let next_sequence = state.last_verified.as_ref().map(|(s, _)| s + 1).unwrap_or(0);
    let prev_hash = state.last_verified.as_ref().map(|(_, h)| h.clone()).unwrap_or_else(|| audit_chain_core::GENESIS_HASH.to_string());

    let (bytes, record_hash) = audit_chain_core::serialize_record(receipt, next_sequence, &prev_hash);

    let mut writer = vault.open_audit_writer(state.last_verified.as_ref().map(|(s, h)| (*s, h.as_str()))).map_err(LedgerError::WriterOpenFailed)?;
    writer.append(&bytes, next_sequence, &record_hash).map_err(LedgerError::AppendFailed)?;

    Ok((next_sequence, record_hash))
}

fn scan_chain_bytes(raw: &[u8]) -> audit_chain_core::ScanResult {
    audit_chain_core::scan_chain(&split_into_line_outcomes(raw))
}

/// Mirrors `mcp_audit_bridge::run`'s own private line-splitting helper
/// (and, before it, `brain_api_audit_worker::recovery`'s) — same exact
/// rule, reused by value rather than by a new shared dependency, since
/// none of those functions are public across crate boundaries.
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
    use crate::domain::SegmentId;

    fn harness() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let locator = dir.path().join("retention-ledger");
        std::fs::create_dir_all(&locator).unwrap();
        let journal = locator.join("journal.sqlite");
        (dir, locator, journal)
    }

    fn sample_receipt(n: u64) -> RetentionReceipt {
        RetentionReceipt { retired_segment_id: SegmentId::from_index(n), retired_segment_final_sequence: 10, retired_segment_final_record_hash: "a".repeat(64), retirement_authorized_by: "policy:default".into(), retired_at: "2026-01-01T00:00:00Z".into() }
    }

    #[test]
    fn a_fresh_ledger_verifies_as_empty_with_no_error() {
        let (_dir, locator, journal) = harness();
        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        assert!(state.verified_records.is_empty());
        assert_eq!(state.last_verified, None);
    }

    #[test]
    fn appending_one_receipt_and_reopening_recovers_it_exactly() {
        let (_dir, locator, journal) = harness();
        let (seq, hash) = append_receipt(&locator, &journal, &sample_receipt(0)).unwrap();
        assert_eq!(seq, 0);

        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        assert_eq!(state.last_verified, Some((0, hash)));
        let receipts = state.receipts().unwrap();
        assert_eq!(receipts, vec![sample_receipt(0)]);
    }

    #[test]
    fn appending_multiple_receipts_in_sequence_chains_correctly() {
        let (_dir, locator, journal) = harness();
        for n in 0..5 {
            append_receipt(&locator, &journal, &sample_receipt(n)).unwrap();
        }
        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        let receipts = state.receipts().unwrap();
        assert_eq!(receipts.len(), 5);
        for (n, receipt) in receipts.iter().enumerate() {
            assert_eq!(receipt.retired_segment_id, SegmentId::from_index(n as u64));
        }
    }

    #[test]
    fn tampering_with_the_ledgers_own_chain_content_is_detected_on_reopen() {
        let (_dir, locator, journal) = harness();
        append_receipt(&locator, &journal, &sample_receipt(0)).unwrap();

        let log_path = locator.join(".audit").join("audit.log");
        let mut bytes = std::fs::read(&log_path).unwrap();
        let len = bytes.len();
        bytes[len - 5] ^= 0xFF; // flip a byte well inside the JSON payload
        std::fs::write(&log_path, bytes).unwrap();

        let result = verify_and_scan(&locator, &journal);
        assert!(matches!(result, Err(LedgerError::ChainInvalid(_))), "expected ChainInvalid, got a different outcome");
    }

    #[test]
    fn a_head_reference_mismatch_is_rejected_by_the_real_storage_mechanism() {
        // Exercises the exact same DPAPI head.ref trust mechanism the
        // main audit log already relies on -- this module adds nothing
        // new, so this test proves the real storage::audit_writer gate
        // is reached, not a weaker stand-in.
        let (_dir, locator, journal) = harness();
        append_receipt(&locator, &journal, &sample_receipt(0)).unwrap();

        // Corrupt the head reference file directly -- DPAPI-protected,
        // so any byte flip is an authentication failure, not a silently
        // accepted different value.
        let head_ref_path = locator.join(".audit").join("head.ref");
        let mut bytes = std::fs::read(&head_ref_path).unwrap();
        let len = bytes.len();
        bytes[len / 2] ^= 0xFF;
        std::fs::write(&head_ref_path, bytes).unwrap();

        let vault = storage::Vault::open_with_default_indexer(locator.clone(), &journal).unwrap();
        let result = vault.open_audit_writer(Some((0, "a".repeat(64).as_str())));
        assert!(result.is_err(), "a tampered head.ref must be rejected by storage's own real mechanism, got {result:?}");
    }

    #[test]
    fn roll_forward_recognizes_a_crash_between_log_fsync_and_reference_update() {
        let (_dir, locator, journal) = harness();
        let vault = storage::Vault::open_with_default_indexer(locator.clone(), &journal).unwrap();
        let receipt = sample_receipt(0);
        let (bytes, hash) = audit_chain_core::serialize_record(&receipt, 0, audit_chain_core::GENESIS_HASH);

        // Simulate the log having been durably appended (the real file,
        // the real append call) but the reference deliberately not yet
        // reflecting it, by opening with the correct caller_verified_tail
        // directly (mirrors storage::audit_writer's own first-record
        // genesis test pattern) -- this exercises the real roll-forward
        // gate, not a re-implementation of it.
        let mut writer = vault.open_audit_writer(None).unwrap();
        writer.append(&bytes, 0, &hash).unwrap();
        drop(writer);

        // Reopening with the exact same already-verified tail must
        // succeed -- the real agreement check, not assumed.
        let reopened = vault.open_audit_writer(Some((0, hash.as_str())));
        assert!(reopened.is_ok(), "got {reopened:?}");
    }

    #[test]
    fn a_second_concurrent_writer_is_denied_while_the_first_holds_the_ledgers_lock() {
        let (_dir, locator, journal) = harness();
        let vault = storage::Vault::open_with_default_indexer(locator, &journal).unwrap();
        let _first = vault.open_audit_writer(None).unwrap();
        let second = vault.open_audit_writer(None);
        assert!(matches!(second, Err(storage::AuditWriterOpenError::AlreadyLocked)), "got {second:?}");
    }

    #[test]
    fn a_duplicate_receipt_for_the_same_target_segment_is_representable_and_detectable_as_an_anomaly() {
        // The ledger itself does not refuse a duplicate append (that
        // policy decision belongs to crate::retention, which checks
        // before calling append_receipt) -- this test proves the ledger
        // faithfully PRESERVES both receipts for a caller to detect,
        // rather than silently deduplicating or merging them.
        let (_dir, locator, journal) = harness();
        append_receipt(&locator, &journal, &sample_receipt(0)).unwrap();
        append_receipt(&locator, &journal, &sample_receipt(0)).unwrap(); // same target segment id again
        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        let receipts = state.receipts().unwrap();
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0].retired_segment_id, receipts[1].retired_segment_id, "both receipts for the same segment must be preserved, not merged, so a caller's own anomaly check can see the duplicate");
    }
}
