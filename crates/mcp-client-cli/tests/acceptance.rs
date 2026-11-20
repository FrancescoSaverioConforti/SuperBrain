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
        let python = python_or_skip().expect("checked by caller via skip_if_no_python");
        let server_dir = unique_test_dir(&format!("{label}-server"));
        let server_script_path = server_dir.join("mock_server.py");
        std::fs::write(&server_script_path, MOCK_SERVER_SCRIPT).unwrap();

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
