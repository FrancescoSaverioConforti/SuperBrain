//! B3-TUNE-v2 Priority 3 Stage 1, Lever P1: exact structural-identifier
//! boost ablation, measured through the real `Index::search` pipeline
//! via `index::Index::with_identifier_boost()` (additive, opt-in; see
//! `crates/index/src/search.rs`). Isolated against official B1 only —
//! never composed with Lever P2 (`b3_tune_v2_priority3_p2_status_tiebreak.rs`).
//! `--ablate <split>` runs B1+P1 vs. plain B1 on dev or calibration.

use index::indexer_impl::{rebuild_index, RealIndexer};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use storage::{NoteId, Vault, Version};
use uuid::Uuid;

const CORPUS_NOTES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/rag/corpus/notes");

const RESULTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/results/B3-TUNE-v2");
const K: usize = 10;
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
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("malformed label row: {e}\nline: {line}"))
        })
        .collect()
}

fn exclude_canary(rows: Vec<LabelRow>) -> Vec<LabelRow> {
    rows.into_iter().filter(|r| r.id != CANARY_ROW_ID).collect()
}

/// Local re-implementation of `search.rs`'s `identifier_boost_applies`
/// matching rule (duplicated across the crate boundary the same way
/// Priority 2 Lever B had to duplicate `bm25::tokenize`, since
/// `extract_structural_identifier`/`identifier_boost_applies` are
/// private to the `index` crate) — same whole-token, case-insensitive,
/// exact-equality rule, never substring/prefix. Pin test below mirrors
/// `crates/index/tests/b1_search.rs`'s own coverage of the real logic.
fn identifier_boost_would_fire(query: &str, source: &str) -> bool {
    let Some(identifier) = extract_structural_identifier(source) else {
        return false;
    };
    let identifier_lower = identifier.to_lowercase();
    query.split_whitespace().any(|token| token.to_lowercase() == identifier_lower)
}

/// Mirrors `crates/index/src/search.rs`'s private `extract_structural_identifier`
/// exactly: reads `source`'s trailing `#<IDENTIFIER>` fragment up to the
/// first non-identifier character.
fn extract_structural_identifier(source: &str) -> Option<String> {
    let fragment_start = source.find('#')?;
    let fragment = &source[fragment_start + 1..];
    let identifier: String = fragment
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!identifier.is_empty()).then_some(identifier)
}

struct TestFixture {
    vault: Option<Vault>,
    pii: Option<vault_security::PiiStore>,
    indexer: Arc<RealIndexer>,
    root: std::path::PathBuf,
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        // Same Windows file-lock fix as Lever A/B/C's own TestFixture:
        // drop vault/pii explicitly, in this order, before touching the
        // filesystem.
        drop(self.vault.take());
        drop(self.pii.take());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl TestFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("superbrain-b3-tune-v2-p1-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3-TUNE-v2 Priority 3 P1", "test@localhost").unwrap();
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

    fn build_index(&self) -> index::Index<'_> {
        rebuild_index(self.vault(), &self.indexer).unwrap();
        index::Index::new(
            self.vault(),
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
    api::RequestContext { schema_version: api::SCHEMA_VERSION, correlation_id: Uuid::new_v4() }
}

#[derive(Debug, Clone, PartialEq)]
enum QueryOutcome {
    NotPresent,
    Matches { retrieved_note_ids: Vec<String> },
}

/// Like Lever C's `run_one_query`, but also reports whether the
/// identifier boost's matching rule fired for at least one of the
/// returned candidates' `source` fields against this query — computed
/// from the SAME search response, before `source` is discarded, so
/// this is never a second, separate query execution.
fn run_one_query_tracking_boost(idx: &index::Index<'_>, question: &str, k: usize) -> (QueryOutcome, bool) {
    let response = api::search(
        idx,
        &allow_policy(),
        request_context(),
        api::SearchRequest { text: question.to_string(), filters: Default::default(), k },
    );
    let search_response = response.result.unwrap_or_else(|e| {
        panic!("search failed for question {question:?}: {e}");
    });
    match search_response.outcome {
        index::search::SearchOutcome::NotPresent => (QueryOutcome::NotPresent, false),
        index::search::SearchOutcome::Matches { first, rest } => {
            let mut results = vec![first];
            results.extend(rest);
            let fired = results.iter().any(|r| {
                r.provenance
                    .source
                    .as_deref()
                    .is_some_and(|source| identifier_boost_would_fire(question, source))
            });
            let mut ids: Vec<String> = results.iter().map(|r| r.note_id.as_str().to_string()).collect();
            ids.truncate(k);
            (QueryOutcome::Matches { retrieved_note_ids: ids }, fired)
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
struct AblationReportWithBreakdown {
    split: String,
    run_commit: String,
    labels_sha256: String,
    /// How many of this split's non-canary queries had the identifier
    /// boost actually fire (at least one returned candidate's source
    /// matched a query token). A report with this at 0 means the
    /// aggregate metrics are identical to plain B1 by construction --
    /// the lever was never exercised, not tried and found unhelpful.
    identifier_boost_fired: usize,
    query_count: usize,
    aggregate: MetricBreakdown,
    by_category: HashMap<String, MetricBreakdown>,
    by_difficulty: HashMap<String, MetricBreakdown>,
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

fn breakdown_by<F: Fn(&LabelRow) -> String>(
    rows: &[LabelRow],
    recall_values: &[f64],
    ndcg_values: &[f64],
    answerable_rows: &[&LabelRow],
    key_fn: F,
    seed: u64,
    resamples: u32,
) -> HashMap<String, MetricBreakdown> {
    let _ = rows;
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

fn run_ablation_for_split(split: &str) -> AblationReportWithBreakdown {
    assert!(
        split == "dev" || split == "calibration",
        "Priority 3 P1's ablation only ever reads dev or calibration, never test or v2-heldout; got {split:?}"
    );
    let labels_path = format!("{}/../../eval/rag/{split}/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let raw_labels_text = std::fs::read_to_string(&labels_path).unwrap();
    let rows = exclude_canary(parse_label_rows(&raw_labels_text));

    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index().with_identifier_boost();

    let mut recall_values: Vec<f64> = Vec::new();
    let mut ndcg_values: Vec<f64> = Vec::new();
    let mut answerable_rows: Vec<&LabelRow> = Vec::new();
    let mut abstain_expected_flags: Vec<bool> = Vec::new();
    let mut predicted_not_present_flags: Vec<bool> = Vec::new();
    let mut identifier_boost_fired = 0usize;

    for row in &rows {
        let (outcome, fired) = run_one_query_tracking_boost(&idx, &row.question, K);
        if fired {
            identifier_boost_fired += 1;
        }
        abstain_expected_flags.push(row.abstain_expected);
        predicted_not_present_flags.push(matches!(outcome, QueryOutcome::NotPresent));
        if !row.abstain_expected {
            let scored = score_answerable_row(row, &outcome);
            recall_values.push(scored.recall_at_10);
            ndcg_values.push(scored.ndcg_at_10);
            answerable_rows.push(row);
        }
    }

    let aggregate = metrics_breakdown(&recall_values, &ndcg_values, BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES);
    let by_category = breakdown_by(
        &rows, &recall_values, &ndcg_values, &answerable_rows,
        |row| row.category.clone(), BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES,
    );
    let by_difficulty = breakdown_by(
        &rows, &recall_values, &ndcg_values, &answerable_rows,
        |row| row.difficulty.clone(), BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES,
    );
    let cm_bootstrap = api::metrics::confusion_matrix_bootstrap(
        &abstain_expected_flags, &predicted_not_present_flags, BOOTSTRAP_SEED, BOOTSTRAP_RESAMPLES,
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

    AblationReportWithBreakdown {
        split: split.to_string(),
        run_commit: git_rev_parse_head(),
        labels_sha256: sha256_hex_of_file(&labels_path),
        identifier_boost_fired,
        query_count: rows.len(),
        aggregate,
        by_category,
        by_difficulty,
        abstention,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--ablate") => {
            let split = args.get(2).cloned().unwrap_or_else(|| panic!("--ablate requires a split (dev|calibration)"));
            let report = run_ablation_for_split(&split);
            println!(
                "{split}: Recall@10={:.4} nDCG@10={:.4} identifier_boost_fired={}/{}",
                report.aggregate.recall_at_10, report.aggregate.ndcg_at_10,
                report.identifier_boost_fired, report.query_count
            );
            std::fs::create_dir_all(RESULTS_DIR).unwrap();
            let out_path = format!("{RESULTS_DIR}/priority3-p1-{split}-ablation.json");
            let json = serde_json::to_string_pretty(&report).unwrap();
            std::fs::write(&out_path, json).unwrap();
            println!("wrote {out_path}");
        }
        other => panic!("usage: --ablate <dev|calibration>; got {other:?}"),
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
    fn identifier_boost_fired_is_true_only_when_a_query_token_matches_a_candidates_identifier() {
        assert!(identifier_boost_would_fire("tell me about sec-001", "docs/x.md#SEC-001"));
        assert!(!identifier_boost_would_fire("tell me about section one", "docs/x.md#SEC-001"));
        assert!(!identifier_boost_would_fire("sec", "docs/x.md#SEC-001"));
    }
}
