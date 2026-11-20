//! The caller-facing handle this crate's worker exposes -- mirrors
//! `brain_api_audit_worker::handle::AuditWorkerHandle`'s own exact
//! public contract.

use crate::commands::{Command, DurableAck};
use crate::error::{DurableRecordError, EnqueueError};
use crate::event::McpClientEvent;

/// A cheaply `Clone`-able reference to the one running worker task --
/// every clone shares the same bounded channel, so every caller submits
/// into the SAME single-consumer queue (never a second, independent
/// writer). Dropping every `AuditWorkerHandle` clone (without an
/// explicit [`AuditWorkerHandle::shutdown`] call) closes the channel's
/// sending side; the worker task then observes `rx.recv()` returning
/// `None` and exits on its own -- not a graceful FIFO drain in that
/// case, since no `Shutdown` command was ever sent (an operator wanting
/// the FIFO-drain guarantee must call `shutdown()` explicitly).
#[derive(Debug, Clone)]
pub struct AuditWorkerHandle {
    tx: tokio::sync::mpsc::Sender<Command>,
}

impl AuditWorkerHandle {
    pub(crate) fn new(tx: tokio::sync::mpsc::Sender<Command>) -> Self {
        Self { tx }
    }

    /// Submits `event` and waits for the real durability outcome --
    /// never resolves before the record is genuinely written and
    /// fsync'd (or definitively failed). Uses the channel's own bounded
    /// `send(...).await` (not `try_send`), so this call applies natural
    /// backpressure under a full queue rather than failing with
    /// [`DurableRecordError::QueueFull`] -- that variant exists for
    /// correctness/completeness of this crate's own typed contract (a
    /// future caller choosing a different submission strategy), but
    /// under THIS method's own bounded-send behavior it is reachable
    /// only if the channel's sender is literally closed mid-send, never
    /// from ordinary capacity pressure alone.
    pub async fn record_durable(&self, event: McpClientEvent) -> Result<DurableAck, DurableRecordError> {
        let (respond_to, response) = tokio::sync::oneshot::channel();
        self.tx.send(Command::Durable { event, respond_to }).await.map_err(|_| DurableRecordError::WorkerGone)?;
        response.await.map_err(|_| DurableRecordError::WorkerGone)?
    }

    /// Submits `event` without waiting for durability -- only for the
    /// enqueue attempt itself. Uses `try_send` (never blocks): a
    /// momentarily full queue is reported as [`EnqueueError::QueueFull`],
    /// never silently dropped and never awaited.
    fn try_enqueue_best_effort(&self, event: McpClientEvent) -> Result<(), EnqueueError> {
        self.tx.try_send(Command::BestEffort { event }).map_err(|e| match e {
            tokio::sync::mpsc::error::TrySendError::Full(_) => EnqueueError::QueueFull,
            tokio::sync::mpsc::error::TrySendError::Closed(_) => EnqueueError::WorkerGone,
        })
    }

    /// The infallible-to-caller, best-effort submission path -- mirrors
    /// `brain_api_audit_worker::AuditWorkerHandle`'s own
    /// `impl AuditSink for AuditWorkerHandle` shape (this crate has no
    /// `AuditSink`-equivalent trait of its own to implement against,
    /// since it has no dependency on `brain-api`; this plain method is
    /// the direct equivalent for a caller that wants "best effort,
    /// never blocks, never panics, silently accepts the enqueue failure
    /// if any").
    pub fn record_best_effort(&self, event: McpClientEvent) {
        let _ = self.try_enqueue_best_effort(event);
    }

    /// Requests graceful shutdown: every command already enqueued
    /// before this call is fully processed first (true FIFO drain),
    /// then the worker task exits. Resolves only after that drain
    /// completes.
    pub async fn shutdown(&self) -> Result<(), EnqueueError> {
        let (respond_to, response) = tokio::sync::oneshot::channel();
        self.tx.send(Command::Shutdown { respond_to }).await.map_err(|_| EnqueueError::WorkerGone)?;
        response.await.map_err(|_| EnqueueError::WorkerGone)
    }
}
