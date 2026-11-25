//! M7: durable identity for one MCP source log, and for one record
//! inside it -- the two concepts the bridge's idempotency depends on,
//! neither of which `storage::Vault` or `audit_chain_core::ChainRecord`
//! owns on their own.
//!
//! **Why `SourceLogId` is bridge-owned, not storage-owned** (per the
//! frozen M7 decision): a filesystem path is a locator, never a trust
//! anchor -- it can be moved, and a brand-new log can later be created at
//! the exact same path. `storage::Vault` has, and keeps, no concept of a
//! stable cross-relocation identity for an audit log; adding one there
//! solely to serve this crate would reach into a crate this bridge must
//! only ever read from (M5-reviewed `storage::audit_writer` stays
//! untouched). So this identity is minted and owned entirely by this
//! crate, the one place that needs it.
//!
//! **Why relocation is explicit, never auto-discovered** (the frozen M7
//! correction): this module performs no filesystem search for "a log
//! that might have moved." A caller (the composition root operating this
//! bridge) states the new locator explicitly; this module's own
//! [`verify_relocation`] then either confirms the new locator's chain is
//! compatible with the last known tail (the only basis on which a
//! locator update is ever accepted) or refuses -- fail closed, never a
//! heuristic guess.

use serde::{Deserialize, Serialize};

/// Durable identity of ONE MCP source log, from the bridge's own point of
/// view -- minted once, at first registration, never derived from a path
/// or from the log's own content. A `Copy` newtype over a `Uuid`: cheap
/// to pass around, compared only by value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceLogId(pub uuid::Uuid);

impl SourceLogId {
    /// Mints a fresh identity -- called exactly once per real source log,
    /// the first time the bridge is ever pointed at it. Never called
    /// again for that same log afterward; every later run loads the
    /// existing identity from the persisted checkpoint instead.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl std::fmt::Display for SourceLogId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Durable identity of ONE record inside a source log's chain -- the
/// bridge's idempotency key. Never derived from `invocation_id`/
/// `correlation_id` (absent for many source variants, and never a stand-in
/// for "this exact persisted record"); always derived from the record's
/// own verified position and hash in its chain, combined with the bridge-
/// owned identity of the log that contains it.
///
/// Equality and `Hash` are field-wise (derived) -- two identities are
/// equal only when `source_log_id`, `sequence`, AND `record_hash` all
/// match; there is no partial-equality notion anywhere in this crate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceRecordIdentity {
    pub source_log_id: SourceLogId,
    /// The record's own verified `audit_chain_core::ChainRecord::sequence`.
    pub sequence: u64,
    /// The record's own verified `audit_chain_core::ChainRecord::record_hash`
    /// (`sha256(prev_record_hash || sequence || content_bytes)`).
    pub record_hash: String,
}

/// The bridge's own durable record of one source log it has registered --
/// persisted via `crate::registration`, in its own file, deliberately
/// separate from the mutable progress checkpoint (`crate::checkpoint`) so
/// losing the checkpoint can never destroy this identity (see
/// `crate::registration`'s own top doc comment for why these are split).
/// `current_locator` is a locator (where to look today), never used as an
/// identity comparison; `last_known_tail` is the last verified
/// `(sequence, record_hash)` this bridge has itself observed for this log,
/// or `None` if it has never yet seen a non-empty log at this identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLogRegistration {
    pub source_log_id: SourceLogId,
    pub current_locator: std::path::PathBuf,
    pub last_known_tail: Option<(u64, String)>,
}

/// Why a candidate locator was refused for an existing [`SourceLogRegistration`]
/// -- always fail-closed; this module itself never attempts to reconcile
/// or auto-repair any of these, by design.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RelocationError {
    /// `last_known_tail` is `Some`, but the candidate locator's own
    /// verified chain does not contain that exact `(sequence,
    /// record_hash)` pair anywhere on its verified prefix -- this is NOT
    /// treated as "probably a new log, rebind anyway": the caller must
    /// register a genuinely new [`SourceLogId`] explicitly if that is
    /// really what is intended.
    #[error("candidate locator's chain does not contain the last known tail (sequence={expected_sequence}, record_hash={expected_record_hash}) -- refusing to treat it as the same source log")]
    TailNotFound { expected_sequence: u64, expected_record_hash: String },
    /// The candidate locator's own chain failed verification outright
    /// (broken link, tampering, or a malformed trailing record) --
    /// refusing before even comparing tails.
    #[error("candidate locator's own chain failed verification: {0:?}")]
    CandidateChainInvalid(audit_chain_core::ScanStopReason),
}

/// Verifies that `candidate_verified_records` (the full, already-chain-
/// verified prefix read from a NEW locator, via `audit_chain_core::
/// scan_chain`) is compatible with `registration.last_known_tail` --  the
/// one and only basis on which this module allows a locator update.
///
/// - `last_known_tail` is `None` (first-ever registration of this
///   `SourceLogId`, nothing observed yet): any chain-valid candidate is
///   accepted unconditionally -- there is nothing yet to be compatible
///   WITH.
/// - `last_known_tail` is `Some((seq, hash))`: the candidate's own
///   verified records must contain a record at exactly that `sequence`
///   whose `record_hash` matches exactly -- this is what distinguishes
///   "the same log, relocated" from "a different log that happens to
///   share a path" (the two cases the frozen M7 correction names
///   explicitly). A mismatch or absence is always [`RelocationError`],
///   never silently accepted and never auto-repaired by this function.
///
/// This function performs no I/O itself and makes no decision about
/// WHETHER to relocate -- that decision belongs to the bridge's own
/// composition root, which supplies the explicit candidate locator in the
/// first place (never discovered automatically by this module).
pub fn verify_relocation(
    registration: &SourceLogRegistration,
    candidate_scan: &audit_chain_core::ScanResult,
) -> Result<(), RelocationError> {
    if let Some(stop_reason) = &candidate_scan.stop_reason {
        if !audit_chain_core::may_resume_without_recovery(candidate_scan) {
            return Err(RelocationError::CandidateChainInvalid(stop_reason.clone()));
        }
    }

    match &registration.last_known_tail {
        None => Ok(()),
        Some((expected_sequence, expected_record_hash)) => {
            let found = candidate_scan.verified_records.iter().any(|record| record.sequence == *expected_sequence && record.record_hash == *expected_record_hash);
            if found {
                Ok(())
            } else {
                Err(RelocationError::TailNotFound { expected_sequence: *expected_sequence, expected_record_hash: expected_record_hash.clone() })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_chain_core::{compute_record_hash, ChainRecord, LineOutcome, GENESIS_HASH};

    fn chain(n: usize) -> Vec<ChainRecord> {
        let mut records = Vec::with_capacity(n);
        let mut prev = GENESIS_HASH.to_string();
        for i in 0..n {
            let content = format!("event-{i}");
            let hash = compute_record_hash(&prev, i as u64, content.as_bytes());
            records.push(ChainRecord { sequence: i as u64, prev_record_hash: prev.clone(), record_hash: hash.clone(), content_bytes: content.into_bytes() });
            prev = hash;
        }
        records
    }

    fn scan(records: &[ChainRecord]) -> audit_chain_core::ScanResult {
        let lines: Vec<LineOutcome> = records.iter().cloned().map(LineOutcome::Parsed).collect();
        audit_chain_core::scan_chain(&lines)
    }

    #[test]
    fn two_source_log_ids_minted_separately_are_never_equal() {
        assert_ne!(SourceLogId::generate(), SourceLogId::generate());
    }

    #[test]
    fn source_record_identity_equality_requires_all_three_fields_to_match() {
        let id = SourceLogId::generate();
        let a = SourceRecordIdentity { source_log_id: id, sequence: 0, record_hash: "h".into() };
        let b = SourceRecordIdentity { source_log_id: id, sequence: 0, record_hash: "h".into() };
        assert_eq!(a, b);

        let different_sequence = SourceRecordIdentity { source_log_id: id, sequence: 1, record_hash: "h".into() };
        assert_ne!(a, different_sequence);

        let different_hash = SourceRecordIdentity { source_log_id: id, sequence: 0, record_hash: "other".into() };
        assert_ne!(a, different_hash);

        let different_log = SourceRecordIdentity { source_log_id: SourceLogId::generate(), sequence: 0, record_hash: "h".into() };
        assert_ne!(a, different_log);
    }

    #[test]
    fn first_ever_registration_accepts_any_chain_valid_candidate() {
        let registration = SourceLogRegistration { source_log_id: SourceLogId::generate(), current_locator: "irrelevant".into(), last_known_tail: None };
        let candidate = scan(&chain(3));
        assert!(verify_relocation(&registration, &candidate).is_ok());
    }

    #[test]
    fn relocation_to_a_locator_whose_chain_still_contains_the_last_known_tail_is_accepted() {
        let records = chain(5);
        let last = &records[2];
        let registration = SourceLogRegistration {
            source_log_id: SourceLogId::generate(),
            current_locator: "old/path".into(),
            last_known_tail: Some((last.sequence, last.record_hash.clone())),
        };
        // Candidate locator has the SAME prefix plus more records appended
        // since -- the realistic "moved, and kept growing" case.
        let candidate = scan(&chain(5));
        assert!(verify_relocation(&registration, &candidate).is_ok());
    }

    #[test]
    fn same_pathname_with_an_incompatible_new_chain_is_refused_not_auto_rebound() {
        // A brand-new log created at the same path: starts from genesis
        // again, with different content, so it does NOT contain the
        // previously observed tail anywhere.
        let old_records = chain(5);
        let last = &old_records[4];
        let registration = SourceLogRegistration {
            source_log_id: SourceLogId::generate(),
            current_locator: "same/path".into(),
            last_known_tail: Some((last.sequence, last.record_hash.clone())),
        };
        let fresh_log_at_same_path = scan(&chain(2)); // shorter, different content at every sequence
        let result = verify_relocation(&registration, &fresh_log_at_same_path);
        assert!(matches!(result, Err(RelocationError::TailNotFound { .. })), "got {result:?}");
    }

    #[test]
    fn a_candidate_with_a_broken_chain_is_refused_before_any_tail_comparison() {
        let mut records = chain(5);
        records[2].content_bytes = b"TAMPERED".to_vec();
        let registration = SourceLogRegistration { source_log_id: SourceLogId::generate(), current_locator: "p".into(), last_known_tail: None };
        let candidate = scan(&records);
        let result = verify_relocation(&registration, &candidate);
        assert!(matches!(result, Err(RelocationError::CandidateChainInvalid(_))), "got {result:?}");
    }

    #[test]
    fn relocation_to_a_shorter_chain_missing_the_tail_is_refused() {
        let records = chain(5);
        let last = &records[4];
        let registration = SourceLogRegistration {
            source_log_id: SourceLogId::generate(),
            current_locator: "p".into(),
            last_known_tail: Some((last.sequence, last.record_hash.clone())),
        };
        let shorter = scan(&chain(3)); // never reaches sequence 4 at all
        let result = verify_relocation(&registration, &shorter);
        assert!(matches!(result, Err(RelocationError::TailNotFound { .. })), "got {result:?}");
    }
}
