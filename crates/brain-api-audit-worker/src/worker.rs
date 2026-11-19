//! The dedicated writer task: exactly one instance per running worker,
//! owning the `storage::AuditWriterHandle` and the in-memory chain-tail
//! state EXCLUSIVELY. No mutex anywhere in this module — strict,
//! sequential one-command-at-a-time processing from the channel IS the
//! serialization (verified safe under real cancellation in M5 Task 2's
//! own design review; the per-call-mutex-across-spawn_blocking
//! alternative was proven UNSAFE there and must not be reintroduced —
//! see this crate's own top-level doc comment).

use crate::commands::{Command, DurableAck};
use crate::error::{DurableRecordError, WorkerStartupError};
use crate::recovery::recover_tail;

/// Bounded queue capacity — fixed, not configurable per the "no
/// unbounded queue growth" requirement having one obvious, auditable
/// value rather than a caller-tunable knob this task was not asked to
/// design a policy around. Chosen generously enough that a short burst
/// of concurrent MCP tool calls (this codebase's own existing
/// concurrency limit, `search.rs`'s semaphore, already bounds in-flight
/// MCP work to a small number) does not spuriously hit `QueueFull`
/// under normal operation, while still being a small, fixed, inspectable
/// number — never "as large as memory allows".
const QUEUE_CAPACITY: usize = 256;

/// Construction-time configuration — currently just the queue capacity,
/// kept as a named struct (not a bare `usize` parameter) so a future,
/// separately-authorized task can add fields without changing
/// `spawn_worker`'s own call shape.
#[derive(Debug, Clone, Copy)]
pub struct WorkerConfig {
    pub queue_capacity: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self { queue_capacity: QUEUE_CAPACITY }
    }
}

/// Opens `vault`'s audit writer (running startup recovery first, exactly
/// as `storage::Vault::open_audit_writer`'s own trust-boundary contract
/// requires: this crate computes `caller_verified_tail` itself via
/// `brain_api::scan_chain`, never skipping that step), then spawns the
/// dedicated writer task onto the CURRENT Tokio runtime (never creates
/// one of its own — see this crate's own `Cargo.toml` comment on why its
/// `tokio` feature set has no `rt-multi-thread`) and returns a cloneable
/// [`crate::AuditWorkerHandle`] for submitting commands to it.
///
/// Fails BEFORE spawning anything if startup recovery finds the existing
/// log is not in a state this worker may safely resume appending to
/// ([`WorkerStartupError::RecoveryRequired`]) or if opening the writer
/// itself fails ([`WorkerStartupError::OpenFailed`] — e.g. another
/// process already holds the writer lock). Neither case leaves a worker
/// task running; the caller gets a plain error back, synchronously.
///
/// **Composition-root contract (for a future, separately authorized
/// wiring task)**: call this once per process per audit log, keep the
/// returned handle (clone it freely — cloning is cheap, an `mpsc::Sender`
/// clone), and call [`crate::AuditWorkerHandle::shutdown`] during
/// graceful process shutdown so in-flight commands drain before the
/// process exits. This task does not call `spawn_worker` from
/// `mcp-server` or any CLI entry point itself.
pub fn spawn_worker(vault: storage::Vault, config: WorkerConfig) -> Result<crate::AuditWorkerHandle, WorkerStartupError> {
    let recovered = match recover_tail(&vault) {
        Ok(recovered) => recovered,
        Err((_, error)) => return Err(WorkerStartupError::RecoveryRequired(error)),
    };

    let mut next_sequence = recovered.sequence.map(|s| s + 1).unwrap_or(0);
    let mut prev_hash = recovered
        .record_hash
        .clone()
        .unwrap_or_else(|| brain_api::GENESIS_HASH.to_string());

    let handle = vault
        .open_audit_writer(recovered.as_caller_verified_tail())
        .map_err(|e| WorkerStartupError::OpenFailed(format!("{e}")))?;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Command>(config.queue_capacity);

    tokio::spawn(async move {
        let mut handle = handle;
        while let Some(command) = rx.recv().await {
            match command {
                Command::BestEffort { event } => {
                    if handle.is_poisoned() {
                        // Best-effort contract: nothing to report back
                        // to (there is no respond_to here) -- silently
                        // discarding is this path's own documented
                        // behavior for a poisoned writer (see
                        // `AuditWorkerHandle`'s own `AuditSink` impl doc
                        // comment: best-effort never claims durability
                        // and never panics for lack of a usable
                        // destination).
                        continue;
                    }
                    let (bytes, record_hash) = brain_api::serialize_record(&event, next_sequence, &prev_hash);
                    let (returned_handle, outcome) = append_blocking(handle, bytes, next_sequence, record_hash.clone()).await;
                    handle = returned_handle;
                    match outcome {
                        Ok(()) | Err(storage::AuditAppendError::ReferenceUpdateFailed(_)) => {
                            // `ReferenceUpdateFailed`: the log append
                            // itself succeeded durably (see that error
                            // variant's own doc comment) -- the
                            // in-memory tail DID advance on disk, so it
                            // must advance here too, even though this
                            // particular append's full contract did not
                            // complete end to end.
                            prev_hash = record_hash;
                            next_sequence += 1;
                        }
                        Err(_) => {
                            // Poisoning I/O failure: `handle.is_poisoned()`
                            // is now true; the in-memory tail must NOT
                            // advance (the append's own effect is of
                            // unknown extent) -- next command's own
                            // poisoned-check above (or the durable
                            // path's own, below) refuses further work.
                        }
                    }
                }
                Command::Durable { event, respond_to } => {
                    if handle.is_poisoned() {
                        let _ = respond_to.send(Err(DurableRecordError::WriterPoisoned));
                        continue;
                    }
                    let (bytes, record_hash) = brain_api::serialize_record(&event, next_sequence, &prev_hash);
                    let (returned_handle, outcome) = append_blocking(handle, bytes, next_sequence, record_hash.clone()).await;
                    handle = returned_handle;
                    let result = match outcome {
                        Ok(()) => {
                            prev_hash = record_hash.clone();
                            next_sequence += 1;
                            Ok(DurableAck { sequence: next_sequence - 1, record_hash })
                        }
                        Err(storage::AuditAppendError::ReferenceUpdateFailed(msg)) => {
                            prev_hash = record_hash;
                            next_sequence += 1;
                            Err(DurableRecordError::ReferenceUpdateFailed(msg))
                        }
                        Err(storage::AuditAppendError::Poisoned) => Err(DurableRecordError::WriterPoisoned),
                        Err(storage::AuditAppendError::Io(io_err)) => Err(DurableRecordError::Io(io_err.to_string())),
                    };
                    // Caller may have cancelled (dropped its receiver)
                    // -- the write still happened (or definitively
                    // failed) regardless; this send's own failure is
                    // never an error FROM the worker's perspective, it
                    // just means nobody is listening for this specific
                    // outcome anymore. See this crate's own top-level
                    // doc comment and `Command::Durable`'s.
                    let _ = respond_to.send(result);
                }
                Command::Shutdown { respond_to } => {
                    // Graceful: every command enqueued BEFORE this one
                    // has already been fully processed above (strict
                    // FIFO channel order) -- nothing further is accepted
                    // once this arm runs, since the loop exits right
                    // after acknowledging.
                    let _ = respond_to.send(());
                    return;
                }
            }
        }
        // `rx.recv()` returned `None`: every `Sender` (every
        // `AuditWorkerHandle` clone) was dropped without an explicit
        // `shutdown()` call -- an implicit, abrupt-from-this-task's-own-
        // perspective shutdown. The writer's own `Drop` (releasing the
        // OS-level lock) still runs normally when `handle` goes out of
        // scope here; no special handling needed beyond letting the
        // async block end.
    });

    Ok(crate::AuditWorkerHandle::new(tx))
}

/// Runs one `AuditWriterHandle::append` call on a blocking-pool thread
/// (per constraint: synchronous filesystem I/O — `write_all`/`sync_all`/
/// the DPAPI reference write inside `append` — must never block a Tokio
/// runtime worker thread), returning OWNERSHIP of `handle` back alongside
/// the outcome.
///
/// **Why moving ownership in and out, not borrowing**: `handle` is not
/// `Clone`/`Sync` in any way that would let `spawn_blocking`'s own
/// `'static` closure borrow it; moving it in and getting it back via the
/// `JoinHandle`'s own result is the standard, safe pattern for handing a
/// single owned resource to exactly one blocking-pool thread at a time.
/// This function's own `.await` is on `spawn_blocking`'s `JoinHandle` —
/// called only from INSIDE the dedicated worker task's own loop, which
/// nothing external ever cancels (only a SUBMITTING caller's own
/// `oneshot::Receiver` await can be cancelled — see `Command::Durable`'s
/// own doc comment) — so this `.await` itself is never the target of the
/// cancellation this crate's whole design exists to be safe against.
///
/// A panic inside the blocking closure (not expected — `append` returns
/// `Result`, never panics by its own contract) would make `spawn_blocking`'s
/// `JoinHandle` return `Err`, which this function propagates as a task
/// panic via `.expect` — deliberately NOT caught and converted into a
/// poisoned-but-continuing state, since a panic here indicates a bug in
/// `storage::AuditWriterHandle::append` itself, not a normal I/O failure
/// this crate's own typed errors are meant to model.
async fn append_blocking(
    mut handle: storage::AuditWriterHandle,
    bytes: Vec<u8>,
    sequence: u64,
    record_hash: String,
) -> (storage::AuditWriterHandle, Result<(), storage::AuditAppendError>) {
    tokio::task::spawn_blocking(move || {
        let outcome = handle.append(&bytes, sequence, &record_hash);
        (handle, outcome)
    })
    .await
    .expect("the blocking append closure itself never panics by storage::AuditWriterHandle::append's own contract (always returns Result, never panics)")
}
