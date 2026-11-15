//! B4 performance smoke test: confirms the REAL production
//! `ProductionDenseScorer` (not the Priority 4 evaluation harness)
//! does not grossly violate the frozen cost gates from
//! `eval/results/B3-TUNE-v2/frozen-config-v2.json`'s own
//! `frozen_cost_gates` block.
//!
//! These are OPERATIONAL ACCEPTANCE GATES, not tuning targets --
//! per the user's explicit instruction, this binary never adjusts
//! configuration to make a gate pass; it reports PASS/FAIL against
//! the already-frozen numbers and stops there. This does NOT re-open
//! the B3-TUNE-v2 scientific benchmark (no new held-out, no model
//! change, no retrieval-config change) -- it only confirms the
//! shipped production code's own cost profile against gates the
//! evaluation cycle already measured and froze.
//!
//! Uses the real, already-fetched, hash-verified cached MiniLM
//! artifacts (same reuse convention as every other B4 test). Skips
//! cleanly if not present on this machine.

use std::sync::Arc;

use api::dense::{self, EmbeddingModel};
use api::dense_lifecycle::ProductionDenseScorer;
use index::search::DenseScorer;
use sysinfo::{get_current_pid, System};
use uuid::Uuid;

// B1's own frozen cost-control numbers
// (eval/results/B3-TUNE-v2/priority4-cost-B1-cost-control.json,
// commit 9822b32), reused verbatim -- this binary does not
// re-measure B1's own cost, only the production DENSE path's.
const B1_WARM_QUERY_P95_MS: f64 = 124.5378;
const B1_BUILD_INDEX_TIME_MS: f64 = 144.9637;

// Frozen gates (eval/results/B3-TUNE-v2/frozen-config-v2.json).
const WARM_QUERY_P95_MAX_MULTIPLE_OF_B1: f64 = 3.0;
const PEAK_RAM_MAX_MB: u64 = 300;
const MODEL_WEIGHT_FOOTPRINT_MAX_MB: u64 = 150;
const VECTOR_INDEX_FOOTPRINT_APPROX_MAX_KB_PER_NOTE: f64 = 2.0;
const BUILD_INDEX_TIME_MAX_MULTIPLE_OF_B1: f64 = 5.0;

fn current_process_memory_bytes() -> u64 {
    let pid = get_current_pid().expect("failed to get current pid");
    let mut sys = System::new().expect("failed to initialize sysinfo System");
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}

fn cached_artifact_paths() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let candidates = [
        std::path::PathBuf::from(format!(
            "{manifest_dir}/../../.worktrees/b3-retrieval-eval/.priority4-model-cache/all-MiniLM-L6-v2"
        )),
        std::path::PathBuf::from(format!(
            "{manifest_dir}/../../.priority4-model-cache/all-MiniLM-L6-v2"
        )),
    ];
    for dir in candidates {
        let onnx = dir.join("model.onnx");
        let tokenizer = dir.join("tokenizer.json");
        if onnx.exists() && tokenizer.exists() {
            return Some((onnx, tokenizer));
        }
    }
    None
}

struct VaultFixture {
    vault: Option<storage::Vault>,
    pii: Option<vault_security::PiiStore>,
    root: std::path::PathBuf,
}
impl Drop for VaultFixture {
    fn drop(&mut self) {
        drop(self.vault.take());
        drop(self.pii.take());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl VaultFixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("superbrain-b4-perf-smoke-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B4 Perf Smoke", "test@localhost").unwrap();
            let oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }
        let pii_root = root.join(".pii-store");
        let pii =
            vault_security::PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();
        pii.bootstrap_deletion_log().unwrap();
        let vault =
            storage::Vault::open_with_pii_store(root.clone(), &root.join("journal.sqlite"), &pii)
                .unwrap();
        Self {
            vault: Some(vault),
            pii: Some(pii),
            root,
        }
    }
    fn vault(&self) -> &storage::Vault {
        self.vault.as_ref().unwrap()
    }
    fn write_note(&self, id: &str, body: &str) {
        let text = format!(
            "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: agent:fixture\ncreated: 2026-10-26\nupdated: 2026-10-26\nevent_period: 2026-10-26\nrelations: {{}}\n---\n{body}"
        );
        let result = storage::propose_and_commit(
            self.vault(),
            &text,
            storage::Version(0),
            Uuid::new_v4(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &policy::Gate::new(policy::SecretPolicy::new(1)),
            self.pii.as_ref(),
        );
        assert!(
            matches!(result, Ok(storage::CommitResult::Committed(_))),
            "{result:?}"
        );
    }
}

fn main() {
    let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
        eprintln!("SKIP: cached model artifacts not found on this machine -- run `cargo run --example b3_tune_v2_priority4_dense_dev --features dense-retrieval` once first, or run crate::dense::setup.");
        return;
    };

    // Model-weight footprint gate -- measured directly from the real
    // on-disk artifact, before anything else.
    let model_weight_bytes = std::fs::metadata(&onnx_path).unwrap().len();
    let model_weight_mb = model_weight_bytes as f64 / (1024.0 * 1024.0);

    let fixture = VaultFixture::new();
    const NOTE_COUNT: usize = 53; // matches B3's own corpus scale, per frozen-config.json
    for i in 0..NOTE_COUNT {
        fixture.write_note(
            &format!("note-{i}"),
            &format!("synthetic performance smoke-test content number {i} with some distinguishing words"),
        );
    }

    let env = dense::shared_environment();
    let peak_rss_after_load_bytes = current_process_memory_bytes();
    let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path)
        .expect("hash-verified real artifacts must load");

    // Deliberately NOT std::env::temp_dir() -- a real finding from this
    // binary's own first run: writing a larger (53-note) index
    // directly under %TEMP% on Windows reproducibly hit a persistent
    // ERROR_SHARING_VIOLATION on rename (os error 32), most plausibly
    // real-time antivirus scanning of a freshly-written file in that
    // commonly-watched location -- not reproducible when writing under
    // the vault fixture's own directory instead. This also matches
    // the real production design (B4.3): the dense index always lives
    // under the vault root, never a system temp directory, so this
    // smoke test's own path choice now matches production reality
    // rather than accidentally exercising a location production code
    // never actually uses. A bounded retry-on-transient-sharing-
    // violation was also added to dense_storage::write_atomic as a
    // defensive measure (crates/api/src/dense_storage.rs), though it
    // alone was not sufficient here since this specific lock was not
    // transient within a ~100ms retry window.
    let index_path = fixture.root.join(format!(
        "superbrain-b4-perf-smoke-index-{}.bin",
        Uuid::new_v4()
    ));
    let scorer = Arc::new(ProductionDenseScorer::new(model, index_path.clone()));

    let build_start = std::time::Instant::now();
    scorer
        .rebuild(fixture.vault())
        .expect("rebuild must succeed");
    let build_index_time_ms = build_start.elapsed().as_secs_f64() * 1000.0;
    let peak_rss_after_build_bytes = current_process_memory_bytes();

    let candidate_ids: Vec<storage::NoteId> = (0..NOTE_COUNT)
        .map(|i| storage::NoteId::parse(&format!("note-{i}")).unwrap())
        .collect();
    let mut warm_query_times_ms = Vec::new();
    for i in 0..30 {
        let query = format!("synthetic performance smoke-test content number {i}");
        let start = std::time::Instant::now();
        let scores = scorer.score(&query, &candidate_ids);
        warm_query_times_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        assert!(
            scores.is_some(),
            "a freshly rebuilt index must be available"
        );
    }
    warm_query_times_ms.sort_by(|a, b| a.total_cmp(b));
    let warm_query_p95_ms =
        warm_query_times_ms[(warm_query_times_ms.len() as f64 * 0.95) as usize - 1];
    let peak_rss_after_queries_bytes = current_process_memory_bytes();

    let vector_index_bytes_on_disk = std::fs::metadata(&index_path).unwrap().len();
    let vector_index_kb_per_note = (vector_index_bytes_on_disk as f64 / 1024.0) / NOTE_COUNT as f64;

    let peak_rss_mb = peak_rss_after_queries_bytes as f64 / (1024.0 * 1024.0);
    let warm_p95_multiple_of_b1 = warm_query_p95_ms / B1_WARM_QUERY_P95_MS;
    let build_time_multiple_of_b1 = build_index_time_ms / B1_BUILD_INDEX_TIME_MS;

    println!("=== B4 Production Dense Path -- Performance Smoke Test ===");
    println!(
        "model_weight_footprint: {model_weight_mb:.1} MB (gate: <= {MODEL_WEIGHT_FOOTPRINT_MAX_MB} MB)"
    );
    println!("peak_rss_after_queries: {peak_rss_mb:.1} MB (gate: <= {PEAK_RAM_MAX_MB} MB)");
    println!(
        "warm_query_p95: {warm_query_p95_ms:.3} ms = {warm_p95_multiple_of_b1:.2}x B1's {B1_WARM_QUERY_P95_MS:.3} ms (gate: <= {WARM_QUERY_P95_MAX_MULTIPLE_OF_B1}x)"
    );
    println!(
        "build_index_time: {build_index_time_ms:.1} ms = {build_time_multiple_of_b1:.2}x B1's {B1_BUILD_INDEX_TIME_MS:.1} ms (gate: <= {BUILD_INDEX_TIME_MAX_MULTIPLE_OF_B1}x)"
    );
    println!(
        "vector_index_footprint: {vector_index_kb_per_note:.3} KB/note (gate: approx <= {VECTOR_INDEX_FOOTPRINT_APPROX_MAX_KB_PER_NOTE} KB/note)"
    );
    println!(
        "peak_rss_after_load: {:.1} MB",
        peak_rss_after_load_bytes as f64 / (1024.0 * 1024.0)
    );
    println!(
        "peak_rss_after_build: {:.1} MB",
        peak_rss_after_build_bytes as f64 / (1024.0 * 1024.0)
    );

    let mut pass = true;
    let mut check = |name: &str, ok: bool| {
        println!("{name}: {}", if ok { "PASS" } else { "FAIL" });
        pass &= ok;
    };
    check(
        "model_weight_footprint",
        model_weight_mb <= MODEL_WEIGHT_FOOTPRINT_MAX_MB as f64,
    );
    check("peak_ram", peak_rss_mb <= PEAK_RAM_MAX_MB as f64);
    check(
        "warm_query_p95",
        warm_p95_multiple_of_b1 <= WARM_QUERY_P95_MAX_MULTIPLE_OF_B1,
    );
    check(
        "build_index_time",
        build_time_multiple_of_b1 <= BUILD_INDEX_TIME_MAX_MULTIPLE_OF_B1,
    );
    check(
        "vector_index_footprint",
        vector_index_kb_per_note <= VECTOR_INDEX_FOOTPRINT_APPROX_MAX_KB_PER_NOTE,
    );

    let _ = std::fs::remove_file(&index_path);

    println!("\n=== Overall: {} ===", if pass { "PASS" } else { "FAIL" });
    if !pass {
        std::process::exit(1);
    }
}
