//! B2 Task 3: [`ParseTree`] — the owned result of a successful parse, and
//! [`parse_with`] — the one shared implementation every per-language
//! [`super::LanguageParser`] wrapper in `mod.rs` calls through.
//!
//! All `unsafe` FFI (loading a grammar's generated C parser table,
//! invoking the native parser) is confined inside `tree_sitter::Parser`/
//! `tree_sitter::Language` themselves, called here only through their
//! ordinary safe API. This file introduces no `unsafe` block of its own.

use tree_sitter::Parser;
use tree_sitter_language::LanguageFn;

use super::{Language, ParseError, MAX_SOURCE_BYTES};

/// An owned parse result: the source bytes this tree was parsed from, and
/// the [`tree_sitter::Tree`] itself, which internally borrows byte
/// offsets into that source — kept together so a caller never has a
/// `Tree` whose source could have been dropped or mutated out from under
/// it. `language` is this tree's own record of which exact grammar
/// variant produced it (see [`Language`]'s doc comment on why that is
/// never collapsed to a coarser category).
pub struct ParseTree {
    language: Language,
    source: Vec<u8>,
    tree: tree_sitter::Tree,
}

impl ParseTree {
    /// Which exact grammar produced this tree.
    pub fn language(&self) -> Language {
        self.language
    }

    /// The source bytes this tree was parsed from.
    pub fn source(&self) -> &[u8] {
        &self.source
    }

    /// The underlying `tree_sitter::Tree`, for Task 4's symbol extraction
    /// to walk. Exposed directly rather than wrapped further: Task 3's
    /// job is to isolate the FFI/setup/error-classification concerns
    /// below this point, not to re-abstract tree-sitter's own tree-walking
    /// API, which Task 4 will use as-is (tree-sitter's `Node`/`TreeCursor`
    /// are already a safe, idiomatic Rust API with no FFI details a
    /// caller needs isolating from).
    pub fn tree(&self) -> &tree_sitter::Tree {
        &self.tree
    }

    /// Whether this tree's root node represents or contains any syntax
    /// error (an `ERROR`/`MISSING` node anywhere in the tree). A `true`
    /// result does NOT mean this `ParseTree` is unusable: tree-sitter's
    /// own design goal is error recovery, producing the best tree it can
    /// around the parts it could not understand. Task 4 decides what, if
    /// anything, to extract from a tree with errors; this module makes no
    /// judgment beyond reporting the fact.
    pub fn has_error(&self) -> bool {
        self.tree.root_node().has_error()
    }
}

/// Shared by every per-language wrapper in `mod.rs`'s `define_language_parser!`
/// macro: validates `source`'s size, sets up a fresh `Parser` for
/// `language_fn`, parses, and classifies the result. No panic path: a
/// grammar setup failure or a parse that does not complete both become
/// typed [`ParseError`] variants, never an `unwrap`/`expect` on tree-sitter's
/// own `Option`/`Result` returns.
pub(super) fn parse_with(language: Language, language_fn: LanguageFn, source: &[u8]) -> Result<ParseTree, ParseError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ParseError::SourceTooLarge {
            max: MAX_SOURCE_BYTES,
            actual: source.len(),
        });
    }

    let mut parser = Parser::new();
    parser.set_language(&language_fn.into()).map_err(|e| ParseError::GrammarSetupFailed {
        language,
        reason: e.to_string(),
    })?;

    let tree = parser.parse(source, None).ok_or(ParseError::ParserFailed { language })?;

    Ok(ParseTree {
        language,
        source: source.to_vec(),
        tree,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_accessor_returns_the_exact_bytes_parsed_not_an_empty_default() {
        // Uses NON-empty source specifically: a mutant that replaces
        // `ParseTree::source` with a leaked empty Vec would still pass an
        // assertion against `b""`, so that case alone (covered by the
        // empty-source test below) cannot catch it.
        let source = b"fn main() {}";
        let tree = parse_with(Language::Rust, tree_sitter_rust::LANGUAGE, source).unwrap();
        assert_eq!(tree.source(), source);
    }

    #[test]
    fn empty_source_parses_without_error_for_a_tolerant_grammar() {
        // Rust's grammar accepts an empty file as a valid, content-free
        // parse (no required top-level item) — a good baseline for
        // "empty source is handled, not rejected or panicked on."
        let result = parse_with(Language::Rust, tree_sitter_rust::LANGUAGE, b"");
        let tree = result.unwrap();
        assert!(!tree.has_error());
        assert_eq!(tree.source(), b"");
    }

    #[test]
    fn truncated_malformed_source_produces_a_tree_with_error_nodes_not_a_rejection() {
        // An unterminated function body: tree-sitter's error-recovery
        // design means this still produces a REAL tree (with ERROR/MISSING
        // nodes), not an Err — pinning the task's explicit requirement
        // that a tree containing error nodes is not auto-converted to a
        // total failure.
        let truncated = b"fn main() { let x = ";
        let result = parse_with(Language::Rust, tree_sitter_rust::LANGUAGE, truncated);
        let tree = result.unwrap();
        assert!(tree.has_error());
    }

    #[test]
    fn syntactically_corrupted_source_still_produces_a_tree_not_a_panic() {
        let corrupted = b"fn ))) {{{ ___ ;;; fn fn fn";
        let result = parse_with(Language::Rust, tree_sitter_rust::LANGUAGE, corrupted);
        // Whether this reports an error tree or, in principle, a clean
        // one is not the point of this test (a pathological token soup
        // like this will, in practice, have error nodes) — the point is
        // that it returns a result at all, never panics the process.
        assert!(result.is_ok());
    }

    #[test]
    fn non_utf8_bytes_do_not_panic_the_parser() {
        // Invalid UTF-8 byte sequences (a lone continuation byte, an
        // overlong encoding) embedded in otherwise-plausible source.
        // tree-sitter's native engine is documented as byte-oriented at
        // its core (it tracks raw byte offsets independent of encoding
        // validity, which is exactly why `parse_custom_encoding` exists
        // for genuinely non-UTF-8 sources) — this test is the empirical
        // confirmation this module relies on, run against the real
        // dependency rather than assumed from documentation, which did
        // not explicitly state what `parse` (the UTF-8-text-oriented
        // entry point this module uses) does with invalid bytes.
        let mut source = b"fn main() { let s = \"".to_vec();
        source.push(0xFF); // invalid UTF-8 continuation byte with no lead byte
        source.push(0x80);
        source.extend_from_slice(b"\"; }");

        let result = parse_with(Language::Rust, tree_sitter_rust::LANGUAGE, &source);
        // The requirement under test is solely "did not panic": whatever
        // typed outcome tree-sitter produces for invalid bytes is
        // acceptable (a tree with error nodes around the invalid bytes,
        // or a typed ParseError) as long as the process did not abort.
        match result {
            Ok(tree) => {
                // Accept either outcome; the invalid bytes are very likely
                // to show up as an error node given they sit inside what
                // looks like a string literal, but a tolerant grammar
                // producing a clean tree around them is not itself wrong.
                let _ = tree.has_error();
            }
            Err(_typed_error) => {}
        }
    }

    #[test]
    fn deeply_nested_input_terminates_without_a_sleep_based_timing_test() {
        // Deterministic adversarial corpus: nesting depth is the
        // independent variable, not wall-clock time. 2000 levels of
        // nested parentheses is far beyond anything realistic source
        // would contain, and this test's pass/fail is "did parse_with
        // return," not "did it return within N milliseconds" — no sleep,
        // no timing assertion.
        let depth = 2000;
        let mut source = String::from("fn main() { ");
        source.push_str(&"(".repeat(depth));
        source.push('1');
        source.push_str(&")".repeat(depth));
        source.push_str("; }");

        let result = parse_with(Language::Rust, tree_sitter_rust::LANGUAGE, source.as_bytes());
        assert!(result.is_ok());
    }

    #[test]
    fn highly_repetitive_input_terminates() {
        // A different adversarial shape from deep nesting: many flat,
        // repeated tokens rather than deep structural recursion.
        let repeated = "a + ".repeat(50_000) + "1;";
        let source = format!("fn main() {{ let _ = {repeated} }}");
        let result = parse_with(Language::Rust, tree_sitter_rust::LANGUAGE, source.as_bytes());
        assert!(result.is_ok());
    }
}
