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
            Some(QueryScores {
                top1_score,
                top2_score,
                margin,
                ratio,
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut split = "dev".to_string();
    let mut conclusion: Option<String> = None;
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
            other => panic!("unrecognized argument: {other}"),
        }
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
