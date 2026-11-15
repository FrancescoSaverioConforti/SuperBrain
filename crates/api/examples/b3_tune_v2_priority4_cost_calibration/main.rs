//! B3-TUNE-v2 Priority 4 calibration cost measurement: MiniLM
//! dense-only, in the same harness pattern as Stage 1 Task 3's
//! `b3_tune_v2_priority4_cost` binary, but against the CALIBRATION
//! split's 24 answerable queries (not DEV's 49). B1's own same-harness
//! cost-control figures are reused from Stage 1
//! (`eval/results/B3-TUNE-v2/priority4-cost-B1-cost-control.json`)
//! rather than re-measured here: same binary/corpus/machine, and B1's
//! search cost does not depend on which specific questions are asked,
//! only on corpus size (unchanged) and sample count -- re-running B1's
//! cost-control a second time for a 24- vs 49-question difference
//! would measure sampling noise, not a real methodology difference.
//! This choice is restated in the calibration results doc.

#[path = "../priority4_common/embedding.rs"]
mod embedding;

use embedding::{cosine_similarity, EmbeddingModel, ModelSpec, MODEL_SPECS};
use index::document::Document;
use std::collections::HashMap;
use std::time::Instant;
use storage::{NoteId, Vault, Version};
use sysinfo::{get_current_pid, System};
use uuid::Uuid;

const CORPUS_NOTES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/rag/corpus/notes");
const RESULTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/results/B3-TUNE-v2");
const CANARY_ROW_ID: &str = "canary-isolation-probe";
const FROZEN_MODEL: &str = "all-MiniLM-L6-v2";

#[derive(Debug, Clone, serde::Deserialize)]
struct LabelRow {
    #[allow(dead_code)]
    id: String,
    question: String,
    #[allow(dead_code)]
    answer: String,
    #[allow(dead_code)]
    category: String,
    #[allow(dead_code)]
    difficulty: String,
    abstain_expected: bool,
    #[allow(dead_code)]
    relevant_note_ids: Vec<String>,
    #[allow(dead_code)]
    graded_relevance: HashMap<String, u8>,
    #[allow(dead_code)]
    citation_spans: Vec<serde_json::Value>,
}

fn parse_label_rows(text: &str) -> Vec<LabelRow> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("malformed label row: {e}")))
        .collect()
}

fn exclude_canary(rows: Vec<LabelRow>) -> Vec<LabelRow> {
    rows.into_iter().filter(|r| r.id != CANARY_ROW_ID).collect()
}

fn calibration_questions() -> Vec<String> {
    let labels_path = format!("{}/../../eval/rag/calibration/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let raw_labels_text = std::fs::read_to_string(&labels_path).unwrap();
    exclude_canary(parse_label_rows(&raw_labels_text))
        .into_iter()
        .filter(|r| !r.abstain_expected)
        .map(|r| r.question)
        .collect()
}

fn current_process_memory_bytes() -> u64 {
    let pid = get_current_pid().expect("failed to get current pid");
    let mut sys = System::new().expect("failed to initialize sysinfo System");
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}

struct VaultFixture {
    vault: Option<Vault>,
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
        let root = std::env::temp_dir().join(format!("superbrain-b3-tune-v2-p4-cost-calib-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3-TUNE-v2 Priority 4 Cost Calibration", "test@localhost").unwrap();
            let oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
        }
        let pii_root = root.join(".pii-store");
        let pii =
            vault_security::PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();
        pii.bootstrap_deletion_log().unwrap();
        let vault =
            Vault::open_with_pii_store(root.clone(), &root.join("journal.sqlite"), &pii).unwrap();
        Self { vault: Some(vault), pii: Some(pii), root }
    }

    fn vault(&self) -> &Vault {
        self.vault.as_ref().unwrap()
    }

    fn pii(&self) -> &vault_security::PiiStore {
        self.pii.as_ref().unwrap()
    }

    fn commit_corpus(&self) {
        let note_paths: Vec<_> = std::fs::read_dir(CORPUS_NOTES_DIR)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        let known_ids: std::collections::HashSet<NoteId> = note_paths
            .iter()
            .map(|path| {
                let stem = path.file_stem().unwrap().to_str().unwrap();
                NoteId::parse(stem).unwrap()
            })
            .collect();
        for path in &note_paths {
            let note_text = std::fs::read_to_string(path).unwrap();
            let outcome = storage::propose_and_commit(
                self.vault(), &note_text, Version(0), Uuid::new_v4(), &known_ids,
                &std::collections::HashSet::new(),
                &policy::Gate::new(policy::SecretPolicy::new(1)), Some(self.pii()),
            ).unwrap();
            assert!(matches!(outcome, storage::CommitResult::Committed(_)));
        }
    }

    fn all_documents(&self) -> Vec<(String, String)> {
        let ids = self.vault().list_note_ids().unwrap();
        ids.into_iter()
            .map(|id| {
                let record = self.vault().read_note(&id).unwrap();
                let doc = Document::from_record_bytes(record.content.as_bytes()).unwrap();
                (id.as_str().to_string(), doc.text_searchable)
            })
            .collect()
    }
}

fn percentile(sorted_sample: &[f64], p: f64) -> f64 {
    if sorted_sample.is_empty() {
        return 0.0;
    }
    let idx = ((sorted_sample.len() as f64 - 1.0) * p).round() as usize;
    sorted_sample[idx.min(sorted_sample.len() - 1)]
}

#[derive(Debug, serde::Serialize)]
struct CostReport {
    target: String,
    split: String,
    run_commit: String,
    startup_or_model_load_ms: f64,
    build_index_time_ms: f64,
    warm_query_p50_ms: f64,
    warm_query_p95_ms: f64,
    warm_query_sample_size: usize,
    embedding_inference_p50_ms: f64,
    embedding_inference_p95_ms: f64,
    vector_search_p50_ms: f64,
    vector_search_p95_ms: f64,
    peak_rss_after_load_bytes: u64,
    peak_rss_after_build_bytes: u64,
    peak_rss_after_queries_bytes: u64,
    model_weight_bytes_on_disk: Option<u64>,
    vector_index_bytes_on_disk: u64,
}

fn git_rev_parse_head() -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to run git rev-parse HEAD");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model_name = match args.get(1).map(String::as_str) {
        Some("--model") => args.get(2).cloned().unwrap_or_else(|| panic!("--model requires a name")),
        other => panic!("usage: --model {FROZEN_MODEL}; got {other:?}"),
    };
    assert_eq!(model_name, FROZEN_MODEL, "calibration cost measurement is authorized for {FROZEN_MODEL:?} only");

    let env = ort::init().build().expect("failed to initialize ONNX Runtime environment");
    let spec: ModelSpec = MODEL_SPECS.into_iter().find(|s| s.name == model_name).unwrap();
    let model_weight_bytes = embedding::model_cache_dir(spec.name)
        .join("model.onnx")
        .metadata()
        .map(|m| m.len())
        .ok();

    let load_start = Instant::now();
    let mut model = EmbeddingModel::load(&env, spec);
    let startup_or_model_load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    let peak_rss_after_load_bytes = current_process_memory_bytes();

    let fixture = VaultFixture::new();
    fixture.commit_corpus();
    let documents = fixture.all_documents();

    let build_start = Instant::now();
    let note_vectors: Vec<(String, Vec<f32>)> = documents
        .iter()
        .map(|(id, text)| (id.clone(), model.embed_document(text).vector))
        .collect();
    let build_index_time_ms = build_start.elapsed().as_secs_f64() * 1000.0;
    let peak_rss_after_build_bytes = current_process_memory_bytes();

    let questions = calibration_questions();
    let sample: Vec<&String> = questions.iter().collect();
    for q in sample.iter().take(3) {
        let _ = model.embed_query(q);
    }

    let mut total_ms: Vec<f64> = Vec::new();
    let mut embed_ms: Vec<f64> = Vec::new();
    let mut search_ms: Vec<f64> = Vec::new();
    for q in &sample {
        let t0 = Instant::now();
        let qv = model.embed_query(q);
        let embed_elapsed = t0.elapsed().as_secs_f64() * 1000.0;

        let t1 = Instant::now();
        let mut scored: Vec<(String, f32)> = note_vectors
            .iter()
            .map(|(id, v)| (id.clone(), cosine_similarity(&qv.vector, v)))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(10);
        let search_elapsed = t1.elapsed().as_secs_f64() * 1000.0;

        embed_ms.push(embed_elapsed);
        search_ms.push(search_elapsed);
        total_ms.push(embed_elapsed + search_elapsed);
    }
    let peak_rss_after_queries_bytes = current_process_memory_bytes();

    total_ms.sort_by(|a, b| a.total_cmp(b));
    embed_ms.sort_by(|a, b| a.total_cmp(b));
    search_ms.sort_by(|a, b| a.total_cmp(b));

    let dim = note_vectors.first().map(|(_, v)| v.len()).unwrap_or(0);
    let vector_index_bytes_estimate = (dim * 4 * note_vectors.len()) as u64;

    let report = CostReport {
        target: model_name.clone(),
        split: "calibration".to_string(),
        run_commit: git_rev_parse_head(),
        startup_or_model_load_ms,
        build_index_time_ms,
        warm_query_p50_ms: percentile(&total_ms, 0.50),
        warm_query_p95_ms: percentile(&total_ms, 0.95),
        warm_query_sample_size: sample.len(),
        embedding_inference_p50_ms: percentile(&embed_ms, 0.50),
        embedding_inference_p95_ms: percentile(&embed_ms, 0.95),
        vector_search_p50_ms: percentile(&search_ms, 0.50),
        vector_search_p95_ms: percentile(&search_ms, 0.95),
        peak_rss_after_load_bytes,
        peak_rss_after_build_bytes,
        peak_rss_after_queries_bytes,
        model_weight_bytes_on_disk: model_weight_bytes,
        vector_index_bytes_on_disk: vector_index_bytes_estimate,
    };

    std::fs::create_dir_all(RESULTS_DIR).unwrap();
    let out_path = format!("{RESULTS_DIR}/priority4-calibration-cost-{model_name}.json");
    std::fs::write(&out_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!(
        "{}: load/startup={:.1}ms build={:.1}ms warm_p50={:.3}ms warm_p95={:.3}ms peak_rss_after_queries={}MB",
        report.target, report.startup_or_model_load_ms, report.build_index_time_ms,
        report.warm_query_p50_ms, report.warm_query_p95_ms,
        report.peak_rss_after_queries_bytes / (1024 * 1024)
    );
    println!("wrote {out_path}");
}
