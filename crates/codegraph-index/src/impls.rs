//! B2-ID-008, Task 1: raw `impl` block extraction — a non-symbol
//! "structural fact," following exactly the same architectural pattern
//! `imports.rs` already established for file-level import facts (see
//! `imports.rs`'s own doc comment for the full rationale, which applies
//! here unchanged): a real Rust `impl` block is not "owned" by any one
//! symbol it contains, so it is modeled as its own fact type rather than
//! forced into `Edge { from: SymbolSnapshotId, .. }`.
//!
//! # The exact defect this module fixes
//!
//! `docs/research/findings/B2-ID-008.md` found that Rust `impl` blocks
//! are never extracted as symbols at all (`extract::rust` recognizes
//! `struct_item`/`enum_item`/`trait_item`/`function_item`/
//! `function_signature_item`, but no `impl_item`), so trait-implementation
//! relationships are entirely invisible to this pipeline. This module
//! does not change `extract::rust` or introduce a new `SymbolKind` —
//! that would wrongly claim an `impl` block is itself a named,
//! independently-addressable symbol, which it is not (it has no name of
//! its own; `impl Trait for Type` is a relationship between two other
//! symbols). Instead, [`extract_impl_statements`] extracts `impl_item`
//! nodes directly from the whole-file tree, exactly the same
//! file/module-level granularity `imports.rs` already uses for the same
//! reason (see `imports.rs`'s own "Why a separate model, not an `Edge`"
//! section) — an `impl` block is no more "owned" by one symbol in the
//! file than an import statement is.
//!
//! # Named-field extraction only — never a positional search
//!
//! A grammar probe (confirmed by direct parse against this workspace's
//! `tree_sitter_rust` version, not assumed) showed `impl_item` exposes
//! two named fields: `"trait"` (present only for a trait impl, e.g.
//! `impl Trait for Type`) and `"type"` (always present — the
//! implementing type). The `"type"` field's own node kind varies: a
//! plain type is a `type_identifier` directly; a parameterized type
//! (`impl<T> Container<T>`) is a `generic_type` whose OWN first child is
//! the base type's `type_identifier` (`generic_type`'s text is the full
//! `"Container<T>"`; this module extracts only the base name
//! `"Container"`). This module always reads these two fields via
//! `child_by_field_name`, NEVER via a positional search for a
//! `type_identifier` child — a positional search is exactly the bug
//! class B2-ID-008 is scoped to fix (the same mistake, if made here,
//! would misidentify the TRAIT's own `type_identifier` as the
//! implementing type, or vice versa, whenever both are plain
//! `type_identifier` nodes at the same syntactic depth).
//!
//! # Scope boundary: raw syntax only, Rust only
//!
//! [`extract_impl_statements`] itself is pure syntax extraction, like
//! `extract_import_statements`: no resolution against a symbol-snapshot
//! pool, no `Edge` construction. Only `Language::Rust` is wired into its
//! language dispatch; generalizing to another language's own
//! impl/trait-ish syntax is explicitly out of scope for B2-ID-008.
//!
//! # Task 2: `ImplFact` resolution and derived `Implements` edges
//!
//! [`ImplFact`] is [`RawImplStatement`]'s resolved counterpart, mirroring
//! `imports.rs`'s own `RawImportStatement` -> `ImportFact` shape exactly:
//! [`resolve_impl_facts`]/[`resolve_raw_impl_statements`] look up
//! `self_type_text`/`trait_type_text` by name against a given
//! `&[SymbolSnapshot]` pool, the same lookup convention every other
//! resolver in this crate already uses (`s.name().as_str() == text`).
//! [`ImplTargetResolution`] mirrors `ImportResolution`'s own
//! Resolved/zero-candidates/ambiguous three-way shape, renamed from
//! `ImportResolution`'s own vocabulary (`Internal`/`External`) because
//! `ImplFact` is a different context: "External" specifically connotes
//! "this import names something outside the repository" in `imports.rs`,
//! which is not quite what "an impl's trait/self-type name was not found
//! in the given pool" means here, so this module uses its own
//! `Resolved`/`Unresolved`/`Ambiguous` names instead.
//!
//! An inherent impl (`impl Type { .. }`, no `for` clause) has
//! `trait_type_text: None`, and `ImplFact::trait_resolution` is `None` to
//! match — never `Some(Unresolved)`. There is no trait to resolve at all
//! for an inherent impl, which is a different claim from "a trait was
//! named but not found," so this module keeps the two distinguishable at
//! the type level rather than collapsing "no trait" and "unresolved
//! trait" into the same `Unresolved` representation.
//!
//! [`resolve_implements_from_impl_facts`] is the one piece with no
//! `imports.rs` equivalent: imports never produce `Edge`s (see this
//! module's own first section above, and `imports.rs`'s own "Why a
//! separate model, not an `Edge`" section, for why), but a real `impl
//! Trait for Type` block IS a genuine, syntactically certain
//! trait-implementation relationship between two symbols, so — unlike
//! `ImportFact` — `ImplFact` has a derived-edge step. It emits one
//! `EdgeKind::Implements` edge per `ImplFact` whose self-type AND trait
//! both resolved to exactly one candidate each, built with
//! `ResolutionMethod::StaticAnalysis` evidence via the exact same
//! underlying `codegraph_domain::Evidence::new` call
//! `edges::resolution::resolved_evidence` wraps (that helper itself is
//! `pub(super)` to the `edges` module and not reachable from here, so
//! this module calls the domain constructor directly rather than
//! widening that helper's visibility — see [`implements_evidence`]'s own
//! doc comment). An inherent impl (`trait_resolution: None`) never
//! reaches this function's edge-emitting branch at all, since there is
//! no `Some(Resolved(trait_id))` to match — confirmed by a dedicated
//! test, not just absence-by-omission in another test.

use codegraph_domain::{Evidence, FileIdentity, ResolutionMethod, SymbolSnapshotId};
use thiserror::Error;

use crate::edges::Edge;
use crate::extract::ByteSpan;
use crate::imports::reject_duplicate_ids;
use crate::parse::Language;
use crate::snapshot::SymbolSnapshot;

/// One syntactically-extracted `impl` block, before any resolution
/// against a symbol-snapshot pool. `self_type_text` is always the base
/// type name only (generic arguments stripped, e.g. `"Container"` for
/// `impl<T> Container<T>`, never the full `"Container<T>"` text).
/// `trait_type_text` is `None` for an inherent impl (`impl Type { .. }`)
/// and `Some` for a trait impl (`impl Trait for Type { .. }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImplStatement {
    pub self_type_text: String,
    pub trait_type_text: Option<String>,
    pub has_generics: bool,
    pub has_lifetime_params: bool,
    pub span: ByteSpan,
    pub associated_items: Vec<AssociatedItemFact>,
}

/// One item declared directly inside a specific `impl` block (not
/// resolved against anything — just "this impl block's own
/// `declaration_list` contains a function_item shaped like this").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociatedItemFact {
    pub name: String,
    pub kind: AssociatedItemKind,
    pub span: ByteSpan,
}

/// Which of the two shapes this plan's scope distinguishes an
/// associated `function_item` as. Deliberately only two variants: see
/// this module's module-level doc comment and the plan's own Task 1
/// brief for why a third "default trait method" variant is explicitly
/// out of scope here (that distinction belongs to the trait's own
/// declaration, not to what one specific impl block declares).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssociatedItemKind {
    /// A `function_item` whose `parameters` child has a `self_parameter`
    /// child — takes a receiver (`&self`, `&mut self`, or by-value
    /// `self`).
    Method,
    /// A `function_item` with a body but no receiver parameter — the
    /// `DirEntry::normal`/`CommandSet::new` constructor shape.
    AssociatedFunction,
}

/// Extracts every `impl` block found anywhere in `tree` (the WHOLE
/// file's own parse tree — not one symbol's signature text; see this
/// module's doc comment for why that granularity matches
/// `extract_import_statements`'s own). Pure syntax extraction: never
/// resolves `self_type_text`/`trait_type_text` against anything: this
/// function alone never consults a symbol-snapshot pool.
pub fn extract_impl_statements(tree: &crate::parse::ParseTree) -> Vec<RawImplStatement> {
    let mut out = Vec::new();
    walk_for_impls(tree.tree().root_node(), tree.source(), tree.language(), &mut out);
    out
}

fn walk_for_impls(node: tree_sitter::Node, source: &[u8], language: Language, out: &mut Vec<RawImplStatement>) {
    if language == Language::Rust {
        collect_rust_impl(node, source, out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_impls(child, source, language, out);
    }
}

fn collect_rust_impl(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawImplStatement>) {
    if node.kind() != "impl_item" {
        return;
    }
    let Some(type_field) = node.child_by_field_name("type") else {
        // No "type" field at all is not a shape a well-formed
        // `impl_item` can produce (confirmed by direct parse across
        // every fixture this module's own tests cover); prefer omitting
        // the fact over fabricating a self type, mirroring
        // `extract::rust`'s own "prefer omit over false certainty"
        // policy for a missing expected name.
        return;
    };
    let Some(self_type_text) = base_type_name(type_field, source) else {
        return;
    };

    let trait_type_text = node.child_by_field_name("trait").and_then(|trait_field| base_type_name(trait_field, source));

    let type_parameters = node.children(&mut node.walk()).find(|c| c.kind() == "type_parameters");
    let has_generics = type_parameters.is_some();
    let has_lifetime_params = type_parameters.is_some_and(|tp| tp.children(&mut tp.walk()).any(|c| c.kind() == "lifetime_parameter"));

    let associated_items = node
        .child_by_field_name("body")
        .or_else(|| node.children(&mut node.walk()).find(|c| c.kind() == "declaration_list"))
        .map(|body| collect_associated_items(body, source))
        .unwrap_or_default();

    out.push(RawImplStatement {
        self_type_text,
        trait_type_text,
        has_generics,
        has_lifetime_params,
        span: ByteSpan::from_node(&node),
        associated_items,
    });
}

/// Extracts the base type name out of a `type`-field or `trait`-field
/// node: directly for a `type_identifier`, or from its OWN first
/// `type_identifier` child for a `generic_type` (never the full
/// `"Name<Args>"` text). Returns `None` for any other node kind (e.g. a
/// `scoped_type_identifier` or a primitive type used as a trait/self
/// type) — out of this plan's own scope per the brief's named-field
/// instruction, which only describes `type_identifier` and
/// `generic_type` shapes; prefer omitting the fact over guessing at an
/// unconfirmed shape.
fn base_type_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(text_of(node, source)),
        "generic_type" => {
            let mut cursor = node.walk();
            let base = node.children(&mut cursor).find(|c| c.kind() == "type_identifier")?;
            Some(text_of(base, source))
        }
        _ => None,
    }
}

fn collect_associated_items(declaration_list: tree_sitter::Node, source: &[u8]) -> Vec<AssociatedItemFact> {
    let mut out = Vec::new();
    let mut cursor = declaration_list.walk();
    for child in declaration_list.children(&mut cursor) {
        if child.kind() != "function_item" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let name = text_of(name_node, source);
        let kind = classify_associated_item(child);
        out.push(AssociatedItemFact {
            name,
            kind,
            span: ByteSpan::from_node(&child),
        });
    }
    out
}

fn classify_associated_item(function_item: tree_sitter::Node) -> AssociatedItemKind {
    let has_receiver = function_item
        .child_by_field_name("parameters")
        .is_some_and(|params| params.children(&mut params.walk()).any(|c| c.kind() == "self_parameter"));
    if has_receiver {
        AssociatedItemKind::Method
    } else {
        AssociatedItemKind::AssociatedFunction
    }
}

fn text_of(node: tree_sitter::Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

/// One `impl` block's resolved, semantic shape — [`RawImplStatement`]'s
/// counterpart after resolving its two name fields against a
/// `&[SymbolSnapshot]` pool. `associated_items` is carried through
/// unchanged from the raw statement (Task 1 already extracted it; this
/// task does not re-derive or re-validate it). `trait_resolution` is
/// `None` exactly when `trait_type_text` is `None` — an inherent impl has
/// no trait to resolve at all, which is a distinct claim from "a trait
/// was named but could not be resolved" ([`ImplTargetResolution::Unresolved`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplFact {
    pub defining_file: FileIdentity,
    pub span: ByteSpan,
    pub self_type_text: String,
    pub trait_type_text: Option<String>,
    pub has_generics: bool,
    pub has_lifetime_params: bool,
    pub associated_items: Vec<AssociatedItemFact>,
    pub self_type_resolution: ImplTargetResolution,
    pub trait_resolution: Option<ImplTargetResolution>,
}

/// How one name field of an [`ImplFact`] (`self_type_text` or
/// `trait_type_text`) resolves against a given candidate pool. Mirrors
/// `ImportResolution`'s own three-way shape (see this module's own doc
/// comment for why the variants are named differently here rather than
/// reusing `Internal`/`External`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImplTargetResolution {
    /// Exactly one candidate in the given pool matched this name.
    Resolved(SymbolSnapshotId),
    /// Zero candidates matched — e.g. the trait/type is external
    /// (imported from another crate, or simply not in the given pool),
    /// exactly like `ImportResolution::External`'s own meaning.
    Unresolved,
    /// More than one candidate in the given pool shares this name —
    /// never arbitrarily picked, per this crate's "no edge beats a false
    /// edge" discipline.
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ImplFactError {
    #[error("duplicate SymbolSnapshotId in the candidate pool: a caller must never include the same snapshot twice")]
    DuplicateSnapshotId,
}

/// Resolves every `impl` block extracted from `defining_file`'s own
/// full-file `tree` against `snapshots` — the convenience wrapper over
/// one file's tree, mirroring `resolve_import_facts`'s own shape exactly.
pub fn resolve_impl_facts(defining_file: &FileIdentity, tree: &crate::parse::ParseTree, snapshots: &[SymbolSnapshot]) -> Result<Vec<ImplFact>, ImplFactError> {
    let raw_statements: Vec<(FileIdentity, RawImplStatement)> = extract_impl_statements(tree).into_iter().map(|s| (defining_file.clone(), s)).collect();
    resolve_raw_impl_statements(&raw_statements, snapshots)
}

/// The global semantic resolution phase: takes already-extracted,
/// already-paired `(defining_file, RawImplStatement)` facts — possibly
/// from many files — and resolves each one's `self_type_text`/
/// `trait_type_text` against `snapshots`, the full current pool, by NAME
/// (`s.name().as_str() == text`), the same lookup convention every other
/// resolver in this crate and `imports.rs` already use. Mirrors
/// `resolve_raw_import_statements`'s own two-function split for the same
/// reason: extraction is per-file and can run incrementally; resolution
/// always needs the complete, repository-wide pool.
pub fn resolve_raw_impl_statements(raw_statements: &[(FileIdentity, RawImplStatement)], snapshots: &[SymbolSnapshot]) -> Result<Vec<ImplFact>, ImplFactError> {
    reject_duplicate_ids(snapshots).map_err(|_| ImplFactError::DuplicateSnapshotId)?;

    let mut facts = Vec::with_capacity(raw_statements.len());
    for (defining_file, statement) in raw_statements {
        let self_type_resolution = resolve_target_by_name(&statement.self_type_text, snapshots);
        let trait_resolution = statement.trait_type_text.as_ref().map(|trait_name| resolve_target_by_name(trait_name, snapshots));

        facts.push(ImplFact {
            defining_file: defining_file.clone(),
            span: statement.span,
            self_type_text: statement.self_type_text.clone(),
            trait_type_text: statement.trait_type_text.clone(),
            has_generics: statement.has_generics,
            has_lifetime_params: statement.has_lifetime_params,
            associated_items: statement.associated_items.clone(),
            self_type_resolution,
            trait_resolution,
        });
    }

    Ok(facts)
}

fn resolve_target_by_name(name: &str, snapshots: &[SymbolSnapshot]) -> ImplTargetResolution {
    let candidates: Vec<&SymbolSnapshot> = snapshots.iter().filter(|s| s.name().as_str() == name).collect();
    match candidates.len() {
        1 => ImplTargetResolution::Resolved(candidates[0].id()),
        0 => ImplTargetResolution::Unresolved,
        _ => ImplTargetResolution::Ambiguous,
    }
}

/// The derived-edge step with no `imports.rs` equivalent (see this
/// module's own doc comment): for each `ImplFact` whose
/// `trait_resolution` is `Some(ImplTargetResolution::Resolved(trait_id))`
/// AND whose `self_type_resolution` is `ImplTargetResolution::Resolved(self_id)`,
/// emits one `Edge { from: self_id, to: trait_id, kind: EdgeKind::Implements, .. }`.
/// An inherent impl (`trait_resolution: None`) can never match the first
/// condition, so it never reaches the `edges.push` below — confirmed by
/// a dedicated test (`inherent_impl_produces_zero_implements_edges`), not
/// just absence-by-omission in another test. Deduplicates by `(from,
/// to)`: per [B2-ID-002](../../docs/research/findings/B2-ID-002.md)'s
/// "one edge per semantic relation, never one per syntactic occurrence"
/// discipline, if the same type were to implement the same trait via more
/// than one impl block (not valid Rust, but this pipeline parses source
/// that may not compile, so defensive code does not assume the input is
/// always compiler-valid), this collapses to one edge, not two.
pub fn resolve_implements_from_impl_facts(facts: &[ImplFact]) -> Vec<Edge> {
    let mut edges = Vec::new();
    let mut seen_pairs: std::collections::HashSet<(SymbolSnapshotId, SymbolSnapshotId)> = std::collections::HashSet::new();

    for fact in facts {
        let Some(ImplTargetResolution::Resolved(trait_id)) = fact.trait_resolution else {
            continue;
        };
        let ImplTargetResolution::Resolved(self_id) = fact.self_type_resolution else {
            continue;
        };
        if !seen_pairs.insert((self_id, trait_id)) {
            continue;
        }
        edges.push(Edge {
            from: self_id,
            to: trait_id,
            kind: codegraph_domain::EdgeKind::Implements,
            evidence: implements_evidence(&format!(
                "real `impl {} for {}` block: both the trait and self type resolved to exactly one candidate each",
                fact.trait_type_text.as_deref().unwrap_or(""),
                fact.self_type_text
            )),
        });
    }

    edges
}

/// Builds `Evidence` for a resolved `Implements` edge derived from a real
/// `impl Trait for Type` block, always `ResolutionMethod::StaticAnalysis`
/// with no source span — calling `codegraph_domain::Evidence::new`
/// directly rather than `crate::edges::resolution::resolved_evidence`,
/// because that helper is `pub(super)` (visible only inside `edges` and
/// its submodules) and this module lives outside `edges` entirely.
/// Widening `resolved_evidence`'s visibility was the other option
/// considered; calling the domain constructor directly here was chosen
/// instead as the less invasive change — it touches nothing in the
/// `edges` module (no risk to its own "one auditable path into
/// `StaticAnalysis` evidence" invariant for every resolver already
/// living there) and produces the IDENTICAL result `resolved_evidence`
/// would: the same `ResolutionMethod::StaticAnalysis`, the same `None`
/// source span, through the same underlying `Evidence::new` call. A
/// reader cannot tell a Rust-sourced `Implements` edge built here apart
/// from a TypeScript-sourced one built by `resolved_evidence` in
/// `edges::extends_implements` by its evidence shape.
fn implements_evidence(note: &str) -> Evidence {
    Evidence::new(ResolutionMethod::StaticAnalysis, None, note).expect("edge evidence notes are built short and bounded by this module")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{LanguageParser, RustParser};

    fn parse_rust(source: &str) -> crate::parse::ParseTree {
        RustParser.parse(source.as_bytes()).unwrap()
    }

    #[test]
    fn trait_impl_is_extracted_with_both_type_names() {
        let tree = parse_rust("trait Trait {} struct Type; impl Trait for Type { fn foo(&self) {} }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].trait_type_text, Some("Trait".to_string()));
        assert_eq!(facts[0].self_type_text, "Type");
    }

    #[test]
    fn inherent_impl_has_no_trait_type_text() {
        let tree = parse_rust("struct Type; impl Type { fn new() -> Self { Type } }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].trait_type_text, None);
        assert_eq!(facts[0].self_type_text, "Type");
    }

    #[test]
    fn generic_impl_strips_generic_args_from_self_type_text_and_sets_has_generics() {
        let tree = parse_rust("struct Container<T>(T); impl<T> Container<T> { fn get(&self) -> &T { &self.0 } }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].self_type_text, "Container");
        assert!(facts[0].has_generics);
        assert!(!facts[0].has_lifetime_params);
    }

    #[test]
    fn impl_with_lifetime_parameter_sets_has_lifetime_params() {
        let tree = parse_rust("struct Holder<'a>(&'a str); impl<'a> Holder<'a> { fn get(&self) -> &'a str { self.0 } }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].has_lifetime_params);
        assert!(facts[0].has_generics, "a lifetime parameter is still a type_parameters child");
    }

    #[test]
    fn multiple_impl_blocks_for_the_same_type_are_extracted_as_two_separate_facts() {
        let tree = parse_rust(
            "trait Bar {} struct Foo; impl Foo { fn inherent_method(&self) {} } impl Bar for Foo { fn bar_method(&self) {} }",
        );
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 2, "expected two separate RawImplStatements, never merged into one");
        assert!(facts.iter().any(|f| f.trait_type_text.is_none() && f.self_type_text == "Foo"));
        assert!(facts.iter().any(|f| f.trait_type_text == Some("Bar".to_string()) && f.self_type_text == "Foo"));
    }

    #[test]
    fn multiple_traits_implemented_by_the_same_type_across_separate_blocks_each_get_their_own_fact() {
        let tree = parse_rust("trait A {} trait B {} struct Foo; impl A for Foo { fn a(&self) {} } impl B for Foo { fn b(&self) {} }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 2);
        assert!(facts.iter().any(|f| f.trait_type_text == Some("A".to_string()) && f.self_type_text == "Foo"));
        assert!(facts.iter().any(|f| f.trait_type_text == Some("B".to_string()) && f.self_type_text == "Foo"));
    }

    #[test]
    fn one_trait_implemented_by_multiple_different_types_each_get_their_own_fact() {
        let tree = parse_rust(
            "trait Shape {} struct Circle; struct Square; impl Shape for Circle { fn area(&self) {} } impl Shape for Square { fn area(&self) {} }",
        );
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 2);
        assert!(facts.iter().any(|f| f.trait_type_text == Some("Shape".to_string()) && f.self_type_text == "Circle"));
        assert!(facts.iter().any(|f| f.trait_type_text == Some("Shape".to_string()) && f.self_type_text == "Square"));
    }

    #[test]
    fn associated_items_are_correctly_classified_as_method_or_associated_function() {
        let tree = parse_rust("struct S; impl S { fn new() -> Self { S } fn bump(&mut self) {} }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        let items = &facts[0].associated_items;
        assert_eq!(items.len(), 2);
        let new_item = items.iter().find(|i| i.name == "new").unwrap();
        assert_eq!(new_item.kind, AssociatedItemKind::AssociatedFunction);
        let bump_item = items.iter().find(|i| i.name == "bump").unwrap();
        assert_eq!(bump_item.kind, AssociatedItemKind::Method);
    }

    #[test]
    fn generic_trait_impl_extracts_both_names_via_named_fields_not_positional_search() {
        // The exact shape the OLD positional-search bug would have
        // gotten wrong: `impl<T: Display> Describe for Wrapper<T>` has
        // a `type_identifier` ("Describe") at the same syntactic depth
        // as `generic_type`'s own `type_identifier` ("Wrapper"). A
        // positional "find the first type_identifier child" search
        // would conflate the trait name with the self type (or vice
        // versa); only `child_by_field_name("trait")` /
        // `child_by_field_name("type")` get both right.
        let tree = parse_rust("use std::fmt::Display; trait Describe { fn describe(&self); } struct Wrapper<T>(T); impl<T: Display> Describe for Wrapper<T> { fn describe(&self) {} }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].trait_type_text, Some("Describe".to_string()));
        assert_eq!(facts[0].self_type_text, "Wrapper".to_string());
    }

    #[test]
    fn parsing_the_same_tree_twice_produces_deterministic_identical_facts() {
        let tree = parse_rust("trait Trait {} struct Type; impl Trait for Type { fn foo(&self) {} }\nimpl Type { fn new() -> Self { Type } }");

        let first = extract_impl_statements(&tree);
        let second = extract_impl_statements(&tree);

        assert_eq!(first, second);
    }

    // --- Task 2: ImplFact resolution + derived Implements edges ---
    mod resolution {
        use super::*;
        use codegraph_domain::{CommitHash, EdgeKind, RepositoryRegistry, RepositoryRoot};

        use crate::extract::RawSymbolCandidate;
        use crate::snapshot::promote;

        fn test_repo() -> codegraph_domain::RepositoryId {
            let mut registry = RepositoryRegistry::new();
            registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
        }

        fn test_file(repo: codegraph_domain::RepositoryId, path: &str) -> FileIdentity {
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            FileIdentity::parse(repo, commit, path).unwrap()
        }

        fn test_snapshot(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
            let file = test_file(repo, path);
            let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
            promote(&candidate, file).unwrap()
        }

        fn raw_trait_impl(self_type: &str, trait_type: &str) -> RawImplStatement {
            RawImplStatement {
                self_type_text: self_type.to_string(),
                trait_type_text: Some(trait_type.to_string()),
                has_generics: false,
                has_lifetime_params: false,
                span: ByteSpan { start: 0, end: 10 },
                associated_items: Vec::new(),
            }
        }

        fn raw_inherent_impl(self_type: &str) -> RawImplStatement {
            RawImplStatement {
                self_type_text: self_type.to_string(),
                trait_type_text: None,
                has_generics: false,
                has_lifetime_params: false,
                span: ByteSpan { start: 0, end: 10 },
                associated_items: Vec::new(),
            }
        }

        #[test]
        fn trait_impl_with_both_names_resolved_produces_one_implements_edge() {
            let repo = test_repo();
            let defining_file = test_file(repo, "src/circle.rs");
            let trait_snapshot = test_snapshot(repo, "src/shape.rs", "Shape", "trait_item", "trait Shape { fn area(&self) -> f64; }");
            let self_snapshot = test_snapshot(repo, "src/circle.rs", "Circle", "struct_item", "struct Circle;");
            let raw = vec![(defining_file, raw_trait_impl("Circle", "Shape"))];

            let facts = resolve_raw_impl_statements(&raw, &[trait_snapshot.clone(), self_snapshot.clone()]).unwrap();

            assert_eq!(facts.len(), 1);
            assert_eq!(facts[0].self_type_resolution, ImplTargetResolution::Resolved(self_snapshot.id()));
            assert_eq!(facts[0].trait_resolution, Some(ImplTargetResolution::Resolved(trait_snapshot.id())));

            let edges = resolve_implements_from_impl_facts(&facts);
            assert_eq!(edges.len(), 1);
            assert_eq!(edges[0].from, self_snapshot.id());
            assert_eq!(edges[0].to, trait_snapshot.id());
            assert_eq!(edges[0].kind, EdgeKind::Implements);
            assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
        }

        #[test]
        fn inherent_impl_produces_zero_implements_edges() {
            // Dedicated test, not absence-by-omission: an inherent impl
            // (no trait) must have trait_resolution == None, and must
            // never reach resolve_implements_from_impl_facts's edge-
            // emitting branch at all.
            let repo = test_repo();
            let defining_file = test_file(repo, "src/circle.rs");
            let self_snapshot = test_snapshot(repo, "src/circle.rs", "Circle", "struct_item", "struct Circle;");
            let raw = vec![(defining_file, raw_inherent_impl("Circle"))];

            let facts = resolve_raw_impl_statements(&raw, &[self_snapshot.clone()]).unwrap();

            assert_eq!(facts.len(), 1);
            assert_eq!(facts[0].trait_resolution, None, "an inherent impl has no trait to resolve at all, not an unresolved one");
            assert_eq!(facts[0].self_type_resolution, ImplTargetResolution::Resolved(self_snapshot.id()));

            let edges = resolve_implements_from_impl_facts(&facts);
            assert!(edges.is_empty(), "an inherent impl must never produce an Implements edge");
        }

        #[test]
        fn unresolved_external_trait_produces_no_edge() {
            // impl Display for MyType where Display is never in the pool
            // (e.g. it's from std::fmt).
            let repo = test_repo();
            let defining_file = test_file(repo, "src/mytype.rs");
            let self_snapshot = test_snapshot(repo, "src/mytype.rs", "MyType", "struct_item", "struct MyType;");
            let raw = vec![(defining_file, raw_trait_impl("MyType", "Display"))];

            let facts = resolve_raw_impl_statements(&raw, &[self_snapshot.clone()]).unwrap();

            assert_eq!(facts.len(), 1);
            assert_eq!(facts[0].trait_resolution, Some(ImplTargetResolution::Unresolved));

            let edges = resolve_implements_from_impl_facts(&facts);
            assert!(edges.is_empty(), "an unresolved external trait must never produce an edge");
        }

        #[test]
        fn unresolved_self_type_produces_no_edge() {
            // impl MyTrait for String where String (the implementing
            // type) is never in the pool.
            let repo = test_repo();
            let defining_file = test_file(repo, "src/mytrait.rs");
            let trait_snapshot = test_snapshot(repo, "src/mytrait.rs", "MyTrait", "trait_item", "trait MyTrait {}");
            let raw = vec![(defining_file, raw_trait_impl("String", "MyTrait"))];

            let facts = resolve_raw_impl_statements(&raw, &[trait_snapshot.clone()]).unwrap();

            assert_eq!(facts.len(), 1);
            assert_eq!(facts[0].self_type_resolution, ImplTargetResolution::Unresolved);

            let edges = resolve_implements_from_impl_facts(&facts);
            assert!(edges.is_empty(), "an unresolved self type must never produce an edge");
        }

        #[test]
        fn ambiguous_trait_name_produces_no_edge() {
            // Two different snapshots in the pool share the trait's name.
            let repo = test_repo();
            let defining_file = test_file(repo, "src/circle.rs");
            let trait_a = test_snapshot(repo, "src/a.rs", "Shape", "trait_item", "trait Shape {}");
            let trait_b = test_snapshot(repo, "src/b.rs", "Shape", "trait_item", "trait Shape {}");
            let self_snapshot = test_snapshot(repo, "src/circle.rs", "Circle", "struct_item", "struct Circle;");
            let raw = vec![(defining_file, raw_trait_impl("Circle", "Shape"))];

            let facts = resolve_raw_impl_statements(&raw, &[trait_a, trait_b, self_snapshot]).unwrap();

            assert_eq!(facts.len(), 1);
            assert_eq!(facts[0].trait_resolution, Some(ImplTargetResolution::Ambiguous));

            let edges = resolve_implements_from_impl_facts(&facts);
            assert!(edges.is_empty(), "an ambiguous trait name must never be arbitrarily picked into an edge");
        }

        #[test]
        fn duplicate_type_trait_pair_across_two_impl_facts_collapses_to_one_edge() {
            // Defensive dedup: constructed directly (two ImplFacts with
            // identical self/trait text), independent of whether real
            // compiler-valid Rust could produce this input (it could
            // not — one type cannot implement the same trait twice).
            let repo = test_repo();
            let defining_file = test_file(repo, "src/circle.rs");
            let trait_snapshot = test_snapshot(repo, "src/shape.rs", "Shape", "trait_item", "trait Shape {}");
            let self_snapshot = test_snapshot(repo, "src/circle.rs", "Circle", "struct_item", "struct Circle;");
            let raw = vec![(defining_file.clone(), raw_trait_impl("Circle", "Shape")), (defining_file, raw_trait_impl("Circle", "Shape"))];

            let facts = resolve_raw_impl_statements(&raw, &[trait_snapshot.clone(), self_snapshot.clone()]).unwrap();
            assert_eq!(facts.len(), 2, "two separate raw statements still produce two separate ImplFacts");

            let edges = resolve_implements_from_impl_facts(&facts);
            assert_eq!(edges.len(), 1, "the same (Type, Trait) pair across two facts must collapse to one edge, not two");
            assert_eq!(edges[0].from, self_snapshot.id());
            assert_eq!(edges[0].to, trait_snapshot.id());
        }

        #[test]
        fn resolving_the_same_raw_statements_twice_produces_deterministic_identical_output() {
            let repo = test_repo();
            let defining_file = test_file(repo, "src/circle.rs");
            let trait_snapshot = test_snapshot(repo, "src/shape.rs", "Shape", "trait_item", "trait Shape {}");
            let self_snapshot = test_snapshot(repo, "src/circle.rs", "Circle", "struct_item", "struct Circle;");
            let raw = vec![(defining_file, raw_trait_impl("Circle", "Shape"))];
            let pool = vec![trait_snapshot, self_snapshot];

            let first_facts = resolve_raw_impl_statements(&raw, &pool).unwrap();
            let second_facts = resolve_raw_impl_statements(&raw, &pool).unwrap();
            assert_eq!(first_facts, second_facts);

            let first_edges = resolve_implements_from_impl_facts(&first_facts);
            let second_edges = resolve_implements_from_impl_facts(&second_facts);
            assert_eq!(first_edges, second_edges);
        }

        #[test]
        fn a_duplicate_snapshot_id_in_the_pool_is_a_typed_rejection() {
            let repo = test_repo();
            let defining_file = test_file(repo, "src/circle.rs");
            let self_snapshot = test_snapshot(repo, "src/circle.rs", "Circle", "struct_item", "struct Circle;");
            let raw = vec![(defining_file, raw_inherent_impl("Circle"))];

            let result = resolve_raw_impl_statements(&raw, &[self_snapshot.clone(), self_snapshot]);

            assert_eq!(result.err(), Some(ImplFactError::DuplicateSnapshotId));
        }
    }
}
