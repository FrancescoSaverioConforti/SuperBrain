//! Isolated audit probe, not production code or a workspace test.
use chrono::NaiveDate;
use std::collections::HashSet;
use storage::{NoteId, Vault, Version};
use uuid::Uuid;
use vault_security::PiiStore;

fn main() {
    let root = std::env::current_dir()
        .unwrap()
        .join(".superpowers")
        .join(format!("security-audit-probe-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let vault_root = root.join("vault");
    std::fs::create_dir_all(&vault_root).unwrap();
    {
        let repo = git2::Repository::init(&vault_root).unwrap();
        let signature = git2::Signature::now("Synthetic Audit", "audit@localhost").unwrap();
        let oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(oid).unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
            .unwrap();
    }
    let pii = PiiStore::open(root.join("pii"), &root.join("master.key")).unwrap();
    let vault =
        Vault::open_with_pii_store(vault_root.clone(), &root.join("journal.sqlite"), &pii).unwrap();
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let old_index = historical_index::indexer_impl::RealIndexer::new();
    let new_index = index::indexer_impl::RealIndexer::new();
    for field in ["created", "updated", "event_period"] {
        let id = format!("audit-temporal-{field}");
        let marker = format!("AUDIT_{field}_CANARY_9381");
        let value = |name| {
            if name == field {
                marker.as_str()
            } else {
                "2026-10-30"
            }
        };
        let yaml = format!("id: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: sensitive\nproject: audit-project\nsource: fixture-source\ncreated: {}\nupdated: {}\nevent_period: {}\nrelations: {{}}\n", value("created"), value("updated"), value("event_period"));
        let note = format!("---\n{yaml}---\nSYNTHETIC-PRIVATE-BODY");
        let metadata = domain::parse_frontmatter(&yaml).unwrap();
        assert!(matches!(
            gate.evaluate(
                &metadata,
                &note,
                Ok(domain::LinkState::Unlinked),
                vault_security::EncryptedStoreAvailability::Available
            ),
            policy::GateDecision::Pass
        ));
        let outcome = storage::propose_and_commit(
            &vault,
            &note,
            Version(0),
            Uuid::new_v4(),
            &HashSet::new(),
            &HashSet::new(),
            &gate,
            Some(&pii),
        )
        .unwrap();
        assert!(matches!(outcome, storage::CommitResult::Committed(_)));
        let record = vault.read_note(&NoteId::parse(&id).unwrap()).unwrap();
        assert!(record.content.contains(&marker));
        assert!(!record.content.contains("SYNTHETIC-PRIVATE-BODY"));
        let disk = std::fs::read(vault_root.join(format!("{id}.record"))).unwrap();
        let repo = git2::Repository::open(&vault_root).unwrap();
        let tree = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap();
        let entry = tree.get_name(&format!("{id}.record")).unwrap();
        let blob = repo.find_blob(entry.id()).unwrap();
        let raw_meta = String::from_utf8(disk).unwrap().contains(&marker);
        let git_meta = String::from_utf8(blob.content().to_vec())
            .unwrap()
            .contains(&marker);
        historical_index::indexer_impl::rebuild_index(&vault, &old_index).unwrap();
        index::indexer_impl::rebuild_index(&vault, &new_index).unwrap();
        let reference = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let old = historical_index::Index::new(&vault, &old_index, "audit-project", reference)
            .search(&marker, Default::default(), 10)
            .unwrap();
        let new = index::Index::new(&vault, &new_index, "audit-project", reference)
            .search(&marker, Default::default(), 10)
            .unwrap();
        let old_exposes = format!("{old:?}").contains(&marker);
        let new_exposes = format!("{new:?}").contains(&marker);
        assert!(old_exposes && !new_exposes);
        println!("temporal field={field} schema_accepts=true gate_passes=true storage_plaintext_metadata={raw_meta} git_metadata={git_meta} old_search_response_exposes={old_exposes} current_search_response_exposes={new_exposes}");
    }

    // Public raw PII API path traversal: only a fixed sibling of this new fixture.
    let operation = Uuid::new_v4();
    pii.stage("../../escaped-canary", b"SYNTHETIC-ESCAPE", operation)
        .unwrap();
    let escaped = root
        .join(format!("escaped-canary-{operation}.pii"))
        .is_file();
    assert!(escaped);
    let escaped_read =
        pii.get(&format!("../escaped-canary-{operation}")).unwrap() == b"SYNTHETIC-ESCAPE";
    assert!(escaped_read);
    println!("raw_pii_stage_sibling_escape={escaped} raw_pii_get_path_escape={escaped_read}");

    // Cross-note authenticated ciphertext swap under one master key.
    for (id, body) in [
        ("cipher-a", b"BODY-A".as_slice()),
        ("cipher-b", b"BODY-B".as_slice()),
    ] {
        let operation = Uuid::new_v4();
        pii.stage(id, body, operation).unwrap();
        pii.commit_staged(id, operation).unwrap();
    }
    std::fs::copy(
        pii.root().join("cipher-a.pii"),
        pii.root().join("cipher-b.pii"),
    )
    .unwrap();
    let substitution = pii.get_unless_deleted("cipher-b").unwrap() == b"BODY-A";
    assert!(substitution);
    println!("pii_authenticated_cross_note_substitution={substitution}");

    // Backup walks raw files, then restore trusts authenticated backup provenance.
    // Plant only a synthetic scanner fixture, never a real credential.
    let raw_source = root.join("raw-backup-source");
    std::fs::create_dir_all(&raw_source).unwrap();
    let source_vault = Vault::open_with_pii_store(
        raw_source.clone(),
        &root.join("raw-source-journal.sqlite"),
        &pii,
    )
    .unwrap();
    let rogue_id = NoteId::parse("raw-import-note").unwrap();
    let rogue_yaml = "id: raw-import-note\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: audit-project\nsource: fixture\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations: {}\n";
    let rogue_note =
        format!("---\n{rogue_yaml}---\n-----BEGIN PRIVATE KEY-----\nSYNTHETIC-NOT-A-REAL-KEY");
    let rogue_metadata = domain::parse_frontmatter(rogue_yaml).unwrap();
    let rejected = matches!(
        gate.evaluate(
            &rogue_metadata,
            &rogue_note,
            Ok(domain::LinkState::Unlinked),
            vault_security::EncryptedStoreAvailability::Available
        ),
        policy::GateDecision::Reject(_)
    );
    assert!(rejected);
    std::fs::write(
        raw_source.join("raw-import-note.record"),
        format!("1\n---\n{rogue_note}"),
    )
    .unwrap();
    assert!(matches!(
        source_vault.read_note(&rogue_id),
        Err(storage::StorageError::NotFound { .. })
    ));
    let master = vault_security::MasterKey::load_or_create(
        &root.join("master.key"),
        Some(&vault_security::StoreFingerprint::existing()),
    )
    .unwrap();
    let log = vault_security::DeletionLog::open(&pii.deletion_log_path()).unwrap();
    let trusted =
        vault_security::DpapiTrustedReferenceStore::new(pii.deletion_log_reference_path());
    let target = vault_security::FilesystemBackupTarget::new(root.join("backup-target"));
    vault_security::backup(&raw_source, &pii, &log, &master, &target).unwrap();
    let (restored, _) = storage::restore_vault_from_backup(
        &target,
        &master,
        &root.join("restored-vault"),
        &root.join("restore-journal.sqlite"),
        &log,
        &trusted,
        &pii,
    )
    .unwrap();
    let promoted = restored
        .read_note(&rogue_id)
        .unwrap()
        .content
        .contains("SYNTHETIC-NOT-A-REAL-KEY");
    assert!(promoted);
    println!("ordinary_gate_rejects_fixture={rejected} raw_record_absent_from_source_journal=true backup_restore_promotes_ungated_fixture={promoted}");
    drop(restored);
    drop(log);

    // Same missing-log issue through the authoritative Vault and current search.
    let deleted_id = NoteId::parse("audit-temporal-created").unwrap();
    let log = vault_security::DeletionLog::open(&pii.deletion_log_path()).unwrap();
    let trusted =
        vault_security::DpapiTrustedReferenceStore::new(pii.deletion_log_reference_path());
    log.record_deletion(deleted_id.as_str(), &trusted).unwrap();
    let blocked = matches!(
        vault.read_note(&deleted_id),
        Err(storage::StorageError::Deleted { .. })
    );
    assert!(blocked);
    drop(log);
    std::fs::rename(
        pii.deletion_log_path(),
        root.join("authentic-deletion-log.saved.sqlite"),
    )
    .unwrap();
    let served = vault.read_note(&deleted_id).is_ok();
    assert!(served && pii.deletion_log_reference_path().is_file());
    let response = index::Index::new(
        &vault,
        &new_index,
        "audit-project",
        NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
    )
    .search(deleted_id.as_str(), Default::default(), 10)
    .unwrap();
    let search_served = format!("{response:?}").contains(deleted_id.as_str());
    assert!(search_served);
    println!("vault_deleted_before_missing_log={blocked} vault_serves_after_missing_log={served} current_search_serves_deleted_id={search_served}");

    let yaml = "id: version-canary\nschema_version: 4294967297\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: audit-project\nsource: fixture\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations: {}\n";
    let version = domain::parse_frontmatter(yaml).unwrap().schema_version;
    assert_eq!(version, 1);
    println!("schema_version_4294967297_truncates_to={version}");
}
