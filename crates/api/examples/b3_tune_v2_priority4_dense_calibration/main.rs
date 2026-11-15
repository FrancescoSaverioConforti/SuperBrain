//! B3-TUNE-v2 Priority 4 calibration run: dense-standalone MiniLM
//! benchmark on the CALIBRATION split, exactly once, per the user's
//! authorization. Frozen model (`all-MiniLM-L6-v2`, Stage 1 Task 4),
//! frozen representation semantics
//! (`docs/superpowers/specs/2026-11-14-b3-tune-v2-priority4-stage1-dev-freeze.md`
//! Section 2), unmodified. Hard-coded to the `calibration` split only
//! -- never dev, never held-out -- same safety-assertion discipline
//! as Stage 1's own `b3_tune_v2_priority4_dense_dev` binary, which
//! this binary is a split-locked copy of (not a parametrized variant,
//! so a calibration run can never be accidentally pointed elsewhere).
//!
//! `--model all-MiniLM-L6-v2` is the only accepted invocation --
//! calibration is authorized for the frozen model only, not for
//! re-running the other 3 shortlisted models.

#[path = "../priority4_common/embedding.rs"]
mod embedding;

use embedding::{cosine_similarity, EmbeddingModel, ModelSpec, MODEL_SPECS};
use index::document::Document;
use ort::environment::Environment;
use std::collections::HashMap;
use storage::{NoteId, Vault, Version};
use uuid::Uuid;

const CORPUS_NOTES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/rag/corpus/notes");
const RESULTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/results/B3-TUNE-v2");
const DENSE_TOP_N: usize = 50;
const K: usize = 10;
const BOOTSTRAP_SEED: u64 = 0;
const BOOTSTRAP_RESAMPLES: u32 = 2000;
const CANARY_ROW_ID: &str = "canary-isolation-probe";
const FROZEN_MODEL: &str = "all-MiniLM-L6-v2";

#[derive(Debug, Clone, serde::Deserialize)]
struct LabelRow {
    id: String,
    question: String,
    #[allow(dead_code)]
    answer: String,
    category: String,
    difficulty: String,
    abstain_expected: bool,
    relevant_note_ids: Vec<String>,
    graded_relevance: HashMap<String, u8>,
    #[allow(dead_code)]
    citation_spans: Vec<serde_json::Value>,
}

fn parse_label_rows(text: &str) -> Vec<LabelRow> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("malformed label row: {e}\nline: {line}"))
        })
        .collect()
}

fn exclude_canary(rows: Vec<LabelRow>) -> Vec<LabelRow> {
    rows.into_iter().filter(|r| r.id != CANARY_ROW_ID).collect()
}

struct CorpusNote {
    id: String,
    text_searchable: String,
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
        let root = std::env::temp_dir()
            .join(format!("superbrain-b3-tune-v2-p4-calib-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3-TUNE-v2 Priority 4 Calibration", "test@localhost").unwrap();
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
                self.vault(),
                &note_text,
                Version(0),
                Uuid::new_v4(),
                &known_ids,
                &std::collections::HashSet::new(),
                &policy::Gate::new(policy::SecretPolicy::new(1)),
                Some(self.pii()),
            )
            .unwrap();
            assert!(
                matches!(outcome, storage::CommitResult::Committed(_)),
                "corpus note {path:?} failed to commit: {outcome:?}"
            );
        }
    }

    fn all_documents(&self) -> Vec<CorpusNote> {
        let ids = self.vault().list_note_ids().unwrap();
        ids.into_iter()
            .map(|id| {
                let record = self.vault().read_note(&id).unwrap();
                let doc = Document::from_record_bytes(record.content.as_bytes())
                    .unwrap_or_else(|e| panic!("failed to parse record for {id:?}: {e}"));
                CorpusNote { id: id.as_str().to_string(), text_searchable: doc.text_searchable }
            })
            .collect()
    }
}

fn spec_by_name(name: &str) -> ModelSpec {
    MODEL_SPECS
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("unknown model {name:?}"))
}

#[derive(Debug, serde::Serialize)]
struct MetricBreakdown {
    recall_at_10: f64,
    ndcg_at_10: f64,
    recall_ci: [f64; 2],
    ndcg_ci: [f64; 2],
    n: usize,
    low_confidence: bool,
}

fn metrics_breakdown(recall_values: &[f64], ndcg_values: &[f64], seed: u64, resamples: u32) -> MetricBreakdown {
    let n = recall_values.len();
    let recall_mean = recall_values.iter().sum::<f64>() / n as f64;
    let ndcg_mean = ndcg_values.iter().sum::<f64>() / n as f64;
    let recall_ci = api::metrics::bootstrap_ci(recall_values, seed, resamples);
    let ndcg_ci = api::metrics::bootstrap_ci(ndcg_values, seed, resamples);
    MetricBreakdown {
        recall_at_10: recall_mean,
        ndcg_at_10: ndcg_mean,
        recall_ci: [recall_ci.low, recall_ci.high],
        ndcg_ci: [ndcg_ci.low, ndcg_ci.high],
        n,
        low_confidence: n < api::metrics::LOW_CONFIDENCE_FLOOR,
    }
}

fn breakdown_by<F: Fn(&LabelRow) -> String>(
    answerable_rows: &[&LabelRow],
    recall_values: &[f64],
    ndcg_values: &[f64],
    key_fn: F,
    seed: u64,
    resamples: u32,
) -> HashMap<String, MetricBreakdown> {
    let mut by_key: HashMap<String, (Vec<f64>, Vec<f64>)> = HashMap::new();
    for (i, row) in answerable_rows.iter().enumerate() {
        let key = key_fn(row);
        let entry = by_key.entry(key).or_default();
        entry.0.push(recall_values[i]);
        entry.1.push(ndcg_values[i]);
    }
    by_key
        .into_iter()
        .map(|(key, (recalls, ndcgs))| (key, metrics_breakdown(&recalls, &ndcgs, seed, resamples)))
        .collect()
}

#[derive(Debug, serde::Serialize)]
struct AbstentionConfusion {
    tp: usize,
    fp: usize,
    tn: usize,
    fn_: usize,
}

#[derive(Debug, serde::Serialize)]
struct CandidateRecallReport {
    model: String,
    split: String,
    run_commit: String,
    labels_sha256: String,
    dense_top_n: usize,
    query_count: usize,
    answerable_query_count: usize,
    candidate_recall_hits: usize,
    candidate_recall_rate: f64,
    truncation_events: usize,
    total_embed_calls: usize,
    dense_recall_at_10: f64,
    dense_recall_at_10_ci: [f64; 2],
    dense_ndcg_at_10: f64,
    dense_ndcg_at_10_ci: [f64; 2],
    delta_recall_vs_b1_frozen: f64,
    delta_ndcg_vs_b1_frozen: f64,
    by_category: HashMap<String, MetricBreakdown>,
    by_difficulty: HashMap<String, MetricBreakdown>,
    abstention_confusion_matrix: AbstentionConfusion,
}

fn git_rev_parse_head() -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to run git rev-parse HEAD");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn sha256_hex_of_file(path: &str) -> String {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("{:x}", hasher.finalize())
}

// B1's own frozen calibration numbers (eval/results/B3/calibration-20261114T123333Z-e613edd.json),
// reused verbatim for delta computation -- never re-measured by this binary.
const B1_FROZEN_CALIBRATION_RECALL_AT_10: f64 = 0.9166666666666666;
const B1_FROZEN_CALIBRATION_NDCG_AT_10: f64 = 0.7090535447985884;

fn run_dense_calibration(env: &Environment, model_name: &str) {
    assert_eq!(
        model_name, FROZEN_MODEL,
        "Priority 4 calibration is authorized for the frozen model {FROZEN_MODEL:?} only; got {model_name:?}"
    );
    let split = "calibration";
    let labels_path = format!("{}/../../eval/rag/{split}/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let raw_labels_text = std::fs::read_to_string(&labels_path).unwrap();
    let rows = exclude_canary(parse_label_rows(&raw_labels_text));

    let fixture = VaultFixture::new();
    fixture.commit_corpus();
    let documents = fixture.all_documents();

    let spec = spec_by_name(model_name);
    let mut model = EmbeddingModel::load(env, spec);

    eprintln!("[{model_name}] embedding {} corpus notes (calibration run)...", documents.len());
    let mut truncation_events = 0usize;
    let mut total_embed_calls = 0usize;
    let note_vectors: Vec<(String, Vec<f32>)> = documents
        .iter()
        .map(|doc| {
            let result = model.embed_document(&doc.text_searchable);
            total_embed_calls += 1;
            if result.truncated {
                truncation_events += 1;
            }
            (doc.id.clone(), result.vector)
        })
        .collect();

    let mut recall_values: Vec<f64> = Vec::new();
    let mut ndcg_values: Vec<f64> = Vec::new();
    let mut candidate_recall_hits = 0usize;
    let mut answerable_rows: Vec<&LabelRow> = Vec::new();

    // Abstention tripwire: this harness has no abstention mechanism of
    // its own (dense cosine search always returns a ranked list), so
    // TP/FP are structurally 0 by construction; FN = abstain_expected
    // count, TN = 0. This mirrors B1's own constant-non-abstaining
    // pattern and is reported as a correctness tripwire only -- not a
    // target to optimize, per the user's explicit instruction.
    let mut abstain_fn = 0usize;

    for row in &rows {
        if row.abstain_expected {
            abstain_fn += 1;
            continue;
        }
        let q = model.embed_query(&row.question);
        total_embed_calls += 1;
        if q.truncated {
            truncation_events += 1;
        }
        let mut scored: Vec<(String, f32)> = note_vectors
            .iter()
            .map(|(id, v)| (id.clone(), cosine_similarity(&q.vector, v)))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(DENSE_TOP_N);

        let any_relevant_in_topn = row
            .relevant_note_ids
            .iter()
            .any(|rid| scored.iter().any(|(id, _)| id == rid));
        if any_relevant_in_topn {
            candidate_recall_hits += 1;
        }

        let top_k_ids: Vec<String> = scored.iter().take(K).map(|(id, _)| id.clone()).collect();
        recall_values.push(api::metrics::recall_at_10(&row.relevant_note_ids, &top_k_ids));
        ndcg_values.push(api::metrics::ndcg_at_10(&top_k_ids, &row.graded_relevance));
        answerable_rows.push(row);
    }

    let recall_ci = api::metrics::bootstrap_ci(&recall_values, BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);
    let ndcg_ci = api::metrics::bootstrap_ci(&ndcg_values, BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);
    let recall_mean = recall_values.iter().sum::<f64>() / recall_values.len() as f64;
    let ndcg_mean = ndcg_values.iter().sum::<f64>() / ndcg_values.len() as f64;

    let by_category = breakdown_by(
        &answerable_rows, &recall_values, &ndcg_values, |r| r.category.clone(),
        BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES,
    );
    let by_difficulty = breakdown_by(
        &answerable_rows, &recall_values, &ndcg_values, |r| r.difficulty.clone(),
        BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES,
    );

    let report = CandidateRecallReport {
        model: model_name.to_string(),
        split: split.to_string(),
        run_commit: git_rev_parse_head(),
        labels_sha256: sha256_hex_of_file(&labels_path),
        dense_top_n: DENSE_TOP_N,
        query_count: rows.len(),
        answerable_query_count: answerable_rows.len(),
        candidate_recall_hits,
        candidate_recall_rate: candidate_recall_hits as f64 / answerable_rows.len() as f64,
        truncation_events,
        total_embed_calls,
        dense_recall_at_10: recall_mean,
        dense_recall_at_10_ci: [recall_ci.low, recall_ci.high],
        dense_ndcg_at_10: ndcg_mean,
        dense_ndcg_at_10_ci: [ndcg_ci.low, ndcg_ci.high],
        delta_recall_vs_b1_frozen: recall_mean - B1_FROZEN_CALIBRATION_RECALL_AT_10,
        delta_ndcg_vs_b1_frozen: ndcg_mean - B1_FROZEN_CALIBRATION_NDCG_AT_10,
        by_category,
        by_difficulty,
        abstention_confusion_matrix: AbstentionConfusion { tp: 0, fp: 0, tn: 0, fn_: abstain_fn },
    };

    println!(
        "{model_name} [{split}]: Recall@10={:.4} (delta={:+.4}) nDCG@10={:.4} (delta={:+.4}) candidate_recall_rate={:.4} ({}/{}) truncation_events={}",
        report.dense_recall_at_10, report.delta_recall_vs_b1_frozen,
        report.dense_ndcg_at_10, report.delta_ndcg_vs_b1_frozen,
        report.candidate_recall_rate, report.candidate_recall_hits, report.answerable_query_count,
        report.truncation_events
    );
    std::fs::create_dir_all(RESULTS_DIR).unwrap();
    let out_path = format!("{RESULTS_DIR}/priority4-calibration-dense-{model_name}.json");
    let json = serde_json::to_string_pretty(&report).unwrap();
    std::fs::write(&out_path, json).unwrap();
    println!("wrote {out_path}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let env = ort::init().build().expect("failed to initialize ONNX Runtime environment");
    match args.get(1).map(String::as_str) {
        Some("--model") => {
            let model_name = args.get(2).cloned().unwrap_or_else(|| {
                panic!("--model requires a name; calibration is authorized for {FROZEN_MODEL:?} only")
            });
            run_dense_calibration(&env, &model_name);
        }
        other => panic!("usage: --model {FROZEN_MODEL}; got {other:?}"),
    }
}
