//! The command envelope sent over the worker's own bounded channel —
//! the ONLY way any caller reaches the worker's owned `AuditWriterHandle`
//! and in-memory chain-tail state.

use crate::error::DurableRecordError;

/// Successful outcome of a durable, acknowledgment-awaiting submission —
/// the sequence/hash the worker actually assigned this record, so a
/// caller that cares (tests, diagnostics) can observe exactly what
/// landed, without the worker handing back the full serialized bytes
/// (those are an internal, replayable fact, not something a caller needs
/// to re-derive from).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableAck {
    pub sequence: u64,
    pub record_hash: String,
}

/// One unit of work the dedicated writer task dequeues and processes to
/// completion before looking at the next one — this strict one-at-a-time
/// processing IS the serialization; no mutex is needed inside the
/// worker.
pub(crate) enum Command {
    /// Best-effort: the caller does not await a durability
    /// acknowledgment at all (`AuditSink::record`'s own synchronous,
    /// fire-and-forget contract) — enqueuing this onto the channel is
    /// itself the only acknowledgment this path ever gives; this crate's
    /// `AuditSink` adapter return value is nothing beyond "did the
    /// enqueue itself succeed," never "was it persisted."
    BestEffort { event: brain_api::AuditEvent },
    /// Durable: the caller awaits `respond_to` for the complete
    /// append+fsync+reference-update outcome. Dropping the receiving
    /// end (caller cancelled) does NOT un-enqueue this command — the
    /// worker still processes it to completion and simply discards the
    /// result if nobody is listening anymore (see
    /// `AuditWorkerHandle::record_durable`'s own doc comment for the
    /// "the write still happened" contract this preserves).
    Durable {
        event: brain_api::AuditEvent,
        respond_to: tokio::sync::oneshot::Sender<Result<DurableAck, DurableRecordError>>,
    },
    /// Graceful shutdown: the worker finishes every command already
    /// enqueued BEFORE this one (strict FIFO — a bounded `mpsc` channel
    /// preserves submission order), then exits its loop without
    /// accepting anything after this. `respond_to` resolves once the
    /// worker task has actually exited (not merely once this command
    /// was dequeued), so a caller awaiting it observes TRUE drain
    /// completion.
    Shutdown { respond_to: tokio::sync::oneshot::Sender<()> },
}
