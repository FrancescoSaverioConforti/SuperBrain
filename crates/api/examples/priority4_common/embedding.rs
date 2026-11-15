//! Shared ONNX embedding-model wrapper for B3-TUNE-v2 Priority 4 Stage 1
//! (DEV-only). Not a production dependency of `index`/`search` — this
//! file is `include!`d by example binaries only, exactly the duplication
//! convention Priority 2/3's own examples already use across the crate
//! boundary (see `b3_tune_v2_priority3_p1_identifier_boost.rs`'s own doc
//! comment on `identifier_boost_would_fire`).
//!
//! Representation semantics implemented here are frozen in
//! `docs/superpowers/specs/2026-11-14-b3-tune-v2-priority4-stage1-dev-freeze.md`
//! Section 2 — per-model prefix, pooling, and normalization come from
//! that document, not invented here.

use ndarray::{Array2, ArrayViewD, Axis};
use ort::environment::Environment;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use std::path::Path;
use tokenizers::Tokenizer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolingMode {
    Mean,
    ClsToken,
}

/// One shortlisted model's frozen representation semantics (Task 0 spec,
/// Section 2). Nothing here is tuned after seeing results — every field
/// is copied verbatim from the frozen spec.
pub struct ModelSpec {
    pub name: &'static str,
    pub hf_repo: &'static str,
    pub pinned_sha: &'static str,
    pub max_seq_len: usize,
    pub query_prefix: &'static str,
    pub document_prefix: &'static str,
    pub pooling: PoolingMode,
}

pub const MODEL_SPECS: [ModelSpec; 4] = [
    ModelSpec {
        name: "all-MiniLM-L6-v2",
        hf_repo: "sentence-transformers/all-MiniLM-L6-v2",
        pinned_sha: "1110a243fdf4706b3f48f1d95db1a4f5529b4d41",
        max_seq_len: 256,
        query_prefix: "",
        document_prefix: "",
        pooling: PoolingMode::Mean,
    },
    ModelSpec {
        name: "bge-small-en-v1.5",
        hf_repo: "BAAI/bge-small-en-v1.5",
        pinned_sha: "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a",
        max_seq_len: 512,
        query_prefix: "Represent this sentence for searching relevant passages: ",
        document_prefix: "",
        pooling: PoolingMode::ClsToken,
    },
    ModelSpec {
        name: "all-mpnet-base-v2",
        hf_repo: "sentence-transformers/all-mpnet-base-v2",
        pinned_sha: "e8c3b32edf5434bc2275fc9bab85f82640a19130",
        max_seq_len: 384,
        query_prefix: "",
        document_prefix: "",
        pooling: PoolingMode::Mean,
    },
    ModelSpec {
        name: "nomic-embed-text-v1.5",
        hf_repo: "nomic-ai/nomic-embed-text-v1.5",
        pinned_sha: "e9b6763023c676ca8431644204f50c2b100d9aab",
        max_seq_len: 8192,
        query_prefix: "search_query: ",
        document_prefix: "search_document: ",
        pooling: PoolingMode::Mean,
    },
];

/// Where fetched model artifacts (ONNX graph + tokenizer) are cached
/// locally, under the workspace's own scratch area — never a system
/// temp directory shared with unrelated processes, and never committed
/// (see Task 2's `.gitignore` entry).
pub fn model_cache_dir(model_name: &str) -> std::path::PathBuf {
    std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.priority4-model-cache"))
        .join(model_name)
}

/// Fetches `onnx/model.onnx` and `tokenizer.json` for one model, from its
/// pinned HF revision, if not already cached. Returns (onnx_path,
/// tokenizer_path). This is the ONLY place this harness performs a
/// network fetch, and only for the 4 authorized shortlist models at
/// their frozen pinned `sha` — never anything else.
pub fn ensure_model_fetched(spec: &ModelSpec) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = model_cache_dir(spec.name);
    std::fs::create_dir_all(&dir).unwrap();
    let onnx_path = dir.join("model.onnx");
    let tokenizer_path = dir.join("tokenizer.json");

    if !onnx_path.exists() {
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/onnx/model.onnx",
            spec.hf_repo, spec.pinned_sha
        );
        fetch_to_file(&url, &onnx_path);
    }
    if !tokenizer_path.exists() {
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/tokenizer.json",
            spec.hf_repo, spec.pinned_sha
        );
        fetch_to_file(&url, &tokenizer_path);
    }
    (onnx_path, tokenizer_path)
}

/// Generous ceiling for a single model artifact fetch (ONNX graph or
/// tokenizer file) -- well above any of the 4 shortlisted models'
/// largest expected file (ureq's own default is 10MB, too small for
/// several of these ONNX exports).
const MAX_FETCH_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn fetch_to_file(url: &str, dest: &Path) {
    let agent = ureq::Agent::new_with_defaults();
    let mut response = agent
        .get(url)
        .call()
        .unwrap_or_else(|e| panic!("fetch failed for {url}: {e}"));
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_FETCH_BYTES)
        .read_to_vec()
        .unwrap_or_else(|e| panic!("read body failed for {url}: {e}"));
    std::fs::write(dest, &bytes).unwrap_or_else(|e| panic!("write failed for {dest:?}: {e}"));
}

pub struct EmbeddingModel {
    session: Session,
    tokenizer: Tokenizer,
    spec: ModelSpec,
}

/// Per-call record of whether truncation actually fired for a given
/// text, per the frozen spec's truncation-transparency requirement
/// (Section 3 of the freeze doc) — never silently absorbed.
pub struct EmbedResult {
    pub vector: Vec<f32>,
    pub truncated: bool,
    pub token_count_before_truncation: usize,
}

impl EmbeddingModel {
    pub fn load(env: &Environment, spec: ModelSpec) -> Self {
        let (onnx_path, tokenizer_path) = ensure_model_fetched(&spec);
        let session = Session::builder(env)
            .unwrap()
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .unwrap()
            .commit_from_file(&onnx_path)
            .unwrap_or_else(|e| panic!("failed to load ONNX model at {onnx_path:?}: {e}"));
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .unwrap_or_else(|e| panic!("failed to load tokenizer at {tokenizer_path:?}: {e}"));
        Self { session, tokenizer, spec }
    }

    pub fn spec(&self) -> &ModelSpec {
        &self.spec
    }

    /// Embeds one document text under this model's frozen document-side
    /// prefix/truncation/pooling/normalization rule (Task 0 spec, Section
    /// 2). `text` must already be B1's own redaction-aware
    /// `text_searchable` (Section 4c privacy parity) — this function
    /// does not know or care where `text` came from, which is exactly
    /// the point: privacy parity is enforced by the CALLER passing the
    /// right field, not by this embedding function trying to redact
    /// anything itself.
    pub fn embed_document(&mut self, text: &str) -> EmbedResult {
        let prefixed = format!("{}{}", self.spec.document_prefix, text);
        self.embed_raw(&prefixed)
    }

    pub fn embed_query(&mut self, text: &str) -> EmbedResult {
        let prefixed = format!("{}{}", self.spec.query_prefix, text);
        self.embed_raw(&prefixed)
    }

    fn embed_raw(&mut self, text: &str) -> EmbedResult {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .unwrap_or_else(|e| panic!("tokenization failed: {e}"));
        let token_count_before_truncation = encoding.get_ids().len();
        let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
        let mut mask: Vec<i64> = encoding.get_attention_mask().iter().map(|&x| x as i64).collect();
        let truncated = ids.len() > self.spec.max_seq_len;
        if truncated {
            ids.truncate(self.spec.max_seq_len);
            mask.truncate(self.spec.max_seq_len);
        }
        let seq_len = ids.len();

        let input_ids = Tensor::from_array(([1usize, seq_len], ids)).unwrap();
        let attention_mask = Tensor::from_array(([1usize, seq_len], mask.clone())).unwrap();

        let mut input_names: Vec<&str> = vec!["input_ids", "attention_mask"];
        let has_token_type = self.session.inputs().iter().any(|i| i.name() == "token_type_ids");
        let token_type_ids = if has_token_type {
            input_names.push("token_type_ids");
            Some(Tensor::from_array(([1usize, seq_len], vec![0i64; seq_len])).unwrap())
        } else {
            None
        };

        let outputs = if let Some(token_type_ids) = token_type_ids {
            self.session
                .run(ort::inputs![
                    "input_ids" => input_ids,
                    "attention_mask" => attention_mask,
                    "token_type_ids" => token_type_ids,
                ])
                .unwrap_or_else(|e| panic!("ONNX inference failed: {e}"))
        } else {
            self.session
                .run(ort::inputs![
                    "input_ids" => input_ids,
                    "attention_mask" => attention_mask,
                ])
                .unwrap_or_else(|e| panic!("ONNX inference failed: {e}"))
        };

        // First output is the token-level hidden states
        // (last_hidden_state), shape [1, seq_len, hidden_size], for every
        // one of the 4 shortlisted models' published ONNX export.
        let (_output_name, output_value) = outputs.iter().next().unwrap();
        let hidden_states: ArrayViewD<f32> =
            output_value.try_extract_array::<f32>().unwrap_or_else(|e| {
                panic!("failed to extract hidden states tensor: {e}")
            });
        let hidden_states = hidden_states
            .into_dimensionality::<ndarray::Ix3>()
            .unwrap_or_else(|e| panic!("unexpected output rank: {e}"));

        let pooled = match self.spec.pooling {
            PoolingMode::Mean => mean_pool(hidden_states.view(), &mask),
            PoolingMode::ClsToken => {
                let cls_row = hidden_states.index_axis(Axis(1), 0);
                cls_row.index_axis(Axis(0), 0).to_owned()
            }
        };
        let normalized = l2_normalize(&pooled);

        EmbedResult {
            vector: normalized.to_vec(),
            truncated,
            token_count_before_truncation,
        }
    }
}

fn mean_pool(hidden_states: ndarray::ArrayView3<f32>, attention_mask: &[i64]) -> ndarray::Array1<f32> {
    let (_batch, seq_len, hidden) = hidden_states.dim();
    let mut sum = ndarray::Array1::<f32>::zeros(hidden);
    let mut mask_sum = 0.0f32;
    let batch0 = hidden_states.index_axis(Axis(0), 0);
    for t in 0..seq_len {
        let m = attention_mask[t] as f32;
        if m == 0.0 {
            continue;
        }
        let row = batch0.index_axis(Axis(0), t);
        sum = sum + row.mapv(|v| v * m);
        mask_sum += m;
    }
    if mask_sum == 0.0 {
        sum
    } else {
        sum / mask_sum
    }
}

fn l2_normalize(v: &ndarray::Array1<f32>) -> ndarray::Array1<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        v.clone()
    } else {
        v / norm
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    // Vectors are already L2-normalized by `embed_raw`, so dot product
    // IS cosine similarity here -- not an independent assumption, a
    // direct consequence of the frozen normalization rule (Task 0 spec,
    // Section 2: "Embedding normalization: L2-normalize" for all 4
    // models).
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[allow(dead_code)]
pub fn matrix_from_vectors(vectors: &[Vec<f32>]) -> Array2<f32> {
    let rows = vectors.len();
    let cols = vectors.first().map(|v| v.len()).unwrap_or(0);
    let mut m = Array2::<f32>::zeros((rows, cols));
    for (i, v) in vectors.iter().enumerate() {
        for (j, x) in v.iter().enumerate() {
            m[[i, j]] = *x;
        }
    }
    m
}
