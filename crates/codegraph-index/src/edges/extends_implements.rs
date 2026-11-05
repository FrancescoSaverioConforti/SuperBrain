//! B2 Task 7: `Extends`/`Implements` edges. Per-language syntax
//! confirmed against real parses before writing this logic (see each
//! language's own comment below for the exact node shapes).
//!
//! # Syntactically explicit vs. ambiguous-by-syntax languages
//!
//! Python (`class X(Base)`), Rust (`impl Trait for Type`), JavaScript's
//! `extends` and TypeScript/Tsx's `extends`/`implements` each have a
//! syntax that ALREADY distinguishes Extends from Implements (or, for
//! Python and plain JS, has only one construct, always `Extends` — see
//! below) without needing to know anything about the target's own kind.
//! `base_names_in` tags every collected name with that syntactic
//! `EdgeKind` directly (`Some(kind)`) for these languages, and
//! `edge_kind_for` always honors that tag over any kind lookup. These
//! resolve to `StaticAnalysis` evidence directly once the base name
//! matches exactly one candidate.
//!
//! C# (`class Derived : Base, IFoo`) and C++ (`class Derived : public
//! Base`) do NOT distinguish a base class from an implemented interface
//! syntactically — both appear in the same base-list construct. For
//! these two languages ONLY, `base_names_in` tags each name with `None`
//! (defer), and `edge_kind_for` falls back to looking up the base
//! name's `SymbolKind` among the given candidates: a candidate whose
//! kind is `Interface` resolves to `Implements`; any other matched kind
//! (`Class`, `Struct`, ...) resolves to `Extends`. If the base name is
//! not found among the given candidates at all, NEITHER edge kind is
//! guessed — this resolver does not assume "no known kind means it's
//! probably a class," since that would be exactly the kind of overclaim
//! the task forbids; it produces no edge, per "no edge beats a false
//! edge."
//!
//! Python's base-class list CAN syntactically contain more than one
//! base (multiple inheritance), and this resolver treats each
//! comma-separated base identically (one potential `Extends` edge per
//! base), never assuming only the first matters.

use codegraph_domain::{EdgeKind, SymbolKind};

use super::resolution::{resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
use crate::edges::calls::parser_for;
use crate::parse::Language;
use crate::snapshot::SymbolSnapshot;

pub fn resolve_extends_implements(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    for snapshot in lookup.snapshots() {
        let Some(parser) = parser_for(snapshot.file()) else {
            continue;
        };
        let Ok(tree) = parser.parse(snapshot.signature().as_str().as_bytes()) else {
            continue;
        };

        for (base_name, syntactic_kind) in base_names_in(&tree) {
            let candidates = lookup.find_by_name(&base_name, snapshot.id());
            if candidates.len() != 1 {
                // Zero: base not found among the given pool (e.g. a
                // stdlib/third-party base type) -> no edge. More than
                // one: ambiguous same-named base across files -> no
                // edge either (unlike Calls, inheritance is a 1:1
                // structural claim about THIS declaration's one actual
                // base, so an ambiguous same-name match cannot honestly
                // be split into multiple heuristic edges the way a call
                // site's multiple real overload candidates can).
                continue;
            }
            let target = candidates[0];
            let kind = edge_kind_for(syntactic_kind, target.kind());
            let evidence = resolved_evidence(&format!(
                "{:?} base/heritage clause names {base_name:?}, resolved to exactly one candidate of kind {:?}",
                tree.language(),
                target.kind()
            ));
            edges.push(Edge {
                from: snapshot.id(),
                to: target.id(),
                kind,
                evidence,
            });
        }
    }

    Ok(edges)
}

/// Picks the final `EdgeKind` for one resolved base/interface name.
/// `syntactic_kind` is `Some(kind)` when the SOURCE SYNTAX ITSELF
/// already said which kind applies — Rust's `impl Trait for Type`
/// (always `Implements`), Python's single base-class construct (always
/// `Extends`), JavaScript's `extends` (always `Extends`), and
/// TypeScript/Tsx's `extends` (`Extends`) vs. `implements`
/// (`Implements`), each tagged at the point `base_names_in` collects
/// them — see `collect_js_heritage`. It is `None` only for C#/C++, whose
/// `base_list`/`base_class_clause` syntax does NOT distinguish a base
/// class from an implemented interface at all; for those, and ONLY for
/// those, this function falls back to the target's own already-resolved
/// `SymbolKind` (`Interface` -> `Implements`, anything else ->
/// `Extends`). A syntactic tag, when present, is NEVER overridden by the
/// target's kind — the earlier version of this resolver incorrectly
/// routed TypeScript/Tsx through the same kind-based fallback as C#/C++
/// despite TS/Tsx's `implements` keyword being just as syntactically
/// explicit as Rust's `for` clause; that was a real defect (TS/Tsx's own
/// syntax was being ignored in favor of a lookup that could disagree
/// with it), fixed here by threading the clause tag through instead of
/// discarding it before this function runs.
fn edge_kind_for(syntactic_kind: Option<EdgeKind>, target_kind: SymbolKind) -> EdgeKind {
    syntactic_kind.unwrap_or_else(|| if target_kind == SymbolKind::Interface { EdgeKind::Implements } else { EdgeKind::Extends })
}

/// Every base-class/trait/interface name found in `tree`'s one
/// construct (a `class_definition`/`impl_item`/`class_declaration`/
/// `class_specifier` re-parsed standalone from a `SymbolSnapshot`'s own
/// signature text — the same re-parse-the-signature approach `calls.rs`
/// and `imports.rs` use), paired with the `EdgeKind` the SOURCE SYNTAX
/// implies when the language's grammar makes that unambiguous, or
/// `None` when it does not (C#/C++ only — see `edge_kind_for`'s doc
/// comment for what happens then).
fn base_names_in(tree: &crate::parse::ParseTree) -> Vec<(String, Option<EdgeKind>)> {
    let mut names = Vec::new();
    match tree.language() {
        Language::Python => {
            let mut plain_names = Vec::new();
            collect_python_bases(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, Some(EdgeKind::Extends))));
        }
        Language::Rust => {
            let mut plain_names = Vec::new();
            collect_rust_impl_trait(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, Some(EdgeKind::Implements))));
        }
        Language::JavaScript => {
            // JS has only `extends` (no `implements` keyword at all in
            // this grammar) — always Extends.
            let mut plain_names = Vec::new();
            collect_js_heritage(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, Some(EdgeKind::Extends))));
        }
        Language::TypeScript | Language::Tsx => {
            collect_ts_heritage_tagged(tree.tree().root_node(), tree.source(), &mut names);
        }
        Language::CSharp => {
            let mut plain_names = Vec::new();
            collect_csharp_base_list(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, None)));
        }
        Language::Cpp => {
            let mut plain_names = Vec::new();
            collect_cpp_base_clause(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, None)));
        }
        Language::Html | Language::Css => {}
    }
    names
}

fn collect_python_bases(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "class_definition" {
        let mut cursor = node.walk();
        let found_args = node.children(&mut cursor).find(|c| c.kind() == "argument_list");
        if let Some(args) = found_args {
            let mut arg_cursor = args.walk();
            for child in args.children(&mut arg_cursor) {
                match child.kind() {
                    "identifier" => out.push(text_of(child, source)),
                    "attribute" => {
                        // `nn.Module` -> use the LAST identifier
                        // ("Module"), confirmed against this
                        // workspace's real tree-sitter-python output:
                        // an `attribute` node's children are
                        // `identifier . identifier`.
                        let mut attr_cursor = child.walk();
                        if let Some(last) = child.children(&mut attr_cursor).filter(|c| c.kind() == "identifier").last() {
                            out.push(text_of(last, source));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_python_bases(child, source, out);
    }
}

fn collect_rust_impl_trait(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "impl_item" {
        // `impl Trait for Type`: children are
        // `impl type_identifier(Trait) for type_identifier(Type) ...`
        // — the TRAIT name is the first type_identifier, only present
        // when a `for` clause exists at all (a bare `impl Type { ... }`
        // inherent impl has no trait to record here).
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        let has_for_clause = children.iter().any(|c| c.kind() == "for");
        if has_for_clause {
            if let Some(trait_name) = children.iter().find(|c| c.kind() == "type_identifier") {
                out.push(text_of(*trait_name, source));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_impl_trait(child, source, out);
    }
}

fn collect_csharp_base_list(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "base_list" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                out.push(text_of(child, source));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_csharp_base_list(child, source, out);
    }
}

fn collect_cpp_base_clause(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "base_class_clause" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                out.push(text_of(child, source));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_cpp_base_clause(child, source, out);
    }
}

fn collect_js_heritage(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    // Plain JavaScript grammar only ever has a bare `extends identifier`
    // in `class_heritage` — no `implements` keyword exists in this
    // grammar at all, so every name collected here is unconditionally
    // Extends (tagged by the caller in `base_names_in`).
    if matches!(node.kind(), "class_heritage") {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "identifier" | "type_identifier") {
                out.push(text_of(child, source));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_js_heritage(child, source, out);
    }
}

/// TypeScript/Tsx's `class_heritage` can carry BOTH a plain `extends
/// identifier` child AND a nested `implements_clause` on the same
/// class. Unlike plain JS, this grammar's `implements` keyword is a
/// real syntactic marker, just as explicit as Rust's `for` clause or
/// Python's base-class list — so each name is tagged with the
/// `EdgeKind` its OWN clause implies, never left to a target-kind
/// fallback. This is the fix for a real defect: an earlier version of
/// this resolver collected every heritage name into one undifferentiated
/// list and then decided Extends vs. Implements by looking up the
/// resolved target's `SymbolKind`, identical to the C#/C++ fallback —
/// which meant a `class X extends Y` where `Y` happened to be an
/// `Interface`-kind snapshot would have been wrongly reported as
/// `Implements`, and `class X implements Z` where `Z` was NOT
/// `Interface`-kind would have been wrongly reported as `Extends`. Both
/// are now impossible: the clause tag is carried from the parse tree
/// straight through to `edge_kind_for`, which never overrides a
/// syntactic tag with a kind lookup.
fn collect_ts_heritage_tagged(node: tree_sitter::Node, source: &[u8], out: &mut Vec<(String, Option<EdgeKind>)>) {
    // Confirmed against this workspace's real tree-sitter-typescript
    // output: `class_heritage`'s direct children are themselves wrapper
    // nodes — `extends_clause` (containing one `identifier`/
    // `type_identifier`) and/or `implements_clause` (containing one or
    // more `type_identifier`s) — never a bare identifier directly.
    if matches!(node.kind(), "class_heritage") {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let tag = match child.kind() {
                "extends_clause" => Some(EdgeKind::Extends),
                "implements_clause" => Some(EdgeKind::Implements),
                _ => None,
            };
            if let Some(tag) = tag {
                let mut clause_cursor = child.walk();
                for grandchild in child.children(&mut clause_cursor) {
                    if matches!(grandchild.kind(), "identifier" | "type_identifier") {
                        out.push((text_of(grandchild, source), Some(tag)));
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ts_heritage_tagged(child, source, out);
    }
}

fn text_of(node: tree_sitter::Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot};

    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::promote;

    fn test_snapshot(path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    #[test]
    fn python_class_base_resolves_to_extends() {
        let base = test_snapshot("src/base.py", "Module", "class_definition", "class Module:\n    pass");
        let derived = test_snapshot("src/model.py", "MyModel", "class_definition", "class MyModel(Module):\n    pass");
        let snapshots = vec![base.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, derived.id());
        assert_eq!(edges[0].to, base.id());
        assert_eq!(edges[0].kind, EdgeKind::Extends);
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn python_dotted_base_resolves_by_its_last_segment() {
        // The exact deep-learning-relevant shape: class MyModel(nn.Module).
        let module = test_snapshot("src/nn.py", "Module", "class_definition", "class Module:\n    pass");
        let model = test_snapshot("src/model.py", "MyModel", "class_definition", "class MyModel(nn.Module):\n    pass");
        let snapshots = vec![module.clone(), model.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, module.id());
        assert_eq!(edges[0].kind, EdgeKind::Extends);
    }

    #[test]
    fn rust_impl_trait_for_type_resolves_to_implements() {
        let trait_snapshot = test_snapshot("src/shape.rs", "Shape", "trait_item", "trait Shape { fn area(&self) -> f64; }");
        let impl_snapshot = test_snapshot("src/circle.rs", "Circle", "struct_item", "struct Circle;");
        // The impl itself needs its own snapshot to be the EDGE SOURCE;
        // model it as a struct-kind snapshot whose signature text is the
        // impl block (this resolver walks the signature text's tree
        // looking for impl_item regardless of what kind_hint produced
        // the outer snapshot).
        let impl_block = test_snapshot("src/circle.rs", "Circle", "struct_item", "impl Shape for Circle { fn area(&self) -> f64 { 1.0 } }");
        let snapshots = vec![trait_snapshot.clone(), impl_snapshot, impl_block.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        let impl_edges: Vec<_> = edges.iter().filter(|e| e.from == impl_block.id()).collect();
        assert_eq!(impl_edges.len(), 1);
        assert_eq!(impl_edges[0].to, trait_snapshot.id());
        assert_eq!(impl_edges[0].kind, EdgeKind::Implements);
    }

    #[test]
    fn a_bare_inherent_rust_impl_with_no_for_clause_produces_no_edge() {
        // `impl Circle { ... }` (no `for Trait`) is an inherent impl —
        // it names no trait at all, and must not be mistaken for one.
        // Pins the `has_for_clause` gate specifically.
        let circle = test_snapshot("src/circle.rs", "Circle", "struct_item", "struct Circle;");
        let inherent_impl = test_snapshot("src/circle.rs", "Circle", "struct_item", "impl Circle { fn area(&self) -> f64 { 1.0 } }");
        let snapshots = vec![circle, inherent_impl.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        let impl_edges: Vec<_> = edges.iter().filter(|e| e.from == inherent_impl.id()).collect();
        assert!(impl_edges.is_empty(), "a bare inherent impl with no `for` clause must never produce an Implements edge");
    }

    #[test]
    fn javascript_extends_resolves_to_extends() {
        let base = test_snapshot("src/base.js", "Base", "class_declaration", "class Base {}");
        let derived = test_snapshot("src/derived.js", "Derived", "class_declaration", "class Derived extends Base {}");
        let snapshots = vec![base.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Extends);
        assert_eq!(edges[0].to, base.id());
    }

    #[test]
    fn typescript_implements_resolves_to_implements_via_syntax() {
        let interface = test_snapshot("src/ifoo.ts", "IFoo", "interface_declaration", "interface IFoo {}");
        let implementor = test_snapshot("src/c.ts", "C", "class_declaration", "class C implements IFoo {}");
        let snapshots = vec![interface.clone(), implementor.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Implements);
        assert_eq!(edges[0].to, interface.id());
    }

    #[test]
    fn typescript_extends_resolves_to_extends_even_when_the_target_kind_is_interface() {
        // The target's resolved SymbolKind happens to be Interface, but
        // the source syntax says `extends`, not `implements` — the
        // clause tag must win over any kind lookup. Before the fix,
        // this would have been wrongly reported as Implements because
        // TS/Tsx was routed through the same kind-based fallback as
        // C#/C++.
        let base_interface_shaped = test_snapshot("src/ifoo.ts", "IFoo", "interface_declaration", "interface IFoo {}");
        let derived = test_snapshot("src/c.ts", "C", "class_declaration", "class C extends IFoo {}");
        let snapshots = vec![base_interface_shaped.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Extends, "extends must resolve to Extends regardless of the target's own SymbolKind");
        assert_eq!(edges[0].to, base_interface_shaped.id());
    }

    #[test]
    fn typescript_implements_resolves_to_implements_even_when_the_target_kind_is_not_interface() {
        // The target's resolved SymbolKind is Class (not Interface), but
        // the source syntax says `implements` — the clause tag must win
        // over any kind lookup. Before the fix, this would have been
        // wrongly reported as Extends.
        let base_class_shaped = test_snapshot("src/base.ts", "Base", "class_declaration", "class Base {}");
        let implementor = test_snapshot("src/c.ts", "C", "class_declaration", "class C implements Base {}");
        let snapshots = vec![base_class_shaped.clone(), implementor.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Implements, "implements must resolve to Implements regardless of the target's own SymbolKind");
        assert_eq!(edges[0].to, base_class_shaped.id());
    }

    #[test]
    fn tsx_extends_and_implements_on_the_same_class_are_both_tagged_correctly() {
        // TS/Tsx grammar allows both clauses on one class declaration:
        // `class X extends Y implements Z`. Confirms both names are
        // collected AND each keeps its own correct tag, not just
        // whichever clause happens to be processed first.
        let base = test_snapshot("src/base.tsx", "Base", "class_declaration", "class Base {}");
        let interface = test_snapshot("src/iface.tsx", "IFoo", "interface_declaration", "interface IFoo {}");
        let derived = test_snapshot("src/derived.tsx", "Derived", "class_declaration", "class Derived extends Base implements IFoo {}");
        let snapshots = vec![base.clone(), interface.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 2);
        let extends_edge = edges.iter().find(|e| e.kind == EdgeKind::Extends).expect("an Extends edge must exist");
        assert_eq!(extends_edge.to, base.id());
        let implements_edge = edges.iter().find(|e| e.kind == EdgeKind::Implements).expect("an Implements edge must exist");
        assert_eq!(implements_edge.to, interface.id());
    }

    #[test]
    fn csharp_base_list_resolves_implements_vs_extends_by_target_kind() {
        let base_class = test_snapshot("src/base.cs", "Base", "class_declaration", "class Base {}");
        let interface = test_snapshot("src/ifoo.cs", "IFoo", "interface_declaration", "interface IFoo {}");
        let derived = test_snapshot("src/derived.cs", "Derived", "class_declaration", "class Derived : Base, IFoo {}");
        let snapshots = vec![base_class.clone(), interface.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        let to_base = edges.iter().find(|e| e.to == base_class.id()).expect("expected an edge to Base");
        assert_eq!(to_base.kind, EdgeKind::Extends);
        let to_interface = edges.iter().find(|e| e.to == interface.id()).expect("expected an edge to IFoo");
        assert_eq!(to_interface.kind, EdgeKind::Implements);
    }

    #[test]
    fn cpp_base_class_clause_resolves_extends() {
        let base = test_snapshot("src/base.hpp", "Base", "class_specifier", "class Base {};");
        let derived = test_snapshot("src/derived.hpp", "Derived", "class_specifier", "class Derived : public Base {};");
        let snapshots = vec![base.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, base.id());
        assert_eq!(edges[0].kind, EdgeKind::Extends);
    }

    #[test]
    fn cpp_base_naming_an_interface_kind_resolves_implements() {
        // C++ has no syntactic `interface` keyword, but if the resolved
        // base's own SymbolKind happens to be Interface (promoted from
        // some other construct this pipeline mapped to Interface), this
        // resolver still reports Implements for it — the SAME
        // kind-based disambiguation rule C# uses.
        let interface_like = test_snapshot("src/ifoo.hpp", "IFoo", "interface_declaration", "struct IFoo {};");
        let derived = test_snapshot("src/derived.hpp", "Derived", "class_specifier", "class Derived : public IFoo {};");
        let snapshots = vec![interface_like.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, interface_like.id());
        assert_eq!(edges[0].kind, EdgeKind::Implements);
    }

    #[test]
    fn a_base_not_found_in_the_pool_produces_no_edge() {
        let derived = test_snapshot("src/derived.cs", "Derived", "class_declaration", "class Derived : SomeUnknownBase {}");
        let snapshots = vec![derived];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert!(edges.is_empty(), "a base class not found among the given pool must never be guessed at, not even as Extends");
    }

    #[test]
    fn an_ambiguous_base_name_across_files_produces_no_edge() {
        let candidate_a = test_snapshot("src/a.cs", "Base", "class_declaration", "class Base {}");
        let candidate_b = test_snapshot("src/b.cs", "Base", "class_declaration", "class Base {}");
        let derived = test_snapshot("src/derived.cs", "Derived", "class_declaration", "class Derived : Base {}");
        let snapshots = vec![candidate_a, candidate_b, derived];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert!(edges.is_empty(), "an ambiguous same-named base across files must never be arbitrarily picked");
    }

    #[test]
    fn python_multiple_inheritance_produces_one_edge_per_base() {
        let base_a = test_snapshot("src/a.py", "A", "class_definition", "class A:\n    pass");
        let base_b = test_snapshot("src/b.py", "B", "class_definition", "class B:\n    pass");
        let derived = test_snapshot("src/c.py", "C", "class_definition", "class C(A, B):\n    pass");
        let snapshots = vec![base_a.clone(), base_b.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 2);
        let targets: Vec<_> = edges.iter().map(|e| e.to).collect();
        assert!(targets.contains(&base_a.id()));
        assert!(targets.contains(&base_b.id()));
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let snapshot = test_snapshot("src/lib.rs", "Foo", "struct_item", "struct Foo;");
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_extends_implements(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }
}
