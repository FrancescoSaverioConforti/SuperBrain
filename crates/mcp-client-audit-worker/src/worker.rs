//! M6.3 Phase 3: the single-owner audit writer task -- mirrors
//! `brain_api_audit_worker::worker`'s own exact architecture, generic
//! over an injected [`crate::writer::DurableAuditWriter`] instead of a
//! concrete `storage::AuditWriterHandle` (this crate's own Phase 2
//! dependency-boundary constraint).
//!
//! **Cancellation safety (the single most load-bearing property this
//! module exists to provide)**: the injected writer `W` is OWNED by this
//! module's own loop as a plain local variable -- never behind a mutex,
//! never cloned, never borrowed across an `.await` point from outside
//! this task. Each append moves `writer` INTO [`tokio::task::
//! spawn_blocking`]'s own `'static` closure and moves it back OUT via
//! the closure's return value (see [`append_blocking`] below) -- the
//! exact mechanism `brain_api_audit_worker::worker`'s own doc comment
//! documents as load-bearing, reused here rather than reinvented. This
//! task itself is never the thing that gets cancelled from outside (a
//! SUBMITTING caller's own `oneshot::Receiver` await can be cancelled;
//! that never reaches into this loop, since an already-sent `Command` on
//! the channel is NOT un-submitted by a cancelled caller -- the worker
//! still processes it to completion, and only the `respond_to.send(...)`
//! then silently fails because nobody is listening). This is precisely
//! Phase 3 item 4's own requirement: "An aborted caller must not cause
//! an accepted write to be silently cancelled or replayed with a
//! duplicate sequence."
//!
//! This module never uses `Handle::block_on` anywhere, nested or
//! otherwise (an explicit Phase 3 prohibition) -- the ONLY blocking-call
//! bridge is `spawn_blocking`, which offloads to a dedicated blocking
//! thread pool rather than blocking the async runtime itself.

use audit_chain_core::{serialize_record, GENESIS_HASH};

use crate::commands::{Command, DurableAck};
use crate::error::{DurableRecordError, WorkerStartupError};
use crate::handle::AuditWorkerHandle;
use crate::recovery::recover_tail;
use crate::writer::{DurableAuditWriter, WriterAppendError};

/// This worker's own bounded queue capacity -- same literal value as
/// `brain_api_audit_worker::worker::QUEUE_CAPACITY`, for the same
/// reason (a deliberately bounded, not unbounded, channel; see
/// [`WorkerConfig`] for how a caller overrides it).
pub const QUEUE_CAPACITY: usize = 256;

/// Configuration for [`spawn_worker`] -- today, only the queue capacity;
/// kept as its own struct (not a bare `usize` parameter) so a future
/// field can be added without breaking `spawn_worker`'s own signature,
/// mirroring `brain_api_audit_worker::WorkerConfig`'s own shape.
#[derive(Debug, Clone, Copy)]
pub struct WorkerConfig {
    pub queue_capacity: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self { queue_capacity: QUEUE_CAPACITY }
    }
}

/// Starts the single-owner audit writer task over `writer` (an already-
/// constructed, already-opened injected writer -- this function neither
/// opens nor closes any filesystem resource itself) and `raw_log_bytes`
/// (the composition root's own already-read raw log bytes, used ONLY
/// for this startup recovery scan; this function performs no I/O of its
/// own beyond that one in-memory scan).
///
/// Recovery runs BEFORE anything is spawned: if `raw_log_bytes`'s own
/// chain does not permit immediate resumption, this function returns
/// `Err(WorkerStartupError::RecoveryRequired(..))` and starts no task at
/// all -- mirrors `brain_api_audit_worker::spawn_worker`'s own
/// fail-before-spawn discipline exactly.
pub fn spawn_worker<W: DurableAuditWriter>(writer: W, raw_log_bytes: &[u8], config: WorkerConfig) -> Result<AuditWorkerHandle, WorkerStartupError> {
    let recovered = recover_tail(raw_log_bytes).map_err(|(_, error)| WorkerStartupError::RecoveryRequired(error))?;

    let mut next_sequence = recovered.sequence.map(|s| s + 1).unwrap_or(0);
    let mut prev_hash = recovered.record_hash.unwrap_or_else(|| GENESIS_HASH.to_string());

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Command>(config.queue_capacity);

    tokio::spawn(async move {
        let mut writer = writer;
        let mut poisoned = false;

        while let Some(command) = rx.recv().await {
            match command {
                Command::BestEffort { event } => {
                    if poisoned {
                        continue;
                    }
                    let (bytes, record_hash) = serialize_record(&event, next_sequence, &prev_hash);
                    let sequence = next_sequence;
                    let (returned_writer, outcome) = append_blocking(writer, bytes, sequence, record_hash.clone()).await;
                    writer = returned_writer;
                    match outcome {
                        Ok(()) => {
                            prev_hash = record_hash;
                            next_sequence += 1;
                        }
                        Err(WriterAppendError::ReferenceUpdateFailed(_)) => {
                            // Log itself is durably on disk; advance the
                            // tail exactly as a successful append would,
                            // matching storage's own "not poisoning"
                            // semantics for this specific failure.
                            prev_hash = record_hash;
                            next_sequence += 1;
                        }
                        Err(_) => {
                            poisoned = true;
                        }
                    }
                }
                Command::Durable { event, respond_to } => {
                    if poisoned {
                        let _ = respond_to.send(Err(DurableRecordError::WriterPoisoned));
                        continue;
                    }
                    let (bytes, record_hash) = serialize_record(&event, next_sequence, &prev_hash);
                    let sequence = next_sequence;
                    let (returned_writer, outcome) = append_blocking(writer, bytes, sequence, record_hash.clone()).await;
                    writer = returned_writer;
                    let result = match outcome {
                        Ok(()) => {
                            prev_hash = record_hash.clone();
                            next_sequence += 1;
                            Ok(DurableAck { sequence, record_hash })
                        }
                        Err(WriterAppendError::ReferenceUpdateFailed(msg)) => {
                            prev_hash = record_hash;
                            next_sequence += 1;
                            Err(DurableRecordError::ReferenceUpdateFailed(msg))
                        }
                        Err(WriterAppendError::Poisoned) => {
                            poisoned = true;
                            Err(DurableRecordError::WriterPoisoned)
                        }
                        Err(WriterAppendError::Io(msg)) => {
                            poisoned = true;
                            Err(DurableRecordError::Io(msg))
                        }
                    };
                    // Ignoring send failure: the caller cancelled its
                    // own await, but the write above already completed
                    // (or definitively failed) regardless -- exactly the
                    // "accepted write is never silently cancelled or
                    // replayed" guarantee this module's own doc comment
                    // states.
                    let _ = respond_to.send(result);
                }
                Command::Shutdown { respond_to } => {
                    let _ = respond_to.send(());
                    return;
                }
            }
        }
    });

    Ok(AuditWorkerHandle::new(tx))
}

/// Moves `writer` into `spawn_blocking`'s own `'static` closure, appends
/// one record on the dedicated blocking thread pool (never on this
/// async task's own worker thread), and moves `writer` back out via the
/// closure's return value -- see this module's own top doc comment for
/// why this exact shape is this worker's cancellation-safety mechanism.
async fn append_blocking<W: DurableAuditWriter>(writer: W, bytes: Vec<u8>, sequence: u64, record_hash: String) -> (W, Result<(), WriterAppendError>) {
    tokio::task::spawn_blocking(move || writer.append_owned(bytes, sequence, record_hash))
        .await
        .expect("the blocking closure itself never panics: append_owned's own Result carries every failure mode this worker needs to observe")
}

#[allow(dead_code)]
/// A regression anchor confirming this module never spells
/// `Handle::block_on` anywhere -- an explicit Phase 3 prohibition.
/// (Enforced for real by `tests/no_block_on.rs`, which reads this file's
/// own source text; this const merely documents the constraint inline,
/// at the one place a future edit would need to look.)
const _NO_NESTED_BLOCK_ON: () = ();
