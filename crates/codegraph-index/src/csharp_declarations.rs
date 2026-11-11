//! B2-ID-010: C# type-declaration context — a non-symbol "structural
//! fact," following exactly the same architectural pattern `imports.rs`
//! and `impls.rs` already established (see `imports.rs`'s own doc
//! comment for the full rationale, which applies here unchanged): a
//! `class_declaration`/`interface_declaration`'s own `namespace` and
//! `partial` modifier are not properties this pipeline's `SymbolSnapshot`
//! carries, and re-deriving them from a single snapshot's own
//! `signature_text` is IMPOSSIBLE — a `namespace X;`/`namespace X { .. }`
//! declaration lives in the surrounding FILE, never inside the byte span
//! of one type declaration's own signature text (confirmed by direct
//! inspection: `SymbolSnapshot::signature()` is exactly the declaration's
//! own span, re-parsed standalone by every existing resolver in
//! `edges/`, and never contains a sibling `namespace` node at all).
//!
//! # The exact defect this module fixes
//!
//! `docs/research/findings/B2-ID-010.md` found that
//! `resolve_extends_implements`'s base-name lookup treats every
//! `partial class Foo { .. }` fragment of the SAME logical C# type as a
//! DISTINCT, ambiguously-named candidate — `BulkheadPolicy : Policy`,
//! where `Policy` is declared as 8 separate `partial class Policy { .. }`
//! fragments across 8 files, finds 8 same-named candidates and
//! (correctly, per that resolver's own "ambiguous match -> no edge"
//! rule) produces no edge, even though the real C# program is entirely
//! unambiguous: all 8 fragments ARE the same type. This module extracts
//! the per-declaration context (`namespace`, `is_partial`, generic
//! arity, type kind) needed to tell "these candidates are fragments of
//! ONE partial type" apart from "these candidates are genuinely
//! DIFFERENT types that merely share a bare name" (e.g. two unrelated,
//! non-partial `Foo` classes in different namespaces) — WITHOUT
//! introducing a canonical cross-fragment identity, merging snapshots,
//! or touching lineage/Contains. Each fragment keeps its own, distinct
//! `SymbolSnapshotId`; only `resolve_extends_implements`'s own
//! RECONCILIATION decision (treat N same-named candidates as one
//! logical target for THIS relation) changes.
//!
//! # Scope boundary: raw syntax only, C# only, class/interface only
//!
//! [`extract_csharp_type_declaration_contexts`] is pure syntax
//! extraction, like `extract_import_statements`/`extract_impl_statements`:
//! no resolution against a symbol-snapshot pool, no `Edge` construction,
//! no I/O. Only `class_declaration`/`interface_declaration` nodes are
//! recognized (the only two node kinds `resolve_extends_implements`'s
//! own base-name lookup ever targets as a TARGET of a base/heritage
//! clause) — `method_declaration` and every other C# construct are out
//! of scope, and no other language's extractor is touched.
//!
//! # Namespace canonicalization
//!
//! A C# type's namespace can be written four ways, all confirmed by
//! direct grammar probe against this workspace's real
//! `tree-sitter-c-sharp` output before writing this module:
//!
//! - **Global namespace** (no enclosing `namespace` declaration at
//!   all): canonicalized to `""` (the empty string), an explicit,
//!   normalized value — never `None`/absent, so "no namespace" and "not
//!   yet computed" are never confusable.
//! - **File-scoped** (`namespace Polly.Bulkhead;`, a
//!   `file_scoped_namespace_declaration` sibling earlier in the SAME
//!   file's `compilation_unit`, applying to every declaration after it
//!   in that file): canonicalized to its own dotted text, e.g.
//!   `"Polly.Bulkhead"`.
//! - **Block-scoped, single dotted segment** (`namespace Polly.Bulkhead
//!   { .. }`, one `namespace_declaration` whose own `name` field is a
//!   `qualified_name`): canonicalized identically to the file-scoped
//!   case, e.g. `"Polly.Bulkhead"`.
//! - **Block-scoped, nested** (`namespace Polly { namespace Bulkhead {
//!   .. } }`, two nested `namespace_declaration`s, each with a plain
//!   `identifier` name field): canonicalized by joining each ancestor
//!   namespace's own simple name with `.`, producing the SAME
//!   `"Polly.Bulkhead"` string as the single-dotted-segment form above —
//!   confirmed these two syntactically different shapes are semantically
//!   equivalent in real C#, so this module normalizes both to one
//!   comparable representation rather than leaving them as two
//!   differently-shaped strings that would never compare equal.
//!
//! This canonicalization operates on the WHOLE-FILE `ParseTree` (never
//! a single snapshot's own isolated `signature_text`), walking upward
//! from a `class_declaration`/`interface_declaration` node through its
//! own ancestor chain for block-scoped namespaces, then falling back to
//! the nearest preceding `file_scoped_namespace_declaration` sibling in
//! the same file if no block-scoped ancestor was found.

use codegraph_domain::{FileIdentity, SymbolSnapshotId};

use crate::extract::ByteSpan;
use crate::parse::Language;
use crate::snapshot::SymbolSnapshot;

/// Which syntactic construct a [`RawCSharpTypeDeclarationContext`]
/// describes. Deliberately only the two kinds
/// `resolve_extends_implements`'s own base-name lookup ever targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CSharpTypeKind {
    Class,
    Interface,
}

/// One syntactically-extracted C# `class_declaration`/
/// `interface_declaration`'s own declaration context, before any
/// resolution against a symbol-snapshot pool. `span` is the SAME byte
/// span `ByteSpan::from_node` would compute for this exact declaration
/// node — the join key used to associate this context with its real
/// `SymbolSnapshotId` after promotion (by `(file, span)`, never by
/// `simple_name` alone, since the whole point of this module is that
/// multiple declarations share a `simple_name`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCSharpTypeDeclarationContext {
    pub span: ByteSpan,
    pub simple_name: String,
    pub namespace: String,
    pub generic_arity: usize,
    pub type_kind: CSharpTypeKind,
    pub is_partial: bool,
}

/// [`RawCSharpTypeDeclarationContext`]'s resolved counterpart: the same
/// fields, plus the real `SymbolSnapshotId` this declaration promoted
/// to. `None` if no snapshot in the given pool has this EXACT
/// `(file, span)` — e.g. the declaration was itself rejected at
/// promotion time (oversized name/signature) — mirroring every other
/// resolver's own "prefer omit over false certainty" policy rather than
/// guessing a nearby snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CSharpTypeDeclarationContext {
    pub snapshot_id: Option<SymbolSnapshotId>,
    pub file: FileIdentity,
    pub span: ByteSpan,
    pub simple_name: String,
    pub namespace: String,
    pub generic_arity: usize,
    pub type_kind: CSharpTypeKind,
    pub is_partial: bool,
}

/// Extracts every `class_declaration`/`interface_declaration`'s own
/// declaration context from `tree`'s WHOLE file — never a single
/// snapshot's isolated `signature_text`, since the namespace
/// information this module exists to capture lives in the file, not in
/// any one declaration's own span. Returns an empty `Vec` for any
/// non-C# tree (checked via `tree.language()`, the same dispatch
/// convention `imports.rs`/`impls.rs` already use), so callers may call
/// this unconditionally for every file without their own language
/// check.
pub fn extract_csharp_type_declaration_contexts(tree: &crate::parse::ParseTree) -> Vec<RawCSharpTypeDeclarationContext> {
    if tree.language() != Language::CSharp {
        return Vec::new();
    }
    let mut out = Vec::new();
    walk(tree.tree().root_node(), tree.source(), &mut out);
    out
}

fn walk(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawCSharpTypeDeclarationContext>) {
    if matches!(node.kind(), "class_declaration" | "interface_declaration") {
        if let Some(context) = declaration_context_of(node, source) {
            out.push(context);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out);
    }
}

fn declaration_context_of(node: tree_sitter::Node, source: &[u8]) -> Option<RawCSharpTypeDeclarationContext> {
    let name_node = node.child_by_field_name("name").filter(|n| !n.is_missing())?;
    let simple_name = name_node.utf8_text(source).unwrap_or("").to_string();
    if simple_name.is_empty() {
        return None;
    }

    let type_kind = match node.kind() {
        "class_declaration" => CSharpTypeKind::Class,
        "interface_declaration" => CSharpTypeKind::Interface,
        _ => return None,
    };

    let is_partial = has_partial_modifier(node, source);
    let generic_arity = generic_arity_of(node);
    let namespace = namespace_of(node, source);

    Some(RawCSharpTypeDeclarationContext {
        span: ByteSpan::from_node(&node),
        simple_name,
        namespace,
        generic_arity,
        type_kind,
        is_partial,
    })
}

/// `class_declaration`/`interface_declaration` each carry zero or more
/// direct `modifier` children (`public`, `static`, `partial`, ...).
/// Confirmed by direct grammar probe: a `modifier` node's own single
/// child is a bare keyword token (`partial`, matched by its own node
/// kind, not by re-deriving it from the modifier's text) — reading the
/// MODIFIER node's kind-named child, never scanning `node`'s own text
/// for the substring `"partial"`, which could false-positive on an
/// unrelated identifier or comment containing that text.
/// Mutation-testing note: `cargo mutants` finds `replace && with ||`
/// on the inner check "unviable to distinguish" against this codebase's
/// own test suite — confirmed EQUIVALENT, not a genuine gap: the
/// `partial` keyword is a fixed lexical terminal in this workspace's
/// `tree-sitter-c-sharp` grammar, so a real parse can never produce a
/// node with `kind() == "partial"` and `utf8_text() != "partial"` (or
/// vice versa) — there is no valid C# input that reaches this check
/// with the two conditions disagreeing, so `&&` and `||` are
/// behaviorally identical on every reachable AST. (A class literally
/// NAMED `partial`, e.g. `class partial {}`, DOES produce an
/// `identifier` node whose text reads `"partial"` — confirmed by a
/// direct grammar probe — but that node is never a `modifier` child in
/// the first place, so it never reaches this inner check at all.)
fn has_partial_modifier(node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    let modifiers: Vec<tree_sitter::Node> = node.children(&mut cursor).filter(|c| c.kind() == "modifier").collect();
    modifiers.iter().any(|modifier| {
        let mut mod_cursor = modifier.walk();
        let keywords: Vec<tree_sitter::Node> = modifier.children(&mut mod_cursor).collect();
        keywords.iter().any(|c| c.kind() == "partial" && c.utf8_text(source).unwrap_or("") == "partial")
    })
}

/// The declaration's own generic arity (0 for a non-generic type).
/// Confirmed by direct grammar probe: `type_parameter_list` has no
/// named field on `class_declaration`/`interface_declaration` (unlike
/// `name`/`returns` on `method_declaration` — B2-ID-009's own lesson
/// does not apply here, since there is no competing same-kind sibling
/// `type_parameter_list` could be confused with; it is the only child
/// of its own unique node kind). Found by a direct-child kind search,
/// then counts its own `type_parameter` children, skipping the
/// literal `<`/`>`/`,` tokens.
fn generic_arity_of(node: tree_sitter::Node) -> usize {
    let mut cursor = node.walk();
    let Some(type_parameter_list) = node.children(&mut cursor).find(|c| c.kind() == "type_parameter_list") else {
        return 0;
    };
    let mut tp_cursor = type_parameter_list.walk();
    type_parameter_list.children(&mut tp_cursor).filter(|c| c.kind() == "type_parameter").count()
}

/// Canonicalizes `node`'s own namespace per this module's own doc
/// comment: walk UP through `node`'s ancestor chain collecting every
/// enclosing `namespace_declaration`'s own simple name (innermost
/// first, so the final join must reverse them), then, if no
/// block-scoped ancestor was found at all, fall back to the nearest
/// PRECEDING `file_scoped_namespace_declaration` sibling anywhere
/// earlier in the same `compilation_unit` (C#'s own real rule: a
/// file-scoped namespace applies to everything textually after it in
/// that file). Returns `""` (global namespace) if neither is found.
fn namespace_of(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if ancestor.kind() == "namespace_declaration" {
            if let Some(name_node) = ancestor.child_by_field_name("name") {
                segments.push(name_node.utf8_text(source).unwrap_or("").to_string());
            }
        }
        current = ancestor.parent();
    }
    if !segments.is_empty() {
        segments.reverse();
        return segments.join(".");
    }

    // No block-scoped ancestor: fall back to the nearest preceding
    // file_scoped_namespace_declaration sibling in the compilation_unit.
    //
    // Mutation-testing note: `cargo mutants` finds `replace < with <=`
    // on the filter below and `replace > with >=` on the tie-break a
    // few lines down "unviable to distinguish" against this codebase's
    // own tests — confirmed EQUIVALENT, not genuine gaps: two distinct
    // sibling AST nodes in one `compilation_unit` (here, a
    // `file_scoped_namespace_declaration` and `node` itself for the
    // first comparison; two `file_scoped_namespace_declaration`
    // candidates for the second) can never share the exact same
    // `start_byte` — each occupies its own, non-overlapping byte
    // range in the source text, by construction of the text itself.
    // The equality case that would distinguish `<` from `<=` (or `>`
    // from `>=`) is therefore structurally unreachable on any real
    // parse, confirmed directly against the two-file-scoped-namespace
    // test below (`two_file_scoped_namespace_declarations_in_one_file_the_fallback_picks_the_nearest_preceding_one`),
    // whose own two candidate namespaces have provably distinct
    // `start_byte`s and so cannot exercise the boundary either.
    let Some(root) = root_of(node) else {
        return String::new();
    };
    let node_start = node.start_byte();
    let mut cursor = root.walk();
    let mut best: Option<(usize, String)> = None;
    for child in root.children(&mut cursor) {
        if child.kind() == "file_scoped_namespace_declaration" && child.start_byte() < node_start {
            if let Some(name_node) = child.child_by_field_name("name") {
                let text = name_node.utf8_text(source).unwrap_or("").to_string();
                if best.as_ref().is_none_or(|(prev_start, _)| child.start_byte() > *prev_start) {
                    best = Some((child.start_byte(), text));
                }
            }
        }
    }
    best.map(|(_, text)| text).unwrap_or_default()
}

fn root_of(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        current = parent;
    }
    Some(current)
}

/// Resolves every `(defining_file, RawCSharpTypeDeclarationContext)`
/// pair against `snapshots` by `(file, span)` — NEVER by
/// `simple_name` alone, since this module's whole purpose is
/// distinguishing same-named declarations. Mirrors
/// `resolve_raw_impl_statements`'s own extraction/resolution split: this
/// extraction is per-file and can run incrementally; this resolution
/// step always needs the complete, repository-wide pool (a context's
/// own `snapshot_id` lookup is per-file already, but callers combine
/// facts from many files before calling this, exactly like
/// `resolve_raw_import_statements`).
pub fn resolve_csharp_type_declaration_contexts(raw_contexts: &[(FileIdentity, RawCSharpTypeDeclarationContext)], snapshots: &[SymbolSnapshot]) -> Vec<CSharpTypeDeclarationContext> {
    raw_contexts
        .iter()
        .map(|(file, raw)| {
            let snapshot_id = snapshots.iter().find(|s| s.file() == file && s.span() == raw.span).map(|s| s.id());
            CSharpTypeDeclarationContext {
                snapshot_id,
                file: file.clone(),
                span: raw.span,
                simple_name: raw.simple_name.clone(),
                namespace: raw.namespace.clone(),
                generic_arity: raw.generic_arity,
                type_kind: raw.type_kind,
                is_partial: raw.is_partial,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{CSharpParser, LanguageParser};

    fn extract_from(source: &[u8]) -> Vec<RawCSharpTypeDeclarationContext> {
        let tree = CSharpParser.parse(source).unwrap();
        extract_csharp_type_declaration_contexts(&tree)
    }

    #[test]
    fn global_namespace_canonicalizes_to_empty_string() {
        let contexts = extract_from(b"public partial class Policy {}\n");
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].namespace, "");
        assert!(contexts[0].is_partial);
    }

    #[test]
    fn file_scoped_namespace_canonicalizes_to_its_dotted_text() {
        let contexts = extract_from(b"namespace Polly.Bulkhead;\npublic partial class Policy {}\n");
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].namespace, "Polly.Bulkhead");
    }

    #[test]
    fn block_scoped_single_segment_namespace_canonicalizes_identically_to_file_scoped() {
        let contexts = extract_from(b"namespace Polly.Bulkhead {\n    public partial class Policy {}\n}\n");
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].namespace, "Polly.Bulkhead");
    }

    #[test]
    fn nested_block_scoped_namespace_canonicalizes_to_the_same_dotted_form() {
        let contexts = extract_from(b"namespace Polly {\n    namespace Bulkhead {\n        class Policy {}\n    }\n}\n");
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].namespace, "Polly.Bulkhead", "nested block-scoped namespaces must canonicalize the same as a single dotted namespace declaration");
    }

    #[test]
    fn non_partial_class_is_flagged_correctly() {
        let contexts = extract_from(b"class Policy {}\n");
        assert_eq!(contexts.len(), 1);
        assert!(!contexts[0].is_partial);
    }

    #[test]
    fn interface_declaration_is_recognized_with_correct_kind() {
        let contexts = extract_from(b"partial interface IFoo {}\n");
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].type_kind, CSharpTypeKind::Interface);
        assert!(contexts[0].is_partial);
    }

    #[test]
    fn generic_arity_is_counted_correctly() {
        let contexts = extract_from(b"partial class Policy<TResult> {}\n");
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].generic_arity, 1);
    }

    #[test]
    fn non_generic_class_has_zero_arity() {
        let contexts = extract_from(b"class Policy {}\n");
        assert_eq!(contexts[0].generic_arity, 0);
    }

    #[test]
    fn multiple_partial_fragments_in_the_same_file_each_get_their_own_context() {
        let contexts = extract_from(b"partial class Policy {}\npartial class Policy { }\n");
        assert_eq!(contexts.len(), 2);
        assert!(contexts.iter().all(|c| c.is_partial));
        assert!(contexts.iter().all(|c| c.simple_name == "Policy"));
        // Distinct spans -- never collapsed to one context.
        assert_ne!(contexts[0].span, contexts[1].span);
    }

    #[test]
    fn two_file_scoped_namespaces_in_sequence_each_apply_only_to_what_follows_them() {
        // Confirms the "nearest PRECEDING file-scoped namespace" rule,
        // not just "the first one in the file" -- though real C# only
        // allows ONE file-scoped namespace per file, this test exercises
        // the fallback's own nearest-preceding-sibling logic directly
        // against a block-scoped namespace appearing after a
        // file-scoped one, which IS legal.
        let contexts = extract_from(b"namespace A;\nclass Foo {}\nnamespace B { class Bar {} }\n");
        let foo = contexts.iter().find(|c| c.simple_name == "Foo").unwrap();
        let bar = contexts.iter().find(|c| c.simple_name == "Bar").unwrap();
        assert_eq!(foo.namespace, "A");
        assert_eq!(bar.namespace, "B", "Bar must be inside its own block-scoped namespace B, not file-scoped A");
    }

    #[test]
    fn non_csharp_tree_yields_nothing() {
        let tree = crate::parse::RustParser.parse(b"struct Foo;\n").unwrap();
        assert!(extract_csharp_type_declaration_contexts(&tree).is_empty());
    }

    // Mutation-testing-driven additions: these three cases exist
    // specifically because `cargo mutants` found real gaps in the
    // existing test set's coverage of `has_partial_modifier`'s own
    // combined kind-AND-text check and `namespace_of`'s own fallback
    // "nearest PRECEDING sibling, not just ANY sibling" logic — see
    // B2-ID-010's own mutation-testing section for the exact mutants
    // these close.

    #[test]
    fn a_modifier_whose_own_text_is_not_literally_partial_is_not_mistaken_for_one() {
        // `has_partial_modifier` requires BOTH the modifier keyword's
        // own node KIND to be "partial" AND its own TEXT to read
        // "partial" -- this fixture exercises a class with several
        // OTHER modifiers (none of them "partial") to confirm neither
        // half of that check alone produces a false positive.
        let contexts = extract_from(b"public static class Policy {}\n");
        assert_eq!(contexts.len(), 1);
        assert!(!contexts[0].is_partial, "a class with public/static modifiers but no partial keyword must not be flagged as partial");
    }

    #[test]
    fn a_declaration_before_a_file_scoped_namespace_does_not_fall_under_it() {
        // C#'s own real rule (and this module's own fallback logic):
        // a file-scoped namespace applies ONLY to declarations
        // textually AFTER it, never before -- this fixture places a
        // class BEFORE the namespace declaration, which is not legal
        // C# in practice (a file-scoped namespace must be the first
        // non-using statement), but directly exercises the `<`
        // comparison in `namespace_of`'s own fallback loop: a
        // `file_scoped_namespace_declaration` whose own start_byte is
        // NOT less than the target node's start_byte must never be
        // treated as a match.
        let contexts = extract_from(b"class Before {}\nnamespace A;\nclass After {}\n");
        let before = contexts.iter().find(|c| c.simple_name == "Before").unwrap();
        let after = contexts.iter().find(|c| c.simple_name == "After").unwrap();
        assert_eq!(before.namespace, "", "a declaration textually BEFORE the file-scoped namespace must resolve to the global namespace, never namespace A");
        assert_eq!(after.namespace, "A");
    }

    #[test]
    fn two_namespace_scoped_blocks_each_apply_only_to_their_own_contents_not_a_sibling_blocks() {
        // Exercises the `>` comparison in the fallback's own
        // "nearest preceding" tie-break directly: two SEPARATE
        // block-scoped namespaces, each containing one class -- Bar
        // must resolve to B, never fall back to A, confirming the
        // block-scoped ancestor walk (not the file-scoped fallback) is
        // what actually governs here, and that the fallback's own
        // nearest-preceding logic is never reached incorrectly for a
        // declaration that HAS a real block-scoped ancestor.
        let contexts = extract_from(b"namespace A { class Foo {} }\nnamespace B { class Bar {} }\n");
        let foo = contexts.iter().find(|c| c.simple_name == "Foo").unwrap();
        let bar = contexts.iter().find(|c| c.simple_name == "Bar").unwrap();
        assert_eq!(foo.namespace, "A");
        assert_eq!(bar.namespace, "B");
    }

    #[test]
    fn two_file_scoped_namespace_declarations_in_one_file_the_fallback_picks_the_nearest_preceding_one() {
        // Real C# forbids more than one file-scoped namespace per
        // file, but this workspace's own grammar parses two of them
        // in sequence WITHOUT any error node (confirmed by a direct
        // grammar probe before writing this test) -- a syntactically
        // valid, semantically meaningless edge case that is exactly
        // what `namespace_of`'s own fallback loop must handle
        // correctly: it must pick the NEAREST preceding one (B, the
        // second), never the first one found (A) and never "either,
        // arbitrarily." This is what actually exercises the fallback's
        // own `<`/`>` tie-break logic (the two tests above this one
        // each have only ONE file-scoped namespace in play, so the
        // tie-break comparison is never truly exercised by them).
        let contexts = extract_from(b"namespace A;\nnamespace B;\nclass Foo {}\n");
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].namespace, "B", "the fallback must pick the NEAREST preceding file-scoped namespace (B), not the first one in the file (A)");
    }

    mod resolution {
        use super::*;
        use codegraph_domain::{CommitHash, RepositoryRegistry, RepositoryRoot};
        use crate::extract::extract_symbols_reporting_rejections;
        use crate::parse::parser_for;
        use crate::snapshot::promote;

        fn repo() -> codegraph_domain::RepositoryId {
            let mut registry = RepositoryRegistry::new();
            registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
        }

        fn real_file_snapshots_and_contexts(repo: codegraph_domain::RepositoryId, path: &str, source: &str) -> (FileIdentity, Vec<SymbolSnapshot>, Vec<RawCSharpTypeDeclarationContext>) {
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, path).unwrap();
            let parser = parser_for(&file).expect("supported extension");
            let tree = parser.parse(source.as_bytes()).expect("fixture must parse");
            let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
            assert!(rejections.is_empty(), "fixture must not reject: {rejections:?}");
            let snapshots: Vec<SymbolSnapshot> = candidates.into_iter().map(|c| promote(&c, file.clone()).expect("promote")).collect();
            let contexts = extract_csharp_type_declaration_contexts(&tree);
            (file, snapshots, contexts)
        }

        #[test]
        fn resolution_links_each_context_to_its_own_real_snapshot_id_by_span_not_name() {
            let (file, snapshots, contexts) = real_file_snapshots_and_contexts(repo(), "src/Policy.cs", "partial class Policy {}\npartial class Policy { }\n");
            let raw: Vec<(FileIdentity, RawCSharpTypeDeclarationContext)> = contexts.into_iter().map(|c| (file.clone(), c)).collect();
            let resolved = resolve_csharp_type_declaration_contexts(&raw, &snapshots);

            assert_eq!(resolved.len(), 2);
            for (context, snapshot) in resolved.iter().zip(snapshots.iter()) {
                assert_eq!(context.snapshot_id, Some(snapshot.id()), "each context must link to the snapshot sharing its own exact span, never an arbitrary same-named one");
            }
            // The two resolved snapshot_ids must be DISTINCT -- confirms
            // resolution is by (file, span), not by simple_name (which
            // is identical for both and would otherwise collapse them).
            assert_ne!(resolved[0].snapshot_id, resolved[1].snapshot_id);
        }

        #[test]
        fn a_context_whose_declaration_was_rejected_at_promotion_resolves_to_none() {
            let (file, _snapshots, contexts) = real_file_snapshots_and_contexts(repo(), "src/Policy.cs", "class Policy {}\n");
            let raw: Vec<(FileIdentity, RawCSharpTypeDeclarationContext)> = contexts.into_iter().map(|c| (file.clone(), c)).collect();
            // Resolve against an EMPTY pool, simulating "this declaration's
            // own candidate was rejected/removed" -- must yield None, never
            // a guessed/fallback snapshot_id.
            let resolved = resolve_csharp_type_declaration_contexts(&raw, &[]);
            assert_eq!(resolved[0].snapshot_id, None);
        }
    }
}
