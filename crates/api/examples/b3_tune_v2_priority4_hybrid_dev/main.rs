//! B3-TUNE-v2 Priority 4 Stage 1, Task 5: lexical+dense RRF hybrid DEV
//! harness, using the single model locked in by Task 4 and the frozen
//! hyperparameters from
//! `docs/superpowers/specs/2026-11-14-b3-tune-v2-priority4-stage1-dev-freeze.md`
//! Section 5. DEV only -- never calibration, never held-out. Does not
//! modify B1's production `index`/`search` pipeline; B1's own ranked
//! list is obtained through the existing public `api::search`.
//!
//! `--model <name>` runs the hybrid DEV benchmark for one model.

#[path = "../priority4_common/embedding.rs"]
mod embedding;

use embedding::{cosine_similarity, EmbeddingModel, ModelSpec, MODEL_SPECS};
use index::document::Document;
use index::indexer_impl::{rebuild_index, RealIndexer};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use storage::{NoteId, Vault, Version};
use uuid::Uuid;

const CORPUS_NOTES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/rag/corpus/notes");
const RESULTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/results/B3-TUNE-v2");
const LEXICAL_TOP_N: usize = 50;
const DENSE_TOP_N: usize = 50;
const FINAL_TOP_K: usize = 10;
const RRF_K: f64 = 60.0;
const BOOTSTRAP_SEED: u64 = 0;
const BOOTSTRAP_RESAMPLES: u32 = 2000;
const CANARY_ROW_ID: &str = "canary-isolation-probe";

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
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("malformed label row: {e}")))
        .collect()
}

fn exclude_canary(rows: Vec<LabelRow>) -> Vec<LabelRow> {
    rows.into_iter().filter(|r| r.id != CANARY_ROW_ID).collect()
}

struct VaultFixture {
    vault: Option<Vault>,
    pii: Option<vault_security::PiiStore>,
    indexer: Arc<RealIndexer>,
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
        let root = std::env::temp_dir().join(format!("superbrain-b3-tune-v2-p4-hybrid-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3-TUNE-v2 Priority 4 Hybrid", "test@localhost").unwrap();
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
        Self { vault: Some(vault), pii: Some(pii), indexer: Arc::new(RealIndexer::new()), root }
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
        let known_ids: HashSet<NoteId> = note_paths
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
                &HashSet::new(), &policy::Gate::new(policy::SecretPolicy::new(1)), Some(self.pii()),
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

    fn build_index(&self) -> index::Index<'_> {
        rebuild_index(self.vault(), &self.indexer).unwrap();
        index::Index::new(
            self.vault(), &self.indexer, "eval-b3",
            chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap(),
        )
    }
}

fn allow_policy() -> policy::ReadPolicy {
    policy::ReadPolicy::new(true, true)
}

fn request_context() -> api::RequestContext {
    api::RequestContext { schema_version: api::SCHEMA_VERSION, correlation_id: Uuid::new_v4() }
}

/// B1's own ranked lexical result list for one query, score-ordered,
/// exactly as `search_impl` produces it (never re-implemented here --
/// `candidate_ids`/the internal sort are `pub(crate)` to `index`, so
/// this harness goes through the real public `api::search` instead,
/// same discipline Priority 2/3's own ablations follow).
fn lexical_ranked_ids(idx: &index::Index<'_>, question: &str) -> Vec<String> {
    let response = api::search(
        idx, &allow_policy(), request_context(),
        api::SearchRequest { text: question.to_string(), filters: Default::default(), k: LEXICAL_TOP_N },
    );
    match response.result.unwrap().outcome {
        index::search::SearchOutcome::NotPresent => Vec::new(),
        index::search::SearchOutcome::Matches { first, rest } => {
            let mut ids = vec![first.note_id.as_str().to_string()];
            ids.extend(rest.into_iter().map(|r| r.note_id.as_str().to_string()));
            ids
        }
    }
}

/// Reciprocal Rank Fusion (frozen spec Section 5): `score(d) = sum of
/// 1/(k + rank_i(d))` over each list the document appears in, ranks
/// 1-indexed. A document absent from a list contributes nothing for
/// that list -- never treated as a zero-rank term.
fn rrf_fuse(lexical_ids: &[String], dense_ids: &[String], k: f64) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for (i, id) in lexical_ids.iter().enumerate() {
        let rank = (i + 1) as f64;
        *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + rank);
    }
    for (i, id) in dense_ids.iter().enumerate() {
        let rank = (i + 1) as f64;
        *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + rank);
    }
    let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
    // Tie-break: B1's existing note-id lexicographic ascending order
    // (frozen spec Section 5), applied only when fused scores are
    // bitwise equal.
    fused.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    fused
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
        recall_at_10: recall_mean, ndcg_at_10: ndcg_mean,
        recall_ci: [recall_ci.low, recall_ci.high], ndcg_ci: [ndcg_ci.low, ndcg_ci.high],
        n, low_confidence: n < api::metrics::LOW_CONFIDENCE_FLOOR,
    }
}

fn breakdown_by<F: Fn(&LabelRow) -> String>(
    answerable_rows: &[&LabelRow], recall_values: &[f64], ndcg_values: &[f64], key_fn: F,
    seed: u64, resamples: u32,
) -> HashMap<String, MetricBreakdown> {
    let mut by_key: HashMap<String, (Vec<f64>, Vec<f64>)> = HashMap::new();
    for (i, row) in answerable_rows.iter().enumerate() {
        let key = key_fn(row);
        let entry = by_key.entry(key).or_default();
        entry.0.push(recall_values[i]);
        entry.1.push(ndcg_values[i]);
    }
    by_key.into_iter().map(|(key, (r, n))| (key, metrics_breakdown(&r, &n, seed, resamples))).collect()
}

#[derive(Debug, serde::Serialize)]
struct ThreeWayComparisonReport {
    model: String,
    split: String,
    run_commit: String,
    labels_sha256: String,
    query_count: usize,
    answerable_query_count: usize,
    b1_lexical_only: MetricBreakdown,
    dense_standalone: MetricBreakdown,
    hybrid_rrf: MetricBreakdown,
    hybrid_by_category: HashMap<String, MetricBreakdown>,
    hybrid_by_difficulty: HashMap<String, MetricBreakdown>,
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
    let bytes = std::fs::read(path).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("{:x}", hasher.finalize())
}

fn run_hybrid_benchmark(env: &ort::environment::Environment, model_name: &str) {
    let labels_path = format!("{}/../../eval/rag/dev/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let raw_labels_text = std::fs::read_to_string(&labels_path).unwrap();
    let rows = exclude_canary(parse_label_rows(&raw_labels_text));

    let fixture = VaultFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index();
    let documents = fixture.all_documents();

    let spec: ModelSpec = MODEL_SPECS.into_iter().find(|s| s.name == model_name).unwrap();
    let mut model = EmbeddingModel::load(env, spec);

    eprintln!("[{model_name}] embedding {} corpus notes for hybrid run...", documents.len());
    let note_vectors: Vec<(String, Vec<f32>)> = documents
        .iter()
        .map(|(id, text)| (id.clone(), model.embed_document(text).vector))
        .collect();

    let mut lexical_recall: Vec<f64> = Vec::new();
    let mut lexical_ndcg: Vec<f64> = Vec::new();
    let mut dense_recall: Vec<f64> = Vec::new();
    let mut dense_ndcg: Vec<f64> = Vec::new();
    let mut hybrid_recall: Vec<f64> = Vec::new();
    let mut hybrid_ndcg: Vec<f64> = Vec::new();
    let mut answerable_rows: Vec<&LabelRow> = Vec::new();

    for row in &rows {
        if row.abstain_expected {
            continue;
        }

        let lexical_ids = lexical_ranked_ids(&idx, &row.question);
        let lexical_top_k: Vec<String> = lexical_ids.iter().take(FINAL_TOP_K).cloned().collect();
        lexical_recall.push(api::metrics::recall_at_10(&row.relevant_note_ids, &lexical_top_k));
        lexical_ndcg.push(api::metrics::ndcg_at_10(&lexical_top_k, &row.graded_relevance));

        let qv = model.embed_query(&row.question).vector;
        let mut scored: Vec<(String, f32)> = note_vectors
            .iter().map(|(id, v)| (id.clone(), cosine_similarity(&qv, v))).collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(DENSE_TOP_N);
        let dense_ids: Vec<String> = scored.iter().map(|(id, _)| id.clone()).collect();
        let dense_top_k: Vec<String> = dense_ids.iter().take(FINAL_TOP_K).cloned().collect();
        dense_recall.push(api::metrics::recall_at_10(&row.relevant_note_ids, &dense_top_k));
        dense_ndcg.push(api::metrics::ndcg_at_10(&dense_top_k, &row.graded_relevance));

        let fused = rrf_fuse(&lexical_ids, &dense_ids, RRF_K);
        let hybrid_top_k: Vec<String> = fused.iter().take(FINAL_TOP_K).map(|(id, _)| id.clone()).collect();
        hybrid_recall.push(api::metrics::recall_at_10(&row.relevant_note_ids, &hybrid_top_k));
        hybrid_ndcg.push(api::metrics::ndcg_at_10(&hybrid_top_k, &row.graded_relevance));

        answerable_rows.push(row);
    }

    let b1_lexical_only = metrics_breakdown(&lexical_recall, &lexical_ndcg, BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);
    let dense_standalone = metrics_breakdown(&dense_recall, &dense_ndcg, BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);
    let hybrid_rrf = metrics_breakdown(&hybrid_recall, &hybrid_ndcg, BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);
    let hybrid_by_category = breakdown_by(&answerable_rows, &hybrid_recall, &hybrid_ndcg, |r| r.category.clone(), BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);
    let hybrid_by_difficulty = breakdown_by(&answerable_rows, &hybrid_recall, &hybrid_ndcg, |r| r.difficulty.clone(), BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);

    let report = ThreeWayComparisonReport {
        model: model_name.to_string(),
        split: "dev".to_string(),
        run_commit: git_rev_parse_head(),
        labels_sha256: sha256_hex_of_file(&labels_path),
        query_count: rows.len(),
        answerable_query_count: answerable_rows.len(),
        b1_lexical_only, dense_standalone, hybrid_rrf,
        hybrid_by_category, hybrid_by_difficulty,
    };

    println!(
        "{model_name} [dev, n={}]: B1-lexical Recall@10={:.4}/nDCG@10={:.4}  dense-standalone Recall@10={:.4}/nDCG@10={:.4}  hybrid-RRF Recall@10={:.4}/nDCG@10={:.4}",
        report.answerable_query_count,
        report.b1_lexical_only.recall_at_10, report.b1_lexical_only.ndcg_at_10,
        report.dense_standalone.recall_at_10, report.dense_standalone.ndcg_at_10,
        report.hybrid_rrf.recall_at_10, report.hybrid_rrf.ndcg_at_10,
    );
    std::fs::create_dir_all(RESULTS_DIR).unwrap();
    let out_path = format!("{RESULTS_DIR}/priority4-hybrid-dev-{model_name}.json");
    std::fs::write(&out_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("wrote {out_path}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let env = ort::init().build().expect("failed to initialize ONNX Runtime environment");
    match args.get(1).map(String::as_str) {
        Some("--model") => {
            let model_name = args.get(2).cloned().unwrap_or_else(|| panic!("--model requires a name"));
            run_hybrid_benchmark(&env, &model_name);
        }
        other => panic!("usage: --model <name>; got {other:?}"),
    }
}
