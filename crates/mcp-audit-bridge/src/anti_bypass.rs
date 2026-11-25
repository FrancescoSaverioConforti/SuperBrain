//! M7.7: a defense-in-depth tripwire against *accidental* use of the
//! legacy, single-vault [`crate::run::run_once`] on a destination that
//! actually participates in segmentation (`audit-segmentation`, a
//! sibling crate this one has, and must keep, zero dependency on).
//!
//! **This is explicitly NOT a security/trust boundary** (the frozen
//! M7.7 instruction): its absence must never be read as positive proof
//! that a destination is "safe" or "not segmented" — it is a plain
//! file-existence check, trivially avoidable by a deliberate bypass
//! (moving the manifest elsewhere), and it makes no claim beyond that.
//! The real security property of a segmented deployment comes from the
//! production composition root using `mcp-audit-segmented` exclusively
//! — never from this tripwire.
//!
//! **Why this introduces no `mcp-audit-bridge -> audit-segmentation`
//! dependency**: this module imports no type, function, or constant
//! from that crate. It duplicates, BY VALUE, the one fact it needs —
//! the literal filename `audit-segmentation::adoption` writes a
//! segmentation manifest under — the same "same value, independently
//! defined, no shared dependency" pattern already used elsewhere in
//! this workspace (e.g. `storage::audit_writer`'s own test-only
//! `GENESIS_CHAIN_HASH` duplicating `audit_chain_core::GENESIS_HASH`'s
//! value). If that filename ever changes on the `audit-segmentation`
//! side, this tripwire simply stops firing — a silent weakening, never
//! a silent failure, consistent with "defense-in-depth only."

/// Duplicated by value from `audit_segmentation::adoption`'s own
/// manifest filename convention — never imported, see this module's
/// own top doc comment for why.
const SEGMENTATION_MANIFEST_FILE_NAME: &str = "segment-manifest.json";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("this destination participates in segmentation (a {SEGMENTATION_MANIFEST_FILE_NAME} marker was found adjacent to the destination vault root) -- run_once (single-vault legacy) must never be used here; use the segmented composition layer (mcp-audit-segmented) instead")]
pub struct SegmentedDestinationGuardError;

/// Checks for the segmentation marker as a SIBLING of
/// `destination_vault_root` (never inside it, never inside its own
/// `.audit/` directory — this never touches anything `storage` owns)
/// — the conventional position a segmented composition root places
/// its manifest relative to the legacy/first adopted segment's own
/// vault root. Returns `Err` if found; this is the ONLY effect this
/// function has — it never opens, parses, or trusts the file's
/// content, only its existence.
pub fn guard_against_segmented_destination(destination_vault_root: &std::path::Path) -> Result<(), SegmentedDestinationGuardError> {
    let marker_found = destination_vault_root.parent().map(|parent| parent.join(SEGMENTATION_MANIFEST_FILE_NAME).exists()).unwrap_or(false);
    if marker_found {
        Err(SegmentedDestinationGuardError)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_marker_present_passes() {
        let dir = tempfile::tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        assert!(guard_against_segmented_destination(&vault_root).is_ok());
    }

    #[test]
    fn a_sibling_manifest_file_trips_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        std::fs::write(dir.path().join(SEGMENTATION_MANIFEST_FILE_NAME), b"{}").unwrap();
        let result = guard_against_segmented_destination(&vault_root);
        assert!(matches!(result, Err(SegmentedDestinationGuardError)));
    }

    #[test]
    fn the_guard_never_reads_or_parses_the_markers_own_content() {
        // Even deliberately invalid/garbage content must still trip the
        // guard the same way -- this function's only job is detecting
        // presence, never validating shape.
        let dir = tempfile::tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        std::fs::write(dir.path().join(SEGMENTATION_MANIFEST_FILE_NAME), b"not even json, does not matter").unwrap();
        assert!(guard_against_segmented_destination(&vault_root).is_err());
    }

    #[test]
    fn a_manifest_inside_the_vault_root_itself_does_not_trip_the_guard() {
        // The marker is checked as a SIBLING of the vault root, never
        // inside it -- a file with the same name placed INSIDE the
        // vault's own directory (not the conventional position) must
        // not be mistaken for the real marker.
        let dir = tempfile::tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        std::fs::write(vault_root.join(SEGMENTATION_MANIFEST_FILE_NAME), b"{}").unwrap();
        assert!(guard_against_segmented_destination(&vault_root).is_ok());
    }
}
