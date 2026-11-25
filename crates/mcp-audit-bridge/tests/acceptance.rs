//! M7 acceptance tests — the scenarios named in the frozen M7 acceptance
//! criteria (as corrected: registration and checkpoint are separate
//! files with separate lifecycles), each against REAL `storage::Vault`-
//! backed source and destination audit logs (never an in-memory stand-
//! in): a real `.audit/audit.log` + `head.ref` on disk for both sides,
//! read and written through the exact same production primitives
//! (`storage::Vault::open_audit_writer`/`read_audit_log_for_recovery_scan`,
//! `audit_chain_core::serialize_record`/`scan_chain`,
//! `brain_api_audit_worker::spawn_worker`) every real composition root
//! uses.
//!
//! Final PASS verification for this acceptance suite is run by the main
//! session, per the user's own standing M7 instruction that subagents
//! may do recon/review but the main session must run the final passes
//! itself.

use mcp_audit_bridge::{register_source, run_once, RunConfig, RunFailure, RunOutcome};

// --- test harness: real vaults, real chains, no shortcuts ---

struct Harness {
    dir: tempfile::TempDir,
    source_vault_root: std::path::PathBuf,
    source_journal_path: std::path::PathBuf,
    destination_vault_root: std::path::PathBuf,
    destination_journal_path: std::path::PathBuf,
    registration_path: std::path::PathBuf,
    checkpoint_path: std::path::PathBuf,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source_vault_root = dir.path().join("source");
        let destination_vault_root = dir.path().join("destination");
        std::fs::create_dir_all(&source_vault_root).unwrap();
        std::fs::create_dir_all(&destination_vault_root).unwrap();
        let source_journal_path = source_vault_root.join("journal.sqlite");
        let destination_journal_path = destination_vault_root.join("journal.sqlite");
        let registration_path = dir.path().join("bridge-registration.json");
        let checkpoint_path = dir.path().join("bridge-checkpoint.json");
        Self { dir, source_vault_root, source_journal_path, destination_vault_root, destination_journal_path, registration_path, checkpoint_path }
    }

    fn config(&self) -> RunConfig {
        RunConfig::new(self.source_vault_root.clone(), self.source_journal_path.clone(), self.destination_vault_root.clone(), self.destination_journal_path.clone(), self.registration_path.clone(), self.checkpoint_path.clone())
    }

    /// M7.7.2: the explicit, one-time registration step every test that
    /// models a genuinely first-ever run must now call -- `run_once`
    /// itself never mints a `SourceLogId` automatically anymore.
    fn register(&self) {
        register_source(&self.registration_path, &self.source_vault_root).unwrap();
    }

    /// Appends `events` to the REAL source vault's own `.audit/audit.log`,
    /// using the exact same `storage::Vault::open_audit_writer` +
    /// `audit_chain_core::serialize_record` primitives a real
    /// `mcp-client-cli` composition root uses — recovering the existing
    /// tail first, exactly as that real binary's own `main.rs` does.
    fn append_source_events(&self, events: &[mcp_client_audit_worker::McpClientEvent]) {
        append_events_to_vault(&self.source_vault_root, &self.source_journal_path, events);
    }

    /// Corrupts one byte near the end of the source log's own on-disk
    /// bytes — the one and only way this harness constructs a
    /// deliberately tampered chain.
    fn corrupt_source_log_byte(&self, byte_offset_from_end: usize) {
        let log_path = self.source_vault_root.join(".audit").join("audit.log");
        let mut bytes = std::fs::read(&log_path).unwrap();
        let len = bytes.len();
        assert!(byte_offset_from_end < len, "corruption offset out of range");
        let idx = len - 1 - byte_offset_from_end;
        bytes[idx] = bytes[idx].wrapping_add(1);
        std::fs::write(&log_path, bytes).unwrap();
    }

    fn read_destination_events(&self) -> Vec<brain_api::AuditEvent> {
        read_events_from_vault(&self.destination_vault_root, &self.destination_journal_path)
    }

    fn append_destination_event_bypassing_the_bridge(&self, kind: brain_api::AuditEventKind) {
        let event = brain_api::event(kind, "2026-01-01T00:00:00Z", uuid::Uuid::new_v4());
        append_raw_destination_event(&self.destination_vault_root, &self.destination_journal_path, &event);
    }
}

fn append_events_to_vault(root: &std::path::Path, journal_path: &std::path::Path, events: &[mcp_client_audit_worker::McpClientEvent]) {
    let vault = storage::Vault::open_with_default_indexer(root.to_path_buf(), journal_path).unwrap();
    let raw = vault.read_audit_log_for_recovery_scan().unwrap();
    let scan = audit_chain_core::scan_chain(&split_lines(&raw));
    let start_sequence = scan.last_verified.as_ref().map(|(s, _)| s + 1).unwrap_or(0);
    let mut prev_hash = scan.last_verified.as_ref().map(|(_, h)| h.clone()).unwrap_or_else(|| audit_chain_core::GENESIS_HASH.to_string());
    let mut writer = vault.open_audit_writer(scan.last_verified.as_ref().map(|(s, h)| (*s, h.as_str()))).unwrap();
    for (offset, event) in events.iter().enumerate() {
        let sequence = start_sequence + offset as u64;
        let (bytes, hash) = audit_chain_core::serialize_record(event, sequence, &prev_hash);
        writer.append(&bytes, sequence, &hash).unwrap();
        prev_hash = hash;
    }
}

fn append_raw_destination_event(root: &std::path::Path, journal_path: &std::path::Path, event: &brain_api::AuditEvent) {
    let vault = storage::Vault::open_with_default_indexer(root.to_path_buf(), journal_path).unwrap();
    let raw = vault.read_audit_log_for_recovery_scan().unwrap();
    let scan = audit_chain_core::scan_chain(&split_lines(&raw));
    let sequence = scan.last_verified.as_ref().map(|(s, _)| s + 1).unwrap_or(0);
    let prev_hash = scan.last_verified.as_ref().map(|(_, h)| h.clone()).unwrap_or_else(|| audit_chain_core::GENESIS_HASH.to_string());
    let (bytes, hash) = audit_chain_core::serialize_record(event, sequence, &prev_hash);
    let mut writer = vault.open_audit_writer(scan.last_verified.as_ref().map(|(s, h)| (*s, h.as_str()))).unwrap();
    writer.append(&bytes, sequence, &hash).unwrap();
}

fn read_events_from_vault(root: &std::path::Path, journal_path: &std::path::Path) -> Vec<brain_api::AuditEvent> {
    let vault = storage::Vault::open_with_default_indexer(root.to_path_buf(), journal_path).unwrap();
    let raw = vault.read_audit_log_for_recovery_scan().unwrap();
    let scan = audit_chain_core::scan_chain(&split_lines(&raw));
    scan.verified_records.iter().map(|r| audit_chain_core::parse_event::<brain_api::AuditEvent>(&r.content_bytes).unwrap()).collect()
}

fn split_lines(raw: &[u8]) -> Vec<audit_chain_core::LineOutcome> {
    let ends_with_newline = raw.last() == Some(&b'\n');
    let trimmed = if ends_with_newline { &raw[..raw.len() - 1] } else { raw };
    if trimmed.is_empty() {
        return Vec::new();
    }
    let raw_lines: Vec<&[u8]> = trimmed.split(|&b| b == b'\n').collect();
    let last_index = raw_lines.len() - 1;
    raw_lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| match audit_chain_core::parse_line(line) {
            Ok(record) => audit_chain_core::LineOutcome::Parsed(record),
            Err(_) => audit_chain_core::LineOutcome::Unparseable { is_physical_tail: !ends_with_newline && index == last_index },
        })
        .collect()
}

fn sample_event(kind: mcp_client_audit_worker::McpClientEventKind, correlation_id: &str) -> mcp_client_audit_worker::McpClientEvent {
    mcp_client_audit_worker::event(kind, "2026-01-01T00:00:00Z", correlation_id)
}

fn bridge_identity_count(events: &[brain_api::AuditEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e.kind, brain_api::AuditEventKind::McpBridgeEvent { .. } | brain_api::AuditEventKind::McpBridgeExternalSideEffectUnknown { .. }))
        .count()
}

fn copy_dir_recursive(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir_recursive(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

// --- M7.7.2: explicit registration policy ---

#[tokio::test]
async fn missing_registration_with_zero_destination_evidence_never_auto_registers() {
    let h = Harness::new();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);
    assert!(!h.registration_path.exists(), "genuinely unregistered source, zero prior destination evidence");

    let result = run_once(&h.config()).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::RegistrationMissing, at_least_one_durable_append_occurred: false })), "got {result:?}");
    assert!(!h.registration_path.exists(), "a RegistrationMissing outcome must never itself create a registration file");
    assert!(h.read_destination_events().is_empty(), "nothing must be imported before an explicit registration exists");

    // The explicit path succeeds, and only then does run_once proceed.
    let registration = register_source(&h.registration_path, &h.source_vault_root).unwrap();
    assert!(h.registration_path.exists());

    let report = run_once(&h.config()).await.unwrap();
    assert_eq!(report.imported.len(), 1);
    let saved = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();
    assert_eq!(saved.registration.source_log_id, registration.source_log_id, "run_once must use the identity register_source already created, never mint a second one");
}

#[tokio::test]
async fn register_source_refuses_to_run_twice_on_the_same_path() {
    let h = Harness::new();
    register_source(&h.registration_path, &h.source_vault_root).unwrap();
    let second = register_source(&h.registration_path, &h.source_vault_root);
    assert!(matches!(second, Err(mcp_audit_bridge::RegisterSourceError::AlreadyRegistered)), "got {second:?}");
}

// --- 1. normal import ---

#[tokio::test]
async fn scenario_1_normal_import_translates_every_source_event_into_the_destination_chain() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[
        sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1"),
        sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered }, "c1"),
    ]);

    let report = run_once(&h.config()).await.unwrap();
    assert_eq!(report.imported.len(), 2);
    assert!(report.skipped_already_imported.is_empty());
    assert!(report.anomalies.is_empty());
    assert_eq!(bridge_identity_count(&h.read_destination_events()), 2);
    assert!(h.registration_path.exists(), "registration must be persisted after a successful run");
    assert!(h.checkpoint_path.exists(), "checkpoint must be persisted after a successful run");
}

// --- 2. duplicate reimport -> zero new Brain facts ---

#[tokio::test]
async fn scenario_2_second_import_of_the_same_source_produces_zero_new_brain_facts() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);

    run_once(&h.config()).await.unwrap();
    let after_first = h.read_destination_events();

    let second_report = run_once(&h.config()).await.unwrap();
    assert!(second_report.imported.is_empty(), "nothing new to import");
    let after_second = h.read_destination_events();
    assert_eq!(after_first.len(), after_second.len(), "re-running the bridge over unchanged source must append zero new destination records");
}

// --- 3. crash after destination append but before checkpoint save -> zero duplicate on restart ---

#[tokio::test]
async fn scenario_3_crash_after_destination_append_before_checkpoint_save_does_not_duplicate_on_restart() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);

    // Real fault injection through the production code path: the
    // destination append durably succeeds, then this run returns an
    // error BEFORE its own checkpoint save for that record.
    let mut crashing_config = h.config();
    crashing_config.crash_after_n_destination_appends = Some(1);
    let result = run_once(&crashing_config).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::InjectedFault { appends_so_far: 1 }, .. })), "got {result:?}");

    let after_crash = h.read_destination_events();
    assert_eq!(bridge_identity_count(&after_crash), 1, "the destination append itself must have genuinely completed before the simulated crash");
    // Registration WAS saved before the simulated crash point (it is
    // saved, then the checkpoint, then the crash check happens only
    // after an import -- re-check the real ordering: in `run_once`, the
    // crash check happens immediately after the append and BEFORE either
    // state file is updated for that record, so NEITHER file reflects
    // this import yet).
    assert!(!h.registration_path.exists() || registration_last_known_tail_sequence(&h.registration_path).is_none(), "registration must not yet record this import at the simulated crash point");
    assert!(!h.checkpoint_path.exists(), "checkpoint must not exist yet at the simulated crash point");

    // Restart for real, with no injected fault this time.
    let restart_report = run_once(&h.config()).await.unwrap();
    assert!(restart_report.imported.is_empty(), "the already-durably-appended fact must be recognized via the full destination scan, not re-imported");
    assert_eq!(restart_report.skipped_already_imported.len(), 1);

    let after_restart = h.read_destination_events();
    assert_eq!(bridge_identity_count(&after_restart), 1, "zero duplicate Brain facts after restart");
}

fn registration_last_known_tail_sequence(path: &std::path::Path) -> Option<u64> {
    mcp_audit_bridge::registration::load(path).ok().flatten().and_then(|f| f.registration.last_known_tail).map(|(seq, _)| seq)
}

// --- 4. interleaved Brain events -> zero duplicate ---

#[tokio::test]
async fn scenario_4_interleaved_unrelated_destination_events_do_not_cause_a_duplicate_on_rerun() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);

    run_once(&h.config()).await.unwrap();
    assert_eq!(bridge_identity_count(&h.read_destination_events()), 1);

    // Ordinary, unrelated Brain activity appended AFTER the bridge's own
    // fact -- pushing it away from the tail.
    h.append_destination_event_bypassing_the_bridge(brain_api::AuditEventKind::WriteMemory { proposal_id: "p1".into() });
    h.append_destination_event_bypassing_the_bridge(brain_api::AuditEventKind::ToolCall { tool: "search".into(), allowed: true });

    let second_report = run_once(&h.config()).await.unwrap();
    assert!(second_report.imported.is_empty(), "the full-chain dedup scan must find the already-imported identity even though it is no longer near the tail");
    assert_eq!(bridge_identity_count(&h.read_destination_events()), 1, "no duplicate bridge fact must be created");
}

// --- 5. tampered source chain -> fail closed ---

#[tokio::test]
async fn scenario_5_a_tampered_source_chain_fails_closed_and_imports_nothing() {
    let h = Harness::new();
    h.append_source_events(&[
        sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1"),
        sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered }, "c1"),
    ]);
    h.corrupt_source_log_byte(5);

    let result = run_once(&h.config()).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::SourceChainInvalid(_), .. })), "got {result:?}");
    assert!(h.read_destination_events().is_empty(), "a fail-closed source scan must import nothing at all, not even a partial prefix");
}

// --- 6. destination with two copies of the same SourceRecordIdentity -> anomaly detected ---

#[tokio::test]
async fn scenario_6_two_copies_of_the_same_source_record_identity_in_the_destination_are_reported_as_an_anomaly() {
    let h = Harness::new();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);

    let source_vault = storage::Vault::open_with_default_indexer(h.source_vault_root.clone(), &h.source_journal_path).unwrap();
    let raw = source_vault.read_audit_log_for_recovery_scan().unwrap();
    let scan = audit_chain_core::scan_chain(&split_lines(&raw));
    let record = &scan.verified_records[0];
    let source_log_id = mcp_audit_bridge::SourceLogId::generate();
    let identity = mcp_audit_bridge::SourceRecordIdentity { source_log_id, sequence: record.sequence, record_hash: record.record_hash.clone() };
    let source_event: mcp_client_audit_worker::McpClientEvent = audit_chain_core::parse_event(&record.content_bytes).unwrap();
    let translated = mcp_audit_bridge::translate(&source_event.kind, &identity);

    // Two copies of the exact same translated fact, written directly
    // (bypassing the bridge's own dedup) -- modeling a pre-existing
    // integrity anomaly already present in the destination chain.
    h.append_destination_event_bypassing_the_bridge(translated.clone());
    h.append_destination_event_bypassing_the_bridge(translated);

    let report = run_once(&h.config()).await.unwrap();
    assert_eq!(report.anomalies.len(), 1, "got {:?}", report.anomalies);
    assert_eq!(report.anomalies[0].occurrence_count, 2);
}

// --- 7. progress checkpoint deleted, registration intact -> zero duplicate + recovery ---

#[tokio::test]
async fn scenario_7_deleting_only_the_progress_checkpoint_recovers_under_the_same_identity_with_zero_duplicates() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);

    run_once(&h.config()).await.unwrap();
    assert!(h.registration_path.exists());
    assert!(h.checkpoint_path.exists());
    let registration_before = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();

    // Delete ONLY the progress checkpoint -- registration (the durable
    // identity) stays intact, per the frozen correction's own split.
    std::fs::remove_file(&h.checkpoint_path).unwrap();
    assert!(!h.checkpoint_path.exists());
    assert!(h.registration_path.exists(), "registration must survive checkpoint loss");

    let report = run_once(&h.config()).await.unwrap();
    assert!(report.imported.is_empty(), "the destination full-chain scan must recognize the already-imported identity even with no checkpoint to consult");
    assert_eq!(report.skipped_already_imported.len(), 1);

    let registration_after = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();
    assert_eq!(registration_before.registration.source_log_id, registration_after.registration.source_log_id, "the SAME SourceLogId must be used across the checkpoint loss -- this is the entire point of the split");

    assert_eq!(bridge_identity_count(&h.read_destination_events()), 1, "zero duplicate Brain facts");
    assert!(h.checkpoint_path.exists(), "checkpoint must be rebuilt by this run");
}

// --- 8. registration corrupted -> fail closed ---

#[tokio::test]
async fn scenario_8_a_corrupted_registration_file_fails_closed_rather_than_being_silently_ignored() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);
    run_once(&h.config()).await.unwrap();

    std::fs::write(&h.registration_path, b"not valid json").unwrap();

    let before = h.read_destination_events();
    let result = run_once(&h.config()).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::RegistrationLoadFailed(_), .. })), "got {result:?}");
    assert_eq!(h.read_destination_events().len(), before.len(), "a fail-closed registration load must import nothing further");
}

// --- 9. registration lost + unique destination evidence -> recovery ---

#[tokio::test]
async fn scenario_9a_losing_the_registration_file_entirely_recovers_the_same_identity_from_unique_destination_evidence() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);

    run_once(&h.config()).await.unwrap();
    let registration_before = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();

    // Lose the registration file ENTIRELY (not just the checkpoint) --
    // the destination chain is the only remaining evidence of identity.
    std::fs::remove_file(&h.registration_path).unwrap();
    std::fs::remove_file(&h.checkpoint_path).unwrap();
    assert!(!h.registration_path.exists());

    let report = run_once(&h.config()).await.unwrap();
    assert!(report.imported.is_empty(), "the recovered registration must carry the SAME SourceLogId the destination already has evidence for, so the full-chain scan finds it as already imported");
    assert_eq!(report.skipped_already_imported.len(), 1);

    let registration_after = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();
    assert_eq!(registration_before.registration.source_log_id, registration_after.registration.source_log_id, "recovery must reconstruct the SAME identity, never mint a fresh one when unique evidence exists");
    assert_eq!(bridge_identity_count(&h.read_destination_events()), 1, "zero duplicate Brain facts after full registration loss, given unique destination evidence");
}

// --- 9b. registration lost, destination evidence ambiguous -> explicit fail closed ---

#[tokio::test]
async fn scenario_9b_losing_the_registration_with_ambiguous_destination_evidence_fails_closed_not_guessed() {
    let h = Harness::new();
    h.append_source_events(&[
        sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1"),
        sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered }, "c1"),
    ]);

    let source_vault = storage::Vault::open_with_default_indexer(h.source_vault_root.clone(), &h.source_journal_path).unwrap();
    let raw = source_vault.read_audit_log_for_recovery_scan().unwrap();
    let scan = audit_chain_core::scan_chain(&split_lines(&raw));

    // Two DIFFERENT SourceLogIds, each with evidence matching a
    // DIFFERENT verified source record -- a genuinely ambiguous
    // destination with no way to pick one automatically.
    let log_id_a = mcp_audit_bridge::SourceLogId::generate();
    let id_a = mcp_audit_bridge::SourceRecordIdentity { source_log_id: log_id_a, sequence: scan.verified_records[0].sequence, record_hash: scan.verified_records[0].record_hash.clone() };
    let event_a: mcp_client_audit_worker::McpClientEvent = audit_chain_core::parse_event(&scan.verified_records[0].content_bytes).unwrap();
    h.append_destination_event_bypassing_the_bridge(mcp_audit_bridge::translate(&event_a.kind, &id_a));

    let log_id_b = mcp_audit_bridge::SourceLogId::generate();
    let id_b = mcp_audit_bridge::SourceRecordIdentity { source_log_id: log_id_b, sequence: scan.verified_records[1].sequence, record_hash: scan.verified_records[1].record_hash.clone() };
    let event_b: mcp_client_audit_worker::McpClientEvent = audit_chain_core::parse_event(&scan.verified_records[1].content_bytes).unwrap();
    h.append_destination_event_bypassing_the_bridge(mcp_audit_bridge::translate(&event_b.kind, &id_b));

    // No registration file exists at all -- ambiguous evidence, must
    // fail closed rather than pick either candidate.
    assert!(!h.registration_path.exists());
    let result = run_once(&h.config()).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::RegistrationRecoveryFailed(mcp_audit_bridge::RegistrationRecoveryError::AmbiguousCandidates(_)), .. })), "got {result:?}");
}

// --- 10. same path, incompatible replaced chain -> fail closed ---

#[tokio::test]
async fn scenario_10_the_same_path_with_an_incompatible_replaced_chain_fails_closed() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[
        sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1"),
        sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered }, "c1"),
    ]);
    run_once(&h.config()).await.unwrap();

    // Replace the source vault's audit log at the SAME path with a
    // brand-new, shorter, differently-content chain -- never
    // auto-rebound onto the existing SourceLogId.
    let log_path = h.source_vault_root.join(".audit").join("audit.log");
    let head_ref_path = h.source_vault_root.join(".audit").join("head.ref");
    std::fs::remove_file(&log_path).unwrap();
    let _ = std::fs::remove_file(&head_ref_path);
    let fresh_vault = storage::Vault::open_with_default_indexer(h.source_vault_root.clone(), &h.source_journal_path).unwrap();
    let fresh_event = sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "different-server".into() }, "c-new");
    let (bytes, hash) = audit_chain_core::serialize_record(&fresh_event, 0, audit_chain_core::GENESIS_HASH);
    let mut writer = fresh_vault.open_audit_writer(None).unwrap();
    writer.append(&bytes, 0, &hash).unwrap();
    drop(writer);

    let before = h.read_destination_events();
    let result = run_once(&h.config()).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::SourceRelocationRefused(_), .. })), "got {result:?}");
    assert_eq!(h.read_destination_events().len(), before.len(), "a fail-closed relocation refusal must import nothing");
}

// --- 11. relocation with stable registration + compatible chain -> controlled rebind ---

#[tokio::test]
async fn scenario_11_relocating_to_a_new_path_with_a_compatible_chain_is_accepted_and_continues_importing() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);
    run_once(&h.config()).await.unwrap();
    let registration_before = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();

    // Relocate: copy the entire source vault directory to a new root
    // (same chain content, different path) and append one more event
    // there -- modeling "the real log moved, and kept growing".
    let new_root = h.dir.path().join("source-relocated");
    copy_dir_recursive(&h.source_vault_root, &new_root);
    let new_journal = new_root.join("journal.sqlite");
    append_events_to_vault(&new_root, &new_journal, &[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryOutcome { server: "s".into(), outcome: mcp_client_audit_worker::DiscoveryOutcomeKind::Discovered }, "c1")]);

    let mut config = h.config();
    config.source_vault_root = new_root;
    config.source_journal_path = new_journal;

    let report = run_once(&config).await.unwrap();
    assert_eq!(report.imported.len(), 1, "only the new record at the relocated path must be imported");

    let registration_after = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();
    assert_eq!(registration_before.registration.source_log_id, registration_after.registration.source_log_id, "relocation must preserve the SAME SourceLogId, never mint a new one");
    assert_eq!(bridge_identity_count(&h.read_destination_events()), 2);
}

// --- concurrency: the bridge lock added after the security/recovery review ---

// 1 & 2: first run_once holds the lock; a second run_once against the
// SAME state paths fails immediately, touching neither registration nor
// checkpoint nor the destination.
#[tokio::test]
async fn concurrency_1_and_2_a_second_run_once_against_the_same_state_fails_without_touching_any_state() {
    let h = Harness::new();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);

    // Hold the lock directly, simulating a first run_once already in
    // progress -- this is the real production lock (crate::bridge_lock),
    // not a stand-in.
    let held_lock = mcp_audit_bridge::bridge_lock::acquire(&h.checkpoint_path).unwrap();
    assert!(!h.registration_path.exists());
    assert!(!h.checkpoint_path.exists());

    let result = run_once(&h.config()).await;
    assert!(matches!(result, Err(RunFailure { outcome: RunOutcome::AlreadyRunning(_), .. })), "got {result:?}");

    // Neither state file was created, and the destination is untouched.
    assert!(!h.registration_path.exists(), "a refused run must never create a registration file");
    assert!(!h.checkpoint_path.exists(), "a refused run must never create a checkpoint file");
    assert!(h.read_destination_events().is_empty(), "a refused run must never append to the destination");

    drop(held_lock);
}

// 3: after the lock is released, a new run_once proceeds normally.
#[tokio::test]
async fn concurrency_3_after_lock_release_a_new_run_once_proceeds() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);

    let held_lock = mcp_audit_bridge::bridge_lock::acquire(&h.checkpoint_path).unwrap();
    let blocked = run_once(&h.config()).await;
    assert!(matches!(blocked, Err(RunFailure { outcome: RunOutcome::AlreadyRunning(_), .. })));
    drop(held_lock);

    let report = run_once(&h.config()).await.unwrap();
    assert_eq!(report.imported.len(), 1, "once the lock is free, run_once must proceed normally");
}

// 4: a real cross-process crash releases the lock -- see the dedicated,
// more thorough reproduction in crate::bridge_lock's own test module
// (killing_the_process_holding_the_bridge_lock_releases_it_for_a_fresh_acquire),
// which spawns a genuinely separate OS process and kills it without any
// cleanup code running. This is deliberately not duplicated at the
// run_once level (spawning this test binary recursively as a run_once
// composition root is out of this acceptance suite's scope); the
// run_once-level tests above and below instead confirm run_once itself
// correctly surfaces BridgeLockError::AlreadyRunning and correctly
// proceeds once free, which is the behavior that actually depends on
// whichever process (this one or a real separate one, per the dedicated
// cross-process test) released the lock.

// 5: a bridge pointed at a DIFFERENT state directory can run in parallel.
#[tokio::test]
async fn concurrency_5_a_bridge_with_a_different_state_directory_runs_in_parallel() {
    let h1 = Harness::new();
    h1.register();
    h1.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s1".into() }, "c1")]);
    let h2 = Harness::new();
    h2.register();
    h2.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s2".into() }, "c2")]);

    let held_lock_1 = mcp_audit_bridge::bridge_lock::acquire(&h1.checkpoint_path).unwrap();

    // h2's own state directory is entirely different, so its own lock
    // acquisition inside run_once must succeed even while h1's lock is held.
    let report_2 = run_once(&h2.config()).await.unwrap();
    assert_eq!(report_2.imported.len(), 1, "a bridge run over a different state directory must not be blocked by h1's lock");

    drop(held_lock_1);
}

// 6: no duplicate Brain fact is produced when two run_once calls race
// on the SAME already-registered source -- the bridge_lock makes this
// deterministic: the second call is always refused outright, never
// allowed to race the first through the dedup/append sequence.
//
// M7.7.2 note: this scenario used to be framed as "no duplicate
// SourceLogId is minted under first-run concurrency," testing the
// pre-M7.7 automatic-mint behavior. Under the new explicit-registration
// policy, run_once never mints a SourceLogId at all (see
// `register_source`), so that specific race no longer exists --
// REPLACED here with the still-real, still-load-bearing concurrent-
// IMPORT race: two run_once calls against the same, already-registered
// state, racing on the destination append/dedup sequence itself.
//
// Multi-threaded runtime + `tokio::spawn`, deliberately: the default
// current-thread `#[tokio::test]` runs cooperatively on one OS thread,
// and even on a multi-thread runtime, `tokio::join!` alone does not
// guarantee two futures actually execute on separate OS threads at the
// same time -- `tokio::spawn` is what hands each one to the runtime's
// own scheduler as an independent task, which the multi-thread runtime
// can then genuinely run in parallel on its worker threads. That real,
// OS-level simultaneity is what this test needs to be a meaningful
// proof of the lock's cross-thread exclusivity (cross-PROCESS
// exclusivity has its own dedicated, more thorough reproduction in
// `crate::bridge_lock`'s own test module).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrency_6_no_duplicate_brain_fact_under_concurrent_import_of_an_already_registered_source() {
    let h = Harness::new();
    h.register();
    h.append_source_events(&[sample_event(mcp_client_audit_worker::McpClientEventKind::DiscoveryAttempted { server: "s".into() }, "c1")]);
    assert!(h.registration_path.exists(), "this scenario starts from an already-registered source");

    let config_a = h.config();
    let config_b = h.config();
    let task_a = tokio::spawn(async move { run_once(&config_a).await });
    let task_b = tokio::spawn(async move { run_once(&config_b).await });
    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let (result_a, result_b) = (result_a.expect("task a must not panic"), result_b.expect("task b must not panic"));

    let outcomes = [result_a, result_b];
    let successes: Vec<_> = outcomes.iter().filter(|r| r.is_ok()).collect();
    let refusals: Vec<_> = outcomes.iter().filter(|r| matches!(r, Err(RunFailure { outcome: RunOutcome::AlreadyRunning(_), .. }))).collect();
    assert_eq!(successes.len(), 1, "exactly one of the two concurrent first-run calls must succeed, got {outcomes:?}");
    assert_eq!(refusals.len(), 1, "the other must be refused as AlreadyRunning, got {outcomes:?}");

    // Exactly one registration, one SourceLogId, no duplicate Brain facts.
    let registration = mcp_audit_bridge::registration::load(&h.registration_path).unwrap().unwrap();
    let destination_events = h.read_destination_events();
    assert_eq!(bridge_identity_count(&destination_events), 1, "exactly one Brain fact, under exactly one SourceLogId, despite the race");
    let _ = registration; // existence + single identity already proven by the single-fact assertion above
}
