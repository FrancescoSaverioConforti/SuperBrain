//! B3.3 metric runner: calls the real `api::search` / `index::search`
//! retrieval path against the frozen B3 corpus and computes Recall@10,
//! nDCG@10, confidence intervals, and category/difficulty/abstention
//! breakdowns for the `dev` or `calibration` split. Never reads
//! `eval/rag/test/` for its content. Writes a JSON report per run to
//! `eval/results/B3/<split>-<timestamp>-<commit>.json`.

use index::indexer_impl::{rebuild_index, RealIndexer};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use storage::{NoteId, Vault, Version};
use uuid::Uuid;

const CANARY_ROW_ID: &str = "canary-isolation-probe";
const CORPUS_NOTES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/rag/corpus/notes");

#[derive(Debug, Clone, serde::Deserialize)]
struct LabelRow {
    id: String,
    question: String,
    // Parsed for schema completeness; the runner never scores on answer
    // text or citation content — that remains eval/rag/validate_corpus.py's job.
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

/// Parses a `labels.jsonl` file's raw text into rows. Panics with a clear
/// message on a malformed line — this reads this project's own committed
/// label files, not an external boundary needing graceful degradation.
fn parse_label_rows(text: &str) -> Vec<LabelRow> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("malformed label row: {e}\nline: {line}"))
        })
        .collect()
}

/// Excludes the mechanical isolation canary row by id, before any metric
/// computation sees it.
fn exclude_canary(rows: Vec<LabelRow>) -> Vec<LabelRow> {
    rows.into_iter().filter(|r| r.id != CANARY_ROW_ID).collect()
}

const KNOWN_CATEGORIES: &[&str] = &[
    "notes-decisions",
    "bugs-fixes",
    "patterns-knowledge",
    "temporal",
    "identifier-code",
    "cross-file-concept",
];
const KNOWN_DIFFICULTIES: &[&str] = &["easy", "medium", "hard"];

/// Fail-fast label invariant validation: the mathematical preconditions
/// `metrics.rs`'s functions need to be meaningful. This deliberately
/// mirrors only the subset of `eval/rag/runner.py::_validate_extended_schema`'s
/// rules that matter here — citation span content/provenance checks stay
/// Python's job, out of scope for this runner.
fn validate_label_rows(rows: &[LabelRow]) -> Result<(), String> {
    for row in rows {
        if !KNOWN_CATEGORIES.contains(&row.category.as_str()) {
            return Err(format!("row {:?}: unknown category {:?}", row.id, row.category));
        }
        if !KNOWN_DIFFICULTIES.contains(&row.difficulty.as_str()) {
            return Err(format!("row {:?}: unknown difficulty {:?}", row.id, row.difficulty));
        }
        if row.abstain_expected {
            if !row.relevant_note_ids.is_empty() {
                return Err(format!(
                    "row {:?}: abstain_expected but relevant_note_ids is nonempty",
                    row.id
                ));
            }
            if !row.graded_relevance.is_empty() {
                return Err(format!(
                    "row {:?}: abstain_expected but graded_relevance is nonempty",
                    row.id
                ));
            }
        } else {
            if row.relevant_note_ids.is_empty() {
                return Err(format!(
                    "row {:?}: answerable row has empty relevant_note_ids",
                    row.id
                ));
            }
            let relevant_set: std::collections::HashSet<&str> =
                row.relevant_note_ids.iter().map(String::as_str).collect();
            let graded_set: std::collections::HashSet<&str> =
                row.graded_relevance.keys().map(String::as_str).collect();
            if relevant_set != graded_set {
                return Err(format!(
                    "row {:?}: graded_relevance keys {:?} do not match relevant_note_ids {:?}",
                    row.id, graded_set, relevant_set
                ));
            }
        }
        for (note_id, grade) in &row.graded_relevance {
            if !(0..=2).contains(grade) {
                return Err(format!(
                    "row {:?}: graded_relevance[{:?}] = {} is outside 0..=2",
                    row.id, note_id, grade
                ));
            }
        }
    }
    Ok(())
}

struct TestFixture {
    vault: Vault,
    pii: vault_security::PiiStore,
    indexer: Arc<RealIndexer>,
    #[allow(dead_code)]
    root: std::path::PathBuf,
}

impl TestFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("superbrain-b3-metric-runner-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3 Metric Runner", "test@localhost").unwrap();
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
        Self {
            vault,
            pii,
            indexer: Arc::new(RealIndexer::new()),
            root,
        }
    }

    /// Commits every real corpus note under `eval/rag/corpus/notes/`
    /// through the real gate/vault path. The full set of corpus note ids
    /// is supplied as `known_ids` up front — some corpus notes cite other
    /// corpus notes via `relations` (`derived_from`/`supersedes`), and
    /// `std::fs::read_dir` gives no commit-order guarantee, so a forward
    /// reference must not look like a `BrokenLink` (the same fix B3.1/B3.2's
    /// Task 7 already found necessary for `held_out_labels`).
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
                &self.vault,
                &note_text,
                Version(0),
                Uuid::new_v4(),
                &known_ids,
                &HashSet::new(),
                &policy::Gate::new(policy::SecretPolicy::new(1)),
                Some(&self.pii),
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

    fn build_index(&self) -> index::Index<'_> {
        rebuild_index(&self.vault, &self.indexer).unwrap();
        // A fixed reference date, same convention `held_out_labels` uses for
        // this corpus — only relevant to relative-time-expression resolution,
        // not to this runner's scoring, which never uses a temporal filter.
        index::Index::new(
            &self.vault,
            &self.indexer,
            "eval-b3",
            chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap(),
        )
    }
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

/// One query's search outcome, reduced to just the ordered note id
/// strings `metrics.rs`'s pure functions need — never the full
/// `index::search::SearchResult`, so this enum stays decoupled from
/// `index`'s own types.
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

const RETRIEVAL_CONFIG: &str = "B1 deterministic trigram+BM25, no embeddings";

#[derive(Debug, serde::Serialize)]
struct MetricBreakdown {
    recall_at_10: f64,
    ndcg_at_10: f64,
    recall_ci: [f64; 2],
    ndcg_ci: [f64; 2],
    n: usize,
    low_confidence: bool,
}

#[derive(Debug, serde::Serialize)]
struct OptionalMetricReport {
    value: Option<f64>,
    ci: Option<[f64; 2]>,
    valid_resamples: u32,
    total_resamples: u32,
    low_confidence: bool,
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
struct ConfusionMatrixReport {
    tp: u32,
    tn: u32,
    fp: u32,
    #[serde(rename = "fn")]
    fn_: u32,
}

#[derive(Debug, serde::Serialize)]
struct PerQueryReport {
    id: String,
    category: String,
    difficulty: String,
    abstain_expected: bool,
    outcome: String,
    recall_at_10: Option<f64>,
    hit_at_10: Option<bool>,
    ndcg_at_10: Option<f64>,
}

#[derive(Debug, serde::Serialize)]
struct Report {
    split: String,
    run_date: String,
    run_timestamp_utc: String,
    runner_commit: String,
    retrieval_config: String,
    corpus_manifest_sha256: String,
    labels_sha256: String,
    bootstrap_seed: u64,
    bootstrap_resamples: u32,
    corpus_manifest_note_count: usize,
    query_count: usize,
    query_count_excluding_canary: usize,
    aggregate: MetricBreakdown,
    by_category: std::collections::BTreeMap<String, MetricBreakdown>,
    by_difficulty: std::collections::BTreeMap<String, MetricBreakdown>,
    abstention: AbstentionReport,
    per_query: Vec<PerQueryReport>,
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

fn metrics_breakdown(
    recall_values: &[f64],
    ndcg_values: &[f64],
    seed: u64,
    resamples: u32,
) -> MetricBreakdown {
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

fn optional_metric_report(result: &api::metrics::OptionalMetricResult) -> OptionalMetricReport {
    OptionalMetricReport {
        value: result.point_estimate,
        ci: result.ci.map(|c| [c.low, c.high]),
        valid_resamples: result.valid_resamples,
        total_resamples: result.total_resamples,
        low_confidence: (result.valid_resamples as usize) < api::metrics::LOW_CONFIDENCE_FLOOR,
    }
}

/// An answerable row's Recall@10/Hit@10/nDCG@10, scored against whatever
/// `index::search` actually returned. `QueryOutcome::NotPresent` scores
/// exactly like an empty retrieved list — Recall@10 = 0.0, Hit@10 =
/// false, nDCG@10 = 0.0 — and must be pushed into aggregation like any
/// other answerable row, never dropped as if it had no score (final
/// review Important #1: dropping it would let a retrieval regression
/// that starts returning NotPresent on real answers silently INFLATE
/// Recall@10/nDCG@10 instead of tanking them).
struct ScoredRow {
    recall_at_10: f64,
    hit_at_10: bool,
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
        hit_at_10: api::metrics::hit_at_10(&row.relevant_note_ids, retrieved_note_ids),
        ndcg_at_10: api::metrics::ndcg_at_10(retrieved_note_ids, &row.graded_relevance),
    }
}

fn run_split(split: &str, seed: u64, resamples: u32) -> Report {
    assert_ne!(split, "test", "B3.3/B3.4 never read the held-out test split's content");

    let labels_path = format!(
        "{}/../../eval/rag/{}/labels.jsonl",
        env!("CARGO_MANIFEST_DIR"),
        split
    );
    let manifest_path = format!("{}/../../eval/rag/corpus-manifest.json", env!("CARGO_MANIFEST_DIR"));

    let raw_labels_text = std::fs::read_to_string(&labels_path).unwrap();
    let all_rows = parse_label_rows(&raw_labels_text);
    let query_count = all_rows.len();
    let rows = exclude_canary(all_rows);
    let query_count_excluding_canary = rows.len();
    if let Err(e) = validate_label_rows(&rows) {
        panic!("label invariant violation: {e}");
    }

    let manifest_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let corpus_manifest_note_count = manifest_json["note_count_total"].as_u64().unwrap() as usize;

    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index();

    let mut recall_values: Vec<f64> = Vec::new();
    let mut ndcg_values: Vec<f64> = Vec::new();
    let mut by_category_recall: HashMap<String, Vec<f64>> = HashMap::new();
    let mut by_category_ndcg: HashMap<String, Vec<f64>> = HashMap::new();
    let mut by_difficulty_recall: HashMap<String, Vec<f64>> = HashMap::new();
    let mut by_difficulty_ndcg: HashMap<String, Vec<f64>> = HashMap::new();
    let mut abstain_expected_flags: Vec<bool> = Vec::new();
    let mut predicted_not_present_flags: Vec<bool> = Vec::new();
    let mut per_query: Vec<PerQueryReport> = Vec::new();

    for row in &rows {
        let outcome = run_one_query(&idx, &row.question, 10);
        abstain_expected_flags.push(row.abstain_expected);
        predicted_not_present_flags.push(matches!(outcome, QueryOutcome::NotPresent));

        let outcome_str = match &outcome {
            QueryOutcome::NotPresent => "NotPresent".to_string(),
            QueryOutcome::Matches { .. } => "Matches".to_string(),
        };
        let (recall, hit, ndcg) = if row.abstain_expected {
            (None, None, None)
        } else {
            // Scored for EVERY answerable row regardless of outcome —
            // NotPresent scores 0.0/false/0.0 via score_answerable_row,
            // never dropped from aggregation (final review Important #1).
            let scored = score_answerable_row(row, &outcome);
            recall_values.push(scored.recall_at_10);
            ndcg_values.push(scored.ndcg_at_10);
            by_category_recall.entry(row.category.clone()).or_default().push(scored.recall_at_10);
            by_category_ndcg.entry(row.category.clone()).or_default().push(scored.ndcg_at_10);
            by_difficulty_recall.entry(row.difficulty.clone()).or_default().push(scored.recall_at_10);
            by_difficulty_ndcg.entry(row.difficulty.clone()).or_default().push(scored.ndcg_at_10);
            (Some(scored.recall_at_10), Some(scored.hit_at_10), Some(scored.ndcg_at_10))
        };

        per_query.push(PerQueryReport {
            id: row.id.clone(),
            category: row.category.clone(),
            difficulty: row.difficulty.clone(),
            abstain_expected: row.abstain_expected,
            outcome: outcome_str,
            recall_at_10: recall,
            hit_at_10: hit,
            ndcg_at_10: ndcg,
        });
    }

    let aggregate = metrics_breakdown(&recall_values, &ndcg_values, seed, resamples);

    let mut by_category = std::collections::BTreeMap::new();
    for category in by_category_recall.keys().cloned().collect::<Vec<_>>() {
        let recall_vals = &by_category_recall[&category];
        let ndcg_vals = &by_category_ndcg[&category];
        by_category.insert(category.clone(), metrics_breakdown(recall_vals, ndcg_vals, seed, resamples));
    }

    let mut by_difficulty = std::collections::BTreeMap::new();
    for difficulty in by_difficulty_recall.keys().cloned().collect::<Vec<_>>() {
        let recall_vals = &by_difficulty_recall[&difficulty];
        let ndcg_vals = &by_difficulty_ndcg[&difficulty];
        by_difficulty.insert(
            difficulty.clone(),
            metrics_breakdown(recall_vals, ndcg_vals, seed, resamples),
        );
    }

    let cm_bootstrap = api::metrics::confusion_matrix_bootstrap(
        &abstain_expected_flags,
        &predicted_not_present_flags,
        seed,
        resamples,
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
        n: query_count_excluding_canary,
    };

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let now = chrono::DateTime::<chrono::Utc>::from_timestamp(now_secs as i64, 0).unwrap();
    let run_date = now.format("%Y-%m-%d").to_string();
    let run_timestamp_utc = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    Report {
        split: split.to_string(),
        run_date,
        run_timestamp_utc,
        runner_commit: git_rev_parse_head(),
        retrieval_config: RETRIEVAL_CONFIG.to_string(),
        corpus_manifest_sha256: sha256_hex_of_file(&manifest_path),
        labels_sha256: sha256_hex_of_file(&labels_path),
        bootstrap_seed: seed,
        bootstrap_resamples: resamples,
        corpus_manifest_note_count,
        query_count,
        query_count_excluding_canary,
        aggregate,
        by_category,
        by_difficulty,
        abstention,
        per_query,
    }
}

const BOOTSTRAP_SEED: u64 = 0;
const BOOTSTRAP_RESAMPLES: u32 = 2000;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let split = args.get(1).map(String::as_str).unwrap_or_else(|| {
        panic!("usage: b3_metric_runner <dev|calibration>");
    });
    if split == "test" {
        panic!(
            "refusing to run against the held-out test split: B3.3/B3.4 never read \
             eval/rag/test/labels.jsonl content (Global Constraints). Final-test runs \
             require separate authorization (B3.5)."
        );
    }
    if split != "dev" && split != "calibration" {
        panic!("usage: b3_metric_runner <dev|calibration>, got {split:?}");
    }

    let report = run_split(split, BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);

    let out_dir = format!("{}/../../eval/results/B3", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&out_dir).unwrap();
    let short_commit = &report.runner_commit[..7.min(report.runner_commit.len())];
    let filename_timestamp = report.run_timestamp_utc.replace([':', '-'], "");
    let out_path = format!("{out_dir}/{split}-{filename_timestamp}-{short_commit}.json");
    let json = serde_json::to_string_pretty(&report).unwrap();
    std::fs::write(&out_path, json).unwrap();
    println!("wrote {out_path}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_split_labels_excludes_the_canary_row_by_id() {
        let jsonl = r#"{"id":"canary-isolation-probe","question":"HELDOUT_QUESTION_CANARY_901","answer":"HELDOUT_ANSWER_CANARY_902","category":"temporal","difficulty":"easy","abstain_expected":true,"relevant_note_ids":[],"graded_relevance":{},"citation_spans":[]}
{"id":"q001","question":"real question","answer":"real answer","category":"bugs-fixes","difficulty":"medium","abstain_expected":false,"relevant_note_ids":["note-a"],"graded_relevance":{"note-a":2},"citation_spans":[]}
"#;
        let rows = parse_label_rows(jsonl);
        let filtered = exclude_canary(rows);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "q001");
    }

    fn make_row(
        abstain_expected: bool,
        category: &str,
        difficulty: &str,
        relevant_note_ids: Vec<&str>,
        graded_relevance: HashMap<&str, u8>,
    ) -> LabelRow {
        LabelRow {
            id: "q1".into(),
            question: "x".into(),
            answer: if abstain_expected { "".into() } else { "y".into() },
            category: category.into(),
            difficulty: difficulty.into(),
            abstain_expected,
            relevant_note_ids: relevant_note_ids.into_iter().map(String::from).collect(),
            graded_relevance: graded_relevance
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            citation_spans: vec![],
        }
    }

    #[test]
    fn validate_label_rows_rejects_answerable_row_with_empty_relevant_note_ids() {
        let row = make_row(false, "bugs-fixes", "easy", vec![], HashMap::new());
        assert!(validate_label_rows(&[row]).is_err());
    }

    #[test]
    fn validate_label_rows_rejects_abstain_row_with_nonempty_relevant_note_ids() {
        let row = make_row(true, "bugs-fixes", "easy", vec!["note-a"], HashMap::new());
        assert!(validate_label_rows(&[row]).is_err());
    }

    #[test]
    fn validate_label_rows_rejects_graded_relevance_keys_not_matching_relevant_note_ids() {
        let row = make_row(
            false,
            "bugs-fixes",
            "easy",
            vec!["note-a"],
            HashMap::from([("note-b", 2u8)]),
        );
        assert!(validate_label_rows(&[row]).is_err());
    }

    #[test]
    fn validate_label_rows_rejects_grade_outside_zero_one_two() {
        let row = make_row(
            false,
            "bugs-fixes",
            "easy",
            vec!["note-a"],
            HashMap::from([("note-a", 5u8)]),
        );
        assert!(validate_label_rows(&[row]).is_err());
    }

    #[test]
    fn validate_label_rows_rejects_unknown_category() {
        let row = make_row(
            false,
            "not-a-real-category",
            "easy",
            vec!["note-a"],
            HashMap::from([("note-a", 2u8)]),
        );
        assert!(validate_label_rows(&[row]).is_err());
    }

    #[test]
    fn validate_label_rows_rejects_unknown_difficulty() {
        let row = make_row(
            false,
            "bugs-fixes",
            "impossible",
            vec!["note-a"],
            HashMap::from([("note-a", 2u8)]),
        );
        assert!(validate_label_rows(&[row]).is_err());
    }

    #[test]
    fn validate_label_rows_accepts_a_well_formed_row() {
        let row = make_row(
            false,
            "bugs-fixes",
            "easy",
            vec!["note-a"],
            HashMap::from([("note-a", 2u8)]),
        );
        assert!(validate_label_rows(&[row]).is_ok());
    }

    #[test]
    fn validate_label_rows_accepts_a_well_formed_abstain_row() {
        let row = make_row(true, "temporal", "easy", vec![], HashMap::new());
        assert!(validate_label_rows(&[row]).is_ok());
    }

    #[test]
    fn builds_a_vault_and_commits_every_real_corpus_note() {
        let fixture = TestFixture::new();
        let committed = fixture.commit_corpus();
        assert!(committed >= 53, "expected at least the 53 known corpus notes, got {committed}");
    }

    #[test]
    fn searches_a_real_corpus_query_and_records_its_outcome() {
        let fixture = TestFixture::new();
        fixture.commit_corpus();
        let idx = fixture.build_index();
        let outcome = run_one_query(&idx, "resolve_configured_timezone", 10);
        assert!(matches!(outcome, QueryOutcome::Matches { .. }));
    }

    #[test]
    fn an_unanswerable_query_returns_not_present() {
        let fixture = TestFixture::new();
        fixture.commit_corpus();
        let idx = fixture.build_index();
        let outcome = run_one_query(&idx, "ZZZ_NO_SUCH_TERM_EVER_QQQ", 10);
        assert!(matches!(outcome, QueryOutcome::NotPresent));
    }

    #[test]
    fn score_answerable_row_scores_not_present_as_zero_not_excluded() {
        // Final review Important #1: an answerable query that comes back
        // NotPresent must score exactly like "nothing relevant retrieved"
        // (Recall@10 = 0.0, Hit@10 = false, nDCG@10 = 0.0), never be
        // dropped from aggregation as if it had no score at all.
        let row = make_row(false, "bugs-fixes", "easy", vec!["note-a"], HashMap::from([("note-a", 2u8)]));
        let scored = score_answerable_row(&row, &QueryOutcome::NotPresent);
        assert_eq!(scored.recall_at_10, 0.0);
        assert!(!scored.hit_at_10);
        assert_eq!(scored.ndcg_at_10, 0.0);
    }

    #[test]
    fn score_answerable_row_scores_matches_via_the_real_metric_functions() {
        let row = make_row(false, "bugs-fixes", "easy", vec!["note-a"], HashMap::from([("note-a", 2u8)]));
        let outcome = QueryOutcome::Matches {
            retrieved_note_ids: vec!["note-a".to_string()],
        };
        let scored = score_answerable_row(&row, &outcome);
        assert_eq!(scored.recall_at_10, 1.0);
        assert!(scored.hit_at_10);
        assert_eq!(scored.ndcg_at_10, 1.0);
    }

    #[test]
    fn a_full_dev_split_run_produces_a_report_with_every_required_field() {
        let report = run_split("dev", 0, 200);
        assert_eq!(report.split, "dev");
        assert!(report.query_count_excluding_canary > 0);
        assert!(report.by_category.contains_key("bugs-fixes"));
        assert!(report.by_difficulty.contains_key("easy"));
        let cm = &report.abstention.confusion_matrix;
        assert_eq!(
            cm.tp + cm.tn + cm.fp + cm.fn_,
            report.query_count_excluding_canary as u32
        );
        assert!(!report.corpus_manifest_sha256.is_empty());
        assert!(!report.labels_sha256.is_empty());
        assert_eq!(report.retrieval_config, "B1 deterministic trigram+BM25, no embeddings");
        assert_eq!(report.bootstrap_seed, 0);
    }
}
