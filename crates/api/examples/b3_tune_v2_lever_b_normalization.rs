//! B3-TUNE-v2 Priority 2 Lever B: conservative punctuation
//! normalization. Two modes:
//! - `--diagnostic` (Task 4): scans every distinct token in the corpus
//!   + dev split, applies `index::lexical_normalize::strip_trailing_sentence_punctuation`,
//!   and reports every token where stripping changes it, for manual
//!   review — this never changes tokenization itself, it only observes
//!   what the rule would do.
//! - `--ablate <split>` (Task 5): runs `B-query` (normalization applied
//!   to the query only) and `B-symmetric` (normalization applied to both
//!   indexed text and query) against the real corpus, measuring the
//!   effect on Recall@10/nDCG@10. `bm25::tokenize` itself is never
//!   modified; this binary duplicates its exact, documented
//!   whitespace+lowercase logic locally (that function is `pub(crate)`
//!   to the `index` crate, not visible here), since this plan does not
//!   widen `bm25::tokenize`'s visibility or otherwise touch the frozen
//!   production tokenizer.

use index::indexer_impl::{rebuild_index, RealIndexer};
use index::lexical_normalize::strip_trailing_sentence_punctuation;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use storage::{NoteId, Vault, Version};
use uuid::Uuid;

const CANARY_ROW_ID: &str = "canary-isolation-probe";
const CORPUS_NOTES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/rag/corpus/notes");
const RESULTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/results/B3-TUNE-v2");
const K: usize = 10;
const BOOTSTRAP_SEED: u64 = 0;
const BOOTSTRAP_RESAMPLES: u32 = 2000;

/// Duplicate of `index::bm25::tokenize`'s exact documented behavior
/// (whitespace-delimited, Unicode-aware, case-folded, no punctuation
/// stripping) — that function is `pub(crate)` to the `index` crate and
/// not visible here; this plan does not widen its visibility, per
/// Global Constraints' "no existing call site changes behavior."
fn tokenize_like_bm25(text: &str) -> Vec<String> {
    text.split_whitespace().map(|token| token.to_lowercase()).collect()
}

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
    vault: Vault,
    pii: vault_security::PiiStore,
    indexer: Arc<RealIndexer>,
    root: std::path::PathBuf,
}

impl TestFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("superbrain-b3-tune-v2-lever-b-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3-TUNE-v2 Lever B", "test@localhost").unwrap();
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
        Self { vault, pii, indexer: Arc::new(RealIndexer::new()), root }
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

#[derive(Debug, Clone, PartialEq)]
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

/// Normalizes `text` by splitting on whitespace, applying
/// `strip_trailing_sentence_punctuation` per token, and rejoining with
/// single spaces. Used for both the query side (always) and, in
/// `B-symmetric`, the indexed-text side too — the SAME function call in
/// both places, so the two sides can never drift from each other.
fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .map(strip_trailing_sentence_punctuation)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `B-query`: applies normalization to the question text only, against
/// a plain, unmodified index (indexed text stays exactly as B1 indexes
/// it today).
fn run_one_query_normalized(idx: &index::Index<'_>, question: &str, k: usize) -> QueryOutcome {
    run_one_query(idx, &normalize_text(question), k)
}

/// `B-symmetric`'s own standalone scoring path: builds a `Bm25Index` +
/// `TrigramIndex` pair directly from the corpus's `text_searchable`
/// content, with `strip_trailing_sentence_punctuation` applied to every
/// token on the INDEXED side (never done by `RealIndexer`, which always
/// calls the frozen, unmodified `bm25::tokenize`) before insertion. This
/// bypasses `RealIndexer`/`rebuild_index` entirely, by design: proving
/// a symmetric effect requires indexed text this plan's own frozen
/// pipeline has never normalized, which is not something `RealIndexer`
/// can be asked to do without touching `bm25::tokenize` itself.
struct SymmetricIndex {
    bm25: index::bm25::Bm25Index,
    trigram: index::trigram::TrigramIndex,
}

impl SymmetricIndex {
    fn build(corpus_dir: &str) -> Self {
        let mut bm25 = index::bm25::Bm25Index::new();
        let mut trigram = index::trigram::TrigramIndex::new();
        for entry in std::fs::read_dir(corpus_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let raw = std::fs::read(&path).unwrap();
            let document = index::document::Document::from_record_bytes(&raw)
                .unwrap_or_else(|e| panic!("failed to parse {path:?}: {e}"));
            let normalized_text = normalize_text(&document.text_searchable);
            let note_id = NoteId::parse(&document.id).unwrap();
            bm25.insert(note_id.clone(), &normalized_text);
            trigram.insert(note_id, &normalized_text);
        }
        Self { bm25, trigram }
    }

    /// Runs one normalized query against this standalone index. Mirrors
    /// `RealIndexer::candidate_ids`'s own candidate generation (unions
    /// `trigram.query_exact(query)` with every note `bm25.query_ranked`
    /// scores above zero) before final BM25 ranking — the trigram side
    /// only widens which notes are even considered, since `bm25.query_ranked`
    /// is what actually produces the returned ranking and scores.
    /// Simplified versus the real `Index::search` in one respect: no
    /// vault/project/classification/time filtering is re-applied, since
    /// this standalone index has none of that — every corpus note is
    /// eligible by construction.
    fn search(&self, query: &str, k: usize) -> QueryOutcome {
        let normalized_query = normalize_text(query);
        let mut candidates: HashSet<NoteId> = self.trigram.query_exact(&normalized_query).into_iter().collect();
        candidates.extend(self.bm25.query_ranked(&normalized_query, usize::MAX).into_iter().map(|(id, _)| id));
        if candidates.is_empty() {
            return QueryOutcome::NotPresent;
        }
        let ranked = self.bm25.query_ranked(&normalized_query, k);
        if ranked.is_empty() {
            return QueryOutcome::NotPresent;
        }
        QueryOutcome::Matches {
            retrieved_note_ids: ranked.into_iter().map(|(id, _)| id.as_str().to_string()).collect(),
        }
    }
}

fn run_one_query_symmetric(idx: &SymmetricIndex, question: &str, k: usize) -> QueryOutcome {
    idx.search(question, k)
}

fn build_index_with_normalization() -> SymmetricIndex {
    SymmetricIndex::build(CORPUS_NOTES_DIR)
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
struct VariantReport {
    variant: String,
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

fn run_variant(rows: &[LabelRow], run_query: impl Fn(&LabelRow) -> QueryOutcome, variant_name: &str) -> VariantReport {
    let mut recall_values: Vec<f64> = Vec::new();
    let mut ndcg_values: Vec<f64> = Vec::new();
    let mut abstain_expected_flags: Vec<bool> = Vec::new();
    let mut predicted_not_present_flags: Vec<bool> = Vec::new();

    for row in rows {
        let outcome = run_query(row);
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

    VariantReport { variant: variant_name.to_string(), aggregate, abstention }
}

#[derive(Debug, serde::Serialize)]
struct AblationReport {
    split: String,
    run_commit: String,
    labels_sha256: String,
    b_query: VariantReport,
    b_symmetric: VariantReport,
}

fn run_ablation_for_split(split: &str) -> AblationReport {
    assert!(
        split == "dev" || split == "calibration",
        "Lever B's ablation only ever reads dev or calibration, never test or v2-heldout; got {split:?}"
    );
    let labels_path = format!("{}/../../eval/rag/{split}/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let raw_labels_text = std::fs::read_to_string(&labels_path).unwrap();
    let rows = exclude_canary(parse_label_rows(&raw_labels_text));

    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let plain_idx = fixture.build_index();
    let b_query = run_variant(&rows, |row| run_one_query_normalized(&plain_idx, &row.question, K), "B-query");

    let symmetric_idx = build_index_with_normalization();
    let b_symmetric = run_variant(&rows, |row| run_one_query_symmetric(&symmetric_idx, &row.question, K), "B-symmetric");

    AblationReport {
        split: split.to_string(),
        run_commit: git_rev_parse_head(),
        labels_sha256: sha256_hex_of_file(&labels_path),
        b_query,
        b_symmetric,
    }
}

#[derive(Debug, serde::Serialize)]
struct TokenDiagnosticEntry {
    original: String,
    stripped: String,
}

fn run_token_diagnostic() -> Vec<TokenDiagnosticEntry> {
    let mut distinct_tokens: HashSet<String> = HashSet::new();

    for entry in std::fs::read_dir(CORPUS_NOTES_DIR).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let raw = std::fs::read(&path).unwrap();
        let document = index::document::Document::from_record_bytes(&raw)
            .unwrap_or_else(|e| panic!("failed to parse {path:?}: {e}"));
        distinct_tokens.extend(tokenize_like_bm25(&document.text_searchable));
    }

    let dev_labels_path = format!("{}/../../eval/rag/dev/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let dev_text = std::fs::read_to_string(&dev_labels_path).unwrap();
    for row in exclude_canary(parse_label_rows(&dev_text)) {
        distinct_tokens.extend(tokenize_like_bm25(&row.question));
    }

    let mut affected: Vec<TokenDiagnosticEntry> = distinct_tokens
        .into_iter()
        .filter_map(|token| {
            let stripped = strip_trailing_sentence_punctuation(&token);
            if stripped == token {
                None
            } else {
                Some(TokenDiagnosticEntry { original: token.clone(), stripped: stripped.to_string() })
            }
        })
        .collect();
    affected.sort_by(|a, b| a.original.cmp(&b.original));
    affected
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--diagnostic") => {
            let affected = run_token_diagnostic();
            for entry in &affected {
                println!("{:?} -> {:?}", entry.original, entry.stripped);
            }
            std::fs::create_dir_all(RESULTS_DIR).unwrap();
            let out_path = format!("{RESULTS_DIR}/priority2-lever-b-token-diagnostic.json");
            let json = serde_json::to_string_pretty(&affected).unwrap();
            std::fs::write(&out_path, json).unwrap();
            println!("wrote {out_path} ({} affected tokens)", affected.len());
        }
        Some("--ablate") => {
            let split = args.get(2).cloned().unwrap_or_else(|| panic!("--ablate requires a split (dev|calibration)"));
            let report = run_ablation_for_split(&split);
            println!(
                "{split}: B-query Recall@10={:.4} nDCG@10={:.4} | B-symmetric Recall@10={:.4} nDCG@10={:.4}",
                report.b_query.aggregate.recall_at_10, report.b_query.aggregate.ndcg_at_10,
                report.b_symmetric.aggregate.recall_at_10, report.b_symmetric.aggregate.ndcg_at_10
            );
            std::fs::create_dir_all(RESULTS_DIR).unwrap();
            let out_path = format!("{RESULTS_DIR}/priority2-lever-b-{split}-ablation.json");
            let json = serde_json::to_string_pretty(&report).unwrap();
            std::fs::write(&out_path, json).unwrap();
            println!("wrote {out_path}");
        }
        other => panic!("usage: --diagnostic | --ablate <dev|calibration>; got {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_with_trailing_punctuation_matches_the_same_as_without() {
        let fixture = TestFixture::new();
        fixture.commit_corpus();
        let idx = fixture.build_index(); // plain B1 index, unmodified
        let with_comma = run_one_query_normalized(&idx, "resolve_configured_timezone,", 10);
        let without_comma = run_one_query_normalized(&idx, "resolve_configured_timezone", 10);
        assert_eq!(with_comma, without_comma);
    }

    #[test]
    fn build_index_with_normalization_indexes_stripped_tokens() {
        let idx = build_index_with_normalization();
        let result = run_one_query_symmetric(&idx, "resolve_configured_timezone", 10);
        assert!(matches!(result, QueryOutcome::Matches { .. }));
    }
}
