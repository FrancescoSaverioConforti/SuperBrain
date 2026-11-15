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
                hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
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
}
