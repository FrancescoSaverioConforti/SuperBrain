//! B3-TUNE-v2 Priority 2 Lever B: a conservative, code-aware punctuation
//! normalization rule, kept entirely separate from `bm25::tokenize`.
//! Nothing in this module is wired into the frozen production tokenizer
//! — it exists only for Lever B's own diagnostic and ablation binaries
//! (`crates/api/examples/b3_tune_v2_lever_b_normalization.rs`) to call
//! explicitly. `bm25::tokenize`'s own behavior, and every existing call
//! site that depends on it, is completely unaffected by this module.

/// Trailing sentence-boundary punctuation this rule strips. Deliberately
/// narrow: no bracket/quote-opening characters, no symbols that appear
/// inside real technical tokens this corpus contains (`+`, `#`, `-`,
/// `_`, `/`, `:`, `.` are only ever stripped from a token's own
/// trailing end, never from its interior, so `C++`/`C#`/`foo::bar`/
/// `foo.bar`/`snake_case`/paths/CLI flags are all unaffected — none of
/// those tokens' own trailing characters are in this set in the first
/// place, except `.` which `foo.bar` only has in the interior).
///
/// **`)`, `]`, `}` are deliberately excluded** (narrowed after the
/// corpus+dev diagnostic — Task 4 Step 6): this corpus is code-heavy
/// and trailing close-brackets very frequently close a real function
/// call, type annotation, or complexity notation (`_check_programs()`,
/// `list[str]`, `O(n^2)`, `Optional[dict]`) rather than ending a
/// sentence. Stripping them left dangling, unbalanced brackets in 50+
/// real corpus/dev tokens during the diagnostic pass, so they were
/// dropped from this set entirely rather than accepted as a known
/// collision, per the plan's "prefer a stricter transformation over a
/// more aggressive one" instruction.
const TRAILING_SENTENCE_PUNCTUATION: &[char] = &['.', ',', ';', ':', '!', '?', '"', '\''];

/// Strips characters in `TRAILING_SENTENCE_PUNCTUATION` from `token`'s
/// end, repeatedly, only while the remainder stays nonempty — a token
/// that is entirely punctuation (e.g. `"..."`, `")"`) passes through
/// unchanged, since stripping it all would leave nothing. Leading and
/// interior characters are never inspected or modified. Pure function;
/// never wired into `bm25::tokenize` (see module doc comment).
pub fn strip_trailing_sentence_punctuation(token: &str) -> &str {
    if token.chars().all(|ch| TRAILING_SENTENCE_PUNCTUATION.contains(&ch)) {
        // The whole token is punctuation (e.g. "...", ")"); stripping
        // any of it would still leave only punctuation or nothing, so
        // the token passes through completely unchanged.
        return token;
    }
    let mut end = token.len();
    for (idx, ch) in token.char_indices().rev() {
        if !TRAILING_SENTENCE_PUNCTUATION.contains(&ch) {
            break;
        }
        end = idx;
    }
    &token[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_sentence_punctuation() {
        assert_eq!(strip_trailing_sentence_punctuation("bm25,"), "bm25");
        assert_eq!(strip_trailing_sentence_punctuation("index."), "index");
        assert_eq!(strip_trailing_sentence_punctuation("really?"), "really");
    }

    #[test]
    fn preserves_technical_identifiers_unchanged() {
        assert_eq!(strip_trailing_sentence_punctuation("c++"), "c++");
        assert_eq!(strip_trailing_sentence_punctuation("c#"), "c#");
        assert_eq!(strip_trailing_sentence_punctuation("sec-001"), "sec-001");
        assert_eq!(strip_trailing_sentence_punctuation("foo::bar"), "foo::bar");
        assert_eq!(strip_trailing_sentence_punctuation("foo.bar"), "foo.bar");
        assert_eq!(strip_trailing_sentence_punctuation("snake_case"), "snake_case");
        assert_eq!(strip_trailing_sentence_punctuation("--split"), "--split");
        assert_eq!(
            strip_trailing_sentence_punctuation("crates/api/src/metrics.rs"),
            "crates/api/src/metrics.rs"
        );
    }

    #[test]
    fn never_strips_leading_or_interior_punctuation() {
        assert_eq!(strip_trailing_sentence_punctuation("(parenthetical"), "(parenthetical");
        assert_eq!(strip_trailing_sentence_punctuation("mid.dle"), "mid.dle");
    }

    #[test]
    fn does_not_strip_a_token_that_is_entirely_punctuation() {
        assert_eq!(strip_trailing_sentence_punctuation("..."), "...");
        assert_eq!(strip_trailing_sentence_punctuation(")"), ")");
    }
}
