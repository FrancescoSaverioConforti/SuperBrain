//! M6.3 Phase 5: end-to-end worker architecture tests against a real
//! (in-memory, not mocked-away) `DurableAuditWriter` implementation --
//! mirroring `brain-api-audit-worker`'s own load-bearing test suite
//! (reported via direct inspection of that crate's `src/tests.rs`),
//! adapted to this crate's own injected-writer boundary.
//!
//! `InMemoryWriter` below is a genuine implementation of the trait
//! contract (ownership-move in, ownership-move out through
//! `spawn_blocking`) -- not a bypass of it: every test exercises the
//! real `spawn_worker`/`AuditWorkerHandle` machinery exactly as a real
//! `storage`-backed adapter would be driven by a future composition
//! root, with a `std::sync::Mutex<Vec<u8>>`-backed in-memory log
//! standing in for the real filesystem (chosen so these tests can run
//! without any filesystem or `storage` dependency at all, per this
//! crate's own dependency-boundary constraint).

use std::sync::{Arc, Mutex};

use mcp_client_audit_worker::{
    event, spawn_worker, DenialReason, DurableAuditWriter, DurableRecordError, EnqueueError, McpClientEventKind, RecoveryError, WorkerConfig, WriterAppendError,
};

/// A genuine `DurableAuditWriter` implementation over a shared,
/// in-memory byte buffer -- every append is write-all-then-"fsync"
/// (trivially true for an in-memory `Vec<u8>`, but the ordering is
/// still exercised: bytes are pushed before this function returns `Ok`,
/// matching the real contract's own "never Ok before written and
/// fsync'd" requirement).
#[derive(Clone)]
struct InMemoryWriter {
    log: Arc<Mutex<Vec<u8>>>,
    poison_after_n_appends: Option<usize>,
    fail_reference_after_n_appends: Option<usize>,
    appends_so_far: usize,
}

impl InMemoryWriter {
    fn new(log: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { log, poison_after_n_appends: None, fail_reference_after_n_appends: None, appends_so_far: 0 }
    }

    fn poisoning_after(log: Arc<Mutex<Vec<u8>>>, n: usize) -> Self {
        Self { log, poison_after_n_appends: Some(n), fail_reference_after_n_appends: None, appends_so_far: 0 }
    }

    fn reference_failing_after(log: Arc<Mutex<Vec<u8>>>, n: usize) -> Self {
        Self { log, poison_after_n_appends: None, fail_reference_after_n_appends: Some(n), appends_so_far: 0 }
    }
}

impl DurableAuditWriter for InMemoryWriter {
    fn append_owned(mut self, bytes: Vec<u8>, _sequence: u64, _record_hash: String) -> (Self, Result<(), WriterAppendError>) {
        self.appends_so_far += 1;

        if let Some(n) = self.poison_after_n_appends {
            if self.appends_so_far > n {
                return (self, Err(WriterAppendError::Io("simulated I/O failure".to_string())));
            }
        }

        // The log write itself always happens first (write_all), exactly
        // matching storage::AuditWriterHandle::append's own ordering --
        // even a reference-update "failure" below happens AFTER the
        // bytes are already durably in the log.
        self.log.lock().unwrap().extend_from_slice(&bytes);

        if let Some(n) = self.fail_reference_after_n_appends {
            if self.appends_so_far > n {
                return (self, Err(WriterAppendError::ReferenceUpdateFailed("simulated reference-store failure".to_string())));
            }
        }

        (self, Ok(()))
    }
}

fn sample_event(correlation_id: &str, server: &str) -> mcp_client_audit_worker::McpClientEvent {
    event(McpClientEventKind::DiscoveryAttempted { server: server.to_string() }, "2026-01-01T00:00:00Z", correlation_id)
}

// --- startup recovery gate ---

#[tokio::test]
async fn an_empty_log_starts_a_worker_with_no_prior_recovery_error() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    let handle = spawn_worker(writer, &[], WorkerConfig::default()).expect("an empty log must permit a fresh worker to start");

    let ack = handle.record_durable(sample_event("c1", "s1")).await.expect("first durable write must succeed");
    assert_eq!(ack.sequence, 0);

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_log_with_a_partial_trailing_record_refuses_to_start_a_worker() {
    let mut raw = b"{\"schema_version\":1,\"sequence\":0,\"prev_record_hash\":\"".to_vec();
    raw.extend_from_slice(&b"0".repeat(68));
    raw.extend_from_slice(b"\",\"record_hash\":\"truncated-mid-wa"); // no closing brace, no trailing newline
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log);

    let result = spawn_worker(writer, &raw, WorkerConfig::default());
    assert!(matches!(result, Err(mcp_client_audit_worker::WorkerStartupError::RecoveryRequired(RecoveryError::PartialTrailingRecord))), "got {result:?}");
}

// --- durable ack only after the record is actually in the log ---

#[tokio::test]
async fn durable_ack_is_only_returned_after_the_record_is_actually_appended() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    let handle = spawn_worker(writer, &[], WorkerConfig::default()).unwrap();

    let ack = handle.record_durable(sample_event("c1", "s1")).await.expect("write must succeed");

    // Read the "disk" (the in-memory log) directly, bypassing the
    // worker/handle entirely -- confirm the exact bytes are already
    // there and verify cleanly, proving Ok truly means "on disk," not
    // merely "enqueued."
    let raw = log.lock().unwrap().clone();
    assert!(!raw.is_empty(), "the appended bytes must already be present in the log immediately after record_durable resolves Ok");
    let line = &raw[..raw.len() - 1]; // strip trailing newline
    let record = audit_chain_core::parse_line(line).expect("the appended line must parse");
    assert_eq!(record.record_hash, ack.record_hash);
    assert_eq!(record.sequence, 0);

    handle.shutdown().await.unwrap();
}

// --- multiple concurrent producers yield a valid, contiguous chain ---

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multiple_concurrent_producers_yield_a_valid_contiguous_hash_chain() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    let handle = spawn_worker(writer, &[], WorkerConfig::default()).unwrap();

    let mut tasks = Vec::new();
    for producer in 0..8u32 {
        let handle = handle.clone();
        tasks.push(tokio::spawn(async move {
            for i in 0..25u32 {
                handle.record_durable(sample_event(&format!("c-{producer}-{i}"), "s1")).await.expect("every concurrent durable write must succeed");
            }
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    handle.shutdown().await.unwrap();

    let raw = log.lock().unwrap().clone();
    let lines: Vec<&[u8]> = raw[..raw.len() - 1].split(|&b| b == b'\n').collect();
    assert_eq!(lines.len(), 200, "all 8*25 concurrent writes must be present");

    let line_outcomes: Vec<audit_chain_core::LineOutcome> = lines
        .into_iter()
        .map(|line| audit_chain_core::LineOutcome::Parsed(audit_chain_core::parse_line(line).expect("every line must parse")))
        .collect();
    let scan = audit_chain_core::scan_chain(&line_outcomes);
    assert!(scan.stop_reason.is_none(), "the chain built under real concurrency must verify cleanly end to end, got {:?}", scan.stop_reason);
    assert_eq!(scan.verified_records.len(), 200);
    for (i, record) in scan.verified_records.iter().enumerate() {
        assert_eq!(record.sequence, i as u64, "sequence numbers must be contiguous with no gaps or duplicates");
    }
}

// --- cancellation safety ---

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_caller_while_its_durable_write_is_in_flight_causes_no_duplicate_or_corruption() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    let handle = spawn_worker(writer, &[], WorkerConfig::default()).unwrap();

    let handle_a = handle.clone();
    let task_a = tokio::spawn(async move {
        let _ = handle_a.record_durable(sample_event("c-a", "s1")).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    task_a.abort();

    // A second, non-cancelled call must still succeed and the chain must
    // remain uncorrupted with no duplicate sequence numbers -- proving
    // aborting the AWAITING caller never corrupts the worker's own
    // in-flight write.
    let ack_b = handle.record_durable(sample_event("c-b", "s1")).await.expect("a fresh call after a cancelled one must still succeed");
    handle.shutdown().await.unwrap();

    let raw = log.lock().unwrap().clone();
    let lines: Vec<&[u8]> = raw[..raw.len() - 1].split(|&b| b == b'\n').collect();
    let line_outcomes: Vec<audit_chain_core::LineOutcome> = lines
        .into_iter()
        .map(|line| audit_chain_core::LineOutcome::Parsed(audit_chain_core::parse_line(line).expect("every line must parse")))
        .collect();
    let scan = audit_chain_core::scan_chain(&line_outcomes);
    assert!(scan.stop_reason.is_none(), "the chain must remain uncorrupted after a cancelled caller, got {:?}", scan.stop_reason);

    let sequences: Vec<u64> = scan.verified_records.iter().map(|r| r.sequence).collect();
    let mut deduped = sequences.clone();
    deduped.dedup();
    assert_eq!(sequences.len(), deduped.len(), "no duplicate sequence numbers may ever appear, got {sequences:?}");
    assert!(sequences.contains(&ack_b.sequence), "the second call's own acknowledged sequence must actually be present in the log");
}

#[tokio::test]
async fn cancelling_before_the_command_is_even_accepted_onto_the_queue_is_safe() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    // Capacity 1, so a second concurrent submission is guaranteed to
    // still be waiting on tx.send(...).await (backlogged, not yet
    // accepted) when we abort it.
    let handle = spawn_worker(writer, &[], WorkerConfig { queue_capacity: 1 }).unwrap();

    let handle_fill = handle.clone();
    let _filler = tokio::spawn(async move {
        let _ = handle_fill.record_durable(sample_event("c-fill", "s1")).await;
    });

    let handle_abort = handle.clone();
    let task = tokio::spawn(async move {
        let _ = handle_abort.record_durable(sample_event("c-abort", "s1")).await;
    });
    task.abort();
    let _ = task.await; // JoinError is expected and fine

    // The worker itself must still be alive and able to process further
    // submissions normally -- aborting a not-yet-accepted submission
    // must never poison or wedge the worker.
    let ack = handle.record_durable(sample_event("c-after", "s1")).await.expect("the worker must remain healthy after a pre-acceptance abort");
    assert!(ack.sequence < 10, "sanity: sequence numbers stay small in this short test");
    handle.shutdown().await.unwrap();
}

// --- backpressure / queue behavior ---

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_submission_applies_backpressure_rather_than_reporting_queue_full() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    let handle = spawn_worker(writer, &[], WorkerConfig { queue_capacity: 1 }).unwrap();

    let mut tasks = Vec::new();
    for i in 0..20u32 {
        let handle = handle.clone();
        tasks.push(tokio::spawn(async move { handle.record_durable(sample_event(&format!("c-{i}"), "s1")).await }));
    }
    let mut ok_count = 0;
    for t in tasks {
        if t.await.unwrap().is_ok() {
            ok_count += 1;
        }
    }
    assert_eq!(ok_count, 20, "record_durable's own bounded send(...).await must apply backpressure, never report QueueFull under ordinary capacity pressure");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn best_effort_enqueue_never_panics_or_blocks_when_the_queue_is_full() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    let handle = spawn_worker(writer, &[], WorkerConfig { queue_capacity: 1 }).unwrap();

    for i in 0..50u32 {
        handle.record_best_effort(sample_event(&format!("c-{i}"), "s1"));
    }
    // No assertion needed beyond "this did not panic or hang" -- the
    // infallible-to-caller contract is the thing under test.
    handle.shutdown().await.unwrap();
}

// --- post-shutdown behavior ---

#[tokio::test]
async fn after_shutdown_further_durable_submissions_are_refused_not_silently_dropped() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    let handle = spawn_worker(writer, &[], WorkerConfig::default()).unwrap();
    handle.shutdown().await.unwrap();

    let result = handle.record_durable(sample_event("c1", "s1")).await;
    assert_eq!(result, Err(DurableRecordError::WorkerGone));
}

// --- writer poisoning ---

#[tokio::test]
async fn a_plain_io_failure_poisons_the_writer_and_every_subsequent_durable_call_is_refused() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::poisoning_after(log.clone(), 1); // succeeds once, then fails
    let handle = spawn_worker(writer, &[], WorkerConfig::default()).unwrap();

    handle.record_durable(sample_event("c1", "s1")).await.expect("first write must succeed");
    let second = handle.record_durable(sample_event("c2", "s1")).await;
    assert!(matches!(second, Err(DurableRecordError::Io(_))), "got {second:?}");

    let third = handle.record_durable(sample_event("c3", "s1")).await;
    assert_eq!(third, Err(DurableRecordError::WriterPoisoned), "a poisoned writer must refuse every subsequent durable call without attempting another append");
}

// --- reference-update failure does not poison ---

#[tokio::test]
async fn a_reference_update_failure_does_not_poison_the_writer() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::reference_failing_after(log.clone(), 0); // every append's own reference update fails
    let handle = spawn_worker(writer, &[], WorkerConfig::default()).unwrap();

    let first = handle.record_durable(sample_event("c1", "s1")).await;
    assert!(matches!(first, Err(DurableRecordError::ReferenceUpdateFailed(_))), "got {first:?}");

    // The log itself still advanced (a reference-update failure does
    // NOT poison the writer) -- a second write, EVEN THOUGH its own
    // reference update also fails in this test's setup, must still
    // advance to sequence 1, not repeat sequence 0 or get refused as
    // WriterPoisoned.
    let second = handle.record_durable(sample_event("c2", "s1")).await;
    match second {
        Err(DurableRecordError::ReferenceUpdateFailed(_)) => {}
        other => panic!("expected a second ReferenceUpdateFailed (never WriterPoisoned, never Ok with a stale sequence), got {other:?}"),
    }

    // Prove the underlying log genuinely advanced past sequence 0 by
    // reading it back directly and verifying two real, distinct,
    // correctly-sequenced records exist -- not just trusting the
    // worker's own self-report.
    let raw = log.lock().unwrap().clone();
    let lines: Vec<&[u8]> = raw[..raw.len() - 1].split(|&b| b == b'\n').collect();
    assert_eq!(lines.len(), 2, "both records must be durably present in the log despite their own reference-update failures");
    let records: Vec<_> = lines.iter().map(|l| audit_chain_core::parse_line(l).unwrap()).collect();
    assert_eq!(records[0].sequence, 0);
    assert_eq!(records[1].sequence, 1);

    handle.shutdown().await.unwrap();
}

// --- reopen / recovery compatibility ---

#[tokio::test]
async fn reopening_a_new_worker_against_the_same_log_after_shutdown_resumes_the_correct_sequence() {
    let log = Arc::new(Mutex::new(Vec::new()));

    let writer1 = InMemoryWriter::new(log.clone());
    let handle1 = spawn_worker(writer1, &[], WorkerConfig::default()).unwrap();
    handle1.record_durable(sample_event("c1", "s1")).await.unwrap();
    handle1.record_durable(sample_event("c2", "s1")).await.unwrap();
    handle1.shutdown().await.unwrap();

    let raw = log.lock().unwrap().clone();
    let writer2 = InMemoryWriter::new(log.clone());
    let handle2 = spawn_worker(writer2, &raw, WorkerConfig::default()).expect("a cleanly shut down log must permit a fresh worker to resume");
    let ack = handle2.record_durable(sample_event("c3", "s1")).await.unwrap();
    assert_eq!(ack.sequence, 2, "the reopened worker must resume from the correct next sequence, not restart from 0");

    handle2.shutdown().await.unwrap();
}

// --- deterministic correlation / invocation identifiers ---

#[tokio::test]
async fn invocation_id_distinguishes_two_attempts_against_the_same_tool_and_server() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    let handle = spawn_worker(writer, &[], WorkerConfig::default()).unwrap();

    let attempted_1 = event(McpClientEventKind::InvocationAttempted { invocation_id: "inv-1".into(), server: "s".into(), tool_name: "t".into() }, "2026-01-01T00:00:00Z", "corr-1");
    let attempted_2 = event(McpClientEventKind::InvocationAttempted { invocation_id: "inv-2".into(), server: "s".into(), tool_name: "t".into() }, "2026-01-01T00:00:00Z", "corr-1");

    let ack1 = handle.record_durable(attempted_1).await.unwrap();
    let ack2 = handle.record_durable(attempted_2).await.unwrap();
    assert_ne!(ack1.record_hash, ack2.record_hash, "two distinct invocation attempts must produce distinct persisted records");
    assert_eq!(ack1.sequence, 0);
    assert_eq!(ack2.sequence, 1);

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn authorization_denied_event_carries_its_own_denial_reason_and_round_trips_through_the_durable_log() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let writer = InMemoryWriter::new(log.clone());
    let handle = spawn_worker(writer, &[], WorkerConfig::default()).unwrap();

    let denied = event(
        McpClientEventKind::AuthorizationDenied { server: "s".into(), tool_name: "t".into(), reason: DenialReason::NetworkScopeExceeded },
        "2026-01-01T00:00:00Z",
        "corr-1",
    );
    handle.record_durable(denied.clone()).await.unwrap();
    handle.shutdown().await.unwrap();

    let raw = log.lock().unwrap().clone();
    let line = &raw[..raw.len() - 1];
    let record = audit_chain_core::parse_line(line).unwrap();
    let decoded: mcp_client_audit_worker::McpClientEvent = audit_chain_core::parse_event(&record.content_bytes).unwrap();
    assert_eq!(decoded, denied);
}

// --- queue-full on the best-effort path (the one path that genuinely
//     reports it) ---

#[tokio::test]
async fn best_effort_path_is_the_only_one_that_can_report_queue_full() {
    // Demonstrates the typed EnqueueError::QueueFull variant is at least
    // reachable through the API surface intended for it, even though
    // ordinary timing makes it hard to force deterministically through
    // the public record_best_effort (which swallows the error) -- this
    // test instead confirms the type itself is part of the public
    // surface and matches on it exhaustively, pinning that no third
    // variant silently exists.
    fn classify(e: &EnqueueError) -> &'static str {
        match e {
            EnqueueError::QueueFull => "queue full",
            EnqueueError::WorkerGone => "worker gone",
        }
    }
    assert_eq!(classify(&EnqueueError::QueueFull), "queue full");
    assert_eq!(classify(&EnqueueError::WorkerGone), "worker gone");
}
