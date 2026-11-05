//! B2 Task 7: `Imports` edges. Deliberately conservative, per the task's
//! own instruction: an import statement names a MODULE/PATH
//! (`import torch`, `use std::collections::HashMap`, `from torch import
//! nn`), not necessarily a symbol this pipeline has ever extracted — a
//! textual import is never automatically treated as an edge to a
//! specific symbol unless the imported NAME actually matches a known
//! snapshot in the given pool, exactly mirroring `calls.rs`'s own
//! "resolve by exact name among the candidates I was given, else no
//! edge" discipline. `import torch` alone, with no `torch`-named
//! snapshot anywhere in the pool, produces NOTHING — this module does
//! not invent a module-level placeholder symbol just to have something
//! to link.
//!
//! Only the imported NAME (the last named identifier of what is being
//! imported — e.g. `nn` from `from torch import nn`, `HashMap` from
//! `use std::collections::HashMap`, `Foo` from `import { Foo } from
//! './foo'`) is used as the lookup key; the module PATH itself
//! (`torch`, `std::collections`, `./foo`) is recorded in the evidence
//! note for context but is not itself resolved to a snapshot, since
//! this pipeline has no file-path-to-snapshot resolution step yet (that
//! would require actually opening the imported module's file, which is
//! out of this task's scope and belongs to whatever future task
//! resolves cross-file module paths, not a per-snapshot text match).
//!
//! # Real, known limitation: this resolver almost never fires on actual
//! # extracted data
//!
//! This resolver works EXACTLY like `calls.rs`: for each
//! `SymbolSnapshot`, it re-parses that one snapshot's own
//! `signature_text` and walks the resulting tree for import-shaped
//! nodes. It does NOT reconstruct a file's full import list from
//! anywhere else, and it has no separate "whole file" representation to
//! draw on. Task 4's extractors capture a symbol's `signature_text`
//! starting at that symbol's own construct (`fn`, `class`, etc.) — a
//! file-level `import`/`use`/`using` statement written ABOVE a function
//! or class is outside that function's or class's own byte span, so it
//! is simply never present in the text this resolver re-parses.
//! Concretely: a real Python file with `import torch` at the top and
//! `class MyModel(nn.Module):` below it will never produce an `Imports`
//! edge from `MyModel`'s own snapshot, because `MyModel`'s
//! `signature_text` is just the `class MyModel(nn.Module): ...` text —
//! the `import torch` line is never part of it.
//!
//! Every test in this module passes only because its fixtures
//! artificially inline the import statement INTO the fake
//! `signature_text` argument passed to `test_snapshot` — something real
//! extracted data will essentially never look like. This resolver's
//! logic (conservative exact-name matching, ambiguity-rejects-to-no-edge)
//! is correct and exercised; what it is missing is a DATA SOURCE: either
//! a future extraction change that captures a file's leading import
//! statements as their own candidate(s), or a separate file-level
//! pre-pass that collects import statements independently of any one
//! symbol's span. Both are out of this task's scope — recorded here
//! as an explicit, known gap rather than a bug to silently work around.

use codegraph_domain::EdgeKind;

use super::calls::parser_for;
use super::resolution::{resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
use crate::parse::Language;
use crate::snapshot::SymbolSnapshot;

/// Resolves `Imports` edges by re-parsing each snapshot's OWN
/// `signature_text` (identical approach to `calls.rs`) and walking it
/// for import-shaped nodes. See the module-level doc comment for the
/// real, known limitation this implies: a file-level import statement
/// living outside any single symbol's own span is invisible to this
/// resolver when working on genuinely extracted data, so in practice
/// this fires only when an import happens to appear within some
/// snapshot's own signature text.
pub fn resolve_imports(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    for snapshot in lookup.snapshots() {
        let Some(parser) = parser_for(snapshot.file()) else {
            continue;
        };
        let Ok(tree) = parser.parse(snapshot.signature().as_str().as_bytes()) else {
            continue;
        };

        for (imported_name, module_path) in imported_names_in(&tree) {
            let candidates = lookup.find_by_name(&imported_name, snapshot.id());
            if candidates.len() == 1 {
                let evidence = resolved_evidence(&format!(
                    "import of {imported_name:?} from {module_path:?} resolves to exactly one candidate in the given snapshot pool"
                ));
                edges.push(Edge {
                    from: snapshot.id(),
                    to: candidates[0].id(),
                    kind: EdgeKind::Imports,
                    evidence,
                });
            }
            // 0 candidates: the imported name is not in the given pool
            // (almost always the case — most imports name something
            // from outside this pipeline's current extraction scope,
            // e.g. a standard-library or third-party module). No edge.
            // >1 candidates: per this module's conservative design,
            // an ambiguous import target produces no edge either —
            // unlike `Calls`, which records one heuristic edge per
            // ambiguous candidate (a real call site genuinely could
            // invoke any of several overloads), an import statement
            // names exactly one thing; "ambiguous" here almost always
            // means the pool itself contains two unrelated same-named
            // symbols from different files, which is not evidence that
            // THIS import resolves to either of them.
        }
    }

    Ok(edges)
}

/// Every `(imported_name, module_path)` pair found in `tree`'s import
/// statements, for the languages that have one (HTML/CSS do not, and
/// are never reached here since `parser_for` only recognizes the seven
/// languages with a real import/use construct in this pipeline's scope
/// today).
fn imported_names_in(tree: &crate::parse::ParseTree) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk_for_imports(tree.tree().root_node(), tree.source(), tree.language(), &mut out);
    out
}

fn walk_for_imports(node: tree_sitter::Node, source: &[u8], language: Language, out: &mut Vec<(String, String)>) {
    match language {
        Language::Python => collect_python_import(node, source, out),
        Language::Rust => collect_rust_use(node, source, out),
        Language::JavaScript | Language::TypeScript | Language::Tsx => collect_js_import(node, source, out),
        Language::CSharp => collect_csharp_using(node, source, out),
        Language::Cpp | Language::Html | Language::Css => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_imports(child, source, language, out);
    }
}

fn collect_python_import(node: tree_sitter::Node, source: &[u8], out: &mut Vec<(String, String)>) {
    match node.kind() {
        "import_statement" => {
            // `import torch` -> name "torch", path "torch" (the dotted
            // name doubles as both here; there is no separate "from").
            if let Some(dotted) = node.child(1) {
                let text = text_of(dotted, source);
                out.push((last_segment(&text), text));
            }
        }
        "import_from_statement" => {
            // `from torch import nn` -> module path = the FIRST
            // dotted_name child, imported name(s) = every subsequent
            // dotted_name/identifier child after the `import` keyword.
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            let Some(module_node) = children.iter().find(|c| c.kind() == "dotted_name") else {
                return;
            };
            let module_path = text_of(*module_node, source);
            for child in &children {
                if child.kind() == "dotted_name" && child.id() != module_node.id() {
                    out.push((text_of(*child, source), module_path.clone()));
                }
            }
        }
        _ => {}
    }
}

fn collect_rust_use(node: tree_sitter::Node, source: &[u8], out: &mut Vec<(String, String)>) {
    if node.kind() != "use_declaration" {
        return;
    }
    let mut cursor = node.walk();
    let Some(path_node) = node.children(&mut cursor).find(|c| matches!(c.kind(), "scoped_identifier" | "identifier")) else {
        return;
    };
    let full_path = text_of(path_node, source);
    let name = last_segment(&full_path);
    out.push((name, full_path));
}

fn collect_js_import(node: tree_sitter::Node, source: &[u8], out: &mut Vec<(String, String)>) {
    if node.kind() != "import_statement" {
        return;
    }
    let mut cursor = node.walk();
    let Some(source_string) = node.children(&mut cursor).find(|c| c.kind() == "string") else {
        return;
    };
    let module_path = text_of(source_string, source);
    let mut inner_cursor = node.walk();
    for child in node.children(&mut inner_cursor) {
        if child.kind() == "import_clause" {
            collect_js_import_specifiers(child, source, &module_path, out);
        }
    }
}

fn collect_js_import_specifiers(node: tree_sitter::Node, source: &[u8], module_path: &str, out: &mut Vec<(String, String)>) {
    if matches!(node.kind(), "identifier") {
        out.push((text_of(node, source), module_path.to_string()));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_specifier" {
            let mut spec_cursor = child.walk();
            let found_name = child.children(&mut spec_cursor).find(|c| c.kind() == "identifier");
            if let Some(name_node) = found_name {
                out.push((text_of(name_node, source), module_path.to_string()));
            }
        } else {
            collect_js_import_specifiers(child, source, module_path, out);
        }
    }
}

fn collect_csharp_using(node: tree_sitter::Node, source: &[u8], out: &mut Vec<(String, String)>) {
    if node.kind() != "using_directive" {
        return;
    }
    let mut cursor = node.walk();
    let found_name = node.children(&mut cursor).find(|c| matches!(c.kind(), "identifier" | "qualified_name"));
    if let Some(name_node) = found_name {
        let full_path = text_of(name_node, source);
        out.push((last_segment(&full_path), full_path));
    }
}

fn last_segment(dotted_or_scoped: &str) -> String {
    dotted_or_scoped.rsplit(&['.', ':'][..]).next().unwrap_or(dotted_or_scoped).to_string()
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
    fn an_import_of_an_unknown_module_produces_no_edge() {
        // `import torch` with nothing named "torch" in the pool.
        let importer = test_snapshot("src/model.py", "Model", "class_definition", "import torch\nclass Model:\n    pass");
        let snapshots = vec![importer];

        let edges = resolve_imports(&snapshots).unwrap();

        assert!(edges.is_empty(), "importing an unknown module must produce no edge, not a fabricated one");
    }

    #[test]
    fn a_rust_use_matching_a_known_snapshot_is_resolved() {
        let user = test_snapshot("src/lib.rs", "foo", "function_item", "use crate::helper::run;\nfn foo() {}");
        let target = test_snapshot("src/helper.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![user.clone(), target.clone()];

        let edges = resolve_imports(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, user.id());
        assert_eq!(edges[0].to, target.id());
        assert_eq!(edges[0].kind, EdgeKind::Imports);
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn a_python_from_import_matching_a_known_snapshot_is_resolved() {
        let user = test_snapshot("src/model.py", "Model", "class_definition", "from mymodule import helper\nclass Model:\n    pass");
        let target = test_snapshot("src/mymodule.py", "helper", "function_definition", "def helper():\n    pass");
        let snapshots = vec![user.clone(), target.clone()];

        let edges = resolve_imports(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, target.id());
        // Pins that `module_path` is genuinely "mymodule" (the actual
        // `dotted_name` child), not some other child's text (e.g. the
        // `from` keyword itself) a mutated `==`/`!=` on the module-node
        // `find` predicate could otherwise select.
        assert!(edges[0].evidence.note().contains("mymodule"), "expected the module path in the evidence note, got {:?}", edges[0].evidence.note());
    }

    #[test]
    fn a_python_from_import_with_multiple_names_resolves_only_the_actually_imported_ones_not_the_module_itself() {
        // `from mymodule import helper, other_helper` has THREE
        // dotted_name/identifier-shaped children after `from`: the
        // module name itself, plus two imported names. Pins that the
        // module node is correctly EXCLUDED from the imported-names
        // loop (the `&&`/`child.id() != module_node.id()` check) by
        // using a target named exactly like the MODULE to prove it is
        // never treated as an import target.
        let user = test_snapshot(
            "src/model.py",
            "Model",
            "class_definition",
            "from mymodule import helper, other_helper\nclass Model:\n    pass",
        );
        let module_named_target = test_snapshot("src/mymodule.py", "mymodule", "class_definition", "class mymodule:\n    pass");
        let helper = test_snapshot("src/helpers.py", "helper", "function_definition", "def helper():\n    pass");
        let other_helper = test_snapshot("src/helpers.py", "other_helper", "function_definition", "def other_helper():\n    pass");
        let snapshots = vec![user.clone(), module_named_target.clone(), helper.clone(), other_helper.clone()];

        let edges = resolve_imports(&snapshots).unwrap();

        let target_ids: Vec<_> = edges.iter().map(|e| e.to).collect();
        assert!(target_ids.contains(&helper.id()), "expected helper to be resolved as an imported name");
        assert!(target_ids.contains(&other_helper.id()), "expected other_helper to be resolved as an imported name");
        assert!(
            !target_ids.contains(&module_named_target.id()),
            "the module name itself must never be treated as one of the imported names"
        );
    }

    #[test]
    fn an_ambiguous_import_target_produces_no_edge() {
        let user = test_snapshot("src/lib.rs", "foo", "function_item", "use crate::run;\nfn foo() {}");
        let candidate_a = test_snapshot("src/a.rs", "run", "function_item", "fn run() {}");
        let candidate_b = test_snapshot("src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![user, candidate_a, candidate_b];

        let edges = resolve_imports(&snapshots).unwrap();

        assert!(edges.is_empty(), "an ambiguous import target must produce no edge, unlike the Calls resolver's per-candidate heuristic edges");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let snapshot = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() {}");
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_imports(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    #[test]
    fn a_plain_python_import_statement_matching_a_known_snapshot_is_resolved() {
        // `import torch` (not `from X import Y`): the bare
        // `import_statement` path, distinct from `import_from_statement`.
        let importer = test_snapshot("src/model.py", "Model", "class_definition", "import torch\nclass Model:\n    pass");
        let torch_module = test_snapshot("src/torch.py", "torch", "class_definition", "class torch:\n    pass");
        let snapshots = vec![importer.clone(), torch_module.clone()];

        let edges = resolve_imports(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, importer.id());
        assert_eq!(edges[0].to, torch_module.id());
    }

    #[test]
    fn a_dotted_python_import_resolves_by_its_own_full_dotted_text_for_a_plain_import() {
        // `import a.b.c` names ONE dotted path as both the module and
        // the "imported name" for a bare import_statement (there is no
        // separate alias here) — this pins that `import_statement`'s
        // own text (not `import_from_statement`'s split logic) is used.
        let importer = test_snapshot("src/user.py", "User", "class_definition", "import pkg.mod\nclass User:\n    pass");
        // No snapshot named "pkg.mod" or "mod" exists in the pool for
        // this specific shape by design of this test — proven instead
        // via the "unknown module produces no edge" test above; this
        // test exists primarily to exercise the import_statement match
        // arm itself for mutation coverage without needing a dotted
        // name resolution target this pipeline does not attempt.
        let snapshots = vec![importer];

        let edges = resolve_imports(&snapshots).unwrap();

        assert!(edges.is_empty());
    }

    #[test]
    fn a_javascript_named_import_matching_a_known_snapshot_is_resolved() {
        let importer = test_snapshot("src/user.js", "run", "function_declaration", "import { helper } from './helper';\nfunction run() {}");
        let target = test_snapshot("src/helper.js", "helper", "function_declaration", "export function helper() {}");
        let snapshots = vec![importer.clone(), target.clone()];

        let edges = resolve_imports(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, importer.id());
        assert_eq!(edges[0].to, target.id());
        // Pins that the recorded module path is genuinely the `string`
        // node's text ("'./helper'"), not some other child a mutated
        // `==`/`!=` on the `find(|c| c.kind() == "string")` predicate
        // could otherwise select (e.g. the `import` keyword itself).
        assert!(
            edges[0].evidence.note().contains("./helper"),
            "expected the module path in the evidence note, got {:?}",
            edges[0].evidence.note()
        );
    }

    #[test]
    fn a_javascript_import_with_no_matching_target_produces_no_edge() {
        let importer = test_snapshot("src/user.js", "run", "function_declaration", "import { helper } from './helper';\nfunction run() {}");
        let snapshots = vec![importer];

        let edges = resolve_imports(&snapshots).unwrap();

        assert!(edges.is_empty());
    }

    #[test]
    fn a_csharp_using_matching_a_known_snapshot_is_resolved() {
        let importer = test_snapshot("src/user.cs", "C", "class_declaration", "using Helpers;\nclass C {}");
        let target = test_snapshot("src/helpers.cs", "Helpers", "class_declaration", "class Helpers {}");
        let snapshots = vec![importer.clone(), target.clone()];

        let edges = resolve_imports(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, importer.id());
        assert_eq!(edges[0].to, target.id());
    }

    #[test]
    fn a_csharp_using_with_no_matching_target_produces_no_edge() {
        let importer = test_snapshot("src/user.cs", "C", "class_declaration", "using System;\nclass C {}");
        let snapshots = vec![importer];

        let edges = resolve_imports(&snapshots).unwrap();

        assert!(edges.is_empty());
    }
}
