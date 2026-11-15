//! B4.3: persistent vector index storage, per
//! `docs/superpowers/specs/2026-11-15-b4-3-vector-index-storage-format.md`.
//! Compiled only under `dense-retrieval` (B4.1), same as `crate::dense`.
//!
//! A hand-rolled, versioned binary format -- matching this project's
//! own existing convention (`storage`'s on-disk record format is also
//! hand-rolled, not an external serialization library) rather than
//! introducing a new dependency for this one job.

use std::io::Write;
use std::path::Path;

pub const MAGIC: &[u8; 4] = b"SBDI";
pub const FORMAT_VERSION: u32 = 1;

/// One note's stored vector entry.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexEntry {
    pub note_id: String,
    /// A fast, non-cryptographic hash of the note's `text_searchable` at
    /// the time it was embedded -- NOT a security boundary (unlike the
    /// model-artifact hash verification in `crate::dense`), purely a
    /// cheap "has this note's searchable text changed" check for the
    /// incremental-update path (B4.4).
    pub content_hash: u64,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DenseIndex {
    pub model_identity_hash: [u8; 32],
    pub embedding_dimension: u32,
    pub entries: Vec<IndexEntry>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    /// Covers a missing magic, a bad checksum, or any structurally
    /// malformed byte layout -- all collapsed into one variant
    /// deliberately (B4.3 design doc, Section 4, step 2): a file that
    /// doesn't even look like this format at all is not meaningfully
    /// distinct from one whose checksum fails, from this type's own
    /// point of view. The caller (B4.4) treats both as "rebuild."
    Corrupted(String),
    FormatVersionMismatch {
        found: u32,
        expected: u32,
    },
    ModelIdentityMismatch,
    EmbeddingDimensionMismatch {
        found: u32,
        expected: u32,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Corrupted(msg) => write!(f, "corrupted index: {msg}"),
            Self::FormatVersionMismatch { found, expected } => {
                write!(
                    f,
                    "format version mismatch: found {found}, expected {expected}"
                )
            }
            Self::ModelIdentityMismatch => write!(f, "model identity hash mismatch"),
            Self::EmbeddingDimensionMismatch { found, expected } => {
                write!(
                    f,
                    "embedding dimension mismatch: found {found}, expected {expected}"
                )
            }
        }
    }
}
impl std::error::Error for LoadError {}

/// Pure, dependency-free CRC32 (IEEE 802.3 polynomial), built from a
/// 256-entry lookup table computed once. Matches the standard CRC-32
/// algorithm used by zlib/gzip/PNG, so this implementation's output is
/// independently verifiable against any reference CRC32 tool, not an
/// invented variant.
fn crc32(data: &[u8]) -> u32 {
    const POLY: u32 = 0xEDB88320;
    let table: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut j = 0;
            while j < 8 {
                c = if c & 1 != 0 { POLY ^ (c >> 1) } else { c >> 1 };
                j += 1;
            }
            table[i] = c;
            i += 1;
        }
        table
    };
    let mut crc = 0xFFFFFFFFu32;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = table[idx] ^ (crc >> 8);
    }
    !crc
}

/// Serializes `index` into the exact on-disk layout (B4.3 design doc,
/// Section 2). Pure, in-memory -- writing it to disk atomically is a
/// separate function (`write_atomic`), so this function alone is
/// trivially unit-testable without touching the filesystem.
pub fn serialize(index: &DenseIndex) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&index.model_identity_hash);
    body.extend_from_slice(&index.embedding_dimension.to_le_bytes());
    body.extend_from_slice(&(index.entries.len() as u32).to_le_bytes());
    for entry in &index.entries {
        let id_bytes = entry.note_id.as_bytes();
        body.extend_from_slice(&(id_bytes.len() as u16).to_le_bytes());
        body.extend_from_slice(id_bytes);
        body.extend_from_slice(&entry.content_hash.to_le_bytes());
        for &value in &entry.embedding {
            body.extend_from_slice(&value.to_le_bytes());
        }
    }
    let checksum = crc32(&body);

    let mut out = Vec::with_capacity(4 + 4 + 4 + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Deserializes `bytes` back into a `DenseIndex`, per the exact
/// validation sequence in the B4.3 design doc, Section 4 (steps 2-4
/// only -- steps 1, 5, 6 are the CALLER's job, since this function has
/// no knowledge of "currently configured model" or "file missing").
pub fn deserialize(bytes: &[u8]) -> Result<DenseIndex, LoadError> {
    if bytes.len() < 12 {
        return Err(LoadError::Corrupted(
            "file shorter than fixed header".to_string(),
        ));
    }
    if &bytes[0..4] != MAGIC {
        return Err(LoadError::Corrupted("bad magic bytes".to_string()));
    }
    let format_version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let stored_checksum = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let body = &bytes[12..];
    let actual_checksum = crc32(body);
    if actual_checksum != stored_checksum {
        return Err(LoadError::Corrupted(format!(
            "checksum mismatch: stored {stored_checksum:08x}, computed {actual_checksum:08x}"
        )));
    }
    if format_version != FORMAT_VERSION {
        return Err(LoadError::FormatVersionMismatch {
            found: format_version,
            expected: FORMAT_VERSION,
        });
    }

    let mut cursor = 0usize;
    let read_bytes = |cursor: &mut usize, n: usize| -> Result<&[u8], LoadError> {
        let end = cursor
            .checked_add(n)
            .ok_or_else(|| LoadError::Corrupted("length overflow".to_string()))?;
        let slice = body
            .get(*cursor..end)
            .ok_or_else(|| LoadError::Corrupted("unexpected end of file".to_string()))?;
        *cursor = end;
        Ok(slice)
    };

    let model_identity_hash: [u8; 32] = read_bytes(&mut cursor, 32)?
        .try_into()
        .map_err(|_| LoadError::Corrupted("model identity hash length".to_string()))?;
    let embedding_dimension = u32::from_le_bytes(
        read_bytes(&mut cursor, 4)?
            .try_into()
            .map_err(|_| LoadError::Corrupted("embedding dimension length".to_string()))?,
    );
    let note_count = u32::from_le_bytes(
        read_bytes(&mut cursor, 4)?
            .try_into()
            .map_err(|_| LoadError::Corrupted("note count length".to_string()))?,
    );

    let mut entries = Vec::with_capacity(note_count as usize);
    for _ in 0..note_count {
        let id_len = u16::from_le_bytes(
            read_bytes(&mut cursor, 2)?
                .try_into()
                .map_err(|_| LoadError::Corrupted("note id length prefix".to_string()))?,
        ) as usize;
        let id_bytes = read_bytes(&mut cursor, id_len)?;
        let note_id = String::from_utf8(id_bytes.to_vec())
            .map_err(|_| LoadError::Corrupted("note id is not valid UTF-8".to_string()))?;
        let content_hash = u64::from_le_bytes(
            read_bytes(&mut cursor, 8)?
                .try_into()
                .map_err(|_| LoadError::Corrupted("content hash length".to_string()))?,
        );
        let mut embedding = Vec::with_capacity(embedding_dimension as usize);
        for _ in 0..embedding_dimension {
            let value = f32::from_le_bytes(
                read_bytes(&mut cursor, 4)?
                    .try_into()
                    .map_err(|_| LoadError::Corrupted("embedding value length".to_string()))?,
            );
            embedding.push(value);
        }
        entries.push(IndexEntry {
            note_id,
            content_hash,
            embedding,
        });
    }

    if cursor != body.len() {
        return Err(LoadError::Corrupted(format!(
            "trailing bytes after the last declared entry: {} unread",
            body.len() - cursor
        )));
    }

    Ok(DenseIndex {
        model_identity_hash,
        embedding_dimension,
        entries,
    })
}

/// Writes `bytes` to `path` atomically: a sibling temp file in the same
/// directory, fsynced, then renamed over `path` -- mirrors
/// `storage::atomic::atomic_write`'s own pattern exactly (not reused
/// directly: that function is `pub(crate)` to `storage`, deliberately
/// not exported, per its own doc comment).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp_name = format!(
        "{}.tmp-{}-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let tmp_path = dir.join(tmp_name);
    {
        let mut tmp_file = std::fs::File::create(&tmp_path)?;
        tmp_file.write_all(bytes)?;
        tmp_file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    #[cfg(unix)]
    {
        if let Ok(dir_file) = std::fs::File::open(dir) {
            let _ = dir_file.sync_all();
        }
    }
    Ok(())
}

/// Reads and deserializes the index at `path`. Returns `Ok(None)` if the
/// file does not exist at all (B4.3 design doc, Section 4, step 1 --
/// "not yet built" is not an error from this function's point of view).
pub fn read_from(path: &Path) -> Result<Option<DenseIndex>, LoadError> {
    match std::fs::read(path) {
        Ok(bytes) => deserialize(&bytes).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(LoadError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_index() -> DenseIndex {
        DenseIndex {
            model_identity_hash: [7u8; 32],
            embedding_dimension: 4,
            entries: vec![
                IndexEntry {
                    note_id: "bug-2026-10-30-example".to_string(),
                    content_hash: 0xDEADBEEF,
                    embedding: vec![0.1, 0.2, 0.3, 0.4],
                },
                IndexEntry {
                    note_id: "decision-2026-11-01-other".to_string(),
                    content_hash: 0x12345678,
                    embedding: vec![-0.5, 0.0, 1.0, -1.0],
                },
            ],
        }
    }

    #[test]
    fn crc32_matches_the_known_reference_value_for_an_empty_input() {
        // The standard CRC-32 of an empty byte sequence is 0, a
        // well-known, independently-checkable property of this exact
        // algorithm (not specific to this implementation).
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn crc32_matches_the_known_reference_value_for_the_bytes_check() {
        // The canonical CRC-32 self-check vector: crc32("123456789") ==
        // 0xCBF43926, cited in the original Rocksoft CRC specification
        // and reproduced by every standard CRC-32 implementation.
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn serialize_then_deserialize_round_trips_exactly() {
        let index = sample_index();
        let bytes = serialize(&index);
        let restored = deserialize(&bytes).unwrap();
        assert_eq!(restored, index);
    }

    #[test]
    fn deserialize_rejects_bad_magic() {
        let mut bytes = serialize(&sample_index());
        bytes[0] = b'X';
        match deserialize(&bytes) {
            Err(LoadError::Corrupted(_)) => {}
            other => panic!("expected Corrupted for bad magic, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_rejects_a_flipped_bit_via_checksum() {
        let mut bytes = serialize(&sample_index());
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01; // flip one bit deep in the last embedding value
        match deserialize(&bytes) {
            Err(LoadError::Corrupted(_)) => {}
            other => panic!("expected Corrupted for a flipped bit, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_rejects_a_truncated_file() {
        let bytes = serialize(&sample_index());
        let truncated = &bytes[..bytes.len() - 10];
        match deserialize(truncated) {
            Err(LoadError::Corrupted(_)) => {}
            other => panic!("expected Corrupted for a truncated file, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_rejects_a_note_count_claiming_more_entries_than_remaining_bytes_can_hold() {
        // Adversarial: a corrupted/malicious file claims 1000 entries but
        // only has bytes for 2. Must fail safely (Corrupted), never
        // panic or read out of bounds -- the checksum alone would
        // already catch a real corruption, so this specifically
        // recomputes the checksum over the TAMPERED body to isolate the
        // length-overrun handling from the checksum check.
        let mut bytes = serialize(&sample_index());
        let note_count_offset = 12 + 32 + 4; // header + model_hash + dimension
        bytes[note_count_offset..note_count_offset + 4].copy_from_slice(&1000u32.to_le_bytes());
        let body = bytes[12..].to_vec();
        let new_checksum = crc32(&body);
        bytes[8..12].copy_from_slice(&new_checksum.to_le_bytes());

        match deserialize(&bytes) {
            Err(LoadError::Corrupted(_)) => {}
            other => panic!("expected Corrupted for an overclaimed note count, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_rejects_a_note_id_length_prefix_longer_than_the_remaining_buffer() {
        // Adversarial: the first entry's note_id_len field claims far
        // more bytes than actually remain in the file. Must fail safely,
        // never panic or read out of bounds.
        let mut bytes = serialize(&sample_index());
        let first_id_len_offset = 12 + 32 + 4 + 4; // header + model_hash + dimension + note_count
        bytes[first_id_len_offset..first_id_len_offset + 2]
            .copy_from_slice(&u16::MAX.to_le_bytes());
        let body = bytes[12..].to_vec();
        let new_checksum = crc32(&body);
        bytes[8..12].copy_from_slice(&new_checksum.to_le_bytes());

        match deserialize(&bytes) {
            Err(LoadError::Corrupted(_)) => {}
            other => {
                panic!("expected Corrupted for an oversized note_id length prefix, got {other:?}")
            }
        }
    }

    #[test]
    fn deserialize_rejects_trailing_garbage_bytes_after_a_valid_body() {
        // Adversarial: a structurally valid file with extra bytes
        // appended after the last declared entry (e.g. a different file
        // concatenated, or a write that didn't truncate a prior longer
        // version -- the atomic-rename path prevents this in practice,
        // but deserialize itself must still detect it defensively).
        let mut bytes = serialize(&sample_index());
        bytes.extend_from_slice(b"trailing garbage");
        let body = bytes[12..].to_vec();
        let new_checksum = crc32(&body);
        bytes[8..12].copy_from_slice(&new_checksum.to_le_bytes());

        match deserialize(&bytes) {
            Err(LoadError::Corrupted(_)) => {}
            other => panic!("expected Corrupted for trailing garbage, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_rejects_an_unrecognized_format_version() {
        let mut bytes = serialize(&sample_index());
        // Overwrite format_version (bytes 4..8), then recompute the
        // checksum over the unchanged body so this test isolates the
        // version check from the checksum check.
        bytes[4..8].copy_from_slice(&99u32.to_le_bytes());
        let body = bytes[12..].to_vec();
        let new_checksum = crc32(&body);
        bytes[8..12].copy_from_slice(&new_checksum.to_le_bytes());

        match deserialize(&bytes) {
            Err(LoadError::FormatVersionMismatch {
                found: 99,
                expected: 1,
            }) => {}
            other => panic!("expected FormatVersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn read_from_a_missing_file_returns_ok_none() {
        let path = std::env::temp_dir().join(format!(
            "superbrain-dense-storage-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert_eq!(read_from(&path).unwrap(), None);
    }

    #[test]
    fn write_atomic_then_read_from_round_trips_through_the_real_filesystem() {
        let dir = std::env::temp_dir().join(format!(
            "superbrain-dense-storage-roundtrip-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("project-x.bin");
        let index = sample_index();
        write_atomic(&path, &serialize(&index)).unwrap();

        let restored = read_from(&path).unwrap().unwrap();
        assert_eq!(restored, index);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_leaves_no_temp_file_behind_on_success() {
        let dir = std::env::temp_dir().join(format!(
            "superbrain-dense-storage-no-leftover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("project-x.bin");
        write_atomic(&path, &serialize(&sample_index())).unwrap();

        let leftover: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftover.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_overwrites_a_previous_index_completely() {
        let dir = std::env::temp_dir().join(format!(
            "superbrain-dense-storage-overwrite-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("project-x.bin");

        let first = sample_index();
        write_atomic(&path, &serialize(&first)).unwrap();

        let second = DenseIndex {
            model_identity_hash: [9u8; 32],
            embedding_dimension: 2,
            entries: vec![IndexEntry {
                note_id: "only-note".to_string(),
                content_hash: 1,
                embedding: vec![0.5, 0.5],
            }],
        };
        write_atomic(&path, &serialize(&second)).unwrap();

        let restored = read_from(&path).unwrap().unwrap();
        assert_eq!(
            restored, second,
            "the second write must fully replace the first, no leftover bytes"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
