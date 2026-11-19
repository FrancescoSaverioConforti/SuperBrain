//! B5.9 Task 29: tool invocation, timeout, and a provisional invocation
//! log.
//!
//! `invoke_external_tool` is the ONE path this crate has for actually
//! calling a tool on an already-connected external server. Order of
//! operations, every step completing before the next begins:
//!   1. [`authorize`](crate::authorize) against `metadata`/`policy` — a
//!      denial returns [`McpClientError::PermissionDenied`] immediately,
//!      before `conn`'s transport is touched at all. No network round
//!      trip, no timeout clock started, nothing sent.
//!   2. The real `tools/call` request, by `metadata.tool_name`, raced
//!      against a timeout.
//!   3. Response validation (see `validate_result`) — a response that
//!      cannot be trusted as a real answer from the real tool is
//!      rejected, never passed through.
//!   4. A log record emitted REGARDLESS of outcome (see this module's own
//!      "Provisional invocation log" section below) — on authorization
//!      denial, on timeout, on a malformed response, and on success
//!      alike.
//!
//! ## Provisional invocation log (NOT Task 33's real audit trail)
//!
//! Task 33 (B5.12, not yet implemented) defines `brain_api::AuditEvent`/
//! `AuditEventKind` — a durable, structured, content-free audit schema
//! that lives in `crates/brain-api`. `mcp-client` has no dependency on
//! `brain-api` (see `lib.rs`'s own B5.10 isolation doc comment) and this
//! task does not add one, invent `brain-api`'s real schema early, or
//! duplicate it. [`InvocationLogEntry`] is a deliberately minimal,
//! LOCAL-to-this-crate, in-process record: it exists so this function's
//! own behavior ("a record is produced on every call, regardless of
//! outcome") is observable and testable today, not so it can stand in
//! for Task 33's real audit requirement.
//!
//! **This is diagnostic evidence, not a tamper-resistant security audit
//! trail.** It is not persisted, not signed, not append-only, and
//! callable code can construct or inspect one freely — none of the
//! properties a real audit mechanism needs. Task 33 is the task that
//! must wire this crate into `brain_api::AuditEvent` at the correct
//! architectural boundary (almost certainly: `mcp-client` emits its own
//! local record as it already does here, and a SEPARATE, trusted
//! boundary — not this crate — translates that into a real
//! `brain_api::AuditEvent`, since `mcp-client` itself cannot construct
//! one without depending on `brain-api`). This module makes no claim of
//! compliance with B5.12's real requirement.
//!
//! Every field on [`InvocationLogEntry`] is an identifier, boolean, or
//! category label — never the tool's real arguments or its real result
//! content, matching the content-free discipline Task 33's own frozen
//! schema already requires for its real variants (the same rule, applied
//! here in advance because it is simply correct practice, not because
//! this type IS that schema).

use std::time::Duration;

use rmcp::model::CallToolRequestParams;

use crate::{ApprovalPolicy, ExternalToolMetadata, McpClientError, McpConnection, authorize};

/// The default timeout for one `tools/call` round trip. Mirrors
/// `registration.rs`'s own `DEFAULT_CONNECT_TIMEOUT` convention — a
/// caller needing a different value uses
/// [`invoke_external_tool_with_timeout`].
pub(crate) const DEFAULT_INVOKE_TIMEOUT: Duration = Duration::from_secs(30);

/// The coarse outcome category this module's provisional log records —
/// never the tool's real result content, never its real error text
/// verbatim (a future real audit event, per Task 33's own frozen schema
/// style, categorizes a failure reason rather than echoing it verbatim).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationOutcome {
    Denied,
    Timeout,
    MalformedResponse,
    ConnectionFailed,
    Success,
}

/// See this module's own "Provisional invocation log" doc comment.
/// Deliberately minimal: identifiers and a category, nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationLogEntry {
    pub server: String,
    pub tool_name: String,
    pub outcome: InvocationOutcome,
}

/// A hook this module calls with exactly one [`InvocationLogEntry`] per
/// `invoke_external_tool`/`invoke_external_tool_with_timeout` call,
/// regardless of outcome. `invoke_external_tool` itself uses a no-op
/// sink (this task defines no real destination for the log — see the
/// module doc comment on why that is Task 33's job, not this one's);
/// [`invoke_external_tool_logged`] exists so tests, and a future trusted
/// boundary, can observe what was recorded without this module owning
/// any actual sink (a file, a channel, a database) itself.
pub trait InvocationLogSink {
    fn record(&mut self, entry: InvocationLogEntry);
}

/// The default sink: discards every entry. `invoke_external_tool` uses
/// this — today, nothing in this crate persists or forwards the log
/// anywhere; see the module doc comment.
#[derive(Debug, Default)]
pub struct NullSink;

impl InvocationLogSink for NullSink {
    fn record(&mut self, _entry: InvocationLogEntry) {}
}

/// Invokes `metadata.tool_name` on `conn` under `policy`, using
/// [`DEFAULT_INVOKE_TIMEOUT`]. See [`invoke_external_tool_with_timeout`]
/// for the full enforcement order and [`invoke_external_tool_logged`] for
/// a variant that exposes the provisional log entry to the caller.
pub async fn invoke_external_tool(conn: &McpConnection, metadata: &ExternalToolMetadata, policy: &ApprovalPolicy, args: serde_json::Value) -> Result<serde_json::Value, McpClientError> {
    invoke_external_tool_with_timeout(conn, metadata, policy, args, DEFAULT_INVOKE_TIMEOUT).await
}

/// Same as [`invoke_external_tool`], with an explicit timeout.
pub async fn invoke_external_tool_with_timeout(conn: &McpConnection, metadata: &ExternalToolMetadata, policy: &ApprovalPolicy, args: serde_json::Value, timeout: Duration) -> Result<serde_json::Value, McpClientError> {
    invoke_external_tool_logged(conn, metadata, policy, args, timeout, &mut NullSink).await
}

/// The real implementation. Enforcement order, every step completing
/// before the next begins (see the module doc comment's own numbered
/// list for the full rationale):
///   1. [`authorize`] — a denial returns immediately; `sink` still
///      records [`InvocationOutcome::Denied`], but `conn`'s transport is
///      never touched.
///   2. The real `tools/call` request, raced against `timeout`.
///   3. [`validate_result`] on a successful reply.
///   4. `sink.record(...)` — always, on every return path.
pub async fn invoke_external_tool_logged(
    conn: &McpConnection,
    metadata: &ExternalToolMetadata,
    policy: &ApprovalPolicy,
    args: serde_json::Value,
    timeout: Duration,
    sink: &mut dyn InvocationLogSink,
) -> Result<serde_json::Value, McpClientError> {
    let log = |outcome: InvocationOutcome| InvocationLogEntry { server: metadata.server.clone(), tool_name: metadata.tool_name.clone(), outcome };

    if let Err(e) = authorize(metadata, policy) {
        sink.record(log(InvocationOutcome::Denied));
        return Err(e);
    }

    let arguments = match args {
        serde_json::Value::Object(map) => Some(map),
        serde_json::Value::Null => None,
        other => {
            sink.record(log(InvocationOutcome::MalformedResponse));
            return Err(McpClientError::MalformedResponse(format!("tool call arguments must be a JSON object or null, got {other:?}")));
        }
    };

    let mut params = CallToolRequestParams::new(metadata.tool_name.clone());
    if let Some(arguments) = arguments {
        params = params.with_arguments(arguments);
    }

    let result = match tokio::time::timeout(timeout, conn.service().call_tool(params)).await {
        Err(_) => {
            sink.record(log(InvocationOutcome::Timeout));
            return Err(McpClientError::Timeout);
        }
        Ok(Err(e)) => {
            sink.record(log(InvocationOutcome::ConnectionFailed));
            return Err(McpClientError::ConnectionFailed(e.to_string()));
        }
        Ok(Ok(result)) => result,
    };

    match validate_result(metadata, result) {
        Ok(value) => {
            sink.record(log(InvocationOutcome::Success));
            Ok(value)
        }
        Err(e) => {
            sink.record(log(InvocationOutcome::MalformedResponse));
            Err(e)
        }
    }
}

/// Validates a `tools/call` reply before trusting it as the real result
/// of invoking `metadata.tool_name`, and extracts the one `serde_json::
/// Value` this function returns to its own caller.
///
/// **What this function can, and cannot, check.** The MCP `tools/call`
/// response (`rmcp::model::CallToolResult`) carries no tool-name field at
/// all — the protocol has no "this result is from tool X" envelope to
/// spoof in the first place (confirmed by reading `rmcp::model::
/// CallToolResult`'s own fields: `content`/`structured_content`/
/// `is_error`/`meta`, none of them a tool identifier). Request/response
/// CORRELATION — "this is genuinely the reply to the request I just
/// sent, not some other pending request's reply" — is `rmcp`'s own
/// responsibility (JSON-RPC `id` matching inside `RunningService::
/// call_tool`), already exercised end-to-end by this crate's own
/// `discovery.rs` tests against real external processes; this function
/// does not re-implement that layer and does not claim to.
///
/// What THIS function rejects, because it genuinely can observe it:
/// - `is_error: Some(true)` is NOT rejected here — a tool-level error is
///   a legitimate, successfully-delivered answer (the external tool ran
///   and reported failure), not a malformed envelope; it is returned to
///   the caller as `Ok(...)`, exactly as the real result it is. (A
///   caller wanting to treat tool-level errors specially inspects the
///   returned `Value`'s own `"is_error"`/`"content"` shape itself — this
///   function's job is transport/envelope trust, not business-outcome
///   interpretation.)
/// - `structured_content`, if present, that is an object carrying a
///   `"tool"`/`"tool_name"` field disagreeing with `metadata.tool_name`:
///   rejected as [`McpClientError::MalformedResponse`]. This is the one
///   SELF-DESCRIBED identity claim the real wire payload can actually
///   carry (an external tool choosing to echo its own name in its
///   structured output) — if it disagrees with what was invoked, that is
///   a real, observable inconsistency worth refusing, even though it is
///   not the only possible spoofing vector and is not claimed to be a
///   complete defense against a malicious server.
fn validate_result(metadata: &ExternalToolMetadata, result: rmcp::model::CallToolResult) -> Result<serde_json::Value, McpClientError> {
    if let Some(serde_json::Value::Object(map)) = &result.structured_content {
        for key in ["tool", "tool_name"] {
            if let Some(claimed) = map.get(key).and_then(|v| v.as_str()) {
                if claimed != metadata.tool_name {
                    return Err(McpClientError::MalformedResponse(format!("response's own {key:?} field claims tool {claimed:?}, but {:?} was invoked", metadata.tool_name)));
                }
            }
        }
    }

    serde_json::to_value(&result).map_err(|e| McpClientError::MalformedResponse(format!("failed to normalize tool result: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sensitivity, ToolClassification};

    fn metadata() -> ExternalToolMetadata {
        ExternalToolMetadata { server: "server-a".to_string(), tool_name: "echo".to_string(), classification: ToolClassification::Read, network_required: false, filesystem_scope: None, sensitivity: Sensitivity::Internal }
    }

    #[derive(Default)]
    struct RecordingSink(Vec<InvocationLogEntry>);
    impl InvocationLogSink for RecordingSink {
        fn record(&mut self, entry: InvocationLogEntry) {
            self.0.push(entry);
        }
    }

    // --- malformed response: wrong shape / spoofed tool identity ---
    // (pure logic, no process/network needed)

    #[test]
    fn a_result_whose_own_structured_content_claims_a_different_tool_name_is_rejected() {
        let result = rmcp::model::CallToolResult::structured(serde_json::json!({"tool_name": "not-echo", "value": 1}));
        let err = validate_result(&metadata(), result);
        assert!(matches!(err, Err(McpClientError::MalformedResponse(_))), "expected MalformedResponse, got {err:?}");
    }

    #[test]
    fn a_result_with_no_tool_identity_claim_at_all_validates_normally() {
        let result = rmcp::model::CallToolResult::structured(serde_json::json!({"value": 1}));
        let value = validate_result(&metadata(), result).expect("a result with no tool-identity claim must validate");
        assert_eq!(value["structuredContent"]["value"], 1);
    }

    #[test]
    fn a_result_whose_structured_content_names_the_same_tool_validates_normally() {
        let result = rmcp::model::CallToolResult::structured(serde_json::json!({"tool": "echo", "value": 1}));
        assert!(validate_result(&metadata(), result).is_ok());
    }

    #[test]
    fn a_tool_level_error_result_is_not_treated_as_a_malformed_envelope() {
        // is_error: true is a legitimate answer ("the tool ran and
        // failed"), never an envelope-trust problem -- returned as Ok,
        // not rejected.
        let result = rmcp::model::CallToolResult::error(vec![rmcp::model::ContentBlock::text("tool-level failure")]);
        assert!(validate_result(&metadata(), result).is_ok());
    }

    // --- real end-to-end tests, against real external MCP servers
    //     spawned as real child processes -- same style as
    //     discovery.rs's own end_to_end module, no mock framework ---

    mod end_to_end {
        use super::*;
        use crate::{McpServerRegistration, TransportConfig, connect};

        fn python_or_skip() -> Option<&'static str> {
            ["python", "python3"].into_iter().find(|candidate| std::process::Command::new(candidate).arg("--version").output().is_ok_and(|o| o.status.success()))
        }

        /// A real, minimal external MCP server whose `echo` tool actually
        /// answers `tools/call` by echoing its own arguments back as
        /// structured content.
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
            "serverInfo": {"name": "echo-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/call":
        params = req.get("params", {})
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "content": [],
            "structuredContent": {"echoed": params.get("arguments", {})},
        }})
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

        /// A real external server that completes the handshake but never
        /// answers `tools/call` at all -- a genuinely hung tool, not a
        /// simulated one.
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
            "serverInfo": {"name": "hanging-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/call":
        pass  # never answers; the real external tool just hangs
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

        async fn spawn(script: &str, name: &str) -> McpConnection {
            let python = python_or_skip().expect("no python3/python interpreter found on PATH -- required for this end-to-end test");
            let registration = McpServerRegistration { name: name.to_string(), transport: TransportConfig::ChildProcess { command: python.to_string(), args: vec!["-c".to_string(), script.to_string()] }, enabled: true };
            connect(&registration).await.expect("connect to the real test server must succeed")
        }

        fn skip_if_no_python() -> bool {
            if python_or_skip().is_none() {
                eprintln!("skipping: no python3/python interpreter found on PATH");
                true
            } else {
                false
            }
        }

        // --- authorization gates invocation; a denied tool never
        //     reaches the network ---

        #[tokio::test]
        async fn a_denied_tool_returns_permission_denied_without_ever_calling_the_real_tool() {
            if skip_if_no_python() {
                return;
            }
            let mut sink = RecordingSink::default();
            // Connect to a REAL, running echo server -- if authorize()
            // did not run first and short-circuit before the real
            // tools/call send, this would reach the server and succeed
            // (the server really does answer echo). Getting
            // PermissionDenied here, with no tools/call round trip
            // delay, is the proof denial happens before the network is
            // ever touched.
            let conn = spawn(ECHO_SERVER_SCRIPT, "echo-test-server").await;

            let result = invoke_external_tool_logged(&conn, &metadata(), &ApprovalPolicy::Deny, serde_json::json!({}), Duration::from_secs(5), &mut sink).await;

            assert_eq!(result, Err(McpClientError::PermissionDenied));
            assert_eq!(sink.0.len(), 1);
            assert_eq!(sink.0[0].outcome, InvocationOutcome::Denied);
            assert_eq!(sink.0[0].server, "server-a");
            assert_eq!(sink.0[0].tool_name, "echo");

            conn.close().await.unwrap();
        }

        #[tokio::test]
        async fn ask_approval_required_also_denies_without_ever_calling_the_real_tool() {
            if skip_if_no_python() {
                return;
            }
            let mut sink = RecordingSink::default();
            let conn = spawn(ECHO_SERVER_SCRIPT, "echo-test-server").await;

            let result = invoke_external_tool_logged(&conn, &metadata(), &ApprovalPolicy::AskApprovalRequired, serde_json::json!({}), Duration::from_secs(5), &mut sink).await;

            assert_eq!(result, Err(McpClientError::PermissionDenied));
            assert_eq!(sink.0[0].outcome, InvocationOutcome::Denied);

            conn.close().await.unwrap();
        }

        // --- timeout: a hung external tool produces Timeout, not a
        //     hang, and still logs ---

        #[tokio::test]
        async fn a_hung_real_tool_call_returns_timeout_and_still_logs_it() {
            if skip_if_no_python() {
                return;
            }
            let mut sink = RecordingSink::default();
            let conn = spawn(HANGING_TOOL_SERVER_SCRIPT, "hanging-test-server").await;
            let allow = ApprovalPolicy::AllowReadOnly;

            let result = invoke_external_tool_logged(&conn, &metadata(), &allow, serde_json::json!({}), Duration::from_millis(500), &mut sink).await;

            assert_eq!(result, Err(McpClientError::Timeout));
            assert_eq!(sink.0.len(), 1);
            assert_eq!(sink.0[0].outcome, InvocationOutcome::Timeout);

            conn.close().await.unwrap();
        }

        // --- malformed arguments rejected before any network call ---

        #[tokio::test]
        async fn non_object_non_null_arguments_are_rejected_as_malformed_before_any_network_call() {
            if skip_if_no_python() {
                return;
            }
            let mut sink = RecordingSink::default();
            let conn = spawn(ECHO_SERVER_SCRIPT, "echo-test-server").await;
            let allow = ApprovalPolicy::AllowReadOnly;

            let result = invoke_external_tool_logged(&conn, &metadata(), &allow, serde_json::json!("not an object"), Duration::from_secs(5), &mut sink).await;

            assert!(matches!(result, Err(McpClientError::MalformedResponse(_))), "expected MalformedResponse, got {result:?}");
            assert_eq!(sink.0[0].outcome, InvocationOutcome::MalformedResponse);

            conn.close().await.unwrap();
        }

        // --- success path, end to end against a real external process ---

        #[tokio::test]
        async fn a_well_behaved_real_server_under_an_allowing_policy_succeeds_and_logs_success() {
            if skip_if_no_python() {
                return;
            }
            let mut sink = RecordingSink::default();
            let conn = spawn(ECHO_SERVER_SCRIPT, "echo-test-server").await;
            let allow = ApprovalPolicy::AllowReadOnly;

            let result = invoke_external_tool_logged(&conn, &metadata(), &allow, serde_json::json!({"text": "hello"}), Duration::from_secs(5), &mut sink).await;

            let value = result.expect("expected success");
            assert_eq!(value["structuredContent"]["echoed"]["text"], "hello", "the real external tool's own real echoed reply must come through, got {value:?}");
            assert_eq!(sink.0[0].outcome, InvocationOutcome::Success);

            conn.close().await.unwrap();
        }

        // --- disconnection: calling a tool after the connection is
        //     already closed is a connection failure, never a silent/odd
        //     success ---

        #[tokio::test]
        async fn invoking_after_the_connection_is_closed_is_a_connection_failure_not_a_silent_success() {
            if skip_if_no_python() {
                return;
            }
            let mut sink = RecordingSink::default();
            let conn = spawn(ECHO_SERVER_SCRIPT, "echo-test-server").await;
            // Cancel the underlying session via its cancellation token
            // (which takes `&self`) rather than `McpConnection::close`
            // (which takes `self`), so this test can still invoke
            // through the same `&McpConnection` value afterward and
            // observe the now-dead connection's real failure mode.
            conn.service().cancellation_token().cancel();
            tokio::time::sleep(Duration::from_millis(100)).await;

            let allow = ApprovalPolicy::AllowReadOnly;
            let result = invoke_external_tool_logged(&conn, &metadata(), &allow, serde_json::json!({}), Duration::from_secs(5), &mut sink).await;

            assert!(result.is_err(), "invoking on a closed connection must be a typed error, got {result:?}");
            assert_ne!(result, Err(McpClientError::PermissionDenied));
        }
    }
}
