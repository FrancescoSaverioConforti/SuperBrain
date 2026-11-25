//! M7.5.5 (shared plumbing): atomic, fail-closed persistence for
//! `domain::SegmentManifest` — same discipline as every other durable
//! state file already proven in `mcp-audit-bridge`
//! (`registration.rs`/`checkpoint.rs`): own format version, atomic
//! write (temp file + fsync + rename), malformed/wrong-version fails
//! closed, path supplied by the caller, never derived from untrusted
//! input.
//!
//! **The manifest is an accelerator/locator store, never an authority**
//! (the frozen rule, restated here because this module is the one place
//! that persists it): nothing in this module performs trust decisions —
//! it only reads/writes bytes and validates the manifest's own internal
//! shape (`SegmentManifest::validate`). Whether to TRUST what a loaded
//! manifest claims about a segment's linkage or a retention's
//! authorization is always a separate, independent verification against
//! the real chains (`crate::ledger`/`crate::verifier`), never performed
//! here.

use std::io::Write as _;

use crate::domain::{SegmentManifest, SegmentManifestError};

#[derive(Debug, thiserror::Error)]
pub enum ManifestStoreError {
    #[error("failed to read manifest file: {0}")]
    Io(std::io::Error),
    #[error("manifest file content is not valid JSON: {0}")]
    MalformedJson(String),
    #[error("manifest failed structural validation: {0}")]
    Invalid(#[from] SegmentManifestError),
}

/// Loads a manifest from `path`, or `Ok(None)` if no file exists there
/// yet. Runs `SegmentManifest::validate` before returning `Ok(Some(_))`
/// — a manifest that loads as valid JSON but fails internal structural
/// validation (two Active segments, a duplicate id, ...) is reported as
/// [`ManifestStoreError::Invalid`], never silently accepted.
pub fn load(path: &std::path::Path) -> Result<Option<SegmentManifest>, ManifestStoreError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ManifestStoreError::Io(e)),
    };
    let manifest: SegmentManifest = serde_json::from_slice(&bytes).map_err(|e| ManifestStoreError::MalformedJson(e.to_string()))?;
    manifest.validate()?;
    Ok(Some(manifest))
}

/// Persists `manifest` to `path` atomically — temp file in the same
/// directory, `fsync`, atomic rename. Validates BEFORE writing, so this
/// module never durably persists a structurally invalid manifest.
pub fn save(path: &std::path::Path, manifest: &SegmentManifest) -> Result<(), ManifestStoreError> {
    manifest.validate()?;
    let bytes = serde_json::to_vec_pretty(manifest).expect("SegmentManifest serialization cannot fail: no non-finite floats, no non-string map keys");
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp_path = dir.join(format!(".{}.tmp-{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("manifest"), uuid::Uuid::new_v4()));

    let mut tmp_file = std::fs::File::create(&tmp_path).map_err(ManifestStoreError::Io)?;
    tmp_file.write_all(&bytes).map_err(ManifestStoreError::Io)?;
    tmp_file.sync_all().map_err(ManifestStoreError::Io)?;
    drop(tmp_file);

    std::fs::rename(&tmp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        ManifestStoreError::Io(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SegmentId, SegmentManifestEntry, SegmentStatus};

    fn sample_manifest() -> SegmentManifest {
        let mut m = SegmentManifest::new("ledger".into());
        m.segments.push(SegmentManifestEntry { segment_id: SegmentId::from_index(0), locator: "seg0".into(), status: SegmentStatus::Active, final_sequence: None, final_record_hash: None, retained_payload_present: true, retention_eligible: false });
        m
    }

    #[test]
    fn loading_a_path_with_no_file_yet_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load(&dir.path().join("m.json")), Ok(None)));
    }

    #[test]
    fn a_saved_manifest_loads_back_equal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        let manifest = sample_manifest();
        save(&path, &manifest).unwrap();
        assert_eq!(load(&path).unwrap().unwrap(), manifest);
    }

    #[test]
    fn malformed_json_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(matches!(load(&path), Err(ManifestStoreError::MalformedJson(_))));
    }

    #[test]
    fn a_structurally_invalid_manifest_fails_closed_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        let mut value = serde_json::to_value(sample_manifest()).unwrap();
        value["segments"][0]["status"] = serde_json::json!("closed"); // Closed but no final state -- invalid
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(load(&path), Err(ManifestStoreError::Invalid(_))));
    }

    #[test]
    fn saving_twice_leaves_no_leftover_tmp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        save(&path, &sample_manifest()).unwrap();
        save(&path, &sample_manifest()).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().contains(".tmp-")).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
