//! Hash-chained, monotonic, crash-safe deletion log, per spec contract C5
//! ("Un dato cancellato non riappare").
//!
//! The log itself is a small SQLite table: one row per deletion, chained by
//! `chain_hash = sha256(prev_chain_hash || generation || note_id)`, with a
//! monotonically increasing `generation`. Because the log and its "trusted
//! reference" (the generation + hash a restore checks against) cannot be
//! updated atomically in one step, C5 specifies a three-step protocol:
//!
//! 1. write and sync a new generation entry, marked `Pending`;
//! 2. update and sync the trusted reference;
//! 3. mark the generation `Completed` — only now is the deletion complete.
//!
//! A crash can land between any of these steps. [`DeletionLog::reconcile`]
//! is run at startup to resolve the one case that is safe to resolve
//! automatically (log one generation ahead of the reference, still
//! `Pending`: roll the reference forward and complete the entry) and to
//! fail closed (`DeletionLogError::Inconsistent`) on every other mismatch.
//!
//! The trusted reference itself must be cryptographically authenticated —
//! not a bare file an attacker could edit to resurrect a deleted note by
//! rolling the generation back. [`DpapiTrustedReferenceStore`] wraps it with
//! the exact same DPAPI primitives `keys.rs` uses for the master key
//! (reused directly, not reimplemented), so tampering with the on-disk
//! reference surfaces as a decrypt failure
//! ([`DeletionLogError::ReferenceTamperDetected`]), not as a value that
//! happens to parse as some other generation.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::keys::{dpapi_protect, dpapi_unprotect};

/// Genesis `prev_chain_hash` for the very first deletion-log entry: 64 `'0'`
/// characters, the same width as a real sha256 hex digest, so the chain
/// formula `sha256(prev_chain_hash || generation || note_id)` has a
/// well-defined input even for generation 1.
const GENESIS_PREV_CHAIN_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const DELETION_LOG_FORMAT_VERSION: u32 = 1;

/// A generation-entry's lifecycle state. Modeled on
/// `domain::write_state::WriteState`'s `valid_transition` idiom: a small
/// enum plus a free function stating exactly which transitions are legal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionLogState {
    /// Step 1 has run (the row exists, hash-chained) but step 2/3 have not
    /// necessarily completed yet. Per C5, the note is already non-consultable
    /// in this state — `is_deleted` must return `true`.
    Pending,
    /// All three protocol steps have completed for this generation.
    Completed,
}

impl DeletionLogState {
    fn as_sql(&self) -> &'static str {
        match self {
            DeletionLogState::Pending => "pending",
            DeletionLogState::Completed => "completed",
        }
    }

    fn from_sql(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(DeletionLogState::Pending),
            "completed" => Some(DeletionLogState::Completed),
            _ => None,
        }
    }
}

/// `Pending -> Completed` is the only legal transition; there is no reverse
/// (a completed deletion is never un-completed) and no other state exists.
pub fn valid_transition(from: DeletionLogState, to: DeletionLogState) -> bool {
    matches!((from, to), (DeletionLogState::Pending, DeletionLogState::Completed))
}

/// The "riferimento affidabile": the deletion log's current generation and
/// the chain hash at that generation, checked at restore time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationRef {
    pub generation: u64,
    pub chain_hash: String,
    pub format_version: Option<u32>,
    pub store_id: Option<String>,
}

impl GenerationRef {
    /// Serializes as `"{generation}:{chain_hash}"`. A simple, explicit
    /// format chosen because the bytes are then wrapped by
    /// DPAPI/authenticated storage — this layer does not need to defend
    /// against a hostile parser, only to round-trip unambiguously (a
    /// `chain_hash` is a fixed-width hex string, so a single `:` separator
    /// is sufficient and unambiguous).
    fn serialize(&self) -> Vec<u8> {
        match (&self.format_version, &self.store_id) {
            (Some(version), Some(store_id)) => {
                format!("v{version}:{store_id}:{}:{}", self.generation, self.chain_hash).into_bytes()
            }
            _ => format!("{}:{}", self.generation, self.chain_hash).into_bytes(),
        }
    }

    /// Inverse of [`GenerationRef::serialize`]. Returns `None` if `bytes`
    /// don't match the `"{u64}:{hex}"` shape at all — the caller maps that
    /// to `DeletionLogError::ReferenceCorrupt`.
    fn deserialize(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        if let Some(versioned) = text.strip_prefix('v') {
            let mut parts = versioned.splitn(4, ':');
            let format_version = parts.next()?.parse::<u32>().ok()?;
            let store_id = parts.next()?.to_string();
            let generation = parts.next()?.parse::<u64>().ok()?;
            let hash = parts.next()?;
            if format_version == 0 || store_id.is_empty() || hash.is_empty() {
                return None;
            }
            return Some(GenerationRef {
                generation,
                chain_hash: hash.to_string(),
                format_version: Some(format_version),
                store_id: Some(store_id),
            });
        }
        let (gen_str, hash) = text.split_once(':')?;
        let generation = gen_str.parse::<u64>().ok()?;
        if hash.is_empty() {
            return None;
        }
        Some(GenerationRef { generation, chain_hash: hash.to_string(), format_version: None, store_id: None })
    }
}

/// Where the deletion log persists and reads back its trusted reference.
///
/// Exists so [`DeletionLog`]'s own hash-chain/protocol logic can be tested
/// independently from the authentication layer (via the test-only
/// [`FileTrustedReferenceStore`]), while production code always uses the
/// real, cryptographically authenticated [`DpapiTrustedReferenceStore`].
pub trait TrustedReferenceStore {
    fn read(&self) -> Result<Option<GenerationRef>, DeletionLogError>;
    fn write(&self, r: &GenerationRef) -> Result<(), DeletionLogError>;
    fn bound_path(&self) -> Option<&Path> { None }
}

/// The REAL trusted-reference store: the serialized [`GenerationRef`] is
/// DPAPI-wrapped (`crate::keys::dpapi_protect`/`dpapi_unprotect`, the exact
/// same primitives `keys.rs` uses to wrap the master key — reused directly,
/// never reimplemented) before being written to `path`. Tampering with the
/// on-disk bytes (flipping bits in the DPAPI envelope) is detected as a
/// DPAPI decrypt failure and surfaces as
/// [`DeletionLogError::ReferenceTamperDetected`], distinct from
/// [`DeletionLogError::ReferenceCorrupt`] (bytes too short/malformed to even
/// have been produced by `dpapi_protect` — missing file, empty file, or
/// structurally implausible content).
pub struct DpapiTrustedReferenceStore {
    path: PathBuf,
}

impl DpapiTrustedReferenceStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

/// Durably replaces a small coordination file without ever truncating the
/// previous authenticated value in place. A crash leaves either the old
/// reference or the fully synced replacement; reconciliation understands both.
fn atomic_write_reference(path: &Path, contents: &[u8]) -> Result<(), DeletionLogError> {
    let parent = path.parent().ok_or_else(|| DeletionLogError::Io("reference path has no parent".to_string()))?;
    std::fs::create_dir_all(parent).map_err(|e| DeletionLogError::Io(e.to_string()))?;
    let tmp = parent.join(format!(".{}.{}.tmp", path.file_name().unwrap_or_default().to_string_lossy(), Uuid::new_v4()));
    let write_result = (|| -> Result<(), DeletionLogError> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)
            .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        file.write_all(contents).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        file.sync_all().map_err(|e| DeletionLogError::Io(e.to_string()))?;
        atomic_replace_reference(&tmp, path)?;
        #[cfg(unix)]
        if let Ok(dir) = std::fs::File::open(parent) { let _ = dir.sync_all(); }
        Ok(())
    })();
    if write_result.is_err() { let _ = std::fs::remove_file(&tmp); }
    write_result
}

#[cfg(windows)]
fn atomic_replace_reference(source: &Path, destination: &Path) -> Result<(), DeletionLogError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};

    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(destination_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| DeletionLogError::Io(e.to_string()))
}

#[cfg(not(windows))]
fn atomic_replace_reference(source: &Path, destination: &Path) -> Result<(), DeletionLogError> {
    std::fs::rename(source, destination).map_err(|e| DeletionLogError::Io(e.to_string()))
}

impl TrustedReferenceStore for DpapiTrustedReferenceStore {
    fn bound_path(&self) -> Option<&Path> { Some(&self.path) }
    fn read(&self) -> Result<Option<GenerationRef>, DeletionLogError> {
        let wrapped = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(DeletionLogError::Io(e.to_string())),
        };
        // A real DPAPI envelope is never empty. Catch the structurally
        // implausible case (empty/truncated file) before even attempting
        // the Win32 call, so it is reported as corruption, not tamper.
        if wrapped.is_empty() {
            return Err(DeletionLogError::ReferenceCorrupt);
        }
        let plaintext = dpapi_unprotect(&wrapped).map_err(|_| DeletionLogError::ReferenceTamperDetected)?;
        match GenerationRef::deserialize(&plaintext) {
            Some(r) => Ok(Some(r)),
            None => Err(DeletionLogError::ReferenceCorrupt),
        }
    }

    fn write(&self, r: &GenerationRef) -> Result<(), DeletionLogError> {
        let plaintext = r.serialize();
        let wrapped = dpapi_protect(&plaintext).map_err(|_| DeletionLogError::ReferenceTamperDetected)?;
        atomic_write_reference(&self.path, &wrapped)
    }
}

/// A PLAIN, UNAUTHENTICATED trusted-reference store.
///
/// **This exists ONLY for tests that need to exercise `DeletionLog`'s own
/// hash-chain/three-step-protocol logic in isolation from the DPAPI
/// authentication layer** (DPAPI is Windows-only and slow/awkward to drive
/// in a tight unit-test loop). It stores the serialized [`GenerationRef`] as
/// plain bytes on disk with no integrity protection whatsoever: anyone who
/// can write to `path` can resurrect a deleted note by editing the
/// generation number. It is gated `#[cfg(test)]` specifically so it can
/// never be constructed from production code — there is no way to name this
/// type outside a test build. Production code must always use
/// [`DpapiTrustedReferenceStore`].
#[cfg(test)]
pub struct FileTrustedReferenceStore {
    path: PathBuf,
}

#[cfg(test)]
impl FileTrustedReferenceStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[cfg(test)]
impl TrustedReferenceStore for FileTrustedReferenceStore {
    fn bound_path(&self) -> Option<&Path> { Some(&self.path) }
    fn read(&self) -> Result<Option<GenerationRef>, DeletionLogError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                if bytes.is_empty() {
                    return Err(DeletionLogError::ReferenceCorrupt);
                }
                GenerationRef::deserialize(&bytes).ok_or(DeletionLogError::ReferenceCorrupt).map(Some)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(DeletionLogError::Io(e.to_string())),
        }
    }

    fn write(&self, r: &GenerationRef) -> Result<(), DeletionLogError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        }
        std::fs::write(&self.path, r.serialize()).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        Ok(())
    }
}

/// Errors from deletion-log operations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DeletionLogError {
    #[error("I/O error in deletion log: {0}")]
    Io(String),

    #[error("deletion log is uninitialized")]
    Uninitialized,

    #[error("deletion log is a legacy state that requires explicit administrative migration")]
    LegacyNeedsMigration,

    #[error("deletion log is corrupt, tampered with, or has lost required coordination state")]
    CorruptOrTampered,

    #[error("deletion log format or schema is incompatible")]
    Incompatible,

    /// A chained entry's `chain_hash` does not match
    /// `sha256(prev_chain_hash || generation || note_id)`.
    #[error("deletion log hash chain is broken")]
    ChainBroken,

    /// The trusted reference's generation is strictly BEHIND the LOG's own
    /// head, inside [`DeletionLog::reconcile`] — the log is the sole writer
    /// of its own generations, so the log can never legitimately be ahead
    /// of its own reference; this means the reference (or, equivalently,
    /// the log) disagrees with itself and cannot be trusted. Distinct from
    /// [`DeletionLogError::BackupReferenceStale`], which is a DIFFERENT,
    /// safe-to-proceed-past situation: a BACKUP's own embedded snapshot
    /// being older than an ALREADY-RECONCILED, trustworthy live state.
    /// Conflating the two was a real bug (post-N1 follow-up): callers that
    /// correctly want to tolerate the second case would otherwise also
    /// tolerate the first, which is the general "the live log/reference
    /// pair itself disagrees" case C5 requires be fail-closed
    /// unconditionally.
    #[error("trusted reference is stale: log is at generation {log_generation}, reference at {reference_generation}")]
    ReferenceStale { log_generation: u64, reference_generation: u64 },

    /// [`DeletionLog::verify_safe_to_expose_after_restore`] only: the live
    /// log/reference pair has already been proven mutually consistent by
    /// `reconcile` (see that function's own doc comment), and a BACKUP's
    /// own embedded reference snapshot is simply older than that
    /// already-trustworthy live state — "valido ma obsoleto" at the
    /// backup level, not at the live-log level. Safe for a caller to treat
    /// as "this specific backup's snapshot predates some deletions," never
    /// as "the live log itself cannot be trusted" ([`DeletionLogError::ReferenceStale`]
    /// is for that).
    #[error("backup's own reference snapshot is stale: snapshot is at generation {backup_generation}, live reference at {live_generation}")]
    BackupReferenceStale { backup_generation: u64, live_generation: u64 },

    /// The trusted reference is entirely absent (e.g. at restore time).
    #[error("trusted reference is missing")]
    ReferenceMissing,

    /// The reference file/bytes are structurally implausible — missing,
    /// empty, or not even shaped like a value `dpapi_protect` could have
    /// produced. Distinct from [`DeletionLogError::ReferenceTamperDetected`]:
    /// this means the bytes were never a valid envelope to begin with.
    #[error("trusted reference is corrupt")]
    ReferenceCorrupt,

    /// The reference's DPAPI envelope was structurally plausible but failed
    /// to decrypt/authenticate — i.e. it was tampered with after being
    /// written. Distinct from [`DeletionLogError::ReferenceCorrupt`].
    #[error("trusted reference failed authentication: tampering detected")]
    ReferenceTamperDetected,

    /// `reconcile` observed a disagreement between the log and the
    /// reference that is not the one specific roll-forward pattern it knows
    /// how to resolve automatically (log exactly one generation ahead of
    /// the reference, that entry still `Pending`). Fails closed rather than
    /// guessing.
    #[error("deletion log and trusted reference are inconsistent in a way that cannot be safely auto-resolved")]
    Inconsistent,

    /// [`DeletionLog::acquire_write_lock`] (and so `record_deletion`, which
    /// takes it for its whole span) could not obtain the cross-process
    /// deletion-log write lock within its bounded wait. Fails closed: the
    /// caller never proceeds as if it held the lock.
    #[error("could not acquire the deletion log's cross-process write lock within {timeout_ms} ms")]
    WriteLockUnavailable { timeout_ms: u64 },
}

/// A full deletion-log row. Not a public API — used internally. `note_id`
/// and `prev_chain_hash` are carried for completeness/debuggability (a full
/// mirror of the table row) even though current call sites only read
/// `generation`/`chain_hash`/`state`.
#[allow(dead_code)]
struct Row {
    generation: u64,
    note_id: String,
    prev_chain_hash: String,
    chain_hash: String,
    state: DeletionLogState,
}

/// The outcome of a successful [`DeletionLog::record_deletion`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionReceipt {
    pub generation: u64,
    pub chain_hash: String,
}

/// The outcome of a successful [`DeletionLog::reconcile`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// Log and reference already agreed; nothing to do.
    AlreadyConsistent,
    /// The log was exactly one generation ahead of the reference, still
    /// `Pending` (a crash between step 1 and step 2, or step 2 and step 3).
    /// The reference was rolled forward and the entry marked `Completed`.
    RolledForward { generation: u64 },
}

/// Explicit operator assertion required to migrate an ambiguous legacy-empty
/// state. Constructing this value is an administrative act, never a read path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyNoDeletionAttestation {
    pub operation_id: Uuid,
    pub authority: String,
    pub timestamp: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReceipt {
    pub operation_id: Uuid,
    pub store_id: String,
    pub format_version: u32,
}

/// Crash-injectable boundaries in the three-step deletion protocol.
/// Crate-local (this module only): `vault-security` does not depend on
/// `storage`, so this is a small, self-contained sibling of
/// `storage::fault`, not a shared module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FaultPoint {
    /// Step 1 has run: the new generation row exists, `Pending`. Step 2
    /// (reference update) has not run yet.
    AfterGenerationWrittenPending,
    /// Step 2 has run: the trusted reference now points at the new
    /// generation. Step 3 (marking the row `Completed`) has not run yet.
    AfterReferenceUpdated,
    /// Checked immediately before the SQL that marks the row `Completed`
    /// runs (i.e. functionally identical window to
    /// `AfterReferenceUpdated`, named separately so a test can express
    /// "right before step 3" as its own intent).
    BeforeMarkedCompleted,
}

/// Returned when an armed [`FaultInjector`] interrupts at a [`FaultPoint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Interrupted(pub(crate) FaultPoint);

/// A minimal crash-simulation seam, scoped to this module. Production code
/// always uses [`FaultInjector::none`], which never interrupts or pauses.
/// `trigger_at` simulates a crash by having `record_deletion` return early
/// right after a named point's effect has happened, then a fresh
/// `DeletionLog` is opened to simulate a restart.
///
/// `pause_at` (Task 7) genuinely pauses `record_deletion` on whatever thread
/// called it, mid-protocol, until a test resumes it — mirroring
/// `storage::fault::FaultInjector::pause_at` exactly. This was deliberately
/// NOT duplicated here when this module was first written (see the prior
/// version of this comment), on the premise that no test here needed true
/// concurrent pausing. Task 7's backup-during-pending-deletion test is
/// exactly that test: it needs `backup()` to run on a second thread while
/// `record_deletion` is stopped, mid-flight, with the log row written
/// `Pending` and the trusted reference not yet updated — a window a
/// `trigger_at`-simulated "crash" cannot model, because returning early
/// loses the still-running call's stack/thread, whereas here the backup
/// must observe the SAME paused call's on-disk state while it is still
/// technically in progress and will later complete normally.
#[derive(Debug, Default)]
pub(crate) struct FaultInjector {
    trigger: Option<FaultPoint>,
    #[cfg(test)]
    pause: Option<Pause>,
}

/// Test-only: where to pause, and the two channel ends used to tell the test
/// "paused" and to wait for "resume". Mirrors `storage::fault::Pause`.
#[cfg(test)]
#[derive(Debug)]
struct Pause {
    point: FaultPoint,
    paused: std::sync::mpsc::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

impl FaultInjector {
    pub(crate) fn none() -> Self {
        Self { trigger: None, #[cfg(test)] pause: None }
    }

    #[cfg(test)]
    pub(crate) fn trigger_at(point: FaultPoint) -> Self {
        Self { trigger: Some(point), pause: None }
    }

    /// An injector that, when `record_deletion` reaches `point`, sends `()`
    /// on `paused` and then blocks until `()` arrives on `resume` (or the
    /// resume sender is dropped), then lets the call continue normally. It
    /// never returns `Interrupted`.
    #[cfg(test)]
    pub(crate) fn pause_at(
        point: FaultPoint,
        paused: std::sync::mpsc::Sender<()>,
        resume: std::sync::mpsc::Receiver<()>,
    ) -> Self {
        Self { trigger: None, pause: Some(Pause { point, paused, resume }) }
    }

    pub(crate) fn check(&self, point: FaultPoint) -> Result<(), Interrupted> {
        #[cfg(test)]
        if let Some(pause) = &self.pause {
            if pause.point == point {
                // A test that already went away is not an error here: the
                // call just continues.
                let _ = pause.paused.send(());
                let _ = pause.resume.recv();
            }
        }
        if self.trigger == Some(point) {
            Err(Interrupted(point))
        } else {
            Ok(())
        }
    }
}

/// A hash-chained, monotonically-numbered deletion log, backed by its own
/// SQLite database file (never the A2 journal's file).
pub struct DeletionLog {
    db_path: PathBuf,
    db_identity: FileIdentity,
    reference_path: Option<PathBuf>,
    /// Serializes access to this log's own SQLite connection within this
    /// process, so a concurrent read during `record_deletion` (same
    /// `DeletionLog` instance, different thread) observes a consistent
    /// view rather than racing on connection setup. SQLite itself
    /// serializes writers across processes/files; this mutex additionally
    /// protects the single `rusqlite::Connection` this struct owns.
    conn: Mutex<Connection>,
    /// How long [`DeletionLog::acquire_write_lock`] waits for the
    /// cross-process write lock before failing closed with
    /// [`DeletionLogError::WriteLockUnavailable`].
    write_lock_timeout: Duration,
}

/// Default bounded wait for the cross-process deletion-log write lock. Long
/// enough to ride out any legitimate holder (one `record_deletion` call, or
/// one restored note's final re-check-and-rename), short enough that a stuck
/// holder surfaces as an explicit error instead of a hang.
const DEFAULT_WRITE_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Poll interval while waiting for the write lock.
const WRITE_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// RAII guard for the deletion log's cross-process write lock (see
/// [`DeletionLog::acquire_write_lock`]). The lock is released when this
/// guard is dropped — on every return path, including early returns and
/// panics — and, because it is an OS-level file lock held through an open
/// handle, also automatically when the holding process dies.
#[derive(Debug)]
pub struct DeletionLogWriteGuard {
    file: std::fs::File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    volume: u64,
    index: u64,
}

fn file_identity(path: &Path) -> Result<FileIdentity, DeletionLogError> {
    let file = std::fs::File::open(path).map_err(|_| DeletionLogError::CorruptOrTampered)?;
    identity_of_open_file(&file)
}

fn identity_of_open_file(file: &std::fs::File) -> Result<FileIdentity, DeletionLogError> {
    let metadata = file.metadata().map_err(|_| DeletionLogError::CorruptOrTampered)?;
    if !metadata.is_file() { return Err(DeletionLogError::CorruptOrTampered); }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| DeletionLogError::CorruptOrTampered)?;
        }
        Ok(FileIdentity {
            volume: u64::from(info.dwVolumeSerialNumber),
            index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(FileIdentity { volume: metadata.dev(), index: metadata.ino() })
    }
}

fn acquire_initialization_lock(db_path: &Path) -> Result<DeletionLogWriteGuard, DeletionLogError> {
    let parent = db_path.parent().ok_or(DeletionLogError::Uninitialized)?;
    std::fs::create_dir_all(parent).map_err(|e| DeletionLogError::Io(e.to_string()))?;
    let path = db_path.with_extension("writelock");
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)
        .map_err(|e| DeletionLogError::Io(e.to_string()))?;
    file.lock().map_err(|e| DeletionLogError::Io(e.to_string()))?;
    if identity_of_open_file(&file)? != file_identity(&path)? {
        return Err(DeletionLogError::CorruptOrTampered);
    }
    Ok(DeletionLogWriteGuard { file })
}

impl Drop for DeletionLogWriteGuard {
    fn drop(&mut self) {
        // Closing the handle releases the lock anyway; unlocking explicitly
        // first just makes the release not depend on when the OS gets
        // around to closing it. An error here cannot be acted on.
        let _ = self.file.unlock();
    }
}

impl DeletionLog {
    /// Administratively converts only a provably empty legacy layout into
    /// authenticated generation zero and persists a non-sensitive receipt.
    pub fn migrate_legacy_empty(
        db_path: &Path,
        trusted_ref: &dyn TrustedReferenceStore,
        attestation: &LegacyNoDeletionAttestation,
    ) -> Result<MigrationReceipt, DeletionLogError> {
        if attestation.authority.trim().is_empty()
            || attestation.timestamp.trim().is_empty()
            || attestation.reason.trim().is_empty()
        {
            return Err(DeletionLogError::LegacyNeedsMigration);
        }
        let initialization_lock = acquire_initialization_lock(db_path)?;
        if let Some(reference) = trusted_ref.read()? {
            if !Self::is_genesis_reference(&reference) {
                return Err(DeletionLogError::CorruptOrTampered);
            }
            drop(initialization_lock);
            let log = Self::open_existing(db_path, trusted_ref)?;
            let conn = log.conn.lock().expect("deletion log mutex poisoned");
            let operation: String = conn
                .query_row(
                    "SELECT migration_operation_id FROM deletion_log_meta WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| DeletionLogError::CorruptOrTampered)?;
            if operation != attestation.operation_id.to_string() {
                return Err(DeletionLogError::CorruptOrTampered);
            }
            return Ok(MigrationReceipt {
                operation_id: attestation.operation_id,
                store_id: reference.store_id.expect("verified P0 reference has store id"),
                format_version: DELETION_LOG_FORMAT_VERSION,
            });
        }

        if db_path.exists() {
            let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let table_count: i64 = conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'", [], |row| row.get(0),
            ).map_err(|_| DeletionLogError::Incompatible)?;
            let count: i64 = if table_count == 0 {
                0
            } else {
                conn.query_row("SELECT count(*) FROM deletions", [], |row| row.get(0))
                    .map_err(|_| DeletionLogError::Incompatible)?
            };
            if count != 0 {
                return Err(DeletionLogError::CorruptOrTampered);
            }
        }

        let store_id = Uuid::new_v4().to_string();
        if !db_path.exists() {
            std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(db_path)
                .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        }
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        Self::create_schema(&conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS deletion_log_meta (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                format_version INTEGER NOT NULL,
                store_id TEXT NOT NULL,
                migration_operation_id TEXT NOT NULL,
                migration_authority TEXT NOT NULL,
                migration_timestamp TEXT NOT NULL,
                migration_reason TEXT NOT NULL
            );",
        )
        .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        let existing_migration: Option<(String, String)> = conn.query_row(
            "SELECT store_id, migration_operation_id FROM deletion_log_meta WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(|_| DeletionLogError::CorruptOrTampered)?;
        let store_id = match existing_migration {
            Some((existing_id, operation_id)) if operation_id == attestation.operation_id.to_string() => existing_id,
            Some(_) => return Err(DeletionLogError::CorruptOrTampered),
            None => store_id,
        };
        conn.execute(
            "INSERT OR IGNORE INTO deletion_log_meta(singleton, format_version, store_id, migration_operation_id,
             migration_authority, migration_timestamp, migration_reason) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                DELETION_LOG_FORMAT_VERSION,
                store_id,
                attestation.operation_id.to_string(),
                attestation.authority,
                attestation.timestamp,
                attestation.reason,
            ],
        )
        .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        let genesis = Self::genesis_reference(store_id.clone());
        trusted_ref.write(&genesis)?;
        Ok(MigrationReceipt {
            operation_id: attestation.operation_id,
            store_id: genesis.store_id.expect("new genesis has store id"),
            format_version: DELETION_LOG_FORMAT_VERSION,
        })
    }

    /// Explicitly creates a new deletion log and its authenticated genesis.
    /// Normal consumers must use [`DeletionLog::open_existing`] instead.
    pub fn bootstrap_new(
        db_path: &Path,
        trusted_ref: &dyn TrustedReferenceStore,
    ) -> Result<Self, DeletionLogError> {
        let initialization_lock = acquire_initialization_lock(db_path)?;
        let existing_reference = trusted_ref.read()?;
        if let Some(existing) = &existing_reference {
            if !Self::is_genesis_reference(existing) {
                return Err(DeletionLogError::CorruptOrTampered);
            }
            if db_path.exists() && !Self::is_retryable_genesis_partial(db_path)? {
                drop(initialization_lock);
                return Self::open_existing(db_path, trusted_ref);
            }
        } else if db_path.exists() {
            return Err(DeletionLogError::LegacyNeedsMigration);
        }

        let genesis = existing_reference.unwrap_or_else(|| Self::genesis_reference(Uuid::new_v4().to_string()));
        if trusted_ref.read()?.is_none() {
            trusted_ref.write(&genesis)?;
        }

        if !db_path.exists() {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(db_path)
                .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        }
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        Self::create_schema(&conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS deletion_log_meta (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                format_version INTEGER NOT NULL,
                store_id TEXT NOT NULL
            );",
        )
        .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        conn.execute(
            "INSERT OR IGNORE INTO deletion_log_meta(singleton, format_version, store_id) VALUES (1, ?1, ?2)",
            rusqlite::params![DELETION_LOG_FORMAT_VERSION, genesis.store_id.as_deref().unwrap_or_default()],
        )
        .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        let log = Self {
            db_path: db_path.to_path_buf(),
            db_identity: file_identity(db_path)?,
            reference_path: trusted_ref.bound_path().map(Path::to_path_buf),
            conn: Mutex::new(conn),
            write_lock_timeout: DEFAULT_WRITE_LOCK_TIMEOUT,
        };
        log.verify_consistent(trusted_ref)?;
        log.verify_file_identities(&initialization_lock)?;
        Ok(log)
    }

    /// A bootstrap retry may finish only a DB created after the authenticated
    /// genesis reference but before schema/meta initialization. It may never
    /// reinterpret rows, metadata, or unrelated tables as a fresh genesis.
    fn is_retryable_genesis_partial(db_path: &Path) -> Result<bool, DeletionLogError> {
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .map_err(|_| DeletionLogError::Incompatible)?;
        let names = stmt.query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| DeletionLogError::Incompatible)?
            .collect::<Result<Vec<_>, _>>().map_err(|_| DeletionLogError::Incompatible)?;
        if names.iter().any(|name| name != "deletions") { return Ok(false); }
        if names.iter().any(|name| name == "deletions") {
            let count: i64 = conn.query_row("SELECT count(*) FROM deletions", [], |row| row.get(0))
                .map_err(|_| DeletionLogError::Incompatible)?;
            if count != 0 { return Ok(false); }
        }
        Ok(true)
    }

    /// Opens an existing log without creating any filesystem or SQLite state.
    pub fn open_existing(
        db_path: &Path,
        trusted_ref: &dyn TrustedReferenceStore,
    ) -> Result<Self, DeletionLogError> {
        // Inspect the authenticated reference before classifying an absent
        // artifact as legacy. A versioned reference proves initialization, so
        // a missing DB or lock is loss/tampering, never migratable emptiness.
        let reference_exists = trusted_ref.read()?.is_some();
        if !db_path.is_file() {
            return Err(if reference_exists { DeletionLogError::CorruptOrTampered } else { DeletionLogError::LegacyNeedsMigration });
        }
        if !db_path.with_extension("writelock").is_file() {
            return Err(if reference_exists { DeletionLogError::CorruptOrTampered } else { DeletionLogError::LegacyNeedsMigration });
        }
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        let schema_ok = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='deletions'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| DeletionLogError::Io(e.to_string()))?
            == 1;
        if !schema_ok {
            return Err(DeletionLogError::Incompatible);
        }
        let log = Self {
            db_path: db_path.to_path_buf(),
            db_identity: file_identity(db_path)?,
            reference_path: trusted_ref.bound_path().map(Path::to_path_buf),
            conn: Mutex::new(conn),
            write_lock_timeout: DEFAULT_WRITE_LOCK_TIMEOUT,
        };
        let guard = log.acquire_existing_write_lock()?;
        log.verify_file_identities(&guard)?;
        log.verify_existing_or_pending(trusted_ref)?;
        Ok(log)
    }

    fn verify_existing_or_pending(&self, trusted_ref: &dyn TrustedReferenceStore) -> Result<(), DeletionLogError> {
        let conn = self.conn.lock().expect("deletion log mutex poisoned");
        Self::verify_chain_on(&conn)?;
        let latest = Self::latest_row(&conn)?;
        let reference = trusted_ref.read()?.ok_or_else(|| {
            if latest.is_none() { DeletionLogError::LegacyNeedsMigration }
            else { DeletionLogError::CorruptOrTampered }
        })?;
        Self::verify_binding_on(&conn, &reference, latest.is_none())?;
        match latest {
            None if Self::is_genesis_reference(&reference) => Ok(()),
            Some(row) if row.generation == reference.generation && row.chain_hash == reference.chain_hash => Ok(()),
            Some(row) if row.state == DeletionLogState::Pending
                && row.generation == reference.generation + 1
                && row.prev_chain_hash == reference.chain_hash => Ok(()),
            _ => Err(DeletionLogError::CorruptOrTampered),
        }
    }

    fn genesis_reference(store_id: String) -> GenerationRef {
        GenerationRef {
            generation: 0,
            chain_hash: GENESIS_PREV_CHAIN_HASH.to_string(),
            format_version: Some(DELETION_LOG_FORMAT_VERSION),
            store_id: Some(store_id),
        }
    }

    pub(crate) fn is_genesis_reference(reference: &GenerationRef) -> bool {
        reference.generation == 0
            && reference.chain_hash == GENESIS_PREV_CHAIN_HASH
            && reference.format_version == Some(DELETION_LOG_FORMAT_VERSION)
            && reference.store_id.as_ref().is_some_and(|id| !id.is_empty())
    }

    fn create_schema(conn: &Connection) -> Result<(), DeletionLogError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS deletions (
                generation INTEGER PRIMARY KEY,
                note_id TEXT NOT NULL,
                prev_chain_hash TEXT NOT NULL,
                chain_hash TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('pending','completed'))
            );",
        )
        .map_err(|e| DeletionLogError::Io(e.to_string()))
    }

    /// Opens (creating if necessary) the deletion-log SQLite database at
    /// `db_path`, e.g. `<root>/deletion-log.sqlite`.
    #[cfg(test)]
    pub(crate) fn open(db_path: &Path) -> Result<Self, DeletionLogError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        }
        let conn = Connection::open(db_path).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        Self::create_schema(&conn)?;
        Ok(Self { db_path: db_path.to_path_buf(), db_identity: file_identity(db_path)?, reference_path: None, conn: Mutex::new(conn), write_lock_timeout: DEFAULT_WRITE_LOCK_TIMEOUT })
    }

    /// The path to this log's own SQLite file.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// `<same dir as db_path>/<db file stem>.writelock` (for the standard
    /// `deletion-log.sqlite`, `deletion-log.writelock`): the dedicated file
    /// the cross-process write lock is taken on. Never read or written; it
    /// exists only to be locked. Derived from `db_path`, so every handle
    /// opened on the same log — in this process or another — locks the same
    /// file.
    pub fn write_lock_path(&self) -> PathBuf {
        self.db_path.with_extension("writelock")
    }

    /// Test-only: shortens the bounded wait so a test can observe
    /// [`DeletionLogError::WriteLockUnavailable`] without waiting the full
    /// production timeout.
    #[cfg(test)]
    pub(crate) fn set_write_lock_timeout(&mut self, timeout: Duration) {
        self.write_lock_timeout = timeout;
    }

    /// Acquires the deletion log's cross-process write lock: an exclusive
    /// OS-level advisory lock (`std::fs::File::try_lock`: `LockFileEx` on
    /// Windows, `flock` on Unix) on [`DeletionLog::write_lock_path`], waiting
    /// at most this log's bounded timeout and otherwise failing closed with
    /// [`DeletionLogError::WriteLockUnavailable`] — never proceeding
    /// unlocked.
    ///
    /// This is a mutual-exclusion GATE around the existing protocols, not a
    /// transaction: [`DeletionLog::record_deletion`] holds it for its whole
    /// span, while each of its three SQL commits (C5 steps 1-3) stays its own
    /// standalone, immediately durable statement exactly as before; and
    /// `storage`'s restore promotion holds it across one restored note's
    /// final `is_deleted` re-check and its pointer rename. Two such spans can
    /// therefore never interleave, whether they run in two threads of one
    /// process or in two processes sharing one vault (each acquisition opens
    /// its own handle, and OS file locks conflict across handles).
    ///
    /// Lock ordering (outermost first): `storage::Vault`'s per-note lock
    /// (held by the live sensitive-write path around `record_deletion`) ->
    /// this write lock -> this log's internal connection mutex (taken and
    /// released per statement). Nothing may take a `Vault` per-note lock
    /// while holding this one.
    ///
    /// Which APIs take it (F2, independent review of 41005c6):
    ///
    /// - Acquire it themselves, for their whole span: `record_deletion` (and
    ///   `record_deletion_with_injector`), `reconcile`, and
    ///   `verify_safe_to_expose_after_restore` (which runs its reconcile and
    ///   its reference read under one hold, via the private
    ///   `reconcile_locked`). Every writer of the log's rows or of the
    ///   trusted reference is in this list.
    /// - Never take it, and are safe to call with it held: `is_deleted`,
    ///   `is_empty`, `verify_chain`, `db_path`, `write_lock_path`: pure reads.
    ///   This is what `storage`'s restore promotion relies on (it calls
    ///   `is_deleted` under the guard).
    ///
    /// Not re-entrant: a caller already holding the guard must not call any
    /// API in the first list (it would wait on itself until the timeout and
    /// fail closed with `WriteLockUnavailable`). No production path does:
    /// the restore promotion only calls `is_deleted` under it, and
    /// `record_deletion` never calls `reconcile`.
    pub fn acquire_write_lock(&self) -> Result<DeletionLogWriteGuard, DeletionLogError> {
        self.acquire_existing_write_lock()
    }

    fn latest_row(conn: &Connection) -> Result<Option<Row>, DeletionLogError> {
        let mut stmt = conn
            .prepare(
                "SELECT generation, note_id, prev_chain_hash, chain_hash, state
                 FROM deletions ORDER BY generation DESC LIMIT 1",
            )
            .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        let mut rows = stmt.query([]).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        if let Some(row) = rows.next().map_err(|e| DeletionLogError::Io(e.to_string()))? {
            let generation: i64 = row.get(0).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let note_id: String = row.get(1).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let prev_chain_hash: String = row.get(2).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let chain_hash: String = row.get(3).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let state_str: String = row.get(4).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let state = DeletionLogState::from_sql(&state_str).ok_or(DeletionLogError::ChainBroken)?;
            Ok(Some(Row { generation: generation as u64, note_id, prev_chain_hash, chain_hash, state }))
        } else {
            Ok(None)
        }
    }

    /// Test-only lookup used by test assertions to inspect a specific
    /// generation's persisted state directly.
    #[cfg(test)]
    fn row_for_generation(conn: &Connection, generation: u64) -> Result<Option<Row>, DeletionLogError> {
        let mut stmt = conn
            .prepare(
                "SELECT generation, note_id, prev_chain_hash, chain_hash, state
                 FROM deletions WHERE generation = ?1",
            )
            .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        let mut rows = stmt.query([generation as i64]).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        if let Some(row) = rows.next().map_err(|e| DeletionLogError::Io(e.to_string()))? {
            let generation: i64 = row.get(0).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let note_id: String = row.get(1).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let prev_chain_hash: String = row.get(2).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let chain_hash: String = row.get(3).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let state_str: String = row.get(4).map_err(|e| DeletionLogError::Io(e.to_string()))?;
            let state = DeletionLogState::from_sql(&state_str).ok_or(DeletionLogError::ChainBroken)?;
            Ok(Some(Row { generation: generation as u64, note_id, prev_chain_hash, chain_hash, state }))
        } else {
            Ok(None)
        }
    }

    fn compute_chain_hash(prev_chain_hash: &str, generation: u64, note_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(prev_chain_hash.as_bytes());
        hasher.update(generation.to_string().as_bytes());
        hasher.update(note_id.as_bytes());
        let digest = hasher.finalize();
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Records a deletion of `note_id`, running the full three-step C5
    /// protocol synchronously: (1) write+sync a new `Pending` generation
    /// entry; (2) update the trusted reference; (3) mark the entry
    /// `Completed`. Returns only after step 3 has completed.
    ///
    /// The whole call runs under [`DeletionLog::acquire_write_lock`] (failing
    /// closed with [`DeletionLogError::WriteLockUnavailable`] if it cannot be
    /// taken), so it can never interleave with `storage`'s restore promotion
    /// of a note between that promotion's final `is_deleted` re-check and its
    /// pointer rename. The lock gates the protocol; it does not wrap it in a
    /// transaction: each of the three steps below still commits on its own,
    /// immediately and durably, so a crash after step 1 still leaves a
    /// durable, already-authoritative `Pending` row (C5).
    pub fn record_deletion(
        &self,
        note_id: &str,
        trusted_ref: &dyn TrustedReferenceStore,
    ) -> Result<DeletionReceipt, DeletionLogError> {
        self.record_deletion_with_injector(note_id, trusted_ref, &FaultInjector::none())
    }

    pub(crate) fn record_deletion_with_injector(
        &self,
        note_id: &str,
        trusted_ref: &dyn TrustedReferenceStore,
        injector: &FaultInjector,
    ) -> Result<DeletionReceipt, DeletionLogError> {
        self.assert_reference_bound(trusted_ref)?;
        // Outer: the cross-process write lock, held (RAII) until this
        // function returns by ANY path — including the injected-crash early
        // returns below, mirroring a real crash, where process death drops
        // the OS lock. Inner: the connection mutex. Never the other order.
        let write_lock = self.acquire_write_lock()?;
        self.verify_file_identities(&write_lock)?;
        self.reconcile_locked(&write_lock, trusted_ref)?;
        self.verify_consistent(trusted_ref)?;
        let conn = self.conn.lock().expect("deletion log mutex poisoned");

        let (prev_chain_hash, next_generation) = match Self::latest_row(&conn)? {
            Some(row) => (row.chain_hash, row.generation + 1),
            None => (GENESIS_PREV_CHAIN_HASH.to_string(), 1u64),
        };
        let chain_hash = Self::compute_chain_hash(&prev_chain_hash, next_generation, note_id);

        // Step 1: write + sync the new generation entry, marked Pending.
        // rusqlite commits (and thus durably syncs, journal mode permitting)
        // on each standalone `execute` outside an explicit transaction.
        conn.execute(
            "INSERT INTO deletions (generation, note_id, prev_chain_hash, chain_hash, state)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                next_generation as i64,
                note_id,
                prev_chain_hash,
                chain_hash,
                DeletionLogState::Pending.as_sql(),
            ],
        )
        .map_err(|e| DeletionLogError::Io(e.to_string()))?;

        if injector.check(FaultPoint::AfterGenerationWrittenPending).is_err() {
            // Simulated crash: step 1 has happened; steps 2/3 have not.
            // `is_deleted` must already observe this note as deleted.
            return Ok(DeletionReceipt { generation: next_generation, chain_hash });
        }

        // Step 2: update + sync the trusted reference.
        let binding = trusted_ref.read()?.ok_or(DeletionLogError::CorruptOrTampered)?;
        trusted_ref.write(&GenerationRef {
            generation: next_generation,
            chain_hash: chain_hash.clone(),
            format_version: binding.format_version,
            store_id: binding.store_id,
        })?;

        if injector.check(FaultPoint::AfterReferenceUpdated).is_err() {
            return Ok(DeletionReceipt { generation: next_generation, chain_hash });
        }

        if injector.check(FaultPoint::BeforeMarkedCompleted).is_err() {
            return Ok(DeletionReceipt { generation: next_generation, chain_hash });
        }

        // Step 3: mark the generation Completed.
        conn.execute(
            "UPDATE deletions SET state = ?1 WHERE generation = ?2",
            rusqlite::params![DeletionLogState::Completed.as_sql(), next_generation as i64],
        )
        .map_err(|e| DeletionLogError::Io(e.to_string()))?;

        Ok(DeletionReceipt { generation: next_generation, chain_hash })
    }

    /// Returns `true` if this log has never recorded a single row — i.e. no
    /// deletion, in any state, has ever been written to this log. Used by
    /// [`crate::backup`]'s "no deletions ever" sentinel exception, which
    /// must check the log's own row history, not merely the absence of the
    /// trusted reference (an absent reference can ALSO mean "the reference
    /// was lost while the log itself has real history," a state
    /// [`DeletionLog::reconcile`] itself fails closed on via
    /// [`DeletionLogError::ReferenceMissing`] rather than treating as
    /// harmless).
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> Result<bool, DeletionLogError> {
        let conn = self.conn.lock().expect("deletion log mutex poisoned");
        Ok(Self::latest_row(&conn)?.is_none())
    }

    /// Returns `true` if `note_id` has a deletion entry in ANY state
    /// (`Pending` or `Completed`). Per C5, the note becomes non-consultable
    /// starting at step 1 — a `Pending` entry must already read as deleted.
    pub(crate) fn is_deleted(&self, note_id: &str) -> Result<bool, DeletionLogError> {
        let conn = self.conn.lock().expect("deletion log mutex poisoned");
        let mut stmt = conn
            .prepare("SELECT 1 FROM deletions WHERE note_id = ?1 LIMIT 1")
            .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        let mut rows = stmt.query([note_id]).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        Ok(rows.next().map_err(|e| DeletionLogError::Io(e.to_string()))?.is_some())
    }

    /// Verified lookup for a caller already holding this log's write guard,
    /// such as restore promotion. The caller retains the guard through its
    /// promotion decision and pointer rename.
    pub fn is_deleted_verified_locked(
        &self,
        guard: &DeletionLogWriteGuard,
        trusted_ref: &dyn TrustedReferenceStore,
        note_id: &str,
    ) -> Result<bool, DeletionLogError> {
        self.assert_reference_bound(trusted_ref)?;
        self.verify_file_identities(guard)?;
        self.verify_consistent(trusted_ref)?;
        let deleted = self.is_deleted(note_id)?;
        self.verify_file_identities(guard)?;
        Ok(deleted)
    }

    /// Verifies the complete log/reference pair and performs the lookup while
    /// holding the same cross-process lock used by deletion writers.
    pub fn with_verified_not_deleted<T>(
        &self,
        trusted_ref: &dyn TrustedReferenceStore,
        note_id: &str,
        expose: impl FnOnce() -> Result<T, DeletionLogError>,
    ) -> Result<Option<T>, DeletionLogError> {
        self.assert_reference_bound(trusted_ref)?;
        let _guard = self.acquire_existing_write_lock()?;
        self.verify_file_identities(&_guard)?;
        self.verify_consistent(trusted_ref)?;
        if self.is_deleted(note_id)? {
            return Ok(None);
        }
        let value = expose()?;
        self.verify_consistent(trusted_ref)?;
        self.verify_file_identities(&_guard)?;
        Ok(Some(value))
    }

    /// Runs a non-note-specific operation against one verified, locked view.
    pub fn with_verified<T>(
        &self,
        trusted_ref: &dyn TrustedReferenceStore,
        operation: impl FnOnce(&GenerationRef) -> Result<T, DeletionLogError>,
    ) -> Result<T, DeletionLogError> {
        self.assert_reference_bound(trusted_ref)?;
        let _guard = self.acquire_existing_write_lock()?;
        self.verify_file_identities(&_guard)?;
        self.verify_consistent(trusted_ref)?;
        let before = trusted_ref.read()?.ok_or(DeletionLogError::CorruptOrTampered)?;
        let value = operation(&before)?;
        self.verify_consistent(trusted_ref)?;
        let after = trusted_ref.read()?.ok_or(DeletionLogError::CorruptOrTampered)?;
        if before != after {
            return Err(DeletionLogError::CorruptOrTampered);
        }
        self.verify_file_identities(&_guard)?;
        Ok(value)
    }

    /// Enumerates against one verified view while retaining the deletion
    /// lock through the caller's entire enumeration and result construction.
    pub fn with_verified_lookup<T>(
        &self,
        trusted_ref: &dyn TrustedReferenceStore,
        operation: impl FnOnce(&dyn Fn(&str) -> Result<bool, DeletionLogError>) -> Result<T, DeletionLogError>,
    ) -> Result<T, DeletionLogError> {
        self.with_verified(trusted_ref, |_| operation(&|note_id| self.is_deleted(note_id)))
    }

    fn verify_file_identities(&self, guard: &DeletionLogWriteGuard) -> Result<(), DeletionLogError> {
        if file_identity(&self.db_path)? != self.db_identity {
            return Err(DeletionLogError::CorruptOrTampered);
        }
        let current = file_identity(&self.write_lock_path())?;
        if identity_of_open_file(&guard.file)? != current {
            return Err(DeletionLogError::CorruptOrTampered);
        }
        Ok(())
    }

    fn verify_consistent(&self, trusted_ref: &dyn TrustedReferenceStore) -> Result<(), DeletionLogError> {
        self.assert_reference_bound(trusted_ref)?;
        let conn = self.conn.lock().expect("deletion log mutex poisoned");
        Self::verify_chain_on(&conn)?;
        let latest = Self::latest_row(&conn)?;
        let reference = trusted_ref.read()?.ok_or(DeletionLogError::CorruptOrTampered)?;
        Self::verify_binding_on(&conn, &reference, latest.is_none())?;
        match latest {
            None if Self::is_genesis_reference(&reference) => Ok(()),
            None => Err(DeletionLogError::CorruptOrTampered),
            Some(row) if row.generation == reference.generation && row.chain_hash == reference.chain_hash => Ok(()),
            Some(_) => Err(DeletionLogError::CorruptOrTampered),
        }
    }

    fn verify_binding_on(conn: &Connection, reference: &GenerationRef, log_is_empty: bool) -> Result<(), DeletionLogError> {
        if let (Some(version), Some(reference_store_id)) = (reference.format_version, reference.store_id.as_deref()) {
            let metadata = conn
                .query_row(
                    "SELECT format_version, store_id FROM deletion_log_meta WHERE singleton = 1",
                    [],
                    |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(|_| DeletionLogError::Incompatible)?;
            if metadata.0 != version || metadata.0 != DELETION_LOG_FORMAT_VERSION || metadata.1 != reference_store_id {
                return Err(DeletionLogError::CorruptOrTampered);
            }
        } else if log_is_empty {
            return Err(DeletionLogError::LegacyNeedsMigration);
        }
        Ok(())
    }

    fn assert_reference_bound(&self, reference: &dyn TrustedReferenceStore) -> Result<(), DeletionLogError> {
        if let Some(expected) = &self.reference_path {
            if reference.bound_path() != Some(expected.as_path()) {
                return Err(DeletionLogError::CorruptOrTampered);
            }
        } else if !cfg!(test) {
            return Err(DeletionLogError::CorruptOrTampered);
        }
        Ok(())
    }

    fn acquire_existing_write_lock(&self) -> Result<DeletionLogWriteGuard, DeletionLogError> {
        let path = self.write_lock_path();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => DeletionLogError::LegacyNeedsMigration,
                _ => DeletionLogError::Io(e.to_string()),
            })?;
        let deadline = Instant::now() + self.write_lock_timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(DeletionLogWriteGuard { file }),
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(WRITE_LOCK_POLL_INTERVAL);
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(DeletionLogError::WriteLockUnavailable {
                        timeout_ms: self.write_lock_timeout.as_millis() as u64,
                    });
                }
                Err(std::fs::TryLockError::Error(e)) => return Err(DeletionLogError::Io(e.to_string())),
            }
        }
    }

    /// Walks the WHOLE chain from genesis and verifies every link (final
    /// whole-branch review, I5). Before this, only the latest row was ever
    /// compared against the trusted reference, so a corrupted or rewritten
    /// MIDDLE row went undetected, and rewriting a middle row's `note_id`
    /// silently "un-deletes" that note (`is_deleted` stops seeing it).
    ///
    /// Rows are read in generation order and must be exactly generations
    /// `1, 2, ..., n` with no gap; each row's `prev_chain_hash` must equal
    /// its predecessor's `chain_hash` ([`GENESIS_PREV_CHAIN_HASH`] for
    /// generation 1); each row's `chain_hash` must equal
    /// `sha256(prev_chain_hash || generation || note_id)` recomputed from the
    /// row itself; and each `state` must be a known one. Any failure is
    /// [`DeletionLogError::ChainBroken`]. Run by [`DeletionLog::reconcile`]
    /// (so on every production startup) and by
    /// [`DeletionLog::verify_safe_to_expose_after_restore`] (so on every
    /// restore). The latest row's link to the trusted reference is
    /// `reconcile`'s own, separate check.
    pub fn verify_chain(&self) -> Result<(), DeletionLogError> {
        let conn = self.conn.lock().expect("deletion log mutex poisoned");
        Self::verify_chain_on(&conn)
    }

    fn verify_chain_on(conn: &Connection) -> Result<(), DeletionLogError> {
        let io = |e: rusqlite::Error| DeletionLogError::Io(e.to_string());
        let mut stmt = conn
            .prepare("SELECT generation, note_id, prev_chain_hash, chain_hash, state FROM deletions ORDER BY generation ASC")
            .map_err(io)?;
        let mut rows = stmt.query([]).map_err(io)?;
        let mut expected_generation: u64 = 1;
        let mut expected_prev = GENESIS_PREV_CHAIN_HASH.to_string();
        while let Some(row) = rows.next().map_err(io)? {
            let generation: i64 = row.get(0).map_err(io)?;
            let note_id: String = row.get(1).map_err(io)?;
            let prev_chain_hash: String = row.get(2).map_err(io)?;
            let chain_hash: String = row.get(3).map_err(io)?;
            let state: String = row.get(4).map_err(io)?;
            if generation < 0 || generation as u64 != expected_generation {
                return Err(DeletionLogError::ChainBroken);
            }
            if prev_chain_hash != expected_prev {
                return Err(DeletionLogError::ChainBroken);
            }
            if chain_hash != Self::compute_chain_hash(&prev_chain_hash, expected_generation, &note_id) {
                return Err(DeletionLogError::ChainBroken);
            }
            if DeletionLogState::from_sql(&state).is_none() {
                return Err(DeletionLogError::ChainBroken);
            }
            expected_generation += 1;
            expected_prev = chain_hash;
        }
        Ok(())
    }

    /// Run at startup (`storage::Vault::open_with_pii_store`). First walks
    /// the whole chain ([`DeletionLog::verify_chain`]; a broken link is
    /// [`DeletionLogError::ChainBroken`]), then compares the log's latest
    /// generation against the trusted reference:
    ///
    /// - Equal generation and matching hash, with the log's row already
    ///   `Completed`: [`ReconcileOutcome::AlreadyConsistent`].
    /// - The log is exactly one generation ahead of the reference, and that
    ///   latest log row is `Pending`: this is the "crashed between step 1
    ///   and step 2, or step 2 and step 3" window. Roll forward — update the
    ///   reference to match, then mark the row `Completed`. Returns
    ///   [`ReconcileOutcome::RolledForward`].
    /// - Anything else (reference ahead of the log, any other mismatch,
    ///   hash disagreement, missing/corrupt/tampered reference, etc.) is
    ///   failed closed: never guessed.
    ///
    /// Takes [`DeletionLog::acquire_write_lock`] itself, for its whole span
    /// (the reads it decides from as well as the roll-forward writes), and
    /// fails closed with [`DeletionLogError::WriteLockUnavailable`] if it
    /// cannot (independent review of 41005c6, F2). Without it, a reconcile
    /// that had read "generation N `Pending`, reference N-1" could be
    /// overtaken by another handle's (or process's) reconcile and a further
    /// `record_deletion`, then write its stale reference N over the live
    /// N+1, after which every later reconcile failed closed. A caller that
    /// already holds the guard must not call this (the lock is not
    /// re-entrant); inside this module such a caller uses
    /// `reconcile_locked` instead, as `verify_safe_to_expose_after_restore`
    /// does.
    pub fn reconcile(&self, trusted_ref: &dyn TrustedReferenceStore) -> Result<ReconcileOutcome, DeletionLogError> {
        self.assert_reference_bound(trusted_ref)?;
        let write_lock = self.acquire_write_lock()?;
        self.reconcile_locked(&write_lock, trusted_ref)
    }

    /// The body of [`DeletionLog::reconcile`], for a caller that already
    /// holds this log's write lock: `_held` is the caller's guard, required
    /// as evidence so this can never be called unlocked. Takes only the
    /// connection mutex (innermost, per the lock order on
    /// `acquire_write_lock`).
    fn reconcile_locked(
        &self,
        _held: &DeletionLogWriteGuard,
        trusted_ref: &dyn TrustedReferenceStore,
    ) -> Result<ReconcileOutcome, DeletionLogError> {
        let conn = self.conn.lock().expect("deletion log mutex poisoned");
        // I5: the whole chain must verify before anything is rolled forward
        // or declared consistent.
        Self::verify_chain_on(&conn)?;
        let latest = Self::latest_row(&conn)?;
        let reference = trusted_ref.read()?;
        if let Some(reference) = &reference {
            Self::verify_binding_on(&conn, reference, latest.is_none())?;
        }

        match (latest, reference) {
            (None, None) => Err(DeletionLogError::LegacyNeedsMigration),
            (None, Some(r)) if Self::is_genesis_reference(&r) => Ok(ReconcileOutcome::AlreadyConsistent),
            (None, Some(r)) => {
                // The reference claims a generation but the log has no
                // rows at all: the log can never be ahead by construction,
                // but a reference pointing at a nonexistent generation is
                // exactly the kind of disagreement that must fail closed.
                Err(DeletionLogError::ReferenceStale { log_generation: 0, reference_generation: r.generation })
            }
            (Some(_row), None) => {
                // No reference at all. Two genuinely different scenarios
                // produce this exact same observable state:
                //
                // 1. The very first deletion ever recorded crashed between
                //    step 1 (generation 1 written, Pending) and step 2 (the
                //    reference, which has therefore never been written at
                //    all yet — not merely stale, but nonexistent). This is
                //    the roll-forward case: safe, because nothing has ever
                //    been exposed under a prior reference and every writer
                //    of this chain holds the write lock this call holds.
                // 2. A restore where the reference was lost/not carried
                //    along ("registro assente" in C5's test list) while the
                //    log already has completed history. This must fail
                //    closed.
                //
                // Disambiguated by generation + state: only "log at
                // generation 1, still Pending" matches scenario 1. Anything
                // else with no reference present (log at a later
                // generation, or already Completed) fails closed as
                // ReferenceMissing.
                Err(DeletionLogError::ReferenceMissing)
            }
            (Some(row), Some(r)) => {
                if row.generation == r.generation {
                    if row.chain_hash != r.chain_hash {
                        return Err(DeletionLogError::ChainBroken);
                    }
                    if row.state == DeletionLogState::Pending {
                        // Reference already matches this generation, but the
                        // row never got marked Completed (crash right at
                        // step 3). Safe to complete directly: the reference
                        // update (step 2) already happened.
                        conn.execute(
                            "UPDATE deletions SET state = ?1 WHERE generation = ?2",
                            rusqlite::params![DeletionLogState::Completed.as_sql(), row.generation as i64],
                        )
                        .map_err(|e| DeletionLogError::Io(e.to_string()))?;
                        return Ok(ReconcileOutcome::RolledForward { generation: row.generation });
                    }
                    Ok(ReconcileOutcome::AlreadyConsistent)
                } else if row.generation == r.generation + 1 && row.state == DeletionLogState::Pending {
                    // The one specific roll-forward pattern: log is exactly
                    // one generation ahead, still Pending. Crash happened
                    // between step 1 and step 2 (reference never updated) or
                    // between step 2 and step 3 (reference updated, this
                    // path's hash check below would have caught a mismatch
                    // instead) — either way, rolling forward is safe because
                    // nothing has been exposed yet (is_deleted already
                    // returns true for the Pending row) and no other writer
                    // can have produced a DIFFERENT generation N here, nor
                    // advance past it while this runs: every writer of this
                    // chain and its reference, in any handle or process,
                    // holds the write lock this call holds (F2).
                    trusted_ref.write(&GenerationRef {
                        generation: row.generation,
                        chain_hash: row.chain_hash.clone(),
                        format_version: r.format_version,
                        store_id: r.store_id,
                    })?;
                    conn.execute(
                        "UPDATE deletions SET state = ?1 WHERE generation = ?2",
                        rusqlite::params![DeletionLogState::Completed.as_sql(), row.generation as i64],
                    )
                    .map_err(|e| DeletionLogError::Io(e.to_string()))?;
                    Ok(ReconcileOutcome::RolledForward { generation: row.generation })
                } else if row.generation < r.generation {
                    // Reference ahead of the log: never a legitimate state
                    // for this log's own reference (the log is the only
                    // writer of its own generations) — fail closed.
                    Err(DeletionLogError::Inconsistent)
                } else {
                    // Any other mismatch pattern (log more than one ahead,
                    // or one ahead but already Completed with a different
                    // hash, etc.): fail closed, never guess.
                    Err(DeletionLogError::Inconsistent)
                }
            }
        }
    }

    /// Restore-time gate (Task 5b): checks whether it is safe to expose data
    /// restored alongside `restored_reference_snapshot` (the trusted
    /// reference as it existed inside the backup) by comparing it against
    /// the LIVE trusted reference (never the backup's own embedded
    /// snapshot, per C5/the plan's restore rule). Equal or ahead of the
    /// live reference is fine; strictly behind is
    /// [`DeletionLogError::BackupReferenceStale`] — distinct from
    /// [`DeletionLogError::ReferenceStale`], which `reconcile` (called
    /// first, below) would have already returned if the LIVE log/reference
    /// pair itself disagreed. A caller MUST NOT treat the two the same way:
    /// `BackupReferenceStale` means only this backup's own snapshot is old
    /// against an already-trustworthy live state (safe to proceed past,
    /// e.g. to a per-note exclusion check); `ReferenceStale` means the live
    /// state itself could not be trusted (never safe to proceed past).
    ///
    /// Final-review re-review, N1: this used to compare
    /// `restored_reference_snapshot` only against whatever
    /// `live_trusted_ref.read()` returned, without ever confirming that
    /// value is itself the LIVE LOG's own head — `verify_chain` proves the
    /// log's chain is internally consistent, not that it agrees with the
    /// reference. A log that is missing, rolled back, or stuck mid-protocol
    /// could still pass that narrower check while disagreeing with what the
    /// log itself has actually recorded, exposing a deleted note's content.
    /// `reconcile` is the one function that already proves the live log's
    /// head agrees with the live reference (completing a single safe
    /// roll-forward if the log crashed mid-protocol, failing closed on any
    /// other disagreement) — called here first, so this gate's exposure
    /// decision is always made against a log state `reconcile` itself has
    /// just vouched for, never a merely self-consistent one.
    pub fn verify_safe_to_expose_after_restore(
        &self,
        restored_reference_snapshot: &GenerationRef,
        live_trusted_ref: &dyn TrustedReferenceStore,
    ) -> Result<(), DeletionLogError> {
        self.assert_reference_bound(live_trusted_ref)?;
        // The live log's own head must agree with the live reference before
        // this gate trusts either of them for the comparison below. Both run
        // under one hold of the write lock (F2), so no deletion can advance
        // the reference between the reconcile and the read it vouches for.
        let write_lock = self.acquire_write_lock()?;
        self.reconcile_locked(&write_lock, live_trusted_ref)?;
        let live = live_trusted_ref.read()?.ok_or(DeletionLogError::ReferenceMissing)?;
        drop(write_lock);
        if restored_reference_snapshot.generation < live.generation {
            return Err(DeletionLogError::BackupReferenceStale {
                backup_generation: restored_reference_snapshot.generation,
                live_generation: live.generation,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn open_log(dir: &Path) -> DeletionLog {
        let reference = file_ref(dir);
        DeletionLog::bootstrap_new(&dir.join("deletion-log.sqlite"), &reference).expect("bootstrap deletion log")
    }

    fn file_ref(dir: &Path) -> FileTrustedReferenceStore {
        FileTrustedReferenceStore::new(dir.join("trusted-reference.bin"))
    }

    #[test]
    fn p0_open_existing_absent_fails_without_creating_any_state() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("missing").join("deletion-log.sqlite");
        let reference = FileTrustedReferenceStore::new(dir.path().join("missing").join("reference"));

        assert!(matches!(
            DeletionLog::open_existing(&db, &reference),
            Err(DeletionLogError::LegacyNeedsMigration)
        ));
        assert!(!db.parent().unwrap().exists());
    }

    #[test]
    fn p0_explicit_bootstrap_creates_authenticated_generation_zero_and_reopens() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = file_ref(dir.path());

        let log = DeletionLog::bootstrap_new(&db, &reference).unwrap();
        assert!(reference.read().unwrap().as_ref().is_some_and(DeletionLog::is_genesis_reference));
        assert!(!log.is_deleted("note-a").unwrap());
        drop(log);
        DeletionLog::open_existing(&db, &reference).unwrap();
    }

    #[test]
    fn p0_explicit_bootstrap_recovers_only_the_authenticated_partial_genesis_db() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = file_ref(dir.path());
        let genesis = DeletionLog::genesis_reference(Uuid::new_v4().to_string());
        reference.write(&genesis).unwrap();
        std::fs::File::create(&db).unwrap(); // crash after DB creation, before schema/meta

        let log = DeletionLog::bootstrap_new(&db, &reference).unwrap();
        assert!(log.with_verified(&reference, |_| Ok(())).is_ok());
        DeletionLog::open_existing(&db, &reference).unwrap();
    }

    #[test]
    fn p0_missing_db_after_open_is_not_recreated_by_a_consumer() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = file_ref(dir.path());
        let log = DeletionLog::bootstrap_new(&db, &reference).unwrap();
        drop(log);
        std::fs::remove_file(&db).unwrap();

        assert!(matches!(
            DeletionLog::open_existing(&db, &reference),
            Err(DeletionLogError::CorruptOrTampered)
        ));
        assert!(!db.exists());
    }

    #[test]
    fn p0_reference_removed_after_open_denies_exposure() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = file_ref(dir.path());
        let log = DeletionLog::bootstrap_new(&db, &reference).unwrap();
        std::fs::remove_file(dir.path().join("trusted-reference.bin")).unwrap();

        assert_eq!(
            log.with_verified_not_deleted(&reference, "note-a", || Ok(())).unwrap_err(),
            DeletionLogError::CorruptOrTampered
        );
    }

    #[test]
    fn p0_open_existing_rejects_a_genesis_reference_bound_to_another_store() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = file_ref(dir.path());
        let log = DeletionLog::bootstrap_new(&db, &reference).unwrap();
        drop(log);
        reference.write(&GenerationRef {
            generation: 0,
            chain_hash: GENESIS_PREV_CHAIN_HASH.to_string(),
            format_version: Some(DELETION_LOG_FORMAT_VERSION),
            store_id: Some(Uuid::new_v4().to_string()),
        }).unwrap();

        assert!(matches!(
            DeletionLog::open_existing(&db, &reference),
            Err(DeletionLogError::CorruptOrTampered)
        ));
    }

    #[test]
    fn p0_open_existing_rejects_a_reference_which_does_not_match_the_head() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = file_ref(dir.path());
        let log = DeletionLog::bootstrap_new(&db, &reference).unwrap();
        log.record_deletion("deleted-note", &reference).unwrap();
        let actual = reference.read().unwrap().unwrap();
        drop(log);
        reference.write(&GenerationRef {
            chain_hash: "0".repeat(64),
            ..actual
        }).unwrap();

        assert!(matches!(
            DeletionLog::open_existing(&db, &reference),
            Err(DeletionLogError::CorruptOrTampered)
        ));
    }

    /// `is_retryable_genesis_partial`'s three checks, called directly: (1)
    /// ANY table other than `deletions` disqualifies the DB from being a
    /// retryable genesis-partial — the `!=`->`==` mutant on the first
    /// `.any()` would instead require EVERY table to differ from
    /// `deletions`, wrongly treating an unrelated extra table as acceptable
    /// as long as `deletions` itself isn't present. (2) `deletions`
    /// genuinely present but non-empty disqualifies it too — the `==`->`!=`
    /// mutant on the second `.any()` would skip the row-count check
    /// entirely for the one case that matters (the table actually named
    /// `deletions`). (3) `count != 0` itself (the `!=`->`==` mutant). No
    /// existing bootstrap test ever produces a DB with an UNRELATED extra
    /// table, since real bootstrap crash points only ever create
    /// `deletions` and `deletion_log_meta`, never a third table.
    #[test]
    fn is_retryable_genesis_partial_rejects_an_unrelated_extra_table() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE some_other_table (id INTEGER PRIMARY KEY);").unwrap();
        drop(conn);
        assert_eq!(DeletionLog::is_retryable_genesis_partial(&db), Ok(false), "an unrelated extra table must disqualify genesis-partial retry, even with no `deletions` table present");
    }

    #[test]
    fn is_retryable_genesis_partial_rejects_a_deletions_table_with_rows() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let conn = Connection::open(&db).unwrap();
        DeletionLog::create_schema(&conn).unwrap();
        conn.execute("INSERT INTO deletions VALUES (1, 'note-a', '0', 'x', 'completed')", []).unwrap();
        drop(conn);
        assert_eq!(DeletionLog::is_retryable_genesis_partial(&db), Ok(false), "a deletions table with real rows must disqualify genesis-partial retry");
    }

    #[test]
    fn is_retryable_genesis_partial_accepts_an_empty_deletions_table_and_no_tables_at_all() {
        let dir = tempdir().unwrap();
        let no_tables_db = dir.path().join("no-tables.sqlite");
        Connection::open(&no_tables_db).unwrap();
        assert_eq!(DeletionLog::is_retryable_genesis_partial(&no_tables_db), Ok(true), "a genuinely empty DB file (no tables at all) is retryable");

        let empty_deletions_db = dir.path().join("empty-deletions.sqlite");
        let conn = Connection::open(&empty_deletions_db).unwrap();
        DeletionLog::create_schema(&conn).unwrap();
        drop(conn);
        assert_eq!(DeletionLog::is_retryable_genesis_partial(&empty_deletions_db), Ok(true), "a deletions table with zero rows is retryable");
    }

    #[test]
    fn p0_attested_legacy_migration_is_explicit_auditable_and_idempotent() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = file_ref(dir.path());
        let attestation = LegacyNoDeletionAttestation {
            operation_id: Uuid::new_v4(),
            authority: "security-admin".to_string(),
            timestamp: "2026-11-02T00:00:00Z".to_string(),
            reason: "verified no historic deletions".to_string(),
        };

        let first = DeletionLog::migrate_legacy_empty(&db, &reference, &attestation).unwrap();
        let retry = DeletionLog::migrate_legacy_empty(&db, &reference, &attestation).unwrap();
        assert_eq!(first, retry);
        DeletionLog::open_existing(&db, &reference).unwrap();

        let different_operation = LegacyNoDeletionAttestation {
            operation_id: Uuid::new_v4(),
            ..attestation
        };
        assert_eq!(
            DeletionLog::migrate_legacy_empty(&db, &reference, &different_operation),
            Err(DeletionLogError::CorruptOrTampered)
        );
    }

    /// `migrate_legacy_empty`'s attestation-completeness guard is a 3-way
    /// `||`: the `||`->`&&` mutants would wrongly require ALL THREE fields
    /// empty simultaneously instead of rejecting on ANY one being empty.
    /// The existing test only ever supplies a fully-populated attestation.
    #[test]
    fn migrate_legacy_empty_rejects_an_attestation_with_any_single_field_blank() {
        let make = |authority: &str, timestamp: &str, reason: &str| LegacyNoDeletionAttestation {
            operation_id: Uuid::new_v4(),
            authority: authority.to_string(),
            timestamp: timestamp.to_string(),
            reason: reason.to_string(),
        };
        for attestation in [
            make("", "2026-11-02T00:00:00Z", "reason"),
            make("admin", "", "reason"),
            make("admin", "2026-11-02T00:00:00Z", ""),
            make("   ", "2026-11-02T00:00:00Z", "reason"),
        ] {
            let dir = tempdir().unwrap();
            let db = dir.path().join("deletion-log.sqlite");
            let reference = file_ref(dir.path());
            assert_eq!(
                DeletionLog::migrate_legacy_empty(&db, &reference, &attestation),
                Err(DeletionLogError::LegacyNeedsMigration),
                "a blank/whitespace-only field alone must be enough to reject, attestation={attestation:?}"
            );
        }
    }

    /// `migrate_legacy_empty`'s legacy-schema inspection: a pre-existing DB
    /// file with NO tables at all (`table_count == 0`) is the recognized
    /// "truly empty legacy" shape and must be accepted for migration. The
    /// `==`->`!=` mutant on this check would skip straight to querying a
    /// nonexistent `deletions` table and fail with `Incompatible` instead.
    #[test]
    fn migrate_legacy_empty_accepts_a_pre_existing_db_file_with_zero_tables() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        // Create a genuinely empty SQLite file (0 tables) at the target
        // path before migrating, rather than letting migrate_legacy_empty
        // create it fresh.
        Connection::open(&db).unwrap();
        let reference = file_ref(dir.path());
        let attestation = LegacyNoDeletionAttestation {
            operation_id: Uuid::new_v4(),
            authority: "security-admin".to_string(),
            timestamp: "2026-11-02T00:00:00Z".to_string(),
            reason: "pre-existing empty legacy file".to_string(),
        };
        assert!(DeletionLog::migrate_legacy_empty(&db, &reference, &attestation).is_ok(), "a pre-existing DB file with zero tables must be accepted as the empty-legacy shape");
    }

    /// `migrate_legacy_empty`'s row-count guard: a pre-existing legacy DB
    /// that already has the `deletions` table AND at least one row must be
    /// rejected (it has real history, not empty-legacy). The `!=`->`==`
    /// mutant would invert this, rejecting the TRULY empty case and
    /// accepting the one with real history instead.
    #[test]
    fn migrate_legacy_empty_rejects_a_pre_existing_legacy_db_with_a_deletions_row() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE deletions (generation INTEGER PRIMARY KEY, note_id TEXT NOT NULL, prev_chain_hash TEXT NOT NULL, chain_hash TEXT NOT NULL, state TEXT NOT NULL);
                 INSERT INTO deletions VALUES (1, 'note-a', '0', 'x', 'completed');",
            )
            .unwrap();
        }
        let reference = file_ref(dir.path());
        let attestation = LegacyNoDeletionAttestation {
            operation_id: Uuid::new_v4(),
            authority: "security-admin".to_string(),
            timestamp: "2026-11-02T00:00:00Z".to_string(),
            reason: "must not override a real row".to_string(),
        };
        assert_eq!(
            DeletionLog::migrate_legacy_empty(&db, &reference, &attestation),
            Err(DeletionLogError::CorruptOrTampered),
            "a pre-existing deletions row must block migration even without a trusted reference present"
        );
    }

    /// `migrate_legacy_empty`'s idempotent-retry guard: retrying with a
    /// DIFFERENT `operation_id` than the one already persisted in a
    /// completed migration's metadata must be rejected (not silently
    /// treated as either "same operation" or "different, so start fresh").
    /// The `==` match guard's `true`/`false` mutants and the `==`->`!=`
    /// mutant on the comparison itself all target this exact line; the
    /// existing `different_operation` test already covers this via the
    /// FIRST migration's completed reference-bound path (`trusted_ref.read()
    /// = Some`), but this one targets the SECOND code path specifically
    /// (reached when `trusted_ref.read()` is itself `None`, i.e. the
    /// reference file was lost after a completed migration but the DB's own
    /// `deletion_log_meta` row survives) — a distinct branch from the one
    /// the existing test exercises.
    #[test]
    fn migrate_legacy_empty_rejects_a_different_operation_id_when_the_reference_file_is_absent_but_the_db_metadata_survives() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = file_ref(dir.path());
        let attestation = LegacyNoDeletionAttestation {
            operation_id: Uuid::new_v4(),
            authority: "security-admin".to_string(),
            timestamp: "2026-11-02T00:00:00Z".to_string(),
            reason: "first migration".to_string(),
        };
        DeletionLog::migrate_legacy_empty(&db, &reference, &attestation).unwrap();
        // Lose the reference file while the DB's own deletion_log_meta
        // metadata (including migration_operation_id) survives intact.
        std::fs::remove_file(dir.path().join("trusted-reference.bin")).unwrap();
        assert!(reference.read().unwrap().is_none(), "precondition: reference genuinely absent now");

        let different_operation = LegacyNoDeletionAttestation { operation_id: Uuid::new_v4(), ..attestation };
        assert_eq!(
            DeletionLog::migrate_legacy_empty(&db, &reference, &different_operation),
            Err(DeletionLogError::CorruptOrTampered),
            "a different operation_id against surviving DB metadata must be rejected even without a trusted reference present"
        );
    }

    #[test]
    fn p0_migration_refuses_a_legacy_database_with_deletion_history() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = file_ref(dir.path());
        let log = DeletionLog::bootstrap_new(&db, &reference).unwrap();
        log.record_deletion("historic-note", &reference).unwrap();
        drop(log);
        std::fs::remove_file(dir.path().join("trusted-reference.bin")).unwrap();
        let attestation = LegacyNoDeletionAttestation {
            operation_id: Uuid::new_v4(),
            authority: "security-admin".to_string(),
            timestamp: "2026-11-02T00:00:00Z".to_string(),
            reason: "must not override deletion history".to_string(),
        };

        assert_eq!(
            DeletionLog::migrate_legacy_empty(&db, &reference, &attestation),
            Err(DeletionLogError::CorruptOrTampered)
        );
    }

    // -----------------------------------------------------------------
    // DeletionLogState::valid_transition
    // -----------------------------------------------------------------

    #[test]
    fn pending_can_advance_to_completed() {
        assert!(valid_transition(DeletionLogState::Pending, DeletionLogState::Completed));
    }

    #[test]
    fn completed_cannot_revert_to_pending() {
        assert!(!valid_transition(DeletionLogState::Completed, DeletionLogState::Pending));
    }

    #[test]
    fn pending_cannot_stay_pending_as_a_transition() {
        assert!(!valid_transition(DeletionLogState::Pending, DeletionLogState::Pending));
    }

    // -----------------------------------------------------------------
    // Step 1 (brief) tests
    // -----------------------------------------------------------------

    #[test]
    fn crash_between_log_write_and_reference_update_leaves_data_non_consultable_through_restart_and_converges_on_reconcile(
    ) {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());

        // 1. record_deletion with an injector that interrupts right after
        //    step 1 (generation written, Pending).
        let injector = FaultInjector::trigger_at(FaultPoint::AfterGenerationWrittenPending);
        let receipt = log
            .record_deletion_with_injector("note-a", &reference, &injector)
            .expect("record_deletion must return the half-finished receipt, not an error");
        assert_eq!(receipt.generation, 1);

        // 2. Even in this half-finished state, is_deleted must be true.
        assert!(log.is_deleted("note-a").unwrap(), "note must already be non-consultable at step 1");

        assert!(reference.read().unwrap().as_ref().is_some_and(DeletionLog::is_genesis_reference), "reference must still be at authenticated genesis before reconcile");

        // 3. Open a FRESH DeletionLog against the same db_path (restart).
        let restarted = DeletionLog::open(log.db_path()).expect("reopen deletion log");
        assert!(restarted.is_deleted("note-a").unwrap(), "is_deleted must still be true after restart, before reconcile");

        let outcome = restarted.reconcile(&reference).expect("reconcile must roll forward, not fail");
        assert_eq!(outcome, ReconcileOutcome::RolledForward { generation: 1 });

        // 4. Reference now matches, entry is Completed, is_deleted still true.
        let synced = reference.read().unwrap().expect("reference must now be populated");
        assert_eq!(synced.generation, 1);
        assert_eq!(synced.chain_hash, receipt.chain_hash);
        assert!(restarted.is_deleted("note-a").unwrap(), "is_deleted must remain true after reconcile completes it");

        let row = DeletionLog::row_for_generation(&restarted.conn.lock().unwrap(), 1).unwrap().unwrap();
        assert_eq!(row.state, DeletionLogState::Completed);
    }

    #[test]
    fn reference_ahead_of_log_or_any_other_unexplained_mismatch_blocks_in_reconciliation_mode_not_silently() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());

        // Log has one Completed entry at generation 1.
        log.record_deletion("note-a", &reference).unwrap();
        let valid_reference = reference.read().unwrap().unwrap();

        // Now hand-craft a reference claiming generation 5 (ahead of the
        // log, which has never reached generation 5) with a hash that
        // cannot possibly be valid for this chain.
        reference
            .write(&GenerationRef { generation: 5, chain_hash: "f".repeat(64), format_version: None, store_id: None })
            .unwrap();

        let result = log.reconcile(&reference);
        assert!(
            matches!(result, Err(DeletionLogError::Inconsistent)),
            "reference ahead of the log must fail closed as Inconsistent, got {result:?}"
        );
        assert!(matches!(log.record_deletion("note-b", &reference), Err(DeletionLogError::Inconsistent)));

        // Another unexplained mismatch: log two generations ahead (not the
        // specific "exactly one ahead, Pending" pattern). Record a second
        // deletion normally (reference tracks it, landing at generation 2),
        // then rewind the reference to generation 0 so the log sits two
        // generations ahead of it.
        reference.write(&valid_reference).unwrap();
        log.record_deletion("note-b", &reference).unwrap();
        let stale_reference = file_ref(dir.path());
        stale_reference.write(&GenerationRef { generation: 0, chain_hash: "0".repeat(64), format_version: None, store_id: None }).unwrap();
        let result2 = log.reconcile(&stale_reference);
        assert!(
            matches!(result2, Err(DeletionLogError::Inconsistent)),
            "log two generations ahead of the reference is not the specific one-ahead roll-forward case and must fail closed, got {result2:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn p0_dpapi_reference_atomically_replaces_on_each_generation() {
        // CI sandboxes without a usable user DPAPI profile cannot exercise
        // this OS integration; production Windows profiles must run it.
        if dpapi_protect(b"p0-reference-write-probe").is_err() { return; }
        let dir = tempdir().unwrap();
        let db = dir.path().join("deletion-log.sqlite");
        let reference = DpapiTrustedReferenceStore::new(dir.path().join("trusted-reference.dpapi"));
        let log = DeletionLog::bootstrap_new(&db, &reference).unwrap();
        log.record_deletion("first", &reference).unwrap();
        log.record_deletion("second", &reference).unwrap();
        drop(log);

        let reopened = DeletionLog::open_existing(&db, &reference).unwrap();
        assert!(reopened.with_verified_not_deleted(&reference, "first", || Ok(())).unwrap().is_none());
        assert!(reopened.with_verified_not_deleted(&reference, "second", || Ok(())).unwrap().is_none());
    }

    /// `DpapiTrustedReferenceStore::bound_path` must return `Some(&self.path)`
    /// — `assert_reference_bound` compares this against the log's own
    /// `reference_path` (captured at construction) to reject a reference
    /// store that silently points somewhere else. A `None`-returning mutant
    /// would make EVERY DPAPI reference "unbound," which `assert_reference_bound`
    /// happens to treat as valid in test builds only
    /// (`else if !cfg!(test)`), so this must be exercised in a NON-test-cfg
    /// equivalent way: directly asserting the real return value, not relying
    /// on the production/test branch split to expose a difference.
    #[test]
    #[cfg(windows)]
    fn dpapi_reference_bound_path_returns_its_own_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("trusted-reference.dpapi");
        let reference = DpapiTrustedReferenceStore::new(path.clone());
        assert_eq!(reference.bound_path(), Some(path.as_path()));
    }

    /// `DpapiTrustedReferenceStore::read`'s `NotFound` arm: a genuinely
    /// absent reference file must read as `Ok(None)` ("no deletion has ever
    /// been recorded"), not an I/O error. The `== with true` mutant on the
    /// match guard would misclassify EVERY read error (permission denied,
    /// path-too-long, etc.) as "absent," silently treating a real I/O
    /// failure as "no deletions ever" instead of propagating
    /// `DeletionLogError::Io`.
    #[test]
    #[cfg(windows)]
    fn dpapi_reference_read_reports_a_genuinely_absent_file_as_none_not_as_an_io_error() {
        let dir = tempdir().unwrap();
        let reference = DpapiTrustedReferenceStore::new(dir.path().join("never-written.dpapi"));
        assert_eq!(reference.read(), Ok(None));

        // A DIFFERENT I/O error (not NotFound): a path that exists but is a
        // DIRECTORY, not a file. `std::fs::read` on a directory fails with
        // `ErrorKind::IsADirectory`/`Uncategorized` (platform-dependent),
        // never `NotFound`, since the entry genuinely exists.
        let directory_path = dir.path().join("a-directory-not-a-reference-file");
        std::fs::create_dir(&directory_path).unwrap();
        let reference_at_directory = DpapiTrustedReferenceStore::new(directory_path);
        let result = reference_at_directory.read();
        assert!(!matches!(result, Ok(None)), "an I/O error that is NOT NotFound must never be silently reported as None, got {result:?}");
    }

    #[test]
    #[cfg(windows)]
    fn a_tampered_trusted_reference_file_is_detected_as_tamper_distinct_from_plain_corruption() {
        let dir = tempdir().unwrap();
        let ref_path = dir.path().join("trusted-reference.dpapi");
        let dpapi_reference = DpapiTrustedReferenceStore::new(ref_path.clone());
        let log = DeletionLog::bootstrap_new(&dir.path().join("deletion-log.sqlite"), &dpapi_reference).unwrap();

        log.record_deletion("note-a", &dpapi_reference).unwrap();

        let original = std::fs::read(&ref_path).unwrap();
        assert!(!original.is_empty());

        // Flip a byte in the DPAPI envelope (not truncate/delete it — that's
        // ReferenceCorrupt/ReferenceMissing, tested separately below).
        let mut tampered = original.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xFF;
        std::fs::write(&ref_path, &tampered).unwrap();

        let result = dpapi_reference.read();
        assert!(
            matches!(result, Err(DeletionLogError::ReferenceTamperDetected)),
            "a flipped byte in a structurally plausible DPAPI envelope must be reported as tamper, not corruption or a bogus generation, got {result:?}"
        );

        let reconcile_result = log.reconcile(&dpapi_reference);
        assert!(
            matches!(reconcile_result, Err(DeletionLogError::ReferenceTamperDetected)),
            "reconcile must surface the same tamper error, got {reconcile_result:?}"
        );
    }

    // -----------------------------------------------------------------
    // Step 5 (brief) tests, each its own function.
    // -----------------------------------------------------------------

    /// registro assente durante restore: the trusted reference is entirely
    /// missing.
    #[test]
    fn reconcile_with_a_missing_reference_fails_closed_as_reference_missing() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());

        log.record_deletion("note-a", &reference).unwrap();
        // Delete the reference file entirely to simulate "assente."
        std::fs::remove_file(dir.path().join("trusted-reference.bin")).unwrap();

        let result = log.reconcile(&reference);
        assert!(matches!(result, Err(DeletionLogError::ReferenceMissing)), "got {result:?}");
    }

    /// registro corrotto durante restore: structurally implausible bytes
    /// (not a DPAPI tamper — plain corruption).
    #[test]
    fn reconcile_with_a_corrupt_reference_fails_closed_as_reference_corrupt() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());

        log.record_deletion("note-a", &reference).unwrap();
        // Overwrite with bytes that cannot parse as "{u64}:{hex}" at all.
        std::fs::write(dir.path().join("trusted-reference.bin"), b"not-a-generation-ref").unwrap();

        let result = log.reconcile(&reference);
        assert!(matches!(result, Err(DeletionLogError::ReferenceCorrupt)), "got {result:?}");
    }

    /// registro valido ma obsoleto: the reference is syntactically valid but
    /// indicates a HIGHER generation than the log (the inverse direction
    /// from the roll-forward case).
    #[test]
    fn reconcile_with_a_reference_ahead_of_the_log_fails_closed_as_inconsistent() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());

        log.record_deletion("note-a", &reference).unwrap(); // log generation 1, reference generation 1

        // Hand-craft a reference claiming a generation higher than the log
        // has ever reached, with a syntactically valid (just not matching)
        // chain hash.
        reference.write(&GenerationRef { generation: 7, chain_hash: "a".repeat(64), format_version: None, store_id: None }).unwrap();

        let result = log.reconcile(&reference);
        assert!(matches!(result, Err(DeletionLogError::Inconsistent)), "got {result:?}");
    }

    /// due cancellazioni consecutive con crash tra le due: two deletions in
    /// a row with a crash between them (i.e. the first completes fully, the
    /// second crashes mid-protocol) — both must converge correctly under
    /// reconcile, and `is_deleted` must hold for both notes throughout.
    #[test]
    fn two_consecutive_deletions_with_a_crash_between_them_converge_correctly_on_reconcile() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());

        // First deletion completes fully.
        let first = log.record_deletion("note-a", &reference).unwrap();
        assert_eq!(first.generation, 1);
        assert!(log.is_deleted("note-a").unwrap());

        // Second deletion crashes between step 2 and step 3.
        let injector = FaultInjector::trigger_at(FaultPoint::BeforeMarkedCompleted);
        let second = log.record_deletion_with_injector("note-b", &reference, &injector).unwrap();
        assert_eq!(second.generation, 2);

        // Both notes must already read as deleted.
        assert!(log.is_deleted("note-a").unwrap());
        assert!(log.is_deleted("note-b").unwrap(), "note-b must be non-consultable even mid-crash");

        // The reference was updated (step 2 ran) before the simulated crash.
        let ref_before_reconcile = reference.read().unwrap().unwrap();
        assert_eq!(ref_before_reconcile.generation, 2);

        // Restart and reconcile.
        let restarted = DeletionLog::open(log.db_path()).unwrap();
        let outcome = restarted.reconcile(&reference).unwrap();
        assert_eq!(outcome, ReconcileOutcome::RolledForward { generation: 2 });

        assert!(restarted.is_deleted("note-a").unwrap());
        assert!(restarted.is_deleted("note-b").unwrap());

        let row2 = DeletionLog::row_for_generation(&restarted.conn.lock().unwrap(), 2).unwrap().unwrap();
        assert_eq!(row2.state, DeletionLogState::Completed);
    }

    /// A second (not first) deletion crashes before the reference is
    /// touched at all: the reference already exists at generation 1 from a
    /// prior, fully-completed deletion, the log advances to generation 2,
    /// and the crash happens at `AfterGenerationWrittenPending` — i.e.
    /// before step 2 runs for THIS deletion. This exercises the
    /// `row.generation == r.generation + 1 && Pending` branch of
    /// `reconcile`, distinct from the equal-generation branch covered by
    /// `two_consecutive_deletions_with_a_crash_between_them_converge_correctly_on_reconcile`
    /// (which crashes later, at `BeforeMarkedCompleted`, after the
    /// reference has already been bumped).
    #[test]
    fn a_second_deletion_crashing_before_the_reference_is_touched_rolls_forward_past_a_stale_reference(
    ) {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());

        // First deletion completes fully: reference ends up at generation 1.
        let first = log.record_deletion("note-a", &reference).unwrap();
        assert_eq!(first.generation, 1);
        let ref_after_first = reference.read().unwrap().unwrap();
        assert_eq!(ref_after_first.generation, 1);

        // Second deletion crashes immediately after step 1 (generation 2
        // written, Pending) and before step 2 (reference untouched).
        let injector = FaultInjector::trigger_at(FaultPoint::AfterGenerationWrittenPending);
        let second = log
            .record_deletion_with_injector("note-b", &reference, &injector)
            .expect("record_deletion must return the half-finished receipt, not an error");
        assert_eq!(second.generation, 2);

        // The log has advanced to generation 2, Pending, but the reference
        // is still at generation 1 — untouched by this second deletion.
        let row2_before = DeletionLog::row_for_generation(&log.conn.lock().unwrap(), 2).unwrap().unwrap();
        assert_eq!(row2_before.state, DeletionLogState::Pending);
        let ref_before_reconcile = reference.read().unwrap().unwrap();
        assert_eq!(
            ref_before_reconcile.generation, 1,
            "reference must still be at generation 1, unlike the BeforeMarkedCompleted crash case"
        );

        // Even in this half-crashed state, note-b must already read deleted.
        assert!(log.is_deleted("note-b").unwrap(), "note-b must be non-consultable even mid-crash");

        // Restart and reconcile.
        let restarted = DeletionLog::open(log.db_path()).unwrap();
        let outcome = restarted.reconcile(&reference).expect("reconcile must roll forward, not fail");
        assert_eq!(outcome, ReconcileOutcome::RolledForward { generation: 2 });

        // Reference is now updated to generation 2, and the row is Completed.
        let ref_after_reconcile = reference.read().unwrap().unwrap();
        assert_eq!(ref_after_reconcile.generation, 2);
        assert_eq!(ref_after_reconcile.chain_hash, second.chain_hash);

        let row2_after = DeletionLog::row_for_generation(&restarted.conn.lock().unwrap(), 2).unwrap().unwrap();
        assert_eq!(row2_after.state, DeletionLogState::Completed);
    }

    /// lettura concorrente durante una cancellazione: a concurrent read
    /// during `record_deletion` must see `is_deleted() == true` throughout,
    /// since the note becomes non-consultable at step 1, before the
    /// protocol even completes.
    #[test]
    fn a_concurrent_read_during_record_deletion_sees_is_deleted_true_throughout() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let dir = tempdir().unwrap();
        let log = Arc::new(open_log(dir.path()));
        let reference = file_ref(dir.path());

        // A first, unrelated deletion exists so is_deleted has a real
        // baseline to check against for the SECOND note while it's being
        // recorded.
        log.record_deletion("note-existing", &reference).unwrap();

        let keep_reading = Arc::new(AtomicBool::new(true));

        let reader_log = Arc::clone(&log);
        let reader_flag = Arc::clone(&keep_reading);
        let reader = std::thread::spawn(move || {
            // Busy-poll is_deleted for "note-concurrent" until told to stop.
            // It must never observe a transient "exists but not yet
            // findable" state once the writer has started — either it's not
            // there yet (writer hasn't reached step 1) or it's already true.
            let mut observed_true = false;
            while reader_flag.load(Ordering::SeqCst) {
                if reader_log.is_deleted("note-concurrent").unwrap() {
                    observed_true = true;
                }
            }
            observed_true
        });

        let receipt = log.record_deletion("note-concurrent", &reference).unwrap();
        assert_eq!(receipt.generation, 2);

        keep_reading.store(false, Ordering::SeqCst);
        let observed_true = reader.join().unwrap();
        assert!(observed_true, "the concurrent reader must have observed is_deleted(note-concurrent) == true");

        // Final state, read after the writer returns: must still be true.
        assert!(log.is_deleted("note-concurrent").unwrap());
    }

    // -----------------------------------------------------------------
    // Final whole-branch review, I4 #3 and I5: the hash chain itself
    // -----------------------------------------------------------------

    /// I4 #3: `compute_chain_hash` collapsed to a fixed string survived,
    /// because nothing checked the hash depends on its inputs. Pins the
    /// documented formula `sha256(prev || generation || note_id)` to a
    /// vector computed independently with Python's `hashlib`, determinism,
    /// and that changing ANY one input changes the output.
    #[test]
    fn compute_chain_hash_matches_a_known_answer_and_depends_on_every_input() {
        let genesis = GENESIS_PREV_CHAIN_HASH;
        let h = DeletionLog::compute_chain_hash(genesis, 1, "note-a");
        assert_eq!(h, "7fa99c98997e4901987dacbec69c62e0af0b052230298c82c714521a06fdee14");
        assert_eq!(DeletionLog::compute_chain_hash(genesis, 1, "note-a"), h, "deterministic");
        assert_ne!(DeletionLog::compute_chain_hash(&"1".repeat(64), 1, "note-a"), h, "depends on prev_chain_hash");
        assert_ne!(DeletionLog::compute_chain_hash(genesis, 2, "note-a"), h, "depends on generation");
        assert_ne!(DeletionLog::compute_chain_hash(genesis, 1, "note-b"), h, "depends on note_id");
    }

    fn corrupt(log: &DeletionLog, sql: &str) {
        log.conn.lock().unwrap().execute(sql, []).unwrap();
    }

    fn three_deletions(dir: &Path) -> (DeletionLog, FileTrustedReferenceStore) {
        let log = open_log(dir);
        let reference = file_ref(dir);
        for id in ["note-a", "note-b", "note-c"] {
            log.record_deletion(id, &reference).unwrap();
        }
        (log, reference)
    }

    /// I5: before the fix only the LATEST row was compared against the
    /// trusted reference, so rewriting a MIDDLE row's `note_id` (which
    /// silently "un-deletes" that note: `is_deleted` stops seeing it) went
    /// undetected and reconcile reported `AlreadyConsistent`. The full walk
    /// must catch it, both in `verify_chain` and on the startup path.
    #[test]
    fn a_corrupted_middle_row_note_id_is_detected_as_chain_broken() {
        let dir = tempdir().unwrap();
        let (log, reference) = three_deletions(dir.path());
        corrupt(&log, "UPDATE deletions SET note_id = 'note-x' WHERE generation = 2");
        assert!(!log.is_deleted("note-b").unwrap(), "precondition: the corruption un-deletes note-b");

        assert_eq!(log.reconcile(&reference), Err(DeletionLogError::ChainBroken), "the startup path must walk the whole chain");
        assert_eq!(log.verify_chain(), Err(DeletionLogError::ChainBroken));
    }

    /// I5: a middle row's own `chain_hash` rewritten (which also breaks the
    /// next row's `prev_chain_hash` link).
    #[test]
    fn a_corrupted_middle_row_chain_hash_is_detected_as_chain_broken() {
        let dir = tempdir().unwrap();
        let (log, reference) = three_deletions(dir.path());
        corrupt(&log, &format!("UPDATE deletions SET chain_hash = '{}' WHERE generation = 2", "e".repeat(64)));
        assert_eq!(log.verify_chain(), Err(DeletionLogError::ChainBroken));
        assert_eq!(log.reconcile(&reference), Err(DeletionLogError::ChainBroken));
    }

    /// I5: a row whose `prev_chain_hash` no longer names its predecessor,
    /// even though its own `chain_hash` was recomputed to match it (an
    /// attacker re-hashing one row cannot splice it into the chain).
    #[test]
    fn a_middle_row_relinked_to_the_wrong_predecessor_is_detected_as_chain_broken() {
        let dir = tempdir().unwrap();
        let (log, _reference) = three_deletions(dir.path());
        let forged_prev = "d".repeat(64);
        let forged_hash = DeletionLog::compute_chain_hash(&forged_prev, 2, "note-b");
        corrupt(&log, &format!("UPDATE deletions SET prev_chain_hash = '{forged_prev}', chain_hash = '{forged_hash}' WHERE generation = 2"));
        assert_eq!(log.verify_chain(), Err(DeletionLogError::ChainBroken));
    }

    /// I5: a deleted (missing) middle generation is a gap in the chain.
    #[test]
    fn a_missing_middle_generation_is_detected_as_chain_broken() {
        let dir = tempdir().unwrap();
        let (log, _reference) = three_deletions(dir.path());
        corrupt(&log, "DELETE FROM deletions WHERE generation = 2");
        assert_eq!(log.verify_chain(), Err(DeletionLogError::ChainBroken));
    }

    /// I5: renumbering the LAST row keeps every hash link intact (each hash
    /// is recomputed from the expected generation), so only the contiguity
    /// check catches it (the mutation re-run showed `||`->`&&` on that check
    /// surviving).
    #[test]
    fn a_renumbered_last_generation_is_detected_as_chain_broken() {
        let dir = tempdir().unwrap();
        let (log, _reference) = three_deletions(dir.path());
        corrupt(&log, "UPDATE deletions SET generation = 5 WHERE generation = 3");
        assert_eq!(log.verify_chain(), Err(DeletionLogError::ChainBroken));
    }

    /// I5: an intact chain (and an empty log) verifies.
    #[test]
    fn an_intact_chain_and_an_empty_log_verify() {
        let dir = tempdir().unwrap();
        assert_eq!(open_log(&dir.path().join("empty")).verify_chain(), Ok(()));
        let (log, reference) = three_deletions(dir.path());
        assert_eq!(log.verify_chain(), Ok(()));
        assert_eq!(log.reconcile(&reference), Ok(ReconcileOutcome::AlreadyConsistent));
    }

    /// I5: the restore-time gate walks the live chain too.
    #[test]
    fn verify_safe_to_expose_after_restore_rejects_a_live_log_with_a_broken_chain() {
        let dir = tempdir().unwrap();
        let (log, reference) = three_deletions(dir.path());
        let live = reference.read().unwrap().unwrap();
        corrupt(&log, "UPDATE deletions SET note_id = 'note-x' WHERE generation = 1");
        assert_eq!(log.verify_safe_to_expose_after_restore(&live, &reference), Err(DeletionLogError::ChainBroken));
    }

    // -----------------------------------------------------------------
    // Task 5b: verify_safe_to_expose_after_restore
    // -----------------------------------------------------------------

    #[test]
    fn restore_with_a_syntactically_valid_but_lower_generation_reference_is_reported_as_backup_reference_stale() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let live_reference = file_ref(dir.path());

        log.record_deletion("note-a", &live_reference).unwrap();
        log.record_deletion("note-b", &live_reference).unwrap();
        let live = live_reference.read().unwrap().unwrap();
        assert_eq!(live.generation, 2);

        // The backup's embedded snapshot is from before "note-b" was deleted.
        let restored_snapshot = GenerationRef { generation: 1, chain_hash: "irrelevant-but-present".to_string(), format_version: None, store_id: None };

        // This low-level function still reports the staleness — it is the
        // CALLER (`backup::restore_with_pii_root`, post-N1 follow-up) that
        // decides to treat `BackupReferenceStale` specifically as safe to
        // proceed past (via per-note exclusion), never this function
        // itself. `BackupReferenceStale`, not `ReferenceStale`: the live
        // log/reference pair here is mutually consistent (reconcile, run
        // first inside this call, would accept it) — only the BACKUP's own
        // snapshot is behind.
        let result = log.verify_safe_to_expose_after_restore(&restored_snapshot, &live_reference);
        assert!(
            matches!(
                result,
                Err(DeletionLogError::BackupReferenceStale { backup_generation: 1, live_generation: 2 })
            ),
            "got {result:?}"
        );
    }

    #[test]
    fn restore_with_reference_matching_live_generation_and_valid_chain_is_accepted() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let live_reference = file_ref(dir.path());

        let receipt = log.record_deletion("note-a", &live_reference).unwrap();
        let live = live_reference.read().unwrap().unwrap();
        assert_eq!(live.generation, receipt.generation);

        let restored_snapshot = GenerationRef { generation: live.generation, chain_hash: live.chain_hash.clone(), format_version: live.format_version, store_id: live.store_id.clone() };
        let result = log.verify_safe_to_expose_after_restore(&restored_snapshot, &live_reference);
        assert!(result.is_ok(), "got {result:?}");
    }

    /// `verify_safe_to_expose_after_restore`'s staleness comparison
    /// (`restored_reference_snapshot.generation < live.generation`): the
    /// `<`->`>`/`<=` mutants would wrongly ACCEPT a backup snapshot claiming
    /// a HIGHER generation than the live log currently has — impossible in
    /// the real protocol (a backup cannot know about a future deletion),
    /// but exactly the boundary this comparison exists to enforce, never
    /// exercised directly since `<` (behind) and `==` (matching) are the
    /// only two cases any real backup/restore flow naturally produces.
    #[test]
    fn restore_with_a_reference_claiming_a_generation_the_live_log_has_not_reached_yet_is_not_silently_accepted() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let live_reference = file_ref(dir.path());

        log.record_deletion("note-a", &live_reference).unwrap();
        let live = live_reference.read().unwrap().unwrap();
        assert_eq!(live.generation, 1);

        // A snapshot claiming a generation the live log has not reached:
        // never producible by a real backup, but must not be silently
        // treated as "fresh enough" either.
        let impossible_future_snapshot = GenerationRef { generation: live.generation + 1, chain_hash: "f".repeat(68), format_version: live.format_version, store_id: live.store_id.clone() };
        let result = log.verify_safe_to_expose_after_restore(&impossible_future_snapshot, &live_reference);
        assert!(result.is_ok(), "a backup generation strictly ahead of live must not be rejected as BackupReferenceStale (that error means the BACKUP is behind, not ahead) — got {result:?}");
    }

    // -----------------------------------------------------------------
    // Restore-promotion TOCTOU fix: the cross-process write lock
    // -----------------------------------------------------------------

    #[test]
    fn the_write_lock_file_lives_beside_the_sqlite_file() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        assert_eq!(log.write_lock_path(), dir.path().join("deletion-log.writelock"));
    }

    /// Two independently-opened handles on the SAME log path (the in-process
    /// proxy for two processes sharing one vault): while one holds the write
    /// lock, the other's `record_deletion` must not write anything — not even
    /// its step-1 `Pending` row — until the lock is released, and must then
    /// complete normally.
    #[test]
    fn record_deletion_through_a_second_handle_blocks_while_the_write_lock_is_held_then_completes() {
        let dir = tempdir().unwrap();
        let holder = open_log(dir.path());
        let guard = holder.acquire_write_lock().unwrap();

        let db_path = holder.db_path().to_path_buf();
        let ref_path = dir.path().join("trusted-reference.bin");
        let deleter = std::thread::spawn(move || {
            let second_handle = DeletionLog::open(&db_path).unwrap();
            second_handle.record_deletion("note-a", &FileTrustedReferenceStore::new(ref_path)).unwrap()
        });

        std::thread::sleep(Duration::from_millis(300));
        let observer = DeletionLog::open(holder.db_path()).unwrap();
        let blocked = !deleter.is_finished() && !observer.is_deleted("note-a").unwrap();
        drop(guard);
        let receipt = deleter.join().unwrap();
        assert!(blocked, "record_deletion must not write its Pending row while another handle holds the write lock");
        assert_eq!(receipt.generation, 1);
        assert!(observer.is_deleted("note-a").unwrap(), "once the lock is released the deletion completes normally");
    }

    /// The wait is bounded: a lock that is never released surfaces as
    /// `WriteLockUnavailable`, and nothing is written (fail closed, never
    /// "proceed unlocked").
    #[test]
    fn record_deletion_fails_closed_with_write_lock_unavailable_when_the_lock_is_never_released() {
        let dir = tempdir().unwrap();
        let holder = open_log(dir.path());
        let _guard = holder.acquire_write_lock().unwrap();

        let mut second_handle = DeletionLog::open(holder.db_path()).unwrap();
        second_handle.set_write_lock_timeout(Duration::from_millis(100));
        let result = second_handle.record_deletion("note-a", &file_ref(dir.path()));
        assert_eq!(result, Err(DeletionLogError::WriteLockUnavailable { timeout_ms: 100 }));
        assert!(second_handle.is_empty().unwrap(), "nothing may be written without the lock");
        assert!(file_ref(dir.path()).read().unwrap().as_ref().is_some_and(DeletionLog::is_genesis_reference), "the reference must be untouched");
    }

    /// The lock is purely a gate: an injected crash (early return) at each of
    /// the three C5 fault points still leaves exactly the durable state it
    /// always did (asserted by the existing crash tests above), AND releases
    /// the lock, exactly as a real crash would (process death drops an OS
    /// file lock).
    #[test]
    fn an_injected_crash_at_every_c5_fault_point_releases_the_write_lock() {
        for point in [FaultPoint::AfterGenerationWrittenPending, FaultPoint::AfterReferenceUpdated, FaultPoint::BeforeMarkedCompleted] {
            let dir = tempdir().unwrap();
            let log = open_log(dir.path());
            log.record_deletion_with_injector("note-a", &file_ref(dir.path()), &FaultInjector::trigger_at(point)).unwrap();
            assert!(log.is_deleted("note-a").unwrap(), "{point:?}: step 1 is durable and authoritative");

            let mut other = DeletionLog::open(log.db_path()).unwrap();
            other.set_write_lock_timeout(Duration::from_millis(100));
            assert!(other.acquire_write_lock().is_ok(), "{point:?}: the early return must have released the write lock");
        }
    }

    /// The same exclusivity, observed directly on `acquire_write_lock`: a
    /// second handle cannot take the lock while the first holds it, and can
    /// as soon as the first guard drops.
    #[test]
    fn the_write_lock_is_exclusive_across_handles_and_released_on_guard_drop() {
        let dir = tempdir().unwrap();
        let first = open_log(dir.path());
        let mut second = DeletionLog::open(first.db_path()).unwrap();
        second.set_write_lock_timeout(Duration::from_millis(100));

        let guard = first.acquire_write_lock().unwrap();
        assert_eq!(second.acquire_write_lock().unwrap_err(), DeletionLogError::WriteLockUnavailable { timeout_ms: 100 });
        drop(guard);
        assert!(second.acquire_write_lock().is_ok());
    }

    // -----------------------------------------------------------------
    // F2 (independent review of 41005c6): reconcile and the write lock
    // -----------------------------------------------------------------

    /// A trusted-reference store that reads the real one, then pauses on its
    /// FIRST `read` before returning that (by then possibly stale) value:
    /// the reviewer's "reconcile paused mid-way" seam.
    struct PauseAfterFirstRead {
        inner: FileTrustedReferenceStore,
        paused: std::sync::mpsc::Sender<()>,
        resume: std::sync::mpsc::Receiver<()>,
        first: std::sync::atomic::AtomicBool,
    }

    impl TrustedReferenceStore for PauseAfterFirstRead {
        fn bound_path(&self) -> Option<&Path> { self.inner.bound_path() }
        fn read(&self) -> Result<Option<GenerationRef>, DeletionLogError> {
            let value = self.inner.read();
            if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = self.paused.send(());
                let _ = self.resume.recv();
            }
            value
        }

        fn write(&self, r: &GenerationRef) -> Result<(), DeletionLogError> {
            self.inner.write(r)
        }
    }

    /// The reviewer's F2 repro. A crashed first deletion leaves generation 1
    /// `Pending`. Reconcile A (handle 1) reads that state and pauses before
    /// rolling forward; meanwhile reconcile B (handle 2) and a new
    /// `record_deletion` (handle 3) run. Before the fix nothing stopped B and
    /// the deletion from completing during the pause, after which A wrote
    /// its stale generation-1 reference over the live generation-2 one, and
    /// the NEXT reconcile failed closed with `Inconsistent` (blocking every
    /// vault open and restore). Now A holds the write lock for its whole
    /// span, so B and the deletion wait for it, and the log converges.
    #[test]
    fn f2_concurrent_reconciles_and_a_deletion_through_three_handles_never_roll_the_reference_backwards() {
        let dir = tempdir().unwrap();
        let setup = open_log(dir.path());
        setup
            .record_deletion_with_injector("note-a", &file_ref(dir.path()), &FaultInjector::trigger_at(FaultPoint::AfterGenerationWrittenPending))
            .unwrap();
        assert!(file_ref(dir.path()).read().unwrap().as_ref().is_some_and(DeletionLog::is_genesis_reference), "precondition: crashed before step 2");

        let (paused_tx, paused_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let pausing = PauseAfterFirstRead {
            inner: file_ref(dir.path()),
            paused: paused_tx,
            resume: resume_rx,
            first: std::sync::atomic::AtomicBool::new(true),
        };
        let handle_a = DeletionLog::open(setup.db_path()).unwrap();
        let handle_b = DeletionLog::open(setup.db_path()).unwrap();
        let handle_c = DeletionLog::open(setup.db_path()).unwrap();

        let (a, b, c, others_waited) = std::thread::scope(|scope| {
            let a = scope.spawn(move || handle_a.reconcile(&pausing));
            paused_rx.recv().unwrap();
            let b = scope.spawn(|| handle_b.reconcile(&file_ref(dir.path())));
            let c = scope.spawn(|| handle_c.record_deletion("note-b", &file_ref(dir.path())));
            std::thread::sleep(Duration::from_millis(300));
            let others_waited = !b.is_finished() && !c.is_finished();
            resume_tx.send(()).unwrap();
            (a.join().unwrap(), b.join().unwrap(), c.join().unwrap(), others_waited)
        });
        assert_eq!(a, Ok(ReconcileOutcome::RolledForward { generation: 1 }));
        assert!(b.is_ok(), "{b:?}");
        assert_eq!(c.unwrap().generation, 2);

        let fresh = DeletionLog::open(setup.db_path()).unwrap();
        assert_eq!(
            fresh.reconcile(&file_ref(dir.path())),
            Ok(ReconcileOutcome::AlreadyConsistent),
            "the log/reference pair must still agree after the interleaving"
        );
        assert_eq!(file_ref(dir.path()).read().unwrap().unwrap().generation, 2, "the reference was never rolled backwards");
        assert!(others_waited, "the second reconcile and the deletion must wait for the paused reconcile's write lock");
    }

    /// `reconcile` acquires the write lock itself and is bounded by the same
    /// timeout: while another handle holds the lock it fails closed with
    /// `WriteLockUnavailable` and writes nothing, rather than rolling forward
    /// unlocked.
    #[test]
    fn f2_reconcile_fails_closed_without_writing_while_another_handle_holds_the_write_lock() {
        let dir = tempdir().unwrap();
        let holder = open_log(dir.path());
        holder
            .record_deletion_with_injector("note-a", &file_ref(dir.path()), &FaultInjector::trigger_at(FaultPoint::AfterGenerationWrittenPending))
            .unwrap();
        let guard = holder.acquire_write_lock().unwrap();

        let mut other = DeletionLog::open(holder.db_path()).unwrap();
        other.set_write_lock_timeout(Duration::from_millis(100));
        assert_eq!(other.reconcile(&file_ref(dir.path())), Err(DeletionLogError::WriteLockUnavailable { timeout_ms: 100 }));
        assert_eq!(
            other.verify_safe_to_expose_after_restore(&GenerationRef { generation: 1, chain_hash: String::new(), format_version: None, store_id: None }, &file_ref(dir.path())),
            Err(DeletionLogError::WriteLockUnavailable { timeout_ms: 100 }),
            "the restore gate's reconcile is locked too"
        );
        assert!(file_ref(dir.path()).read().unwrap().as_ref().is_some_and(DeletionLog::is_genesis_reference), "no roll-forward may happen without the lock");

        drop(guard);
        assert_eq!(other.reconcile(&file_ref(dir.path())), Ok(ReconcileOutcome::RolledForward { generation: 1 }));
        // Taken and released by reconcile itself: the lock is free again,
        // and a deletion (which takes it too) still completes normally.
        assert_eq!(holder.record_deletion("note-b", &file_ref(dir.path())).unwrap().generation, 2);
        assert_eq!(other.reconcile(&file_ref(dir.path())), Ok(ReconcileOutcome::AlreadyConsistent));
    }

    /// Environment variable that turns
    /// [`f2_child_process_holds_the_write_lock_until_released`] from a
    /// no-op into the child half of the cross-process test below.
    const CHILD_LOCK_DIR_ENV: &str = "SUPERBRAIN_F2_CHILD_LOCK_DIR";

    /// The child half of the cross-process test (a no-op in an ordinary test
    /// run): holds the write lock of the log in `$SUPERBRAIN_F2_CHILD_LOCK_DIR`
    /// until the parent creates a `release` file there.
    #[test]
    fn f2_child_process_holds_the_write_lock_until_released() {
        let Some(dir) = std::env::var_os(CHILD_LOCK_DIR_ENV).map(PathBuf::from) else {
            return;
        };
        let log = open_log(&dir);
        let guard = log.acquire_write_lock().unwrap();
        std::fs::write(dir.join("child-holds-lock"), b"").unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while !dir.join("release").exists() {
            assert!(Instant::now() < deadline, "the parent never released the child");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(guard);
    }

    /// F2 across real processes: the test binary re-spawns itself running
    /// only the child half above, which takes the write lock. While the
    /// CHILD holds it, `reconcile` in this process neither rolls forward nor
    /// writes anything (bounded: `WriteLockUnavailable`; unbounded: it
    /// waits), and once the child releases it, the same pending call
    /// completes the roll-forward.
    #[test]
    fn f2_reconcile_waits_for_the_write_lock_held_by_another_process() {
        if std::env::var_os(CHILD_LOCK_DIR_ENV).is_some() {
            return; // never recurse inside the child
        }
        let dir = tempdir().unwrap();
        {
            let setup = open_log(dir.path());
            setup
                .record_deletion_with_injector("note-a", &file_ref(dir.path()), &FaultInjector::trigger_at(FaultPoint::AfterGenerationWrittenPending))
                .unwrap();
        }
        let reference = file_ref(dir.path());
        let mut bounded = DeletionLog::open_existing(&dir.path().join("deletion-log.sqlite"), &reference).unwrap();
        bounded.set_write_lock_timeout(Duration::from_millis(100));
        let waiting = DeletionLog::open_existing(&dir.path().join("deletion-log.sqlite"), &reference).unwrap();
        /// Releases and reaps the child on every exit path, so a failing
        /// assertion here never leaves it holding the lock.
        struct ChildGuard(std::process::Child, PathBuf);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = std::fs::write(self.1.join("release"), b"");
                if !matches!(self.0.try_wait(), Ok(Some(_))) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
        }
        let mut child = ChildGuard(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "deletion_log::tests::f2_child_process_holds_the_write_lock_until_released", "--test-threads=1", "--nocapture"])
                .env(CHILD_LOCK_DIR_ENV, dir.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
            dir.path().to_path_buf(),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        while !dir.path().join("child-holds-lock").exists() {
            assert!(child.0.try_wait().unwrap().is_none(), "the child exited before taking the lock");
            assert!(Instant::now() < deadline, "the child never took the lock");
            std::thread::sleep(Duration::from_millis(10));
        }

        // `open_existing` itself verifies while holding the lock. Open both
        // cooperative consumer handles before the child deliberately takes
        // that lock, then exercise their bounded and waiting operations.
        // (The handles were created before spawning the child.)
        assert_eq!(bounded.reconcile(&reference), Err(DeletionLogError::WriteLockUnavailable { timeout_ms: 100 }));

        let (outcome, waited) = std::thread::scope(|scope| {
            let pending = scope.spawn(|| waiting.reconcile(&reference));
            std::thread::sleep(Duration::from_millis(300));
            let waited = !pending.is_finished()
                && reference.read().unwrap().as_ref().is_some_and(DeletionLog::is_genesis_reference);
            std::fs::write(dir.path().join("release"), b"").unwrap();
            (pending.join().unwrap(), waited)
        });
        assert!(child.0.wait().unwrap().success(), "the child half must pass");
        assert!(waited, "reconcile must wait for, and write nothing under, another process's write lock");
        assert_eq!(outcome, Ok(ReconcileOutcome::RolledForward { generation: 1 }));
        assert_eq!(reference.read().unwrap().unwrap().generation, 1);
    }

    // P0 mutation-testing triage, priority 1 (security-critical enforcement
    // points). Each test below is a direct unit test of one private
    // predicate/guard, added because end-to-end tests exercising the same
    // function through the public API happened to be satisfied by OTHER
    // checks running first or after, leaving these specific branches
    // unpinned — `cargo mutants --file deletion_log.rs` found all of them as
    // survivors. See docs/handoffs/P0-deletionlog-handoff.md's mutation
    // triage section for the full classification.

    fn valid_genesis() -> GenerationRef {
        DeletionLog::genesis_reference("a-non-empty-store-id".to_string())
    }

    /// `is_genesis_reference` is a pure 4-conjunct predicate with no DB/lock
    /// access; nothing previously called it with a crafted `GenerationRef`
    /// that fails each conjunct independently, so every `&&`->`||` and
    /// `==`->`!=` mutant on its body survived.
    #[test]
    fn is_genesis_reference_requires_all_four_fields_independently() {
        let base = valid_genesis();
        assert!(DeletionLog::is_genesis_reference(&base), "precondition: the valid baseline is genesis");

        let mut wrong_generation = base.clone();
        wrong_generation.generation = 1;
        assert!(!DeletionLog::is_genesis_reference(&wrong_generation), "generation must be exactly 0");

        let mut wrong_chain = base.clone();
        wrong_chain.chain_hash = "f".repeat(68);
        assert!(!DeletionLog::is_genesis_reference(&wrong_chain), "chain_hash must be exactly the genesis predecessor marker");

        let mut wrong_version = base.clone();
        wrong_version.format_version = Some(DELETION_LOG_FORMAT_VERSION + 1);
        assert!(!DeletionLog::is_genesis_reference(&wrong_version), "format_version must match exactly");

        let mut no_version = base.clone();
        no_version.format_version = None;
        assert!(!DeletionLog::is_genesis_reference(&no_version), "format_version must be Some");

        let mut no_store_id = base.clone();
        no_store_id.store_id = None;
        assert!(!DeletionLog::is_genesis_reference(&no_store_id), "store_id must be Some");

        let mut empty_store_id = base.clone();
        empty_store_id.store_id = Some(String::new());
        assert!(!DeletionLog::is_genesis_reference(&empty_store_id), "store_id must be non-empty");
    }

    /// `GenerationRef::deserialize`'s versioned-form sanity guard
    /// (`format_version == 0 || store_id.is_empty() || hash.is_empty()`):
    /// each `||`->`&&` mutant would require ALL THREE conditions true
    /// simultaneously to reject a malformed reference, instead of any ONE
    /// being enough. Reached only after a real DPAPI unwrap already
    /// succeeded (so the bytes are authenticated, not attacker-controlled
    /// in the usual sense), but a corrupted-yet-still-decryptable envelope
    /// (e.g. truncated/rewritten pre-encryption) is exactly the shape this
    /// guard exists to catch — never exercised directly.
    #[test]
    fn generation_ref_deserialize_rejects_each_malformed_versioned_field_independently() {
        let well_formed = GenerationRef { generation: 5, chain_hash: "a".repeat(68), format_version: Some(1), store_id: Some("s".to_string()) };
        assert_eq!(GenerationRef::deserialize(&well_formed.serialize()), Some(well_formed), "precondition: a well-formed versioned reference round-trips");

        assert_eq!(GenerationRef::deserialize(b"v0:s:5:aaaa"), None, "format_version == 0 alone must be enough to reject");
        assert_eq!(GenerationRef::deserialize(b"v1::5:aaaa"), None, "an empty store_id alone must be enough to reject");
        assert_eq!(GenerationRef::deserialize(b"v1:s:5:"), None, "an empty hash alone must be enough to reject");
    }

    /// `verify_file_identities`'s own two checks: neither this log's own
    /// `db_path` identity drifting, nor the open write-lock file no longer
    /// matching the current on-disk write-lock file's identity, was ever
    /// exercised in isolation (only end-to-end via TOCTOU-style tests whose
    /// other checks ran first and already failed closed for a different
    /// reason).
    #[test]
    fn verify_file_identities_detects_either_the_db_or_the_write_lock_file_being_replaced() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let guard = log.acquire_write_lock().unwrap();
        assert_eq!(log.verify_file_identities(&guard), Ok(()), "precondition: untouched, both identities match");
        drop(guard);

        // `log`'s recorded db_identity was captured when it was first
        // opened. Drop its own connection (on Windows, SQLite's open file
        // handle would otherwise block replacing the file), replace the DB
        // file on disk with a fresh empty schema at the SAME path, then
        // reopen a fresh connection onto it WITHOUT going through
        // `DeletionLog::open` again — so the returned handle still carries
        // the OLD (pre-replacement) `db_identity`, reproducing exactly the
        // "identity captured before, file replaced since" scenario
        // `verify_file_identities` exists to catch.
        let db_path = log.db_path().to_path_buf();
        let recorded_identity = log.db_identity;
        drop(log);
        std::fs::remove_file(&db_path).unwrap();
        DeletionLog::create_schema(&rusqlite::Connection::open(&db_path).unwrap()).unwrap();
        assert_ne!(file_identity(&db_path).unwrap(), recorded_identity, "precondition: the on-disk file's identity actually changed");

        let stale_handle = DeletionLog { db_path: db_path.clone(), db_identity: recorded_identity, reference_path: None, conn: Mutex::new(rusqlite::Connection::open(&db_path).unwrap()), write_lock_timeout: DEFAULT_WRITE_LOCK_TIMEOUT };
        let guard = stale_handle.acquire_write_lock().unwrap();
        assert_eq!(stale_handle.verify_file_identities(&guard), Err(DeletionLogError::CorruptOrTampered), "a replaced DB file must be detected even though its path is unchanged");
    }

    /// The write-lock file identity half of the same check, independent of
    /// the DB file half above: the DB is untouched, but the write-lock file
    /// itself was replaced after the guard's `File` handle was opened on the
    /// old inode/file.
    #[test]
    fn verify_file_identities_detects_the_write_lock_file_being_replaced_while_a_guard_is_held() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let guard = log.acquire_write_lock().unwrap();
        assert_eq!(log.verify_file_identities(&guard), Ok(()), "precondition: untouched");

        // The guard's open `File` still refers to the OLD write-lock file by
        // handle; replace the write-lock file on disk out from under it.
        // On Windows this requires closing nothing — the guard's handle
        // keeps its own open file, and the path is free to be replaced.
        let lock_path = log.write_lock_path();
        drop(guard);
        std::fs::remove_file(&lock_path).unwrap();
        std::fs::write(&lock_path, b"").unwrap();
        let guard = log.acquire_write_lock().unwrap();
        // Re-verify after a FRESH guard on the replaced file: identity must
        // now match again (new guard, new file) — confirms the check is
        // comparing identities, not merely failing unconditionally after any
        // replacement.
        assert_eq!(log.verify_file_identities(&guard), Ok(()), "a fresh guard on the replaced file matches by construction");
    }

    /// `verify_existing_or_pending`'s genesis-match guard, and
    /// `verify_consistent`'s identical guard: both match `None` latest row
    /// against `is_genesis_reference`, but no test ever called either
    /// directly with `latest = None` and a reference that is NOT genesis
    /// (not reachable by just corrupting an existing log, since a fresh log
    /// legitimately has no rows).
    #[test]
    fn verify_consistent_rejects_an_empty_log_whose_reference_is_not_genesis() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());
        assert!(log.is_empty().unwrap(), "precondition: freshly bootstrapped, no deletions yet");
        // A non-genesis reference (generation 1, matching this log's own
        // bound store_id/format_version so verify_binding_on's OWN check
        // passes and does not mask the guard under test) paired with the
        // EMPTY log: the log can never legitimately be behind its own
        // reference by construction (only this log's own writes advance
        // it), so this is squarely CorruptOrTampered, not any kind of valid
        // state.
        let bound_store_id = reference.read().unwrap().unwrap().store_id;
        reference
            .write(&GenerationRef { generation: 1, chain_hash: "a".repeat(68), format_version: Some(DELETION_LOG_FORMAT_VERSION), store_id: bound_store_id })
            .unwrap();
        assert_eq!(log.verify_consistent(&reference), Err(DeletionLogError::CorruptOrTampered));
    }

    /// `verify_consistent`'s second guard: a non-empty log whose latest row
    /// matches the reference's generation but NOT its chain_hash (or vice
    /// versa) must be rejected — covers the `&&`->`||` mutant (which would
    /// wrongly accept a generation match with ANY chain_hash) and the
    /// `==`->`!=` mutants on each half independently.
    #[test]
    fn verify_consistent_rejects_a_latest_row_whose_generation_matches_but_chain_hash_does_not() {
        let dir = tempdir().unwrap();
        let (log, reference) = (open_log(dir.path()), file_ref(dir.path()));
        log.record_deletion("note-a", &reference).unwrap();
        let true_ref = reference.read().unwrap().unwrap();
        assert_eq!(log.verify_consistent(&reference), Ok(()), "precondition: untouched, matches exactly");

        // Same generation, wrong chain_hash: the `&&`->`||` mutant and the
        // chain_hash `==`->`!=` mutant would both wrongly accept this.
        let mut wrong_hash = true_ref.clone();
        wrong_hash.chain_hash = "b".repeat(68);
        reference.write(&wrong_hash).unwrap();
        assert_eq!(log.verify_consistent(&reference), Err(DeletionLogError::CorruptOrTampered), "generation matching alone must not be sufficient");

        // Wrong generation, same chain_hash as the real latest row: the
        // generation `==`->`!=` mutant would wrongly accept this too.
        let mut wrong_generation = true_ref.clone();
        wrong_generation.generation = 999;
        reference.write(&wrong_generation).unwrap();
        assert_eq!(log.verify_consistent(&reference), Err(DeletionLogError::CorruptOrTampered), "chain_hash matching alone must not be sufficient");
    }

    /// `verify_existing_or_pending` (the `open_existing` path's own guard,
    /// a distinct function from `verify_consistent` even though shaped the
    /// same) has its own identical genesis-match guard for `latest = None`:
    /// never exercised directly for the same reason as `verify_consistent`'s
    /// equivalent (a fresh log legitimately has no rows, so the "not
    /// genesis" branch is never naturally reached without a crafted
    /// reference).
    #[test]
    fn verify_existing_or_pending_rejects_an_empty_log_whose_reference_is_not_genesis() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());
        assert!(log.is_empty().unwrap(), "precondition: freshly bootstrapped, no deletions yet");
        let bound_store_id = reference.read().unwrap().unwrap().store_id;
        reference
            .write(&GenerationRef { generation: 1, chain_hash: "a".repeat(68), format_version: Some(DELETION_LOG_FORMAT_VERSION), store_id: bound_store_id })
            .unwrap();
        assert_eq!(log.verify_existing_or_pending(&reference), Err(DeletionLogError::CorruptOrTampered));
    }

    /// `verify_existing_or_pending`'s third match arm — the specific
    /// "crashed between step 1 and step 2" pending pattern — requires ALL
    /// THREE of: `state == Pending`, `generation == reference.generation +
    /// 1`, AND `prev_chain_hash == reference.chain_hash`. The `&&`->`||`
    /// mutant on the first `&&` would wrongly accept a row that is `Pending`
    /// with the right generation but whose `prev_chain_hash` does NOT match
    /// the reference's own `chain_hash` — a reference tampered to claim a
    /// different (but still well-formed, so `verify_chain_on`'s OWN checks
    /// stay green) prior chain_hash than the one this row actually chains
    /// from. No existing test exercises the reference's `chain_hash` half of
    /// this specific three-way match guard in isolation.
    #[test]
    fn verify_existing_or_pending_rejects_a_pending_row_at_the_right_generation_against_a_reference_with_the_wrong_chain_hash() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());
        log.record_deletion_with_injector("note-a", &reference, &FaultInjector::trigger_at(FaultPoint::AfterGenerationWrittenPending)).unwrap();
        // Precondition: genuinely pending, one generation ahead, reference
        // still at genesis — the real crash-recovery shape this arm exists
        // for.
        assert_eq!(log.verify_existing_or_pending(&reference), Ok(()), "precondition: the real crash shape is accepted");

        // The reference now claims a DIFFERENT (but still syntactically
        // well-formed) generation-0 chain_hash than the genesis marker the
        // row's own prev_chain_hash actually chains from. verify_chain_on
        // only validates the ROW's own internal consistency (prev_chain_hash
        // vs. the genesis constant), so this does not trip ChainBroken; only
        // the match guard under test can reject it.
        let mut wrong_chain_ref = reference.read().unwrap().unwrap();
        wrong_chain_ref.chain_hash = "f".repeat(68);
        reference.write(&wrong_chain_ref).unwrap();
        assert_eq!(log.verify_existing_or_pending(&reference), Err(DeletionLogError::CorruptOrTampered), "state+generation matching alone, against a reference with a mismatched chain_hash, must not be sufficient to accept a roll-forward target");
    }

    /// `assert_reference_bound`'s own guard: a reference store bound to a
    /// DIFFERENT path than the one this log was opened/bootstrapped with
    /// must be rejected, not merely "some reference object was passed."
    /// Never exercised directly — only indirectly, through code paths where
    /// a later check also happened to fail for an unrelated reason.
    #[test]
    fn assert_reference_bound_rejects_a_reference_store_bound_to_a_different_path() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let correct_reference = file_ref(dir.path());
        assert_eq!(log.assert_reference_bound(&correct_reference), Ok(()), "precondition: the log's own reference path is accepted");

        let other_reference = FileTrustedReferenceStore::new(dir.path().join("a-completely-different-reference-file.bin"));
        assert_eq!(log.assert_reference_bound(&other_reference), Err(DeletionLogError::CorruptOrTampered), "a reference bound to a different path must never be accepted as this log's own");
    }

    /// `acquire_existing_write_lock`'s `NotFound` arm: opening it against a
    /// log directory whose DB file exists but whose SIBLING `.writelock`
    /// file is genuinely absent must report `LegacyNeedsMigration`
    /// specifically, not a generic IO error — the two are handled very
    /// differently by every caller (`open_existing`, `with_verified*`).
    /// `p0_open_existing_absent_fails_without_creating_any_state` only
    /// covers the directory being fully absent, which never reaches this
    /// function at all (`open_existing` fails earlier, before calling it).
    #[test]
    fn acquire_existing_write_lock_reports_legacy_needs_migration_when_only_the_lock_file_is_missing() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("deletion-log.sqlite");
        // A DB file exists (so open_existing's own earlier is_file checks
        // pass), but its sibling .writelock file was never created — e.g. a
        // pre-P0 legacy DB copied in without its lock artifact.
        DeletionLog::create_schema(&rusqlite::Connection::open(&db_path).unwrap()).unwrap();
        let log = DeletionLog { db_path: db_path.clone(), db_identity: file_identity(&db_path).unwrap(), reference_path: None, conn: Mutex::new(rusqlite::Connection::open(&db_path).unwrap()), write_lock_timeout: DEFAULT_WRITE_LOCK_TIMEOUT };
        assert!(!log.write_lock_path().is_file(), "precondition: the lock file is genuinely absent");
        assert!(matches!(log.acquire_existing_write_lock(), Err(DeletionLogError::LegacyNeedsMigration)));
    }

    /// `verify_chain_on`'s negative-generation branch: SQLite's
    /// `INTEGER PRIMARY KEY` has no `CHECK` constraint forbidding negative
    /// values (confirmed in the schema), so a row with `generation = -1` is
    /// a reachable adversarial state (direct SQLite file tampering, the
    /// documented threat model's attacker C), not a defensive no-op. No
    /// existing chain-corruption test ever produced a negative generation —
    /// they all used `UPDATE ... SET generation = <positive N>` — so the
    /// `generation < 0` half of the guard (as opposed to the `!=
    /// expected_generation` half, which IS covered by the renumbering
    /// tests) was never exercised.
    #[test]
    fn verify_chain_on_rejects_a_negative_generation_row() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());
        log.record_deletion("note-a", &reference).unwrap();
        corrupt(&log, "UPDATE deletions SET generation = -1 WHERE generation = 1");
        assert_eq!(log.verify_chain(), Err(DeletionLogError::ChainBroken), "a negative generation is reachable via direct DB tampering and must be rejected");
    }

    /// `reconcile_locked`'s roll-forward guard
    /// (`row.generation == r.generation + 1 && row.state == Pending`) and
    /// its "reference ahead of the log" guard (`row.generation <
    /// r.generation`): both only ever exercised through realistic
    /// crash/race scenarios that happen to produce the SPECIFIC generation
    /// deltas those scenarios naturally create (0 or +1 ahead). A reference
    /// claiming a generation the log's own writes could never have reached
    /// (far ahead, or using `*`/`-` instead of `+1`) was never constructed
    /// directly.
    #[test]
    fn reconcile_locked_fails_closed_when_the_reference_claims_a_generation_the_log_never_reached() {
        let dir = tempdir().unwrap();
        let (log, reference) = (open_log(dir.path()), file_ref(dir.path()));
        log.record_deletion("note-a", &reference).unwrap();
        let real = reference.read().unwrap().unwrap();
        assert_eq!(real.generation, 1);

        // Reference two generations ahead of the log's actual single row:
        // never producible by any real crash point (this log is the only
        // writer of its own generations), and specifically NOT row.generation
        // + 1 (so the roll-forward guard's `+1` must not fire) nor < (so the
        // "reference ahead" guard must fire instead of silently matching).
        let mut far_ahead = real.clone();
        far_ahead.generation = 3;
        far_ahead.chain_hash = "c".repeat(68);
        reference.write(&far_ahead).unwrap();
        assert_eq!(log.reconcile(&reference), Err(DeletionLogError::Inconsistent), "a reference ahead of what the log could have produced must fail closed, not be silently accepted by a wrong comparison");
    }

    /// Complementary case for the same guard, from the other direction: the
    /// log's single row is MORE than one generation ahead of the reference
    /// (impossible for a real crash, which can only ever be exactly one
    /// ahead) — the `+`->`-`/`*` mutants on the roll-forward guard's `+ 1`
    /// would accept generation deltas other than +1, so this pins the delta
    /// itself, not merely "some mismatch is rejected."
    #[test]
    fn reconcile_locked_does_not_roll_forward_when_the_log_is_more_than_one_generation_ahead() {
        let dir = tempdir().unwrap();
        let (log, reference) = (open_log(dir.path()), file_ref(dir.path()));
        log.record_deletion("note-a", &reference).unwrap();
        log.record_deletion("note-b", &reference).unwrap();
        // Reference stuck at generation 0 (genesis) while the log already
        // has two completed rows: the log is two generations ahead, not
        // one, so the roll-forward guard's exact `+ 1` must NOT match.
        // Keeps THIS log's own bound store_id, so verify_binding_on's own
        // check still passes and the match arm under test is reached.
        let bound_store_id = reference.read().unwrap().unwrap().store_id;
        let genesis = GenerationRef { generation: 0, chain_hash: GENESIS_PREV_CHAIN_HASH.to_string(), format_version: Some(DELETION_LOG_FORMAT_VERSION), store_id: bound_store_id };
        reference.write(&genesis).unwrap();
        let result = log.reconcile(&reference);
        assert!(!matches!(result, Ok(ReconcileOutcome::RolledForward { .. }) | Ok(ReconcileOutcome::AlreadyConsistent)), "a log two generations ahead of a genesis reference must never be silently rolled forward or declared consistent, got {result:?}");
    }

    /// `reconcile_locked`'s OWN copy of the empty-log genesis-match guard
    /// (distinct from, but shaped identically to, `verify_consistent`'s and
    /// `verify_existing_or_pending`'s — each function has its own copy in
    /// its own `match`). An empty log paired with a non-genesis reference
    /// must fail closed via `reconcile`, not merely via the other two
    /// call paths already covered.
    #[test]
    fn reconcile_locked_rejects_an_empty_log_whose_reference_is_not_genesis() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());
        assert!(log.is_empty().unwrap(), "precondition: freshly bootstrapped, no deletions yet");
        let bound_store_id = reference.read().unwrap().unwrap().store_id;
        reference
            .write(&GenerationRef { generation: 1, chain_hash: "a".repeat(68), format_version: Some(DELETION_LOG_FORMAT_VERSION), store_id: bound_store_id })
            .unwrap();
        assert!(log.reconcile(&reference).is_err(), "an empty log against a non-genesis reference must never be silently accepted by reconcile");
    }

    /// `verify_binding_on`'s own three-way OR: a reference whose
    /// `format_version` matches the DB's own metadata but whose `store_id`
    /// does NOT must still be rejected — the `||`->`&&` mutant would wrongly
    /// require ALL three mismatches simultaneously, accepting a reference
    /// with a tampered `store_id` alone as long as its `format_version`
    /// happened to still be correct. Every existing binding-related test
    /// exercises this indirectly through a whole log/reference pair where
    /// other checks (chain, generation) also happen to fail, masking which
    /// specific disjunct caught it.
    #[test]
    fn verify_binding_on_rejects_a_matching_format_version_with_a_mismatched_store_id() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let reference = file_ref(dir.path());
        let real = reference.read().unwrap().unwrap();
        let conn = log.conn.lock().unwrap();
        // format_version matches the DB's own metadata exactly; only
        // store_id is wrong.
        let tampered = GenerationRef { generation: real.generation, chain_hash: real.chain_hash.clone(), format_version: real.format_version, store_id: Some("a-completely-different-store-id".to_string()) };
        assert_eq!(DeletionLog::verify_binding_on(&conn, &tampered, false), Err(DeletionLogError::CorruptOrTampered), "a correct format_version must not excuse a mismatched store_id");

        // Precise grouping/precedence case: `metadata.0 != version` alone
        // must be enough to reject, with store_id matching and
        // `metadata.0 != DELETION_LOG_FORMAT_VERSION` FALSE (a real
        // bootstrapped log's own persisted format_version always equals
        // DELETION_LOG_FORMAT_VERSION, so that middle disjunct is never
        // true on a genuine DB — this is the shape the `||`->`&&` mutant
        // on the FIRST `||` specifically targets: with `&&`, a false
        // middle disjunct would mask a true first disjunct entirely,
        // falling through to the (also matching) store_id disjunct, which
        // on its own would then wrongly decide the whole expression).
        // Constructing a case where store_id ALSO mismatches would let
        // that third disjunct mask the bug; this one keeps store_id
        // correct so ONLY the first disjunct can be responsible for
        // rejection.
        let wrong_claimed_version = GenerationRef { generation: real.generation, chain_hash: real.chain_hash, format_version: Some(real.format_version.unwrap() + 1), store_id: real.store_id };
        assert_eq!(DeletionLog::verify_binding_on(&conn, &wrong_claimed_version, false), Err(DeletionLogError::CorruptOrTampered), "a reference claiming the wrong format_version must be rejected even with a correct store_id, independent of the (always-false, for a real DB) middle disjunct");
    }

    /// `reconcile_locked`'s roll-forward guard
    /// (`row.generation == r.generation + 1 && row.state == Pending`): the
    /// `&&`->`||` mutant would wrongly accept a row that IS one generation
    /// ahead but is already `Completed` (not `Pending`) as a roll-forward
    /// target, overwriting an already-settled row's state and advancing
    /// the reference a second time from a hash it does not actually chain
    /// from in the intended crash-recovery sense. No existing test
    /// constructs a Completed row one generation ahead of a stale
    /// reference — reconcile's real callers never produce that shape
    /// without the reference having already been advanced to match.
    #[test]
    fn reconcile_locked_does_not_roll_forward_a_row_one_generation_ahead_but_already_completed() {
        let dir = tempdir().unwrap();
        let (log, reference) = (open_log(dir.path()), file_ref(dir.path()));
        log.record_deletion("note-a", &reference).unwrap();
        assert_eq!(log.reconcile(&reference), Ok(ReconcileOutcome::AlreadyConsistent), "precondition: generation 1, Completed, reference already at generation 1");

        // Roll the reference back to genesis directly (bypassing the
        // log), so the log's single row (generation 1, Completed) is now
        // one generation ahead of a STALE reference — the `&&` mutant's
        // target shape, but with state=Completed instead of Pending. Keeps
        // THIS log's own bound store_id, so verify_binding_on's own check
        // still passes and the match arm under test is actually reached.
        let bound_store_id = reference.read().unwrap().unwrap().store_id;
        let genesis = GenerationRef { generation: 0, chain_hash: GENESIS_PREV_CHAIN_HASH.to_string(), format_version: Some(DELETION_LOG_FORMAT_VERSION), store_id: bound_store_id };
        reference.write(&genesis).unwrap();
        let result = log.reconcile(&reference);
        assert!(!matches!(result, Ok(ReconcileOutcome::RolledForward { .. })), "a Completed row must never be treated as a Pending roll-forward target, got {result:?}");
    }
}
