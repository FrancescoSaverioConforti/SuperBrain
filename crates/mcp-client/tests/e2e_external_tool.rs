//! B5 Task 32: E2E flow 2 — SuperBrain's own `mcp-client` registers a
//! real external MCP server, discovers it, and invokes one of its real
//! tools under a real `ApprovalPolicy`, receiving a real result over a
//! real transport. Every "external server" in this file is a genuinely
//! separate process (a minimal Python stdlib script speaking real MCP
//! over real stdio, the same established pattern as `src/discovery.rs`'s
//! and `src/invoke.rs`'s own end-to-end test modules) — never an in-process
//! mock object standing in for the transport, the protocol, or
//! `authorize`'s own decision logic. A trivial tool implementation
//! (`echo`) inside that real process is what the plan's own "mock
//! external MCP server" phrase means here: the TOOL's business logic is
//! a stand-in for a real third-party integration, but the process,
//! transport, protocol handshake, and every one of `mcp-client`'s own
//! real functions under test are genuinely real.
//!
//! Composition under test, end to end: [`connect`] → [`discover_tools`]
//! → (caller builds its own [`ExternalToolMetadata`], never auto-derived
//! from the discovered tool — see `permission.rs`'s own module doc
//! comment) → [`invoke_external_tool_logged`] (which itself calls
//! [`authorize`] first). Nothing in this file bypasses that order or
//! substitutes a shortcut for any step.

use std::time::Duration;

use mcp_client::{ApprovalPolicy, ExternalToolMetadata, McpClientError, McpServerRegistration, ScopeLimits, Sensitivity, ToolClassification, TransportConfig, connect, discover_tools, invoke_external_tool_logged};

fn python_or_skip() -> Option<&'static str> {
    ["python", "python3"].into_iter().find(|candidate| std::process::Command::new(candidate).arg("--version").output().is_ok_and(|o| o.status.success()))
}

fn skip_if_no_python() -> bool {
    if python_or_skip().is_none() {
        eprintln!("skipping: no python3/python interpreter found on PATH");
        true
    } else {
        false
    }
}

/// A real, minimal external MCP server whose `echo` tool actually
/// answers `tools/call` by echoing its own arguments back as structured
/// content — real `tools/list` and real `tools/call` support, same
/// shape as `src/invoke.rs`'s own `ECHO_SERVER_SCRIPT`.
const ECHO_SERVER_SCRIPT: &str = r#"
import sys, json

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    method = req.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "e2e-echo-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "tools": [
                {"name": "echo", "description": "echoes input", "inputSchema": {"type": "object"}},
            ]
        }})
    elif method == "tools/call":
        params = req.get("params", {})
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "content": [],
            "structuredContent": {"echoed": params.get("arguments", {})},
        }})
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

/// Completes the MCP `initialize` handshake, then exits immediately —
/// never answers `tools/list`/`tools/call` at all. A genuinely
/// unavailable TOOL (the server itself is reachable and briefly alive,
/// but offers nothing further), distinct from the hanging-server case
/// below (which stays running but never replies).
const IMMEDIATELY_EXITING_SERVER_SCRIPT: &str = r#"
import sys, json

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

line = sys.stdin.readline().strip()
req = json.loads(line)
if req.get("method") == "initialize":
    send({"jsonrpc": "2.0", "id": req["id"], "result": {
        "protocolVersion": "2025-11-25",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "e2e-vanishing-server", "version": "0.0.1"},
    }})
sys.exit(0)
"#;

/// Completes the handshake and answers `tools/list`, but never answers
/// `tools/call` — a genuinely hung real tool (real process, real
/// transport, no simulated delay).
const HANGING_TOOL_SERVER_SCRIPT: &str = r#"
import sys, json

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    method = req.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "e2e-hanging-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "tools": [
                {"name": "echo", "description": "echoes input", "inputSchema": {"type": "object"}},
            ]
        }})
    elif method == "tools/call":
        pass  # never answers
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

/// A real server whose `tools/call` reply is well-formed MCP protocol
/// JSON, but carries a self-reported `tool_name` disagreeing with the
/// tool actually invoked — the one identity claim `validate_result`
/// (Task 29) can genuinely check and reject.
const SPOOFING_SERVER_SCRIPT: &str = r#"
import sys, json

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    method = req.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "e2e-spoofing-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "tools": [
                {"name": "echo", "description": "echoes input", "inputSchema": {"type": "object"}},
            ]
        }})
    elif method == "tools/call":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "content": [],
            "structuredContent": {"tool_name": "not-echo", "value": 1},
        }})
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

fn registration(name: &str, script: &str) -> McpServerRegistration {
    let python = python_or_skip().expect("checked by caller via skip_if_no_python");
    McpServerRegistration { name: name.to_string(), transport: TransportConfig::ChildProcess { command: python.to_string(), args: vec!["-c".to_string(), script.to_string()] }, enabled: true }
}

fn echo_metadata(server: &str) -> ExternalToolMetadata {
    ExternalToolMetadata { server: server.to_string(), tool_name: "echo".to_string(), classification: ToolClassification::Execute, network_required: false, filesystem_scope: None, sensitivity: Sensitivity::Internal }
}

fn scoped_policy() -> ApprovalPolicy {
    ApprovalPolicy::AllowScopedExecution(ScopeLimits { allow_network: false, allowed_filesystem_scope: None })
}

// --- the real, full flow: register -> discover -> authorize -> invoke ---

#[tokio::test]
async fn a_real_external_tool_is_registered_discovered_and_invoked_under_allow_scoped_execution() {
    if skip_if_no_python() {
        return;
    }
    let server_name = "e2e-echo-server";
    let reg = registration(server_name, ECHO_SERVER_SCRIPT);
    let conn = connect(&reg).await.expect("connect to the real echo server must succeed");

    let mut cache = mcp_client::ToolCache::new();
    let discovered = discover_tools(&conn, server_name, &mut cache).await.expect("discover_tools against the real server must succeed");
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].name, "echo");
    assert_eq!(discovered[0].classification, None, "discovery never classifies -- Task 28's own placeholder rule");

    // The caller's OWN judgment, never derived from `discovered[0]`'s
    // server-supplied name/description/schema -- see permission.rs's
    // own module doc comment on why `authorize` has no code path that
    // reads a `DiscoveredTool` at all.
    let metadata = echo_metadata(server_name);
    let policy = scoped_policy();

    let result = invoke_external_tool_logged(&conn, &metadata, &policy, serde_json::json!({"text": "hello"}), Duration::from_secs(5), &mut mcp_client::NullSink).await;

    let value = result.expect("a genuinely authorized, well-behaved real tool call must succeed");
    assert_eq!(value["structuredContent"]["echoed"]["text"], "hello", "the real external tool's own real echoed reply must come through, got {value:?}");

    conn.close().await.unwrap();
}

// --- Step 5's required variants ---

#[tokio::test]
async fn denied_tool_is_never_invoked() {
    if skip_if_no_python() {
        return;
    }
    let server_name = "e2e-echo-server-deny";
    let reg = registration(server_name, ECHO_SERVER_SCRIPT);
    // Connect to a REAL, running echo server -- if authorize() did not
    // run first and short-circuit before any tools/call send, this
    // would reach the server and succeed (it really does answer echo).
    // Getting PermissionDenied here, with no round-trip delay, is the
    // proof denial happens before the network is ever touched.
    let conn = connect(&reg).await.expect("connect must succeed");
    let metadata = echo_metadata(server_name);

    let result = invoke_external_tool_logged(&conn, &metadata, &ApprovalPolicy::Deny, serde_json::json!({}), Duration::from_secs(5), &mut mcp_client::NullSink).await;

    assert_eq!(result, Err(McpClientError::PermissionDenied));
    conn.close().await.unwrap();
}

#[tokio::test]
async fn unavailable_server_returns_a_typed_error_not_a_hang() {
    if skip_if_no_python() {
        return;
    }
    let server_name = "e2e-vanishing-server";
    let reg = registration(server_name, IMMEDIATELY_EXITING_SERVER_SCRIPT);
    let conn = connect(&reg).await.expect("the handshake itself must still succeed -- the server is reachable just long enough to complete it, then exits");
    let metadata = echo_metadata(server_name);
    let policy = scoped_policy();

    let result = invoke_external_tool_logged(&conn, &metadata, &policy, serde_json::json!({}), Duration::from_secs(5), &mut mcp_client::NullSink).await;

    assert!(result.is_err(), "invoking a tool on a server that has already exited must be a typed error, got {result:?}");
    assert_ne!(result, Err(McpClientError::PermissionDenied), "this must fail for a REAL connection/transport reason, not be confused with an authorization denial");

    conn.close().await.ok();
}

#[tokio::test]
async fn timeout_is_enforced() {
    if skip_if_no_python() {
        return;
    }
    let server_name = "e2e-hanging-server";
    let reg = registration(server_name, HANGING_TOOL_SERVER_SCRIPT);
    let conn = connect(&reg).await.expect("connect must succeed; the hang is only in tools/call, not the handshake");
    let metadata = echo_metadata(server_name);
    let policy = scoped_policy();

    let result = invoke_external_tool_logged(&conn, &metadata, &policy, serde_json::json!({}), Duration::from_millis(500), &mut mcp_client::NullSink).await;

    assert_eq!(result, Err(McpClientError::Timeout));
    conn.close().await.unwrap();
}

#[tokio::test]
async fn malformed_response_is_rejected() {
    if skip_if_no_python() {
        return;
    }
    let server_name = "e2e-spoofing-server";
    let reg = registration(server_name, SPOOFING_SERVER_SCRIPT);
    let conn = connect(&reg).await.expect("connect must succeed; the handshake and tools/list are both well-formed");
    let metadata = echo_metadata(server_name);
    let policy = scoped_policy();

    let result = invoke_external_tool_logged(&conn, &metadata, &policy, serde_json::json!({}), Duration::from_secs(5), &mut mcp_client::NullSink).await;

    assert!(matches!(result, Err(McpClientError::MalformedResponse(_))), "a reply whose own structured_content claims a different tool name must be rejected, got {result:?}");
    conn.close().await.unwrap();
}

#[tokio::test]
async fn revoking_permission_mid_session_denies_the_next_call() {
    if skip_if_no_python() {
        return;
    }
    let server_name = "e2e-echo-server-revoke";
    let reg = registration(server_name, ECHO_SERVER_SCRIPT);
    let conn = connect(&reg).await.expect("connect must succeed");
    let metadata = echo_metadata(server_name);

    // First call: genuinely authorized, over the SAME live connection
    // (the "session") -- succeeds for real.
    let allowed = scoped_policy();
    let first = invoke_external_tool_logged(&conn, &metadata, &allowed, serde_json::json!({"text": "first"}), Duration::from_secs(5), &mut mcp_client::NullSink).await;
    assert!(first.is_ok(), "the first, genuinely-authorized call must succeed, got {first:?}");

    // "Revoking permission mid-session": the connection/session itself
    // is untouched -- still the same live `conn` -- but the policy
    // supplied to the NEXT call is now denying. `authorize` is a pure
    // function re-evaluated fresh on every single invocation (Task 28's
    // own design: it has no cache, no session-sticky state at all), so
    // this is the real mechanism by which a caller revokes permission
    // between two calls on one connection -- never a special "session
    // revocation" API this crate would need to invent.
    let revoked = ApprovalPolicy::Deny;
    let second = invoke_external_tool_logged(&conn, &metadata, &revoked, serde_json::json!({"text": "second"}), Duration::from_secs(5), &mut mcp_client::NullSink).await;
    assert_eq!(second, Err(McpClientError::PermissionDenied), "the next call, under a now-denying policy, must be denied even though the underlying connection/session never changed");

    conn.close().await.unwrap();
}
