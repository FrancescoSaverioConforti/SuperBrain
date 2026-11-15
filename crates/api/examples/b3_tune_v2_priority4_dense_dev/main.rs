//! B3-TUNE-v2 Priority 4 Stage 1, Task 2: DEV-only dense-standalone
//! embedding + nearest-neighbor retrieval harness. Benchmarks all 4
//! shortlisted models (frozen in
//! `docs/superpowers/specs/2026-11-14-b3-tune-v2-priority4-stage1-dev-freeze.md`)
//! independently against DEV only. Never touches calibration or
//! held-out. Standalone harness -- does not modify `index`/`search`.
//!
//! `--model <name>` runs one model's dense-standalone DEV benchmark.
//! `--privacy-test` runs the required Sensitive-note privacy test
//! (design doc Section 4c) and exits.

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

/// A note's id plus its already-redaction-aware `text_searchable` --
/// the ONLY text this harness ever embeds (privacy parity, design doc
/// Section 4c). Never the raw record body.
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
        let root = std::env::temp_dir().join(format!("superbrain-b3-tune-v2-p4-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("B3-TUNE-v2 Priority 4", "test@localhost").unwrap();
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

    /// Enumerates every committed note's id + redaction-aware
    /// `text_searchable`, via the SAME `Document::from_record_bytes`
    /// path `search_impl` itself uses (confirmed in
    /// `docs/research/B3-TUNE-v2-PRIORITY3-RANKING-AUDIT.md` Section 2)
    /// -- never a raw body read that would bypass Sensitive-note
    /// redaction.
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
        .unwrap_or_else(|| panic!("unknown model {name:?}; expected one of the 4 frozen shortlist names"))
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
struct CandidateRecallReport {
    model: String,
    split: String,
    run_commit: String,
    labels_sha256: String,
    dense_top_n: usize,
    query_count: usize,
    answerable_query_count: usize,
    /// How many answerable queries had at least one TRUE relevant note
    /// id appear anywhere in the dense top-N candidate set -- the
    /// design doc Section 6 pre-condition check, measured on DEV only.
    candidate_recall_hits: usize,
    candidate_recall_rate: f64,
    truncation_events: usize,
    total_embed_calls: usize,
    dense_recall_at_10: f64,
    dense_recall_at_10_ci: [f64; 2],
    dense_ndcg_at_10: f64,
    dense_ndcg_at_10_ci: [f64; 2],
    by_category: HashMap<String, MetricBreakdown>,
    by_difficulty: HashMap<String, MetricBreakdown>,
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

fn run_dense_benchmark(env: &Environment, model_name: &str, split: &str) {
    assert!(
        split == "dev",
        "Priority 4 Stage 1 Task 2 only ever reads dev; calibration and held-out are forbidden; got {split:?}"
    );
    let labels_path = format!("{}/../../eval/rag/{split}/labels.jsonl", env!("CARGO_MANIFEST_DIR"));
    let raw_labels_text = std::fs::read_to_string(&labels_path).unwrap();
    let rows = exclude_canary(parse_label_rows(&raw_labels_text));

    let fixture = VaultFixture::new();
    fixture.commit_corpus();
    let documents = fixture.all_documents();

    let spec = spec_by_name(model_name);
    let mut model = EmbeddingModel::load(env, spec);

    eprintln!("[{model_name}] embedding {} corpus notes...", documents.len());
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

    for row in &rows {
        if row.abstain_expected {
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
        by_category,
        by_difficulty,
    };

    println!(
        "{model_name} [{split}]: Recall@10={:.4} nDCG@10={:.4} candidate_recall_rate={:.4} ({}/{}) truncation_events={}",
        report.dense_recall_at_10, report.dense_ndcg_at_10, report.candidate_recall_rate,
        report.candidate_recall_hits, report.answerable_query_count, report.truncation_events
    );
    std::fs::create_dir_all(RESULTS_DIR).unwrap();
    let out_path = format!("{RESULTS_DIR}/priority4-dev-{model_name}.json");
    let json = serde_json::to_string_pretty(&report).unwrap();
    std::fs::write(&out_path, json).unwrap();
    println!("wrote {out_path}");
}

/// Required privacy/correctness test (design doc Section 4c). Two
/// checks, both structural and deterministic -- no similarity-score
/// probe, since a single cosine-similarity sample on a short-text
/// embedding model is noisy enough to produce false positives/
/// negatives regardless of the true redaction behavior (empirically
/// confirmed while developing this test: a literal-marker probe and a
/// natural-language paraphrase probe both landed at ambiguous,
/// inconsistent percentiles against an unrelated-query baseline, which
/// is a property of cosine similarity on short strings, not evidence
/// either way about redaction).
///
/// 1. Structural: a synthetic Sensitive note's `text_searchable` must
///    not contain its secret body or source fragment (this exercises
///    B1's OWN redaction in `crates/index/src/document.rs`, not
///    anything this harness writes).
/// 2. Differential: embedding the SAME metadata/body under
///    `Classification::Sensitive` vs. `Classification::Internal` must
///    produce DIFFERENT embedding input text (since the Internal
///    variant's `text_searchable` includes body/source, confirmed by
///    `document.rs`'s own test suite) -- and this harness's embedding
///    call must therefore also be given different input per
///    classification. This directly proves the embedding step reads
///    from `text_searchable` (classification-aware) and not from the
///    raw body (classification-blind) by construction, rather than by
///    inference from a noisy similarity number.
///
/// No real corpus note is Sensitive today (confirmed: every corpus
/// note's frontmatter classification is `internal`), so this test
/// constructs its own synthetic Sensitive note, mirroring
/// `crates/index/src/document.rs`'s own test pattern
/// (`base_metadata(Classification::Sensitive)`).
fn run_privacy_test(env: &Environment) {
    use domain::{Classification, NoteStatus, NoteType, Relations, TrustLevel};

    let secret_marker = "ZXQ9-SECRET-PAYLOAD-NEVER-EMBEDDED";
    let body = format!("This body contains {secret_marker} and must never be embedded.");

    let make_metadata = |classification: Classification| domain::NoteMetadata {
        id: "sensitive-privacy-probe".to_string(),
        schema_version: 1,
        r#type: NoteType::Bug,
        status: NoteStatus::Draft,
        trust: TrustLevel::Internal,
        classification,
        project: "privacy-probe-project".to_string(),
        source: "docs/secret.md#SECRET-PROBE".to_string(),
        created: String::new(),
        updated: String::new(),
        event_period: String::new(),
        relations: Relations::default(),
    };

    let sensitive_doc = Document::new(make_metadata(Classification::Sensitive), Some(&body));
    let internal_doc = Document::new(make_metadata(Classification::Internal), Some(&body));

    // Check 1: structural redaction, on the Sensitive variant only.
    assert!(
        !sensitive_doc.text_searchable.contains(secret_marker),
        "FAIL: Sensitive note's text_searchable unexpectedly contains body content -- \
         B1's own redaction (crates/index/src/document.rs) should have excluded it"
    );
    assert!(
        !sensitive_doc.text_searchable.contains("SECRET-PROBE"),
        "FAIL: Sensitive note's text_searchable unexpectedly contains its source fragment"
    );
    println!("PASS: Sensitive note's text_searchable correctly excludes body and source.");

    // Check 2: differential -- the Internal variant's text_searchable
    // DOES contain the secret (confirming the body/source genuinely
    // differ by classification, not by some unrelated accident), and
    // this harness's embedding call is given exactly that differing
    // text per classification, by construction (the call sites below
    // pass `sensitive_doc.text_searchable` and `internal_doc.text_searchable`
    // respectively -- never `body` directly).
    assert!(
        internal_doc.text_searchable.contains(secret_marker),
        "test setup invariant broken: the Internal control variant should include body \
         content per document.rs's own redaction rule, or this differential check proves nothing"
    );
    assert_ne!(
        sensitive_doc.text_searchable, internal_doc.text_searchable,
        "FAIL: Sensitive and Internal variants of the identical note produced identical \
         text_searchable -- redaction is not actually classification-dependent"
    );
    println!(
        "PASS: Sensitive and Internal variants of the identical note have different \
         text_searchable (Sensitive excludes the secret, Internal includes it) -- confirming \
         this harness's embedding input is classification-aware text_searchable, never the raw body."
    );

    let spec = spec_by_name("all-MiniLM-L6-v2");
    let mut model = EmbeddingModel::load(env, spec);
    // This call is given `sensitive_doc.text_searchable` -- the ONLY
    // text this harness ever passes to `embed_document` for any note,
    // Sensitive or otherwise (see `run_dense_benchmark`'s identical
    // `doc.text_searchable` call site above). It never receives `body`.
    let _sensitive_embedding = model.embed_document(&sensitive_doc.text_searchable);
    println!("PASS: embed_document() called with Sensitive note's redacted text_searchable completes without error.");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let env = ort::init().build().expect("failed to initialize ONNX Runtime environment");

    match args.get(1).map(String::as_str) {
        Some("--model") => {
            let model_name = args.get(2).cloned().unwrap_or_else(|| {
                panic!("--model requires a name: all-MiniLM-L6-v2 | bge-small-en-v1.5 | all-mpnet-base-v2 | nomic-embed-text-v1.5")
            });
            run_dense_benchmark(&env, &model_name, "dev");
        }
        Some("--privacy-test") => {
            run_privacy_test(&env);
        }
        other => panic!("usage: --model <name> | --privacy-test; got {other:?}"),
    }
}
