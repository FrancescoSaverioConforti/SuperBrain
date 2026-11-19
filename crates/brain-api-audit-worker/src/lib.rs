//! B5 remediation M5 Task 4: dedicated, cancellation-safe durable audit
//! writer worker.
//!
//! **What this crate owns, and nothing else:** bounded command delivery
//! to exactly one dedicated writer task, that task's own lifecycle
//! (startup recovery, shutdown drain), and the two acknowledgment
//! contracts (best-effort enqueue vs. durable append+fsync+reference-
//! update confirmation). It owns no audit semantics (`brain-api` does:
//! `AuditEvent`/`AuditEventKind`/the line format in
//! `brain_api::audit_record`/hash-chain verification in the facade this
//! crate calls) and no filesystem/locking/DPAPI mechanics (`storage`
//! does, via `storage::Vault::open_audit_writer`/`AuditWriterHandle`).
//!
//! **Why a dedicated single-owner task, not a per-call mutex**: M5 Task
//! 2's own design review found a `tokio::sync::Mutex` held across
//! `spawn_blocking` is cancellation-UNSAFE — a caller's task aborted
//! mid-write can desynchronize the mutex guard from the actual
//! in-flight blocking operation, reproducibly causing duplicate sequence
//! numbers on disk. This crate instead uses the pattern that direct
//! reproduction proved safe: one task owns the resource exclusively and
//! processes commands strictly one at a time from an `mpsc` channel;
//! cancelling a SUBMITTING caller's own `.await` (by dropping its
//! `oneshot::Receiver`) never reaches into the worker task, which always
//! finishes whatever command it already dequeued before looking at the
//! next one.
//!
//! **Composition-root only, no production wiring in this task**: this
//! crate is usable by `mcp-server` or a CLI entry point, but neither is
//! wired to actually construct/use it yet (M5 Task 4's own explicit
//! scope limit) — see [`spawn_worker`]'s own doc comment for the
//! construction contract a future wiring task will follow.

mod commands;
mod error;
mod handle;
mod recovery;
mod worker;

pub use commands::DurableAck;
pub use error::{DurableRecordError, EnqueueError, RecoveryError, WorkerStartupError};
pub use handle::AuditWorkerHandle;
pub use worker::{spawn_worker, WorkerConfig};

#[cfg(test)]
mod tests;
