//! B2 Task 4: per-language symbol extraction from a [`crate::parse::ParseTree`].
//!
//! Produces [`RawSymbolCandidate`] values only — no [`codegraph_domain::SymbolSnapshot`],
//! no `SymbolSnapshotId`, no lineage, no typed edges, no persistence, no
//! API. The Raw -> Validated promotion is Task 5's job, not this
//! module's: `kind_hint` here is each language's own raw tree-sitter
//! node-kind vocabulary (e.g. `"function_definition"`, `"class_declaration"`),
//! never mapped against `codegraph_domain::SymbolKind`'s closed enum by
//! this module.
//!
//! # Partial-parse semantics
//!
//! A [`ParseTree`] with `has_error() == true` is still walked for
//! symbols: this module never fails an entire file just because
//! tree-sitter's error recovery produced `ERROR`/`MISSING` nodes
//! somewhere in it (per the plan's explicit instruction). Each
//! per-language extractor is a tree walk that only emits a candidate for
//! node kinds it recognizes as a real construct; an `ERROR` node itself
//! is never recognized as any construct and never contributes a
//! candidate, so malformed input cannot fabricate a symbol out of
//! unparsed text. When a node a given extractor is otherwise interested
//! in (e.g. a `function_definition`) is itself malformed enough that its
//! expected name child is MISSING (not merely syntactically odd, but
//! literally absent — `node.is_missing()` on the child, or the child not
//! present at all), the candidate for that specific node is OMITTED
//! rather than guessed: this module never fabricates a name/signature
//! for an ambiguous or incomplete construct, per the plan's explicit
//! "prefer omit over false certainty" instruction. This is a per-node
//! decision, not a per-file one — a malformed function elsewhere in the
//! same file does not suppress a well-formed class before or after it.
//!
//! # HTML/CSS symbol semantics (explicitly decided, not implied)
//!
//! HTML: only elements with an explicit `id` attribute are candidates.
//! An anonymous element (no `id`, even with a `class`) is never a
//! candidate — a `class` attribute names a potential relationship target
//! for a LATER task's edge resolution, not a DOM-instance identity of its
//! own. CSS: rule sets with a referenceable selector (id/class/tag/
//! custom-element, including composite selectors containing one of
//! those) are candidates, with `signature_text` bounded to the selector
//! text alone (never the rule's full declaration block). Named at-rules
//! with their own identity are candidates too, specifically
//! `@keyframes <name>`. A bare property/declaration inside a rule's body
//! is never, on its own, a candidate. This module does NOT resolve any
//! cross-reference (CSS selector <-> HTML id/class, JS <-> HTML id) —
//! that is explicitly a later edge-resolution task's job.

mod candidate;
mod cpp;
mod csharp;
mod css;
mod html;
mod javascript;
mod python;
mod rust;

pub use candidate::{ByteSpan, CandidateRejection, RawSymbolCandidate, MAX_NAME_BYTES, MAX_SIGNATURE_BYTES};

use crate::parse::{Language, ParseTree};

/// Extracts every recognized candidate from `tree`, dispatching to the
/// extractor matching `tree.language()`. A candidate whose `name` or
/// `signature_text` exceeds the bounded-text limits
/// ([`MAX_NAME_BYTES`]/[`MAX_SIGNATURE_BYTES`]) is DROPPED from the
/// returned list rather than failing extraction for the whole file — one
/// pathologically long construct should not suppress every other, normal
/// candidate in the same file. This chosen semantics (drop-and-continue,
/// not fail-the-file) is this module's one explicit, tested policy
/// decision for the bounded-text rule; [`extract_symbols_reporting_rejections`]
/// is available when a caller wants to know what was dropped and why,
/// rather than silently losing that information.
pub fn extract_symbols(tree: &ParseTree) -> Vec<RawSymbolCandidate> {
    extract_symbols_reporting_rejections(tree).0
}

/// Same extraction as [`extract_symbols`], additionally returning the
/// list of rejections (oversized name/signature) alongside the accepted
/// candidates, so a caller that cares can log/report them instead of
/// losing the information silently.
pub fn extract_symbols_reporting_rejections(tree: &ParseTree) -> (Vec<RawSymbolCandidate>, Vec<CandidateRejection>) {
    let source = tree.source();
    let root = tree.tree().root_node();
    let raw_candidates = match tree.language() {
        Language::Python => python::extract(root, source),
        Language::CSharp => csharp::extract(root, source),
        Language::Cpp => cpp::extract(root, source),
        Language::JavaScript => javascript::extract_javascript(root, source),
        Language::TypeScript => javascript::extract_typescript(root, source),
        Language::Tsx => javascript::extract_typescript(root, source),
        Language::Html => html::extract(root, source),
        Language::Css => css::extract(root, source),
        Language::Rust => rust::extract(root, source),
    };

    let mut candidates = Vec::with_capacity(raw_candidates.len());
    let mut rejections = Vec::new();
    for (name, kind_hint, span, signature_text) in raw_candidates {
        match RawSymbolCandidate::new(name, kind_hint, span, signature_text) {
            Ok(candidate) => candidates.push(candidate),
            Err(rejection) => rejections.push(rejection),
        }
    }
    (candidates, rejections)
}

/// Returns the UTF-8 text of `node` within `source`, or an empty string
/// if the node's byte range is not valid UTF-8 (never panics on an
/// invalid-UTF-8 slice — `Node::utf8_text` already returns a `Result`
/// rather than panicking, and this module treats a decode failure as "no
/// usable text" rather than propagating a new error type for it, since
/// the ONLY thing that can make a tree-sitter-identified node's own byte
/// range invalid UTF-8 is an adversarial multi-byte sequence straddling
/// the node boundary — already covered by this module's "omit rather
/// than guess" partial-parse policy, since a child with unusable text is
/// equivalent to a missing one for naming purposes).
pub(crate) fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{CSharpParser, CppParser, LanguageParser, PythonParser, RustParser};

    fn extract(parser: &dyn LanguageParser, source: &[u8]) -> Vec<RawSymbolCandidate> {
        let tree = parser.parse(source).unwrap();
        extract_symbols(&tree)
    }

    #[test]
    fn extract_symbols_returns_the_real_candidates_for_a_file_with_symbols() {
        // Pins `extract_symbols` itself (the public entry point, not just
        // its `_reporting_rejections` sibling) against a mutant that
        // replaces its body with `vec![]` unconditionally.
        let candidates = extract(&RustParser, b"fn foo() {}\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "foo");
    }

    #[test]
    fn a_file_with_no_symbols_yields_an_empty_list_for_every_language() {
        let cases: Vec<(&dyn LanguageParser, &[u8])> = vec![
            (&PythonParser, b"x = 1\ny = 2\n"),
            (&CSharpParser, b"// just a comment\n"),
            (&CppParser, b"int x = 1;\n"),
            (&RustParser, b"let x = 1;\n"),
        ];
        for (parser, source) in cases {
            let candidates = extract(parser, source);
            assert!(candidates.is_empty(), "expected no symbols for {source:?}, got {candidates:?}");
        }
    }

    #[test]
    fn oversized_name_is_dropped_not_truncated_and_reported_as_a_rejection() {
        let huge_name = "a".repeat(MAX_NAME_BYTES + 1);
        let source = format!("fn {huge_name}() {{}}");
        let tree = RustParser.parse(source.as_bytes()).unwrap();
        let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);

        assert!(candidates.is_empty(), "the oversized-name candidate must be dropped, not truncated into an accepted one");
        assert_eq!(rejections.len(), 1);
        assert!(matches!(rejections[0], CandidateRejection::NameTooLong { .. }));
    }
}
