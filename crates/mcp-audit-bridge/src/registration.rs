//! M7.3, second correction: the bridge's durable SOURCE LOG REGISTRATION
//! — `SourceLogId` plus its trusted locator and binding evidence — lives
//! in its OWN file, with its OWN lifecycle, separate from the mutable
//! progress checkpoint (`crate::checkpoint`).
//!
//! **Why these are two files, not one** (the second frozen correction):
//! the first checkpoint design put both concerns in one file, which
//! meant losing the progress pointer (`last_imported`) ALSO destroyed the
//! only record of `SourceLogId` — forcing a fresh run to mint a brand
//! new identity and re-import everything under it, producing real
//! duplicate Brain facts the destination-scan dedup could never catch
//! (different `source_log_id`, so `SourceRecordIdentity` never matches).
//! Splitting the two removes this failure mode structurally: progress
//! can be lost and rebuilt from the destination chain under the SAME
//! identity, because that identity now survives independently.
//!
//! **Registration lifecycle, explicitly**:
//! - *registration present, checkpoint lost*: fully recoverable — the
//!   identity survives, `crate::checkpoint`'s own loss is harmless (see
//!   that module's own doc comment; this is its ordinary "accelerator
//!   lost" case).
//! - *registration file corrupted (exists, fails to parse/verify)*:
//!   fail closed. Never guessed, never silently replaced.
//! - *registration file completely absent, but no checkpoint either
//!   (first-ever run)*: an ordinary new registration, minted fresh.
//! - *registration file absent, but a checkpoint OR prior destination
//!   evidence suggests this source log was registered before*: this is
//!   "registration lost," a DIFFERENT case from ordinary checkpoint
//!   loss, and is never treated as a fresh first-ever run automatically
//!   — see [`recover_from_destination_evidence`].

use serde::{Deserialize, Serialize};
use std::io::Write as _;

use crate::identity::SourceLogId;

/// This registration file's own format version — independent of, and
/// never confused with, `crate::checkpoint::BRIDGE_CHECKPOINT_FORMAT_VERSION`
/// (a deliberately separate schema with its own lifecycle, per this
/// module's own top doc comment) or any chain/event format version.
pub const SOURCE_LOG_REGISTRATION_FORMAT_VERSION: u32 = 1;

/// The bridge's durable record of one source log's identity and trusted
/// locator — see [`crate::identity::SourceLogRegistration`] for the
/// fields themselves (this module only adds the format-versioned
/// persistence envelope around that type).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationFile {
    pub format_version: u32,
    pub registration: crate::identity::SourceLogRegistration,
}

impl RegistrationFile {
    pub fn new(registration: crate::identity::SourceLogRegistration) -> Self {
        Self { format_version: SOURCE_LOG_REGISTRATION_FORMAT_VERSION, registration }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("failed to read registration file: {0}")]
    Io(std::io::Error),
    #[error("registration file content is not valid JSON: {0}")]
    MalformedJson(String),
    #[error("registration declares format_version {found}, this code only accepts {SOURCE_LOG_REGISTRATION_FORMAT_VERSION}")]
    UnsupportedFormatVersion { found: u32 },
}

/// Loads a registration from `path`, or `Ok(None)` if no file exists
/// there — the only "absence is fine" case this function itself
/// recognizes; see [`recover_from_destination_evidence`] for what a
/// caller should do with that `None` BEFORE assuming "first-ever run"
/// (it is not this function's job to make that distinction — it only
/// reports what is literally on disk).
pub fn load(path: &std::path::Path) -> Result<Option<RegistrationFile>, RegistrationError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RegistrationError::Io(e)),
    };
    let file: RegistrationFile = serde_json::from_slice(&bytes).map_err(|e| RegistrationError::MalformedJson(e.to_string()))?;
    if file.format_version != SOURCE_LOG_REGISTRATION_FORMAT_VERSION {
        return Err(RegistrationError::UnsupportedFormatVersion { found: file.format_version });
    }
    Ok(Some(file))
}

/// Persists `file` to `path` atomically — identical discipline to
/// `crate::checkpoint::save` (temp file in the same directory, `fsync`,
/// atomic rename), kept as a separate function (not shared code) only
/// because the two types differ; the persistence DISCIPLINE is
/// intentionally identical.
pub fn save(path: &std::path::Path, file: &RegistrationFile) -> Result<(), RegistrationError> {
    let bytes = serde_json::to_vec_pretty(file).expect("RegistrationFile serialization cannot fail: no non-finite floats, no non-string map keys");
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp_path = dir.join(format!(".{}.tmp-{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("registration"), uuid::Uuid::new_v4()));

    let mut tmp_file = std::fs::File::create(&tmp_path).map_err(RegistrationError::Io)?;
    tmp_file.write_all(&bytes).map_err(RegistrationError::Io)?;
    tmp_file.sync_all().map_err(RegistrationError::Io)?;
    drop(tmp_file);

    std::fs::rename(&tmp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        RegistrationError::Io(e)
    })
}

/// Why registration recovery from destination evidence was refused —
/// always fail-closed; this module never guesses among ambiguous
/// candidates and never silently treats "no evidence" as "safe to mint a
/// fresh identity" once the caller has already decided recovery (not a
/// first-ever run) is what is being attempted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistrationRecoveryError {
    /// No `SourceLogId` in the destination's own bridge-provenance
    /// evidence has a verified-prefix match against the source chain —
    /// insufficient evidence to reconstruct anything.
    #[error("no destination SourceLogId has evidence matching the verified source chain prefix -- insufficient evidence for automatic recovery")]
    NoCandidateFound,
    /// More than one distinct `SourceLogId` in the destination has at
    /// least one matching `(sequence, record_hash)` pair against the
    /// source chain — never resolved automatically; a human must
    /// disambiguate (e.g. by direct re-registration).
    #[error("multiple distinct SourceLogId candidates ({0:?}) are each compatible with the verified source chain -- refusing to pick one automatically")]
    AmbiguousCandidates(Vec<SourceLogId>),
}

/// Attempts to reconstruct a lost [`crate::identity::SourceLogRegistration`]
/// purely from evidence already durably present in the destination
/// chain's own bridge-provenance fields (`source_log_id`/`source_sequence`/
/// `source_record_hash` on `McpBridgeEvent`/`McpBridgeExternalSideEffectUnknown`),
/// cross-checked against `source_verified_records` (the CURRENT, already
/// chain-verified source records).
///
/// Succeeds with `Ok(Some(registration))` ONLY when exactly one distinct
/// `SourceLogId` in the destination has at least one `(sequence,
/// record_hash)` pair that also appears, verified, in
/// `source_verified_records` — the unique-candidate requirement the
/// frozen correction demands. Returns `Ok(None)` when the destination has
/// no bridge-provenance evidence at all (nothing to recover from — a
/// caller distinguishes this from "first-ever run" using its own
/// knowledge of whether a checkpoint/other signal suggested this source
/// was registered before; this function itself makes no such claim).
/// Returns `Err` for zero-matching-but-nonempty-evidence or ambiguous
/// multi-candidate cases — both fail closed, never resolved by guessing.
pub fn recover_from_destination_evidence(destination_verified_records: &[audit_chain_core::ChainRecord], source_verified_records: &[audit_chain_core::ChainRecord], current_locator: &std::path::Path) -> Result<Option<crate::identity::SourceLogRegistration>, RegistrationRecoveryError> {
    use std::collections::{HashMap, HashSet};

    let source_pairs: HashSet<(u64, &str)> = source_verified_records.iter().map(|r| (r.sequence, r.record_hash.as_str())).collect();

    // For every distinct SourceLogId mentioned in the destination, the
    // full set of (sequence, record_hash) pairs it claims -- and
    // separately, the LATEST (max sequence) verified tail it claims, so
    // a successful recovery can seed `last_known_tail` correctly rather
    // than leaving it `None` (which would incorrectly permit ANY future
    // chain as a "first ever" relocation target instead of requiring
    // compatibility with what is actually already imported).
    let mut claimed_pairs: HashMap<SourceLogId, HashSet<(u64, String)>> = HashMap::new();
    for record in destination_verified_records {
        let Ok(event) = audit_chain_core::parse_event::<brain_api::AuditEvent>(&record.content_bytes) else {
            continue;
        };
        if let Some((id, seq, hash)) = bridge_provenance(&event.kind) {
            claimed_pairs.entry(id).or_default().insert((seq, hash));
        }
    }

    let mut matching_candidates: Vec<SourceLogId> = Vec::new();
    for (id, pairs) in &claimed_pairs {
        let has_verified_match = pairs.iter().any(|(seq, hash)| source_pairs.contains(&(*seq, hash.as_str())));
        if has_verified_match {
            matching_candidates.push(*id);
        }
    }

    if claimed_pairs.is_empty() {
        return Ok(None);
    }
    match matching_candidates.len() {
        0 => Err(RegistrationRecoveryError::NoCandidateFound),
        1 => {
            let id = matching_candidates[0];
            let max_tail = claimed_pairs[&id].iter().max_by_key(|(seq, _)| *seq).cloned();
            Ok(Some(crate::identity::SourceLogRegistration { source_log_id: id, current_locator: current_locator.to_path_buf(), last_known_tail: max_tail }))
        }
        _ => Err(RegistrationRecoveryError::AmbiguousCandidates(matching_candidates)),
    }
}

fn bridge_provenance(kind: &brain_api::AuditEventKind) -> Option<(SourceLogId, u64, String)> {
    use brain_api::AuditEventKind as K;
    match kind {
        K::McpBridgeEvent { source_log_id, source_sequence, source_record_hash, .. } | K::McpBridgeExternalSideEffectUnknown { source_log_id, source_sequence, source_record_hash, .. } => uuid::Uuid::parse_str(source_log_id).ok().map(|u| (SourceLogId(u), *source_sequence, source_record_hash.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::SourceLogRegistration;
    use audit_chain_core::{compute_record_hash, serialize_record, ChainRecord, GENESIS_HASH};

    fn sample_registration() -> SourceLogRegistration {
        SourceLogRegistration { source_log_id: SourceLogId::generate(), current_locator: "some/path".into(), last_known_tail: Some((3, "h".repeat(64))) }
    }

    // --- persistence ---

    #[test]
    fn loading_a_path_with_no_file_yet_returns_none_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load(&dir.path().join("reg.json")), Ok(None)));
    }

    #[test]
    fn a_saved_registration_loads_back_byte_for_byte_equal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reg.json");
        let file = RegistrationFile::new(sample_registration());
        save(&path, &file).unwrap();
        assert_eq!(load(&path).unwrap().unwrap(), file);
    }

    #[test]
    fn a_malformed_registration_file_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reg.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(matches!(load(&path), Err(RegistrationError::MalformedJson(_))));
    }

    #[test]
    fn an_unsupported_format_version_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reg.json");
        let mut value = serde_json::to_value(RegistrationFile::new(sample_registration())).unwrap();
        value["format_version"] = serde_json::json!(999);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(load(&path), Err(RegistrationError::UnsupportedFormatVersion { found: 999 })));
    }

    // --- recovery from destination evidence ---

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

    fn destination_record_for(kind: brain_api::AuditEventKind, sequence: u64, prev_hash: &str) -> ChainRecord {
        let event = brain_api::event(kind, "2026-01-01T00:00:00Z", uuid::Uuid::new_v4());
        let (_, record_hash) = serialize_record(&event, sequence, prev_hash);
        let content_bytes = serde_json::to_vec(&event).unwrap();
        ChainRecord { sequence, prev_record_hash: prev_hash.to_string(), record_hash, content_bytes }
    }

    fn bridge_kind(log_id: SourceLogId, sequence: u64, record_hash: &str) -> brain_api::AuditEventKind {
        brain_api::AuditEventKind::McpBridgeEvent {
            source_phase: brain_api::McpBridgePhase::Invocation,
            source_kind: "invocation_attempted".into(),
            server: Some("s".into()),
            tool_name: Some("t".into()),
            invocation_id: Some("i1".into()),
            outcome: None,
            source_log_id: log_id.to_string(),
            source_sequence: sequence,
            source_record_hash: record_hash.to_string(),
        }
    }

    #[test]
    fn empty_destination_evidence_returns_none_not_an_error() {
        let source = chain(3);
        let result = recover_from_destination_evidence(&[], &source, std::path::Path::new("p"));
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn a_unique_matching_candidate_is_recovered_with_its_last_known_tail_seeded() {
        let source = chain(3);
        let log_id = SourceLogId::generate();
        let dest = vec![destination_record_for(bridge_kind(log_id, source[0].sequence, &source[0].record_hash), 0, GENESIS_HASH)];
        let result = recover_from_destination_evidence(&dest, &source, std::path::Path::new("p")).unwrap().unwrap();
        assert_eq!(result.source_log_id, log_id);
        assert_eq!(result.last_known_tail, Some((source[0].sequence, source[0].record_hash.clone())));
    }

    #[test]
    fn a_candidate_with_no_verified_match_against_the_current_source_chain_fails_closed() {
        let source = chain(3);
        let log_id = SourceLogId::generate();
        // Claims a (sequence, hash) pair that does NOT appear in the
        // current source chain at all.
        let dest = vec![destination_record_for(bridge_kind(log_id, 99, &"z".repeat(64)), 0, GENESIS_HASH)];
        let result = recover_from_destination_evidence(&dest, &source, std::path::Path::new("p"));
        assert!(matches!(result, Err(RegistrationRecoveryError::NoCandidateFound)));
    }

    #[test]
    fn two_distinct_candidates_each_compatible_with_the_source_chain_fail_closed_as_ambiguous() {
        let source = chain(3);
        let log_id_a = SourceLogId::generate();
        let log_id_b = SourceLogId::generate();
        let mut prev = GENESIS_HASH.to_string();
        let rec_a = destination_record_for(bridge_kind(log_id_a, source[0].sequence, &source[0].record_hash), 0, &prev);
        prev = rec_a.record_hash.clone();
        let rec_b = destination_record_for(bridge_kind(log_id_b, source[1].sequence, &source[1].record_hash), 1, &prev);
        let result = recover_from_destination_evidence(&[rec_a, rec_b], &source, std::path::Path::new("p"));
        assert!(matches!(result, Err(RegistrationRecoveryError::AmbiguousCandidates(_))), "got {result:?}");
    }
}
