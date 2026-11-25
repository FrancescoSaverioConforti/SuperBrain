//! M7.7.3: the generic durable retention-pin store — a second,
//! dedicated `storage::Vault`, structurally identical to
//! `crate::ledger`'s own retention ledger (same reuse of
//! `Vault::open_audit_writer`/`AuditWriterHandle::append`/
//! `read_audit_log_for_recovery_scan`, never a raw writer).
//!
//! **Generic, never MCP-aware** (the frozen M7.7 rule): a pin's
//! `owner_class` is a bounded, content-free [`crate::domain::OwnerClass`]
//! identifier — this module never interprets what any particular value
//! means, only that it is well-formed. `mcp-audit-bridge` (via the
//! sibling `mcp-audit-segmented` composition crate) is one possible
//! owner among any number of future ones; this module knows nothing
//! about MCP, bridges, or audit import.
//!
//! **A pin's Active/Released state is DERIVED, never a mutable flag**
//! (the same "the chain is the authority" discipline already applied
//! to every other durable fact in this crate): a pin is Active if and
//! only if a `PinAcquired` with that `pin_id` exists in the verified
//! chain and no `PinReleased` for the SAME `pin_id` also exists.
//!
//! **Why a single mixed-payload chain, not two separate stores**: one
//! append-only chain carrying both `PinAcquired` and `PinReleased`
//! events preserves their relative ORDER (acquire-before-release is
//! itself meaningful and auditable) — the same reason a single audit
//! chain, not two, carries every event kind for one segment.
//!
//! **Trusted-root derivation** (the frozen M7.7 correction): this
//! module's own functions take a `locator` parameter like
//! `crate::ledger` does, but — unlike the retention ledger, whose
//! locator the manifest may legitimately describe (subject to a
//! caller's own comparison against a trusted root) — nothing in this
//! crate ever reads a pin-store locator FROM the manifest at all. The
//! composition layer (`mcp-audit-segmented`) is responsible for always
//! deriving this locator from its own trusted root, never from any
//! manifest field; this module itself is locator-agnostic and trusts
//! whatever path it is given exactly as much as `crate::ledger` does
//! (full chain + head.ref verification, never the path string alone).

use audit_chain_core::ChainRecord;

use crate::domain::{OwnerClass, PinAcquired, PinReason, PinReleased, SegmentId};

#[derive(Debug, thiserror::Error)]
pub enum PinStoreError {
    #[error("pin store vault error: {0:?}")]
    Vault(storage::StorageError),
    #[error("pin store chain failed verification: {0:?}")]
    ChainInvalid(audit_chain_core::ScanStopReason),
    #[error("failed to open the pin store's durable writer: {0:?}")]
    WriterOpenFailed(storage::AuditWriterOpenError),
    #[error("pin store append failed: {0:?}")]
    AppendFailed(storage::AuditAppendError),
    #[error("a record in the pin store's own verified chain failed to decode as either PinAcquired or PinReleased: {0}")]
    MalformedRecord(String),
    #[error("duplicate/replayed PinAcquired: pin_id {0} already exists in the verified chain -- never re-appended, never silently treated as a fresh acquisition")]
    DuplicatePinAcquired(uuid::Uuid),
    #[error("PinReleased for pin_id {0} does not correlate to any PinAcquired in the verified chain")]
    ReleaseWithoutAcquire(uuid::Uuid),
}

/// One decoded record from the pin store's own mixed-payload chain.
#[derive(Debug, Clone, PartialEq)]
pub enum PinStoreRecord {
    Acquired(PinAcquired),
    Released(PinReleased),
}

pub struct VerifiedPinStoreState {
    pub verified_records: Vec<ChainRecord>,
    pub last_verified: Option<(u64, String)>,
}

impl VerifiedPinStoreState {
    /// Every record in the pin store's own verified prefix, decoded as
    /// either `PinAcquired` or `PinReleased` — tries `PinAcquired`
    /// first (arbitrary but fixed order; the two types' own JSON shapes
    /// are disjoint enough in practice that this never matters, but the
    /// function never silently accepts a record that decodes as
    /// neither).
    pub fn records(&self) -> Result<Vec<PinStoreRecord>, PinStoreError> {
        self.verified_records
            .iter()
            .map(|r| {
                if let Ok(acquired) = audit_chain_core::parse_event::<PinAcquired>(&r.content_bytes) {
                    Ok(PinStoreRecord::Acquired(acquired))
                } else if let Ok(released) = audit_chain_core::parse_event::<PinReleased>(&r.content_bytes) {
                    Ok(PinStoreRecord::Released(released))
                } else {
                    Err(PinStoreError::MalformedRecord(format!("record at sequence {} decodes as neither PinAcquired nor PinReleased", r.sequence)))
                }
            })
            .collect()
    }

    /// Every `PinAcquired` in the verified chain for `segment_id` that
    /// has NO corresponding `PinReleased` (same `pin_id`) anywhere in
    /// the same verified chain — the ONE derivation this crate ever
    /// performs for "is this segment pinned," never a cached/mutable
    /// flag.
    pub fn active_pins_for(&self, segment_id: &SegmentId) -> Result<Vec<PinAcquired>, PinStoreError> {
        let records = self.records()?;
        let released_ids: std::collections::HashSet<uuid::Uuid> = records.iter().filter_map(|r| if let PinStoreRecord::Released(rel) = r { Some(rel.pin_id) } else { None }).collect();
        Ok(records
            .into_iter()
            .filter_map(|r| if let PinStoreRecord::Acquired(acq) = r { Some(acq) } else { None })
            .filter(|acq| &acq.segment_id == segment_id && !released_ids.contains(&acq.pin_id))
            .collect())
    }

    /// Detects a duplicate/replayed `PinAcquired`: the same `pin_id`
    /// appearing more than once in the verified chain — never produced
    /// by this module's own `acquire` (which always mints a fresh
    /// `uuid::Uuid::new_v4()`), so a positive result here means
    /// tampering or a replayed record, never a normal operation. Fails
    /// closed with the FIRST duplicate found, rather than silently
    /// normalizing — mirrors `crate::ledger`'s own "preserve, never
    /// silently merge" anomaly discipline.
    pub fn check_no_duplicate_pin_ids(&self) -> Result<(), PinStoreError> {
        let records = self.records()?;
        let mut seen = std::collections::HashSet::new();
        for record in &records {
            if let PinStoreRecord::Acquired(acquired) = record {
                if !seen.insert(acquired.pin_id) {
                    return Err(PinStoreError::DuplicatePinAcquired(acquired.pin_id));
                }
            }
        }
        Ok(())
    }
}

/// Opens the dedicated pin-store Vault at `locator` and runs a REAL
/// `scan_chain` against its REAL raw bytes — same discipline as
/// `crate::ledger::verify_and_scan`.
pub fn verify_and_scan(locator: &std::path::Path, journal_path: &std::path::Path) -> Result<(storage::Vault, VerifiedPinStoreState), PinStoreError> {
    let vault = storage::Vault::open_with_default_indexer(locator.to_path_buf(), journal_path).map_err(PinStoreError::Vault)?;
    let raw = vault.read_audit_log_for_recovery_scan().map_err(PinStoreError::Vault)?;
    let scan = scan_chain_bytes(&raw);
    if let Some(stop_reason) = &scan.stop_reason {
        if !audit_chain_core::may_resume_without_recovery(&scan) {
            return Err(PinStoreError::ChainInvalid(stop_reason.clone()));
        }
    }
    Ok((vault, VerifiedPinStoreState { verified_records: scan.verified_records, last_verified: scan.last_verified }))
}

/// Acquires a new pin on `segment_id` — appends one durable
/// `PinAcquired` record. Re-verifies the chain immediately before
/// opening the writer (never trusts a caller-supplied stale tail).
/// Returns the new pin's own `pin_id`, which the caller must hold onto
/// to release it later (see [`release`]).
///
/// This function does NOT check `retention_eligible` or anything about
/// the segment itself — that policy belongs to `crate::retention`
/// (which checks pins, among other things, before Phase 1), never to
/// this generic pin store.
pub fn acquire(locator: &std::path::Path, journal_path: &std::path::Path, segment_id: &SegmentId, owner_class: OwnerClass, reason: PinReason, acquired_at: &str) -> Result<uuid::Uuid, PinStoreError> {
    let (vault, state) = verify_and_scan(locator, journal_path)?;
    let pin_id = uuid::Uuid::new_v4();
    let pin = PinAcquired { pin_id, segment_id: segment_id.clone(), owner_class, reason, acquired_at: acquired_at.to_string() };
    append_record(&vault, &state, &pin)?;
    Ok(pin_id)
}

/// Releases a previously-acquired pin — appends one durable
/// `PinReleased` record correlating to `pin_id`. Fails if `pin_id`
/// does not correspond to any `PinAcquired` in the verified chain
/// (never silently accepted — a release for an unknown/already-
/// released pin is itself worth surfacing, never ignored).
pub fn release(locator: &std::path::Path, journal_path: &std::path::Path, pin_id: uuid::Uuid, released_at: &str) -> Result<(), PinStoreError> {
    let (vault, state) = verify_and_scan(locator, journal_path)?;
    let records = state.records()?;
    let has_matching_acquire = records.iter().any(|r| matches!(r, PinStoreRecord::Acquired(a) if a.pin_id == pin_id));
    if !has_matching_acquire {
        return Err(PinStoreError::ReleaseWithoutAcquire(pin_id));
    }
    let released = PinReleased { pin_id, released_at: released_at.to_string() };
    append_record(&vault, &state, &released)
}

fn append_record<T: serde::Serialize>(vault: &storage::Vault, state: &VerifiedPinStoreState, payload: &T) -> Result<(), PinStoreError> {
    let next_sequence = state.last_verified.as_ref().map(|(s, _)| s + 1).unwrap_or(0);
    let prev_hash = state.last_verified.as_ref().map(|(_, h)| h.clone()).unwrap_or_else(|| audit_chain_core::GENESIS_HASH.to_string());
    let (bytes, record_hash) = audit_chain_core::serialize_record(payload, next_sequence, &prev_hash);
    let mut writer = vault.open_audit_writer(state.last_verified.as_ref().map(|(s, h)| (*s, h.as_str()))).map_err(PinStoreError::WriterOpenFailed)?;
    writer.append(&bytes, next_sequence, &record_hash).map_err(PinStoreError::AppendFailed)
}

fn scan_chain_bytes(raw: &[u8]) -> audit_chain_core::ScanResult {
    audit_chain_core::scan_chain(&split_into_line_outcomes(raw))
}

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

    fn harness() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let locator = dir.path().join("pin-store");
        std::fs::create_dir_all(&locator).unwrap();
        let journal = locator.join("journal.sqlite");
        (dir, locator, journal)
    }

    fn owner() -> OwnerClass {
        OwnerClass::new("mcp-audit-bridge").unwrap()
    }

    #[test]
    fn a_fresh_pin_store_verifies_as_empty_with_no_error() {
        let (_dir, locator, journal) = harness();
        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        assert!(state.verified_records.is_empty());
    }

    #[test]
    fn acquiring_a_pin_makes_the_segment_show_an_active_pin() {
        let (_dir, locator, journal) = harness();
        let segment = SegmentId::from_index(1);
        let pin_id = acquire(&locator, &journal, &segment, owner(), PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();

        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        let active = state.active_pins_for(&segment).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].pin_id, pin_id);
    }

    #[test]
    fn releasing_a_pin_removes_it_from_the_active_set() {
        let (_dir, locator, journal) = harness();
        let segment = SegmentId::from_index(1);
        let pin_id = acquire(&locator, &journal, &segment, owner(), PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();
        release(&locator, &journal, pin_id, "2026-01-01T00:05:00Z").unwrap();

        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        let active = state.active_pins_for(&segment).unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn a_pin_on_one_segment_does_not_appear_active_for_a_different_segment() {
        let (_dir, locator, journal) = harness();
        let segment_a = SegmentId::from_index(1);
        let segment_b = SegmentId::from_index(2);
        acquire(&locator, &journal, &segment_a, owner(), PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();

        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        assert!(state.active_pins_for(&segment_b).unwrap().is_empty());
        assert_eq!(state.active_pins_for(&segment_a).unwrap().len(), 1);
    }

    #[test]
    fn releasing_a_pin_id_with_no_matching_acquire_fails() {
        let (_dir, locator, journal) = harness();
        let fake_pin_id = uuid::Uuid::new_v4();
        let result = release(&locator, &journal, fake_pin_id, "2026-01-01T00:00:00Z");
        assert!(matches!(result, Err(PinStoreError::ReleaseWithoutAcquire(id)) if id == fake_pin_id), "got {result:?}");
    }

    #[test]
    fn multiple_pins_from_different_owners_on_the_same_segment_are_all_active_independently() {
        let (_dir, locator, journal) = harness();
        let segment = SegmentId::from_index(1);
        let pin_1 = acquire(&locator, &journal, &segment, OwnerClass::new("mcp-audit-bridge").unwrap(), PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();
        let pin_2 = acquire(&locator, &journal, &segment, OwnerClass::new("some-other-owner").unwrap(), PinReason::ManualOperatorHold, "2026-01-01T00:01:00Z").unwrap();

        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        let active = state.active_pins_for(&segment).unwrap();
        assert_eq!(active.len(), 2);
        let ids: std::collections::HashSet<_> = active.iter().map(|p| p.pin_id).collect();
        assert!(ids.contains(&pin_1) && ids.contains(&pin_2));

        // Releasing one leaves the other active.
        release(&locator, &journal, pin_1, "2026-01-01T00:02:00Z").unwrap();
        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        let active = state.active_pins_for(&segment).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].pin_id, pin_2);
    }

    #[test]
    fn tampering_with_the_pin_stores_own_chain_content_is_detected_on_reopen() {
        let (_dir, locator, journal) = harness();
        acquire(&locator, &journal, &SegmentId::from_index(1), owner(), PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();

        let log_path = locator.join(".audit").join("audit.log");
        let mut bytes = std::fs::read(&log_path).unwrap();
        let len = bytes.len();
        bytes[len - 5] ^= 0xFF;
        std::fs::write(&log_path, bytes).unwrap();

        let result = verify_and_scan(&locator, &journal);
        assert!(matches!(result, Err(PinStoreError::ChainInvalid(_))), "expected ChainInvalid, got a different outcome");
    }

    #[test]
    fn a_second_concurrent_writer_is_denied_while_the_first_holds_the_pin_stores_lock() {
        let (_dir, locator, journal) = harness();
        let vault = storage::Vault::open_with_default_indexer(locator, &journal).unwrap();
        let _first = vault.open_audit_writer(None).unwrap();
        let second = vault.open_audit_writer(None);
        assert!(matches!(second, Err(storage::AuditWriterOpenError::AlreadyLocked)), "got {second:?}");
    }

    #[test]
    fn a_replayed_pin_acquired_record_with_a_duplicate_pin_id_is_representable_and_detectable() {
        // The pin store itself does not refuse a duplicate pin_id at
        // append time via this module's own `acquire` (which always
        // mints a fresh uuid) -- this test proves that IF a duplicate
        // ever appeared (e.g. a tampered/replayed chain), it is
        // detectable by a caller inspecting `records()` directly,
        // mirroring `crate::ledger`'s own "preserve, never silently
        // merge" anomaly discipline.
        let (_dir, locator, journal) = harness();
        let segment = SegmentId::from_index(1);
        let pin_id = uuid::Uuid::new_v4();
        let pin = PinAcquired { pin_id, segment_id: segment, owner_class: owner(), reason: PinReason::PendingCheckpointWrite, acquired_at: "2026-01-01T00:00:00Z".into() };

        let (vault, state) = verify_and_scan(&locator, &journal).unwrap();
        append_record(&vault, &state, &pin).unwrap();
        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        append_record(&vault, &state, &pin).unwrap(); // same pin_id again, bypassing acquire()'s own fresh-uuid guarantee

        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        let records = state.records().unwrap();
        let duplicate_count = records.iter().filter(|r| matches!(r, PinStoreRecord::Acquired(a) if a.pin_id == pin_id)).count();
        assert_eq!(duplicate_count, 2, "a duplicate/replayed PinAcquired must be preserved and countable, never silently merged");

        let result = state.check_no_duplicate_pin_ids();
        assert!(matches!(result, Err(PinStoreError::DuplicatePinAcquired(id)) if id == pin_id), "got {result:?}");
    }

    #[test]
    fn a_pin_store_with_no_duplicates_passes_the_duplicate_check() {
        let (_dir, locator, journal) = harness();
        acquire(&locator, &journal, &SegmentId::from_index(1), owner(), PinReason::PendingCheckpointWrite, "2026-01-01T00:00:00Z").unwrap();
        acquire(&locator, &journal, &SegmentId::from_index(2), owner(), PinReason::PendingCheckpointWrite, "2026-01-01T00:01:00Z").unwrap();
        let (_, state) = verify_and_scan(&locator, &journal).unwrap();
        assert!(state.check_no_duplicate_pin_ids().is_ok());
    }
}
