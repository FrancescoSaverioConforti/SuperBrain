//! M7.3, third correction: a cross-process, OS-released, non-blocking
//! exclusive lock serializing `run_once` invocations that share the same
//! bridge state (registration + checkpoint) — reusing the exact
//! primitive `storage::audit_writer`'s own single-writer lock already
//! proves safe in this codebase (`std::fs::File::try_lock()`, the
//! stdlib's cross-platform advisory lock), rather than inventing a new
//! locking protocol.
//!
//! **Why this exists**: without it, two concurrent `run_once` calls
//! against the same `registration_path`/`checkpoint_path` could both
//! pass through `resolve_registration`'s "no registration yet, mint
//! fresh" branch before either reaches the destination vault's own
//! writer lock — each writing a DIFFERENT fresh `SourceLogId` to the
//! same file, racing non-atomically. Only one process can ever hold the
//! destination writer lock, so no actual duplicate Brain fact results,
//! but the registration file itself could end up out of sync with what
//! the destination chain reflects. This lock closes that window
//! structurally: the ENTIRE critical section (registration resolve
//! through checkpoint/registration save) now runs under one process-
//! wide, cross-process exclusive lock.
//!
//! **Why a lock FILE, not a bare mutex**: a `std::sync::Mutex` only
//! serializes threads within one process; `run_once` can legitimately be
//! invoked from separate OS processes (a scheduler re-running the bridge
//! binary, for instance) pointed at the same state directory. The lock
//! must work across process boundaries.
//!
//! **Why `try_lock`, never a blocking wait**: the frozen requirement is
//! fail-closed, not fail-slow — a second concurrent run must be refused
//! immediately ([`BridgeLockError::AlreadyRunning`]), never left waiting
//! indefinitely for the first to finish.
//!
//! **Crash safety**: `std::fs::File::try_lock()` is an OS-level advisory
//! lock tied to the open file handle/descriptor, not a marker written
//! INTO the file's own content — the OS itself releases it the instant
//! the holding process exits for ANY reason (normal return, panic, or
//! being killed), including a hard crash. This module never checks for
//! "does a lock file exist" as its own signal (that would be exactly the
//! stale-marker failure mode the frozen correction explicitly warns
//! against) — the file's mere presence on disk is irrelevant; only the
//! OS-level lock state on that open handle matters, and `try_lock`
//! queries that directly.
//!
//! **Lock ownership and path**: the lock file lives at a path derived
//! INTERNALLY by this module from the bridge's own state directory
//! (specifically, a sibling of `checkpoint_path`, inside the same
//! directory the composition root already controls for its other bridge
//! state files) — never a second, independently caller-supplied path
//! that could point anywhere.

/// A held cross-process exclusive lock over one bridge state directory.
/// Keeping this alive (not dropped) IS what holds the lock; dropping it
/// (including via an early return — this type's `Drop` runs
/// unconditionally) releases it immediately. The file handle is kept
/// open for exactly this purpose and is never read from or written to.
#[derive(Debug)]
pub struct BridgeLock {
    _file: std::fs::File,
}

#[derive(Debug, thiserror::Error)]
pub enum BridgeLockError {
    /// Another `run_once` (in this process or another) already holds the
    /// lock for this exact state directory — fail closed immediately,
    /// never waited out.
    #[error("another bridge run is already in progress for this state directory")]
    AlreadyRunning,
    #[error("failed to open bridge lock file: {0}")]
    Io(std::io::Error),
}

/// The lock file's name, always a fixed sibling of `checkpoint_path` —
/// never a name a caller supplies, so there is no caller-controlled
/// arbitrary path for this lock.
const LOCK_FILE_NAME: &str = ".mcp-audit-bridge.lock";

/// Derives this bridge state directory's own lock file path from
/// `checkpoint_path` — a fixed, internally-chosen sibling file name, not
/// a second independently configurable path. Using `checkpoint_path`'s
/// own parent directory (rather than, say, `registration_path`'s) is an
/// arbitrary but fixed choice: what matters is that every `run_once`
/// call against the SAME state directory derives the SAME lock path,
/// which holds as long as a composition root keeps its registration and
/// checkpoint files in the same directory (the normal case; see
/// `RunConfig`'s own fields).
fn lock_path_for(checkpoint_path: &std::path::Path) -> std::path::PathBuf {
    let dir = checkpoint_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    dir.join(LOCK_FILE_NAME)
}

/// Attempts to acquire the exclusive bridge lock for the state directory
/// containing `checkpoint_path`. Never blocks: returns
/// [`BridgeLockError::AlreadyRunning`] immediately if another process
/// already holds it. On success, the returned [`BridgeLock`] must be
/// held for the ENTIRE critical section (registration resolve through
/// final checkpoint/registration save) and only dropped after the
/// destination worker has been shut down and all state is durably
/// persisted — see `crate::run::run_once`'s own call site for the exact
/// scope.
pub fn acquire(checkpoint_path: &std::path::Path) -> Result<BridgeLock, BridgeLockError> {
    let path = lock_path_for(checkpoint_path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(BridgeLockError::Io)?;
    }
    let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(&path).map_err(BridgeLockError::Io)?;

    match file.try_lock() {
        Ok(()) => Ok(BridgeLock { _file: file }),
        Err(std::fs::TryLockError::WouldBlock) => Err(BridgeLockError::AlreadyRunning),
        Err(std::fs::TryLockError::Error(err)) => Err(BridgeLockError::Io(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_acquire_on_the_same_checkpoint_path_fails_while_the_first_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let checkpoint_path = dir.path().join("checkpoint.json");
        let first = acquire(&checkpoint_path).unwrap();
        let second = acquire(&checkpoint_path);
        assert!(matches!(second, Err(BridgeLockError::AlreadyRunning)), "got {second:?}");
        drop(first);
    }

    #[test]
    fn after_the_first_lock_is_dropped_a_new_acquire_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let checkpoint_path = dir.path().join("checkpoint.json");
        let first = acquire(&checkpoint_path).unwrap();
        drop(first);
        let second = acquire(&checkpoint_path);
        assert!(second.is_ok(), "got {second:?}");
    }

    #[test]
    fn two_different_checkpoint_paths_in_different_directories_can_be_locked_concurrently() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let lock_a = acquire(&dir_a.path().join("checkpoint.json"));
        let lock_b = acquire(&dir_b.path().join("checkpoint.json"));
        assert!(lock_a.is_ok());
        assert!(lock_b.is_ok());
    }

    #[test]
    fn the_lock_survives_the_file_remaining_on_disk_after_release() {
        // The frozen requirement: presence of the lock FILE on disk must
        // never, by itself, be treated as "locked" -- only the live OS
        // lock state does. This test holds, releases, and re-acquires
        // while leaving the file itself in place throughout, proving the
        // module never consults "does the file exist" as its own signal.
        let dir = tempfile::tempdir().unwrap();
        let checkpoint_path = dir.path().join("checkpoint.json");
        let lock_path = lock_path_for(&checkpoint_path);

        let first = acquire(&checkpoint_path).unwrap();
        drop(first);
        assert!(lock_path.exists(), "the lock file itself is expected to remain on disk after release");

        let second = acquire(&checkpoint_path);
        assert!(second.is_ok(), "a stale-but-unlocked file on disk must not block a fresh acquire, got {second:?}");
    }

    // --- real cross-process reproduction, same pattern as
    //     storage::audit_writer::tests::killing_the_process_holding_the_lock_releases_it_for_a_fresh_acquire:
    //     re-invoke this SAME test binary as a child process that holds
    //     the lock, then kill it without any chance to run cleanup code,
    //     and prove the OS itself released it. ---

    #[test]
    fn killing_the_process_holding_the_bridge_lock_releases_it_for_a_fresh_acquire() {
        let dir = tempfile::tempdir().unwrap();
        let checkpoint_path = dir.path().join("checkpoint.json");

        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(&exe).args(["--exact", "bridge_lock::tests::__bridge_lock_holder_child_helper", "--ignored", "--nocapture"]).env("SUPERBRAIN_BRIDGE_LOCK_HOLDER_CHECKPOINT_PATH", &checkpoint_path).spawn().expect("failed to spawn the bridge-lock-holder child process");

        std::thread::sleep(std::time::Duration::from_millis(500));

        let blocked = acquire(&checkpoint_path);
        assert!(matches!(blocked, Err(BridgeLockError::AlreadyRunning)), "the child process must still hold the lock at this point, got {blocked:?}");

        child.kill().expect("failed to kill the bridge-lock-holder child process");
        child.wait().expect("failed to reap the killed child process");

        let mut acquired = false;
        for _ in 0..50 {
            if acquire(&checkpoint_path).is_ok() {
                acquired = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(acquired, "a fresh acquire must succeed shortly after the lock-holding process is killed, proving the OS releases the lock on process termination without any cleanup code running");
    }

    /// Not a real test: a helper entry point re-invoked as a child
    /// process (via `--exact ... --ignored`) by the test above, to hold
    /// the bridge lock indefinitely so the parent can kill it without
    /// warning. `#[ignore]` keeps it out of the normal test run.
    #[test]
    #[ignore]
    fn __bridge_lock_holder_child_helper() {
        let checkpoint_path = std::path::PathBuf::from(std::env::var("SUPERBRAIN_BRIDGE_LOCK_HOLDER_CHECKPOINT_PATH").expect("missing SUPERBRAIN_BRIDGE_LOCK_HOLDER_CHECKPOINT_PATH"));
        let _lock = acquire(&checkpoint_path).expect("child helper must be able to acquire the lock");
        std::thread::sleep(std::time::Duration::from_secs(120));
    }
}
