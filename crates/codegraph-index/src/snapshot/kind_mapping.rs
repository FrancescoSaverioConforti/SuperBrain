//! B2 Task 5: `kind_hint -> SymbolKind` mapping — the exhaustive table
//! over every raw `kind_hint` string Task 4's nine extractors actually
//! emit. NOT `codegraph_domain::SymbolKind::parse` (which parses
//! `SymbolKind`'s own unrelated canonical string form, e.g.
//! `"method-signature"` — see that method's doc comment for why Task
//! 1's original assumption that it WAS this mapping was wrong). This
//! mapping is many-to-one: several different raw tree-sitter node kinds,
//! across different languages, collapse to the same `SymbolKind` — see
//! the table on [`map_kind_hint`] for every language's exact raw strings.
//!
//! No fallback, no default, no "nearest known kind," no normalization:
//! an unrecognized `kind_hint` is always `PromotionError::UnrecognizedKind`.
//! This table is exhaustive over exactly the set Task 4's current
//! extractors can produce — see `docs/superpowers/plans/2026-11-05-b2-code-graph.md`
//! section 8b for the policy on how/when this set grows.

use codegraph_domain::SymbolKind;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unrecognized kind_hint: {kind_hint:?} (no SymbolKind mapping exists for this raw tree-sitter node kind)")]
pub struct UnrecognizedKindHint {
    pub kind_hint: String,
}

/// Maps a raw `kind_hint` (as `extract::RawSymbolCandidate::kind_hint`
/// carries it) to a validated `SymbolKind`, or a typed rejection.
///
/// | `kind_hint` | Language(s) | `SymbolKind` |
/// |---|---|---|
/// | `function_item` | Rust | `Function` |
/// | `function_signature_item` | Rust | `MethodSignature` |
/// | `function_definition` | Python, C++ | `Function` |
/// | `function_declaration` | JavaScript, TypeScript, Tsx | `Function` |
/// | `class_definition` | Python | `Class` |
/// | `class_declaration` | C#, JavaScript, TypeScript, Tsx | `Class` |
/// | `class_specifier` | C++ | `Class` |
/// | `struct_item` | Rust | `Struct` |
/// | `struct_specifier` | C++ | `Struct` |
/// | `enum_item` | Rust | `Enum` |
/// | `trait_item` | Rust | `Interface` (Rust's trait is this pipeline's closest match to the cross-language "interface" concept — see the note below) |
/// | `interface_declaration` | C# | `Interface` |
/// | `method_declaration` | C# | `Method` |
/// | `method_definition` | JavaScript, TypeScript, Tsx | `Method` |
/// | `method_signature` | TypeScript | `MethodSignature` |
/// | `arrow_function` | JavaScript, TypeScript, Tsx | `Function` |
/// | `element_with_id` | HTML | `HtmlIdElement` |
/// | `rule_set` | CSS | `CssRule` |
/// | `keyframes_statement` | CSS | `CssKeyframes` |
/// | `self_attribute_assignment` | Python | `Field` (Task 9.5; a synthetic `kind_hint`, like `element_with_id` — no single raw tree-sitter node kind names "an instance attribute assignment" on its own, since Python's grammar uses the same `assignment` node for every assignment statement; the Python extractor itself recognizes the specific `attribute`-on-the-left, `self`-receiver shape and labels it this way before it ever reaches this mapping) |
///
/// Note on `trait_item -> Interface`: a Rust `trait` and a C# `interface`
/// are not semantically identical (a trait can carry default method
/// bodies and associated types; this mapping does not claim they are
/// interchangeable in every respect) — they are mapped to the same
/// `SymbolKind` only because `SymbolKind`'s current baseline (Task 1,
/// kept deliberately minimal per section 8b) has no separate `Trait`
/// variant, and introducing one is exactly the kind of coverage-expansion
/// decision section 8b says must wait for real-repository evidence, not
/// be added speculatively here. `function_definition` is also listed for
/// both Python and C++ deliberately: Task 4's own extractors both use
/// exactly that raw node-kind string for their respective language's
/// function/method construct (confirmed against each extractor's own
/// source), and this mapping does not need to know which language
/// produced it to pick the same `SymbolKind` for it — the ambiguity (if
/// any) is resolved by `FileIdentity`'s own path/repository context the
/// caller already carries, not by this table.
pub fn map_kind_hint(kind_hint: &str) -> Result<SymbolKind, UnrecognizedKindHint> {
    match kind_hint {
        "function_item" | "function_definition" | "function_declaration" | "arrow_function" => Ok(SymbolKind::Function),
        "function_signature_item" | "method_signature" => Ok(SymbolKind::MethodSignature),
        "class_definition" | "class_declaration" | "class_specifier" => Ok(SymbolKind::Class),
        "struct_item" | "struct_specifier" => Ok(SymbolKind::Struct),
        "enum_item" => Ok(SymbolKind::Enum),
        "trait_item" | "interface_declaration" => Ok(SymbolKind::Interface),
        "method_declaration" | "method_definition" => Ok(SymbolKind::Method),
        "element_with_id" => Ok(SymbolKind::HtmlIdElement),
        "rule_set" => Ok(SymbolKind::CssRule),
        "keyframes_statement" => Ok(SymbolKind::CssKeyframes),
        "self_attribute_assignment" => Ok(SymbolKind::Field),
        _ => Err(UnrecognizedKindHint {
            kind_hint: kind_hint.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_hint_task_4_actually_produces_maps_to_a_symbol_kind() {
        // Exhaustive over the real, grep-confirmed set of kind_hint
        // strings every one of the nine extractors in
        // crates/codegraph-index/src/extract/ can emit — not a guessed
        // subset. See this module's own doc table for the full mapping
        // and which language(s) produce each one.
        let all_real_kind_hints = [
            // Rust
            "struct_item",
            "enum_item",
            "trait_item",
            "function_item",
            "function_signature_item",
            // Python
            "class_definition",
            "function_definition",
            // C#
            "interface_declaration",
            "class_declaration",
            "method_declaration",
            // C++ (class_specifier/struct_specifier/function_definition
            // already covered above where shared with Rust/Python)
            "class_specifier",
            "struct_specifier",
            // JS/TS/Tsx (class_declaration covered above)
            "function_declaration",
            "method_definition",
            "method_signature",
            "arrow_function",
            // HTML
            "element_with_id",
            // CSS
            "rule_set",
            "keyframes_statement",
            // Python Field (Task 9.5)
            "self_attribute_assignment",
        ];
        for kind_hint in all_real_kind_hints {
            assert!(map_kind_hint(kind_hint).is_ok(), "expected a mapping for real kind_hint {kind_hint:?}, got none");
        }
    }

    #[test]
    fn rust_kind_hints_map_correctly() {
        assert_eq!(map_kind_hint("struct_item"), Ok(SymbolKind::Struct));
        assert_eq!(map_kind_hint("enum_item"), Ok(SymbolKind::Enum));
        assert_eq!(map_kind_hint("trait_item"), Ok(SymbolKind::Interface));
        assert_eq!(map_kind_hint("function_item"), Ok(SymbolKind::Function));
        assert_eq!(map_kind_hint("function_signature_item"), Ok(SymbolKind::MethodSignature));
    }

    #[test]
    fn python_kind_hints_map_correctly() {
        assert_eq!(map_kind_hint("class_definition"), Ok(SymbolKind::Class));
        assert_eq!(map_kind_hint("function_definition"), Ok(SymbolKind::Function));
    }

    #[test]
    fn csharp_kind_hints_map_correctly() {
        assert_eq!(map_kind_hint("interface_declaration"), Ok(SymbolKind::Interface));
        assert_eq!(map_kind_hint("class_declaration"), Ok(SymbolKind::Class));
        assert_eq!(map_kind_hint("method_declaration"), Ok(SymbolKind::Method));
    }

    #[test]
    fn cpp_kind_hints_map_correctly() {
        assert_eq!(map_kind_hint("class_specifier"), Ok(SymbolKind::Class));
        assert_eq!(map_kind_hint("struct_specifier"), Ok(SymbolKind::Struct));
        assert_eq!(map_kind_hint("function_definition"), Ok(SymbolKind::Function));
    }

    #[test]
    fn javascript_family_kind_hints_map_correctly() {
        assert_eq!(map_kind_hint("class_declaration"), Ok(SymbolKind::Class));
        assert_eq!(map_kind_hint("function_declaration"), Ok(SymbolKind::Function));
        assert_eq!(map_kind_hint("method_definition"), Ok(SymbolKind::Method));
        assert_eq!(map_kind_hint("method_signature"), Ok(SymbolKind::MethodSignature));
        assert_eq!(map_kind_hint("arrow_function"), Ok(SymbolKind::Function));
    }

    #[test]
    fn html_kind_hint_maps_correctly() {
        assert_eq!(map_kind_hint("element_with_id"), Ok(SymbolKind::HtmlIdElement));
    }

    #[test]
    fn css_kind_hints_map_correctly() {
        assert_eq!(map_kind_hint("rule_set"), Ok(SymbolKind::CssRule));
        assert_eq!(map_kind_hint("keyframes_statement"), Ok(SymbolKind::CssKeyframes));
    }

    #[test]
    fn python_field_kind_hint_maps_correctly() {
        assert_eq!(map_kind_hint("self_attribute_assignment"), Ok(SymbolKind::Field));
    }

    #[test]
    fn an_unrecognized_kind_hint_is_a_typed_rejection_not_a_default() {
        assert_eq!(
            map_kind_hint("totally_unknown_node_kind"),
            Err(UnrecognizedKindHint {
                kind_hint: "totally_unknown_node_kind".to_string()
            })
        );
    }

    #[test]
    fn negative_tests_per_language_family_reject_a_plausible_but_wrong_string() {
        // One deliberately-plausible-looking but NOT actually produced
        // string per language family, confirming this table does not
        // silently accept near-misses.
        assert!(map_kind_hint("fn_item").is_err(), "Rust: not the real node-kind spelling");
        assert!(map_kind_hint("def_statement").is_err(), "Python: not the real node-kind spelling");
        assert!(map_kind_hint("interface_body").is_err(), "C#: a real but DIFFERENT node kind, never a symbol itself");
        assert!(map_kind_hint("field_declaration").is_err(), "C++: a real but DIFFERENT node kind (not a symbol in this pipeline)");
        assert!(map_kind_hint("lexical_declaration").is_err(), "JS family: a real but DIFFERENT node kind, never promoted directly");
        assert!(map_kind_hint("start_tag").is_err(), "HTML: a real but DIFFERENT node kind, never the synthetic kind_hint itself");
        assert!(map_kind_hint("declaration").is_err(), "CSS: a real but DIFFERENT node kind (a bare property, never its own symbol)");
    }

    #[test]
    fn case_sensitivity_is_preserved_not_normalized() {
        assert!(map_kind_hint("Function_Item").is_err(), "must not silently match a differently-cased spelling");
    }
}
