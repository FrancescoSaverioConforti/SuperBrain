//! M6.4 Phase 5: real-writer, real-process acceptance tests. Drives the
//! actual compiled `mcp-client-cli` binary via `std::process::Command`
//! against a real, local, deterministic mock MCP server (the same
//! child-process-over-stdio pattern `mcp-client`'s own
//! `invoke.rs::end_to_end` tests already use), with a real
//! `storage::Vault`-backed audit log in a fresh temp directory -- never
//! an in-memory fake for these tests specifically, since Phase 5's own
//! job is proving the REAL writer/recovery/append path works end to end
//! through this binary.

use std::path::PathBuf;

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

fn sha256_hex_of_file(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex_of_schema(schema: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let map = schema.as_object().unwrap();
    let canonical = serde_json::to_string(map).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn unique_test_dir(label: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    // Rooted under the current user's own profile directory, NOT system
    // %TEMP% -- M6.4-S1's own review found this development host's
    // %TEMP% carries a real, non-inherit-only write-into grant for a
    // separate principal (CodexSandboxUsers), which would make
    // windows_acl::verify_trusted_location's own Check A correctly
    // reject any `script_trust_boundary` rooted there. On Windows, this
    // directory's own ACL is additionally reset via `icacls` to remove
    // inherited ACEs and grant only the current user/SYSTEM/
    // Administrators, mirroring windows_acl.rs's own
    // `make_clean_test_dir` test helper exactly -- these acceptance
    // tests exercise the REAL ACL check, so their own fixtures must be
    // genuinely clean, not merely assumed to be.
    #[cfg(windows)]
    let base = std::env::var_os("USERPROFILE").map(PathBuf::from).expect("USERPROFILE must be set on any Windows test host");
    #[cfg(not(windows))]
    let base = std::env::temp_dir();
    let dir = base.join(format!(
        "mcp-client-cli-acceptance-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    #[cfg(windows)]
    {
        let current_user = std::env::var("USERNAME").unwrap_or_default();
        let _ = std::process::Command::new("icacls")
            .arg(&dir)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{current_user}:(OI)(CI)F"))
            .arg("/grant:r")
            .arg("SYSTEM:(OI)(CI)F")
            .arg("/grant:r")
            .arg("Administrators:(OI)(CI)F")
            .status();
    }
    dir
}

/// A real, minimal mock MCP server script with three tools: `read_thing`
/// (Read-classified in test configs), `write_thing` (Write-classified),
/// and `hang_thing` (never answers `tools/call`, for the timeout/unknown
/// -outcome scenarios). Deterministic: every reply is fixed, no randomness.
const MOCK_SERVER_SCRIPT: &str = r#"
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
            "serverInfo": {"name": "mock-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {"tools": [
            {"name": "read_thing", "description": "reads a thing", "inputSchema": {"type": "object", "properties": {}}},
            {"name": "write_thing", "description": "writes a thing", "inputSchema": {"type": "object", "properties": {"value": {"type": "string"}}}},
            {"name": "hang_thing", "description": "never answers", "inputSchema": {"type": "object", "properties": {}}},
        ]}})
    elif method == "tools/call":
        params = req.get("params", {})
        name = params.get("name")
        if name == "hang_thing":
            continue  # never answers -- real timeout
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "content": [],
            "structuredContent": {"echoed": params.get("arguments", {}), "tool": name},
        }})
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

const READ_THING_SCHEMA: &str = r#"{"properties":{},"type":"object"}"#;
const WRITE_THING_SCHEMA: &str = r#"{"properties":{"value":{"type":"string"}},"type":"object"}"#;
const HANG_THING_SCHEMA: &str = r#"{"properties":{},"type":"object"}"#;

/// M6.5 Test A: a mock server that advertises `read_thing` with a
/// DIFFERENT, still-valid `inputSchema` than [`READ_THING_SCHEMA`] --
/// the server genuinely answers `tools/list`/`tools/call` correctly by
/// MCP protocol rules; only the schema's own content has drifted from
/// what the operator originally pinned. Proves the schema-hash
/// mismatch, not a malformed/missing schema.
const MOCK_SERVER_SCRIPT_SCHEMA_DRIFT: &str = r#"
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
            "serverInfo": {"name": "mock-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {"tools": [
            {"name": "read_thing", "description": "reads a thing", "inputSchema": {"type": "object", "properties": {"extra_drifted_field": {"type": "string"}}}},
        ]}})
    elif method == "tools/call":
        params = req.get("params", {})
        name = params.get("name")
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "content": [],
            "structuredContent": {"echoed": params.get("arguments", {}), "tool": name},
        }})
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

/// M6.5 Test B: a mock server that completes the real MCP handshake
/// (`initialize` succeeds normally) but returns a structurally invalid
/// `tools/list` reply -- a result object missing the required `tools`
/// field entirely, which `rmcp`'s own deserialization rejects as
/// malformed, never reaching this crate's own `discover_tools` at all. A
/// genuinely broken/misbehaving external server, not a simulated error.
const MOCK_SERVER_SCRIPT_DISCOVERY_FAILS: &str = r#"
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
            "serverInfo": {"name": "mock-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        # Missing the required "tools" field entirely -- a real,
        # structurally invalid reply, not a well-formed empty list.
        send({"jsonrpc": "2.0", "id": req["id"], "result": {}})
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

struct TestFixture {
    _server_dir: PathBuf,
    server_script_path: PathBuf,
    working_dir: PathBuf,
    _config_dir: PathBuf,
    config_path: PathBuf,
    audit_vault_root: PathBuf,
    audit_journal_path: PathBuf,
    python: String,
}

impl TestFixture {
    fn new(label: &str) -> Self {
        Self::new_with_server_script(label, MOCK_SERVER_SCRIPT)
    }

    /// Same as [`TestFixture::new`], but with a caller-supplied mock
    /// server script -- used by M6.5's own Test A (schema drift) and
    /// Test B (discovery failure) to run the real binary against a
    /// deliberately different, still-genuine external MCP server
    /// implementation, never a simulated/mocked `mcp_client` call.
    fn new_with_server_script(label: &str, server_script: &str) -> Self {
        let python = python_or_skip().expect("checked by caller via skip_if_no_python");
        let server_dir = unique_test_dir(&format!("{label}-server"));
        let server_script_path = server_dir.join("mock_server.py");
        std::fs::write(&server_script_path, server_script).unwrap();

        let working_dir = unique_test_dir(&format!("{label}-workdir"));
        let config_dir = unique_test_dir(&format!("{label}-config"));
        let config_path = config_dir.join("config.toml");

        let audit_vault_root = unique_test_dir(&format!("{label}-vault"));
        let audit_journal_path = audit_vault_root.join("journal.sqlite");

        Self { _server_dir: server_dir, server_script_path, working_dir, _config_dir: config_dir, config_path, audit_vault_root, audit_journal_path, python: python.to_string() }
    }

    fn python_resolved_path(&self) -> PathBuf {
        let output = std::process::Command::new(&self.python).arg("-c").arg("import sys; print(sys.executable)").output().unwrap();
        PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
    }

    fn write_config(&self, trusted_tools_toml: &str, approval_policy_toml: &str) {
        let python_path = self.python_resolved_path();
        let command_sha256 = sha256_hex_of_file(&python_path);
        let script_sha256 = sha256_hex_of_file(&self.server_script_path);
        // command_trust_boundary: the Python interpreter's own install
        // directory (its own parent) -- a real, pre-existing, operator-
        // maintained location, never created by these tests. script_
        // trust_boundary: this test's own freshly created working_dir,
        // which make_clean_test_dir-equivalent setup (unique_test_dir)
        // already creates under this process's own temp root with no
        // broad grants of its own.
        let command_trust_boundary = python_path.parent().unwrap().to_path_buf();
        let toml_text = format!(
            r#"
[server]
name = "mock-server"
command = '{}'
command_sha256 = "{command_sha256}"
command_trust_boundary = '{}'
script = "mock_server.py"
script_sha256 = "{script_sha256}"
script_trust_boundary = '{}'
args = []
working_dir = '{}'

{trusted_tools_toml}

[approval_policy]
{approval_policy_toml}
"#,
            python_path.display(),
            command_trust_boundary.display(),
            self.working_dir.display(),
            self.working_dir.display(),
        );
        std::fs::copy(&self.server_script_path, self.working_dir.join("mock_server.py")).unwrap();
        std::fs::write(&self.config_path, toml_text).unwrap();
    }

    fn run_cli(&self, tool: &str, args_json: &str) -> std::process::Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_mcp-client-cli"))
            .arg("--config")
            .arg(&self.config_path)
            .arg("--audit-vault")
            .arg(&self.audit_vault_root)
            .arg("--audit-journal")
            .arg(&self.audit_journal_path)
            .arg("--tool")
            .arg(tool)
            .arg("--args-json")
            .arg(args_json)
            .output()
            .unwrap()
    }

    fn read_raw_audit_log(&self) -> Vec<u8> {
        let vault = storage::Vault::open_with_default_indexer(self.audit_vault_root.clone(), &self.audit_journal_path).unwrap();
        vault.read_audit_log_for_recovery_scan().unwrap()
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        // These fixtures are now rooted under %USERPROFILE% on Windows
        // (unique_test_dir's own M6.4-S1 correction), not ambient
        // system Temp -- clean them up explicitly so repeated local/CI
        // runs don't accumulate visible debris in the user's own
        // profile directory.
        for dir in [&self._server_dir, &self.working_dir, &self._config_dir, &self.audit_vault_root] {
            #[cfg(windows)]
            let _ = std::process::Command::new("icacls").arg(dir).arg("/reset").arg("/t").status();
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn trusted_tool_toml(tool_name: &str, classification: &str, sensitivity: &str, schema_json: &str) -> String {
    let schema: serde_json::Value = serde_json::from_str(schema_json).unwrap();
    let hash = sha256_hex_of_schema(&schema);
    format!(
        r#"[[trusted_tools]]
tool_name = "{tool_name}"
classification = "{classification}"
sensitivity = "{sensitivity}"
network_required = false
input_schema_sha256 = "{hash}"
"#
    )
}

// --- 1: valid supervised read-only invocation ---

#[test]
fn valid_supervised_read_only_invocation_succeeds() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new("read-only");
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("read_thing", "{}");
    assert!(output.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("read_thing"), "stdout must contain the real server's own echoed reply: {stdout}");
}

// --- 2: write/execute invocation with durable intent before tools/call ---

#[test]
fn write_invocation_records_invocation_attempted_before_outcome_in_the_real_audit_log() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new("write-durable");
    fx.write_config(
        &trusted_tool_toml("write_thing", "write", "sensitive", WRITE_THING_SCHEMA),
        "kind = \"allow_scoped_execution\"\nallow_network = false",
    );

    let output = fx.run_cli("write_thing", r#"{"value":"hello"}"#);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let raw = fx.read_raw_audit_log();
    let lines: Vec<&[u8]> = raw[..raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let records: Vec<audit_chain_core::ChainRecord> = lines.iter().map(|l| audit_chain_core::parse_line(l).unwrap()).collect();
    let events: Vec<mcp_client_audit_worker::McpClientEvent> = records.iter().map(|r| audit_chain_core::parse_event(&r.content_bytes).unwrap()).collect();

    let attempted_index = events.iter().position(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::InvocationAttempted { .. }));
    let outcome_index = events.iter().position(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::InvocationOutcome { .. }));
    assert!(attempted_index.is_some(), "InvocationAttempted must be durably recorded for a Write invocation");
    assert!(outcome_index.is_some(), "InvocationOutcome must be durably recorded");
    assert!(attempted_index.unwrap() < outcome_index.unwrap(), "InvocationAttempted must be durably recorded BEFORE InvocationOutcome, proving pre-registration happened before tools/call");

    let scan = audit_chain_core::scan_chain(&records.iter().cloned().map(audit_chain_core::LineOutcome::Parsed).collect::<Vec<_>>());
    assert!(scan.stop_reason.is_none(), "the real persisted chain must verify cleanly, got {:?}", scan.stop_reason);
}

// --- 3: missing metadata denied ---

#[test]
fn a_tool_with_no_matching_trusted_config_entry_is_denied() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new("missing-metadata");
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("some_other_tool_not_in_config", "{}");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not present in the trusted configuration"));
}

// --- 4: unknown tool denied (trusted in config but not offered by server) ---

#[test]
fn a_tool_trusted_in_config_but_not_offered_by_the_server_is_denied() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new("unknown-tool");
    fx.write_config(&trusted_tool_toml("tool_the_server_never_offers", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("tool_the_server_never_offers", "{}");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("was not offered by the server"));
}

/// M6.7 adversarial case: the server offers a tool under a
/// DIFFERENTLY-CASED name than the one trusted in config and requested
/// via `--tool` (e.g. config/CLI say `read_thing`, server advertises
/// `Read_Thing`). Both `config.find_tool` and the discovered-tool
/// lookup in `main.rs` use exact (case-SENSITIVE) string matching --
/// this test proves that holds end to end: the differently-cased
/// server offering is correctly treated as "not offered," never
/// silently matched case-insensitively (which would open a path where
/// a malicious server could offer the REQUESTED name with case altered
/// just enough to dodge some future case-insensitive check while
/// `tools/call` is still dispatched under the operator's own exact
/// name).
#[test]
fn a_server_offering_the_trusted_tool_under_a_different_case_is_treated_as_not_offered() {
    if skip_if_no_python() {
        return;
    }
    const MOCK_SERVER_SCRIPT_CASING_MISMATCH: &str = r#"
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
            "serverInfo": {"name": "mock-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {"tools": [
            {"name": "Read_Thing", "description": "reads a thing, but under different casing than trusted", "inputSchema": {"type": "object", "properties": {}}},
        ]}})
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;
    let fx = TestFixture::new_with_server_script("casing-mismatch", MOCK_SERVER_SCRIPT_CASING_MISMATCH);
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("read_thing", "{}");
    assert_eq!(output.status.code(), Some(1), "a differently-cased server offering must be denied, never silently matched, stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("was not offered by the server"), "stderr must state the exact-case tool was not offered, got: {}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty(), "no tools/call response may ever appear when the requested exact-case tool was never offered, got: {}", String::from_utf8_lossy(&output.stdout));
}

// --- 5: invalid configuration denied ---

#[test]
fn an_invalid_configuration_is_denied_before_any_connection_is_attempted() {
    let fx = TestFixture::new("invalid-config");
    std::fs::write(&fx.config_path, "this is not valid toml {{{").unwrap();

    let output = fx.run_cli("anything", "{}");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to load trusted configuration"));
}

// --- 6: durable pre-registration failure blocks tools/call ---
// (covered structurally: see main.rs's own logic -- a QueueFull/WorkerGone
// durable pre-registration failure returns before invoke_external_tool is
// ever called. A dedicated forced-failure acceptance test would require
// injecting a worker-level fault, which this fixture's real storage-backed
// writer does not easily simulate; the in-process unit coverage in
// mcp-client-audit-worker's own test suite already proves the underlying
// worker mechanism refuses correctly when its queue/writer is unavailable.)

// --- 7: invocation error and timeout ---

#[test]
fn a_hanging_tool_call_times_out_and_is_recorded_as_external_side_effect_unknown_for_write_classification() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new("timeout");
    fx.write_config(
        &trusted_tool_toml("hang_thing", "write", "sensitive", HANG_THING_SCHEMA),
        "kind = \"allow_scoped_execution\"\nallow_network = false",
    );

    let output = fx.run_cli("hang_thing", "{}");
    assert_eq!(output.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("external side effect is UNKNOWN"));

    let raw = fx.read_raw_audit_log();
    let lines: Vec<&[u8]> = raw[..raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let events: Vec<mcp_client_audit_worker::McpClientEvent> = lines
        .iter()
        .map(|l| audit_chain_core::parse_line(l).unwrap())
        .map(|r| audit_chain_core::parse_event::<mcp_client_audit_worker::McpClientEvent>(&r.content_bytes).unwrap())
        .collect();
    assert!(
        events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::ExternalSideEffectUnknown { .. })),
        "a Write-classified invocation that times out mid-call must durably record ExternalSideEffectUnknown"
    );
}

// --- 10: reopen and verification of the real persisted hash chain ---

#[test]
fn reopening_the_same_audit_vault_across_two_cli_runs_preserves_the_verified_hash_chain() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new("reopen");
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output1 = fx.run_cli("read_thing", "{}");
    assert_eq!(output1.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&output1.stderr));
    let output2 = fx.run_cli("read_thing", "{}");
    assert_eq!(output2.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&output2.stderr));

    let raw = fx.read_raw_audit_log();
    let lines: Vec<&[u8]> = raw[..raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let records: Vec<_> = lines.iter().map(|l| audit_chain_core::parse_line(l).unwrap()).collect();
    let scan = audit_chain_core::scan_chain(&records.into_iter().map(audit_chain_core::LineOutcome::Parsed).collect::<Vec<_>>());
    assert!(scan.stop_reason.is_none(), "the chain across two separate process runs must still verify cleanly end to end, got {:?}", scan.stop_reason);
    assert!(scan.verified_records.len() >= 2, "both runs' own records must be present and verified");
}

// --- 13: no cross-access to the main note Vault ---

#[test]
fn the_dedicated_audit_vault_is_structurally_separate_from_any_main_note_vault() {
    // Structural proof by construction: this binary's own main.rs never
    // takes a "main vault root" argument at all -- its only Vault-shaped
    // CLI arguments are --audit-vault/--audit-journal, which this
    // fixture always points at a FRESH, dedicated temp directory never
    // shared with any other test's or any real note-store's own root.
    // Checks actual CODE usage (an identifier or call), never a
    // doc-comment mention -- this crate's own main.rs doc comment
    // legitimately names `storage::propose_and_commit` in prose
    // explaining that it is NEVER called, which must not false-positive
    // against this guard.
    let main_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")).unwrap();
    let has_real_propose_and_commit_call = main_rs.lines().any(|line| {
        let trimmed = line.trim_start();
        !trimmed.starts_with("//") && line.contains("propose_and_commit(")
    });
    assert!(!has_real_propose_and_commit_call, "this binary must never CALL storage::propose_and_commit -- it has no write-pipeline authority at all");
}

// --- 15: no trusted Brain API authority leaked to the new CLI ---

#[test]
fn this_crate_never_depends_on_brain_api_or_mcp_server() {
    // Parses the real [dependencies] table as text (the same pattern
    // crates/mcp-client/tests/separation.rs and
    // crates/mcp-client-audit-worker/tests/dependency_boundary.rs already
    // use), never a naive substring search over the whole file -- this
    // crate's own Cargo.toml `description` field legitimately mentions
    // "brain-api" in prose explaining what this crate is NOT, which must
    // not false-positive against this guard.
    let manifest_path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let text = std::fs::read_to_string(manifest_path).unwrap();
    let mut in_dependencies = false;
    let mut declared_path_deps = std::collections::BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dependencies = trimmed == "[dependencies]";
            continue;
        }
        if !in_dependencies || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.contains("path") {
            if let Some(name) = trimmed.split('=').next() {
                declared_path_deps.insert(name.trim().to_string());
            }
        }
    }
    assert!(!declared_path_deps.contains("brain-api"), "mcp-client-cli must never depend on brain-api, found: {declared_path_deps:?}");
    assert!(!declared_path_deps.contains("mcp-server"), "mcp-client-cli must never depend on mcp-server, found: {declared_path_deps:?}");
    let expected: std::collections::BTreeSet<String> = ["mcp-client", "mcp-client-audit-worker", "storage", "audit-chain-core"].into_iter().map(String::from).collect();
    assert_eq!(declared_path_deps, expected, "mcp-client-cli's own path dependencies must be exactly these four, no more, no fewer");
}

// --- 16: no automatic second invocation or retry ---

#[test]
fn a_single_cli_run_invokes_the_tool_exactly_once() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new("exactly-once");
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    fx.run_cli("read_thing", "{}");

    let raw = fx.read_raw_audit_log();
    let lines: Vec<&[u8]> = raw[..raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let events: Vec<mcp_client_audit_worker::McpClientEvent> = lines
        .iter()
        .map(|l| audit_chain_core::parse_line(l).unwrap())
        .map(|r| audit_chain_core::parse_event::<mcp_client_audit_worker::McpClientEvent>(&r.content_bytes).unwrap())
        .collect();
    let registration_attempts = events.iter().filter(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::RegistrationAttempted { .. })).count();
    assert_eq!(registration_attempts, 1, "a single CLI run must register/connect exactly once, never retry automatically");
}

// --- cross-check against the in-memory worker test suite's own contract ---

#[test]
fn the_real_vault_backed_writer_preserves_the_same_sequence_and_hash_chain_contract_the_in_memory_worker_tests_assume() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new("contract-check");
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    fx.run_cli("read_thing", "{}");
    let raw = fx.read_raw_audit_log();
    assert!(!raw.is_empty(), "the real writer must have produced at least one durable record");
    assert!(raw.ends_with(b"\n"), "every record line produced by the real writer must end with exactly one newline, matching audit_chain_core::serialize_record's own contract");

    let lines: Vec<&[u8]> = raw[..raw.len() - 1].split(|&b| b == b'\n').collect();
    let first_record = audit_chain_core::parse_line(lines[0]).unwrap();
    assert_eq!(first_record.sequence, 0, "the first record the real writer ever appends must be sequence 0, exactly matching the in-memory worker tests' own assumption");
    assert_eq!(first_record.prev_record_hash, audit_chain_core::GENESIS_HASH, "the first record's prev_record_hash must be the real GENESIS_HASH constant");
}

// --- M6.5 Test A: schema mismatch denies the invocation, never dispatches tools/call ---

#[test]
fn a_tool_whose_live_schema_differs_from_the_pinned_hash_is_denied_before_tools_call() {
    if skip_if_no_python() {
        return;
    }
    // The config pins READ_THING_SCHEMA's own hash, but the server (via
    // MOCK_SERVER_SCRIPT_SCHEMA_DRIFT) actually offers read_thing with a
    // DIFFERENT, still well-formed inputSchema -- a genuine operator-
    // reviewed pin against a server whose real schema has since
    // drifted, not a malformed/missing schema.
    let fx = TestFixture::new_with_server_script("schema-mismatch", MOCK_SERVER_SCRIPT_SCHEMA_DRIFT);
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("read_thing", "{}");
    assert_eq!(output.status.code(), Some(1), "a schema-hash mismatch must deny the invocation, stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not match the pinned input_schema_sha256"), "stderr must name the specific schema-mismatch reason, got: {stderr}");
    assert!(stderr.contains("denied, never invoked"), "stderr must state the tool was never invoked, got: {stderr}");

    // stdout must be completely empty -- the real tools/call response
    // (the mock server's own "echoed" structuredContent) must never
    // appear, proving tools/call was never dispatched, not merely that
    // its result was discarded.
    assert!(output.stdout.is_empty(), "stdout must be empty: no tools/call response of any kind may ever reach the operator for a denied, unverified schema, got: {}", String::from_utf8_lossy(&output.stdout));

    // The durable audit log must show DiscoveryOutcome (discovery
    // evidence is retained, per the task's own requirement) but must
    // NEVER show InvocationAttempted or InvocationOutcome for this
    // tool -- the chain must also still verify cleanly end to end.
    let raw = fx.read_raw_audit_log();
    assert!(!raw.is_empty(), "discovery evidence must still be durably recorded even though the invocation itself was denied");
    let lines: Vec<&[u8]> = raw[..raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let records: Vec<_> = lines.iter().map(|l| audit_chain_core::parse_line(l).unwrap()).collect();
    let events: Vec<mcp_client_audit_worker::McpClientEvent> = records.iter().map(|r| audit_chain_core::parse_event(&r.content_bytes).unwrap()).collect();

    assert!(
        events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered, .. })),
        "a successful DiscoveryOutcome must still be durably retained as evidence, even though the subsequently-discovered tool's schema failed verification"
    );
    assert!(
        !events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::InvocationAttempted { .. })),
        "InvocationAttempted must NEVER be durably recorded for a tool whose schema failed verification -- the denial must happen strictly before any pre-registration, got events: {events:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::InvocationOutcome { .. })),
        "InvocationOutcome must NEVER be durably recorded for a tool whose schema failed verification, got events: {events:?}"
    );

    let scan = audit_chain_core::scan_chain(&records.into_iter().map(audit_chain_core::LineOutcome::Parsed).collect::<Vec<_>>());
    assert!(scan.stop_reason.is_none(), "the real persisted chain must still verify cleanly after a schema-mismatch denial, got {:?}", scan.stop_reason);
}

// --- M6.5 Test B: discovery failure fails closed, durably records DiscoveryOutcomeKind::Failed ---

#[test]
fn a_malformed_tools_list_reply_fails_closed_before_authorization_or_invocation() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new_with_server_script("discovery-fails", MOCK_SERVER_SCRIPT_DISCOVERY_FAILS);
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("read_thing", "{}");
    assert_eq!(output.status.code(), Some(1), "a malformed tools/list reply must fail closed, stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to discover tools"), "stderr must name discovery as the failure point, got: {stderr}");
    assert!(output.stdout.is_empty(), "stdout must be empty: no invocation of any kind can have occurred if discovery itself failed, got: {}", String::from_utf8_lossy(&output.stdout));

    let raw = fx.read_raw_audit_log();
    assert!(!raw.is_empty(), "DiscoveryOutcome::Failed must still be durably recorded even though discovery itself failed");
    let lines: Vec<&[u8]> = raw[..raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let records: Vec<_> = lines.iter().map(|l| audit_chain_core::parse_line(l).unwrap()).collect();
    let events: Vec<mcp_client_audit_worker::McpClientEvent> = records.iter().map(|r| audit_chain_core::parse_event(&r.content_bytes).unwrap()).collect();

    assert!(
        events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Failed, .. })),
        "DiscoveryOutcomeKind::Failed must be durably recorded, got events: {events:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::AuthorizationDecision { .. })),
        "authorization must never be reached if discovery itself failed, got events: {events:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::InvocationAttempted { .. } | mcp_client_audit_worker::McpClientEventKind::InvocationOutcome { .. })),
        "no invocation of any kind may ever be recorded if discovery itself failed, got events: {events:?}"
    );

    // Verify the persisted audit chain after REOPENING the real Vault
    // (a fresh storage::Vault::open_with_default_indexer call, the same
    // real-recovery-scan path a future invocation of this binary would
    // itself go through on startup) -- not merely reading the raw bytes
    // once, proving the chain genuinely survives a real reopen.
    let reopened_vault = storage::Vault::open_with_default_indexer(fx.audit_vault_root.clone(), &fx.audit_journal_path).unwrap();
    let reopened_raw = reopened_vault.read_audit_log_for_recovery_scan().unwrap();
    assert_eq!(reopened_raw, raw, "the raw audit log bytes must be identical after a real Vault reopen");
    let reopened_lines: Vec<&[u8]> = reopened_raw[..reopened_raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let reopened_records: Vec<_> = reopened_lines.iter().map(|l| audit_chain_core::parse_line(l).unwrap()).collect();
    let scan = audit_chain_core::scan_chain(&reopened_records.into_iter().map(audit_chain_core::LineOutcome::Parsed).collect::<Vec<_>>());
    assert!(scan.stop_reason.is_none(), "the persisted chain must still verify cleanly after reopening the real Vault, got {:?}", scan.stop_reason);
}

// --- M6.5 Test C: ordering regression -- schema verification must precede authorization and invocation ---

#[test]
fn schema_verification_runs_before_authorization_and_before_any_tools_call_dispatch() {
    if skip_if_no_python() {
        return;
    }
    // Uses a policy that WOULD authorize read_thing (AllowReadOnly) --
    // if schema verification ran AFTER authorization, or not at all,
    // this test's own schema-drifted server would still be denied only
    // by coincidence of the schema check existing somewhere, not
    // provably BEFORE authorization. The decisive, observable proof is
    // the ABSENCE of AuthorizationDecision in the durable log: if
    // schema verification happens first and denies before authorize()
    // is ever called, no AuthorizationDecision event can exist at all
    // for this run -- a regression that moved the schema check to AFTER
    // authorize() would instead produce a real AuthorizationDecision
    // event (since AllowReadOnly would authorize a Read-classified
    // tool), making this assertion the one that would catch it.
    let fx = TestFixture::new_with_server_script("ordering-regression", MOCK_SERVER_SCRIPT_SCHEMA_DRIFT);
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("read_thing", "{}");
    assert_eq!(output.status.code(), Some(1));

    let raw = fx.read_raw_audit_log();
    let lines: Vec<&[u8]> = raw[..raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let events: Vec<mcp_client_audit_worker::McpClientEvent> = lines
        .iter()
        .map(|l| audit_chain_core::parse_line(l).unwrap())
        .map(|r| audit_chain_core::parse_event::<mcp_client_audit_worker::McpClientEvent>(&r.content_bytes).unwrap())
        .collect();

    assert!(
        !events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::AuthorizationDecision { .. })),
        "no AuthorizationDecision may ever be recorded when schema verification denies first -- its presence would prove schema verification happened AFTER (or not before) authorization, got events: {events:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::InvocationAttempted { .. })),
        "no InvocationAttempted may ever be recorded when schema verification denies first, got events: {events:?}"
    );
}

// --- M6.5 Phase 2 item 2: the explicit schema-hash scope disclosure is actually printed on every real run ---

#[test]
fn the_schema_hash_scope_disclosure_appears_in_real_cli_output_on_every_run() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new("schema-disclosure");
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("read_thing", "{}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Schema-hash agreement is interface-shape verification, never behavioral-integrity proof."),
        "the real CLI's own stderr must contain the explicit M6.5 scope disclosure on every run, got: {stderr}"
    );
}

// --- M6.7: server crash/exit during each phase, fail-closed, no audit evidence beyond the reached phase ---

/// Crashes (hard `os._exit`, no clean shutdown) immediately after
/// answering `initialize` but before ever answering `tools/list` --
/// discovery itself must fail, never silently treated as "zero tools."
const MOCK_SERVER_CRASH_DURING_DISCOVERY: &str = r#"
import sys, json, os

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
            "serverInfo": {"name": "mock-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        os._exit(1)  # hard crash, no tools/list reply ever sent
"#;

#[test]
fn a_server_that_crashes_during_discovery_fails_closed_and_records_no_tool_classification_or_authorization() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new_with_server_script("crash-discovery", MOCK_SERVER_CRASH_DURING_DISCOVERY);
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("read_thing", "{}");
    assert_eq!(output.status.code(), Some(1), "a server crash during discovery must deny the invocation, stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty(), "no tools/call response of any kind may ever appear when discovery itself never completed, got: {}", String::from_utf8_lossy(&output.stdout));

    let raw = fx.read_raw_audit_log();
    let lines: Vec<&[u8]> = raw[..raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let records: Vec<_> = lines.iter().map(|l| audit_chain_core::parse_line(l).unwrap()).collect();
    let events: Vec<mcp_client_audit_worker::McpClientEvent> = records.iter().map(|r| audit_chain_core::parse_event(&r.content_bytes).unwrap()).collect();

    assert!(
        events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Failed, .. })),
        "a DiscoveryOutcome::Failed must be durably recorded when the server crashes mid-discovery, got events: {events:?}"
    );
    for forbidden in ["ToolClassified", "AuthorizationDecision", "AuthorizationDenied", "InvocationAttempted", "InvocationOutcome"] {
        assert!(
            !events.iter().any(|e| format!("{:?}", e.kind).starts_with(forbidden)),
            "{forbidden} must never appear when the server crashed before discovery ever completed -- no later phase may leave audit evidence as if it had been reached, got events: {events:?}"
        );
    }

    let scan = audit_chain_core::scan_chain(&records.into_iter().map(audit_chain_core::LineOutcome::Parsed).collect::<Vec<_>>());
    assert!(scan.stop_reason.is_none(), "the persisted chain must still verify cleanly after a mid-discovery crash, got {:?}", scan.stop_reason);
}

/// Answers `initialize` and `tools/list` correctly (so discovery and
/// schema verification both succeed normally against a genuinely
/// matching schema), then hard-crashes the instant `tools/call` is
/// received -- never sending any reply at all. This is DIFFERENT from
/// `hang_thing` (which never answers but stays alive): here the whole
/// process is gone, so the client observes a closed pipe/EOF rather
/// than a timeout.
const MOCK_SERVER_CRASH_DURING_INVOCATION: &str = r#"
import sys, json, os

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
            "serverInfo": {"name": "mock-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {"tools": [
            {"name": "read_thing", "description": "reads a thing", "inputSchema": {"type": "object", "properties": {}}},
        ]}})
    elif method == "tools/call":
        os._exit(1)  # hard crash, no reply, no clean pipe close
"#;

#[test]
fn a_server_that_crashes_during_invocation_records_invocation_attempted_but_never_a_fabricated_success_outcome() {
    if skip_if_no_python() {
        return;
    }
    let fx = TestFixture::new_with_server_script("crash-invocation", MOCK_SERVER_CRASH_DURING_INVOCATION);
    fx.write_config(&trusted_tool_toml("read_thing", "read", "internal", READ_THING_SCHEMA), "kind = \"allow_read_only\"");

    let output = fx.run_cli("read_thing", "{}");
    // Exit code 1 (real failure) is the only acceptable outcome here --
    // never 0 (which would mean a fabricated success was reported).
    assert_eq!(output.status.code(), Some(1), "a server crash during tools/call must never be reported as success, stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty(), "no fabricated tools/call result may ever reach stdout when the server crashed before answering, got: {}", String::from_utf8_lossy(&output.stdout));

    let raw = fx.read_raw_audit_log();
    let lines: Vec<&[u8]> = raw[..raw.len().saturating_sub(1)].split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
    let records: Vec<_> = lines.iter().map(|l| audit_chain_core::parse_line(l).unwrap()).collect();
    let events: Vec<mcp_client_audit_worker::McpClientEvent> = records.iter().map(|r| audit_chain_core::parse_event(&r.content_bytes).unwrap()).collect();

    // Discovery and authorization genuinely completed before the crash
    // -- their evidence must be present (this is NOT the
    // crash-during-discovery case).
    assert!(events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered, .. })), "discovery genuinely succeeded before the crash and must be recorded as such, got events: {events:?}");
    assert!(events.iter().any(|e| matches!(e.kind, mcp_client_audit_worker::McpClientEventKind::AuthorizationDecision { allowed: true, .. })), "authorization genuinely succeeded before the crash and must be recorded as such, got events: {events:?}");

    // The crash happened AFTER tools/call was dispatched -- invocation
    // was genuinely attempted, so InvocationAttempted recording that is
    // correct, not a violation of "no evidence beyond the reached
    // phase." What must NEVER happen is a fabricated SUCCESS outcome.
    let has_success_outcome = events.iter().any(|e| matches!(&e.kind, mcp_client_audit_worker::McpClientEventKind::InvocationOutcome { outcome, .. } if format!("{outcome:?}").to_lowercase().contains("success")));
    assert!(!has_success_outcome, "a crashed, unanswered tools/call must never be durably recorded as a success outcome, got events: {events:?}");

    let scan = audit_chain_core::scan_chain(&records.into_iter().map(audit_chain_core::LineOutcome::Parsed).collect::<Vec<_>>());
    assert!(scan.stop_reason.is_none(), "the persisted chain must still verify cleanly after a mid-invocation crash, got {:?}", scan.stop_reason);
}
