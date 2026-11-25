//! M7.5.5: adopts a pre-existing `storage::Vault` (the single, un-
//! segmented log every vault had before this crate existed) as the
//! permanent legacy segment `0000000000` — in place, with zero file
//! movement or duplication, and permanently `retention_eligible: false`
//! (never derived from the id string — see `crate::domain`'s own doc
//! comment on that frozen correction).
//!
//! **Why this must never be retirable, structurally, not just by
//! policy** (the frozen correction): a dedicated segment created by
//! `crate::rotation` owns an entire directory created exclusively for
//! audit purposes, so `locator == retirable_payload_path` safely for
//! those. The legacy segment's `locator` is the REAL vault root —
//! deleting it would delete notes/storage that have nothing to do with
//! the audit log. This module therefore never computes or exposes a
//! "retirable payload path" for the legacy segment at all; the only
//! code path that could ever call `std::fs::remove_dir_all` on a
//! segment's payload (`crate::retention`) checks `retention_eligible`
//! BEFORE even the first phase of the protocol, and this module is what
//! sets that field to `false`, permanently, for this one segment.

use crate::domain::{SegmentId, SegmentManifest, SegmentManifestEntry, SegmentStatus};

#[derive(Debug, thiserror::Error)]
pub enum AdoptionError {
    #[error("a manifest already exists at this path -- adoption only runs once, on a brand-new state directory")]
    ManifestAlreadyExists,
    #[error("failed to persist the adopted manifest: {0}")]
    ManifestSaveFailed(#[from] crate::manifest_store::ManifestStoreError),
}

/// Adopts `existing_vault_root` as the legacy segment, writing a brand
/// new manifest to `manifest_path` with exactly one entry: segment
/// `0000000000`, `locator: existing_vault_root` (unchanged, nothing
/// copied or moved), `status: Active` (it is still the one vault
/// callers keep appending to until/unless a real rotation happens),
/// `retention_eligible: false` (permanent, explicit — see this module's
/// own top doc comment).
///
/// Fails if a manifest already exists at `manifest_path` — adoption is
/// a one-time, explicit action on a brand-new state directory, never an
/// implicit "overwrite whatever was already decided."
pub fn adopt_existing_vault(manifest_path: &std::path::Path, existing_vault_root: &std::path::Path, retention_ledger_locator: &std::path::Path) -> Result<SegmentManifest, AdoptionError> {
    if crate::manifest_store::load(manifest_path).map_err(AdoptionError::ManifestSaveFailed)?.is_some() {
        return Err(AdoptionError::ManifestAlreadyExists);
    }

    let mut manifest = SegmentManifest::new(retention_ledger_locator.to_path_buf());
    manifest.segments.push(SegmentManifestEntry {
        segment_id: SegmentId::from_index(0),
        locator: existing_vault_root.to_path_buf(),
        status: SegmentStatus::Active,
        final_sequence: None,
        final_record_hash: None,
        retained_payload_present: true,
        retention_eligible: false,
    });

    crate::manifest_store::save(manifest_path, &manifest)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adopting_a_fresh_state_directory_creates_exactly_one_legacy_entry() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("segment-manifest.json");
        let vault_root = dir.path().join("existing-vault");
        let ledger_locator = dir.path().join("retention-ledger");

        let manifest = adopt_existing_vault(&manifest_path, &vault_root, &ledger_locator).unwrap();

        assert_eq!(manifest.segments.len(), 1);
        let legacy = &manifest.segments[0];
        assert_eq!(legacy.segment_id, SegmentId::from_index(0));
        assert_eq!(legacy.locator, vault_root);
        assert_eq!(legacy.status, SegmentStatus::Active);
        assert!(!legacy.retention_eligible, "the legacy segment must be permanently non-retirable");
    }

    #[test]
    fn adoption_never_moves_or_creates_anything_at_the_existing_vault_root() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("segment-manifest.json");
        let vault_root = dir.path().join("existing-vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        std::fs::create_dir_all(vault_root.join(".audit")).unwrap();
        std::fs::write(vault_root.join(".audit").join("audit.log"), b"pre-existing-content\n").unwrap();
        let before = std::fs::read(vault_root.join(".audit").join("audit.log")).unwrap();

        adopt_existing_vault(&manifest_path, &vault_root, &dir.path().join("retention-ledger")).unwrap();

        let after = std::fs::read(vault_root.join(".audit").join("audit.log")).unwrap();
        assert_eq!(before, after, "adoption must never touch the pre-existing audit.log's own bytes");
        assert!(!dir.path().join("existing-vault-copy").exists(), "adoption must never duplicate the vault elsewhere");
    }

    #[test]
    fn adopting_twice_on_the_same_manifest_path_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("segment-manifest.json");
        adopt_existing_vault(&manifest_path, &dir.path().join("vault-a"), &dir.path().join("ledger")).unwrap();
        let second = adopt_existing_vault(&manifest_path, &dir.path().join("vault-b"), &dir.path().join("ledger"));
        assert!(matches!(second, Err(AdoptionError::ManifestAlreadyExists)));
    }

    #[test]
    fn the_resulting_manifest_validates_structurally() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = adopt_existing_vault(&dir.path().join("segment-manifest.json"), &dir.path().join("vault"), &dir.path().join("ledger")).unwrap();
        assert!(manifest.validate().is_ok());
    }
}
