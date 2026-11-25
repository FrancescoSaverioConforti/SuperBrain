//! M7.3, second correction: the bridge's MUTABLE PROGRESS checkpoint —
//! `last_imported` only, separate from the durable source identity (see
//! `crate::registration`, split out precisely so losing THIS file can
//! never destroy that identity).
//!
//! **The checkpoint is an accelerator, never the authority** (frozen):
//! losing this file is always recoverable without duplicating anything,
//! as long as the registration (`crate::registration`) survives — a
//! full destination-chain scan (`crate::dedup`) under the SAME
//! `SourceLogId` reconstructs exactly which records were already
//! imported, and this file is simply rebuilt from that. This file's own
//! survival is never load-bearing for correctness, only for skipping a
//! full rescan on every ordinary run.

use serde::{Deserialize, Serialize};
use std::io::Write as _;

/// This checkpoint file's own format version — independent of, and never
/// confused with, `crate::registration::SOURCE_LOG_REGISTRATION_FORMAT_VERSION`
/// or any chain/event format version.
pub const BRIDGE_CHECKPOINT_FORMAT_VERSION: u32 = 1;

/// The bridge's own progress state: the last source record this bridge
/// has itself successfully imported, or `None` before its first import.
/// Deliberately carries NO `SourceLogId`/registration data of its own —
/// see this module's own top doc comment for why.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCheckpoint {
    pub format_version: u32,
    pub last_imported: Option<crate::identity::SourceRecordIdentity>,
}

impl BridgeCheckpoint {
    pub fn new(last_imported: Option<crate::identity::SourceRecordIdentity>) -> Self {
        Self { format_version: BRIDGE_CHECKPOINT_FORMAT_VERSION, last_imported }
    }
}

/// Why a checkpoint could not be loaded or saved — every variant is
/// fail-closed: this module never guesses a reasonable-looking recovery
/// for a malformed file, never truncates or rewrites a corrupt one on
/// its own.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("failed to read checkpoint file: {0}")]
    Io(std::io::Error),
    #[error("checkpoint file content is not valid JSON: {0}")]
    MalformedJson(String),
    #[error("checkpoint declares format_version {found}, this code only accepts {BRIDGE_CHECKPOINT_FORMAT_VERSION}")]
    UnsupportedFormatVersion { found: u32 },
}

/// Loads a checkpoint from `path`, or returns `Ok(None)` if no file
/// exists there yet — the one "absence is fine" case (see this module's
/// own top doc comment: absence here never implies anything about the
/// SOURCE REGISTRATION, which is a wholly separate file with its own
/// absence semantics in `crate::registration`). Any other failure (I/O
/// error short of not-found, malformed JSON, wrong format version) is
/// reported as [`CheckpointError`], never silently treated as "start
/// fresh."
pub fn load(path: &std::path::Path) -> Result<Option<BridgeCheckpoint>, CheckpointError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CheckpointError::Io(e)),
    };
    let checkpoint: BridgeCheckpoint = serde_json::from_slice(&bytes).map_err(|e| CheckpointError::MalformedJson(e.to_string()))?;
    if checkpoint.format_version != BRIDGE_CHECKPOINT_FORMAT_VERSION {
        return Err(CheckpointError::UnsupportedFormatVersion { found: checkpoint.format_version });
    }
    Ok(Some(checkpoint))
}

/// Persists `checkpoint` to `path` atomically: writes to a sibling
/// temporary file in the same directory, `fsync`s it, then renames it
/// over `path` — a crash at any point before the rename leaves the
/// PRIOR checkpoint (or no checkpoint) intact; a crash after the rename
/// leaves the NEW checkpoint fully intact (same-filesystem rename is
/// atomic). This is the one and only way this crate ever writes its own
/// checkpoint file.
pub fn save(path: &std::path::Path, checkpoint: &BridgeCheckpoint) -> Result<(), CheckpointError> {
    let bytes = serde_json::to_vec_pretty(checkpoint).expect("BridgeCheckpoint serialization cannot fail: no non-finite floats, no non-string map keys");
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp_path = dir.join(format!(".{}.tmp-{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("checkpoint"), uuid::Uuid::new_v4()));

    let mut tmp_file = std::fs::File::create(&tmp_path).map_err(CheckpointError::Io)?;
    tmp_file.write_all(&bytes).map_err(CheckpointError::Io)?;
    tmp_file.sync_all().map_err(CheckpointError::Io)?;
    drop(tmp_file);

    std::fs::rename(&tmp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        CheckpointError::Io(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{SourceLogId, SourceRecordIdentity};

    fn sample_checkpoint() -> BridgeCheckpoint {
        BridgeCheckpoint::new(Some(SourceRecordIdentity { source_log_id: SourceLogId::generate(), sequence: 3, record_hash: "h".repeat(64) }))
    }

    #[test]
    fn loading_a_path_with_no_file_yet_returns_none_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        assert!(matches!(load(&path), Ok(None)));
    }

    #[test]
    fn a_saved_checkpoint_loads_back_byte_for_byte_equal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        let checkpoint = sample_checkpoint();
        save(&path, &checkpoint).unwrap();
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded, checkpoint);
    }

    #[test]
    fn saving_twice_leaves_only_the_second_checkpoint_and_no_leftover_tmp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        let mut checkpoint = sample_checkpoint();
        save(&path, &checkpoint).unwrap();
        checkpoint.last_imported = Some(SourceRecordIdentity { source_log_id: SourceLogId::generate(), sequence: 4, record_hash: "b".repeat(64) });
        save(&path, &checkpoint).unwrap();
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded, checkpoint);

        let leftover_tmp_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftover_tmp_files.is_empty(), "atomic save must never leave a temp file behind: {leftover_tmp_files:?}");
    }

    #[test]
    fn a_malformed_json_checkpoint_file_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        std::fs::write(&path, b"not valid json at all").unwrap();
        assert!(matches!(load(&path), Err(CheckpointError::MalformedJson(_))));
    }

    #[test]
    fn a_checkpoint_declaring_an_unsupported_format_version_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        let mut value = serde_json::to_value(sample_checkpoint()).unwrap();
        value["format_version"] = serde_json::json!(999);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let result = load(&path);
        assert!(matches!(result, Err(CheckpointError::UnsupportedFormatVersion { found: 999 })), "got {result:?}");
    }

    #[test]
    fn a_checkpoint_truncated_mid_write_fails_closed_as_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        let full = serde_json::to_vec(&sample_checkpoint()).unwrap();
        let truncated = &full[..full.len() / 2];
        std::fs::write(&path, truncated).unwrap();
        assert!(matches!(load(&path), Err(CheckpointError::MalformedJson(_))));
    }
}
