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
