//! B4.2: production implementation of `index::search::DenseScorer`,
//! compiled only under the `dense-retrieval` feature (B4.1).
//!
//! Reuses the frozen representation semantics the Priority 4 evaluation
//! harness already validated
//! (`eval/results/B3-TUNE-v2/frozen-config-v2.json`,
//! `crates/api/examples/priority4_common/embedding.rs`) -- not
//! re-derived from memory, and not imported from that example-only code
//! (per the design review's own "what must not enter the production
//! path" list,
//! `docs/research/B3-TUNE-v2-PRIORITY4-DESIGN-REVIEW.md`). Closes one
//! real gap the evaluation harness never had to close: this module
//! verifies the fetched ONNX/tokenizer artifacts' SHA-256 against
//! `frozen-config-v2.json`'s own recorded hashes before loading either
//! one, per B4.1's "Pinning/hash verification" decision
//! (`docs/superpowers/specs/2026-11-15-b4-1-dependency-promotion-decisions.md`).
//! A mismatch is a hard load failure, not a silent continue.

/// Model identity, verbatim from `eval/results/B3-TUNE-v2/frozen-config-v2.json`'s
/// `model_runtime` block -- the single accepted v2 configuration. Not a
/// parameter: this module implements exactly this model, nothing else.
pub const HF_REPO: &str = "sentence-transformers/all-MiniLM-L6-v2";
pub const PINNED_REVISION_SHA: &str = "1110a243fdf4706b3f48f1d95db1a4f5529b4d41";
pub const ONNX_ARTIFACT_SHA256: &str =
    "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452";
pub const TOKENIZER_ARTIFACT_SHA256: &str =
    "be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037";
pub const EMBEDDING_DIMENSION: usize = 384;
/// Frozen representation semantics (`frozen-config-v2.json`'s
/// `representation_semantics` block): no query/document prefix for
/// `all-MiniLM-L6-v2` specifically (unlike BGE/Nomic in the shortlist
/// audit, which DO require one -- this module implements only MiniLM).
pub const QUERY_PREFIX: &str = "";
pub const DOCUMENT_PREFIX: &str = "";
pub const MAX_SEQUENCE_LENGTH: usize = 256;

/// Hash-verification outcome for one artifact. Deliberately NOT a
/// `Result<(), Error>` -- per B4.0's frozen fallback contract
/// (`DenseScorer::score` returns `Option`, never a `SearchError`, for a
/// structurally-unavailable dense path), a hash mismatch at load time is
/// exactly that kind of structural unavailability, so this function's
/// caller (the real model-loading code, built in a later B4.2 commit)
/// turns a `Mismatch` into "dense path unavailable," never a panic
/// reaching a `DenseScorer::score` caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashCheck {
    Match,
    Mismatch,
}

/// Verifies `bytes`' SHA-256 against `expected_hex` (a 64-character
/// lowercase hex string, e.g. `ONNX_ARTIFACT_SHA256`). Case-insensitive
/// on `expected_hex`'s own casing (so a hash copied with uppercase hex
/// still matches), but the computed digest is always compared as its
/// canonical lowercase form.
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> HashCheck {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = format!("{:x}", hasher.finalize());
    if actual.eq_ignore_ascii_case(expected_hex) {
        HashCheck::Match
    } else {
        HashCheck::Mismatch
    }
}

/// Why `EmbeddingModel::load` can fail, distinct from `DenseScorer::score`'s
/// own `Option` return: this is the LOADING step's own error type, used by
/// B4.6's wiring code to decide whether to construct a working
/// `DenseScorer` implementation at all. A `LoadError` here is exactly one
/// of B4.0's frozen "structurally unavailable" conditions once B4.6 wires
/// it through -- never a panic reaching a caller outside this module.
#[derive(Debug)]
pub enum LoadError {
    ArtifactReadFailed {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    HashMismatch {
        path: std::path::PathBuf,
        expected: String,
    },
    TokenizerLoadFailed(String),
    OnnxSessionFailed(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArtifactReadFailed { path, source } => {
                write!(f, "failed to read artifact at {path:?}: {source}")
            }
            Self::HashMismatch { path, expected } => {
                write!(f, "hash mismatch for {path:?}: expected {expected}")
            }
            Self::TokenizerLoadFailed(msg) => write!(f, "tokenizer load failed: {msg}"),
            Self::OnnxSessionFailed(msg) => write!(f, "ONNX session load failed: {msg}"),
        }
    }
}
impl std::error::Error for LoadError {}

/// Reads `path`, verifies its SHA-256 against `expected_hex`, and returns
/// the verified bytes. A missing/unreadable file and a hash mismatch are
/// distinct `LoadError` variants (B4.8's packaging diagnostics need to
/// tell these apart: "artifact missing, run setup" vs. "artifact present
/// but corrupted/tampered/wrong revision" are different operator actions).
fn read_and_verify(path: &std::path::Path, expected_hex: &str) -> Result<Vec<u8>, LoadError> {
    let bytes = std::fs::read(path).map_err(|source| LoadError::ArtifactReadFailed {
        path: path.to_path_buf(),
        source,
    })?;
    match verify_sha256(&bytes, expected_hex) {
        HashCheck::Match => Ok(bytes),
        HashCheck::Mismatch => Err(LoadError::HashMismatch {
            path: path.to_path_buf(),
            expected: expected_hex.to_string(),
        }),
    }
}

/// Per-call record of whether truncation actually fired for a given text
/// -- mirrors the evaluation harness's own `EmbedResult`'s truncation-
/// transparency field, never silently absorbed.
#[derive(Debug, Clone)]
pub struct EmbedResult {
    pub vector: Vec<f32>,
    pub truncated: bool,
}

/// Process-global `ort::environment::Environment`, initialized exactly
/// once (B4.0's frozen "ONNX Runtime lifecycle" contract: one
/// `Environment` per process, never re-initialized per search request).
/// `ort` itself enforces this at runtime (a second
/// `ort::init().build()` call in the same process errors with "only
/// one environment is allowed per process," confirmed directly by this
/// module's own test suite colliding under parallel test execution
/// before this helper existed) -- `std::sync::OnceLock` makes the
/// single-initialization requirement structural rather than a
/// caller-discipline convention every call site has to remember.
static SHARED_ENVIRONMENT: std::sync::OnceLock<ort::environment::Environment> =
    std::sync::OnceLock::new();

/// Returns the process-wide `Environment`, initializing it on first
/// call. Every production and test call site should go through this,
/// never call `ort::init().build()` directly.
pub fn shared_environment() -> &'static ort::environment::Environment {
    SHARED_ENVIRONMENT.get_or_init(|| ort::init().build().expect("ONNX Runtime environment init"))
}

/// The real, loaded `all-MiniLM-L6-v2` ONNX model + tokenizer, hash-
/// verified at load time. Implements the SAME representation semantics
/// (`QUERY_PREFIX`/`DOCUMENT_PREFIX` empty, `MAX_SEQUENCE_LENGTH` = 256,
/// mean pooling over the attention mask, L2 normalization, cosine
/// similarity) the Priority 4 evaluation harness already validated --
/// re-implemented fresh against the frozen spec, not imported from the
/// example-only code (module doc comment).
pub struct EmbeddingModel {
    session: ort::session::Session,
    tokenizer: tokenizers::Tokenizer,
}

impl EmbeddingModel {
    /// Loads the model from `onnx_path`/`tokenizer_path`, verifying both
    /// artifacts' SHA-256 against `ONNX_ARTIFACT_SHA256`/
    /// `TOKENIZER_ARTIFACT_SHA256` before ever handing their bytes to
    /// `ort`/`tokenizers`. A hash mismatch is a hard `LoadError`, never a
    /// silent continue with unverified content.
    pub fn load(
        env: &ort::environment::Environment,
        onnx_path: &std::path::Path,
        tokenizer_path: &std::path::Path,
    ) -> Result<Self, LoadError> {
        let _onnx_bytes = read_and_verify(onnx_path, ONNX_ARTIFACT_SHA256)?;
        let tokenizer_bytes = read_and_verify(tokenizer_path, TOKENIZER_ARTIFACT_SHA256)?;

        // `ort::session::Session::builder(...).commit_from_file(...)` reads
        // the file itself rather than taking already-read bytes -- the
        // hash verification above still ran against the exact bytes on
        // disk before this call, so a verified-then-swapped file between
        // these two steps is the only gap, which is not a realistic
        // local-filesystem TOCTOU this module is positioned to close (the
        // same class of risk every "verify then use a path" pattern has;
        // B4.8's packaging/setup flow is responsible for the artifact's
        // provenance, not a re-read-to-bytes defense here).
        let session = ort::session::Session::builder(env)
            .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?
            .commit_from_file(onnx_path)
            .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?;

        let mut tokenizer = tokenizers::Tokenizer::from_bytes(&tokenizer_bytes)
            .map_err(|e| LoadError::TokenizerLoadFailed(e.to_string()))?;

        // The published tokenizer.json ships its OWN baked-in truncation
        // (max_length: 128) and fixed padding (128) -- confirmed by
        // reading the real file's `truncation`/`padding` blocks directly,
        // not assumed. Left as-is, `encode(...)` would silently truncate
        // every input to 128 tokens before this module's own
        // MAX_SEQUENCE_LENGTH (256, per frozen-config-v2.json) ever sees
        // it, making the `truncated` flag this module reports
        // permanently false regardless of real input length -- exactly
        // the latent bug this override exists to close (found by this
        // module's own test suite actually exercising a long input,
        // not assumed safe because the evaluation harness's equivalent
        // counter also always read zero). Override both explicitly to
        // the frozen contract's own values.
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: MAX_SEQUENCE_LENGTH,
                strategy: tokenizers::TruncationStrategy::LongestFirst,
                stride: 0,
                direction: tokenizers::TruncationDirection::Right,
            }))
            .map_err(|e| LoadError::TokenizerLoadFailed(e.to_string()))?;
        tokenizer.with_padding(None);

        Ok(Self { session, tokenizer })
    }

    pub fn embed_document(&mut self, text: &str) -> Result<EmbedResult, LoadError> {
        let prefixed = format!("{DOCUMENT_PREFIX}{text}");
        self.embed_raw(&prefixed)
    }

    pub fn embed_query(&mut self, text: &str) -> Result<EmbedResult, LoadError> {
        let prefixed = format!("{QUERY_PREFIX}{text}");
        self.embed_raw(&prefixed)
    }

    fn embed_raw(&mut self, text: &str) -> Result<EmbedResult, LoadError> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| LoadError::TokenizerLoadFailed(e.to_string()))?;
        // `truncated` must come from the tokenizer's own overflow record,
        // not from comparing `ids.len()` to `MAX_SEQUENCE_LENGTH`: once
        // `with_truncation` (set at load time, above) has already
        // truncated the encoding to exactly `MAX_SEQUENCE_LENGTH`,
        // `ids.len()` can never exceed it, so a length-based check can
        // never detect that truncation occurred -- a real bug this
        // module's own test suite caught (a text proven, by direct
        // measurement, to tokenize to far more than 256 ids was
        // incorrectly reported as `truncated: false`, because by the
        // time this code saw `ids`, the tokenizer had already cut it
        // down to exactly 256). `get_overflowing()` is non-empty if and
        // only if truncation actually happened, which is the direct
        // signal, not an inference from a length that is ALWAYS capped
        // by construction once truncation is configured at the
        // tokenizer level.
        let truncated = !encoding.get_overflowing().is_empty();
        let ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
        let mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&x| x as i64)
            .collect();
        let seq_len = ids.len();

        let input_ids = ort::value::Tensor::from_array(([1usize, seq_len], ids))
            .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?;
        let attention_mask = ort::value::Tensor::from_array(([1usize, seq_len], mask.clone()))
            .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?;

        let mut input_names: Vec<&str> = vec!["input_ids", "attention_mask"];
        let has_token_type = self
            .session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        let token_type_ids = if has_token_type {
            input_names.push("token_type_ids");
            Some(
                ort::value::Tensor::from_array(([1usize, seq_len], vec![0i64; seq_len]))
                    .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?,
            )
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
                .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?
        } else {
            self.session
                .run(ort::inputs![
                    "input_ids" => input_ids,
                    "attention_mask" => attention_mask,
                ])
                .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?
        };

        let (_output_name, output_value) = outputs.iter().next().ok_or_else(|| {
            LoadError::OnnxSessionFailed("model produced no output tensors".to_string())
        })?;
        let hidden_states: ndarray::ArrayViewD<f32> = output_value
            .try_extract_array::<f32>()
            .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?;
        let hidden_states = hidden_states
            .into_dimensionality::<ndarray::Ix3>()
            .map_err(|e| LoadError::OnnxSessionFailed(e.to_string()))?;

        let pooled = mean_pool(hidden_states.view(), &mask);
        let normalized = l2_normalize(&pooled);

        Ok(EmbedResult {
            vector: normalized.to_vec(),
            truncated,
        })
    }
}

fn mean_pool(
    hidden_states: ndarray::ArrayView3<f32>,
    attention_mask: &[i64],
) -> ndarray::Array1<f32> {
    let (_batch, seq_len, hidden) = hidden_states.dim();
    let mut sum = ndarray::Array1::<f32>::zeros(hidden);
    let mut mask_sum = 0.0f32;
    let batch0 = hidden_states.index_axis(ndarray::Axis(0), 0);
    for (t, &mask_value) in attention_mask.iter().enumerate().take(seq_len) {
        let m = mask_value as f32;
        if m == 0.0 {
            continue;
        }
        let row = batch0.index_axis(ndarray::Axis(0), t);
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

/// Cosine similarity for two already-L2-normalized vectors (dot product
/// IS cosine similarity here, a direct consequence of `l2_normalize`
/// above having already run on both -- not an independent assumption).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_hashes_are_exactly_64_lowercase_hex_characters() {
        // Pins the exact format this module's verify_sha256 compares
        // against -- a malformed constant here would silently make every
        // real artifact check fail, not just a test.
        for (name, hash) in [
            ("onnx", ONNX_ARTIFACT_SHA256),
            ("tokenizer", TOKENIZER_ARTIFACT_SHA256),
        ] {
            assert_eq!(hash.len(), 64, "{name} hash must be 64 hex characters");
            assert!(
                hash.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{name} hash must be lowercase hex: {hash:?}"
            );
        }
    }

    // sha256("hello world"), independently computed with `hashlib.sha256`
    // (Python) during this test's own authoring, not recalled from memory
    // -- a memorized "well-known" test vector turned out to be wrong when
    // actually checked, which is exactly the discipline this comment
    // exists to record.
    const SHA256_HELLO_WORLD: &str =
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    #[test]
    fn hello_world_test_vector_is_64_hex_characters() {
        assert_eq!(SHA256_HELLO_WORLD.len(), 64);
    }

    #[test]
    fn verify_sha256_matches_known_content_against_its_real_digest() {
        assert_eq!(
            verify_sha256(b"hello world", SHA256_HELLO_WORLD),
            HashCheck::Match
        );
    }

    #[test]
    fn verify_sha256_rejects_mismatched_content() {
        assert_eq!(
            verify_sha256(b"goodbye world", SHA256_HELLO_WORLD),
            HashCheck::Mismatch
        );
    }

    #[test]
    fn verify_sha256_is_case_insensitive_on_the_expected_argument() {
        let upper = SHA256_HELLO_WORLD.to_uppercase();
        assert_eq!(verify_sha256(b"hello world", &upper), HashCheck::Match);
    }

    // --- Real model load + inference. Requires the actual, hash-verified
    // all-MiniLM-L6-v2 ONNX + tokenizer artifacts on disk -- fetched once
    // by the Priority 4 evaluation harness and reused here unchanged
    // (hash re-verified by this test itself, independently, before
    // trusting them). Skips cleanly (prints a message, does not fail)
    // if the artifacts are not present on this machine, rather than
    // requiring every contributor's machine to have them. ---

    fn cached_artifact_paths() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        // Reuses the Priority 4 evaluation harness's own cache from the
        // b3-retrieval-eval worktree, if present, rather than requiring a
        // second ~90MB fetch for the same already-hash-verified files.
        // Production code itself uses .superbrain-model-cache/ (B4.1
        // decision record) -- this path is this TEST's own convenience
        // only, not a production convention.
        let candidates = [
            std::path::PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.worktrees/b3-retrieval-eval/.priority4-model-cache/all-MiniLM-L6-v2"
            )),
            std::path::PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.priority4-model-cache/all-MiniLM-L6-v2"
            )),
        ];
        for dir in candidates {
            let onnx = dir.join("model.onnx");
            let tokenizer = dir.join("tokenizer.json");
            if onnx.exists() && tokenizer.exists() {
                return Some((onnx, tokenizer));
            }
        }
        None
    }

    #[test]
    fn load_rejects_a_tampered_onnx_artifact_before_touching_ort() {
        let Some((_onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let dir = std::env::temp_dir().join(format!(
            "superbrain-dense-tamper-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let tampered_onnx = dir.join("model.onnx");
        std::fs::write(&tampered_onnx, b"not actually an onnx model").unwrap();

        let env = shared_environment();
        let result = EmbeddingModel::load(env, &tampered_onnx, &tokenizer_path);
        let _ = std::fs::remove_dir_all(&dir);

        match result {
            Err(LoadError::HashMismatch { .. }) => {}
            Err(other) => panic!(
                "expected HashMismatch for a tampered ONNX file, got a different error: {other}"
            ),
            Ok(_) => panic!("expected HashMismatch for a tampered ONNX file, got Ok"),
        }
    }

    #[test]
    fn load_and_embed_real_minilm_produces_a_384_dim_l2_normalized_vector() {
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let env = shared_environment();
        let mut model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path)
            .expect("hash-verified real artifacts must load successfully");

        let result = model
            .embed_query("find the bug fix for the login timeout")
            .expect("embedding a real query must succeed");

        assert_eq!(
            result.vector.len(),
            EMBEDDING_DIMENSION,
            "MiniLM's own frozen embedding dimension"
        );
        assert!(
            !result.truncated,
            "a short query must not be truncated at 256 tokens"
        );

        let norm: f32 = result.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-4,
            "the returned vector must be L2-normalized (norm ~= 1.0), got {norm}"
        );
    }

    #[test]
    fn identical_text_produces_identical_embeddings_deterministically() {
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let env = shared_environment();
        let mut model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();

        let a = model
            .embed_document("deterministic embedding probe text")
            .unwrap();
        let b = model
            .embed_document("deterministic embedding probe text")
            .unwrap();

        assert_eq!(
            a.vector, b.vector,
            "the same text must embed to the exact same vector every time"
        );
    }

    #[test]
    fn different_text_produces_a_lower_cosine_similarity_than_identical_text() {
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let env = shared_environment();
        let mut model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();

        let base = model
            .embed_document("the quick brown fox jumps over the lazy dog")
            .unwrap();
        let same = model
            .embed_document("the quick brown fox jumps over the lazy dog")
            .unwrap();
        let different = model
            .embed_document("quarterly financial report for the fiscal year")
            .unwrap();

        let sim_same = cosine_similarity(&base.vector, &same.vector);
        let sim_different = cosine_similarity(&base.vector, &different.vector);

        assert!(
            sim_same > sim_different,
            "identical text must be more similar to itself ({sim_same}) than to unrelated text ({sim_different})"
        );
        assert!(
            (sim_same - 1.0).abs() < 1e-4,
            "identical text embedded twice must have cosine similarity ~= 1.0, got {sim_same}"
        );
    }

    #[test]
    fn a_text_longer_than_256_tokens_is_reported_as_truncated() {
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let env = shared_environment();
        let mut model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();

        // 500 distinct numbered words -- comfortably over 256 word-pieces
        // once tokenized, and avoids relying on how a repeated identical
        // token happens to be handled by WordPiece tokenization (a single
        // repeated word was tried first and did NOT reliably exceed 256
        // tokens for this tokenizer -- caught by actually running the test
        // against the real tokenizer rather than assuming word count maps
        // 1:1 to token count).
        let long_text: String = (0..500).map(|i| format!("uniqueword{i} ")).collect();
        let result = model.embed_document(&long_text).unwrap();

        assert!(
            result.truncated,
            "a text this long must be reported as truncated at MAX_SEQUENCE_LENGTH"
        );
        assert_eq!(
            result.vector.len(),
            EMBEDDING_DIMENSION,
            "truncation must not change the output dimension"
        );
    }
}
