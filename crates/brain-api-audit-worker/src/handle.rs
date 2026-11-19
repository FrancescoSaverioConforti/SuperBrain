//! The caller-facing handle to a running worker — cheap to clone (an
//! `mpsc::Sender` clone), submits commands onto the worker's own bounded
//! channel, never touches the `AuditWriterHandle`/chain state directly.

use crate::commands::{Command, DurableAck};
use crate::error::{DurableRecordError, EnqueueError};

/// A cloneable submission point for a running audit worker. Cloning this
/// is cheap and safe to share across many concurrent callers (MCP tool
/// handlers, the CLI surface) — EVERY clone funnels through the one
/// dedicated writer task `spawn_worker` created; this type itself holds
/// no mutable state and performs no I/O.
#[derive(Debug, Clone)]
pub struct AuditWorkerHandle {
    tx: tokio::sync::mpsc::Sender<Command>,
}

impl AuditWorkerHandle {
    pub(crate) fn new(tx: tokio::sync::mpsc::Sender<Command>) -> Self {
        Self { tx }
    }

    /// Submits `event` for durable appending and awaits the COMPLETE
    /// append+fsync+reference-update outcome — this is the acknowledgment
    /// contract [`brain_api::AuditSink::record`] deliberately cannot
    /// offer (that trait's own signature is synchronous and returns
    /// nothing).
    ///
    /// **Cancellation**: if the CALLING task's own `.await` on this
    /// method is cancelled (the task that called `record_durable` is
    /// itself aborted), the command this call already sent onto the
    /// channel is NOT un-submitted — the worker task still dequeues and
    /// processes it to completion exactly as if cancellation had never
    /// happened; this method's own future simply never resolves for the
    /// caller that dropped it. The write's outcome (success, poisoning,
    /// or reference-update failure) is NOT separately observable after
    /// the fact by this crate (no persisted command-id-to-outcome table
    /// — out of this task's scope); a caller that must know the outcome
    /// despite its own cancellation should not cancel this specific
    /// await, or should accept that a cancelled call provides no
    /// after-the-fact reconciliation beyond what the durable audit LOG
    /// ITSELF already is (reading it back is always authoritative).
    pub async fn record_durable(&self, event: brain_api::AuditEvent) -> Result<DurableAck, DurableRecordError> {
        let (respond_to, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Command::Durable { event, respond_to })
            .await
            .map_err(|_| DurableRecordError::WorkerGone)?;
        rx.await.map_err(|_| DurableRecordError::WorkerGone)?
    }

    /// Best-effort, non-blocking-from-the-caller's-perspective enqueue —
    /// the synchronous counterpart used by [`AuditSink::record`]'s own
    /// impl below. Uses `try_send` (never `.await`s) so a synchronous
    /// trait method can call it at all; returns immediately with whether
    /// the ENQUEUE itself succeeded, never a durability claim.
    fn try_enqueue_best_effort(&self, event: brain_api::AuditEvent) -> Result<(), EnqueueError> {
        self.tx.try_send(Command::BestEffort { event }).map_err(|err| match err {
            tokio::sync::mpsc::error::TrySendError::Full(_) => EnqueueError::QueueFull,
            tokio::sync::mpsc::error::TrySendError::Closed(_) => EnqueueError::WorkerGone,
        })
    }

    /// Gracefully shuts down the worker: every command already enqueued
    /// before this call is fully processed first (FIFO), then the
    /// worker task exits. Awaits true exit, not merely acceptance.
    /// Returns `Err` only if the worker was already gone (e.g. panicked
    /// earlier) — shutdown is then a no-op from this caller's
    /// perspective, since there is nothing left to drain.
    pub async fn shutdown(&self) -> Result<(), EnqueueError> {
        let (respond_to, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Command::Shutdown { respond_to })
            .await
            .map_err(|_| EnqueueError::WorkerGone)?;
        rx.await.map_err(|_| EnqueueError::WorkerGone)
    }
}

/// Best-effort adapter: preserves `AuditSink::record`'s own exact
/// synchronous, infallible-from-the-caller's-perspective signature.
/// **Never claims durability** — a successful call here means only "the
/// event was accepted onto the worker's bounded queue," not "it was
/// persisted." Queue-full/worker-gone are silently dropped (this
/// signature has no way to report them, matching `NullAuditSink`'s own
/// "never panics or blocks for lack of a configured destination"
/// contract) rather than panicking or blocking the caller — a caller
/// that needs to know whether its event was actually durably persisted
/// must use [`AuditWorkerHandle::record_durable`] instead, never this
/// trait impl.
impl brain_api::AuditSink for AuditWorkerHandle {
    fn record(&self, event: brain_api::AuditEvent) {
        let _ = self.try_enqueue_best_effort(event);
    }
}
