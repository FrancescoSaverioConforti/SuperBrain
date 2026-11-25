//! M7.5.4: a cross-process, OS-released, non-blocking exclusive lock
//! covering segment ROTATION, RETENTION, and manifest MUTATION —
//! reusing the exact primitive `storage::audit_writer`'s own single-
//! writer lock already proves safe in this codebase, and the same
//! design pattern `mcp_audit_bridge::bridge_lock` already implements
//! and tests for its own, structurally identical first-run-concurrency
//! problem (reused as a PATTERN, not shared code, per this crate's own
//! sibling-not-child relationship to `mcp-audit-bridge`).
//!
//! **Why this exists, specifically for this crate**: the single-writer
//! lock `storage::Vault::open_audit_writer` already holds covers
//! ordinary APPEND to one already-open segment or the ledger. It does
//! NOT cover the higher-level decision "should I close this segment and
//! open a new one" or "should I retire this segment" — two concurrent
//! processes could each decide independently to rotate, or one could
//! rotate while another retires, racing on the SAME manifest file and
//! the SAME "which segment is Active" question. This lock serializes
//! exactly those three higher-level operations, never ordinary append
//! (which stays covered by `storage::audit_writer`'s own lock,
//! unmodified).
//!
//! **Crash safety, identically to the bridge's own lock**:
//! `std::fs::File::try_lock()` is an OS-level advisory lock tied to the
//! open file handle, not a marker written into the file's own content —
//! released by the OS the instant the holding process exits for ANY
//! reason. This module never treats "does the lock file exist" as a
//! signal of anything; only the live OS lock state matters.

/// A held cross-process exclusive lock over one segmentation state
/// directory's lifecycle operations. Dropping it (including via an
/// early return) releases it immediately.
#[derive(Debug)]
pub struct LifecycleLock {
    _file: std::fs::File,
}

#[derive(Debug, thiserror::Error)]
pub enum LifecycleLockError {
    /// Another rotation/retention/manifest-mutation is already in
    /// progress for this exact state directory — fail closed
    /// immediately, never waited out.
    #[error("another segment lifecycle operation (rotation/retention/manifest mutation) is already in progress for this state directory")]
    AlreadyRunning,
    #[error("failed to open lifecycle lock file: {0}")]
    Io(std::io::Error),
}

/// Fixed, internally-chosen sibling file name — never a name a caller
/// supplies, so there is no caller-controlled arbitrary path for this
/// lock (the same "no marker-file semantics, no second caller-supplied
/// path" discipline `mcp_audit_bridge::bridge_lock` already
/// establishes).
const LOCK_FILE_NAME: &str = ".audit-segmentation-lifecycle.lock";

/// Derives this state directory's own lifecycle lock path from
/// `manifest_path`'s own parent directory — a fixed sibling, not a
/// second independently configurable path.
fn lock_path_for(manifest_path: &std::path::Path) -> std::path::PathBuf {
    let dir = manifest_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    dir.join(LOCK_FILE_NAME)
}

/// Attempts to acquire the exclusive lifecycle lock for the state
/// directory containing `manifest_path`. Never blocks. Must be held for
/// the ENTIRE critical section of a rotation, a retention, or any
/// direct manifest mutation — see `crate::rotation`/`crate::retention`'s
/// own call sites for the exact scope each one holds it across.
pub fn acquire(manifest_path: &std::path::Path) -> Result<LifecycleLock, LifecycleLockError> {
    let path = lock_path_for(manifest_path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(LifecycleLockError::Io)?;
    }
    let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(&path).map_err(LifecycleLockError::Io)?;

    match file.try_lock() {
        Ok(()) => Ok(LifecycleLock { _file: file }),
        Err(std::fs::TryLockError::WouldBlock) => Err(LifecycleLockError::AlreadyRunning),
        Err(std::fs::TryLockError::Error(err)) => Err(LifecycleLockError::Io(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_acquire_on_the_same_manifest_path_fails_while_the_first_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("segment-manifest.json");
        let first = acquire(&manifest_path).unwrap();
        let second = acquire(&manifest_path);
        assert!(matches!(second, Err(LifecycleLockError::AlreadyRunning)));
        drop(first);
    }

    #[test]
    fn after_the_first_lock_is_dropped_a_new_acquire_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("segment-manifest.json");
        let first = acquire(&manifest_path).unwrap();
        drop(first);
        assert!(acquire(&manifest_path).is_ok());
    }

    #[test]
    fn two_different_manifest_paths_in_different_directories_can_be_locked_concurrently() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let lock_a = acquire(&dir_a.path().join("segment-manifest.json"));
        let lock_b = acquire(&dir_b.path().join("segment-manifest.json"));
        assert!(lock_a.is_ok());
        assert!(lock_b.is_ok());
    }

    #[test]
    fn killing_the_process_holding_the_lifecycle_lock_releases_it_for_a_fresh_acquire() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("segment-manifest.json");

        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(&exe).args(["--exact", "lifecycle_lock::tests::__lifecycle_lock_holder_child_helper", "--ignored", "--nocapture"]).env("SUPERBRAIN_LIFECYCLE_LOCK_HOLDER_MANIFEST_PATH", &manifest_path).spawn().expect("failed to spawn the lifecycle-lock-holder child process");

        std::thread::sleep(std::time::Duration::from_millis(500));

        let blocked = acquire(&manifest_path);
        assert!(matches!(blocked, Err(LifecycleLockError::AlreadyRunning)), "the child process must still hold the lock at this point, got {blocked:?}");

        child.kill().expect("failed to kill the lifecycle-lock-holder child process");
        child.wait().expect("failed to reap the killed child process");

        let mut acquired = false;
        for _ in 0..50 {
            if acquire(&manifest_path).is_ok() {
                acquired = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(acquired, "a fresh acquire must succeed shortly after the lock-holding process is killed, proving the OS releases the lock on process termination without any cleanup code running");
    }

    #[test]
    #[ignore]
    fn __lifecycle_lock_holder_child_helper() {
        let manifest_path = std::path::PathBuf::from(std::env::var("SUPERBRAIN_LIFECYCLE_LOCK_HOLDER_MANIFEST_PATH").expect("missing SUPERBRAIN_LIFECYCLE_LOCK_HOLDER_MANIFEST_PATH"));
        let _lock = acquire(&manifest_path).expect("child helper must be able to acquire the lock");
        std::thread::sleep(std::time::Duration::from_secs(120));
    }
}
