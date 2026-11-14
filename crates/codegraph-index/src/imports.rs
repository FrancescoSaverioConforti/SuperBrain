//! B2 Task 9.5, Priority 6: file-level import facts — a REPLACEMENT for
//! Task 7's original `EdgeKind::Imports`/`resolve_imports` (removed; see
//! `docs/research/findings/B2-ID-005.md` and `edges::mod`'s own doc
//! comment), not an addition alongside it.
//!
//! # Why a separate model, not an `Edge`
//!
//! A real import statement (`import torch`, `from foo.bar import Baz`,
//! `use crate::helper::run;`, `using System;`) is a FILE/module-level
//! fact: it is written once, above (or alongside) every symbol in that
//! file, and it is not more "true" of one symbol in that file than of
//! any other. `Edge { from: SymbolSnapshotId, .. }` has no honest value
//! to put in `from` for a fact that belongs to the FILE, not to any one
//! symbol — every option considered (emit from every symbol in the
//! file; emit from one arbitrarily chosen "representative" symbol;
//! invent a synthetic `Module`/`File` snapshot right now) either
//! fabricates a false per-symbol dependency, picks an arbitrary and
//! unmotivated "representative," or prematurely decides the still-open
//! `Module`/`File` structural-node question this task's Priority 2 already
//! deferred. [`ImportFact`] instead keys directly off [`FileIdentity`],
//! with no `SymbolSnapshotId` anywhere in its own shape, and is NOT
//! inserted into `CodeGraph` (no persistence, no API, no MCP) — it is a
//! separate in-memory structure a caller queries directly.
//!
//! # No invented internal targets
//!
//! When an import's resolution status is anything other than
//! [`ImportResolution::Internal`], this module NEVER invents a
//! `SymbolSnapshotId` to point at — exactly the same "no edge beats a
//! false edge" discipline `edges::resolution` already applies. A
//! standard-library or third-party import (the overwhelming majority of
//! real imports, confirmed empirically: 0 of 589 real import statements
//! in a real 97-file corpus named anything this pipeline had indexed)
//! is recorded as [`ImportResolution::External`], never silently
//! dropped and never pointed at a fabricated placeholder.
//!
//! # Deliberately NOT yet a symbol-dependency claim
//!
//! `ImportFact` records what a FILE imports. It deliberately does NOT
//! claim that every symbol in that file "depends on" or "uses" that
//! import — that would require actual symbol-level reference/data-flow
//! analysis this pipeline does not do (confirmed: a file-level import
//! is routinely unused by many of that file's own symbols in real
//! code). A future resolver built on top of real symbol references
//! could derive that narrower claim; this module does not attempt it.

use codegraph_domain::FileIdentity;
use thiserror::Error;

use crate::extract::ByteSpan;
use crate::parse::Language;
use crate::snapshot::SymbolSnapshot;

/// One import statement's extracted, resolved fact. `module_path` is
/// always present (the dotted/scoped/quoted path as written in source);
/// `imported_symbol` is `None` for a bare module import with nothing
/// named out of it (`import torch`, `use std::io` with no further
/// segment beyond the module path's own last segment — see each
/// language's own extraction function for the exact distinction),
/// `Some` for a `from`/named-import form (`from torch import nn` ->
/// `Some("nn")`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFact {
    pub importing_file: FileIdentity,
    pub module_path: String,
    pub imported_symbol: Option<String>,
    pub alias: Option<String>,
    pub span: ByteSpan,
    pub resolution: ImportResolution,
}

/// How an [`ImportFact`] resolves. `Internal` is the only variant that
/// carries a real `SymbolSnapshotId` — every other variant deliberately
/// carries none, per this module's own "no invented internal targets"
/// rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportResolution {
    /// The imported name resolves to EXACTLY ONE `SymbolSnapshot` in
    /// the given pool.
    Internal(codegraph_domain::SymbolSnapshotId),
    /// The imported name does not match any snapshot in the given
    /// pool — the overwhelmingly common real case (standard library,
    /// third-party dependency, or simply a file/module this indexing
    /// pass was not given). This is a DISTINCT outcome from
    /// `Unresolved`, per the Evidence-before-claims Experimental
    /// Policy's "unresolved: no candidate" vs. "unresolved: ambiguity
    /// remains" distinction — `External` specifically means "zero
    /// candidates," recorded as its own variant rather than merged into
    /// a single catch-all "not resolved."
    External,
    /// The imported name matches MORE THAN ONE snapshot in the given
    /// pool — ambiguous, per this module's own "no edge beats a false
    /// edge" discipline, never resolved to an arbitrary pick.
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ImportFactError {
    #[error("duplicate SymbolSnapshotId in the candidate pool: a caller must never include the same snapshot twice")]
    DuplicateSnapshotId,
}

/// Extracts every import statement found anywhere in `tree` (the WHOLE
/// file's own parse tree — NOT one symbol's `signature_text`, which is
/// exactly the granularity mismatch `B2-ID-005` found and removed) as
/// raw `(module_path, imported_symbol, alias, span)` tuples, independent
/// of any resolution step. Pure syntax extraction: this function alone
/// never resolves a name against a snapshot pool, so it is usable both
/// by [`resolve_import_facts`] (which does resolve) and by a future
/// caller that only wants the raw facts (e.g. an "aliases defined in
/// this file" query).
pub fn extract_import_statements(tree: &crate::parse::ParseTree) -> Vec<RawImportStatement> {
    let mut out = Vec::new();
    walk_for_imports(tree.tree().root_node(), tree.source(), tree.language(), &mut out);
    out
}

/// One syntactically-extracted import statement, before any resolution
/// against a snapshot pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImportStatement {
    pub module_path: String,
    pub imported_symbol: Option<String>,
    pub alias: Option<String>,
    pub span: ByteSpan,
}

/// Resolves every import statement extracted from `importing_file`'s
/// own full-file `tree` against `snapshots` — the SAME snapshot pool
/// shape every `edges` resolver already takes, reused here for
/// consistency even though `ImportFact` is not an `Edge`. The lookup key
/// is the imported NAME when one exists (`from X import Y` -> `Y`), or
/// the module path's own last segment for a bare import (`import torch`
/// -> `torch`) — mirroring the ORIGINAL `resolve_imports`'s own matching
/// convention, which this replacement keeps (that part of the design was
/// never the defect; the SYMBOL-TO-SYMBOL granularity was).
pub fn resolve_import_facts(importing_file: &FileIdentity, tree: &crate::parse::ParseTree, snapshots: &[SymbolSnapshot]) -> Result<Vec<ImportFact>, ImportFactError> {
    let raw_statements: Vec<(FileIdentity, RawImportStatement)> = extract_import_statements(tree).into_iter().map(|s| (importing_file.clone(), s)).collect();
    resolve_raw_import_statements(&raw_statements, snapshots)
}

/// The GLOBAL semantic resolution phase (Task 9.5, Priority 6): takes
/// already-extracted, already-paired `(importing_file, RawImportStatement)`
/// facts — produced independently, and possibly incrementally, by
/// `incremental::pipeline::index_files` for exactly the files it parsed
/// — and resolves each one against `snapshots`, the FULL current
/// repository-wide pool (same requirement `edges::resolve_all_typed_edges`
/// already has). Deliberately separate from raw extraction: extraction
/// is a per-file, local, syntax-only operation that can be scoped to
/// just the changed/new files on an incremental update; resolution
/// depends on symbols that may live in ANY file, including ones never
/// touched by this update, so it must always run against the complete
/// pool, never a partial one — see this module's own doc comment.
pub fn resolve_raw_import_statements(raw_statements: &[(FileIdentity, RawImportStatement)], snapshots: &[SymbolSnapshot]) -> Result<Vec<ImportFact>, ImportFactError> {
    reject_duplicate_ids(snapshots)?;

    let mut facts = Vec::with_capacity(raw_statements.len());
    for (importing_file, statement) in raw_statements {
        let lookup_name = statement.imported_symbol.clone().unwrap_or_else(|| last_segment(&statement.module_path));
        let candidates: Vec<&SymbolSnapshot> = snapshots.iter().filter(|s| s.name().as_str() == lookup_name).collect();
        let resolution = match candidates.len() {
            1 => ImportResolution::Internal(candidates[0].id()),
            0 => ImportResolution::External,
            _ => ImportResolution::Ambiguous,
        };
        facts.push(ImportFact {
            importing_file: importing_file.clone(),
            module_path: statement.module_path.clone(),
            imported_symbol: statement.imported_symbol.clone(),
            alias: statement.alias.clone(),
            span: statement.span,
            resolution,
        });
    }

    Ok(facts)
}

/// `pub(crate)` so [`crate::impls`]'s own `ImplFactError::DuplicateSnapshotId`
/// check can reuse this exact rejection logic rather than maintaining a
/// second, separately-drifting duplicate-detection function — the two
/// modules' error enums are structurally identical for the same reason
/// (`ImplFact`/`ImportFact` both resolve against the same kind of
/// `&[SymbolSnapshot]` candidate pool), so one shared implementation is
/// the honest choice over a copy that could silently diverge.
pub(crate) fn reject_duplicate_ids(snapshots: &[SymbolSnapshot]) -> Result<(), ImportFactError> {
    let mut seen = std::collections::HashSet::with_capacity(snapshots.len());
    for snapshot in snapshots {
        if !seen.insert(snapshot.id()) {
            return Err(ImportFactError::DuplicateSnapshotId);
        }
    }
    Ok(())
}

fn walk_for_imports(node: tree_sitter::Node, source: &[u8], language: Language, out: &mut Vec<RawImportStatement>) {
    match language {
        Language::Python => collect_python_import(node, source, out),
        Language::Rust => collect_rust_use(node, source, out),
        Language::JavaScript | Language::TypeScript | Language::Tsx => {
            collect_js_import(node, source, out);
            collect_js_require(node, source, out);
        }
        Language::CSharp => collect_csharp_using(node, source, out),
        Language::Cpp | Language::Html | Language::Css => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_imports(child, source, language, out);
    }
}

fn collect_python_import(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawImportStatement>) {
    match node.kind() {
        "import_statement" => {
            // `import torch` -> module_path "torch", no separate
            // imported_symbol. `import torch as t` -> alias "t" (a
            // `aliased_import` child wraps the dotted name and the `as`
            // target).
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dotted_name" => out.push(RawImportStatement {
                        module_path: text_of(child, source),
                        imported_symbol: None,
                        alias: None,
                        span: ByteSpan::from_node(&node),
                    }),
                    "aliased_import" => {
                        let mut inner_cursor = child.walk();
                        let inner_children: Vec<_> = child.children(&mut inner_cursor).collect();
                        let module = inner_children.iter().find(|c| c.kind() == "dotted_name").map(|c| text_of(*c, source));
                        let alias_name = inner_children.iter().rev().find(|c| c.kind() == "identifier").map(|c| text_of(*c, source));
                        if let Some(module_path) = module {
                            out.push(RawImportStatement {
                                module_path,
                                imported_symbol: None,
                                alias: alias_name,
                                span: ByteSpan::from_node(&node),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        "import_from_statement" => {
            // `from torch import nn` -> module_path "torch", one fact
            // with imported_symbol "nn". `from torch import nn as N` ->
            // alias "N". `from torch import nn, functional` -> two
            // separate facts, one per imported name.
            //
            // CALL-RES-A4 (follow-on fix): a RELATIVE import (`from
            // .ml import X`, `from ..pkg.mod import Y`) wraps its own
            // module path in a `relative_import` node — `import_prefix`
            // (the leading dot(s)) followed by an OPTIONAL `dotted_name`
            // (absent for a bare `from . import X`, confirmed via
            // direct grammar probe). CRITICALLY, confirmed via the SAME
            // probe: whenever a `relative_import` node is present, the
            // statement's OWN imported name(s) are ALSO direct
            // `dotted_name` children of `import_from_statement` itself
            // (`from .ml import helper` -> a direct `dotted_name`
            // "helper" sibling of `relative_import`) — the exact same
            // shape an ABSOLUTE import's own module-path `dotted_name`
            // has. A naive "first direct `dotted_name` child wins" check
            // (this module's OWN earlier, incorrect fix attempt,
            // confirmed wrong by this exact test suite before this
            // comment was written) would misidentify the IMPORTED NAME
            // as the module path whenever a relative import also names
            // something — the fix is to check for `relative_import`
            // FIRST and unconditionally prefer it as the module-path
            // source whenever it exists at all, falling back to a
            // direct `dotted_name` child ONLY for a genuine absolute
            // import (no `relative_import` node present). Before THIS
            // fix, a relative import's own `module_node` search found
            // NOTHING (no direct `dotted_name` child existed for the
            // module itself in that shape) and the WHOLE statement
            // silently produced zero facts — the exact real-repository
            // shape CRC-003 documents (`from .ml import
            // get_features_targets as _get_features_targets`).
            // `module_path` for a relative import is the inner
            // `dotted_name`'s own text when present (`"ml"`), or the
            // dot prefix alone (`"."`/`".."`) for a bare `from . import
            // X`/`from .. import Y` — never fabricated, always exactly
            // what the real `import_prefix` text reads, so a caller can
            // still tell "this was a relative import" from the leading
            // dot(s) in `module_path` even though this module does not
            // yet resolve a relative path against the importing file's
            // own package location (a separate, NOT-yet-implemented
            // feature — this fix only makes the STATEMENT extractable
            // at all, never claims to resolve `.` relative to anything).
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            let relative_import_node = children.iter().find(|c| c.kind() == "relative_import").copied();
            let module_node = if let Some(relative) = relative_import_node {
                let mut rel_cursor = relative.walk();
                let found = relative.children(&mut rel_cursor).find(|c| c.kind() == "dotted_name");
                found
            } else {
                children.iter().find(|c| c.kind() == "dotted_name").copied()
            };
            let module_path = match module_node {
                Some(node) => text_of(node, source),
                None => {
                    // A bare `from . import X`/`from .. import Y` (no
                    // module name after the dots at all) — fall back to
                    // the `relative_import` node's own full text (the
                    // dot prefix alone), never silently dropping the
                    // statement the way the pre-fix code did.
                    let Some(relative) = relative_import_node else {
                        return;
                    };
                    text_of(relative, source)
                }
            };
            let module_node_id = module_node.map(|n| n.id());
            for child in &children {
                match child.kind() {
                    "dotted_name" if Some(child.id()) != module_node_id => {
                        out.push(RawImportStatement {
                            module_path: module_path.clone(),
                            imported_symbol: Some(text_of(*child, source)),
                            alias: None,
                            span: ByteSpan::from_node(&node),
                        });
                    }
                    "aliased_import" => {
                        let mut inner_cursor = child.walk();
                        let inner_children: Vec<_> = child.children(&mut inner_cursor).collect();
                        let imported = inner_children.first().map(|c| text_of(*c, source));
                        let alias_name = inner_children.iter().rev().find(|c| c.kind() == "identifier").map(|c| text_of(*c, source));
                        if let Some(imported_symbol) = imported {
                            out.push(RawImportStatement {
                                module_path: module_path.clone(),
                                imported_symbol: Some(imported_symbol),
                                alias: alias_name,
                                span: ByteSpan::from_node(&node),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn collect_rust_use(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawImportStatement>) {
    if node.kind() != "use_declaration" {
        return;
    }
    let mut cursor = node.walk();
    // `use crate::helper::run;` -> scoped_identifier "crate::helper::run"
    // `use crate::helper::run as r;` -> use_as_clause wrapping the path + alias.
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" | "identifier" => {
                let full_path = text_of(child, source);
                out.push(RawImportStatement {
                    module_path: full_path,
                    imported_symbol: None,
                    alias: None,
                    span: ByteSpan::from_node(&node),
                });
            }
            "use_as_clause" => {
                let mut inner_cursor = child.walk();
                let inner_children: Vec<_> = child.children(&mut inner_cursor).collect();
                let path = inner_children.iter().find(|c| matches!(c.kind(), "scoped_identifier" | "identifier")).map(|c| text_of(*c, source));
                let alias_name = inner_children.iter().rev().find(|c| c.kind() == "identifier").map(|c| text_of(*c, source));
                if let Some(module_path) = path {
                    out.push(RawImportStatement {
                        module_path,
                        imported_symbol: None,
                        alias: alias_name,
                        span: ByteSpan::from_node(&node),
                    });
                }
            }
            _ => {}
        }
    }
}

fn collect_js_import(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawImportStatement>) {
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

fn collect_js_import_specifiers(node: tree_sitter::Node, source: &[u8], module_path: &str, out: &mut Vec<RawImportStatement>) {
    if node.kind() == "identifier" {
        // Default import: `import Foo from './foo'`.
        out.push(RawImportStatement {
            module_path: module_path.to_string(),
            imported_symbol: None,
            alias: Some(text_of(node, source)),
            span: ByteSpan::from_node(&node),
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_specifier" {
            let mut spec_cursor = child.walk();
            let spec_children: Vec<_> = child.children(&mut spec_cursor).filter(|c| c.kind() == "identifier").collect();
            // `import { Foo } from ...` -> one identifier (the name).
            // `import { Foo as Bar } from ...` -> two identifiers: the
            // real name first, the alias second.
            if let Some(name_node) = spec_children.first() {
                let alias_name = if spec_children.len() > 1 { Some(text_of(spec_children[1], source)) } else { None };
                out.push(RawImportStatement {
                    module_path: module_path.to_string(),
                    imported_symbol: Some(text_of(*name_node, source)),
                    alias: alias_name,
                    span: ByteSpan::from_node(&child),
                });
            }
        } else {
            collect_js_import_specifiers(child, source, module_path, out);
        }
    }
}

/// CommonJS Extraction Checkpoint (B2-ID-013): `require(...)` call sites
/// never produced an `ImportFact` — `collect_js_import` only matches
/// ESM `import_statement`. This collector recognizes the real shapes
/// confirmed by direct parse inspection (`const x = require('./x')`,
/// `const { a, b } = require('./x')`, `var y = require('./mod')`), all
/// of which share the same AST skeleton: a `call_expression` whose
/// callee is the bare `identifier` `"require"` and whose sole argument
/// is a `string`, as the right-hand side of a `variable_declarator`.
///
/// This resolver is intentionally narrow: it recognizes `require(...)`
/// ONLY when it is the direct right-hand side of a `variable_declarator`
/// (binding the module to a name or a destructuring pattern) — a bare
/// `require('./x');` with no assignment, or `require(...)` nested
/// inside some other expression (e.g. `foo(require('./x'))`), produces
/// no fact here, per this checkpoint's own explicit scope (no general
/// call-expression analysis, no data-flow). This mirrors every other
/// import collector in this module: syntax recognition only, no
/// resolution (resolution happens later, in [`resolve_raw_import_statements`]).
fn collect_js_require(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawImportStatement>) {
    if node.kind() != "variable_declarator" {
        return;
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    let Some(lhs) = children.first() else { return };
    let Some(call) = children.iter().find(|c| c.kind() == "call_expression") else {
        return;
    };
    let Some(module_path) = require_call_module_path(*call, source) else {
        return;
    };
    let span = ByteSpan::from_node(&node);

    match lhs.kind() {
        // `const x = require('./x')` / `var y = require('./mod')` ->
        // one fact, the bound name recorded as the alias (mirroring
        // `collect_js_import_specifiers`'s own default-import
        // convention: a bare bound name with no further named
        // destructuring is recorded as `alias`, not `imported_symbol`,
        // since nothing is named OUT of the module here distinct from
        // the module itself).
        "identifier" => {
            out.push(RawImportStatement {
                module_path,
                imported_symbol: None,
                alias: Some(text_of(*lhs, source)),
                span,
            });
        }
        // `const { a, b } = require('./x')` -> one fact per destructured
        // name, mirroring Python's own `from X import a, b` convention
        // (one fact per name, never one fact for the whole pattern).
        "object_pattern" => {
            let mut pattern_cursor = lhs.walk();
            for child in lhs.children(&mut pattern_cursor) {
                if child.kind() == "shorthand_property_identifier_pattern" {
                    out.push(RawImportStatement {
                        module_path: module_path.clone(),
                        imported_symbol: Some(text_of(child, source)),
                        alias: None,
                        span,
                    });
                }
            }
        }
        _ => {}
    }
}

/// `true`/`Some(module_path)` only when `call` is syntactically exactly
/// `require(<string>)` — callee is the bare `identifier` `"require"`
/// (never a member expression like `foo.require(...)`, which is a
/// different, unrelated call), and the arguments list contains exactly
/// one `string` argument. Confirmed against a real parse: the callee
/// and the `arguments` node are both direct children of `call_expression`.
fn require_call_module_path(call: tree_sitter::Node, source: &[u8]) -> Option<String> {
    if call.kind() != "call_expression" {
        return None;
    }
    let mut cursor = call.walk();
    let children: Vec<_> = call.children(&mut cursor).collect();
    let callee = children.iter().find(|c| c.kind() == "identifier")?;
    if text_of(*callee, source) != "require" {
        return None;
    }
    let arguments = children.iter().find(|c| c.kind() == "arguments")?;
    let mut args_cursor = arguments.walk();
    let string_args: Vec<_> = arguments.children(&mut args_cursor).filter(|c| c.kind() == "string").collect();
    if string_args.len() != 1 {
        return None;
    }
    Some(text_of(string_args[0], source))
}

fn collect_csharp_using(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawImportStatement>) {
    if node.kind() != "using_directive" {
        return;
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    // `using System;` -> ONE name node (identifier/qualified_name), no
    // `=` child at all. `using Foo = System.Bar;` -> TWO name-shaped
    // nodes with a literal `=` token between them: the alias
    // (`identifier "Foo"`, always first) and the real target
    // (`qualified_name`/`identifier`, always last) — confirmed via a
    // real `tree-sitter-c-sharp` parse dump (Task 9.5, Priority 6);
    // there is NO wrapping `name_equals` node in this grammar. The same
    // holds for `global using ...` and `using static ...` (also
    // confirmed via real dumps): neither form ever produces 2+
    // name-shaped children without an `=`, nor an `=` with fewer than
    // 2. `has_alias` and `name_nodes.len() >= 2` are therefore
    // PERFECTLY CORRELATED across every grammar-producible
    // `using_directive` shape — there is no real C# syntax where one
    // holds and the other does not. (Mutation testing, Task 9.5's
    // closing gate: flipping `==`/`&&` here to `!=`/`||` is an
    // EQUIVALENT mutant under this grammar constraint, not a missed
    // test — no input tree-sitter-c-sharp can produce distinguishes
    // the two. Do not "simplify" this guard assuming it is redundant;
    // the redundancy is a property of the grammar, not of this code,
    // and a future grammar version could in principle decorrelate
    // them.)
    let has_alias = children.iter().any(|c| c.kind() == "=");
    let name_nodes: Vec<tree_sitter::Node> = children.iter().copied().filter(|c| matches!(c.kind(), "identifier" | "qualified_name")).collect();
    let (alias_name, target) = if has_alias && name_nodes.len() >= 2 {
        (Some(text_of(name_nodes[0], source)), name_nodes.last().copied())
    } else {
        (None, name_nodes.last().copied())
    };
    if let Some(name_node) = target {
        out.push(RawImportStatement {
            module_path: text_of(name_node, source),
            imported_symbol: None,
            alias: alias_name,
            span: ByteSpan::from_node(&node),
        });
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
    use codegraph_domain::{CommitHash, RepositoryRegistry, RepositoryRoot};

    use crate::extract::RawSymbolCandidate;
    use crate::parse::LanguageParser;
    use crate::snapshot::promote;

    fn test_file(repo: codegraph_domain::RepositoryId, path: &str) -> FileIdentity {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        FileIdentity::parse(repo, commit, path).unwrap()
    }

    fn test_repo() -> codegraph_domain::RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    fn test_snapshot(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let file = test_file(repo, path);
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    fn parse_python(source: &str) -> crate::parse::ParseTree {
        crate::parse::PythonParser.parse(source.as_bytes()).unwrap()
    }

    fn parse_rust(source: &str) -> crate::parse::ParseTree {
        crate::parse::RustParser.parse(source.as_bytes()).unwrap()
    }

    fn parse_js(source: &str) -> crate::parse::ParseTree {
        crate::parse::JavaScriptParser.parse(source.as_bytes()).unwrap()
    }

    fn parse_csharp(source: &str) -> crate::parse::ParseTree {
        crate::parse::CSharpParser.parse(source.as_bytes()).unwrap()
    }


    // --- Raw extraction (syntax only) ---

    // --- CALL-RES-A4 follow-on fix: relative import extraction,
    // previously silently dropped entirely ---

    #[test]
    fn python_relative_import_with_module_and_alias_is_extracted() {
        // The exact real CRC-003 shape: before this fix, this entire
        // statement produced ZERO facts (the pre-fix `module_node`
        // search only recognized a DIRECT `dotted_name` child, never
        // the `relative_import` wrapper a relative import actually
        // uses).
        let tree = parse_python("from .ml import get_features_targets as _get_features_targets\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1, "a relative import must no longer be silently dropped");
        assert_eq!(facts[0].module_path, "ml");
        assert_eq!(facts[0].imported_symbol, Some("get_features_targets".to_string()));
        assert_eq!(facts[0].alias, Some("_get_features_targets".to_string()));
    }

    #[test]
    fn python_relative_import_with_multiple_dots_is_extracted() {
        let tree = parse_python("from ..pkg.sub import helper\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "pkg.sub");
        assert_eq!(facts[0].imported_symbol, Some("helper".to_string()));
    }

    #[test]
    fn python_relative_import_with_no_module_name_falls_back_to_the_dot_prefix() {
        // `from . import X` -- the relative_import node carries ONLY
        // the dot prefix, no inner dotted_name at all. Must still
        // extract a fact (never silently dropped), with module_path
        // honestly reflecting the dot prefix itself, never a guessed
        // real module name.
        let tree = parse_python("from . import helper\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, ".");
        assert_eq!(facts[0].imported_symbol, Some("helper".to_string()));
    }

    #[test]
    fn python_relative_import_with_multiple_names_produces_one_fact_per_name() {
        let tree = parse_python("from .ml import a, b\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 2, "a relative import with multiple named imports must still produce one fact per name, same as an absolute import");
        let names: Vec<_> = facts.iter().filter_map(|f| f.imported_symbol.clone()).collect();
        assert!(names.contains(&"a".to_string()));
        assert!(names.contains(&"b".to_string()));
    }

    #[test]
    fn python_bare_import_is_extracted_with_no_imported_symbol() {
        let tree = parse_python("import torch\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "torch");
        assert_eq!(facts[0].imported_symbol, None);
        assert_eq!(facts[0].alias, None);
    }

    #[test]
    fn python_bare_import_with_alias_is_extracted() {
        let tree = parse_python("import torch as t\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "torch");
        assert_eq!(facts[0].alias, Some("t".to_string()));
    }

    #[test]
    fn python_from_import_is_extracted_with_module_and_symbol_separated() {
        let tree = parse_python("from foo.bar import Baz\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "foo.bar");
        assert_eq!(facts[0].imported_symbol, Some("Baz".to_string()));
        assert_eq!(facts[0].alias, None);
    }

    #[test]
    fn python_from_import_with_alias_is_extracted() {
        let tree = parse_python("from foo import bar as baz\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "foo");
        assert_eq!(facts[0].imported_symbol, Some("bar".to_string()));
        assert_eq!(facts[0].alias, Some("baz".to_string()));
    }

    #[test]
    fn python_from_import_with_multiple_names_produces_one_fact_per_name() {
        let tree = parse_python("from foo import a, b, c\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 3);
        let names: Vec<_> = facts.iter().filter_map(|f| f.imported_symbol.clone()).collect();
        assert!(names.contains(&"a".to_string()));
        assert!(names.contains(&"b".to_string()));
        assert!(names.contains(&"c".to_string()));
    }

    #[test]
    fn multiple_imports_in_one_file_each_produce_their_own_fact() {
        let tree = parse_python("import torch\nimport numpy\nfrom foo import bar\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 3);
    }

    #[test]
    fn rust_use_declaration_is_extracted() {
        let tree = parse_rust("use crate::helper::run;\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "crate::helper::run");
        assert_eq!(facts[0].alias, None);
    }

    #[test]
    fn rust_use_with_alias_is_extracted() {
        let tree = parse_rust("use crate::helper::run as r;\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "crate::helper::run");
        assert_eq!(facts[0].alias, Some("r".to_string()));
    }

    #[test]
    fn javascript_named_import_is_extracted() {
        let tree = parse_js("import { helper } from './helper';\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "'./helper'");
        assert_eq!(facts[0].imported_symbol, Some("helper".to_string()));
    }

    #[test]
    fn javascript_named_import_with_alias_is_extracted() {
        let tree = parse_js("import { helper as h } from './helper';\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].imported_symbol, Some("helper".to_string()));
        assert_eq!(facts[0].alias, Some("h".to_string()));
    }

    // --- CommonJS Extraction Checkpoint: require() -> ImportFact (B2-ID-013) ---

    #[test]
    fn javascript_require_simple_binding_is_extracted() {
        let tree = parse_js("const x = require('./x');\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "'./x'");
        assert_eq!(facts[0].imported_symbol, None);
        assert_eq!(facts[0].alias, Some("x".to_string()));
    }

    #[test]
    fn javascript_require_aliased_binding_is_extracted() {
        let tree = parse_js("var y = require('./mod');\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "'./mod'");
        assert_eq!(facts[0].alias, Some("y".to_string()));
    }

    #[test]
    fn javascript_require_destructured_binding_produces_one_fact_per_name() {
        let tree = parse_js("const { a, b } = require('./x');\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].module_path, "'./x'");
        assert_eq!(facts[0].imported_symbol, Some("a".to_string()));
        assert_eq!(facts[0].alias, None);
        assert_eq!(facts[1].imported_symbol, Some("b".to_string()));
    }

    #[test]
    fn javascript_require_with_no_assignment_produces_no_fact() {
        // Deliberately out of scope: a bare `require('./x');` with no
        // binding at all is never recognized — there is no name this
        // checkpoint's own narrow collector can honestly attach the
        // fact to via a `variable_declarator`.
        let tree = parse_js("require('./x');\n");
        let facts = extract_import_statements(&tree);
        assert!(facts.is_empty(), "a bare require() call with no variable_declarator must produce no ImportFact in this checkpoint's scope");
    }

    #[test]
    fn javascript_require_nested_inside_another_call_produces_no_fact() {
        // Deliberately out of scope: `require(...)` as an argument to
        // some other call is not a `variable_declarator`'s direct RHS.
        let tree = parse_js("const x = wrap(require('./x'));\n");
        let facts = extract_import_statements(&tree);
        assert!(facts.is_empty(), "require() nested inside another call is out of this checkpoint's narrow scope");
    }

    #[test]
    fn javascript_require_of_a_package_not_a_relative_path_is_still_extracted() {
        let tree = parse_js("const extend = require('extend');\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "'extend'");
    }

    #[test]
    fn javascript_member_expression_call_named_require_is_not_confused_with_the_real_builtin() {
        // `foo.require('./x')` is a DIFFERENT call (a method named
        // "require" on some object) — its callee is a
        // `member_expression`, not a bare `identifier`, so
        // `require_call_module_path` must reject it, never
        // mistaking it for the real CommonJS builtin.
        let tree = parse_js("const x = foo.require('./x');\n");
        let facts = extract_import_statements(&tree);
        assert!(facts.is_empty(), "a method call merely NAMED require on some receiver must never be treated as the CommonJS builtin");
    }

    #[test]
    fn csharp_using_directive_is_extracted() {
        let tree = parse_csharp("using System;\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "System");
    }

    #[test]
    fn csharp_using_alias_directive_is_extracted() {
        let tree = parse_csharp("using Foo = System.Bar;\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "System.Bar");
        assert_eq!(facts[0].alias, Some("Foo".to_string()));
    }

    #[test]
    fn csharp_global_using_directive_is_extracted_with_no_alias() {
        let tree = parse_csharp("global using System;\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "System");
        assert_eq!(facts[0].alias, None);
    }

    #[test]
    fn csharp_using_static_directive_is_extracted_with_no_alias() {
        let tree = parse_csharp("using static System.Math;\n");
        let facts = extract_import_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].module_path, "System.Math");
        assert_eq!(facts[0].alias, None);
    }

    // --- Resolution against a snapshot pool ---

    #[test]
    fn an_import_resolving_to_exactly_one_internal_snapshot_is_internal() {
        let repo = test_repo();
        let importing_file = test_file(repo, "src/model.py");
        let tree = parse_python("from mymodule import helper\n");
        let target = test_snapshot(repo, "src/mymodule.py", "helper", "function_definition", "def helper():\n    pass");

        let facts = resolve_import_facts(&importing_file, &tree, &[target.clone()]).unwrap();

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].resolution, ImportResolution::Internal(target.id()));
    }

    #[test]
    fn an_import_of_a_standard_library_module_is_external_not_invented() {
        let repo = test_repo();
        let importing_file = test_file(repo, "src/model.py");
        let tree = parse_python("import torch\n");

        let facts = resolve_import_facts(&importing_file, &tree, &[]).unwrap();

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].resolution, ImportResolution::External);
    }

    #[test]
    fn an_import_matching_two_candidates_is_ambiguous_not_arbitrarily_picked() {
        let repo = test_repo();
        let importing_file = test_file(repo, "src/model.py");
        let tree = parse_python("from mymodule import helper\n");
        let candidate_a = test_snapshot(repo, "src/a.py", "helper", "function_definition", "def helper():\n    pass");
        let candidate_b = test_snapshot(repo, "src/b.py", "helper", "function_definition", "def helper():\n    pass");

        let facts = resolve_import_facts(&importing_file, &tree, &[candidate_a, candidate_b]).unwrap();

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].resolution, ImportResolution::Ambiguous);
    }

    #[test]
    fn a_bare_import_resolves_by_its_module_paths_last_segment() {
        let repo = test_repo();
        let importing_file = test_file(repo, "src/user.py");
        let tree = parse_python("import torch\n");
        let torch_module = test_snapshot(repo, "src/torch.py", "torch", "class_definition", "class torch:\n    pass");

        let facts = resolve_import_facts(&importing_file, &tree, &[torch_module.clone()]).unwrap();

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].resolution, ImportResolution::Internal(torch_module.id()));
    }

    #[test]
    fn a_duplicate_snapshot_id_in_the_pool_is_a_typed_rejection() {
        let repo = test_repo();
        let importing_file = test_file(repo, "src/model.py");
        let tree = parse_python("from mymodule import helper\n");
        let target = test_snapshot(repo, "src/mymodule.py", "helper", "function_definition", "def helper():\n    pass");

        let result = resolve_import_facts(&importing_file, &tree, &[target.clone(), target]);

        assert_eq!(result.err(), Some(ImportFactError::DuplicateSnapshotId));
    }

    #[test]
    fn two_files_importing_the_same_target_each_produce_their_own_distinct_fact() {
        // The whole point of keying by FileIdentity rather than
        // duplicating onto symbols: two DIFFERENT importing files each
        // get their own ImportFact, never merged or deduplicated across
        // files just because they resolve to the same target.
        let repo = test_repo();
        let file_a = test_file(repo, "src/a.py");
        let file_b = test_file(repo, "src/b.py");
        let tree = parse_python("from mymodule import helper\n");
        let target = test_snapshot(repo, "src/mymodule.py", "helper", "function_definition", "def helper():\n    pass");

        let facts_a = resolve_import_facts(&file_a, &tree, &[target.clone()]).unwrap();
        let facts_b = resolve_import_facts(&file_b, &tree, &[target.clone()]).unwrap();

        assert_eq!(facts_a.len(), 1);
        assert_eq!(facts_b.len(), 1);
        assert_eq!(facts_a[0].importing_file, file_a);
        assert_eq!(facts_b[0].importing_file, file_b);
        assert_ne!(facts_a[0].importing_file, facts_b[0].importing_file);
    }

    #[test]
    fn resolving_the_same_tree_twice_produces_deterministic_identical_facts() {
        let repo = test_repo();
        let importing_file = test_file(repo, "src/model.py");
        let tree = parse_python("import torch\nfrom foo import bar\nfrom foo import baz as b\n");
        let bar_target = test_snapshot(repo, "src/foo.py", "bar", "function_definition", "def bar():\n    pass");

        let first = resolve_import_facts(&importing_file, &tree, &[bar_target.clone()]).unwrap();
        let second = resolve_import_facts(&importing_file, &tree, &[bar_target]).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn an_import_statement_never_produces_one_fact_per_symbol_in_the_file() {
        // The exact granularity defect B2-ID-005 removed: resolving
        // against a pool that contains MULTIPLE symbols from the
        // importing file itself must still produce exactly ONE fact per
        // import statement, never one per (import, symbol-in-file) pair.
        let repo = test_repo();
        let importing_file = test_file(repo, "src/model.py");
        let tree = parse_python("from mymodule import helper\nclass A:\n    pass\nclass B:\n    pass\nclass C:\n    pass\n");
        let target = test_snapshot(repo, "src/mymodule.py", "helper", "function_definition", "def helper():\n    pass");
        let unrelated_a = test_snapshot(repo, "src/model.py", "A", "class_definition", "class A:\n    pass");
        let unrelated_b = test_snapshot(repo, "src/model.py", "B", "class_definition", "class B:\n    pass");
        let unrelated_c = test_snapshot(repo, "src/model.py", "C", "class_definition", "class C:\n    pass");

        let facts = resolve_import_facts(&importing_file, &tree, &[target, unrelated_a, unrelated_b, unrelated_c]).unwrap();

        assert_eq!(facts.len(), 1, "exactly one fact for the one import statement, regardless of how many unrelated symbols exist in the given pool");
    }
}
