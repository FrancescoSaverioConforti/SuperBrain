//! Typed errors for every failure mode this crate's own contract names
//! explicitly — queue closure, writer poisoning, I/O failure, startup
//! recovery, and "the service is not available" are kept as distinct
//! variants rather than collapsed into one generic error, so a caller
//! can tell "retry might help" (queue momentarily full — not modeled as
//! an error at all, see [`crate::EnqueueError`]) apart from "this
//! service will never accept another write" (poisoned/closed).

/// Why a best-effort [`brain_api::AuditSink::record`]-shaped enqueue
/// could not even be ACCEPTED onto the queue (never about durability —
/// a successful enqueue here makes no durability claim at all; see this
/// crate's own top-level doc comment and [`crate::handle::AuditWorkerHandle`]'s).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnqueueError {
    /// The bounded queue is completely full RIGHT NOW. Never silently
    /// dropped — the caller decides (per its own best-effort contract)
    /// whether to drop this one event or apply backpressure.
    #[error("audit worker queue is full")]
    QueueFull,
    /// The worker task is gone (panicked, or already shut down) — no
    /// command submitted after this will ever be processed.
    #[error("audit worker is no longer running")]
    WorkerGone,
}

/// Why a durable, acknowledgment-awaiting submission did not (and will
/// not) succeed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DurableRecordError {
    /// Same meaning as [`EnqueueError::QueueFull`], for the durable path.
    #[error("audit worker queue is full")]
    QueueFull,
    /// Same meaning as [`EnqueueError::WorkerGone`].
    #[error("audit worker is no longer running")]
    WorkerGone,
    /// The worker's own `AuditWriterHandle` is poisoned (an earlier,
    /// ambiguous I/O failure) and refuses every further append. The
    /// caller must not retry on this same worker; a fresh worker
    /// (`spawn_worker` again) re-runs full startup recovery from
    /// scratch.
    #[error("audit writer is poisoned after an earlier ambiguous I/O failure")]
    WriterPoisoned,
    /// The log bytes were durably appended and fsync'd, but the
    /// subsequent head-reference update failed
    /// (`storage::AuditAppendError::ReferenceUpdateFailed`'s own exact
    /// meaning, carried through unchanged) — NOT poisoning on the
    /// storage side, but THIS specific submitted command did not
    /// complete its full durability contract end to end, so the worker
    /// surfaces it as a failure to this one caller rather than silently
    /// reporting success.
    #[error("audit log record was durably appended, but the head reference update failed: {0}")]
    ReferenceUpdateFailed(String),
    /// A plain I/O failure during append/fsync (not the ambiguous case
    /// that poisons the writer — see `storage::AuditAppendError::Io`'s
    /// own doc comment for which failures poison vs. do not).
    #[error("audit log append I/O error: {0}")]
    Io(String),
}

/// Why `spawn_worker` itself could not start — distinguished from every
/// later per-command failure above, since nothing has been accepted
/// onto any queue yet when this happens.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerStartupError {
    #[error("failed to open the audit writer: {0}")]
    OpenFailed(String),
    #[error("startup recovery scan requires an explicit recovery procedure before this worker may start: {0:?}")]
    RecoveryRequired(RecoveryError),
}

/// What startup recovery found that prevents immediate, safe resumption
/// — this crate's own classification of
/// `brain_api::may_resume_without_recovery`'s `false` case, carrying
/// enough detail for an operator/caller to understand WHY, without this
/// crate inventing a repair procedure of its own (none is authorized in
/// this task).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// The existing log's tail is an unparseable partial record sitting
    /// at the true physical end of the file — the one shape a FUTURE,
    /// separately authorized recovery task could safely resume past.
    /// This crate does not implement that resumption.
    PartialTrailingRecord,
    /// An unparseable line was found that is NOT at the physical tail —
    /// corruption in the middle of the file.
    UnparseableNotAtTail,
    /// A line parsed structurally but failed chain linkage (corruption,
    /// a sequence gap/duplicate, or a genuine fork).
    LinkBroken,
}
