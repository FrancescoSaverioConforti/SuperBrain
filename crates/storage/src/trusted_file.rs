//! M6.4: the smallest possible seam exposing `handle_safety`'s existing
//! symlink/reparse-point/hard-link/containment verification to a caller
//! OUTSIDE this crate, for reading a trusted, operator-controlled local
//! file (an MCP Client composition root's own TOML configuration, and
//! the resolved executable/script files it content-hashes) — never a
//! general-purpose file-reading API, and never a modification to
//! `handle_safety` itself.
//!
//! **This module is a pure pass-through, zero new logic**: it does not
//! change `handle_safety`'s own visibility (still `pub(crate)`, still
//! unreachable directly from outside this crate), does not add a new
//! `OpenMode` variant, and does not alter any check `read_verified`
//! already performs. The one function below calls that existing,
//! already-audited primitive exactly as-is — reusing it rather than
//! duplicating security-sensitive filesystem logic a second time, the
//! same discipline this crate's own `audit_writer.rs` already states for
//! itself ("this module adds no new security logic of its own").

use std::path::Path;

use crate::handle_safety;
use crate::vault::StorageError;

/// Reads `dir.join(file_name)`'s bytes through the exact same
/// open-without-following-a-reparse-point, verify-regular-file,
/// verify-single-hard-link, verify-resolves-inside-`dir` check
/// `handle_safety::read_verified` already performs for this crate's own
/// internal callers — never a separate open-then-stat window, and never
/// a weaker guarantee than what `storage`'s own audit log enjoys.
///
/// `label` is used only to label a returned error (mirrors
/// `read_verified`'s own `note_id_label` parameter) — it carries no
/// trust meaning of its own.
pub fn read_trusted_file(dir: &Path, file_name: &Path, label: &str) -> Result<Vec<u8>, StorageError> {
    handle_safety::read_verified(dir, file_name, label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-trusted-file-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_back_the_exact_bytes_of_a_genuine_regular_file() {
        let dir = unique_test_dir();
        std::fs::write(dir.join("config.toml"), b"hello = \"world\"\n").unwrap();
        let bytes = read_trusted_file(&dir, Path::new("config.toml"), "<test-config>").unwrap();
        assert_eq!(bytes, b"hello = \"world\"\n");
    }

    #[test]
    fn a_missing_file_is_reported_as_not_found() {
        let dir = unique_test_dir();
        let result = read_trusted_file(&dir, Path::new("does-not-exist.toml"), "<test-config>");
        assert!(matches!(result, Err(StorageError::NotFound { .. })), "got {result:?}");
    }

    #[test]
    fn a_symlink_planted_at_the_trusted_file_path_is_rejected_not_followed() {
        let dir = unique_test_dir();
        let outside = unique_test_dir();
        let outside_file = outside.join("victim.toml");
        std::fs::write(&outside_file, b"attacker content").unwrap();

        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_file(&outside_file, dir.join("config.toml"));
        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink(&outside_file, dir.join("config.toml"));

        match link_result {
            Ok(()) => {}
            Err(e) => {
                eprintln!("skipping: could not create a real symlink on this host (needs admin/dev-mode on Windows): {e}");
                return;
            }
        }

        let result = read_trusted_file(&dir, Path::new("config.toml"), "<test-config>");
        assert!(result.is_err(), "a symlink planted at the trusted file's own path must be rejected, never followed, got {result:?}");
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"attacker content", "the symlink target outside the directory must remain completely untouched");
    }

    #[test]
    fn a_hard_link_at_the_trusted_file_path_is_rejected() {
        let dir = unique_test_dir();
        let outside = unique_test_dir();
        let outside_file = outside.join("victim.toml");
        std::fs::write(&outside_file, b"attacker content").unwrap();

        std::fs::hard_link(&outside_file, dir.join("config.toml")).unwrap();

        let result = read_trusted_file(&dir, Path::new("config.toml"), "<test-config>");
        assert!(result.is_err(), "a hard-linked file at the trusted file's own path must be rejected, got {result:?}");
    }

    #[test]
    fn handle_safety_itself_remains_unreachable_directly_from_outside_this_crate() {
        // Structural, compile-time property: this test file, which lives
        // INSIDE the storage crate, can name `handle_safety` only because
        // it is `pub(crate)` -- an external crate has no such access.
        // This test's own existence (compiling at all) does not prove
        // this on its own; it is documented here as the one place a
        // future accidental `pub mod handle_safety;` change would be
        // most likely to be noticed by a developer reading this file,
        // matching this module's own doc comment's explicit promise.
        let _: fn(&Path, &Path, &str) -> Result<Vec<u8>, StorageError> = handle_safety::read_verified;
    }
}
