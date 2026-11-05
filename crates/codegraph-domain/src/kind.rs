//! B2 Task 1: closed symbol/edge kind enums. No "other"/catch-all variant
//! on either enum: an unrecognized raw kind string must be a typed
//! rejection at the promotion boundary (Task 5), never silently accepted
//! into an ambiguous bucket — the same discipline `domain::Classification`
//! uses by deliberately having no `Secret` variant (`schema.rs`'s comment:
//! "classification: secret must never become a valid in-memory value").

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Interface,
    Module,
    Const,
    Macro,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unrecognized symbol kind: {raw:?}")]
pub struct UnrecognizedSymbolKind {
    pub raw: String,
}

impl SymbolKind {
    /// The only way to obtain a `SymbolKind` from a raw string (as produced
    /// by a per-language tree-sitter extractor, Task 4). Exhaustive,
    /// case-sensitive, lowercase-kebab-case names; anything else is a typed
    /// rejection, never a default variant.
    pub fn parse(raw: &str) -> Result<Self, UnrecognizedSymbolKind> {
        match raw {
            "function" => Ok(Self::Function),
            "method" => Ok(Self::Method),
            "class" => Ok(Self::Class),
            "struct" => Ok(Self::Struct),
            "enum" => Ok(Self::Enum),
            "interface" => Ok(Self::Interface),
            "module" => Ok(Self::Module),
            "const" => Ok(Self::Const),
            "macro" => Ok(Self::Macro),
            _ => Err(UnrecognizedSymbolKind { raw: raw.to_string() }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Calls,
    Imports,
    Implements,
    Extends,
    References,
    TestsFor,
    RenamedTo,
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
            "imports" => Ok(Self::Imports),
            "implements" => Ok(Self::Implements),
            "extends" => Ok(Self::Extends),
            "references" => Ok(Self::References),
            "tests_for" => Ok(Self::TestsFor),
            "renamed_to" => Ok(Self::RenamedTo),
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
            ("class", SymbolKind::Class),
            ("struct", SymbolKind::Struct),
            ("enum", SymbolKind::Enum),
            ("interface", SymbolKind::Interface),
            ("module", SymbolKind::Module),
            ("const", SymbolKind::Const),
            ("macro", SymbolKind::Macro),
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
            ("imports", EdgeKind::Imports),
            ("implements", EdgeKind::Implements),
            ("extends", EdgeKind::Extends),
            ("references", EdgeKind::References),
            ("tests_for", EdgeKind::TestsFor),
            ("renamed_to", EdgeKind::RenamedTo),
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
