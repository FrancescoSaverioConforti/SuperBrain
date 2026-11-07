//! B2 Task 1/5: closed symbol/edge kind enums. No "other"/catch-all variant
//! on either enum: an unrecognized raw kind string must be a typed
//! rejection at the promotion boundary (Task 5), never silently accepted
//! into an ambiguous bucket — the same discipline `domain::Classification`
//! uses by deliberately having no `Secret` variant (`schema.rs`'s comment:
//! "classification: secret must never become a valid in-memory value").
//!
//! `SymbolKind` is an explicit MINIMUM BASELINE, not a final taxonomy —
//! see `docs/superpowers/plans/2026-11-05-b2-code-graph.md` section 8b for
//! the full rationale, the coverage-matrix process for proposing new
//! variants, and the required real-repository validation pass before B2's
//! final closure. `Module`/`Const`/`Macro` are kept even though Task 4's
//! current nine extractors never produce a `RawSymbolCandidate` for any of
//! them yet — removing a variant because nothing currently reaches it
//! would be the same mistake as adding one on intuition, just in reverse.
//! `MethodSignature`/`HtmlIdElement`/`CssRule`/`CssKeyframes` were added at
//! Task 5 because Task 4's extractors DO produce candidates with those
//! `kind_hint`s and no existing variant fit them.

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    /// A method declared without a body: a trait/interface method
    /// signature (Rust's `function_signature_item`, TypeScript's
    /// `method_signature`). Distinct from `Method` because a signature
    /// has no implementation to analyze and a future caller (e.g. an
    /// edge-resolution task) needs to tell the two apart rather than
    /// treating a declaration as if it were a definition.
    MethodSignature,
    Class,
    Struct,
    Enum,
    Interface,
    Module,
    Const,
    Macro,
    /// An HTML element with an explicit, non-empty `id` attribute — the
    /// only HTML construct Task 4 treats as a symbol (see
    /// `codegraph-index::extract::html`'s module doc comment for the
    /// full rationale: an anonymous or class-only element is never a
    /// candidate).
    HtmlIdElement,
    /// A CSS rule set with a referenceable selector (id/class/tag,
    /// including inside a composite selector) — see
    /// `codegraph-index::extract::css`'s module doc comment.
    CssRule,
    /// A named `@keyframes <name>` CSS at-rule.
    CssKeyframes,
    /// Task 9.5, first expansion-set variant (see the plan's section 8b
    /// roadmap): an explicit instance attribute/field assignment —
    /// Python's `self.X = <expr>`, confirmed by a real-repository
    /// validation pass (`docs/research/evaluations/B2-EVAL-001.md`) to
    /// be a frequent, significant, stably-identified construct real
    /// deep-learning code assigns submodules/components to (and plain
    /// scalar/config state too — `Field` extraction does not require
    /// the right-hand side to be a constructor call; see
    /// `codegraph-index::extract::python`'s module doc comment for the
    /// exact, conservative AST rule and what is deliberately excluded).
    /// `self` here is Python-specific; other languages' own receiver/
    /// `this`-equivalent semantics are a separate, future, evidence-
    /// driven decision — `Field` is not assumed universal across
    /// languages by this variant's existence alone.
    Field,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unrecognized symbol kind: {raw:?}")]
pub struct UnrecognizedSymbolKind {
    pub raw: String,
}

impl SymbolKind {
    /// B2-ID-006: whether a snapshot of this `SymbolKind` can itself be
    /// the SOURCE of a `Calls` edge — i.e. whether its own captured
    /// span represents a single, real, directly executable unit of
    /// code, as opposed to a structural container whose span merely
    /// CONTAINS other symbols' own executable bodies (and whose
    /// `signature_text`, per every current extractor's design, is the
    /// full span's text either way — see `codegraph-index::extract`'s
    /// per-language modules).
    ///
    /// This is the caller-eligibility half of B2-ID-006's two-part fix
    /// (see `codegraph-index::edges::calls`'s own module doc comment
    /// for the other half, the nested-scope span boundary — NEITHER
    /// part alone is sufficient; see that module's doc for why).
    /// Centralized here, rather than scattered as an inline
    /// `if kind != Class && ...` check inside `resolve_calls`, so the
    /// classification is a single, named, independently testable
    /// capability a reviewer can audit against this type's own real
    /// variants, not a blacklist that silently misses a future variant.
    ///
    /// Classification (confirmed against every current extractor's
    /// real behavior — `codegraph-index::extract`'s per-language
    /// modules — during B2-ID-006's own investigation, not assumed
    /// from variant names):
    ///
    /// - `Function`, `Method` → **true**. Every extractor's own
    ///   `signature_text` for these IS the directly executable body
    ///   (a Python/C++ `function_definition`, a Rust `function_item`, a
    ///   C# `method_declaration`, a JS/TS `method_definition`/
    ///   `arrow_function`/`function_declaration`). A nested function
    ///   INSIDE another function's body is still correctly eligible by
    ///   this predicate alone — distinguishing "outer" from "inner"
    ///   when both are legitimately eligible is the OTHER half of the
    ///   fix (the nested-scope boundary), not this predicate's job.
    /// - `MethodSignature` → **false**. By construction, a trait/
    ///   interface method SIGNATURE (Rust's `function_signature_item`,
    ///   TypeScript's `method_signature`) has no body at all (confirmed
    ///   by `extract/rust.rs`'s own doc comment: "a trait method
    ///   declaration with no body, e.g. `fn req(&self);`") — it cannot
    ///   syntactically contain a call expression, so this is a
    ///   defensive, always-correct exclusion, not a behavior change for
    ///   any real input.
    /// - `Class`, `Struct`, `Interface` → **false**. Confirmed, for
    ///   every language that produces these (`class_definition`/
    ///   `class_declaration`/`class_specifier`; `struct_item`/
    ///   `struct_specifier`; `trait_item`/`interface_declaration`),
    ///   that `signature_text` is the FULL container body, including
    ///   every nested method's own body — exactly the B2-ID-006 defect
    ///   this predicate exists to close. A Rust trait's default method
    ///   body belongs to the METHOD snapshot nested inside it (already
    ///   `Function`/`Method`, already eligible on its own), never to
    ///   the trait/`Interface` snapshot itself.
    /// - `Enum` → **false**. Only Rust's `enum_item` produces this
    ///   variant (confirmed: no other extractor maps anything to
    ///   `Enum`); a Rust enum's own body is its variant list (optional
    ///   associated data types, never executable statements) — there
    ///   is no real construct this would ever need to treat as
    ///   "caller-eligible," and excluding it defensively costs nothing.
    /// - `Field` → **false**. Confirmed by direct synthetic
    ///   reproduction during B2-ID-006's investigation: a `Field`
    ///   (`self.x = <rhs>`, Python's `self_attribute_assignment`) can
    ///   only ever occur inside a method body (`self` is only bound as
    ///   a parameter inside one), so a `Field` snapshot's own span is
    ///   ALWAYS nested inside its enclosing `Function`/`Method`
    ///   snapshot's own span. Treating `Field` as caller-eligible would
    ///   reproduce the exact B2-ID-006 defect one level deeper (a call
    ///   in `self.x = f(g())` would be attributed to the `Field` AND
    ///   to its enclosing method, both of which would still remain
    ///   eligible) — the nested-scope boundary's own general nesting
    ///   check does not need `Field` to be eligible for that call to be
    ///   attributed correctly to the enclosing method, since the
    ///   method is already `Function`/`Method`-eligible by the first
    ///   two variants above.
    /// - `Module`, `Const`, `Macro` → **false**. Confirmed (per this
    ///   type's own module doc comment and `codegraph-index::snapshot::map_kind_hint`'s
    ///   own exhaustive mapping table) that NO current extractor ever
    ///   produces a candidate of any of these three kinds — there is no
    ///   real `SymbolSnapshot` of these kinds for this predicate to be
    ///   exercised against today. Decided conservatively (`false`) for
    ///   exhaustiveness and forward-compatibility rather than left to
    ///   guess later if a future extractor starts producing one: a
    ///   `Module`/`File` would be the ultimate "structural container,
    ///   never itself executable" case if ever implemented (consistent
    ///   with `codegraph-index::edges::contains`'s own deferred
    ///   Module/File design note); `Const`/`Macro` are deliberately NOT
    ///   claimed as semantically decided by this method (a `const fn`
    ///   body, if ever extracted as its own `Const`-kind candidate
    ///   rather than folded into `Function`, or a Rust `macro_rules!`
    ///   body, if ever made call-visible past the known `token_tree`
    ///   opacity gap `edges::calls`'s own module doc already names,
    ///   could reasonably need their own dedicated re-evaluation before
    ///   either is ever flipped to `true` — not decided here, since no
    ///   real data exists yet to decide it against).
    /// - `HtmlIdElement`, `CssRule`, `CssKeyframes` → **false**. Already
    ///   structurally non-executable for an unrelated reason (HTML/CSS
    ///   have no call-expression concept at all), and already excluded
    ///   from `resolve_calls`'s own call-site walk by language
    ///   (`walk_for_calls`'s `Language::Html | Language::Css => false`
    ///   branch) independent of this predicate — included here only
    ///   for this method's own exhaustiveness, not because either
    ///   exclusion depends on the other.
    pub fn can_originate_calls(self) -> bool {
        match self {
            SymbolKind::Function | SymbolKind::Method => true,
            SymbolKind::MethodSignature
            | SymbolKind::Class
            | SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Interface
            | SymbolKind::Module
            | SymbolKind::Const
            | SymbolKind::Macro
            | SymbolKind::HtmlIdElement
            | SymbolKind::CssRule
            | SymbolKind::CssKeyframes
            | SymbolKind::Field => false,
        }
    }

    /// Parses `SymbolKind`'s own canonical lowercase-kebab-case string
    /// form (`"function"`, `"method-signature"`, `"html-id-element"`, ...)
    /// — NOT Task 4's raw tree-sitter `kind_hint` vocabulary
    /// (`"function_item"`, `"class_declaration"`, `"element_with_id"`,
    /// ...), which uses different per-language node-kind names that do
    /// not match this canonical form at all. Task 1's original doc
    /// comment on this method incorrectly assumed the two vocabularies
    /// would coincide; Task 4's real extractors proved otherwise. The
    /// actual `kind_hint -> SymbolKind` mapping Task 5 needs is
    /// `codegraph_index::promote::map_kind_hint`, a separate,
    /// language-aware, many-to-one mapping (several different raw node
    /// kinds across languages collapse to the same `SymbolKind`) — this
    /// method remains available for a canonical string this type's own
    /// `Display`/round-trip might use, but is not that mapping.
    pub fn parse(raw: &str) -> Result<Self, UnrecognizedSymbolKind> {
        match raw {
            "function" => Ok(Self::Function),
            "method" => Ok(Self::Method),
            "method-signature" => Ok(Self::MethodSignature),
            "class" => Ok(Self::Class),
            "struct" => Ok(Self::Struct),
            "enum" => Ok(Self::Enum),
            "interface" => Ok(Self::Interface),
            "module" => Ok(Self::Module),
            "const" => Ok(Self::Const),
            "macro" => Ok(Self::Macro),
            "html-id-element" => Ok(Self::HtmlIdElement),
            "css-rule" => Ok(Self::CssRule),
            "css-keyframes" => Ok(Self::CssKeyframes),
            "field" => Ok(Self::Field),
            _ => Err(UnrecognizedSymbolKind { raw: raw.to_string() }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Calls,
    /// `Imports` was REMOVED (Task 9.5, Priority 6 — see
    /// B2-ID-005): the original symbol-to-symbol design mismatched
    /// real import semantics (a file/module-level fact, never a single
    /// symbol's own) and was empirically confirmed to capture 0 of 589
    /// real import statements in a real-repository evaluation. Its
    /// replacement, `codegraph_index::imports::ImportFact`, is a
    /// separate, file-level fact — deliberately NOT an `EdgeKind`
    /// variant, since there is no symbol this fact can honestly be
    /// "from" without reintroducing the same granularity mismatch. This
    /// comment is kept (rather than silently renumbering the enum) so a
    /// reader of old data/docs referencing "Imports" finds the reason
    /// it is gone, not just its absence.
    Implements,
    Extends,
    References,
    TestsFor,
    RenamedTo,
    /// Task 9.5: a structural containment relation — a class containing
    /// a method or field, or a module/file containing a top-level
    /// symbol. Always `StaticAnalysis`: containment is read directly
    /// off the AST's own nesting, never a heuristic guess. Deliberately
    /// NOT a catch-all — only class-contains-method, class-contains-
    /// field, and module/file-contains-top-level-symbol are produced;
    /// no other containment shape is invented to fit this one variant.
    Contains,
    /// Task 9.5: a field/attribute's right-hand side is a resolvable
    /// constructor/call expression naming another known symbol —
    /// `self.encoder = TransformerEncoder(...)` once `TransformerEncoder`
    /// resolves to a snapshot in the given pool. Stricter than `Field`
    /// extraction itself: not every `Field` has a corresponding
    /// `Instantiates` edge (a scalar/config assignment like
    /// `self.ninp = ninp` never does), and no edge is ever produced for
    /// an external/unresolvable constructor (e.g. `nn.Linear`, a
    /// third-party class never indexed in the given pool) — "no edge
    /// beats a false edge" applies here exactly as it does for every
    /// other edge kind in this project.
    Instantiates,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unrecognized edge kind: {raw:?}")]
pub struct UnrecognizedEdgeKind {
    pub raw: String,
}

impl EdgeKind {
    pub fn parse(raw: &str) -> Result<Self, UnrecognizedEdgeKind> {
        match raw {
            "calls" => Ok(Self::Calls),
            "implements" => Ok(Self::Implements),
            "extends" => Ok(Self::Extends),
            "references" => Ok(Self::References),
            "tests_for" => Ok(Self::TestsFor),
            "renamed_to" => Ok(Self::RenamedTo),
            "contains" => Ok(Self::Contains),
            "instantiates" => Ok(Self::Instantiates),
            _ => Err(UnrecognizedEdgeKind { raw: raw.to_string() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_function_and_method_can_originate_calls() {
        // B2-ID-006: exhaustive over every CURRENT variant, written as
        // an explicit allow-list rather than negated from a blacklist,
        // so a future new SymbolKind variant must be added to ONE of
        // these two lists explicitly (the match in
        // `can_originate_calls` itself is non-exhaustive-proof at
        // compile time; this test is the explicit, readable
        // human-facing statement of the same fact).
        let eligible = [SymbolKind::Function, SymbolKind::Method];
        let ineligible = [
            SymbolKind::MethodSignature,
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Interface,
            SymbolKind::Module,
            SymbolKind::Const,
            SymbolKind::Macro,
            SymbolKind::HtmlIdElement,
            SymbolKind::CssRule,
            SymbolKind::CssKeyframes,
            SymbolKind::Field,
        ];
        for kind in eligible {
            assert!(kind.can_originate_calls(), "{kind:?} must be caller-eligible");
        }
        for kind in ineligible {
            assert!(!kind.can_originate_calls(), "{kind:?} must NOT be caller-eligible");
        }
    }

    #[test]
    fn every_symbol_kind_variant_round_trips_through_its_canonical_string() {
        let cases = [
            ("function", SymbolKind::Function),
            ("method", SymbolKind::Method),
            ("method-signature", SymbolKind::MethodSignature),
            ("class", SymbolKind::Class),
            ("struct", SymbolKind::Struct),
            ("enum", SymbolKind::Enum),
            ("interface", SymbolKind::Interface),
            ("module", SymbolKind::Module),
            ("const", SymbolKind::Const),
            ("macro", SymbolKind::Macro),
            ("html-id-element", SymbolKind::HtmlIdElement),
            ("css-rule", SymbolKind::CssRule),
            ("css-keyframes", SymbolKind::CssKeyframes),
            ("field", SymbolKind::Field),
        ];
        for (raw, expected) in cases {
            assert_eq!(SymbolKind::parse(raw), Ok(expected));
        }
    }

    #[test]
    fn unrecognized_symbol_kind_is_a_typed_rejection_not_a_default() {
        assert_eq!(
            SymbolKind::parse("totally-unknown-construct"),
            Err(UnrecognizedSymbolKind { raw: "totally-unknown-construct".to_string() })
        );
    }

    #[test]
    fn symbol_kind_parse_is_case_sensitive_not_normalized() {
        // "Function" must not silently match "function": accepting both
        // would let a careless extractor produce two spellings for the
        // same kind without either being an error.
        assert!(matches!(SymbolKind::parse("Function"), Err(UnrecognizedSymbolKind { .. })));
    }

    #[test]
    fn every_edge_kind_variant_round_trips_through_its_canonical_string() {
        let cases = [
            ("calls", EdgeKind::Calls),
            ("implements", EdgeKind::Implements),
            ("extends", EdgeKind::Extends),
            ("references", EdgeKind::References),
            ("tests_for", EdgeKind::TestsFor),
            ("renamed_to", EdgeKind::RenamedTo),
            ("contains", EdgeKind::Contains),
            ("instantiates", EdgeKind::Instantiates),
        ];
        for (raw, expected) in cases {
            assert_eq!(EdgeKind::parse(raw), Ok(expected));
        }
    }

    #[test]
    fn unrecognized_edge_kind_is_a_typed_rejection_not_a_default() {
        assert_eq!(
            EdgeKind::parse("teleports_to"),
            Err(UnrecognizedEdgeKind { raw: "teleports_to".to_string() })
        );
    }
}
