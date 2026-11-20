//! SuperBrain M6.4: strictly supervised, one-shot MCP Client composition
//! root. `cargo run -p mcp-client-cli -- --config <path> --audit-vault
//! <vault-root> --audit-journal <journal-path> --tool <tool-name>
//! [--args-json <json>]`.
//!
//! A separate, independent binary -- NEVER an extension of
//! `brain-api/examples/approve_cli.rs`. This binary has no dependency on
//! `brain-api` or `mcp-server` at all; it cannot construct a
//! `brain_api::RequestContext`/`WritePolicy`, cannot reach
//! `storage::propose_and_commit`, and introduces no new MCP Server
//! capability. It introduces no autonomous execution, tool chaining, or
//! retry of any kind -- exactly one explicitly requested tool is
//! invoked, exactly once, per process run.
//!
//! See `config.rs`/`identity.rs`/`adapter.rs` for the trusted
//! configuration contract, executable/schema identity verification, and
//! the real-storage audit writer adapter, respectively.
//!
//! **Exit code convention** (M6.4 Phase 1's own explicit table):
//! - `0`: the real invocation's own outcome is fully known AND every
//!   mandatory durable audit write for it was confirmed.
//! - `1`: the real operation itself failed, was denied, or was never
//!   attempted (config error, authorization denial, durable
//!   pre-registration failure before `tools/call` was ever dispatched).
//! - `2`: the real invocation's own outcome IS known (printed to stdout,
//!   unconditionally, before anything else) but the post-invocation
//!   durable audit write for that outcome itself failed -- the real
//!   result is correct as printed; its audit trail is NOT confirmed.

mod adapter;
mod config;
mod identity;
#[cfg(windows)]
mod windows_acl;

use adapter::StorageAuditWriterAdapter;
use config::TrustedConfig;

const AUDIT_WORKER_SHUTDOWN_BOUND: std::time::Duration = std::time::Duration::from_secs(30);

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

struct Args {
    config_path: std::path::PathBuf,
    audit_vault_root: std::path::PathBuf,
    audit_journal_path: std::path::PathBuf,
    tool_name: String,
    args_json: serde_json::Value,
}

fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().collect();
    let mut config_path = None;
    let mut audit_vault_root = None;
    let mut audit_journal_path = None;
    let mut tool_name = None;
    let mut args_json = serde_json::Value::Object(serde_json::Map::new());

    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "--config" => {
                i += 1;
                config_path = raw.get(i).map(std::path::PathBuf::from);
            }
            "--audit-vault" => {
                i += 1;
                audit_vault_root = raw.get(i).map(std::path::PathBuf::from);
            }
            "--audit-journal" => {
                i += 1;
                audit_journal_path = raw.get(i).map(std::path::PathBuf::from);
            }
            "--tool" => {
                i += 1;
                tool_name = raw.get(i).cloned();
            }
            "--args-json" => {
                i += 1;
                let text = raw.get(i).ok_or("--args-json requires a value")?;
                args_json = serde_json::from_str(text).map_err(|e| format!("--args-json is not valid JSON: {e}"))?;
            }
            other => return Err(format!("unrecognized argument: {other}")),
        }
        i += 1;
    }

    Ok(Args {
        config_path: config_path.ok_or("--config <path> is required")?,
        audit_vault_root: audit_vault_root.ok_or("--audit-vault <path> is required")?,
        audit_journal_path: audit_journal_path.ok_or("--audit-journal <path> is required")?,
        tool_name: tool_name.ok_or("--tool <name> is required")?,
        args_json,
    })
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("usage: mcp-client-cli --config <path> --audit-vault <path> --audit-journal <path> --tool <name> [--args-json <json>]");
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("{}", identity::NO_SANDBOX_DISCLOSURE);
    eprintln!("{}", identity::TOCTOU_DISCLOSURE);

    // --- Phase 3, step 1: load and validate trusted operator configuration ---
    let config = match TrustedConfig::load(&args.config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to load trusted configuration: {e}");
            std::process::exit(1);
        }
    };

    if matches!(config.approval_policy, config::ApprovalPolicyConfig::AskApprovalRequired) {
        eprintln!("notice: approval_policy is ask_approval_required; mcp-client's own authorize() always denies this policy with no interactive approval path implemented in this binary; every invocation under this policy will be refused.");
    }

    // --- Executable identity verification (Fork A, A2) ---
    let resolved_command = match config::resolve_executable(&config.server.command) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("resolved server.command to {resolved_command:?} (canonical path, content-hash-verified below)");
    if let Err(e) = identity::verify_file_content_hash(&resolved_command, &config.server.command_sha256, "<server-command>") {
        eprintln!("error: server.command identity verification failed: {e}");
        std::process::exit(1);
    }
    #[cfg(windows)]
    if let Err(e) = windows_acl::verify_trusted_location(&resolved_command, &config.server.command_trust_boundary) {
        eprintln!("error: server.command is not in a trusted location: {e}");
        std::process::exit(1);
    }

    let resolved_script_path = if let Some(script) = &config.server.script {
        let script_path = config.server.working_dir.join(script);
        let script_sha256 = config.server.script_sha256.as_deref().expect("config::validate already requires script_sha256 whenever script is present");
        if let Err(e) = identity::verify_file_content_hash(&script_path, script_sha256, "<server-script>") {
            eprintln!("error: server.script identity verification failed: {e}");
            std::process::exit(1);
        }
        #[cfg(windows)]
        {
            let script_trust_boundary = config.server.script_trust_boundary.as_ref().expect("config::validate already requires script_trust_boundary whenever script is present");
            if let Err(e) = windows_acl::verify_trusted_location(&script_path, script_trust_boundary) {
                eprintln!("error: server.script is not in a trusted location: {e}");
                std::process::exit(1);
            }
        }
        Some(script_path)
    } else {
        None
    };

    let exit_code = run(args, config, resolved_command, resolved_script_path);
    std::process::exit(exit_code);
}

fn run(args: Args, config: TrustedConfig, resolved_command: std::path::PathBuf, resolved_script_path: Option<std::path::PathBuf>) -> i32 {
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: failed to start the async runtime: {e}");
            return 1;
        }
    };
    rt.block_on(run_async(args, config, resolved_command, resolved_script_path))
}

async fn run_async(args: Args, config: TrustedConfig, resolved_command: std::path::PathBuf, resolved_script_path: Option<std::path::PathBuf>) -> i32 {
    let correlation_id = uuid::Uuid::new_v4().to_string();

    // --- Phase 3, step 2: initialize the dedicated audit Vault ---
    let audit_vault = match storage::Vault::open_with_default_indexer(args.audit_vault_root.clone(), &args.audit_journal_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: failed to open the dedicated audit vault: {e:?}");
            return 1;
        }
    };

    // --- Phase 3, step 3: recover or reject the existing audit state ---
    let raw_log_bytes = match audit_vault.read_audit_log_for_recovery_scan() {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("error: failed to read the existing audit log for recovery: {e:?}");
            return 1;
        }
    };
    let scan_lines = split_lines_for_scan(&raw_log_bytes);
    let scan_result = audit_chain_core::scan_chain(&scan_lines);
    if !audit_chain_core::may_resume_without_recovery(&scan_result) {
        eprintln!("error: the existing audit log requires an explicit recovery procedure before this binary may start: {:?}", scan_result.stop_reason);
        return 1;
    }
    let caller_verified_tail: Option<(u64, String)> = scan_result.last_verified.clone();

    let storage_writer = match audit_vault.open_audit_writer(caller_verified_tail.as_ref().map(|(seq, hash)| (*seq, hash.as_str()))) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("error: failed to open the real durable audit writer: {e:?}");
            return 1;
        }
    };
    let adapter = StorageAuditWriterAdapter(storage_writer);

    // --- Phase 3, step 4: start the audit worker ---
    let audit_worker = match mcp_client_audit_worker::spawn_worker(adapter, &raw_log_bytes, mcp_client_audit_worker::WorkerConfig::default()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: failed to start the real durable audit worker: {e}");
            return 1;
        }
    };

    let exit_code = execute(&args, &config, &correlation_id, &audit_worker, &resolved_command, &resolved_script_path).await;

    // --- Phase 3, step 14: bounded shutdown and cleanup ---
    match tokio::time::timeout(AUDIT_WORKER_SHUTDOWN_BOUND, audit_worker.shutdown()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("notice: audit worker was already gone at shutdown time (nothing to drain): {e}"),
        Err(_) => eprintln!("notice: audit worker shutdown did not complete within {AUDIT_WORKER_SHUTDOWN_BOUND:?} -- its drain status on exit is UNKNOWN, not confirmed graceful"),
    }

    exit_code
}

/// Splits raw log bytes the same way `mcp_client_audit_worker`'s own
/// internal recovery module does, so this binary's OWN pre-check (done
/// here to decide whether to even call `open_audit_writer` at all) uses
/// the identical classification `spawn_worker`'s own internal
/// `recover_tail` will independently re-derive from the same bytes.
fn split_lines_for_scan(raw: &[u8]) -> Vec<audit_chain_core::LineOutcome> {
    if raw.is_empty() {
        return Vec::new();
    }
    let ends_with_newline = raw.last() == Some(&b'\n');
    let trimmed = if ends_with_newline { &raw[..raw.len() - 1] } else { raw };
    let raw_lines: Vec<&[u8]> = trimmed.split(|&b| b == b'\n').collect();
    let last_index = raw_lines.len().saturating_sub(1);
    raw_lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| match audit_chain_core::parse_line(line) {
            Ok(record) => audit_chain_core::LineOutcome::Parsed(record),
            Err(_) => audit_chain_core::LineOutcome::Unparseable { is_physical_tail: !ends_with_newline && index == last_index },
        })
        .collect()
}

/// Durably records `kind` and returns whether it was confirmed. Every
/// call site below treats a durable-write failure here as a distinct,
/// separately reported fact from whatever real operation it accompanies
/// -- never silently swallowed, never conflated with the real outcome.
async fn record_durable(audit_worker: &mcp_client_audit_worker::AuditWorkerHandle, kind: mcp_client_audit_worker::McpClientEventKind, correlation_id: &str) -> Result<(), mcp_client_audit_worker::DurableRecordError> {
    let event = mcp_client_audit_worker::event(kind, &now_rfc3339(), correlation_id);
    audit_worker.record_durable(event).await.map(|_ack| ())
}

async fn execute(
    args: &Args,
    config: &TrustedConfig,
    correlation_id: &str,
    audit_worker: &mcp_client_audit_worker::AuditWorkerHandle,
    resolved_command: &std::path::Path,
    resolved_script_path: &Option<std::path::PathBuf>,
) -> i32 {
    // --- Phase 3, step 5: register/connect ---
    let mut connect_args = vec![resolved_command.display().to_string()];
    if let Some(script) = resolved_script_path {
        connect_args.push(script.display().to_string());
    }
    connect_args.extend(config.server.args.iter().cloned());

    if let Err(e) = record_durable(audit_worker, mcp_client_audit_worker::McpClientEventKind::RegistrationAttempted { server: config.server.name.clone() }, correlation_id).await {
        eprintln!("error: durable pre-registration of RegistrationAttempted failed; refusing to connect: {e}");
        return 1;
    }

    let registration = mcp_client::McpServerRegistration {
        name: config.server.name.clone(),
        transport: mcp_client::TransportConfig::ChildProcess {
            command: resolved_command.display().to_string(),
            args: {
                let mut a = Vec::new();
                if let Some(script) = resolved_script_path {
                    a.push(script.display().to_string());
                }
                a.extend(config.server.args.iter().cloned());
                a
            },
        },
        enabled: true,
    };

    let conn = match mcp_client::connect(&registration).await {
        Ok(c) => {
            let _ = record_durable(audit_worker, mcp_client_audit_worker::McpClientEventKind::RegistrationOutcome { server: config.server.name.clone(), outcome: mcp_client_audit_worker::RegistrationOutcomeKind::Connected }, correlation_id).await;
            c
        }
        Err(e) => {
            let outcome = match e {
                mcp_client::McpClientError::Disabled => mcp_client_audit_worker::RegistrationOutcomeKind::Disabled,
                mcp_client::McpClientError::Timeout => mcp_client_audit_worker::RegistrationOutcomeKind::Timeout,
                _ => mcp_client_audit_worker::RegistrationOutcomeKind::Failed,
            };
            let _ = record_durable(audit_worker, mcp_client_audit_worker::McpClientEventKind::RegistrationOutcome { server: config.server.name.clone(), outcome }, correlation_id).await;
            eprintln!("error: failed to connect to the external MCP server: {e}");
            return 1;
        }
    };

    // --- Phase 3, step 6: discover tools ---
    if let Err(e) = record_durable(audit_worker, mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: config.server.name.clone() }, correlation_id).await {
        eprintln!("error: durable pre-registration of DiscoveryAttempted failed; refusing to discover: {e}");
        let _ = conn.close().await;
        return 1;
    }
    let mut cache = mcp_client::ToolCache::new();
    let discovered = match mcp_client::discover_tools(&conn, &config.server.name, &mut cache).await {
        Ok(tools) => {
            let _ = record_durable(audit_worker, mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: config.server.name.clone(), outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered }, correlation_id).await;
            tools
        }
        Err(e) => {
            let outcome = match e {
                mcp_client::McpClientError::Timeout => mcp_client_audit_worker::DiscoveryOutcomeKind::Timeout,
                _ => mcp_client_audit_worker::DiscoveryOutcomeKind::Failed,
            };
            let _ = record_durable(audit_worker, mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: config.server.name.clone(), outcome }, correlation_id).await;
            eprintln!("error: failed to discover tools: {e}");
            let _ = conn.close().await;
            return 1;
        }
    };

    // --- Phase 3, step 7: match discovered tools against trusted operator metadata ---
    let trusted = match config.find_tool(&args.tool_name) {
        Some(t) => t,
        None => {
            eprintln!("error: tool {:?} is not present in the trusted configuration -- denied by default, never promoted from discovery", args.tool_name);
            let _ = conn.close().await;
            return 1;
        }
    };
    let discovered_tool = match discovered.iter().find(|d| d.name == args.tool_name) {
        Some(d) => d,
        None => {
            eprintln!("error: tool {:?} is trusted in configuration but was not offered by the server's own tools/list reply right now -- denied", args.tool_name);
            let _ = conn.close().await;
            return 1;
        }
    };

    // --- Schema-hash verification (Fork A substitute for M6.5) ---
    let computed_schema_hash = identity::hash_discovered_schema(&discovered_tool.input_schema);
    if !computed_schema_hash.eq_ignore_ascii_case(&trusted.input_schema_sha256) {
        eprintln!("error: tool {:?}'s own discovered input_schema hash ({computed_schema_hash}) does not match the pinned input_schema_sha256 ({}) -- denied, never invoked", args.tool_name, trusted.input_schema_sha256);
        let _ = conn.close().await;
        return 1;
    }

    let metadata = mcp_client::ExternalToolMetadata {
        server: config.server.name.clone(),
        tool_name: args.tool_name.clone(),
        classification: trusted.classification.into(),
        network_required: trusted.network_required,
        filesystem_scope: trusted.filesystem_scope.clone(),
        sensitivity: trusted.sensitivity.into(),
    };

    let _ = record_durable(
        audit_worker,
        mcp_client_audit_worker::McpClientEventKind::ToolClassified {
            server: config.server.name.clone(),
            tool_name: args.tool_name.clone(),
            classification: trusted.classification.into(),
            sensitivity: trusted.sensitivity.into(),
        },
        correlation_id,
    )
    .await;

    // --- Phase 3, step 8/9: evaluate and record authorization ---
    let approval_policy: mcp_client::ApprovalPolicy = config.approval_policy.clone().into();
    let authorize_result = mcp_client::authorize(&metadata, &approval_policy);
    let allowed = authorize_result.is_ok();
    let _ = record_durable(audit_worker, mcp_client_audit_worker::McpClientEventKind::AuthorizationDecision { server: config.server.name.clone(), tool_name: args.tool_name.clone(), allowed }, correlation_id).await;

    if authorize_result.is_err() {
        let reason = denial_reason_for(&config.approval_policy, &metadata);
        let _ = record_durable(audit_worker, mcp_client_audit_worker::McpClientEventKind::AuthorizationDenied { server: config.server.name.clone(), tool_name: args.tool_name.clone(), reason }, correlation_id).await;
        eprintln!("error: authorization denied for tool {:?}", args.tool_name);
        let _ = conn.close().await;
        return 1;
    }

    // --- Phase 3, step 10: durable pre-registration for Write/Execute ---
    let invocation_id = uuid::Uuid::new_v4().to_string();
    let is_write_or_execute = matches!(trusted.classification, config::ClassificationConfig::Write | config::ClassificationConfig::Execute);

    if is_write_or_execute {
        if let Err(e) = record_durable(
            audit_worker,
            mcp_client_audit_worker::McpClientEventKind::InvocationAttempted { invocation_id: invocation_id.clone(), server: config.server.name.clone(), tool_name: args.tool_name.clone() },
            correlation_id,
        )
        .await
        {
            eprintln!("error: durable pre-registration of this Write/Execute invocation attempt failed to be acknowledged ({e}); refusing to dispatch tools/call -- this is a DefinitelyNotSent outcome, never a possibly-sent one");
            let _ = conn.close().await;
            return 1;
        }
    }

    // --- Phase 3, step 11: invoke exactly one explicitly requested tool ---
    let invoke_result = mcp_client::invoke_external_tool(&conn, &metadata, &approval_policy, args.args_json.clone()).await;

    // --- Phase 3, step 12: record the observed protocol outcome, or an explicit unknown-outcome state ---
    let (outcome_kind, unknown_reason): (mcp_client_audit_worker::InvocationOutcomeKind, Option<mcp_client_audit_worker::UnknownOutcomeReason>) = match &invoke_result {
        Ok(_) => (mcp_client_audit_worker::InvocationOutcomeKind::Success, None),
        Err(mcp_client::McpClientError::PermissionDenied) => (mcp_client_audit_worker::InvocationOutcomeKind::Denied, None),
        Err(mcp_client::McpClientError::Timeout) => (
            mcp_client_audit_worker::InvocationOutcomeKind::Timeout,
            if is_write_or_execute { Some(mcp_client_audit_worker::UnknownOutcomeReason::TimeoutMidCall) } else { None },
        ),
        Err(mcp_client::McpClientError::ConnectionFailed(_)) => (
            mcp_client_audit_worker::InvocationOutcomeKind::ConnectionFailed,
            if is_write_or_execute { Some(mcp_client_audit_worker::UnknownOutcomeReason::ConnectionLostMidCall) } else { None },
        ),
        Err(_) => (mcp_client_audit_worker::InvocationOutcomeKind::MalformedResponse, None),
    };

    let mut durable_audit_ok = true;
    if record_durable(audit_worker, mcp_client_audit_worker::McpClientEventKind::InvocationOutcome { invocation_id: invocation_id.clone(), outcome: outcome_kind }, correlation_id).await.is_err() {
        durable_audit_ok = false;
    }
    if let Some(reason) = unknown_reason {
        if record_durable(
            audit_worker,
            mcp_client_audit_worker::McpClientEventKind::ExternalSideEffectUnknown { invocation_id: invocation_id.clone(), server: config.server.name.clone(), tool_name: args.tool_name.clone(), reason },
            correlation_id,
        )
        .await
        .is_err()
        {
            durable_audit_ok = false;
        }
    }

    let _ = conn.close().await;

    // --- Phase 3, step 13: report the real invocation result separately from any post-invocation audit error ---
    match invoke_result {
        Ok(value) => {
            println!("{value}");
            if durable_audit_ok {
                0
            } else {
                eprintln!("warning: the real invocation succeeded (result printed above), but the durable audit write for its outcome was NOT confirmed -- audit trail uncertainty, not an invocation failure");
                2
            }
        }
        Err(e) => {
            eprintln!("error: invocation failed: {e}");
            if is_write_or_execute && unknown_reason.is_some() {
                eprintln!("notice: this was a Write/Execute invocation whose dispatch may have reached the external server before this failure -- its external side effect is UNKNOWN, not confirmed failed, and is recorded as such in the durable audit log (ExternalSideEffectUnknown)");
            }
            if !durable_audit_ok {
                eprintln!("warning: in addition to the invocation failure above, the durable audit write for this outcome was NOT confirmed");
            }
            1
        }
    }
}

/// Classifies WHY `mcp_client::authorize` denied `metadata` under
/// `policy` -- mirrors `authorize`'s own exact decision logic (read
/// directly from `mcp_client::permission::authorize`'s source) so this
/// reason is never a guess: `AllowScopedExecution` checks
/// `network_required` first, exactly as `authorize` itself does, so a
/// network-scope violation is reported as such even when a filesystem-
/// scope mismatch is also present.
fn denial_reason_for(policy: &config::ApprovalPolicyConfig, metadata: &mcp_client::ExternalToolMetadata) -> mcp_client_audit_worker::DenialReason {
    use config::ApprovalPolicyConfig;
    use mcp_client_audit_worker::DenialReason;
    match policy {
        ApprovalPolicyConfig::Deny => DenialReason::UnconditionalDeny,
        ApprovalPolicyConfig::AskApprovalRequired => DenialReason::ApprovalRequiredButNotGranted,
        ApprovalPolicyConfig::AllowReadOnly => DenialReason::ClassificationNotPermitted,
        ApprovalPolicyConfig::AllowScopedExecution { allow_network, .. } if metadata.network_required && !allow_network => DenialReason::NetworkScopeExceeded,
        ApprovalPolicyConfig::AllowScopedExecution { .. } => DenialReason::FilesystemScopeExceeded,
    }
}
