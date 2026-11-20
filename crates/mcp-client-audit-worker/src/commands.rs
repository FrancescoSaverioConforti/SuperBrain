//! Internal command enum this crate's single worker task consumes --
//! mirrors `brain_api_audit_worker::commands`'s own exact shape and
//! ordering guarantees, adapted to this crate's own event type.

use crate::error::DurableRecordError;
use crate::event::McpClientEvent;

/// What a durable, caller-awaited submission receives once the record is
/// confirmed written and fsync'd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableAck {
    pub sequence: u64,
    pub record_hash: String,
}

/// One unit of work the worker task's own single consume-loop processes
/// strictly in FIFO order -- there is no reordering, no priority queue,
/// and no concurrent processing of two commands at once; the single
/// `while let Some(command) = rx.recv().await` loop IS the
/// serialization, the same design `brain_api_audit_worker::worker`'s own
/// doc comment establishes.
pub(crate) enum Command {
    /// Fire-and-forget: the caller does not wait for durability, only
    /// for the enqueue to have been attempted (see
    /// [`crate::handle::AuditWorkerHandle::record_best_effort`]).
    BestEffort { event: McpClientEvent },
    /// The caller awaits `respond_to` for the real durability outcome --
    /// never resolved before the underlying write genuinely completed
    /// (or definitively failed).
    Durable { event: McpClientEvent, respond_to: tokio::sync::oneshot::Sender<Result<DurableAck, DurableRecordError>> },
    /// Graceful shutdown: every command already enqueued BEFORE this one
    /// is fully processed first (FIFO drain); `respond_to` resolves only
    /// after that drain completes and the worker loop is about to exit,
    /// and nothing enqueued after this is ever processed.
    Shutdown { respond_to: tokio::sync::oneshot::Sender<()> },
}
