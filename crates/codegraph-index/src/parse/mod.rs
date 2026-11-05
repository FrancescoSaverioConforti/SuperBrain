//! B2 Task 3: a language-agnostic parse-tree abstraction over tree-sitter
//! grammars. No symbol extraction here — that is Task 4. This module's
//! only job is: given raw bytes and a chosen [`Language`], produce a
//! [`ParseTree`] or a typed [`ParseError`], without panicking, and without
//! `codegraph-domain` or the rest of `codegraph-index` needing to know
//! anything about tree-sitter's FFI details.
//!
//! # Version compatibility (verified against crates.io before writing any
//! grammar-specific code)
//!
//! Every grammar crate this module depends on (`tree-sitter-rust`,
//! `-python`, `-c-sharp`, `-cpp`, `-javascript`, `-typescript`, `-html`,
//! `-css`) has exactly one RUNTIME dependency: `tree-sitter-language
//! ^0.1`. None of them has a runtime dependency on the `tree-sitter` core
//! crate itself — each one's own `tree-sitter` dependency is a
//! dev-dependency, used only for that grammar's own internal test suite,
//! and never appears in the dependency graph `codegraph-index` actually
//! builds against. The core `tree-sitter` crate (this crate's own direct
//! dependency, `0.27`) itself depends on `tree-sitter-language ^0.1.9` —
//! the same shared, stable ABI crate every grammar also targets. There is
//! therefore no version conflict to resolve: `cargo tree -d` run after
//! adding all eight grammar/core dependencies shows no duplicate
//! `tree-sitter` (core) version in the graph, only one `tree-sitter`
//! build-dependency-only duplicate class each grammar pulls in purely for
//! its own dev-dependency closure (never linked into this crate's own
//! build). See the plan's checkpoint notes for the exact `cargo tree -d`
//! output recorded at this task's closure.
//!
//! # FFI isolation
//!
//! Every grammar crate's public surface is a `tree_sitter::LanguageFn`
//! constant (`LANGUAGE`, or `LANGUAGE_TYPESCRIPT`/`LANGUAGE_TSX` for the
//! two-grammar `tree-sitter-typescript` crate) — a thin, `unsafe`-free
//! Rust wrapper over the grammar's generated C parser table. The actual
//! `unsafe` FFI (loading the C symbol table, invoking the native parser)
//! lives entirely inside `tree-sitter`'s own `Parser`/`Language` types,
//! which this module calls through their ordinary safe Rust API
//! (`Parser::set_language`, `Parser::parse`) — this module introduces no
//! `unsafe` block of its own. [`language_fn`] is the one place that
//! matches on [`Language`] to select which grammar's `LanguageFn` to use;
//! everything past that point is identical, safe code for every language.

use thiserror::Error;

mod tree;

pub use tree::ParseTree;

/// Maximum accepted source size, in bytes, for a single [`LanguageParser::parse`]
/// call. This is NOT Gate S3's real resource budget (that is Task 13's
/// job, and will likely be policy-configurable rather than a crate
/// constant) — it exists only so this module's own API is not implicitly
/// "parse anything, no matter how large," per the task's instruction.
/// Exceeding it is a typed rejection, never a silent truncation: the
/// caller gets the exact byte count back and the source is never cut
/// short before tree-sitter ever sees it.
pub const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;

/// Every language this module can parse. Deliberately one variant per
/// distinct GRAMMAR, not one variant per file extension or per "family":
/// `JavaScript`, `TypeScript` and `Tsx` are three separate variants
/// because `tree-sitter-typescript` itself exposes them as two distinct
/// `LanguageFn` constants (TypeScript and Tsx have different grammars,
/// not just different file extensions of the same one), and `Html`/`Css`
/// are separate from both and from each other. A [`ParseTree`] always
/// reports which exact variant parsed it — never a coarser category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Python,
    CSharp,
    Cpp,
    JavaScript,
    TypeScript,
    Tsx,
    Html,
    Css,
    Rust,
}

/// No `UnsupportedLanguage` runtime variant exists: [`Language`] is a
/// closed enum whose every constructible value names one of the nine
/// grammars this module actually wires up in `mod.rs`'s
/// `define_language_parser!` calls. A caller cannot construct a
/// `Language` value this module does not support — there is no "unknown
/// language" string/code a caller passes in that this type would need to
/// validate at runtime, unlike e.g. `codegraph_domain::SymbolKind::parse`,
/// which DOES validate an open-ended raw string against a closed set
/// (Task 4's job, operating on language-internal node-kind strings, not
/// this module's job). "Unsupported language" is therefore a compile-time
/// property of `Language` itself here, not a [`ParseError`] variant.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    /// `source` exceeded [`MAX_SOURCE_BYTES`]. Never truncated; the whole
    /// call is rejected.
    #[error("source is {actual} bytes, maximum accepted is {max} (Task 13 will define the real resource budget; this is a minimal, explicit placeholder so this API is not implicitly unbounded)")]
    SourceTooLarge { max: usize, actual: usize },
    /// Loading or assigning the grammar itself failed — a problem with
    /// the parser's setup, not with the input bytes. Distinct from
    /// [`ParseError::ParserFailed`] so a caller can tell "this language is
    /// broken for everyone" apart from "this one input defeated the
    /// parser." Not reachable through this module's own, correctly
    /// version-matched dependency set (see this module's doc comment on
    /// version compatibility) — kept as a distinct variant because
    /// `Parser::set_language` genuinely returns a `Result` this module
    /// must handle, not because this module has observed it fail.
    #[error("failed to set up the {language:?} grammar: {reason}")]
    GrammarSetupFailed { language: Language, reason: String },
    /// `tree_sitter::Parser::parse` returned `None` for a reason other
    /// than a missing language assignment (which [`ParseError::GrammarSetupFailed`]
    /// already rules out by the time this module calls `parse`). This
    /// module does not configure a timeout or cancellation callback, so
    /// nothing in its own current code path can actually produce this
    /// variant today — it exists because `parse` returns `Option<Tree>`,
    /// not `Tree`, and a future caller that does add
    /// `Parser::set_timeout_micros`/a progress callback (still inside
    /// this module, not propagated as new API surface, if that ever
    /// happens) would need this variant already present rather than
    /// introduced as a breaking change later. Never an input-content
    /// failure: tree-sitter's whole design goal is to always produce SOME
    /// tree (with `ERROR` nodes) rather than fail on malformed syntax.
    #[error("parsing {language:?} source did not complete")]
    ParserFailed { language: Language },
}

/// Parses `source` as `language`. No implementation may treat a syntax
/// tree containing `ERROR` nodes as a failure: tree-sitter's own design
/// goal is error recovery — it always returns a complete tree for
/// syntactically invalid input, marking the parts it could not understand
/// with `ERROR`/`MISSING` nodes rather than failing the whole parse. This
/// trait's job stops at "did tree-sitter produce a tree at all"; deciding
/// what to extract from a tree that contains error nodes is Task 4's job,
/// not this trait's.
pub trait LanguageParser: Send + Sync {
    fn parse(&self, source: &[u8]) -> Result<ParseTree, ParseError>;
}

/// One [`LanguageParser`] per grammar, each a thin wrapper around the
/// shared [`tree::parse_with`] helper — avoiding six copies of the same
/// size-check/parser-setup/error-mapping logic while keeping one
/// concrete, independently nameable type per language, as the plan
/// specifies.
macro_rules! define_language_parser {
    ($name:ident, $language:expr, $language_fn:expr) => {
        #[derive(Debug, Default)]
        pub struct $name;

        impl LanguageParser for $name {
            fn parse(&self, source: &[u8]) -> Result<ParseTree, ParseError> {
                tree::parse_with($language, $language_fn, source)
            }
        }
    };
}

define_language_parser!(PythonParser, Language::Python, tree_sitter_python::LANGUAGE);
define_language_parser!(CSharpParser, Language::CSharp, tree_sitter_c_sharp::LANGUAGE);
define_language_parser!(CppParser, Language::Cpp, tree_sitter_cpp::LANGUAGE);
define_language_parser!(JavaScriptParser, Language::JavaScript, tree_sitter_javascript::LANGUAGE);
define_language_parser!(TypeScriptParser, Language::TypeScript, tree_sitter_typescript::LANGUAGE_TYPESCRIPT);
define_language_parser!(TsxParser, Language::Tsx, tree_sitter_typescript::LANGUAGE_TSX);
define_language_parser!(HtmlParser, Language::Html, tree_sitter_html::LANGUAGE);
define_language_parser!(CssParser, Language::Css, tree_sitter_css::LANGUAGE);
define_language_parser!(RustParser, Language::Rust, tree_sitter_rust::LANGUAGE);

#[cfg(test)]
mod tests {
    use super::*;

    /// One fixture set per language: minimal valid source, a non-trivial
    /// source with nested constructs, and a truncated/malformed source.
    /// Table-driven rather than one function per language per case, to
    /// avoid 9 languages x 3+ cases as near-duplicate functions while
    /// still giving each language its own real coverage (not one shared
    /// Rust-only fixture standing in for the rest).
    struct LanguageFixture {
        name: &'static str,
        parser: &'static dyn LanguageParser,
        minimal_valid: &'static [u8],
        nested_valid: &'static [u8],
        truncated_malformed: &'static [u8],
    }

    fn fixtures() -> Vec<LanguageFixture> {
        vec![
            LanguageFixture {
                name: "python",
                parser: &PythonParser,
                minimal_valid: b"def f():\n    pass\n",
                nested_valid: b"class Outer:\n    def method(self):\n        if True:\n            for x in range(3):\n                return x\n",
                truncated_malformed: b"def f(\n    if True:\n        return",
            },
            LanguageFixture {
                name: "csharp",
                parser: &CSharpParser,
                minimal_valid: b"class C {}\n",
                nested_valid: b"namespace N { class Outer { class Inner { int M() { if (true) { return 1; } return 0; } } } }\n",
                truncated_malformed: b"class C { void M( {{{ if (",
            },
            LanguageFixture {
                name: "cpp",
                parser: &CppParser,
                minimal_valid: b"int main() { return 0; }\n",
                nested_valid: b"namespace n { class Outer { public: class Inner { public: int m() { if (true) { for (int i = 0; i < 3; i++) { return i; } } return 0; } }; }; }\n",
                truncated_malformed: b"int main() { if ( for (;;",
            },
            LanguageFixture {
                name: "javascript",
                parser: &JavaScriptParser,
                minimal_valid: b"function f() {}\n",
                nested_valid: b"function outer() { return function inner() { if (true) { for (let i = 0; i < 3; i++) { return i; } } }; }\n",
                truncated_malformed: b"function f( { if (true",
            },
            LanguageFixture {
                name: "typescript",
                parser: &TypeScriptParser,
                minimal_valid: b"function f(): void {}\n",
                nested_valid: b"interface Outer { inner: { value: number }; } function f(x: Outer): number { if (x.inner.value > 0) { return x.inner.value; } return 0; }\n",
                truncated_malformed: b"function f(x: {): numb",
            },
            LanguageFixture {
                name: "tsx",
                parser: &TsxParser,
                minimal_valid: b"const x = <div />;\n",
                nested_valid: b"const x = <div><span>{items.map(i => <p key={i}>{i}</p>)}</span></div>;\n",
                truncated_malformed: b"const x = <div><span>{",
            },
            LanguageFixture {
                name: "html",
                parser: &HtmlParser,
                minimal_valid: b"<html></html>\n",
                nested_valid: b"<html><body><div><ul><li>one</li><li>two</li></ul></div></body></html>\n",
                truncated_malformed: b"<html><body><div><ul><li>",
            },
            LanguageFixture {
                name: "css",
                parser: &CssParser,
                minimal_valid: b".a { color: red; }\n",
                nested_valid: b"@media screen { .a .b > .c:hover { color: red; background: url(x.png); } }\n",
                truncated_malformed: b".a { color: red; .b {",
            },
            LanguageFixture {
                name: "rust",
                parser: &RustParser,
                minimal_valid: b"fn main() {}\n",
                nested_valid: b"mod outer { pub struct S { pub v: i32 } impl S { pub fn m(&self) -> i32 { if self.v > 0 { for i in 0..3 { return i; } } 0 } } }\n",
                truncated_malformed: b"fn main() { let x = if true {",
            },
        ]
    }

    #[test]
    fn every_language_parses_its_minimal_valid_fixture_with_no_error_nodes() {
        for fixture in fixtures() {
            let tree = fixture.parser.parse(fixture.minimal_valid).unwrap();
            assert!(
                !tree.has_error(),
                "{}: expected no ERROR nodes for minimal valid source",
                fixture.name
            );
        }
    }

    #[test]
    fn every_language_parses_its_nested_construct_fixture_with_no_error_nodes() {
        for fixture in fixtures() {
            let tree = fixture.parser.parse(fixture.nested_valid).unwrap();
            assert!(
                !tree.has_error(),
                "{}: expected no ERROR nodes for a valid, non-trivially-nested source",
                fixture.name
            );
        }
    }

    #[test]
    fn every_language_produces_a_tree_not_a_rejection_for_truncated_malformed_source() {
        for fixture in fixtures() {
            let result = fixture.parser.parse(fixture.truncated_malformed);
            // The requirement under test: truncated/malformed input is
            // not auto-converted to a parse failure. tree-sitter's error
            // recovery should produce a real tree (almost certainly with
            // error nodes, though that specific fact isn't the point
            // here) rather than this module returning an Err for a
            // syntax problem — Err is reserved for size/setup/engine
            // failures (ParseError's own variants), never "the input
            // had bad syntax."
            assert!(result.is_ok(), "{}: truncated/malformed source must still produce a tree, not an Err", fixture.name);
        }
    }

    #[test]
    fn every_language_handles_empty_source_without_panicking() {
        // Some grammars may report empty source as containing an error
        // node (a required top-level construct missing) while others
        // (like Rust) accept it cleanly — both are legitimate. The
        // requirement under test is solely that `parse` returns at all
        // (does not panic); reaching the loop's end after calling it for
        // every fixture is the assertion.
        for fixture in fixtures() {
            let _ = fixture.parser.parse(b"");
        }
    }

    #[test]
    fn max_source_bytes_is_exactly_eight_mebibytes() {
        // Pins the constant's actual VALUE against a hardcoded
        // expectation independent of the constant itself — every other
        // test in this file uses `MAX_SOURCE_BYTES` symbolically, which
        // is blind to a mutant that changes the constant's own
        // definition (e.g. `8 * 1024 * 1024` mutated to `8 + 1024 +
        // 1024`, a much smaller number) as long as the surrounding
        // boundary logic still uses whatever the mutated value is.
        assert_eq!(MAX_SOURCE_BYTES, 8_388_608);
    }

    #[test]
    fn source_exactly_at_the_size_limit_is_accepted_not_rejected() {
        // Pins the `>` (strictly greater than), not `>=`, boundary — the
        // same discipline Task 1's FileIdentity length check used.
        // Whitespace-only input is syntactically valid Rust (an empty,
        // content-free parse), so this exercises the SIZE check alone,
        // not an unrelated syntax failure.
        let exactly_at_limit = vec![b' '; MAX_SOURCE_BYTES];
        let result = RustParser.parse(&exactly_at_limit);
        assert!(result.is_ok());
    }

    #[test]
    fn source_one_byte_over_the_size_limit_is_rejected() {
        let one_over = vec![b' '; MAX_SOURCE_BYTES + 1];
        let result = RustParser.parse(&one_over);
        match result {
            Err(err) => assert_eq!(
                err,
                ParseError::SourceTooLarge {
                    max: MAX_SOURCE_BYTES,
                    actual: MAX_SOURCE_BYTES + 1,
                }
            ),
            Ok(_) => panic!("expected SourceTooLarge, got Ok"),
        }
    }

    #[test]
    fn invalid_utf8_bytes_inside_a_string_literal_do_not_panic_any_language() {
        // Cross-language counterpart to `tree::tests::non_utf8_bytes_do_not_panic_the_parser`
        // (which exercises this in depth for Rust alone): every language
        // here gets an invalid UTF-8 byte sequence embedded inside
        // whatever that language's own string/text-literal syntax is, to
        // confirm the "no panic on non-UTF-8 bytes" property holds
        // across languages, not just the one already covered in depth.
        // HTML/CSS have no string-literal-with-quotes-and-FFI-adjacent
        // concern distinct from "arbitrary bytes somewhere in the
        // document/declaration," so they get the invalid bytes spliced
        // into their general text content instead.
        let invalid_utf8: &[u8] = &[0xFF, 0x80];

        let mut python = b"s = \"".to_vec();
        python.extend_from_slice(invalid_utf8);
        python.extend_from_slice(b"\"\n");

        let mut csharp = b"class C { string s = \"".to_vec();
        csharp.extend_from_slice(invalid_utf8);
        csharp.extend_from_slice(b"\"; }");

        let mut cpp = b"const char* s = \"".to_vec();
        cpp.extend_from_slice(invalid_utf8);
        cpp.extend_from_slice(b"\";");

        let mut javascript = b"const s = \"".to_vec();
        javascript.extend_from_slice(invalid_utf8);
        javascript.extend_from_slice(b"\";");

        let mut typescript = b"const s: string = \"".to_vec();
        typescript.extend_from_slice(invalid_utf8);
        typescript.extend_from_slice(b"\";");

        let mut tsx = b"const x = <div>".to_vec();
        tsx.extend_from_slice(invalid_utf8);
        tsx.extend_from_slice(b"</div>;");

        let mut html = b"<p>".to_vec();
        html.extend_from_slice(invalid_utf8);
        html.extend_from_slice(b"</p>");

        let mut css = b"/* ".to_vec();
        css.extend_from_slice(invalid_utf8);
        css.extend_from_slice(b" */ .a { color: red; }");

        let cases: Vec<(&dyn LanguageParser, Vec<u8>)> = vec![
            (&PythonParser, python),
            (&CSharpParser, csharp),
            (&CppParser, cpp),
            (&JavaScriptParser, javascript),
            (&TypeScriptParser, typescript),
            (&TsxParser, tsx),
            (&HtmlParser, html),
            (&CssParser, css),
        ];
        for (parser, source) in cases {
            // No panic is the requirement; either typed outcome is
            // acceptable.
            let _ = parser.parse(&source);
        }
    }
}
