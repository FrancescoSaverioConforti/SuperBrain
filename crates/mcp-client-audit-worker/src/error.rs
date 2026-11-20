//! Typed errors for every failure mode this crate's own contract names
//! explicitly -- mirrors `brain_api_audit_worker::error`'s own exact
//! structure and naming (queue closure, writer poisoning, I/O failure,
//! startup recovery, and "the service is not available" kept as
//! distinct variants, never collapsed into one generic error), adapted
//! to this crate's own injected-writer boundary (`crate::writer::
//! WriterAppendError`) instead of a direct `storage` dependency.

/// Why a best-effort, fire-and-forget enqueue could not even be
/// ACCEPTED onto the queue -- never about durability.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnqueueError {
    /// The bounded queue is completely full RIGHT NOW. Never silently
    /// dropped -- the caller decides whether to drop this one event or
    /// apply backpressure.
    #[error("mcp client audit worker queue is full")]
    QueueFull,
    /// The worker task is gone (panicked, or already shut down) -- no
    /// command submitted after this will ever be processed.
    #[error("mcp client audit worker is no longer running")]
    WorkerGone,
}

/// Why a durable, acknowledgment-awaiting submission did not (and will
/// not) succeed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DurableRecordError {
    /// Same meaning as [`EnqueueError::QueueFull`], for the durable path.
    #[error("mcp client audit worker queue is full")]
    QueueFull,
    /// Same meaning as [`EnqueueError::WorkerGone`].
    #[error("mcp client audit worker is no longer running")]
    WorkerGone,
    /// The injected [`crate::writer::DurableAuditWriter`] is poisoned by
    /// an earlier ambiguous I/O failure and refuses every further
    /// append. The caller must not retry on this same worker; a fresh
    /// worker ([`crate::spawn_worker`] again, with a fresh writer) is
    /// required.
    #[error("audit writer is poisoned after an earlier ambiguous I/O failure")]
    WriterPoisoned,
    /// The log bytes were durably appended and fsync'd, but the
    /// subsequent head-reference update failed -- carried through
    /// unchanged from [`crate::writer::WriterAppendError::ReferenceUpdateFailed`].
    /// NOT poisoning: this one submitted command did not complete its
    /// full durability contract end to end, so the worker surfaces it
    /// as a failure to this one caller rather than silently reporting
    /// success, even though the underlying log is self-consistent.
    #[error("audit log record was durably appended, but the head reference update failed: {0}")]
    ReferenceUpdateFailed(String),
    /// A plain I/O failure during append/fsync (not the ambiguous case
    /// that poisons the writer).
    #[error("audit log append I/O error: {0}")]
    Io(String),
}

/// Why `spawn_worker` itself could not start -- distinguished from every
/// later per-command failure above, since nothing has been accepted
/// onto any queue yet when this happens.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerStartupError {
    #[error("startup recovery scan requires an explicit recovery procedure before this worker may start: {0:?}")]
    RecoveryRequired(RecoveryError),
}

/// What startup recovery found that prevents immediate, safe
/// resumption -- this crate's own classification of
/// `audit_chain_core::may_resume_without_recovery`'s `false` case,
/// mirroring `brain_api_audit_worker::RecoveryError`'s own three
/// variants exactly. This crate does not implement a repair procedure
/// of its own for any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryError {
    /// The existing log's tail is an unparseable partial record sitting
    /// at the true physical end of the file -- the one shape a future,
    /// separately authorized recovery task could safely resume past.
    PartialTrailingRecord,
    /// An unparseable line was found that is NOT at the physical tail --
    /// corruption in the middle of the file.
    UnparseableNotAtTail,
    /// A line parsed structurally but failed chain linkage (corruption,
    /// a sequence gap/duplicate, or a genuine fork).
    LinkBroken,
}
