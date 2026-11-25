//! M7.5.1: pure domain/state types for the segment lifecycle — no I/O
//! anywhere in this module. Every type here is a plain, serializable
//! value; validation is a pure function over already-constructed values,
//! never a filesystem check.
//!
//! **`segment-manifest.json` is an accelerator/locator store, never an
//! authority** (the frozen M7.5/M7.6 rule): [`SegmentManifest`] and
//! [`SegmentManifestEntry`] below are exactly that — a convenient index
//! a caller may cache and rebuild, never the sole source of truth for
//! whether a segment is linked, retained, or authorized. The real
//! authority is always the audit chain itself (`SegmentLinkage`/
//! `RetentionAuthorized` records) and the retention ledger's own
//! verified chain — this module defines the SHAPES those live in, not
//! their trust semantics (that is `crate::ledger`/`crate::retention`'s
//! job, built on top of these types).
//!
//! **`retention_eligible` is explicit, never derived from `segment_id`**
//! (the frozen correction): the legacy segment (adopted in place from a
//! pre-existing vault, see `crate::adoption`) is permanently
//! `retention_eligible: false` because its own field says so, not
//! because any code path special-cases a particular id string.

use serde::{Deserialize, Serialize};

/// A segment's own durable identity — a zero-padded, monotonically
/// increasing decimal string (`"0000000000"`, `"0000000001"`, ...), so
/// lexicographic and numeric ordering agree and there is no ambiguity
/// from a timestamp-based id (which can be non-monotonic on a real
/// system, or manipulated). Never derived from path or content — this
/// mirrors the same "identity is bridge/segmentation-owned, never
/// content-derived" discipline `mcp-audit-bridge::identity::SourceLogId`
/// already established for the sibling crate's own identity concept.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SegmentId(String);

/// Fixed width matches `u64::MAX`'s own decimal digit count (20), wide
/// enough that this scheme never needs widening — chosen once, up
/// front, rather than risking an ordering break from a later width
/// change.
const SEGMENT_ID_WIDTH: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SegmentIdError {
    #[error("segment id string {0:?} is not exactly {SEGMENT_ID_WIDTH} ASCII digits")]
    NotFixedWidthDigits(String),
}

impl SegmentId {
    pub fn from_index(index: u64) -> Self {
        Self(format!("{index:0width$}", width = SEGMENT_ID_WIDTH))
    }

    /// Parses an already-persisted id string, validating its shape —
    /// this is the one place a value coming back from disk (e.g. the
    /// manifest) is checked before being trusted as a well-formed
    /// `SegmentId` at all.
    pub fn parse(raw: &str) -> Result<Self, SegmentIdError> {
        if raw.len() == SEGMENT_ID_WIDTH && raw.bytes().all(|b| b.is_ascii_digit()) {
            Ok(Self(raw.to_string()))
        } else {
            Err(SegmentIdError::NotFixedWidthDigits(raw.to_string()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The next segment id in rotation order — the one and only way a
    /// new id is ever derived from an old one, so there is a single
    /// place that enforces "always exactly one more than the current
    /// maximum," never a gap or a duplicate by construction.
    pub fn next(&self) -> Result<Self, SegmentIdError> {
        let n: u64 = self.0.parse().map_err(|_| SegmentIdError::NotFixedWidthDigits(self.0.clone()))?;
        Ok(Self::from_index(n + 1))
    }
}

impl std::fmt::Display for SegmentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
impl SegmentId {
    /// Test-only helper mirroring `from_index` but named for readability
    /// at call sites that just want "the Nth segment id," not
    /// implying anything about how a real caller derives the next one
    /// (that is `next()`'s job).
    pub(crate) fn test_nth(n: u64) -> Self {
        Self::from_index(n)
    }
}

/// A segment's own lifecycle state — `Active` (the one segment a writer
/// may currently append to) or `Closed` (immutable, rotated away from).
/// Exactly one segment in a well-formed manifest may ever be `Active` at
/// a time (enforced by `SegmentManifest::validate`, never assumed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentStatus {
    Active,
    Closed,
}

/// One segment's full manifest entry — a cache/projection of facts that
/// are independently verifiable from the segment's own chain and the
/// retention ledger (see this module's own top doc comment); never
/// trusted on its own by `crate::retention`/`crate::verifier` without
/// that independent verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentManifestEntry {
    pub segment_id: SegmentId,
    /// Where to find this segment's own Vault root today — a locator,
    /// never an identity comparison (same "path is never a trust
    /// anchor" discipline `mcp-audit-bridge::identity` already
    /// established for its own, structurally similar locator field).
    pub locator: std::path::PathBuf,
    pub status: SegmentStatus,
    /// `None` until the segment is `Closed` and its final chain state is
    /// known.
    pub final_sequence: Option<u64>,
    pub final_record_hash: Option<String>,
    /// Whether this segment's payload has already been removed by an
    /// authorized retention (see `crate::retention`). Defaults to
    /// `false` for every segment until a real retention completes.
    pub retained_payload_present: bool,
    /// Explicit, never derived from `segment_id` (the frozen
    /// correction) — `false` permanently for the legacy segment
    /// (`crate::adoption`), `true` by default for every segment created
    /// by rotation (`crate::rotation`).
    pub retention_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SegmentManifestError {
    #[error("manifest declares format_version {found}, this code only accepts {SEGMENT_MANIFEST_FORMAT_VERSION}")]
    UnsupportedFormatVersion { found: u32 },
    #[error("more than one segment is marked Active: {0:?}")]
    MultipleActiveSegments(Vec<SegmentId>),
    #[error("no segment is marked Active")]
    NoActiveSegment,
    #[error("duplicate segment id {0} appears more than once in the manifest")]
    DuplicateSegmentId(SegmentId),
    #[error("segment {0} is Closed but has no final_sequence/final_record_hash recorded")]
    ClosedSegmentMissingFinalState(SegmentId),
    #[error("segment {0} is Active but has a final_sequence/final_record_hash recorded (Active segments have no final state yet)")]
    ActiveSegmentHasFinalState(SegmentId),
    #[error("segment {0} has the lowest SegmentId in the manifest but is retention_eligible: true -- the lowest-id segment must always be the permanent, non-retirable legacy anchor")]
    LowestIdSegmentIsRetentionEligible(SegmentId),
}

/// This manifest format's own version marker — independent of, and
/// never confused with, `AUDIT_RECORD_FORMAT_VERSION` or any other
/// format version in this workspace. Bumped only if this crate's own
/// on-disk manifest shape changes in a way a prior reader could not
/// parse.
pub const SEGMENT_MANIFEST_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentManifest {
    pub format_version: u32,
    /// Where the dedicated retention-ledger Vault lives — a locator
    /// only, per the frozen rule: a caller must independently verify
    /// the ledger found at this path (its own chain via `scan_chain`,
    /// its own trusted `head.ref`) before trusting anything about it;
    /// this field alone never authorizes trust (`crate::ledger`'s own
    /// doc comment covers that verification).
    pub retention_ledger_locator: std::path::PathBuf,
    pub segments: Vec<SegmentManifestEntry>,
}

impl SegmentManifest {
    pub fn new(retention_ledger_locator: std::path::PathBuf) -> Self {
        Self { format_version: SEGMENT_MANIFEST_FORMAT_VERSION, retention_ledger_locator, segments: Vec::new() }
    }

    /// Pure structural validation — no I/O, no filesystem check against
    /// `locator`/`retention_ledger_locator` (that is a caller's own,
    /// separate, explicit verification step, never folded into this
    /// function). Checks only internal consistency of the manifest's
    /// own declared state: exactly one `Active` segment, no duplicate
    /// ids, `Closed` segments carry final chain state and `Active`
    /// segments do not.
    pub fn validate(&self) -> Result<(), SegmentManifestError> {
        if self.format_version != SEGMENT_MANIFEST_FORMAT_VERSION {
            return Err(SegmentManifestError::UnsupportedFormatVersion { found: self.format_version });
        }

        let mut seen = std::collections::HashSet::new();
        let mut active_ids = Vec::new();
        for entry in &self.segments {
            if !seen.insert(entry.segment_id.clone()) {
                return Err(SegmentManifestError::DuplicateSegmentId(entry.segment_id.clone()));
            }
            match entry.status {
                SegmentStatus::Active => {
                    active_ids.push(entry.segment_id.clone());
                    if entry.final_sequence.is_some() || entry.final_record_hash.is_some() {
                        return Err(SegmentManifestError::ActiveSegmentHasFinalState(entry.segment_id.clone()));
                    }
                }
                SegmentStatus::Closed => {
                    if entry.final_sequence.is_none() || entry.final_record_hash.is_none() {
                        return Err(SegmentManifestError::ClosedSegmentMissingFinalState(entry.segment_id.clone()));
                    }
                }
            }
        }

        match active_ids.len() {
            0 if self.segments.is_empty() => {} // brand-new manifest, nothing adopted/created yet
            0 => return Err(SegmentManifestError::NoActiveSegment),
            1 => {}
            _ => return Err(SegmentManifestError::MultipleActiveSegments(active_ids)),
        }

        // The lowest SegmentId by the type's own real Ord (never a
        // hardcoded string, never Vec position, never a "first entry"
        // assumption) must always be retention_eligible: false -- the
        // permanent legacy anchor, regardless of manifest array order.
        if let Some(lowest) = self.segments.iter().min_by_key(|entry| entry.segment_id.clone()) {
            if lowest.retention_eligible {
                return Err(SegmentManifestError::LowestIdSegmentIsRetentionEligible(lowest.segment_id.clone()));
            }
        }

        Ok(())
    }

    pub fn active_segment(&self) -> Option<&SegmentManifestEntry> {
        self.segments.iter().find(|s| s.status == SegmentStatus::Active)
    }

    pub fn find(&self, id: &SegmentId) -> Option<&SegmentManifestEntry> {
        self.segments.iter().find(|s| &s.segment_id == id)
    }
}

/// One durable receipt in the retention ledger — never an
/// `AuditEventKind`, never written into any audit segment; this is the
/// retention ledger's own, separate payload type, serialized/appended
/// through the exact same `audit_chain_core::serialize_record`/
/// `storage::Vault::open_audit_writer` machinery an audit segment uses,
/// but as its own independent chain (see `crate::ledger`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetentionReceipt {
    pub retired_segment_id: SegmentId,
    pub retired_segment_final_sequence: u64,
    pub retired_segment_final_record_hash: String,
    /// A short, content-free identifier of who/what authorized this
    /// retirement (an operator id, a policy id) — never free text, same
    /// content-free discipline every other audit-shaped type in this
    /// workspace already follows.
    pub retirement_authorized_by: String,
    pub retired_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SegmentId ---

    #[test]
    fn from_index_zero_pads_to_the_fixed_width() {
        assert_eq!(SegmentId::from_index(0).as_str(), "00000000000000000000");
        assert_eq!(SegmentId::from_index(1).as_str(), "00000000000000000001");
    }

    #[test]
    fn lexicographic_ordering_of_segment_ids_matches_numeric_ordering() {
        let a = SegmentId::from_index(9);
        let b = SegmentId::from_index(10);
        assert!(a < b, "fixed-width zero-padding must keep lexicographic order aligned with numeric order");
    }

    #[test]
    fn next_produces_exactly_one_more_than_the_current_id() {
        let first = SegmentId::from_index(0);
        let second = first.next().unwrap();
        assert_eq!(second, SegmentId::from_index(1));
    }

    #[test]
    fn parse_rejects_a_string_of_the_wrong_width() {
        assert!(SegmentId::parse("1").is_err());
        assert!(SegmentId::parse("000000000000000000001").is_err()); // one too many digits
    }

    #[test]
    fn parse_rejects_non_digit_characters() {
        assert!(SegmentId::parse("0000000000000000000a").is_err());
    }

    #[test]
    fn parse_accepts_a_well_formed_id_round_tripped_through_from_index() {
        let id = SegmentId::from_index(42);
        let parsed = SegmentId::parse(id.as_str()).unwrap();
        assert_eq!(parsed, id);
    }

    // --- SegmentManifest validation ---

    // Mirrors real production behavior (crate::adoption/crate::rotation):
    // segment 0 is never retention_eligible, every other id defaults to
    // true -- so every existing fixture call site keeps its own
    // intended meaning (testing active/closed/duplicate/etc.) without
    // ALSO incidentally violating the new lowest-id invariant, unless a
    // test deliberately constructs a violation (see the dedicated tests
    // for that below, which build the entry inline instead of through
    // these helpers).
    fn closed_entry(id: SegmentId) -> SegmentManifestEntry {
        let retention_eligible = id != SegmentId::test_nth(0);
        SegmentManifestEntry { segment_id: id, locator: "p".into(), status: SegmentStatus::Closed, final_sequence: Some(5), final_record_hash: Some("h".repeat(64)), retained_payload_present: true, retention_eligible }
    }

    fn active_entry(id: SegmentId) -> SegmentManifestEntry {
        let retention_eligible = id != SegmentId::test_nth(0);
        SegmentManifestEntry { segment_id: id, locator: "p".into(), status: SegmentStatus::Active, final_sequence: None, final_record_hash: None, retained_payload_present: true, retention_eligible }
    }

    #[test]
    fn a_brand_new_empty_manifest_validates_as_ok() {
        let manifest = SegmentManifest::new("ledger".into());
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn exactly_one_active_segment_validates_as_ok() {
        let mut manifest = SegmentManifest::new("ledger".into());
        manifest.segments.push(closed_entry(SegmentId::test_nth(0)));
        manifest.segments.push(active_entry(SegmentId::test_nth(1)));
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn two_active_segments_fails_validation() {
        let mut manifest = SegmentManifest::new("ledger".into());
        manifest.segments.push(active_entry(SegmentId::test_nth(0)));
        manifest.segments.push(active_entry(SegmentId::test_nth(1)));
        assert!(matches!(manifest.validate(), Err(SegmentManifestError::MultipleActiveSegments(_))));
    }

    #[test]
    fn no_active_segment_among_a_nonempty_set_fails_validation() {
        let mut manifest = SegmentManifest::new("ledger".into());
        manifest.segments.push(closed_entry(SegmentId::test_nth(0)));
        assert!(matches!(manifest.validate(), Err(SegmentManifestError::NoActiveSegment)));
    }

    #[test]
    fn a_duplicate_segment_id_fails_validation() {
        let mut manifest = SegmentManifest::new("ledger".into());
        manifest.segments.push(closed_entry(SegmentId::test_nth(0)));
        manifest.segments.push(active_entry(SegmentId::test_nth(0)));
        assert!(matches!(manifest.validate(), Err(SegmentManifestError::DuplicateSegmentId(_))));
    }

    #[test]
    fn a_closed_segment_missing_final_state_fails_validation() {
        let mut manifest = SegmentManifest::new("ledger".into());
        let mut bad = closed_entry(SegmentId::test_nth(0));
        bad.final_sequence = None;
        manifest.segments.push(bad);
        manifest.segments.push(active_entry(SegmentId::test_nth(1)));
        assert!(matches!(manifest.validate(), Err(SegmentManifestError::ClosedSegmentMissingFinalState(_))));
    }

    #[test]
    fn an_active_segment_carrying_final_state_fails_validation() {
        let mut manifest = SegmentManifest::new("ledger".into());
        let mut bad = active_entry(SegmentId::test_nth(0));
        bad.final_sequence = Some(3);
        manifest.segments.push(bad);
        assert!(matches!(manifest.validate(), Err(SegmentManifestError::ActiveSegmentHasFinalState(_))));
    }

    #[test]
    fn active_segment_finds_the_one_active_entry() {
        let mut manifest = SegmentManifest::new("ledger".into());
        manifest.segments.push(closed_entry(SegmentId::test_nth(0)));
        manifest.segments.push(active_entry(SegmentId::test_nth(1)));
        assert_eq!(manifest.active_segment().unwrap().segment_id, SegmentId::test_nth(1));
    }

    // --- lowest-id-must-be-non-retirable invariant (fixes the gap a
    //     prior security review found: this was previously enforced
    //     only by convention in crate::retention's own runtime check,
    //     never by SegmentManifest::validate itself) ---

    #[test]
    fn a_manifest_whose_lowest_id_segment_is_non_retirable_validates_as_ok() {
        let mut manifest = SegmentManifest::new("ledger".into());
        manifest.segments.push(closed_entry(SegmentId::test_nth(0))); // retention_eligible: false via the fixture helper
        manifest.segments.push(active_entry(SegmentId::test_nth(1)));
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn a_manifest_whose_lowest_id_segment_is_retention_eligible_fails_validation() {
        let mut manifest = SegmentManifest::new("ledger".into());
        let mut violating_legacy = closed_entry(SegmentId::test_nth(0));
        violating_legacy.retention_eligible = true; // deliberately construct the violation inline, bypassing the fixture's own safe default
        manifest.segments.push(violating_legacy);
        manifest.segments.push(active_entry(SegmentId::test_nth(1)));
        let result = manifest.validate();
        assert!(matches!(result, Err(SegmentManifestError::LowestIdSegmentIsRetentionEligible(ref id)) if id == &SegmentId::test_nth(0)), "got {result:?}");
    }

    #[test]
    fn out_of_order_entries_with_a_non_retirable_lowest_id_still_validate_as_ok() {
        // The invariant must be independent of manifest ARRAY order --
        // computed from the real SegmentId::Ord, never Vec position.
        let mut manifest = SegmentManifest::new("ledger".into());
        manifest.segments.push(active_entry(SegmentId::test_nth(2)));
        manifest.segments.push(closed_entry(SegmentId::test_nth(0))); // lowest id, pushed SECOND, still non-retirable via the fixture
        manifest.segments.push(closed_entry(SegmentId::test_nth(1)));
        assert!(manifest.validate().is_ok(), "array order must not matter -- only the real minimum SegmentId does");
    }

    #[test]
    fn out_of_order_entries_with_a_retirable_lowest_id_fail_validation() {
        let mut manifest = SegmentManifest::new("ledger".into());
        manifest.segments.push(active_entry(SegmentId::test_nth(2)));
        let mut violating_legacy = closed_entry(SegmentId::test_nth(0));
        violating_legacy.retention_eligible = true;
        manifest.segments.push(violating_legacy); // lowest id, pushed SECOND, violating
        manifest.segments.push(closed_entry(SegmentId::test_nth(1)));
        let result = manifest.validate();
        assert!(matches!(result, Err(SegmentManifestError::LowestIdSegmentIsRetentionEligible(ref id)) if id == &SegmentId::test_nth(0)), "got {result:?}");
    }

    #[test]
    fn segments_after_the_lowest_id_being_retention_eligible_true_remain_valid() {
        let mut manifest = SegmentManifest::new("ledger".into());
        manifest.segments.push(closed_entry(SegmentId::test_nth(0))); // non-retirable, via the fixture
        manifest.segments.push(closed_entry(SegmentId::test_nth(1))); // retention_eligible: true, via the fixture -- and that is fine, only the LOWEST id is constrained
        manifest.segments.push(active_entry(SegmentId::test_nth(2)));
        assert!(manifest.validate().is_ok());
        assert!(manifest.find(&SegmentId::test_nth(1)).unwrap().retention_eligible);
    }

    #[test]
    fn an_empty_manifest_has_no_lowest_id_segment_to_check_and_validates_as_ok() {
        // Confirms the empty-manifest case explicitly: with no segments
        // at all, there is no "lowest id" to examine, so this invariant
        // imposes no constraint -- the pre-existing empty-manifest
        // acceptance (a_brand_new_empty_manifest_validates_as_ok, above)
        // must not be broken by this new check.
        let manifest = SegmentManifest::new("ledger".into());
        assert!(manifest.segments.is_empty());
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn retention_eligible_is_an_explicit_field_never_implied_by_segment_id() {
        // The frozen correction's own regression guard: constructing a
        // segment 0000000000 entry with retention_eligible: true must
        // compile and behave exactly as declared -- this module itself
        // must never special-case the legacy id string anywhere.
        let legacy_but_eligible = SegmentManifestEntry { segment_id: SegmentId::from_index(0), locator: "p".into(), status: SegmentStatus::Closed, final_sequence: Some(0), final_record_hash: Some("h".repeat(64)), retained_payload_present: true, retention_eligible: true };
        assert!(legacy_but_eligible.retention_eligible, "this module's own types impose no id-based special case -- that policy lives in crate::adoption, not here");
    }

    // --- RetentionReceipt / SegmentManifestEntry serialization ---

    #[test]
    fn retention_receipt_round_trips_through_serde_json_exactly() {
        let receipt = RetentionReceipt { retired_segment_id: SegmentId::test_nth(3), retired_segment_final_sequence: 10, retired_segment_final_record_hash: "a".repeat(64), retirement_authorized_by: "policy:default-retention".into(), retired_at: "2026-01-01T00:00:00Z".into() };
        let json = serde_json::to_string(&receipt).unwrap();
        let decoded: RetentionReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, receipt);
    }

    #[test]
    fn segment_manifest_round_trips_through_serde_json_exactly() {
        let mut manifest = SegmentManifest::new("ledger-root".into());
        manifest.segments.push(closed_entry(SegmentId::test_nth(0)));
        manifest.segments.push(active_entry(SegmentId::test_nth(1)));
        let json = serde_json::to_string(&manifest).unwrap();
        let decoded: SegmentManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, manifest);
    }
}
