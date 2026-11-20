//! M6.3 Phase 3 item 9: recovery behavior compatible with the storage
//! writer -- reuses `audit_chain_core::scan_chain`/`may_resume_without_recovery`
//! exactly as `brain-api-audit-worker`'s own `recovery.rs` does (same
//! pattern, proven there), adapted to this crate's own event schema and
//! its own "no storage dependency" constraint: this module takes already
//! -read raw log bytes (the composition root's own job to supply, via
//! its real `storage::Vault::read_audit_log_for_recovery_scan`-shaped
//! call) rather than a `storage::Vault` itself.

use crate::error::RecoveryError;

/// What startup recovery found, in the exact shape `spawn_worker` needs
/// to compute its own `next_sequence`/`prev_hash` before opening the
/// writer -- mirrors `brain_api_audit_worker`'s own `RecoveredTail`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecoveredTail {
    pub(crate) sequence: Option<u64>,
    pub(crate) record_hash: Option<String>,
}

/// Splits raw, already-read log bytes into `audit_chain_core::LineOutcome`s
/// the exact way `brain-api-audit-worker`'s own `split_into_line_outcomes`
/// does: newline-delimited, with `is_physical_tail: true` set ONLY on the
/// final line when the raw bytes do NOT end with a trailing newline (a
/// genuinely truncated last line) -- never on any earlier line, and
/// never on the final line when the raw bytes DO end cleanly with `\n`
/// (a complete log has no "tail" to recover from at all).
fn split_into_line_outcomes(raw: &[u8]) -> Vec<audit_chain_core::LineOutcome> {
    if raw.is_empty() {
        return Vec::new();
    }
    let ends_with_newline = raw.last() == Some(&b'\n');
    let trimmed = if ends_with_newline { &raw[..raw.len() - 1] } else { raw };
    let raw_lines: Vec<&[u8]> = trimmed.split(|&b| b == b'\n').collect();
    let last_index = raw_lines.len().saturating_sub(1);
    raw_lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| match audit_chain_core::parse_line(line) {
            Ok(record) => audit_chain_core::LineOutcome::Parsed(record),
            Err(_) => audit_chain_core::LineOutcome::Unparseable { is_physical_tail: !ends_with_newline && index == last_index },
        })
        .collect()
}

/// Runs the full startup recovery scan over `raw` (the composition
/// root's own already-read log bytes) -- mirrors
/// `brain_api_audit_worker::recovery::recover_tail`'s own exact
/// contract: `Ok` means immediate resumption is safe; `Err` carries both
/// the verified prefix (for diagnostics) and the specific
/// [`RecoveryError`] that blocks resumption.
pub(crate) fn recover_tail(raw: &[u8]) -> Result<RecoveredTail, (RecoveredTail, RecoveryError)> {
    if raw.is_empty() {
        return Ok(RecoveredTail { sequence: None, record_hash: None });
    }

    let lines = split_into_line_outcomes(raw);
    let scan_result = audit_chain_core::scan_chain(&lines);

    let recovered = match &scan_result.last_verified {
        Some((seq, hash)) => RecoveredTail { sequence: Some(*seq), record_hash: Some(hash.clone()) },
        None => RecoveredTail { sequence: None, record_hash: None },
    };

    if audit_chain_core::may_resume_without_recovery(&scan_result) {
        return Ok(recovered);
    }

    let error = match scan_result.stop_reason {
        Some(audit_chain_core::ScanStopReason::PartialTrailingRecord) => RecoveryError::PartialTrailingRecord,
        Some(audit_chain_core::ScanStopReason::UnparseableNotAtTail) => RecoveryError::UnparseableNotAtTail,
        Some(audit_chain_core::ScanStopReason::LinkBroken { .. }) => RecoveryError::LinkBroken,
        None => unreachable!("may_resume_without_recovery returned false but stop_reason is None -- scan_chain's own documented invariant guarantees these two always agree"),
    };
    Err((recovered, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{event, McpClientEventKind};
    use audit_chain_core::{serialize_record, GENESIS_HASH};

    fn sample_event(n: u64) -> crate::event::McpClientEvent {
        event(McpClientEventKind::DiscoveryAttempted { server: format!("s{n}") }, "2026-01-01T00:00:00Z", "corr-1")
    }

    #[test]
    fn an_empty_log_recovers_as_no_tail_at_all() {
        let recovered = recover_tail(&[]).expect("empty log must permit immediate resumption");
        assert_eq!(recovered.sequence, None);
        assert_eq!(recovered.record_hash, None);
    }

    #[test]
    fn a_valid_chain_recovers_the_exact_last_verified_tail() {
        let e0 = sample_event(0);
        let (line0, hash0) = serialize_record(&e0, 0, GENESIS_HASH);
        let e1 = sample_event(1);
        let (line1, hash1) = serialize_record(&e1, 1, &hash0);

        let mut raw = line0;
        raw.extend_from_slice(&line1);

        let recovered = recover_tail(&raw).expect("a fully valid chain must permit immediate resumption");
        assert_eq!(recovered.sequence, Some(1));
        assert_eq!(recovered.record_hash, Some(hash1.clone()));
    }

    #[test]
    fn a_partial_trailing_record_is_classified_as_recovery_required() {
        let e0 = sample_event(0);
        let (line0, _) = serialize_record(&e0, 0, GENESIS_HASH);
        let mut raw = line0;
        raw.extend_from_slice(b"{\"schema_version\":1,\"sequence\":1,\"prev_rec"); // truncated, no trailing newline

        let (recovered, error) = recover_tail(&raw).expect_err("a truncated trailing record must require recovery");
        assert_eq!(recovered.sequence, Some(0));
        assert_eq!(error, RecoveryError::PartialTrailingRecord);
    }

    #[test]
    fn corruption_not_at_the_tail_is_classified_as_unparseable_not_at_tail() {
        let e0 = sample_event(0);
        let (line0, hash0) = serialize_record(&e0, 0, GENESIS_HASH);
        let e2 = sample_event(2);
        let (line2, _) = serialize_record(&e2, 2, &hash0); // sequence 2, not 1 -- but still well-formed JSON

        let mut raw = line0;
        raw.extend_from_slice(b"garbage-not-json\n");
        raw.extend_from_slice(&line2);

        let (recovered, error) = recover_tail(&raw).expect_err("mid-file corruption must require recovery");
        assert_eq!(recovered.sequence, Some(0));
        assert_eq!(error, RecoveryError::UnparseableNotAtTail);
    }

    #[test]
    fn a_broken_chain_link_is_classified_as_link_broken() {
        let e0 = sample_event(0);
        let (line0, _) = serialize_record(&e0, 0, GENESIS_HASH);
        let e1 = sample_event(1);
        let (line1, _) = serialize_record(&e1, 1, "wrong-prev-hash-entirely-0000000000000000000000000000000000000000");

        let mut raw = line0;
        raw.extend_from_slice(&line1);

        let (recovered, error) = recover_tail(&raw).expect_err("a broken link must require recovery");
        assert_eq!(recovered.sequence, Some(0));
        assert_eq!(error, RecoveryError::LinkBroken);
    }
}
