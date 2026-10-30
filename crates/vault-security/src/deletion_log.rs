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

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::keys::{dpapi_protect, dpapi_unprotect};

/// Genesis `prev_chain_hash` for the very first deletion-log entry: 64 `'0'`
/// characters, the same width as a real sha256 hex digest, so the chain
/// formula `sha256(prev_chain_hash || generation || note_id)` has a
/// well-defined input even for generation 1.
const GENESIS_PREV_CHAIN_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

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
}

impl GenerationRef {
    /// Serializes as `"{generation}:{chain_hash}"`. A simple, explicit
    /// format chosen because the bytes are then wrapped by
    /// DPAPI/authenticated storage — this layer does not need to defend
    /// against a hostile parser, only to round-trip unambiguously (a
    /// `chain_hash` is a fixed-width hex string, so a single `:` separator
    /// is sufficient and unambiguous).
    fn serialize(&self) -> Vec<u8> {
        format!("{}:{}", self.generation, self.chain_hash).into_bytes()
    }

    /// Inverse of [`GenerationRef::serialize`]. Returns `None` if `bytes`
    /// don't match the `"{u64}:{hex}"` shape at all — the caller maps that
    /// to `DeletionLogError::ReferenceCorrupt`.
    fn deserialize(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let (gen_str, hash) = text.split_once(':')?;
        let generation = gen_str.parse::<u64>().ok()?;
        if hash.is_empty() {
            return None;
        }
        Some(GenerationRef { generation, chain_hash: hash.to_string() })
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

impl TrustedReferenceStore for DpapiTrustedReferenceStore {
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
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        }
        std::fs::write(&self.path, wrapped).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        Ok(())
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

    /// A chained entry's `chain_hash` does not match
    /// `sha256(prev_chain_hash || generation || note_id)`.
    #[error("deletion log hash chain is broken")]
    ChainBroken,

    /// The trusted reference's generation is strictly behind the log's (or
    /// behind what a restore is being checked against) — a syntactically
    /// valid but obsolete reference, per C5 "valido ma obsoleto."
    #[error("trusted reference is stale: log is at generation {log_generation}, reference at {reference_generation}")]
    ReferenceStale { log_generation: u64, reference_generation: u64 },

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
    /// Serializes access to this log's own SQLite connection within this
    /// process, so a concurrent read during `record_deletion` (same
    /// `DeletionLog` instance, different thread) observes a consistent
    /// view rather than racing on connection setup. SQLite itself
    /// serializes writers across processes/files; this mutex additionally
    /// protects the single `rusqlite::Connection` this struct owns.
    conn: Mutex<Connection>,
}

impl DeletionLog {
    /// Opens (creating if necessary) the deletion-log SQLite database at
    /// `db_path`, e.g. `<root>/deletion-log.sqlite`.
    pub fn open(db_path: &Path) -> Result<Self, DeletionLogError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        }
        let conn = Connection::open(db_path).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS deletions (
                generation INTEGER PRIMARY KEY,
                note_id TEXT NOT NULL,
                prev_chain_hash TEXT NOT NULL,
                chain_hash TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('pending','completed'))
            );",
        )
        .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        Ok(Self { db_path: db_path.to_path_buf(), conn: Mutex::new(conn) })
    }

    /// The path to this log's own SQLite file.
    pub fn db_path(&self) -> &Path {
        &self.db_path
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
        trusted_ref.write(&GenerationRef { generation: next_generation, chain_hash: chain_hash.clone() })?;

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
    pub(crate) fn is_empty(&self) -> Result<bool, DeletionLogError> {
        let conn = self.conn.lock().expect("deletion log mutex poisoned");
        Ok(Self::latest_row(&conn)?.is_none())
    }

    /// Returns `true` if `note_id` has a deletion entry in ANY state
    /// (`Pending` or `Completed`). Per C5, the note becomes non-consultable
    /// starting at step 1 — a `Pending` entry must already read as deleted.
    pub fn is_deleted(&self, note_id: &str) -> Result<bool, DeletionLogError> {
        let conn = self.conn.lock().expect("deletion log mutex poisoned");
        let mut stmt = conn
            .prepare("SELECT 1 FROM deletions WHERE note_id = ?1 LIMIT 1")
            .map_err(|e| DeletionLogError::Io(e.to_string()))?;
        let mut rows = stmt.query([note_id]).map_err(|e| DeletionLogError::Io(e.to_string()))?;
        Ok(rows.next().map_err(|e| DeletionLogError::Io(e.to_string()))?.is_some())
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
    pub fn reconcile(&self, trusted_ref: &dyn TrustedReferenceStore) -> Result<ReconcileOutcome, DeletionLogError> {
        let conn = self.conn.lock().expect("deletion log mutex poisoned");
        // I5: the whole chain must verify before anything is rolled forward
        // or declared consistent.
        Self::verify_chain_on(&conn)?;
        let latest = Self::latest_row(&conn)?;
        let reference = trusted_ref.read()?;

        match (latest, reference) {
            (None, None) => Ok(ReconcileOutcome::AlreadyConsistent),
            (None, Some(r)) => {
                // The reference claims a generation but the log has no
                // rows at all: the log can never be ahead by construction,
                // but a reference pointing at a nonexistent generation is
                // exactly the kind of disagreement that must fail closed.
                Err(DeletionLogError::ReferenceStale { log_generation: 0, reference_generation: r.generation })
            }
            (Some(row), None) => {
                // No reference at all. Two genuinely different scenarios
                // produce this exact same observable state:
                //
                // 1. The very first deletion ever recorded crashed between
                //    step 1 (generation 1 written, Pending) and step 2 (the
                //    reference, which has therefore never been written at
                //    all yet — not merely stale, but nonexistent). This is
                //    the roll-forward case: safe, because nothing has ever
                //    been exposed under a prior reference and this log is
                //    the only writer of its own chain.
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
                if row.generation == 1 && row.state == DeletionLogState::Pending {
                    trusted_ref.write(&GenerationRef { generation: row.generation, chain_hash: row.chain_hash.clone() })?;
                    conn.execute(
                        "UPDATE deletions SET state = ?1 WHERE generation = ?2",
                        rusqlite::params![DeletionLogState::Completed.as_sql(), row.generation as i64],
                    )
                    .map_err(|e| DeletionLogError::Io(e.to_string()))?;
                    Ok(ReconcileOutcome::RolledForward { generation: row.generation })
                } else {
                    Err(DeletionLogError::ReferenceMissing)
                }
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
                    // can have produced a DIFFERENT generation N here: this
                    // log is the only writer of its own chain.
                    trusted_ref.write(&GenerationRef { generation: row.generation, chain_hash: row.chain_hash.clone() })?;
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
    /// [`DeletionLogError::ReferenceStale`].
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
        // The live log's own head must agree with the live reference before
        // this gate trusts either of them for the comparison below.
        self.reconcile(live_trusted_ref)?;
        let live = live_trusted_ref.read()?.ok_or(DeletionLogError::ReferenceMissing)?;
        if restored_reference_snapshot.generation < live.generation {
            return Err(DeletionLogError::ReferenceStale {
                log_generation: restored_reference_snapshot.generation,
                reference_generation: live.generation,
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
        DeletionLog::open(&dir.join("deletion-log.sqlite")).expect("open deletion log")
    }

    fn file_ref(dir: &Path) -> FileTrustedReferenceStore {
        FileTrustedReferenceStore::new(dir.join("trusted-reference.bin"))
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

        // The reference was never written (crash before step 2).
        assert!(reference.read().unwrap().is_none(), "reference must still be stale/absent before reconcile");

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

        // Now hand-craft a reference claiming generation 5 (ahead of the
        // log, which has never reached generation 5) with a hash that
        // cannot possibly be valid for this chain.
        reference
            .write(&GenerationRef { generation: 5, chain_hash: "f".repeat(64) })
            .unwrap();

        let result = log.reconcile(&reference);
        assert!(
            matches!(result, Err(DeletionLogError::Inconsistent)),
            "reference ahead of the log must fail closed as Inconsistent, got {result:?}"
        );

        // Another unexplained mismatch: log two generations ahead (not the
        // specific "exactly one ahead, Pending" pattern). Record a second
        // deletion normally (reference tracks it, landing at generation 2),
        // then rewind the reference to generation 0 so the log sits two
        // generations ahead of it.
        log.record_deletion("note-b", &file_ref(dir.path())).unwrap();
        let stale_reference = file_ref(dir.path());
        stale_reference.write(&GenerationRef { generation: 0, chain_hash: "0".repeat(64) }).unwrap();
        let result2 = log.reconcile(&stale_reference);
        assert!(
            matches!(result2, Err(DeletionLogError::Inconsistent)),
            "log two generations ahead of the reference is not the specific one-ahead roll-forward case and must fail closed, got {result2:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn a_tampered_trusted_reference_file_is_detected_as_tamper_distinct_from_plain_corruption() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let ref_path = dir.path().join("trusted-reference.dpapi");
        let dpapi_reference = DpapiTrustedReferenceStore::new(ref_path.clone());

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
        reference.write(&GenerationRef { generation: 7, chain_hash: "a".repeat(64) }).unwrap();

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
    fn restore_with_a_syntactically_valid_but_lower_generation_reference_is_rejected_as_stale_not_accepted() {
        let dir = tempdir().unwrap();
        let log = open_log(dir.path());
        let live_reference = file_ref(dir.path());

        log.record_deletion("note-a", &live_reference).unwrap();
        log.record_deletion("note-b", &live_reference).unwrap();
        let live = live_reference.read().unwrap().unwrap();
        assert_eq!(live.generation, 2);

        // The backup's embedded snapshot is from before "note-b" was deleted.
        let restored_snapshot = GenerationRef { generation: 1, chain_hash: "irrelevant-but-present".to_string() };

        let result = log.verify_safe_to_expose_after_restore(&restored_snapshot, &live_reference);
        assert!(
            matches!(
                result,
                Err(DeletionLogError::ReferenceStale { log_generation: 1, reference_generation: 2 })
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

        let restored_snapshot = GenerationRef { generation: live.generation, chain_hash: live.chain_hash.clone() };
        let result = log.verify_safe_to_expose_after_restore(&restored_snapshot, &live_reference);
        assert!(result.is_ok(), "got {result:?}");
    }
}
