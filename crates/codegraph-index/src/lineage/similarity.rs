//! B2 Task 6: the similarity score behind a `Hypothesized` lineage edge's
//! [`codegraph_domain::Confidence`]. Never used to justify `Confirmed` —
//! see `resolver.rs`'s module doc comment for why a similarity score,
//! however high, can only ever produce `Hypothesized`.
//!
//! # Features, weights, range
//!
//! Two features, each already in `[0.0, 1.0]`, combined by a fixed
//! weighted average:
//!
//! - **Name similarity** (weight 0.4): `1.0` if the two snapshots' names
//!   are byte-for-byte equal, `0.0` otherwise. Deliberately binary, not a
//!   string-edit-distance metric: this pipeline does not yet have a
//!   calibrated distance function for "two names are probably the same
//!   thing renamed" (e.g. `fetch_user` vs `get_user` vs `fetchUser`)
//!   that would not itself need its own threshold/weight tuning — adding
//!   one is exactly the kind of speculative sophistication the task
//!   asked to avoid ("non usare un numero opaco senza semantica"). A
//!   binary name match is the one case this pipeline currently has solid
//!   grounds to weight: identical names at the same kind are strong
//!   (but still only heuristic — never deterministic) evidence of
//!   continuity even across a file move.
//! - **Signature similarity** (weight 0.6): the fraction of the SHORTER
//!   signature's bytes that form a contiguous common substring with the
//!   LONGER one, via a simple longest-common-substring search —
//!   `longest_common_substring_len / shorter_signature_len`. Weighted
//!   higher than name similarity because a renamed function's BODY/
//!   PARAMETER LIST is the more informative continuity signal in this
//!   pipeline's signature text (which already includes parameters and,
//!   in several languages, the body) — a rename alone leaves the
//!   signature's non-name bytes largely intact, so this feature is the
//!   one that actually distinguishes "renamed `foo` with the same body"
//!   from "an unrelated new function that happens to be named `foo`
//!   again."
//!
//! `score = 0.4 * name_similarity + 0.6 * signature_similarity`, always
//! in `[0.0, 1.0]` since both inputs are. Fed directly to
//! `codegraph_domain::Confidence::from_score`, whose buckets (fixed at
//! Task 1, unchanged here) mean: `>= 0.85` Certain, `>= 0.5` Likely,
//! otherwise Speculative — but ALL THREE buckets, including `Certain`,
//! only ever justify `Hypothesized`, never `Confirmed` (see this
//! module's and `resolver.rs`'s doc comments for why). A `Certain`-
//! bucketed `Hypothesized` edge communicates "the heuristic is very
//! confident," never "this is a structural fact," and the plan's
//! explicit instruction is that even very high similarity-based
//! confidence must never auto-promote to `Confirmed`.

/// Computes the weighted similarity score described in this module's
/// doc comment. Always returns a finite value in `[0.0, 1.0]`.
pub fn similarity_score(name_a: &str, name_b: &str, signature_a: &str, signature_b: &str) -> f64 {
    let name_similarity = if name_a == name_b { 1.0 } else { 0.0 };
    let signature_similarity = signature_similarity(signature_a, signature_b);
    0.4 * name_similarity + 0.6 * signature_similarity
}

/// `longest_common_substring_len(a, b) / shorter_len`, or `0.0` if either
/// input is empty (an empty signature has no meaningful substring
/// overlap to report, and dividing by a zero shorter-length would
/// otherwise need its own special case here instead of a clean `0.0`).
fn signature_similarity(a: &str, b: &str) -> f64 {
    let shorter_len = a.len().min(b.len());
    if shorter_len == 0 {
        return 0.0;
    }
    let lcs_len = longest_common_substring_len(a.as_bytes(), b.as_bytes());
    lcs_len as f64 / shorter_len as f64
}

/// Classic O(n*m) dynamic-programming longest-common-SUBSTRING (contiguous),
/// not longest-common-SUBSEQUENCE (which would allow gaps and overstate
/// similarity between two signatures that merely share scattered
/// characters). Operates on bytes, not chars: this pipeline's signatures
/// are already-valid source text where the relevant shared structure
/// (keywords, parameter lists, punctuation) is ASCII, and byte-level
/// comparison avoids needing a UTF-8 grapheme-boundary-aware algorithm
/// for a similarity HEURISTIC that is explicitly never load-bearing for
/// `Confirmed` status.
fn longest_common_substring_len(a: &[u8], b: &[u8]) -> usize {
    // No empty-input early return: the loop below already yields 0 for
    // either empty slice on its own (an empty `a` never enters the outer
    // loop; an empty `b` makes every `curr_row` identically 0) — a
    // separate guard here was a redundant, correctness-irrelevant
    // optimization. Removed rather than kept as an unprovable branch
    // (no test could distinguish `||` from `&&` here without an
    // allocation-count assertion that has nothing to do with this
    // function's actual contract) once the mutation sweep surfaced it.
    let mut prev_row = vec![0usize; b.len() + 1];
    let mut best = 0;
    for &byte_a in a {
        let mut curr_row = vec![0usize; b.len() + 1];
        for (j, &byte_b) in b.iter().enumerate() {
            if byte_a == byte_b {
                curr_row[j + 1] = prev_row[j] + 1;
                best = best.max(curr_row[j + 1]);
            }
        }
        prev_row = curr_row;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_name_and_signature_score_one() {
        let score = similarity_score("foo", "foo", "fn foo()", "fn foo()");
        assert_eq!(score, 1.0);
    }

    #[test]
    fn very_different_name_and_signature_score_low() {
        // Not exactly 0.0: even unrelated text can share a few
        // incidental bytes (spaces, common letters) via the
        // longest-common-substring measure — the requirement is a LOW
        // score, not a literal zero, which this test now checks against
        // the computed value rather than an assumed one.
        let score = similarity_score("foo", "xyz", "fn foo() { 1 }", "struct Unrelated;");
        assert!(score < 0.3, "expected a low score for unrelated name/signature, got {score}");
    }

    #[test]
    fn renamed_function_with_identical_body_scores_meaningfully_above_the_hypothesis_floor() {
        // Different name (name_similarity = 0.0), but the signature's
        // non-name bytes (parameters, body) are identical, so the
        // signature-similarity term alone should still drive the score
        // meaningfully above the resolver's MIN_HYPOTHESIS_SCORE floor
        // (0.3) — not asserting an exact value, since that depends on
        // exactly how long the shared substring is relative to the
        // whole signature, which is incidental to THIS test's point
        // (that signature similarity alone, with zero name match, is
        // enough to clear the floor for a plausible rename hypothesis).
        let score = similarity_score("foo", "bar", "fn foo(x: i32) -> i32 { x + 1 }", "fn bar(x: i32) -> i32 { x + 1 }");
        assert!(score > 0.4, "expected a score meaningfully above the hypothesis floor from shared signature structure, got {score}");
    }

    #[test]
    fn score_is_always_within_unit_range() {
        let cases = [("", "", "", ""), ("a", "a", "a", "a"), ("x", "y", "completely different text", "nothing in common")];
        for (name_a, name_b, sig_a, sig_b) in cases {
            let score = similarity_score(name_a, name_b, sig_a, sig_b);
            assert!((0.0..=1.0).contains(&score), "score {score} out of range for {name_a:?}/{name_b:?}");
        }
    }

    #[test]
    fn empty_signatures_do_not_panic_or_divide_by_zero() {
        let score = similarity_score("foo", "foo", "", "");
        assert!(score.is_finite());
    }

    #[test]
    fn one_empty_one_nonempty_signature_has_zero_substring_overlap() {
        // Pins the `||` (either side empty), not `&&` (both must be
        // empty), guard in longest_common_substring_len: an empty `a`
        // with a non-empty `b` has no shared substring at all, and the
        // other way around.
        assert_eq!(longest_common_substring_len(b"", b"something"), 0);
        assert_eq!(longest_common_substring_len(b"something", b""), 0);
    }

    #[test]
    fn longest_common_substring_is_contiguous_not_subsequence() {
        // "abc" and "axbxc" share the subsequence a,b,c (len 3) but no
        // contiguous substring longer than 1 — confirming this uses
        // substring, not subsequence, matching.
        assert_eq!(longest_common_substring_len(b"abc", b"axbxc"), 1);
    }

    #[test]
    fn longest_common_substring_finds_a_real_contiguous_match() {
        // Verified independently (not guessed): the longest shared
        // contiguous run between these two strings is "(x: i32)" — 8
        // bytes. "fn " does not extend the match because the very next
        // character ("f" vs "b") breaks contiguity.
        assert_eq!(longest_common_substring_len(b"fn foo(x: i32)", b"fn bar(x: i32)"), 8);
    }
}
