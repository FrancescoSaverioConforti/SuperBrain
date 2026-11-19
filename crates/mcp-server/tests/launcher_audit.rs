//! M5 Task 6 acceptance tests: the real `serve_stdio` launcher
//! (`examples/serve_stdio.rs`), exercised as a REAL spawned child process
//! speaking real stdio, now ALSO proving the production audit worker
//! wiring added in this task — mirrors `tests/launcher.rs`'s own harness
//! style exactly (real process, real stdio, real pipes), kept in its own
//! file since these tests additionally depend on `brain-api-audit-worker`
//! (a `dev-dependency` of this crate, used only by the `serve_stdio`
//! example and by these tests themselves to independently verify/reopen
//! the audit log the real process produced).
//!
//! **What these tests prove that `tests/launcher.rs` does not**: that a
//! real tool call over the real process boundary causes a real,
//! verifiable `AuditEvent` to land in `.audit/audit.log`; that the
//! DPAPI head reference agrees with the log on reopen; that a corrupted
//! audit store makes the real process fail closed before ever answering
//! MCP traffic; that a second process cannot concurrently own the audit
//! writer; and that the 12-tool/no-trusted-tool guarantees `tests/
//! launcher.rs` already proves still hold with the real audit sink wired
//! in (never a regression introduced by this task).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn serve_stdio_binary_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_target = manifest_dir.join("../../target");
    let profile_dir = if cfg!(debug_assertions) { "debug" } else { "release" };
    let mut path = workspace_target.join(profile_dir).join("examples").join("serve_stdio");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    assert!(path.exists(), "serve_stdio example binary not found at {path:?} — run `cargo build -p mcp-server --example serve_stdio` first");
    path
}

fn unique_test_dir(label: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-mcp-server-launcher-audit-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct LineReader {
    rx: std::sync::mpsc::Receiver<std::io::Result<String>>,
}

impl LineReader {
    fn new(stdout: ChildStdout) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut reader = std::io::BufReader::new(stdout);
            loop {
                let mut line = String::new();
                let result = reader.read_line(&mut line).map(|n| if n == 0 { None } else { Some(line) });
                let to_send = match result {
                    Ok(Some(line)) => Ok(line),
                    Ok(None) => break,
                    Err(err) => Err(err),
                };
                if tx.send(to_send).is_err() {
                    break;
                }
            }
        });
        Self { rx }
    }

    fn read_line_with_timeout(&self, timeout: std::time::Duration) -> String {
        match self.rx.recv_timeout(timeout) {
            Ok(Ok(line)) => line,
            Ok(Err(err)) => panic!("failed to read a line from the server's stdout: {err}"),
            Err(_) => panic!("server did not reply within {timeout:?}"),
        }
    }

    /// Non-panicking variant: `None` on a timeout or stream end, used
    /// only where a caller genuinely cannot know in advance how many
    /// replies will arrive (e.g. after sending real cancellations for
    /// some of a burst of requests).
    fn try_read_line_with_timeout(&self, timeout: std::time::Duration) -> Option<String> {
        self.rx.recv_timeout(timeout).ok().and_then(|r| r.ok())
    }
}

fn send_line(stdin: &mut ChildStdin, line: &str) {
    stdin.write_all(line.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
}

fn spawn_server_against(root: &std::path::Path, journal: &std::path::Path) -> (Child, ChildStdin, LineReader) {
    let mut child = Command::new(serve_stdio_binary_path())
        .arg(root)
        .arg(journal)
        .arg("project-x")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("serve_stdio must spawn as a real child process");
    let stdin = child.stdin.take().unwrap();
    let stdout = LineReader::new(child.stdout.take().unwrap());
    (child, stdin, stdout)
}

fn initialize(stdin: &mut ChildStdin, stdout: &mut LineReader) {
    send_line(stdin, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"acceptance-test","version":"0.0.1"}}}"#);
    let init_reply: serde_json::Value = serde_json::from_str(stdout.read_line_with_timeout(std::time::Duration::from_secs(10)).trim()).expect("initialize must get a valid JSON-RPC reply");
    assert!(init_reply.get("result").is_some(), "initialize must succeed, got {init_reply}");
    send_line(stdin, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
}

fn read_full_log(root: &std::path::Path, journal: &std::path::Path) -> Vec<u8> {
    let vault = storage::Vault::open_with_default_indexer(root.to_path_buf(), journal).unwrap();
    vault.read_audit_log_for_recovery_scan().unwrap()
}

fn verified_chain(raw: &[u8]) -> brain_api::ScanResult {
    let lines: Vec<brain_api::LineOutcome> = if raw.is_empty() {
        Vec::new()
    } else {
        let ends_with_newline = raw.last() == Some(&b'\n');
        let trimmed = if ends_with_newline { &raw[..raw.len() - 1] } else { raw };
        let raw_lines: Vec<&[u8]> = trimmed.split(|&b| b == b'\n').collect();
        let last_index = raw_lines.len().saturating_sub(1);
        raw_lines
            .into_iter()
            .enumerate()
            .map(|(index, line)| match brain_api::parse_line(line) {
                Ok(record) => brain_api::LineOutcome::Parsed(record),
                Err(_) => brain_api::LineOutcome::Unparseable { is_physical_tail: !ends_with_newline && index == last_index },
            })
            .collect()
    };
    brain_api::scan_chain(&lines)
}

// --- 1/2: startup creates the expected audit store, a real tool call
//     persists a real AuditEvent ---

#[test]
fn a_real_tool_call_over_the_real_process_persists_a_verifiable_audit_event() {
    let root = unique_test_dir("real-tool-call");
    let journal = root.join("journal.sqlite");
    let (mut child, mut stdin, mut stdout) = spawn_server_against(&root, &journal);
    initialize(&mut stdin, &mut stdout);

    send_line(&mut stdin, r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"anything","k":10}}}"#);
    let reply: serde_json::Value = serde_json::from_str(stdout.read_line_with_timeout(std::time::Duration::from_secs(10)).trim()).unwrap();
    assert!(reply.get("result").is_some() || reply.get("error").is_some(), "malformed reply: {reply}");

    // Graceful shutdown: close stdin (EOF), which `rmcp`'s own transport
    // treats as the client disconnecting -- the real process then runs
    // this task's own bounded audit-worker drain before exiting.
    drop(stdin);
    let status = child.wait().expect("the real process must exit on stdin EOF");
    assert!(status.success(), "the real process must exit cleanly after a normal client disconnect, got {status:?}");

    let raw = read_full_log(&root, &journal);
    assert!(!raw.is_empty(), "the real process must have durably persisted at least one audit record for the tool call above");
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "the real process's own audit log must verify cleanly end to end, got {:?}", scan.stop_reason);
    assert!(!scan.verified_records.is_empty());

    let has_tool_call = scan.verified_records.iter().any(|r| {
        brain_api::parse_event(&r.content_bytes)
            .map(|e| matches!(e.kind, brain_api::AuditEventKind::ToolCall { ref tool, .. } if tool == "search"))
            .unwrap_or(false)
    });
    assert!(has_tool_call, "the real search tool call must have produced a real, persisted ToolCall{{tool:\"search\"}} audit event");
}

// --- 3: multiple tool calls form a valid, contiguous hash chain ---

#[test]
fn multiple_real_tool_calls_form_a_valid_contiguous_hash_chain() {
    let root = unique_test_dir("contiguous-chain");
    let journal = root.join("journal.sqlite");
    let (mut child, mut stdin, mut stdout) = spawn_server_against(&root, &journal);
    initialize(&mut stdin, &mut stdout);

    for id in 0..5 {
        send_line(&mut stdin, &format!(r#"{{"jsonrpc":"2.0","id":{},"method":"tools/call","params":{{"name":"search","arguments":{{"query":"q{}","k":10}}}}}}"#, id + 2, id));
    }
    for _ in 0..5 {
        let reply: serde_json::Value = serde_json::from_str(stdout.read_line_with_timeout(std::time::Duration::from_secs(10)).trim()).unwrap();
        assert!(reply.get("result").is_some() || reply.get("error").is_some());
    }

    drop(stdin);
    let _ = child.wait();

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "got {:?}", scan.stop_reason);
    assert_eq!(scan.verified_records.len(), 5, "exactly 5 ToolCall events, one per real tool call, must have landed");
    for (i, record) in scan.verified_records.iter().enumerate() {
        assert_eq!(record.sequence, i as u64, "sequence numbers must be contiguous with no gaps or duplicates");
    }
}

// --- 4: audit log and DPAPI head reference agree on reopen ---

#[tokio::test]
async fn audit_log_and_head_reference_agree_on_reopen_after_the_real_process_exits() {
    let root = unique_test_dir("reopen-agreement");
    let journal = root.join("journal.sqlite");
    let (mut child, mut stdin, mut stdout) = spawn_server_against(&root, &journal);
    initialize(&mut stdin, &mut stdout);

    send_line(&mut stdin, r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"anything","k":10}}}"#);
    let _: serde_json::Value = serde_json::from_str(stdout.read_line_with_timeout(std::time::Duration::from_secs(10)).trim()).unwrap();

    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success());

    // A fresh worker against the SAME root/journal must be able to start
    // at all -- `spawn_worker`'s own startup recovery scan fails closed
    // (`WorkerStartupError::RecoveryRequired`/`OpenFailed`) on any
    // log/reference disagreement, so successfully spawning here is
    // exactly the "agree on reopen" proof this test needs, without this
    // crate reaching into storage's own private reference-file format.
    // Requires a real Tokio runtime context (`spawn_worker` itself calls
    // `tokio::spawn`), hence `#[tokio::test]` rather than a plain `#[test]`.
    let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
    let reopened = brain_api_audit_worker::spawn_worker(vault, brain_api_audit_worker::WorkerConfig::default());
    assert!(reopened.is_ok(), "a fresh worker must reopen successfully after the real process's own clean shutdown, proving the log and head reference agree, got {reopened:?}");
    reopened.unwrap().shutdown().await.unwrap();
}

// --- 5/6: the 12 tools remain available, no trusted tool reachable,
//     with the real audit sink now wired in (regression guard) ---

#[test]
fn all_12_tools_remain_registered_and_no_trusted_approval_tool_is_reachable_with_the_real_audit_sink_wired_in() {
    let root = unique_test_dir("tools-unchanged");
    let journal = root.join("journal.sqlite");
    let (mut child, mut stdin, mut stdout) = spawn_server_against(&root, &journal);
    initialize(&mut stdin, &mut stdout);

    send_line(&mut stdin, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let list_reply: serde_json::Value = serde_json::from_str(stdout.read_line_with_timeout(std::time::Duration::from_secs(10)).trim()).unwrap();
    let tools = list_reply["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 12, "the real process with the real audit worker wired in must still register exactly 12 tools, got {tools:?}");
    let names: std::collections::HashSet<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for forbidden in ["approve_proposal", "reject_proposal"] {
        assert!(!names.contains(forbidden));
    }

    send_line(&mut stdin, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"approve_proposal","arguments":{"proposal_id":"00000000-0000-0000-0000-000000000000"}}}"#);
    let reply: serde_json::Value = serde_json::from_str(stdout.read_line_with_timeout(std::time::Duration::from_secs(10)).trim()).unwrap();
    assert!(reply.get("error").is_some(), "approve_proposal must still never be callable, got {reply}");

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

// --- 7: corrupted audit startup state fails explicitly, without serving ---

#[test]
fn a_corrupted_audit_log_makes_the_real_process_fail_closed_before_serving_any_request() {
    let root = unique_test_dir("corrupt-audit-startup");
    let journal = root.join("journal.sqlite");
    // Pre-create a vault with a deliberately corrupt (unparseable, not at
    // the tail) audit log, mirroring brain-api-audit-worker's own
    // recovery test fixture exactly.
    {
        let vault = storage::Vault::open_with_default_indexer(root.clone(), &journal).unwrap();
        let audit_dir = root.join(".audit");
        std::fs::create_dir_all(&audit_dir).unwrap();
        std::fs::write(audit_dir.join("audit.log"), b"not valid json at all\n").unwrap();
        drop(vault);
    }

    let output = Command::new(serve_stdio_binary_path())
        .arg(&root)
        .arg(&journal)
        .arg("project-x")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("process must run to completion (fail fast), not hang");

    assert!(!output.status.success(), "the real process must exit non-zero when the audit store cannot be verified, got {output:?}");
    assert!(output.stdout.is_empty(), "no MCP traffic must ever be written to stdout when the audit store failed to open — this binary must fail BEFORE the handshake, never serve requests with a null/fallback sink");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("audit"), "the failure must be attributable to the audit worker, got stderr: {stderr}");
}

// --- 8: a second process cannot concurrently own the same audit writer ---

#[test]
fn a_second_real_process_cannot_concurrently_own_the_same_audit_writer() {
    let root = unique_test_dir("cross-process-exclusion");
    let journal = root.join("journal.sqlite");
    let (mut first, mut first_stdin, mut first_stdout) = spawn_server_against(&root, &journal);
    initialize(&mut first_stdin, &mut first_stdout);

    // A second real process against the SAME root/journal while the
    // first is still running must fail closed (the audit writer's own
    // OS-level exclusive lock, held for the first worker's whole
    // lifetime) -- never silently share, queue behind, or ignore it.
    let output = Command::new(serve_stdio_binary_path())
        .arg(&root)
        .arg(&journal)
        .arg("project-x")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("second process must run to completion");
    assert!(!output.status.success(), "a second concurrent process against the same audit store must fail closed while the first still holds the writer lock");
    assert!(output.stdout.is_empty(), "the second process must never serve MCP traffic while excluded from the audit writer");

    drop(first_stdin);
    let _ = first.kill();
    let _ = first.wait();
}

// --- 10/11: worker shutdown behavior is measured, queue-saturation/
//     best-effort semantics are not misrepresented as lossless ---

#[test]
fn clean_client_disconnect_reports_a_measured_graceful_audit_worker_drain_on_stderr() {
    let root = unique_test_dir("measured-shutdown");
    let journal = root.join("journal.sqlite");
    let (child, mut stdin, mut stdout) = spawn_server_against(&root, &journal);
    initialize(&mut stdin, &mut stdout);

    send_line(&mut stdin, r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"anything","k":10}}}"#);
    let _: serde_json::Value = serde_json::from_str(stdout.read_line_with_timeout(std::time::Duration::from_secs(10)).trim()).unwrap();

    drop(stdin);
    let output = child.wait_with_output().expect("process must exit after stdin EOF");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("audit worker drained and shut down gracefully"),
        "a clean disconnect must report a measured, honest graceful-drain outcome on stderr (not silence, not an unqualified blanket claim), got stderr: {stderr}"
    );
}

// --- 9: cancellation during queued MCP requests does not corrupt the
//     audit chain ---

#[test]
fn cancelling_queued_mcp_requests_over_the_real_wire_never_corrupts_the_audit_chain() {
    let root = unique_test_dir("cancellation-no-corruption");
    let journal = root.join("journal.sqlite");
    let (mut child, mut stdin, mut stdout) = spawn_server_against(&root, &journal);
    initialize(&mut stdin, &mut stdout);

    // Fire a burst of real tool calls, cancelling roughly half of them
    // immediately via a real `notifications/cancelled` for that exact
    // request id -- a genuine, wire-level reproduction of "an MCP caller
    // cancelled while its tool call (and the best-effort audit enqueue
    // inside it) was still in flight", not a white-box simulation.
    let total = 20;
    for id in 2..(2 + total) {
        send_line(&mut stdin, &format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"search","arguments":{{"query":"anything","k":10}}}}}}"#));
        if id % 2 == 0 {
            send_line(&mut stdin, &format!(r#"{{"jsonrpc":"2.0","method":"notifications/cancelled","params":{{"requestId":{id}}}}}"#));
        }
    }

    // Drain every reply that does arrive (a cancelled request's own
    // response may or may not be sent at all -- this test does not
    // assert on that; it only needs the server to keep responding to
    // SOME of the burst without hanging, then shut down cleanly).
    let mut received = 0;
    while received < total {
        if stdout.try_read_line_with_timeout(std::time::Duration::from_millis(500)).is_some() {
            received += 1;
        } else {
            break;
        }
    }

    drop(stdin);
    let status = child.wait().expect("the real process must exit after stdin EOF even after a burst of cancellations");
    assert!(status.success(), "the real process must still shut down cleanly after a burst of real cancellations, got {status:?}");

    let raw = read_full_log(&root, &journal);
    let scan = verified_chain(&raw);
    assert!(scan.stop_reason.is_none(), "cancellation must never corrupt the audit chain -- every record that DID land must still verify end to end, got {:?}", scan.stop_reason);
    let sequences: Vec<u64> = scan.verified_records.iter().map(|r| r.sequence).collect();
    let mut deduped = sequences.clone();
    deduped.dedup();
    assert_eq!(sequences, deduped, "no duplicate sequence numbers may appear even under a burst of real wire-level cancellations, got {sequences:?}");
}
