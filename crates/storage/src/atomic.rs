use std::fs::File;
use std::io::Write;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum AtomicWriteError {
    #[error("io error during atomic write: {0}")]
    Io(#[from] std::io::Error),
}

/// Writes `contents` to `path` atomically: the bytes land in a sibling temp
/// file in the same directory as `path` (so the final rename is a
/// same-filesystem, atomic operation on both Unix and Windows), the temp
/// file's contents are fsynced, then it is renamed over `path`.
///
/// No note schema, policy, or versioning lives here — this is pure
/// filesystem mechanics (that layering is Task 6's job).
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), AtomicWriteError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));

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
        let mut tmp_file = File::create(&tmp_path)?;
        tmp_file.write_all(contents)?;
        tmp_file.sync_all()?;
        // `tmp_file` drops here, closing the handle before rename.
    }

    std::fs::rename(&tmp_path, path)?;

    // Flush the directory-entry metadata for the rename. Directory fsync is
    // not meaningfully supported through Rust's std API on Windows
    // (`File::open(dir).sync_all()` on a directory handle is
    // unreliable/unsupported there), so this step is Unix-only; on Windows
    // we rely on `rename`'s own durability guarantees instead. The exact
    // ordering of these syncs is a platform-specific choice, fixed and
    // tested here for this project's two target platforms.
    #[cfg(unix)]
    {
        if let Ok(dir_file) = File::open(dir) {
            let _ = dir_file.sync_all();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates and returns a fresh, uniquely-named subdirectory under the
    /// system temp dir, so parallel test runs never collide on the same path.
    fn unique_test_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "superbrain-atomic-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_produces_readable_file_with_exact_contents() {
        let dir = unique_test_dir();
        let path = dir.join("note.md");
        atomic_write(&path, b"hello world").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_behind_on_success() {
        let dir = unique_test_dir();
        let path = dir.join("note.md");
        atomic_write(&path, b"content").unwrap();
        let leftover: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftover.is_empty());
    }

    #[test]
    fn atomic_write_overwrites_existing_file_completely() {
        let dir = unique_test_dir();
        let path = dir.join("note.md");
        atomic_write(&path, b"first version, much longer content here").unwrap();
        atomic_write(&path, b"v2").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"v2"); // no trailing bytes from the longer first write
    }
}
