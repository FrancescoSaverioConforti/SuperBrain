//! B3-TUNE-v2 Priority 2 Lever C: a small, hand-curated, deterministic
//! query-time alias/synonym table. Candidate identification uses only
//! the corpus and the dev split — **never calibration** (design doc
//! correction #1). Modes:
//! - `--diagnostic` (Task 6): scans the corpus's `source:` frontmatter
//!   for `#<IDENTIFIER>` fragments, cross-references each against dev's
//!   `relevant_note_ids`, and writes every candidate (dev-referenced or
//!   not) for human review.
//! - `--ablate <split>` (Task 8): runs the frozen alias table (Task 7's
//!   output) against dev and calibration, measuring query-time
//!   expansion's effect on Recall@10/nDCG@10.

use index::indexer_impl::{rebuild_index, RealIndexer};
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

#[derive(Debug, Clone, serde::Deserialize)]
struct LabelRow {
    id: String,
    question: String,
    #[allow(dead_code)]
    answer: String,
    #[allow(dead_code)]
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

/// A candidate alias pair derived purely from corpus structure: the
/// `source:` frontmatter's trailing `#<IDENTIFIER>` fragment. `evidence`
/// is the note's own `id:` — Step 5's human review is where a real
/// `expansion` phrase gets chosen by reading the note body, never
/// synthesized automatically here.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct AliasCandidate {
    short_form: String,
    note_id: String,
    evidence: String,
}

/// Parses `note_text`'s frontmatter `source:` field for a trailing
/// `#<IDENTIFIER>` fragment (e.g. `docs/technical-debt.md#SEC-001`).
/// The identifier is read up to the first non-identifier character
/// (whitespace, `(`, etc.) — `source: docs/technical-debt.md#SEC-002
/// (quarta revisione)` yields `SEC-002`, not the parenthetical. Returns
/// one candidate if a fragment is found, none otherwise.
fn extract_structural_candidates(note_text: &str) -> Vec<AliasCandidate> {
    let mut note_id = None;
    let mut source = None;
    for line in note_text.lines() {
        if let Some(rest) = line.strip_prefix("id:") {
            note_id = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("source:") {
            source = Some(rest.trim().to_string());
        }
    }
    let (Some(note_id), Some(source)) = (note_id, source) else {
        return Vec::new();
    };
    let Some(fragment_start) = source.find('#') else {
        return Vec::new();
    };
    let fragment = &source[fragment_start + 1..];
    let identifier: String = fragment
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if identifier.is_empty() {
        return Vec::new();
    }
    vec![AliasCandidate { short_form: identifier, note_id: note_id.clone(), evidence: note_id }]
}

#[derive(Debug, serde::Serialize)]
struct CandidateReport {
    short_form: String,
    note_id: String,
    evidence: String,
    dev_referenced: bool,
}

fn run_candidate_diagnostic() -> Vec<CandidateReport> {
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(CORPUS_NOTES_DIR).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let note_text = std::fs::read_to_string(&path).unwrap();
        candidates.extend(extract_structural_candidates(&note_text));
    }

    let dev_labels_path = format!("{}/../../eval/rag/dev/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let dev_text = std::fs::read_to_string(&dev_labels_path).unwrap();
    let dev_rows = exclude_canary(parse_label_rows(&dev_text));
    let dev_referenced_note_ids: HashSet<String> =
        dev_rows.iter().flat_map(|row| row.relevant_note_ids.iter().cloned()).collect();

    candidates
        .into_iter()
        .map(|c| CandidateReport {
            dev_referenced: dev_referenced_note_ids.contains(&c.note_id),
            short_form: c.short_form,
            note_id: c.note_id,
            evidence: c.evidence,
        })
        .collect()
}

struct TestFixture {
    vault: Vault,
    pii: vault_security::PiiStore,
    indexer: Arc<RealIndexer>,
    root: std::path::PathBuf,
}

impl TestFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("superbrain-b3-tune-v2-lever-c-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3-TUNE-v2 Lever C", "test@localhost").unwrap();
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

/// Expands `query` by appending the expansion phrase of every
/// whitespace-delimited token (lowercased for lookup only — the
/// original casing of `query` itself is preserved) found in `table`.
/// Appends, never replaces: the original short form stays in the
/// query, so it remains matchable too; this is strictly additive to
/// query-term coverage. Matches whole tokens only, never a substring
/// inside a different token (e.g. "SEC" never matches inside
/// "SECTION").
/// True if `query` contains at least one whitespace-delimited token
/// (case-insensitive) present in `table` — i.e. whether
/// `apply_alias_expansion` would actually change `query`. Added after
/// an independent review found the frozen 7-entry table never fired on
/// a single dev or calibration query (see
/// `priority2-stage1-summary.md`'s Lever C correction): this lets an
/// ablation report count real activations, rather than only reporting
/// metrics that stay identical to B1 whether the table fired zero
/// times or many.
fn alias_expansion_fired(query: &str, table: &HashMap<String, String>) -> bool {
    query.split_whitespace().any(|token| table.contains_key(&token.to_lowercase()))
}

fn apply_alias_expansion(query: &str, table: &HashMap<String, String>) -> String {
    let mut expansions: Vec<&str> = Vec::new();
    for token in query.split_whitespace() {
        let lower = token.to_lowercase();
        if let Some(expansion) = table.get(&lower) {
            if !expansions.contains(&expansion.as_str()) {
                expansions.push(expansion.as_str());
            }
        }
    }
    if expansions.is_empty() {
        return query.to_string();
    }
    let mut result = query.to_string();
    for expansion in expansions {
        result.push(' ');
        result.push_str(expansion);
    }
    result
}

fn load_frozen_alias_table() -> HashMap<String, String> {
    let path = format!("{RESULTS_DIR}/priority2-lever-c-alias-table.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))).unwrap();
    json["entries"]
        .as_object()
        .unwrap_or_else(|| panic!("{path} missing \"entries\" object"))
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v["expansion"].as_str().unwrap().to_string()))
        .collect()
}

fn run_one_query_with_aliases(idx: &index::Index<'_>, question: &str, k: usize, table: &HashMap<String, String>) -> QueryOutcome {
    run_one_query(idx, &apply_alias_expansion(question, table), k)
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
struct AblationReport {
    split: String,
    run_commit: String,
    labels_sha256: String,
    /// How many of this split's non-canary queries had the alias table
    /// actually fire (at least one whitespace token matched a frozen
    /// short form). A report with `expansions_fired: 0` means the
    /// aggregate metrics below are identical to B1 by construction —
    /// the table was never exercised, not that it was tried and found
    /// unhelpful. Added after independent review found this
    /// distinction missing (see `priority2-stage1-summary.md`).
    expansions_fired: usize,
    query_count: usize,
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

fn run_ablation_for_split(split: &str) -> AblationReport {
    assert!(
        split == "dev" || split == "calibration",
        "Lever C's ablation only ever reads dev or calibration, never test or v2-heldout; got {split:?}"
    );
    let table = load_frozen_alias_table();
    let labels_path = format!("{}/../../eval/rag/{split}/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let raw_labels_text = std::fs::read_to_string(&labels_path).unwrap();
    let rows = exclude_canary(parse_label_rows(&raw_labels_text));

    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index();

    let mut recall_values: Vec<f64> = Vec::new();
    let mut ndcg_values: Vec<f64> = Vec::new();
    let mut abstain_expected_flags: Vec<bool> = Vec::new();
    let mut predicted_not_present_flags: Vec<bool> = Vec::new();
    let mut expansions_fired = 0usize;

    for row in &rows {
        if alias_expansion_fired(&row.question, &table) {
            expansions_fired += 1;
        }
        let outcome = run_one_query_with_aliases(&idx, &row.question, K, &table);
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

    AblationReport {
        split: split.to_string(),
        run_commit: git_rev_parse_head(),
        labels_sha256: sha256_hex_of_file(&labels_path),
        expansions_fired,
        query_count: rows.len(),
        aggregate,
        abstention,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--diagnostic") => {
            let candidates = run_candidate_diagnostic();
            for c in &candidates {
                println!("{} -> note {} (dev_referenced={})", c.short_form, c.note_id, c.dev_referenced);
            }
            std::fs::create_dir_all(RESULTS_DIR).unwrap();
            let out_path = format!("{RESULTS_DIR}/priority2-lever-c-alias-candidates.json");
            let json = serde_json::to_string_pretty(&candidates).unwrap();
            std::fs::write(&out_path, json).unwrap();
            println!("wrote {out_path} ({} candidates, {} dev-referenced)", candidates.len(), candidates.iter().filter(|c| c.dev_referenced).count());
        }
        Some("--ablate") => {
            let split = args.get(2).cloned().unwrap_or_else(|| panic!("--ablate requires a split (dev|calibration)"));
            let report = run_ablation_for_split(&split);
            println!(
                "{split}: Recall@10={:.4} nDCG@10={:.4} expansions_fired={}/{}",
                report.aggregate.recall_at_10, report.aggregate.ndcg_at_10,
                report.expansions_fired, report.query_count
            );
            std::fs::create_dir_all(RESULTS_DIR).unwrap();
            let out_path = format!("{RESULTS_DIR}/priority2-lever-c-{split}-ablation.json");
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
    fn extracts_a_source_identifier_to_body_phrase_candidate() {
        let note_text = std::fs::read_to_string(
            format!("{CORPUS_NOTES_DIR}/bug-2026-10-27-sec001-toctou.md")
        ).unwrap();
        let candidates = extract_structural_candidates(&note_text);
        assert!(candidates.iter().any(|c| c.short_form == "SEC-001"));
    }

    #[test]
    fn stops_the_identifier_at_the_first_non_identifier_character() {
        let note_text = std::fs::read_to_string(
            format!("{CORPUS_NOTES_DIR}/bug-2026-10-27-sec002-fix-incomplete.md")
        ).unwrap();
        let candidates = extract_structural_candidates(&note_text);
        assert!(candidates.iter().any(|c| c.short_form == "SEC-002"));
        assert!(!candidates.iter().any(|c| c.short_form.contains('(')));
    }

    #[test]
    fn alias_expansion_fired_is_true_only_when_a_token_matched() {
        let table = HashMap::from([("sec-001".to_string(), "symlink toctou bug".to_string())]);
        assert!(alias_expansion_fired("what is sec-001 about", &table));
        assert!(!alias_expansion_fired("what is section about", &table));
    }

    #[test]
    fn expands_a_whole_token_match() {
        let table = HashMap::from([("sec-001".to_string(), "symlink toctou bug".to_string())]);
        assert_eq!(apply_alias_expansion("what is sec-001 about", &table), "what is sec-001 about symlink toctou bug");
    }

    #[test]
    fn does_not_expand_a_substring_match_inside_a_different_token() {
        let table = HashMap::from([("sec".to_string(), "security".to_string())]);
        assert_eq!(apply_alias_expansion("see the section below", &table), "see the section below");
    }

    #[test]
    fn is_case_insensitive_on_the_short_form() {
        let table = HashMap::from([("sec-001".to_string(), "symlink toctou bug".to_string())]);
        assert_eq!(apply_alias_expansion("SEC-001 details", &table), "SEC-001 details symlink toctou bug");
    }
}
