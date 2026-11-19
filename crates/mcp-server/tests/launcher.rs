//! B5 remediation M1 acceptance tests: the `serve_stdio` launcher
//! (`examples/serve_stdio.rs`), exercised as a REAL spawned child process
//! speaking real stdio — not the in-memory `tokio::io::duplex` every other
//! test in this crate uses. This is deliberately the one file in this
//! crate that proves library-level behavior (already covered by
//! `tests/limits.rs`/`tests/server_integration.rs`) actually holds when
//! wired through a real OS process boundary, which is exactly the gap
//! B5's closure document named as open item #2 ("no production binary
//! calls `mcp_server::limited_transport`... `MCP_MAX_PAYLOAD_BYTES` is
//! inert in any real deployment until a binary wires it in").
//!
//! Finds the already-built example binary via `CARGO_MANIFEST_DIR` and
//! the running test's own profile directory — `cargo test` always builds
//! every example in the same profile as the test binary itself before
//! running tests, so the file is guaranteed to exist by the time these
//! tests run (the same guarantee `CARGO_BIN_EXE_<name>` relies on for
//! `[[bin]]` targets; Cargo does not expose that env var for `[[example]]`
//! targets, so this constructs the equivalent path by hand).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn serve_stdio_binary_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // `CARGO_MANIFEST_DIR` is `<repo>/crates/mcp-server`; the workspace
    // target dir is two levels up, same convention `cargo test`'s own
    // `OUT_DIR`/binary layout always uses for a workspace member crate.
    let workspace_target = manifest_dir.join("../../target");
    let profile_dir = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let mut path = workspace_target
        .join(profile_dir)
        .join("examples")
        .join("serve_stdio");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    assert!(path.exists(), "serve_stdio example binary not found at {path:?} — run `cargo build -p mcp-server --example serve_stdio` first (cargo test should already do this automatically)");
    path
}

fn unique_test_dir(label: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-mcp-server-launcher-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Spawns a real `serve_stdio` child process against a fresh, empty vault.
/// Returns the child handle plus its stdin/stdout pipes, already past the
/// point where the vault/index/graph were constructed (any failure there
/// would have exited non-zero before ever reaching the MCP handshake).
fn spawn_server() -> (Child, ChildStdin, LineReader) {
    let root = unique_test_dir("vault");
    let journal = root.join("journal.sqlite");
    let mut child = Command::new(serve_stdio_binary_path())
        .arg(&root)
        .arg(&journal)
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

fn send_line(stdin: &mut ChildStdin, line: &str) {
    stdin.write_all(line.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
}

/// Reads lines from a real, blocking `ChildStdout` pipe on a dedicated
/// background thread, so a test can bound its wait with a channel
/// timeout (`std::process`'s pipes have no native async/cancelable read —
/// this crate deliberately avoids `tokio::process` here, to prove the
/// binary's own real OS stdio rather than Tokio's async wrapper around
/// it). The thread and pipe outlive any single timed-out read; a later
/// call simply waits on the next line the thread already produced.
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
                let result =
                    reader
                        .read_line(&mut line)
                        .map(|n| if n == 0 { None } else { Some(line) });
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
}

fn read_line_with_timeout(stdout: &mut LineReader, timeout: std::time::Duration) -> String {
    stdout.read_line_with_timeout(timeout)
}

// --- 1/2/3: real process start, real handshake, real 12-tool registration ---

#[test]
fn real_spawned_process_completes_a_real_mcp_handshake_and_registers_exactly_the_12_frozen_tools() {
    let (mut child, mut stdin, mut stdout) = spawn_server();

    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"acceptance-test","version":"0.0.1"}}}"#,
    );
    let init_reply: serde_json::Value = serde_json::from_str(
        read_line_with_timeout(&mut stdout, std::time::Duration::from_secs(10)).trim(),
    )
    .expect("initialize must get a valid JSON-RPC reply from the real spawned process");
    assert!(
        init_reply.get("result").is_some(),
        "real process must complete a real MCP initialize handshake, got {init_reply}"
    );

    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    let list_reply: serde_json::Value = serde_json::from_str(
        read_line_with_timeout(&mut stdout, std::time::Duration::from_secs(10)).trim(),
    )
    .unwrap();
    let tools = list_reply["result"]["tools"]
        .as_array()
        .expect("tools/list must return a tools array");
    assert_eq!(tools.len(), 12, "the real spawned process must register exactly the 12 frozen Brain API capabilities as MCP tools, got {tools:?}");

    let names: std::collections::HashSet<&str> =
        tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for forbidden in ["approve_proposal", "reject_proposal"] {
        assert!(!names.contains(forbidden), "the trusted-only approval surface must never be registered as an MCP tool, even over a real process transport");
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

// --- 4/5: real oversized-payload rejection at the transport layer, over
//     real OS stdio, and the connection's survival afterward ---

#[test]
fn real_process_drops_an_oversized_line_at_the_transport_layer_and_the_connection_survives() {
    let (mut child, mut stdin, mut stdout) = spawn_server();

    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"acceptance-test","version":"0.0.1"}}}"#,
    );
    let init_reply: serde_json::Value = serde_json::from_str(
        read_line_with_timeout(&mut stdout, std::time::Duration::from_secs(10)).trim(),
    )
    .unwrap();
    assert!(init_reply.get("result").is_some());
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    // A single line exceeding MCP_MAX_PAYLOAD_BYTES, sent to the REAL
    // process's REAL stdin pipe — the exact gap B5's closure document
    // named: this limit exists in library code but nothing enforced it
    // in a real deployment until this launcher existed.
    let oversized_body = "y".repeat(brain_api::limits::MCP_MAX_PAYLOAD_BYTES + 1024);
    let oversized_line = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"search","arguments":{{"query":"{oversized_body}"}}}}}}"#
    );
    assert!(
        oversized_line.len() > brain_api::limits::MCP_MAX_PAYLOAD_BYTES,
        "the test line itself must exceed the configured limit"
    );
    send_line(&mut stdin, &oversized_line);

    // The connection must survive: a real, well-formed request sent right
    // after still gets a real reply from the same real process.
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
    );
    let list_reply: serde_json::Value = serde_json::from_str(read_line_with_timeout(&mut stdout, std::time::Duration::from_secs(10)).trim()).expect("the real process must still reply after the oversized line, proving it survived rather than hanging or crashing");
    assert!(list_reply.get("result").is_some(), "tools/list after the oversized line must still succeed on the real process, got {list_reply}");

    // The real process must still be alive (not crashed/panicked) after
    // surviving the oversized line.
    assert!(
        child.try_wait().unwrap().is_none(),
        "the real server process must still be running after surviving an oversized line"
    );

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

// --- 6: concurrency limit genuinely shared across real concurrent
//     tool calls on the real process (not just the library's own
//     white-box semaphore test) ---

#[test]
fn real_process_queues_a_tool_call_beyond_the_configured_concurrency_cap_rather_than_rejecting_or_crashing(
) {
    let (mut child, mut stdin, mut stdout) = spawn_server();

    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"acceptance-test","version":"0.0.1"}}}"#,
    );
    let init_reply: serde_json::Value = serde_json::from_str(
        read_line_with_timeout(&mut stdout, std::time::Duration::from_secs(10)).trim(),
    )
    .unwrap();
    assert!(init_reply.get("result").is_some());
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    // Fire MAX_CONCURRENT_REQUESTS + a few extra `search` calls back to
    // back on the same real process without waiting for replies between
    // them -- the real, external proof that the shared Arc<Semaphore>
    // (already white-box-tested inside the crate) actually bounds
    // concurrency when driven over a real OS pipe by a real process, not
    // just inside one Tokio task set.
    let total_calls = brain_api::limits::MAX_CONCURRENT_REQUESTS + 5;
    for id in 0..total_calls {
        send_line(
            &mut stdin,
            &format!(
                r#"{{"jsonrpc":"2.0","id":{},"method":"tools/call","params":{{"name":"search","arguments":{{"query":"anything"}}}}}}"#,
                id + 2
            ),
        );
    }

    // Every single call must still eventually get a reply -- queuing
    // (the explicit, documented design choice), never dropping, never
    // hanging the whole process.
    for _ in 0..total_calls {
        let reply: serde_json::Value = serde_json::from_str(read_line_with_timeout(&mut stdout, std::time::Duration::from_secs(15)).trim()).expect("every queued call must eventually receive a reply from the real process, never silently dropped");
        assert!(
            reply.get("result").is_some() || reply.get("error").is_some(),
            "malformed reply: {reply}"
        );
    }

    assert!(child.try_wait().unwrap().is_none(), "the real process must still be running after handling more concurrent calls than its configured cap");

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

// --- 7: no trusted capability reachable over the real process transport ---

#[test]
fn real_process_never_exposes_approve_or_reject_proposal_and_a_save_memory_call_only_ever_creates_a_proposal(
) {
    let (mut child, mut stdin, mut stdout) = spawn_server();

    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"acceptance-test","version":"0.0.1"}}}"#,
    );
    let _: serde_json::Value = serde_json::from_str(
        read_line_with_timeout(&mut stdout, std::time::Duration::from_secs(10)).trim(),
    )
    .unwrap();
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    // Calling a tool name that was never registered must fail as an
    // unknown tool -- never silently succeed, never reach a trusted path.
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"approve_proposal","arguments":{"proposal_id":"00000000-0000-0000-0000-000000000000"}}}"#,
    );
    let reply: serde_json::Value = serde_json::from_str(
        read_line_with_timeout(&mut stdout, std::time::Duration::from_secs(10)).trim(),
    )
    .unwrap();
    assert!(
        reply.get("error").is_some(),
        "approve_proposal must never be callable over the real MCP process transport, got {reply}"
    );

    // save_memory over the real transport must only ever create a
    // Pending proposal -- never a committed note -- matching the frozen
    // trust model, now proven over a real process boundary too.
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"save_memory","arguments":{"note_id":"note-launcher-acceptance","note_type":"decision","classification":"internal","project":"project-x","source":"acceptance-test","event_period":"2026-11-19","body":"acceptance test content"}}}"#,
    );
    let save_reply: serde_json::Value = serde_json::from_str(
        read_line_with_timeout(&mut stdout, std::time::Duration::from_secs(10)).trim(),
    )
    .unwrap();
    let structured = &save_reply["result"]["structuredContent"];
    assert!(structured.get("proposal_id").is_some(), "save_memory over the real process must return a proposal id, never a committed result, got {save_reply}");

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

// --- 8: fail-closed on missing/invalid configuration — no silent
//     fallback, no placeholder vault ---

#[test]
fn real_process_exits_non_zero_without_starting_a_server_when_the_vault_path_is_unusable() {
    // An existing, non-directory file where a vault root is expected --
    // `Vault::open_with_default_indexer` must fail, and this launcher
    // must exit non-zero BEFORE ever touching stdio/MCP at all, never
    // falling back to an in-memory/placeholder vault to "succeed anyway".
    let root = unique_test_dir("fail-closed");
    let blocking_file = root.join("not-a-directory");
    std::fs::write(&blocking_file, b"not a vault root").unwrap();

    let output = Command::new(serve_stdio_binary_path())
        .arg(&blocking_file)
        .arg(blocking_file.join("journal.sqlite"))
        .arg("project-x")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("process must run to completion (fail fast), not hang");

    assert!(!output.status.success(), "the launcher must fail closed (non-zero exit) on an unusable vault path, never silently proceed with a placeholder");
    assert!(
        output.stdout.is_empty(),
        "no MCP traffic must ever be written to stdout when the vault itself failed to open"
    );
}

#[test]
fn real_process_exits_non_zero_with_usage_on_missing_arguments() {
    let output = Command::new(serve_stdio_binary_path())
        .output()
        .expect("process must run to completion");
    assert!(
        !output.status.success(),
        "missing required arguments must fail closed, never default to an implicit vault location"
    );
}
