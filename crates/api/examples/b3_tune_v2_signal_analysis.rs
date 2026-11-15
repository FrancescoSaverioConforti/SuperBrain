//! B3-TUNE-v2 Priority 1 diagnostic: computes score-derived signals
//! (top1 score, top1-top2 margin, top1/top2 ratio) per query on the
//! `dev` or `calibration` split, and reports which signal best
//! separates answerable from abstention-expected queries. Never reads
//! `eval/rag/test/` or `eval/rag/v2-heldout/` content. Diagnostic only;
//! not part of the frozen B3 pipeline, does not change `index::search`'s
//! production behavior.

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
    #[allow(dead_code)]
    question: String,
    #[allow(dead_code)]
    answer: String,
    category: String,
    #[allow(dead_code)]
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
    vault: Vault,
    pii: vault_security::PiiStore,
    indexer: Arc<RealIndexer>,
    #[allow(dead_code)]
    root: std::path::PathBuf,
}

impl TestFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("superbrain-b3-tune-v2-signal-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3-TUNE-v2 Signal Analysis", "test@localhost").unwrap();
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

/// The three candidate abstention-discrimination signals for one query's
/// top-1/top-2 search results. `None` fields mean there was no second
/// candidate (margin/ratio are then undefined for that query).
#[derive(Debug, Clone, serde::Serialize)]
struct QueryScores {
    top1_score: f64,
    top2_score: Option<f64>,
    margin: Option<f64>,
    ratio: Option<f64>,
    /// The ordered note ids `index::search` actually returned (top-`k`),
    /// kept so Task 2's sweep can recompute Recall@10/nDCG@10 under a
    /// hypothetical threshold override without a second search call.
    retrieved_note_ids: Vec<String>,
}

/// Runs a real search and extracts the top-1/top-2 scores. Returns `None`
/// on `SearchOutcome::NotPresent` — that case has no candidate score at
/// all, and is a separate, already-distinguishable outcome (B1 already
/// correctly says "no match" there); the case Priority 1 analyzes is a
/// low-quality `Matches` outcome, not a `NotPresent` one.
fn top_scores(idx: &index::Index<'_>, question: &str, k: usize) -> Option<QueryScores> {
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
        index::search::SearchOutcome::NotPresent => None,
        index::search::SearchOutcome::Matches { first, rest } => {
            let top1_score = first.score;
            let top2_score = rest.first().map(|r| r.score);
            let margin = top2_score.map(|t2| top1_score - t2);
            let ratio = top2_score.filter(|t2| *t2 != 0.0).map(|t2| top1_score / t2);
            let mut retrieved_note_ids: Vec<String> = vec![first.note_id.as_str().to_string()];
            retrieved_note_ids.extend(rest.iter().map(|r| r.note_id.as_str().to_string()));
            retrieved_note_ids.truncate(k);
            Some(QueryScores {
                top1_score,
                top2_score,
                margin,
                ratio,
                retrieved_note_ids,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_top1_and_top2_scores_from_a_real_matches_outcome() {
        let fixture = TestFixture::new();
        fixture.commit_corpus();
        let idx = fixture.build_index();
        let scores = top_scores(&idx, "resolve_configured_timezone", 10);
        let scores = scores.expect("expected a Matches outcome for this real corpus question");
        assert!(scores.top1_score > 0.0);
    }

    /// Two abstain-expected rows (ratio 1.0, 1.2) and two answerable rows
    /// (ratio 1.1, 1.3), one of which (ratio 1.1) would flip to a correct
    /// abstention at cutoff 1.15 while staying a false-abstention at a
    /// too-aggressive cutoff like 1.25. Exercises: the candidate-cutoff
    /// set is exactly the distinct observed ratio values; TP/FP counts
    /// shift correctly as the cutoff crosses each candidate.
    fn sweep_fixture_rows() -> Vec<SweepRow> {
        vec![
            SweepRow { abstain_expected: true, ratio: 1.0, relevant_note_ids: vec![], graded_relevance: HashMap::new(), retrieved_note_ids: vec![] },
            SweepRow { abstain_expected: true, ratio: 1.2, relevant_note_ids: vec![], graded_relevance: HashMap::new(), retrieved_note_ids: vec![] },
            SweepRow {
                abstain_expected: false,
                ratio: 1.1,
                relevant_note_ids: vec!["n1".to_string()],
                graded_relevance: HashMap::from([("n1".to_string(), 2u8)]),
                retrieved_note_ids: vec!["n1".to_string()],
            },
            SweepRow {
                abstain_expected: false,
                ratio: 1.3,
                relevant_note_ids: vec!["n2".to_string()],
                graded_relevance: HashMap::from([("n2".to_string(), 2u8)]),
                retrieved_note_ids: vec!["n2".to_string()],
            },
        ]
    }

    #[test]
    fn sweep_cutoffs_candidate_set_is_exactly_the_distinct_observed_values() {
        let rows = sweep_fixture_rows();
        let sweep = sweep_cutoffs(&rows);
        let mut cutoffs: Vec<f64> = sweep.iter().map(|r| r.cutoff).collect();
        cutoffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(cutoffs, vec![1.0, 1.1, 1.2, 1.3]);
    }

    #[test]
    fn sweep_cutoffs_confusion_matrix_shifts_correctly_across_cutoffs() {
        // predicted_not_present_at uses strict "ratio < cutoff -> predicted abstain"
        // (matching Task 3's "below threshold -> abstain" direction), so a row
        // whose own ratio equals the candidate cutoff is NOT flagged at that
        // cutoff -- it only flips once the cutoff exceeds its ratio.
        let rows = sweep_fixture_rows();
        let sweep = sweep_cutoffs(&rows);
        // At cutoff 1.0 (the min): nothing is strictly below it -> TP=0, FN=2, FP=0.
        let at_1_0 = sweep.iter().find(|r| r.cutoff == 1.0).unwrap();
        assert_eq!((at_1_0.confusion_matrix.tp, at_1_0.confusion_matrix.fn_, at_1_0.confusion_matrix.fp), (0, 2, 0));
        // At cutoff 1.2: the ratio==1.0 abstain row is strictly below -> TP=1, FN=1;
        // the answerable ratio==1.1 row is also strictly below -> FP=1.
        let at_1_2 = sweep.iter().find(|r| r.cutoff == 1.2).unwrap();
        assert_eq!((at_1_2.confusion_matrix.tp, at_1_2.confusion_matrix.fn_, at_1_2.confusion_matrix.fp), (1, 1, 1));
        // At cutoff 1.3 (the max): both abstain rows (1.0, 1.2) are strictly below -> TP=2, FN=0;
        // the answerable ratio==1.1 row is strictly below -> FP=1; the answerable
        // ratio==1.3 row equals the cutoff, not strictly below it -> stays TN.
        let at_1_3 = sweep.iter().find(|r| r.cutoff == 1.3).unwrap();
        assert_eq!((at_1_3.confusion_matrix.tp, at_1_3.confusion_matrix.fn_, at_1_3.confusion_matrix.fp), (2, 0, 1));
    }
}

const EVAL_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/rag");
const RESULTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/results/B3-TUNE-v2");
const K: usize = 10;

/// One signal's separation summary between the abstain-expected and
/// answerable groups: the full sorted value lists (never just means —
/// B3-TUNE-v2's Review Focus risk), plus a simple separation measure
/// (the gap between the highest abstain-expected value and the lowest
/// answerable value; negative means the groups overlap).
#[derive(Debug, serde::Serialize)]
struct SignalSeparation {
    abstain_expected_values_sorted: Vec<f64>,
    answerable_values_sorted: Vec<f64>,
    abstain_expected_n: usize,
    answerable_n: usize,
    /// max(abstain_expected) subtracted from min(answerable): positive
    /// means the two groups are cleanly separated by a gap of this size;
    /// zero or negative means they overlap. `None` if either group is
    /// empty for this signal (e.g. all abstain-expected queries had no
    /// second candidate, so margin/ratio are undefined for all of them).
    separation_gap: Option<f64>,
}

fn separation_for(mut abstain: Vec<f64>, mut answerable: Vec<f64>) -> SignalSeparation {
    abstain.sort_by(|a, b| a.partial_cmp(b).unwrap());
    answerable.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let separation_gap = match (abstain.last(), answerable.first()) {
        (Some(&max_abstain), Some(&min_answerable)) => Some(min_answerable - max_abstain),
        _ => None,
    };
    SignalSeparation {
        abstain_expected_n: abstain.len(),
        answerable_n: answerable.len(),
        abstain_expected_values_sorted: abstain,
        answerable_values_sorted: answerable,
        separation_gap,
    }
}

#[derive(Debug, serde::Serialize)]
struct PerQueryScoreReport {
    id: String,
    category: String,
    abstain_expected: bool,
    top1_score: Option<f64>,
    top2_score: Option<f64>,
    margin: Option<f64>,
    ratio: Option<f64>,
    /// true if `index::search` itself returned `NotPresent` (no
    /// candidate at all) — excluded from the three signal summaries
    /// below, since margin/ratio/top1 are undefined without a candidate.
    not_present: bool,
    /// The note ids this query's label row says are relevant (empty for
    /// abstain-expected rows), and the ids `index::search` actually
    /// retrieved — carried so Task 2's sweep can recompute Recall@10/
    /// nDCG@10 under a hypothetical threshold override without a second
    /// search call against this same split.
    relevant_note_ids: Vec<String>,
    graded_relevance: HashMap<String, u8>,
    retrieved_note_ids: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct SignalAnalysisReport {
    split: String,
    run_commit: String,
    labels_sha256: String,
    query_count: usize,
    query_count_excluding_canary: usize,
    not_present_count: usize,
    top1_score: SignalSeparation,
    margin: SignalSeparation,
    ratio: SignalSeparation,
    per_query: Vec<PerQueryScoreReport>,
    /// Hand-written (or `--conclusion`-supplied) analysis text recording
    /// which signal separates best on this split. `None` until Step 7.
    analysis_conclusion: Option<String>,
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

fn run_signal_analysis(split: &str) -> SignalAnalysisReport {
    assert!(
        split == "dev" || split == "calibration",
        "signal analysis only ever reads dev or calibration, never test or v2-heldout; got {split:?}"
    );
    let labels_path = format!("{EVAL_ROOT}/{split}/labels.jsonl");
    let labels_text = std::fs::read_to_string(&labels_path)
        .unwrap_or_else(|e| panic!("failed to read {labels_path}: {e}"));
    let rows = exclude_canary(parse_label_rows(&labels_text));

    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index();

    let mut per_query = Vec::with_capacity(rows.len());
    let mut not_present_count = 0;
    let mut top1_abstain = Vec::new();
    let mut top1_answerable = Vec::new();
    let mut margin_abstain = Vec::new();
    let mut margin_answerable = Vec::new();
    let mut ratio_abstain = Vec::new();
    let mut ratio_answerable = Vec::new();

    for row in &rows {
        let scores = top_scores(&idx, &row.question, K);
        match &scores {
            None => not_present_count += 1,
            Some(s) => {
                if row.abstain_expected {
                    top1_abstain.push(s.top1_score);
                    if let Some(m) = s.margin {
                        margin_abstain.push(m);
                    }
                    if let Some(r) = s.ratio {
                        ratio_abstain.push(r);
                    }
                } else {
                    top1_answerable.push(s.top1_score);
                    if let Some(m) = s.margin {
                        margin_answerable.push(m);
                    }
                    if let Some(r) = s.ratio {
                        ratio_answerable.push(r);
                    }
                }
            }
        }
        per_query.push(PerQueryScoreReport {
            id: row.id.clone(),
            category: row.category.clone(),
            abstain_expected: row.abstain_expected,
            top1_score: scores.as_ref().map(|s| s.top1_score),
            top2_score: scores.as_ref().and_then(|s| s.top2_score),
            margin: scores.as_ref().and_then(|s| s.margin),
            ratio: scores.as_ref().and_then(|s| s.ratio),
            not_present: scores.is_none(),
            relevant_note_ids: row.relevant_note_ids.clone(),
            graded_relevance: row.graded_relevance.clone(),
            retrieved_note_ids: scores.as_ref().map(|s| s.retrieved_note_ids.clone()).unwrap_or_default(),
        });
    }

    SignalAnalysisReport {
        split: split.to_string(),
        run_commit: git_rev_parse_head(),
        labels_sha256: sha256_hex_of_file(&labels_path),
        query_count: rows.len(),
        query_count_excluding_canary: rows.len(),
        not_present_count,
        top1_score: separation_for(top1_abstain, top1_answerable),
        margin: separation_for(margin_abstain, margin_answerable),
        ratio: separation_for(ratio_abstain, ratio_answerable),
        per_query,
        analysis_conclusion: None,
    }
}

/// One query's data reduced to exactly what the deterministic cutoff
/// sweep needs: the winning signal's value (here always `ratio` — Task 1
/// identified it as the strongest candidate on both dev and calibration),
/// ground truth, and what's needed to recompute Recall@10/nDCG@10 under
/// a hypothetical override.
#[derive(Debug, Clone)]
struct SweepRow {
    abstain_expected: bool,
    ratio: f64,
    relevant_note_ids: Vec<String>,
    graded_relevance: HashMap<String, u8>,
    retrieved_note_ids: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct SweepResult {
    cutoff: f64,
    confusion_matrix: metrics_confusion_matrix_report::ConfusionMatrixReport,
    precision: Option<f64>,
    recall: Option<f64>,
    f1: Option<f64>,
    /// FP / (FP + TN): the fraction of answerable rows this cutoff would
    /// incorrectly flag as abstain (a false abstention).
    false_abstention_rate: Option<f64>,
    recall_at_10_mean: f64,
    ndcg_at_10_mean: f64,
    /// Delta versus the OFFICIAL B1 baseline numbers recorded in
    /// `docs/research/B3-CLOSURE-SUMMARY.md` for calibration
    /// (Recall@10=0.9166666666666666, nDCG@10=0.7090535447985884),
    /// never versus an unthresholded rerun of this same commit.
    recall_at_10_delta_vs_b1_baseline: f64,
    ndcg_at_10_delta_vs_b1_baseline: f64,
}

/// The official B1 baseline's calibration-split aggregate numbers, copied
/// verbatim from `eval/results/B3/calibration-20261114T123333Z-e613edd.json`
/// (commit c7ad08e, recorded in `docs/research/B3-CLOSURE-SUMMARY.md`
/// section 5). Every sweep candidate's delta is computed against these
/// fixed numbers, never against an unthresholded rerun of this binary's
/// own commit.
const B1_BASELINE_CALIBRATION_RECALL_AT_10: f64 = 0.9166666666666666;
const B1_BASELINE_CALIBRATION_NDCG_AT_10: f64 = 0.7090535447985884;

/// Local serializable mirror of `api::metrics::ConfusionMatrix` (that
/// type doesn't derive `Serialize`, and this binary doesn't need the rest
/// of that module's surface) — kept in its own tiny module so the name
/// doesn't collide with anything this file defines for its own use.
mod metrics_confusion_matrix_report {
    #[derive(Debug, serde::Serialize)]
    pub struct ConfusionMatrixReport {
        pub tp: u32,
        pub tn: u32,
        pub fp: u32,
        #[serde(rename = "fn")]
        pub fn_: u32,
    }
}

/// Applies a candidate `cutoff` to one row: below the cutoff means the
/// row is overridden to a predicted abstention (`NotPresent`), mirroring
/// Task 3's not-yet-implemented `apply_abstention_threshold` for the
/// `Ratio` signal specifically — duplicated here deliberately (per the
/// plan's Task 2 Step 2 note) since this task runs before Task 3 exists.
fn predicted_not_present_at(ratio: f64, cutoff: f64) -> bool {
    ratio < cutoff
}

/// The deterministic cutoff sweep: evaluates every distinct observed
/// `ratio` value in `rows` as a candidate cutoff (the only cutoffs that
/// can change any row's classification — values strictly between two
/// adjacent observations all produce the same confusion matrix), and
/// reports the full confusion-matrix-derived metrics plus the overridden
/// Recall@10/nDCG@10 mean for each.
fn sweep_cutoffs(rows: &[SweepRow]) -> Vec<SweepResult> {
    let mut candidates: Vec<f64> = rows.iter().map(|r| r.ratio).collect();
    candidates.sort_by(|a, b| a.partial_cmp(b).unwrap());
    candidates.dedup_by(|a, b| (*a - *b).abs() < f64::EPSILON);

    candidates
        .into_iter()
        .map(|cutoff| {
            let abstain_expected: Vec<bool> = rows.iter().map(|r| r.abstain_expected).collect();
            let predicted_not_present: Vec<bool> =
                rows.iter().map(|r| predicted_not_present_at(r.ratio, cutoff)).collect();

            let mut cm = metrics_confusion_matrix_report::ConfusionMatrixReport { tp: 0, tn: 0, fp: 0, fn_: 0 };
            for (&truth, &pred) in abstain_expected.iter().zip(predicted_not_present.iter()) {
                match (truth, pred) {
                    (true, true) => cm.tp += 1,
                    (true, false) => cm.fn_ += 1,
                    (false, true) => cm.fp += 1,
                    (false, false) => cm.tn += 1,
                }
            }
            let precision = if cm.tp + cm.fp == 0 { None } else { Some(cm.tp as f64 / (cm.tp + cm.fp) as f64) };
            let recall = if cm.tp + cm.fn_ == 0 { None } else { Some(cm.tp as f64 / (cm.tp + cm.fn_) as f64) };
            let f1 = match (precision, recall) {
                (Some(p), Some(r)) if p + r > 0.0 => Some(2.0 * p * r / (p + r)),
                (Some(_), Some(_)) => Some(0.0),
                _ => None,
            };
            let false_abstention_rate =
                if cm.fp + cm.tn == 0 { None } else { Some(cm.fp as f64 / (cm.fp + cm.tn) as f64) };

            // Recall@10/nDCG@10 under this cutoff: a row overridden to
            // predicted-abstain is scored as if it retrieved nothing
            // (empty retrieved list), matching the frozen metric formula's
            // "NotPresent scores 0.0, never excluded" rule. Abstain-
            // expected rows have no relevant_note_ids and are not scored
            // (same convention the B3 runner uses: these metrics are
            // computed over answerable rows only).
            let mut recall_values = Vec::new();
            let mut ndcg_values = Vec::new();
            for (row, &pred) in rows.iter().zip(predicted_not_present.iter()) {
                if row.abstain_expected {
                    continue;
                }
                let retrieved: Vec<String> = if pred { Vec::new() } else { row.retrieved_note_ids.clone() };
                recall_values.push(api::metrics::recall_at_10(&row.relevant_note_ids, &retrieved));
                ndcg_values.push(api::metrics::ndcg_at_10(&retrieved, &row.graded_relevance));
            }
            let recall_at_10_mean = recall_values.iter().sum::<f64>() / recall_values.len().max(1) as f64;
            let ndcg_at_10_mean = ndcg_values.iter().sum::<f64>() / ndcg_values.len().max(1) as f64;

            SweepResult {
                cutoff,
                confusion_matrix: cm,
                precision,
                recall,
                f1,
                false_abstention_rate,
                recall_at_10_mean,
                ndcg_at_10_mean,
                recall_at_10_delta_vs_b1_baseline: recall_at_10_mean - B1_BASELINE_CALIBRATION_RECALL_AT_10,
                ndcg_at_10_delta_vs_b1_baseline: ndcg_at_10_mean - B1_BASELINE_CALIBRATION_NDCG_AT_10,
            }
        })
        .collect()
}

fn sweep_rows_from_report(report: &SignalAnalysisReport) -> Vec<SweepRow> {
    report
        .per_query
        .iter()
        .filter_map(|q| {
            let ratio = q.ratio?; // rows with no second candidate (or NotPresent) can't be swept on ratio
            Some(SweepRow {
                abstain_expected: q.abstain_expected,
                ratio,
                relevant_note_ids: q.relevant_note_ids.clone(),
                graded_relevance: q.graded_relevance.clone(),
                retrieved_note_ids: q.retrieved_note_ids.clone(),
            })
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut split = "dev".to_string();
    let mut conclusion: Option<String> = None;
    let mut sweep_split: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--split" => {
                split = args.get(i + 1).cloned().unwrap_or_else(|| {
                    panic!("--split requires a value (dev|calibration)")
                });
                i += 2;
            }
            "--conclusion" => {
                conclusion = Some(args.get(i + 1).cloned().unwrap_or_else(|| {
                    panic!("--conclusion requires a value")
                }));
                i += 2;
            }
            "--sweep" => {
                sweep_split = Some(args.get(i + 1).cloned().unwrap_or_else(|| {
                    panic!("--sweep requires a value (dev|calibration)")
                }));
                i += 2;
            }
            other => panic!("unrecognized argument: {other}"),
        }
    }

    if let Some(sweep_split) = sweep_split {
        assert!(
            sweep_split == "calibration",
            "the Priority 1 threshold sweep is calibration-only per the design doc's \
             \"calibration decides, not dev\" rule; got {sweep_split:?}"
        );
        let report = run_signal_analysis(&sweep_split);
        let sweep_rows = sweep_rows_from_report(&report);
        let sweep = sweep_cutoffs(&sweep_rows);
        std::fs::create_dir_all(RESULTS_DIR).unwrap();
        let out_path = format!("{RESULTS_DIR}/priority1-threshold-sweep.json");
        let json = serde_json::to_string_pretty(&sweep).unwrap();
        std::fs::write(&out_path, json).unwrap();
        println!("wrote {out_path} ({} candidate cutoffs evaluated)", sweep.len());
        return;
    }

    let mut report = run_signal_analysis(&split);
    report.analysis_conclusion = conclusion;

    std::fs::create_dir_all(RESULTS_DIR).unwrap();
    let out_name = if split == "dev" {
        "priority1-signal-analysis.json".to_string()
    } else {
        format!("priority1-signal-analysis-{split}.json")
    };
    let out_path = format!("{RESULTS_DIR}/{out_name}");
    let json = serde_json::to_string_pretty(&report).unwrap();
    std::fs::write(&out_path, json).unwrap();
    println!("wrote {out_path}");
}
