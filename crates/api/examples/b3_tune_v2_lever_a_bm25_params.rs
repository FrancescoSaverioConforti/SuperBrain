//! B3-TUNE-v2 Priority 2 Lever A: BM25 `k1`/`b` ablation. Two variants:
//! `A-score` (custom params reach only `Index::search`'s final-scoring
//! `Bm25Index`, candidate prefilter stays at defaults — diagnostic) and
//! `A-full` (custom params reach every `Bm25Index` in the real pipeline,
//! including `RealIndexer`'s own persistent candidate-prefilter index —
//! this is what Lever A's KEEP/REJECT decision is actually about). Both
//! are additive, default-preserving: `index::search`'s own frozen
//! behavior never changes for any existing call site. Never reads
//! `eval/rag/test/` or `eval/rag/v2-heldout/` content.

use index::indexer_impl::{rebuild_index, RealIndexer};
use std::collections::{HashMap, HashSet};
use storage::{NoteId, Vault, Version};
use uuid::Uuid;

const CANARY_ROW_ID: &str = "canary-isolation-probe";
const CORPUS_NOTES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/rag/corpus/notes");
const RESULTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/results/B3-TUNE-v2");
const K: usize = 10;
const BOOTSTRAP_SEED: u64 = 0;
const BOOTSTRAP_RESAMPLES: u32 = 2000;

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

struct TestFixture {
    vault: Option<Vault>,
    pii: Option<vault_security::PiiStore>,
    root: std::path::PathBuf,
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        // Custom Drop::drop runs BEFORE the struct's own fields are
        // dropped, so an un-dropped `self.vault`/`self.pii` would still
        // hold their file locks (the vault's SQLite connection, the PII
        // store's key file) when this body runs, making `remove_dir_all`
        // silently fail on Windows. Taking both out of their `Option`
        // and dropping them explicitly, in this order, before touching
        // the filesystem avoids that race entirely rather than retrying
        // against a lock that cannot yet be released.
        drop(self.vault.take());
        drop(self.pii.take());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl TestFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("superbrain-b3-tune-v2-lever-a-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3-TUNE-v2 Lever A", "test@localhost").unwrap();
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
            Vault::open_with_pii_store(root.clone(), &root.join("journal.sqlite"), &pii).unwrap();
        Self { vault: Some(vault), pii: Some(pii), root }
    }

    fn vault(&self) -> &Vault {
        self.vault.as_ref().unwrap()
    }

    fn pii(&self) -> &vault_security::PiiStore {
        self.pii.as_ref().unwrap()
    }

    fn commit_corpus(&self) -> usize {
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
        let mut committed = 0;
        for path in &note_paths {
            let note_text = std::fs::read_to_string(path).unwrap();
            let outcome = storage::propose_and_commit(
                self.vault(),
                &note_text,
                Version(0),
                Uuid::new_v4(),
                &known_ids,
                &HashSet::new(),
                &policy::Gate::new(policy::SecretPolicy::new(1)),
                Some(self.pii()),
            )
            .unwrap();
            assert!(
                matches!(outcome, storage::CommitResult::Committed(_)),
                "corpus note {path:?} failed to commit: {outcome:?}"
            );
            committed += 1;
        }
        committed
    }
}

/// Which `Bm25Index`(es) in the real search pipeline a custom `k1`/`b`
/// reaches. `AScore`: only `Index::search`'s per-query final-scoring
/// index (candidate prefilter stays default). `AFull`: both the
/// candidate prefilter (`RealIndexer`'s own persistent index) and final
/// scoring — the variant Lever A's KEEP/REJECT decision is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BmVariant {
    AScore,
    AFull,
}

fn allow_policy() -> policy::ReadPolicy {
    policy::ReadPolicy::new(true, true)
}

fn request_context() -> api::RequestContext {
    api::RequestContext {
        schema_version: api::SCHEMA_VERSION,
        correlation_id: Uuid::new_v4(),
    }
}

#[derive(Debug, Clone)]
enum QueryOutcome {
    NotPresent,
    Matches { retrieved_note_ids: Vec<String> },
}

fn run_one_query(idx: &index::Index<'_>, question: &str, k: usize) -> QueryOutcome {
    let response = api::search(
        idx,
        &allow_policy(),
        request_context(),
        api::SearchRequest {
            text: question.to_string(),
            filters: Default::default(),
            k,
        },
    );
    let search_response = response.result.unwrap_or_else(|e| {
        panic!("search failed for question {question:?}: {e}");
    });
    match search_response.outcome {
        index::search::SearchOutcome::NotPresent => QueryOutcome::NotPresent,
        index::search::SearchOutcome::Matches { first, rest } => {
            let mut ids: Vec<String> = vec![first.note_id.as_str().to_string()];
            ids.extend(rest.iter().map(|r| r.note_id.as_str().to_string()));
            ids.truncate(k);
            QueryOutcome::Matches { retrieved_note_ids: ids }
        }
    }
}

struct ScoredRow {
    recall_at_10: f64,
    ndcg_at_10: f64,
}

fn score_answerable_row(row: &LabelRow, outcome: &QueryOutcome) -> ScoredRow {
    let empty: Vec<String> = Vec::new();
    let retrieved_note_ids = match outcome {
        QueryOutcome::NotPresent => &empty,
        QueryOutcome::Matches { retrieved_note_ids } => retrieved_note_ids,
    };
    ScoredRow {
        recall_at_10: api::metrics::recall_at_10(&row.relevant_note_ids, retrieved_note_ids),
        ndcg_at_10: api::metrics::ndcg_at_10(retrieved_note_ids, &row.graded_relevance),
    }
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

#[derive(Debug, serde::Serialize)]
struct ConfusionMatrixReport {
    tp: u32,
    tn: u32,
    fp: u32,
    #[serde(rename = "fn")]
    fn_: u32,
}

#[derive(Debug, serde::Serialize)]
struct OptionalMetricReport {
    value: Option<f64>,
    ci: Option<[f64; 2]>,
    valid_resamples: u32,
    total_resamples: u32,
    low_confidence: bool,
}

fn optional_metric_report(result: &api::metrics::OptionalMetricResult) -> OptionalMetricReport {
    OptionalMetricReport {
        value: result.point_estimate,
        ci: result.ci.map(|c| [c.low, c.high]),
        valid_resamples: result.valid_resamples,
        total_resamples: result.total_resamples,
        low_confidence: (result.valid_resamples as usize) < api::metrics::LOW_CONFIDENCE_FLOOR,
    }
}

#[derive(Debug, serde::Serialize)]
struct AbstentionReport {
    confusion_matrix: ConfusionMatrixReport,
    precision: OptionalMetricReport,
    recall: OptionalMetricReport,
    f1: OptionalMetricReport,
    accuracy: OptionalMetricReport,
    n: usize,
}

#[derive(Debug, serde::Serialize)]
struct GridPointReport {
    k1: f64,
    b: f64,
    variant: String,
    split: String,
    run_commit: String,
    labels_sha256: String,
    aggregate: MetricBreakdown,
    abstention: AbstentionReport,
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

/// Builds the real search `Index` for one grid point and variant.
/// `AScore`: plain `RealIndexer::new()` (candidate prefilter stays
/// default), `.with_bm25_params(k1, b)` only on the returned `Index`.
/// `AFull`: `RealIndexer::new_with_bm25_params(k1, b)` AND
/// `.with_bm25_params(k1, b)` on the `Index` — both carry the same
/// params, since `A-full`'s whole point is custom params everywhere in
/// the real pipeline, not just final scoring.
fn build_index_with_bm25_params<'a>(
    fixture: &'a TestFixture,
    indexer: &'a RealIndexer,
    k1: f64,
    b: f64,
) -> index::Index<'a> {
    rebuild_index(fixture.vault(), indexer).unwrap();
    index::Index::new(
        fixture.vault(),
        indexer,
        "eval-b3",
        chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap(),
    )
    .with_bm25_params(k1, b)
}

fn run_split_with_bm25_params(split: &str, k1: f64, b: f64, variant: BmVariant) -> GridPointReport {
    assert!(
        split == "dev" || split == "calibration",
        "Lever A's ablation only ever reads dev or calibration, never test or v2-heldout; got {split:?}"
    );
    let labels_path = format!("{}/../../eval/rag/{split}/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let raw_labels_text = std::fs::read_to_string(&labels_path).unwrap();
    let rows = exclude_canary(parse_label_rows(&raw_labels_text));

    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let indexer = match variant {
        BmVariant::AScore => RealIndexer::new(),
        BmVariant::AFull => RealIndexer::new_with_bm25_params(k1, b),
    };
    let idx = build_index_with_bm25_params(&fixture, &indexer, k1, b);

    let mut recall_values: Vec<f64> = Vec::new();
    let mut ndcg_values: Vec<f64> = Vec::new();
    let mut abstain_expected_flags: Vec<bool> = Vec::new();
    let mut predicted_not_present_flags: Vec<bool> = Vec::new();

    for row in &rows {
        let outcome = run_one_query(&idx, &row.question, K);
        abstain_expected_flags.push(row.abstain_expected);
        predicted_not_present_flags.push(matches!(outcome, QueryOutcome::NotPresent));
        if !row.abstain_expected {
            let scored = score_answerable_row(row, &outcome);
            recall_values.push(scored.recall_at_10);
            ndcg_values.push(scored.ndcg_at_10);
        }
    }

    let aggregate = metrics_breakdown(&recall_values, &ndcg_values, BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);

    let cm_bootstrap = api::metrics::confusion_matrix_bootstrap(
        &abstain_expected_flags,
        &predicted_not_present_flags,
        BOOTSTRAP_SEED,
        BOOTSTRAP_RESAMPLES,
    );
    let abstention = AbstentionReport {
        confusion_matrix: ConfusionMatrixReport {
            tp: cm_bootstrap.confusion_matrix.tp,
            tn: cm_bootstrap.confusion_matrix.tn,
            fp: cm_bootstrap.confusion_matrix.fp,
            fn_: cm_bootstrap.confusion_matrix.fn_,
        },
        precision: optional_metric_report(&cm_bootstrap.precision),
        recall: optional_metric_report(&cm_bootstrap.recall),
        f1: optional_metric_report(&cm_bootstrap.f1),
        accuracy: optional_metric_report(&cm_bootstrap.accuracy),
        n: rows.len(),
    };

    GridPointReport {
        k1,
        b,
        variant: match variant {
            BmVariant::AScore => "A-score".to_string(),
            BmVariant::AFull => "A-full".to_string(),
        },
        split: split.to_string(),
        run_commit: git_rev_parse_head(),
        labels_sha256: sha256_hex_of_file(&labels_path),
        aggregate,
        abstention,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_a_fixture_removes_its_temp_directory() {
        let fixture = TestFixture::new();
        let root = fixture.root.clone();
        assert!(root.exists());
        drop(fixture);
        assert!(!root.exists(), "TestFixture's temp directory must be removed on drop: {root:?}");
    }

    #[test]
    fn grid_point_runs_against_the_real_corpus_and_returns_metrics_for_both_modes() {
        let score_report = run_split_with_bm25_params("dev", 1.2, 0.75, BmVariant::AScore);
        let full_report = run_split_with_bm25_params("dev", 1.2, 0.75, BmVariant::AFull);
        assert_eq!(score_report.k1, 1.2);
        assert_eq!(score_report.b, 0.75);
        assert!(score_report.aggregate.recall_at_10 > 0.0);
        assert!(full_report.aggregate.recall_at_10 > 0.0);
        // At the current defaults, A-score and A-full must agree exactly --
        // both are just B1 itself when k1/b match the hardcoded defaults.
        assert_eq!(score_report.aggregate.recall_at_10, full_report.aggregate.recall_at_10);
    }
}

const K1_GRID: &[f64] = &[0.9, 1.2, 1.5, 1.8];
const B_GRID: &[f64] = &[0.4, 0.6, 0.75, 0.9];

/// The selected grid point (Task 3 Step 1 — frozen rule applied to the
/// grid: max dev Recall@10, tie-break max dev nDCG@10, tie-break min
/// distance to defaults; `(1.8, 0.75)` won uniquely on nDCG@10, no
/// further tie-break needed). See `priority2-lever-a-selection.md`.
const SELECTED_K1: f64 = 1.8;
const SELECTED_B: f64 = 0.75;

#[derive(Debug, serde::Serialize)]
struct AblationReport {
    split: String,
    selected_k1: f64,
    selected_b: f64,
    a_score: GridPointReport,
    a_full: GridPointReport,
    /// Candidate-set-drift signal: if `A-full`'s own retrieved-note
    /// count pattern differs from `A-score`'s because the candidate
    /// prefilter itself changed membership, that is expected signal,
    /// not an error — reported explicitly here as the delta in mean
    /// retrieved-candidate behavior via each variant's own aggregate
    /// Recall@10 (a difference in aggregate Recall@10 between A-score
    /// and A-full at the SAME k1/b can only come from candidate-set
    /// membership changing, since final re-ranking alone cannot change
    /// which notes are present when a note's own score stays the same).
    a_full_vs_a_score_recall_at_10_delta: f64,
}

fn run_ablation_for_split(split: &str, k1: f64, b: f64) -> AblationReport {
    let a_score = run_split_with_bm25_params(split, k1, b, BmVariant::AScore);
    let a_full = run_split_with_bm25_params(split, k1, b, BmVariant::AFull);
    let delta = a_full.aggregate.recall_at_10 - a_score.aggregate.recall_at_10;
    AblationReport {
        split: split.to_string(),
        selected_k1: k1,
        selected_b: b,
        a_score,
        a_full,
        a_full_vs_a_score_recall_at_10_delta: delta,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--ablate") {
        let split = args.get(2).cloned().unwrap_or_else(|| panic!("--ablate requires a split (dev|calibration)"));
        let report = run_ablation_for_split(&split, SELECTED_K1, SELECTED_B);
        println!(
            "{split}: A-score Recall@10={:.4} nDCG@10={:.4} | A-full Recall@10={:.4} nDCG@10={:.4} | candidate-set-drift(Recall@10 delta)={:+.4}",
            report.a_score.aggregate.recall_at_10, report.a_score.aggregate.ndcg_at_10,
            report.a_full.aggregate.recall_at_10, report.a_full.aggregate.ndcg_at_10,
            report.a_full_vs_a_score_recall_at_10_delta
        );
        std::fs::create_dir_all(RESULTS_DIR).unwrap();
        let out_path = format!("{RESULTS_DIR}/priority2-lever-a-{split}-ablation.json");
        let json = serde_json::to_string_pretty(&report).unwrap();
        std::fs::write(&out_path, json).unwrap();
        println!("wrote {out_path}");
        return;
    }

    let mut reports = Vec::with_capacity(K1_GRID.len() * B_GRID.len());
    for &k1 in K1_GRID {
        for &b in B_GRID {
            let report = run_split_with_bm25_params("dev", k1, b, BmVariant::AScore);
            println!(
                "k1={k1:.2} b={b:.2} Recall@10={:.4} nDCG@10={:.4}",
                report.aggregate.recall_at_10, report.aggregate.ndcg_at_10
            );
            reports.push(report);
        }
    }

    std::fs::create_dir_all(RESULTS_DIR).unwrap();
    let out_path = format!("{RESULTS_DIR}/priority2-lever-a-grid.json");
    let json = serde_json::to_string_pretty(&reports).unwrap();
    std::fs::write(&out_path, json).unwrap();
    println!("wrote {out_path}");
}
