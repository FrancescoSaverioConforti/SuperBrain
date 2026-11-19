//! Startup recovery: turns the existing audit log's raw bytes into the
//! chain-tail state (`caller_verified_tail`) `storage::Vault::
//! open_audit_writer` requires, by splitting the raw bytes into lines
//! and running `brain_api::audit_record::parse_line`-shaped parsing
//! followed by `brain_api::scan_chain` — never reimplementing chain
//! verification itself.

use crate::error::RecoveryError;

/// The chain state recovered from an existing log, or `None` for a
/// genuinely empty/fresh log — this is exactly the shape
/// `storage::Vault::open_audit_writer`'s own `caller_verified_tail`
/// parameter expects.
#[derive(Debug)]
pub(crate) struct RecoveredTail {
    pub sequence: Option<u64>,
    pub record_hash: Option<String>,
}

impl RecoveredTail {
    /// Borrows out the `Option<(u64, &str)>` shape
    /// `Vault::open_audit_writer` takes, without this crate needing to
    /// re-derive that shape at every call site.
    pub(crate) fn as_caller_verified_tail(&self) -> Option<(u64, &str)> {
        match (&self.sequence, &self.record_hash) {
            (Some(seq), Some(hash)) => Some((*seq, hash.as_str())),
            _ => None,
        }
    }
}

/// Reads `vault`'s raw audit log bytes and classifies them into the
/// recovered tail, or a [`RecoveryError`] if the existing content is not
/// in a state this worker may safely resume appending to. Pure
/// classification only — never truncates, repairs, or writes anything.
pub(crate) fn recover_tail(vault: &storage::Vault) -> Result<RecoveredTail, (RecoveredTail, RecoveryError)> {
    let raw = vault.read_audit_log_for_recovery_scan().expect("recovery-scan read is only ever called before any writer lock is held by this worker; a filesystem failure here is an environment problem, not a recoverable chain state -- surfaced as a panic matching storage's own 'never silently repair' discipline rather than a RecoveryError this module's own typed set does not model I/O failures under");

    if raw.is_empty() {
        return Ok(RecoveredTail { sequence: None, record_hash: None });
    }

    let lines = split_into_line_outcomes(&raw);
    let scan_result = brain_api::scan_chain(&lines);

    let recovered = RecoveredTail {
        sequence: scan_result.last_verified.as_ref().map(|(seq, _)| *seq),
        record_hash: scan_result.last_verified.as_ref().map(|(_, hash)| hash.clone()),
    };

    if brain_api::may_resume_without_recovery(&scan_result) {
        return Ok(recovered);
    }

    let error = match scan_result.stop_reason {
        Some(brain_api::ScanStopReason::PartialTrailingRecord) => RecoveryError::PartialTrailingRecord,
        Some(brain_api::ScanStopReason::UnparseableNotAtTail) => RecoveryError::UnparseableNotAtTail,
        Some(brain_api::ScanStopReason::LinkBroken { .. }) => RecoveryError::LinkBroken,
        None => unreachable!("may_resume_without_recovery returned false but stop_reason is None -- scan_chain's own documented invariant guarantees these two always agree"),
    };
    Err((recovered, error))
}

/// Splits raw log bytes into newline-delimited lines and classifies each
/// as `Parsed`/`Unparseable` via `brain_api::parse_line`
/// (`audit_record`'s own facade), marking the final line
/// `is_physical_tail: true` only when the raw bytes end WITHOUT a
/// trailing newline (a complete, newline-terminated final line is NOT a
/// partial tail, regardless of its own content) -- a raw byte sequence
/// ending exactly at a newline has no "physical tail" fragment left over
/// at all.
fn split_into_line_outcomes(raw: &[u8]) -> Vec<brain_api::LineOutcome> {
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
        .map(|(index, line)| match brain_api::parse_line(line) {
            Ok(record) => brain_api::LineOutcome::Parsed(record),
            Err(_) => {
                let is_physical_tail = !ends_with_newline && index == last_index;
                brain_api::LineOutcome::Unparseable { is_physical_tail }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event(n: u64) -> brain_api::AuditEvent {
        brain_api::event(
            brain_api::AuditEventKind::WriteMemory { proposal_id: format!("p{n}") },
            "2026-11-19T00:00:00Z",
            uuid::Uuid::new_v4(),
        )
    }

    fn write_lines(dir: &std::path::Path, lines: &[u8]) -> storage::Vault {
        let vault = storage::Vault::open_with_default_indexer(dir.to_path_buf(), &dir.join("journal.sqlite")).unwrap();
        let audit_dir = dir.join(".audit");
        std::fs::create_dir_all(&audit_dir).unwrap();
        std::fs::write(audit_dir.join("audit.log"), lines).unwrap();
        vault
    }

    fn unique_test_dir(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-audit-worker-recovery-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_empty_log_recovers_as_no_tail_at_all() {
        let dir = unique_test_dir("empty");
        let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
        let recovered = recover_tail(&vault).unwrap();
        assert_eq!(recovered.as_caller_verified_tail(), None);
    }

    #[test]
    fn a_valid_chain_recovers_the_exact_last_verified_tail() {
        let dir = unique_test_dir("valid-chain");
        let e0 = sample_event(0);
        let (line0, hash0) = brain_api::serialize_record(&e0, 0, brain_api::GENESIS_HASH);
        let e1 = sample_event(1);
        let (line1, hash1) = brain_api::serialize_record(&e1, 1, &hash0);
        let mut bytes = line0;
        bytes.extend_from_slice(&line1);
        let vault = write_lines(&dir, &bytes);

        let recovered = recover_tail(&vault).unwrap();
        assert_eq!(recovered.as_caller_verified_tail(), Some((1, hash1.as_str())));
    }

    #[test]
    fn a_partial_trailing_record_is_classified_as_recovery_required() {
        let dir = unique_test_dir("partial-tail");
        let e0 = sample_event(0);
        let (line0, _) = brain_api::serialize_record(&e0, 0, brain_api::GENESIS_HASH);
        let mut bytes = line0;
        bytes.extend_from_slice(b"{\"format_version\":1,\"sequence\":1,\"trunc"); // no trailing newline
        let vault = write_lines(&dir, &bytes);

        let result = recover_tail(&vault);
        assert!(matches!(result, Err((_, RecoveryError::PartialTrailingRecord))), "got {:?}", result.err().map(|(_, e)| e));
    }

    #[test]
    fn corruption_not_at_the_tail_is_classified_as_unparseable_not_at_tail() {
        let dir = unique_test_dir("corrupt-middle");
        let e0 = sample_event(0);
        let (line0, hash0) = brain_api::serialize_record(&e0, 0, brain_api::GENESIS_HASH);
        let e2 = sample_event(2);
        let (line2, _) = brain_api::serialize_record(&e2, 2, &hash0);
        let mut bytes = line0;
        bytes.extend_from_slice(b"not valid json at all\n");
        bytes.extend_from_slice(&line2);
        let vault = write_lines(&dir, &bytes);

        let result = recover_tail(&vault);
        assert!(matches!(result, Err((_, RecoveryError::UnparseableNotAtTail))), "got {:?}", result.err().map(|(_, e)| e));
    }

    #[test]
    fn a_broken_chain_link_is_classified_as_link_broken() {
        let dir = unique_test_dir("link-broken");
        let e0 = sample_event(0);
        let (line0, _correct_hash0) = brain_api::serialize_record(&e0, 0, brain_api::GENESIS_HASH);
        let e1 = sample_event(1);
        // Deliberately link against the WRONG prev_hash.
        let (line1, _) = brain_api::serialize_record(&e1, 1, "wrong-prev-hash");
        let mut bytes = line0;
        bytes.extend_from_slice(&line1);
        let vault = write_lines(&dir, &bytes);

        let result = recover_tail(&vault);
        assert!(matches!(result, Err((_, RecoveryError::LinkBroken))), "got {:?}", result.err().map(|(_, e)| e));
    }
}
