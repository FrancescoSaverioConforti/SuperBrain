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
//!
//! # B2-ID-007: nested-declaration ownership boundary
//!
//! Each `collect_*` function below walks the ENTIRE re-parsed fragment
//! of one snapshot's own `signature_text` looking for base/heritage
//! clauses — and, per every current extractor's own design, that
//! fragment is the FULL body of whatever declaration the snapshot
//! represents, which can textually contain an unrelated, NESTED type
//! declaration's own base clause too (e.g. Python's `class Outer:
//! class Inner(Base): ...`). Confirmed, via a real-parse reproduction
//! during B2-ID-007's own resolver-ownership audit, that this
//! resolver shared the identical defect [B2-ID-006](../../../../docs/research/findings/B2-ID-006.md)
//! found in `resolve_calls`: without a boundary, `Outer`'s own
//! re-parse would ALSO rediscover `Inner`'s base clause, spuriously
//! producing `Outer -> Base` alongside the correct `Inner -> Base`.
//!
//! The correct ownership rule here is DIFFERENT from `Calls`'s own
//! "nearest executable scope" invariant — inheritance has no
//! executable-scope concept at all; a base/heritage clause belongs to
//! the SPECIFIC type declaration whose own header/clause syntax names
//! it, full stop. `for_each_node_not_inside_a_nested_boundary` (below)
//! implements exactly this: it visits the outermost declaration
//! `collect_*` is being asked about (always visited, regardless of its
//! own node kind), and recurses into its body, but stops the moment it
//! encounters a SECOND node of one of the given `boundary_kinds` —
//! a genuinely nested declaration — visiting neither that node nor
//! anything beneath it. That nested declaration's own base clause is
//! picked up correctly and independently when `resolve_extends_implements`'s
//! own outer loop reaches ITS specific snapshot on a later iteration.
//! See [B2-ID-007](../../../../docs/research/findings/B2-ID-007.md)
//! for the full investigation, the resolver-ownership audit that found
//! this, and why `resolve_instantiates`/`resolve_contains` were
//! confirmed NOT to share this defect.

use std::collections::HashMap;

use codegraph_domain::{EdgeKind, SymbolKind, SymbolSnapshotId};

use super::resolution::{resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
use crate::csharp_declarations::{CSharpTypeDeclarationContext, CSharpTypeKind};
use crate::edges::calls::parser_for;
use crate::parse::Language;
use crate::snapshot::SymbolSnapshot;

/// Compatibility wrapper: identical behavior to calling
/// [`resolve_extends_implements_with_csharp_contexts`] with an empty
/// `csharp_contexts` slice — confirmed by a dedicated equivalence test.
/// Every one of this crate's ~30 existing call sites (tests, and every
/// prior domain evaluation harness, none of which wire up C#
/// declaration contexts) keeps this exact signature and exact behavior
/// unchanged by B2-ID-010.
pub fn resolve_extends_implements(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    resolve_extends_implements_with_csharp_contexts(snapshots, &[])
}

/// B2-ID-010: identical to [`resolve_extends_implements`] in every way
/// EXCEPT for one additional reconciliation step this function's own
/// base-name lookup applies when `candidates.len() != 1` for a C#
/// base/heritage name: before giving up on an ambiguous or
/// zero-candidate match, check whether `csharp_contexts` shows that
/// EVERY same-named candidate for THIS name among `snapshots` is a
/// `partial` fragment of the SAME logical type (same namespace, same
/// generic arity, same `CSharpTypeKind`) — if so, treat them as one
/// logical target for this relation only (never merging the
/// underlying snapshots, never changing their identity), per
/// [B2-ID-010](../../../../docs/research/findings/B2-ID-010.md). A
/// non-C# base name, or a C# name with no matching context at all
/// (e.g. `csharp_contexts` is empty), falls through to the EXACT same
/// "ambiguous/zero -> no edge" behavior `resolve_extends_implements`
/// always had.
pub fn resolve_extends_implements_with_csharp_contexts(snapshots: &[SymbolSnapshot], csharp_contexts: &[CSharpTypeDeclarationContext]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    // Index contexts by (namespace, simple_name, generic_arity,
    // type_kind) — the FULL semantic type key, not just
    // (namespace, simple_name). A real C# file commonly declares
    // BOTH a non-generic and a generic partial type sharing one bare
    // name in the SAME file (e.g. `partial class Policy { .. }`
    // alongside `partial class Policy<TResult> { .. }` in
    // Policy.HandleSyntax.cs) — grouping by name alone would merge
    // these two genuinely DIFFERENT types' fragments into one group,
    // whose own arity values disagree, incorrectly blocking
    // reconciliation for BOTH instead of correctly reconciling each
    // arity's own fragment set independently. Confirmed as a real
    // defect via B2-EVAL-008's own real-corpus remeasurement on
    // `App-vNext/Polly` before this fix: `BulkheadPolicy : Policy`
    // (arity 0) failed to reconcile because `Policy`'s own 8 arity-0
    // fragments and 2 arity-1 fragments were grouped together under
    // one name-only key. `snapshot_id` is `None` for any declaration
    // whose own candidate was rejected at promotion — such a context
    // is never a usable reconciliation target (no real snapshot to
    // point an edge at).
    let mut contexts_by_name: HashMap<(&str, &str, usize, CSharpTypeKind), Vec<&CSharpTypeDeclarationContext>> = HashMap::new();
    for context in csharp_contexts {
        if context.snapshot_id.is_some() {
            contexts_by_name.entry((context.namespace.as_str(), context.simple_name.as_str(), context.generic_arity, context.type_kind)).or_default().push(context);
        }
    }

    for snapshot in lookup.snapshots() {
        let Some(parser) = parser_for(snapshot.file()) else {
            continue;
        };
        let Ok(tree) = parser.parse(snapshot.signature().as_str().as_bytes()) else {
            continue;
        };

        // B2-ID-002 scope audit: the same base/interface name appearing
        // more than once in one declaration's base/heritage list (a
        // syntactically odd but parseable shape, e.g. `class C(A, A)`)
        // must collapse to ONE edge per `(to, kind)`, never one per
        // occurrence — confirmed present here too by a dedicated
        // regression test, the same occurrence-vs-relation mismatch
        // `resolve_calls` had. Every edge this resolver produces is
        // `StaticAnalysis` (an ambiguous/unresolved base produces no
        // edge at all), so there is no evidence-strength tie-break to
        // make here either.
        let mut seen_targets: std::collections::HashSet<(SymbolSnapshotId, EdgeKind)> = std::collections::HashSet::new();

        for (base_name, syntactic_kind) in base_names_in(&tree) {
            let candidates = lookup.find_by_name(&base_name, snapshot.id());
            let resolved_target = if candidates.len() == 1 {
                Some(candidates[0])
            } else if tree.language() == Language::CSharp {
                reconcile_partial_fragments(&base_name, &candidates, &contexts_by_name)
            } else {
                None
            };

            let Some(target) = resolved_target else {
                // Zero: base not found among the given pool (e.g. a
                // stdlib/third-party base type) -> no edge. More than
                // one, and not a fully-partial fragment set of one
                // logical type: ambiguous same-named base across files
                // -> no edge either (unlike Calls, inheritance is a 1:1
                // structural claim about THIS declaration's one actual
                // base, so an ambiguous same-name match cannot honestly
                // be split into multiple heuristic edges the way a call
                // site's multiple real overload candidates can).
                continue;
            };
            let kind = edge_kind_for(syntactic_kind, target.kind());
            if !seen_targets.insert((target.id(), kind)) {
                continue;
            }
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

/// B2-ID-010's own reconciliation rule. `candidates` is every snapshot
/// in the pool named `base_name` (already excluding the caller's own
/// snapshot), i.e. exactly the set `resolve_extends_implements`'s own
/// base case found ambiguous (`!= 1`). Returns `Some` ONLY when:
///
/// 1. `csharp_contexts` has a context for at least one of `candidates`
///    (if a C# base name has no context coverage at all — e.g. the
///    caller never wired up C# contexts — this function changes
///    nothing, matching the compatibility wrapper's own guarantee);
/// 2. EVERY context found for THAT `(namespace, simple_name)` key is
///    `is_partial: true` — a single non-partial declaration anywhere
///    in that group means these are genuinely distinct types that
///    merely share a bare name (ordinary duplicate-name ambiguity),
///    never reconciled;
/// 3. every context's own `generic_arity`/`type_kind` agrees — a
///    `Policy`/`Policy<T>` pair is NEVER reconciled together (that is
///    the SEPARATE, Moderate generic-arity-collision gap, explicitly
///    out of this fix's scope); a `Class`/`Interface` pair sharing a
///    bare name is likewise never reconciled (incompatible kinds can
///    never be fragments of the same logical type);
/// 4. the reconciled candidate SET (by `snapshot_id`) exactly equals
///    `candidates` — if a context's own namespace/arity/kind group
///    contains a snapshot NOT present in `candidates` (or vice versa),
///    the context data and the live candidate pool disagree, and this
///    function conservatively refuses to guess rather than reconcile
///    on stale/mismatched information. (This equality requirement also
///    guarantees the representative this function ultimately returns
///    is always found inside `candidates` — never needs a separate
///    fallback lookup against a wider pool.)
///
/// On success, returns ONE deterministically-chosen representative
/// candidate (the lowest-byte-span-start fragment, a stable,
/// order-independent tie-break) — never merges or fuses the
/// candidates' own distinct `SymbolSnapshotId`s; the OTHER fragments
/// remain fully independent snapshots, simply not individually pointed
/// at by this one edge.
fn reconcile_partial_fragments<'a>(base_name: &str, candidates: &[&'a SymbolSnapshot], contexts_by_name: &HashMap<(&str, &str, usize, CSharpTypeKind), Vec<&CSharpTypeDeclarationContext>>) -> Option<&'a SymbolSnapshot> {
    if candidates.is_empty() {
        return None;
    }

    // `candidates` is every snapshot named `base_name` in the WHOLE
    // pool, by bare name alone — this can legitimately span MULTIPLE
    // distinct semantic types sharing that name (e.g. a real file
    // commonly declares both `partial class Policy { .. }` and
    // `partial class Policy<TResult> { .. }` side by side — two
    // different arities, two different logical types, one shared bare
    // name). `contexts_by_name` is already grouped by the FULL type
    // key (namespace, simple_name, generic_arity, type_kind), so each
    // group here represents exactly ONE candidate logical type's own
    // fragment set. A base/heritage clause names a type by its bare
    // simple name with no arity marker at all (C#'s own `: Policy`
    // syntax never carries `<T>`), so the base clause itself always
    // means the NON-GENERIC (arity 0) member of that name — the
    // search below is therefore scoped to `generic_arity == 0`
    // groups only, never guessing among multiple different-arity
    // candidates sharing the bare name.
    //
    // Every group (across ALL namespaces/arities/kinds) sharing this
    // bare `simple_name` must be examined together: `candidates` (the
    // full, bare-by-name pool) can legitimately be the UNION of
    // several genuinely distinct semantic types' own fragment sets
    // (e.g. one non-generic `partial class Foo` plus an unrelated
    // generic `partial class Foo<T>` in the same file; or two
    // unrelated partial `Foo` types in two different namespaces).
    // Reconciliation is sound ONLY when that union accounts for
    // `candidate_ids` EXACTLY — i.e. there is NO leftover candidate
    // this name-grouping cannot explain — which both guards against
    // a stale/mismatched context data set (same invariant the
    // original equality check enforced) and, crucially, against ever
    // reconciling just because one namespace/arity GROUP happens to
    // be a subset while ANOTHER real candidate (a different arity, a
    // different namespace) is left unaccounted for.
    let candidate_ids: std::collections::HashSet<SymbolSnapshotId> = candidates.iter().map(|c| c.id()).collect();

    let all_groups_for_this_name: Vec<&Vec<&CSharpTypeDeclarationContext>> = contexts_by_name.iter().filter(|((_, simple_name, _, _), _)| *simple_name == base_name).map(|(_, group)| group).collect();
    let union_of_all_groups: std::collections::HashSet<SymbolSnapshotId> = all_groups_for_this_name.iter().flat_map(|group| group.iter().filter_map(|c| c.snapshot_id)).collect();
    if union_of_all_groups != candidate_ids {
        return None;
    }

    // Exactly ONE arity-0 group (a base/heritage clause's own bare
    // name, e.g. `: Policy`, always means arity 0 — C# base-clause
    // syntax never carries a `<T>` marker) must exist among those
    // groups; more than one would mean the SAME namespace declared
    // this arity-0 type twice independently (impossible if every
    // group is internally `is_partial`-consistent and this group set
    // was built from real, non-overlapping namespace/arity/kind keys,
    // but checked explicitly rather than assumed).
    let arity_0_groups: Vec<&&Vec<&CSharpTypeDeclarationContext>> = all_groups_for_this_name.iter().filter(|group| group.iter().all(|c| c.generic_arity == 0)).collect();
    if arity_0_groups.len() != 1 {
        return None;
    }
    let matching_group = *arity_0_groups[0];

    // A group of exactly ONE fragment is not a partial-class split at
    // all — nothing to reconcile. If `candidates` was ambiguous
    // (`.len() > 1`) while this arity-0 group has only one member, the
    // real ambiguity is caused entirely by OTHER groups sharing this
    // bare name (e.g. a same-named but different-arity type, or a
    // same-named but non-partial duplicate) — never something this
    // fix should explain away. Requiring at least 2 fragments here is
    // what actually distinguishes "Foo (arity 0, 1 fragment) vs. Foo<T>
    // (arity 1, 1 fragment), ambiguous by bare name alone" from "Foo
    // (arity 0, split across 2+ partial fragments)."
    if matching_group.len() < 2 {
        return None;
    }

    // Every context in the matching group must be partial — a single
    // non-partial declaration anywhere in this exact-arity, exact-kind
    // group means these are genuinely distinct types that merely share
    // a bare name (ordinary duplicate-name ambiguity), never
    // reconciled. (generic_arity/type_kind agreement is already
    // guaranteed by construction: `contexts_by_name`'s own key
    // includes both, so every context inside one group shares them —
    // this is no longer a separate check to perform here.)
    let all_partial = matching_group.iter().all(|c| c.is_partial);
    if !all_partial {
        return None;
    }

    // Deterministic representative: the fragment with the lowest span
    // start, found by looking up each context's own snapshot_id back
    // against `candidates` (never guessed by name alone). The lookup
    // is guaranteed to succeed: `matching_group` was found above by
    // requiring its own snapshot-id set to equal `candidate_ids`
    // exactly, so `representative_id` — drawn from that same group —
    // is always a member of `candidates`.
    let mut sorted_group: Vec<&CSharpTypeDeclarationContext> = matching_group.clone();
    sorted_group.sort_by_key(|c| c.span.start);
    let representative_id = sorted_group.first()?.snapshot_id?;
    candidates.iter().find(|c| c.id() == representative_id).copied()
}

/// Picks the final `EdgeKind` for one resolved base/interface name.
/// `syntactic_kind` is `Some(kind)` when the SOURCE SYNTAX ITSELF
/// already said which kind applies — Python's single base-class
/// construct (always `Extends`), JavaScript's `extends` (always
/// `Extends`), and TypeScript/Tsx's `extends` (`Extends`) vs.
/// `implements` (`Implements`), each tagged at the point `base_names_in`
/// collects them — see `collect_js_heritage`. (Rust's own `impl Trait
/// for Type` is NOT one of these languages: B2-ID-008 moved Rust's
/// trait-implementation handling entirely into `impls.rs`'s `ImplFact`
/// mechanism, so `base_names_in` never collects anything for
/// `Language::Rust` at all — see that match arm's own comment below.)
/// It is `None` only for C#/C++, whose `base_list`/`base_class_clause`
/// syntax does NOT distinguish a base class from an implemented
/// interface at all; for those, and ONLY for those, this function falls
/// back to the target's own already-resolved `SymbolKind` (`Interface`
/// -> `Implements`, anything else -> `Extends`). A syntactic tag, when
/// present, is NEVER overridden by the target's kind — the earlier
/// version of this resolver incorrectly routed TypeScript/Tsx through
/// the same kind-based fallback as C#/C++ despite TS/Tsx's `implements`
/// keyword being just as syntactically explicit as Python's/JS's own
/// `extends`; that was a real defect (TS/Tsx's own syntax was being
/// ignored in favor of a lookup that could disagree with it), fixed here
/// by threading the clause tag through instead of discarding it before
/// this function runs.
fn edge_kind_for(syntactic_kind: Option<EdgeKind>, target_kind: SymbolKind) -> EdgeKind {
    syntactic_kind.unwrap_or_else(|| if target_kind == SymbolKind::Interface { EdgeKind::Implements } else { EdgeKind::Extends })
}

/// Every base-class/interface name found in `tree`'s one construct (a
/// `class_definition`/`class_declaration`/`class_specifier` re-parsed
/// standalone from a `SymbolSnapshot`'s own signature text — the same
/// re-parse-the-signature approach `calls.rs` and `imports.rs` use),
/// paired with the `EdgeKind` the SOURCE SYNTAX implies when the
/// language's grammar makes that unambiguous, or `None` when it does
/// not (C#/C++ only — see `edge_kind_for`'s doc comment for what
/// happens then). Rust's own `impl_item` is deliberately NOT one of the
/// node kinds this function (or anything it dispatches to) ever
/// collects from — see the `Language::Rust` match arm below.
fn base_names_in(tree: &crate::parse::ParseTree) -> Vec<(String, Option<EdgeKind>)> {
    let mut names = Vec::new();
    match tree.language() {
        Language::Python => {
            let mut plain_names = Vec::new();
            collect_python_bases(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, Some(EdgeKind::Extends))));
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
        // Rust yields no `Extends`/`Implements` edges from this resolver by
        // design: `impl Trait for Type` is extracted as an `ImplFact` in
        // `impls.rs` (B2-ID-008), not as a base/heritage name here.
        Language::Rust | Language::Html | Language::Css => {}
    }
    names
}

/// B2-ID-007: visits every descendant of `node` (the SAME full-subtree
/// walk every `collect_*` function already performed before this fix),
/// calling `visit` on each one, EXCEPT any node strictly nested inside
/// a DIFFERENT type declaration than the one `node` itself started in
/// — i.e. once the walk passes a node whose own `kind()` is one of
/// `boundary_kinds` (a NEW type declaration starting, e.g. a nested
/// `class_definition`), neither that boundary node NOR anything
/// beneath it is visited. `node` itself is always visited first
/// (matching every existing `collect_*` function's own original
/// behavior of checking the entry node before recursing into it), and
/// the "we have crossed a boundary" state is threaded explicitly
/// through the recursion via `inside_a_nested_boundary`, not
/// re-derived fresh from each step's own immediate parent — re-deriving
/// it fresh was an earlier, incorrect version of this fix: it only
/// checked a node's DIRECT children for a boundary kind, which misses
/// a nested declaration reachable through an intermediate non-boundary
/// WRAPPER node (confirmed concretely: Python's own `class_definition
/// -> block -> class_definition` shape, where `block` sits between an
/// outer class and a nested one and is not itself a boundary kind,
/// let the nested class through unprotected once the recursive call's
/// own locally-recomputed "is a boundary" check reset at that wrapper
/// step). Threading the flag explicitly, rather than recomputing it,
/// fixes that gap: once `true`, it stays `true` for the rest of that
/// subtree, through any number of intermediate wrapper nodes.
fn for_each_node_not_inside_a_nested_boundary(node: tree_sitter::Node, boundary_kinds: &[&str], mut visit: impl FnMut(tree_sitter::Node)) {
    // `already_entered_one_boundary` tracks whether the walk has
    // already passed through (and visited) ONE boundary-kind node —
    // the outermost declaration itself, which must always be visited
    // regardless of its own kind. Only a SECOND (or deeper) boundary-
    // kind node, found AFTER the first one, is a genuinely NESTED
    // declaration and gets skipped. Without this distinction, the
    // entry declaration itself (whose `kind()` IS a boundary kind,
    // e.g. the outermost `class_definition`) would incorrectly skip
    // itself on the very first step.
    fn walk(node: tree_sitter::Node, boundary_kinds: &[&str], already_entered_one_boundary: bool, visit: &mut impl FnMut(tree_sitter::Node)) {
        let this_node_is_a_boundary = boundary_kinds.contains(&node.kind());
        if this_node_is_a_boundary && already_entered_one_boundary {
            return;
        }
        visit(node);
        let next_already_entered_one_boundary = already_entered_one_boundary || this_node_is_a_boundary;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, boundary_kinds, next_already_entered_one_boundary, visit);
        }
    }
    walk(node, boundary_kinds, false, &mut visit);
}

fn collect_python_bases(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    for_each_node_not_inside_a_nested_boundary(node, &["class_definition"], |n| {
        if n.kind() != "class_definition" {
            return;
        }
        let mut cursor = n.walk();
        let found_args = n.children(&mut cursor).find(|c| c.kind() == "argument_list");
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
    });
}

fn collect_csharp_base_list(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    for_each_node_not_inside_a_nested_boundary(node, &["class_declaration", "interface_declaration"], |n| {
        if n.kind() != "base_list" {
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "identifier" {
                out.push(text_of(child, source));
            }
        }
    });
}

fn collect_cpp_base_clause(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    for_each_node_not_inside_a_nested_boundary(node, &["class_specifier", "struct_specifier"], |n| {
        if n.kind() != "base_class_clause" {
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                out.push(text_of(child, source));
            }
        }
    });
}

fn collect_js_heritage(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    // Plain JavaScript grammar only ever has a bare `extends identifier`
    // in `class_heritage` — no `implements` keyword exists in this
    // grammar at all, so every name collected here is unconditionally
    // Extends (tagged by the caller in `base_names_in`).
    for_each_node_not_inside_a_nested_boundary(node, &["class_declaration"], |n| {
        if !matches!(n.kind(), "class_heritage") {
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if matches!(child.kind(), "identifier" | "type_identifier") {
                out.push(text_of(child, source));
            }
        }
    });
}

/// TypeScript/Tsx's `class_heritage` can carry BOTH a plain `extends
/// identifier` child AND a nested `implements_clause` on the same
/// class. Unlike plain JS, this grammar's `implements` keyword is a
/// real syntactic marker, just as explicit as Python's base-class list
/// (Rust's own trait-vs-type distinction is no longer this file's own
/// concern at all — see the `Language::Rust` match arm's own comment in
/// `base_names_in`) — so each name is tagged with the
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
    // output: `class_declaration`/`abstract_class_declaration`'s
    // `class_heritage` direct children are wrapper nodes —
    // `extends_clause` (containing one `identifier`/`type_identifier`,
    // optionally generic) and/or `implements_clause` (containing one or
    // more `type_identifier`s, each optionally generic) — never a bare
    // identifier directly. `interface_declaration` has its OWN distinct
    // wrapper, `extends_type_clause` (no `implements_clause` exists for
    // interfaces in this grammar), attached directly under the
    // interface node rather than under a `class_heritage`.
    //
    // B2-ID-019: a heritage name can ALSO be wrapped one level deeper in
    // a `generic_type` node (`Bar<T>`'s child is `generic_type`, whose
    // own first child is the plain `type_identifier` "Bar", followed by
    // a `type_arguments` node this resolver has no use for — B2-ID-019's
    // scope is the simple target name, never the type arguments
    // themselves, so `type_arguments` is never descended into). Confirmed
    // against this workspace's real tree-sitter-typescript output for
    // `implements Bar<T>`, `extends Bar<T>` and comma-separated mixes of
    // generic/non-generic names in both `implements_clause` and
    // `extends_type_clause`. Any wrapper shape other than a bare
    // `identifier`/`type_identifier` or a `generic_type` whose own first
    // named child is one of those is NOT unwrapped — fail closed, no
    // guessed name, per "no edge beats a false edge".
    for_each_node_not_inside_a_nested_boundary(node, &["class_declaration", "abstract_class_declaration", "interface_declaration"], |n| {
        match n.kind() {
            "class_heritage" => {
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    let tag = match child.kind() {
                        "extends_clause" => Some(EdgeKind::Extends),
                        "implements_clause" => Some(EdgeKind::Implements),
                        _ => None,
                    };
                    if let Some(tag) = tag {
                        collect_ts_heritage_names(child, source, tag, out);
                    }
                }
            }
            "extends_type_clause" => {
                collect_ts_heritage_names(n, source, EdgeKind::Extends, out);
            }
            _ => {}
        }
    });
}

/// Collects every heritage target name directly under `clause` (an
/// `extends_clause`, `implements_clause`, or `extends_type_clause`
/// node), unwrapping one level of `generic_type` where present. See
/// `collect_ts_heritage_tagged`'s own doc comment for the confirmed node
/// shapes this relies on.
fn collect_ts_heritage_names(clause: tree_sitter::Node, source: &[u8], tag: EdgeKind, out: &mut Vec<(String, Option<EdgeKind>)>) {
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        match child.kind() {
            "identifier" | "type_identifier" => {
                out.push((text_of(child, source), Some(tag)));
            }
            "generic_type" => {
                if let Some(name_node) = child.child(0) {
                    if matches!(name_node.kind(), "identifier" | "type_identifier") {
                        out.push((text_of(name_node, source), Some(tag)));
                    }
                }
            }
            _ => {}
        }
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
    fn rust_impl_trait_for_type_produces_zero_extends_or_implements_edges_by_design() {
        // B2-ID-008: `impl Trait for Type` is now extracted by `impls.rs`'s
        // `ImplFact` mechanism, not by this resolver. Pins — against a REAL
        // parse of real Rust source, not a placeholder string — that
        // `resolve_extends_implements` deliberately produces no Extends or
        // Implements edge for Rust, so this is a tested fact rather than an
        // accident of deleting `collect_rust_impl_trait`.
        let source = "trait Shape { fn area(&self) -> f64; }\n\nstruct Circle;\n\nimpl Shape for Circle { fn area(&self) -> f64 { 1.0 } }\n";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::RustParser, source.as_bytes()).expect("the fixture must be valid Rust");
        let root = tree.tree().root_node();
        let mut cursor = root.walk();
        assert!(
            root.children(&mut cursor).any(|c| c.kind() == "impl_item"),
            "the real parse must actually contain an impl_item, or this test would prove nothing"
        );

        let repo = {
            let mut registry = RepositoryRegistry::new();
            registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
        };
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, "src/shapes.rs").unwrap();
        let (candidates, rejections) = crate::extract::extract_symbols_reporting_rejections(&tree);
        assert!(rejections.is_empty(), "fixture source must not trigger any extraction rejection: {rejections:?}");
        let snapshots: Vec<SymbolSnapshot> = candidates.into_iter().map(|c| promote(&c, file.clone()).expect("a real extracted candidate must always promote")).collect();

        let edges = resolve_extends_implements(&snapshots).unwrap();

        let structural_edges: Vec<_> = edges.iter().filter(|e| matches!(e.kind, EdgeKind::Extends | EdgeKind::Implements)).collect();
        assert!(structural_edges.is_empty(), "Rust must produce zero Extends/Implements edges from resolve_extends_implements; ImplFact (impls.rs) is the real mechanism");
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

    // B2-ID-019: `implements`/`extends` naming a generic interface/base
    // (`Bar<T>`) must resolve identically to the non-generic case — the
    // resolver extracts only the simple target name, never the type
    // argument, and never attempts type inference.

    #[test]
    fn typescript_generic_implements_resolves_by_simple_name() {
        let interface = test_snapshot("src/ifoo.ts", "Bar", "interface_declaration", "interface Bar<T> {}");
        let implementor = test_snapshot("src/c.ts", "C", "class_declaration", "class C implements Bar<T> {}");
        let snapshots = vec![interface.clone(), implementor.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Implements);
        assert_eq!(edges[0].to, interface.id());
    }

    #[test]
    fn typescript_generic_extends_resolves_by_simple_name() {
        let base_interface = test_snapshot("src/ibar.ts", "IBar", "interface_declaration", "interface IBar<T> {}");
        let derived_interface = test_snapshot("src/ifoo.ts", "IFoo", "interface_declaration", "interface IFoo extends IBar<T> {}");
        let snapshots = vec![base_interface.clone(), derived_interface.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Extends);
        assert_eq!(edges[0].to, base_interface.id());
    }

    #[test]
    fn typescript_multiple_generic_interfaces_all_resolve() {
        let a = test_snapshot("src/a.ts", "A", "interface_declaration", "interface A<T> {}");
        let b = test_snapshot("src/b.ts", "B", "interface_declaration", "interface B<U> {}");
        let implementor = test_snapshot("src/c.ts", "C", "class_declaration", "class C implements A<T>, B<U> {}");
        let snapshots = vec![a.clone(), b.clone(), implementor.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|e| e.kind == EdgeKind::Implements));
        assert!(edges.iter().any(|e| e.to == a.id()));
        assert!(edges.iter().any(|e| e.to == b.id()));
    }

    #[test]
    fn typescript_mixed_generic_and_non_generic_heritage_both_resolve() {
        let a = test_snapshot("src/a.ts", "A", "interface_declaration", "interface A {}");
        let b = test_snapshot("src/b.ts", "B", "interface_declaration", "interface B<T> {}");
        let implementor = test_snapshot("src/c.ts", "C", "class_declaration", "class C implements A, B<T> {}");
        let snapshots = vec![a.clone(), b.clone(), implementor.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 2);
        assert!(edges.iter().any(|e| e.to == a.id()));
        assert!(edges.iter().any(|e| e.to == b.id()));
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
    fn b2_id_002_scope_audit_the_same_base_named_twice_does_not_produce_a_duplicate_edge() {
        // Scope audit for B2-ID-002 (found in resolve_calls, also found
        // in resolve_imports): checks whether resolve_extends_implements
        // has the same occurrence-vs-relation mismatch. A syntactically
        // odd but parseable `class C(A, A)` (the same base named twice)
        // must collapse to ONE Extends edge, not two identical ones.
        let base_a = test_snapshot("src/a.py", "A", "class_definition", "class A:\n    pass");
        let derived = test_snapshot("src/c.py", "C", "class_definition", "class C(A, A):\n    pass");
        let snapshots = vec![base_a.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        let to_a: Vec<_> = edges.iter().filter(|e| e.to == base_a.id()).collect();
        assert_eq!(to_a.len(), 1, "the same base named twice must collapse to one edge, not duplicate (from,to,Extends)");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let snapshot = test_snapshot("src/lib.rs", "Foo", "struct_item", "struct Foo;");
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_extends_implements(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    // B2-ID-007 regression tests: a nested type declaration's own
    // base/heritage clause must be owned by that declaration alone,
    // never also by an outer ancestor container. Every snapshot below
    // is built from REAL extraction (`extract_symbols_reporting_rejections`
    // + `promote`), not a hand-written placeholder `signature_text` —
    // the same reason B2-ID-006's own regression tests use real
    // extraction: a placeholder body structurally cannot exercise this
    // defect.
    mod b2_id_007_nested_declaration_ownership {
        use super::*;
        use crate::extract::extract_symbols_reporting_rejections;
        use crate::parse::parser_for;

        fn real_snapshots(repo: codegraph_domain::RepositoryId, path: &str, source: &str) -> Vec<SymbolSnapshot> {
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, path).unwrap();
            let parser = parser_for(&file).expect("a supported extension must have a parser");
            let tree = parser.parse(source.as_bytes()).expect("the hand-written fixture source must parse");
            let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
            assert!(rejections.is_empty(), "fixture source must not trigger any extraction rejection: {rejections:?}");
            candidates.into_iter().map(|c| promote(&c, file.clone()).expect("a real extracted candidate must always promote")).collect()
        }

        fn find<'a>(snapshots: &'a [SymbolSnapshot], name: &str) -> &'a SymbolSnapshot {
            snapshots.iter().find(|s| s.name().as_str() == name).unwrap_or_else(|| panic!("no snapshot named {name:?} among {:?}", snapshots.iter().map(|s| s.name().as_str()).collect::<Vec<_>>()))
        }

        fn sources_of<'a>(edges: &'a [Edge], target: &SymbolSnapshot, snapshots: &'a [SymbolSnapshot]) -> Vec<&'a str> {
            let mut names: Vec<&str> = edges.iter().filter(|e| e.to == target.id()).filter_map(|e| snapshots.iter().find(|s| s.id() == e.from)).map(|s| s.name().as_str()).collect();
            names.sort_unstable();
            names
        }

        fn repo() -> codegraph_domain::RepositoryId {
            let mut registry = RepositoryRegistry::new();
            registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
        }

        #[test]
        fn python_nested_class_extends_is_owned_by_inner_not_outer() {
            let snapshots = real_snapshots(repo(), "src/nested.py", "class Base:\n    pass\n\n\nclass Outer:\n    class Inner(Base):\n        pass\n");
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base; Outer must not also appear");
        }

        #[test]
        fn python_outer_and_inner_both_extend_different_bases_independently() {
            let snapshots = real_snapshots(
                repo(),
                "src/both.py",
                "class BaseA:\n    pass\n\n\nclass BaseB:\n    pass\n\n\nclass Outer(BaseA):\n    class Inner(BaseB):\n        pass\n",
            );
            let base_a = find(&snapshots, "BaseA");
            let base_b = find(&snapshots, "BaseB");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base_a, &snapshots), vec!["Outer"], "Outer's own base must still resolve correctly");
            assert_eq!(sources_of(&edges, base_b, &snapshots), vec!["Inner"], "Inner's own, different base must resolve to Inner alone");
        }

        #[test]
        fn python_sibling_nested_classes_each_own_their_own_base_independently() {
            let snapshots = real_snapshots(
                repo(),
                "src/siblings.py",
                "class Base:\n    pass\n\n\nclass Outer:\n    class InnerA(Base):\n        pass\n\n    class InnerB(Base):\n        pass\n",
            );
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["InnerA", "InnerB"], "both siblings own their own edge; Outer must not also appear");
        }

        #[test]
        fn csharp_nested_class_extends_is_owned_by_inner_not_outer() {
            let snapshots = real_snapshots(repo(), "src/Nested.cs", "class Base {}\n\nclass Outer {\n    class Inner : Base {}\n}\n");
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base in C#; Outer must not also appear");
        }

        #[test]
        fn csharp_nested_interface_implementation_is_owned_by_inner_not_outer() {
            let snapshots = real_snapshots(repo(), "src/NestedIface.cs", "interface IBase {}\n\nclass Outer {\n    class Inner : IBase {}\n}\n");
            let ibase = find(&snapshots, "IBase");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, ibase, &snapshots), vec!["Inner"]);
            assert_eq!(edges.iter().find(|e| e.to == ibase.id()).unwrap().kind, EdgeKind::Implements, "IBase is Interface-kind, so this must resolve as Implements, not Extends");
        }

        #[test]
        fn cpp_nested_class_extends_is_owned_by_inner_not_outer() {
            let snapshots = real_snapshots(repo(), "src/nested.cpp", "class Base {};\n\nclass Outer {\n    class Inner : public Base {};\n};\n");
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base in C++; Outer must not also appear");
        }

        #[test]
        fn javascript_nested_class_extends_is_owned_by_inner_not_outer() {
            // JS doesn't have "nested classes" in a block-scoped sense
            // the way Python/C#/C++ do, but a class CAN be declared
            // inside another class's static initializer block or
            // method body in real JS — the simplest real, parseable
            // shape exercising the same AST nesting is a class
            // expression assigned inside a static block. To keep this
            // reproducible against the actual grammar this project
            // targets, this test instead confirms the boundary holds
            // for the one real nested-class shape tree-sitter-javascript
            // parses without error: a class declared inside another
            // class's own class body via a static block.
            let snapshots = real_snapshots(
                repo(),
                "src/nested.js",
                "class Base {}\n\nclass Outer {\n    static {\n        class Inner extends Base {}\n    }\n}\n",
            );
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base in JS; Outer must not also appear");
        }

        #[test]
        fn typescript_nested_class_extends_is_owned_by_inner_not_outer() {
            // This test uses a class-extends-class shape rather than
            // implements-an-interface, exercising the exact same
            // `class_heritage`/boundary code path
            // `collect_ts_heritage_tagged` shares with the JS
            // collector; the nested-class boundary behavior this test
            // targets is identical either way, and B2-ID-018's
            // interface-specific coverage lives in
            // `extract::javascript`'s own test module instead.
            let snapshots = real_snapshots(
                repo(),
                "src/nested.ts",
                "class Base {}\n\nclass Outer {\n    static {\n        class Inner extends Base {}\n    }\n}\n",
            );
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base in TS; Outer must not also appear");
        }

        #[test]
        fn result_is_deterministic_regardless_of_snapshot_order() {
            let snapshots = real_snapshots(repo(), "src/order.py", "class Base:\n    pass\n\n\nclass Outer:\n    class Inner(Base):\n        pass\n");
            let base = find(&snapshots, "Base");

            let forward_edges = resolve_extends_implements(&snapshots).unwrap();
            let mut reversed = snapshots.clone();
            reversed.reverse();
            let reversed_edges = resolve_extends_implements(&reversed).unwrap();

            assert_eq!(sources_of(&forward_edges, base, &snapshots), sources_of(&reversed_edges, base, &reversed), "ownership attribution must not depend on snapshot pool ordering");
        }
    }

    // B2-ID-010 regression tests: C# `partial class`/`partial interface`
    // fragmentation must be reconciled for Extends/Implements base-name
    // resolution ONLY when every same-named candidate is confirmed, via
    // real `CSharpTypeDeclarationContext` extraction, to be a `partial`
    // fragment of the SAME logical type (same namespace, same generic
    // arity, same type kind). Every fixture below uses REAL extraction
    // (`extract_symbols_reporting_rejections` + `promote` +
    // `extract_csharp_type_declaration_contexts` +
    // `resolve_csharp_type_declaration_contexts`), never a hand-written
    // placeholder context, for the same reason B2-ID-007's own tests do
    // — a placeholder cannot exercise the real grammar shapes this fix
    // depends on (the `modifier`/`partial` node, the `namespace`
    // ancestor walk).
    mod b2_id_010_partial_class_reconciliation {
        use super::*;
        use crate::csharp_declarations::{extract_csharp_type_declaration_contexts, resolve_csharp_type_declaration_contexts};
        use crate::extract::extract_symbols_reporting_rejections;
        use crate::parse::parser_for;

        fn repo() -> codegraph_domain::RepositoryId {
            let mut registry = RepositoryRegistry::new();
            registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
        }

        /// Builds real snapshots AND real, correctly-resolved
        /// `CSharpTypeDeclarationContext`s from one or more (path,
        /// source) files, mirroring exactly what `pipeline.rs`'s own
        /// `index_files` loop does today (extract per-file, then
        /// resolve the combined raw contexts against the combined
        /// snapshot pool).
        fn real_snapshots_and_contexts(repo: codegraph_domain::RepositoryId, files: &[(&str, &str)]) -> (Vec<SymbolSnapshot>, Vec<CSharpTypeDeclarationContext>) {
            let mut all_snapshots = Vec::new();
            let mut raw_contexts = Vec::new();
            for (path, source) in files {
                let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
                let file = FileIdentity::parse(repo, commit, path).unwrap();
                let parser = parser_for(&file).expect("supported extension");
                let tree = parser.parse(source.as_bytes()).expect("fixture must parse");
                let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
                assert!(rejections.is_empty(), "fixture must not trigger rejection: {rejections:?}");
                for candidate in candidates {
                    all_snapshots.push(promote(&candidate, file.clone()).expect("promote"));
                }
                for context in extract_csharp_type_declaration_contexts(&tree) {
                    raw_contexts.push((file.clone(), context));
                }
            }
            let resolved_contexts = resolve_csharp_type_declaration_contexts(&raw_contexts, &all_snapshots);
            (all_snapshots, resolved_contexts)
        }

        fn find<'a>(snapshots: &'a [SymbolSnapshot], name: &str) -> &'a SymbolSnapshot {
            snapshots.iter().find(|s| s.name().as_str() == name).unwrap_or_else(|| panic!("no snapshot named {name:?}"))
        }

        fn sources_of<'a>(edges: &'a [Edge], target: &SymbolSnapshot, snapshots: &'a [SymbolSnapshot]) -> Vec<&'a str> {
            let mut names: Vec<&str> = edges.iter().filter(|e| e.to == target.id()).filter_map(|e| snapshots.iter().find(|s| s.id() == e.from)).map(|s| s.name().as_str()).collect();
            names.sort_unstable();
            names
        }

        /// Case 1 (required): partial class, base on only ONE part,
        /// same file.
        #[test]
        fn partial_target_split_same_file_reconciles() {
            let (snapshots, contexts) = real_snapshots_and_contexts(
                repo(),
                &[("src/Case1.cs", "class Base {}\npartial class Foo : Base\n{\n}\npartial class Foo\n{\n}\n")],
            );
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let base = find(&snapshots, "Base");

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Foo"]);
            let to_base = edges.iter().find(|e| e.to == base.id()).unwrap();
            assert_eq!(to_base.kind, EdgeKind::Extends);
        }

        /// Mutation-testing-driven addition: verifies the reconciled
        /// edge's own `to` is EXACTLY the TARGET fragment with the
        /// lowest byte-span start (the documented tie-break), by
        /// comparing real `SymbolSnapshotId`s directly -- not just the
        /// shared name "Foo" every fragment has. This is what actually
        /// exercises the final `==` candidate-id lookup in
        /// `reconcile_partial_fragments`: every OTHER test in this
        /// module checks only the resolved NAME of the reconciled
        /// target, which cannot distinguish "picked the right
        /// fragment" from "picked some other same-named fragment,"
        /// since every fragment of one logical type shares one name by
        /// construction. Here `Foo` (the AMBIGUOUS, multi-fragment
        /// name) is the base clause's own TARGET, not the derived
        /// type -- the exact shape `reconcile_partial_fragments` is
        /// written to resolve.
        #[test]
        fn reconciled_edge_points_to_exactly_the_lowest_span_start_target_fragment() {
            let (snapshots, contexts) = real_snapshots_and_contexts(
                repo(),
                &[("src/Case1b.cs", "partial class Foo {}\npartial class Foo { }\nclass Derived : Foo {}\n")],
            );
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let derived = find(&snapshots, "Derived");
            let foo_fragments: Vec<&SymbolSnapshot> = snapshots.iter().filter(|s| s.name().as_str() == "Foo").collect();
            assert_eq!(foo_fragments.len(), 2);
            let lowest_span_fragment = foo_fragments.iter().min_by_key(|s| s.span().start).unwrap();

            let derived_extends = edges.iter().find(|e| e.from == derived.id()).unwrap();
            assert_eq!(derived_extends.to, lowest_span_fragment.id(), "the reconciled edge must point to exactly the lowest-span-start Foo fragment, not an arbitrary same-named one");
        }

        /// Case 2 (required): partial class, base split across
        /// different files.
        #[test]
        fn partial_target_split_across_files_reconciles() {
            let (snapshots, contexts) = real_snapshots_and_contexts(
                repo(),
                &[
                    ("src/Base.cs", "class Base {}\n"),
                    ("src/Foo.A.cs", "partial class Foo : Base\n{\n}\n"),
                    ("src/Foo.B.cs", "partial class Foo\n{\n}\n"),
                    ("src/Foo.C.cs", "partial class Foo\n{\n}\n"),
                ],
            );
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let base = find(&snapshots, "Base");

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Foo"], "exactly one Extends edge, even though Foo has 3 partial fragments across 3 files");
        }

        /// Case 3 (required): ALL candidates partial -> reconciliation
        /// (the real Polly shape — base split 3 ways, derived elsewhere).
        #[test]
        fn real_polly_shape_all_partial_fragments_reconciles() {
            let (snapshots, contexts) = real_snapshots_and_contexts(
                repo(),
                &[
                    ("src/Policy.cs", "public partial class Policy {}\n"),
                    ("src/Policy.TResult.cs", "public partial class Policy { }\n"),
                    ("src/Policy.HandleSyntax.cs", "public partial class Policy { }\n"),
                    ("src/IBulkheadPolicy.cs", "public interface IBulkheadPolicy {}\n"),
                    ("src/BulkheadPolicy.cs", "public class BulkheadPolicy : Policy, IBulkheadPolicy {}\n"),
                ],
            );
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let policy_fragments: Vec<&SymbolSnapshot> = snapshots.iter().filter(|s| s.name().as_str() == "Policy").collect();
            assert_eq!(policy_fragments.len(), 3, "Policy must still be 3 DISTINCT snapshots -- never merged");

            let bulkhead = find(&snapshots, "BulkheadPolicy");
            let extends: Vec<_> = edges.iter().filter(|e| e.from == bulkhead.id() && e.kind == EdgeKind::Extends).collect();
            assert_eq!(extends.len(), 1, "exactly one Extends edge must now exist toward one of Policy's 3 fragments");
            assert!(policy_fragments.iter().any(|p| p.id() == extends[0].to), "the Extends edge's target must be one of the real Policy fragments");

            let implements: Vec<_> = edges.iter().filter(|e| e.from == bulkhead.id() && e.kind == EdgeKind::Implements).collect();
            assert_eq!(implements.len(), 1, "IBulkheadPolicy (not partial, unambiguous) must still resolve normally");
        }

        /// Case 4 (required): ONE candidate non-partial -> NO
        /// reconciliation (mixed partial/non-partial stays ambiguous).
        #[test]
        fn one_non_partial_candidate_blocks_reconciliation() {
            // One partial Foo, one NON-partial Foo, both same
            // namespace/arity/kind -- a mix of partial and non-partial
            // same-named candidates must never be reconciled.
            let source = "namespace N {\n    partial class Foo {}\n    class Foo {}\n}\nclass Derived : Foo {}\n";
            let (snapshots, contexts) = real_snapshots_and_contexts(repo(), &[("src/Mixed.cs", source)]);
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();

            let foo_candidates: Vec<&SymbolSnapshot> = snapshots.iter().filter(|s| s.name().as_str() == "Foo").collect();
            assert_eq!(foo_candidates.len(), 2, "one partial, one non-partial Foo, both real");
            let derived = find(&snapshots, "Derived");
            let derived_extends: Vec<_> = edges.iter().filter(|e| e.from == derived.id()).collect();
            assert!(derived_extends.is_empty(), "a mix of partial and non-partial same-named candidates must NEVER be reconciled -- stays ambiguous, no edge");
        }

        /// Case 5 (required): same simple name, DIFFERENT namespace ->
        /// NO reconciliation, even if both are partial.
        #[test]
        fn same_name_different_namespace_both_partial_blocks_reconciliation() {
            let source = "namespace A {\n    partial class Foo {}\n}\nnamespace B {\n    partial class Foo {}\n}\nclass Derived : Foo {}\n";
            let (snapshots, contexts) = real_snapshots_and_contexts(repo(), &[("src/NsClash.cs", source)]);
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();

            let foo_candidates: Vec<&SymbolSnapshot> = snapshots.iter().filter(|s| s.name().as_str() == "Foo").collect();
            assert_eq!(foo_candidates.len(), 2, "two distinct partial Foo types in two different namespaces");
            let derived = find(&snapshots, "Derived");
            let derived_extends: Vec<_> = edges.iter().filter(|e| e.from == derived.id()).collect();
            assert!(derived_extends.is_empty(), "two partial Foo types in DIFFERENT namespaces are genuinely distinct types -- must never reconcile, per the exact counterexample that motivated adding namespace to this fix's own invariant");
        }

        /// Case 6 (required): same name, different generic arity -> NO
        /// reconciliation, even if both are partial.
        #[test]
        fn same_name_different_generic_arity_both_partial_blocks_reconciliation() {
            let source = "partial class Foo {}\npartial class Foo<T> {}\nclass Derived : Foo {}\n";
            let (snapshots, contexts) = real_snapshots_and_contexts(repo(), &[("src/Arity.cs", source)]);
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();

            let derived = find(&snapshots, "Derived");
            let derived_extends: Vec<_> = edges.iter().filter(|e| e.from == derived.id()).collect();
            assert!(derived_extends.is_empty(), "Foo and Foo<T> are different generic-arity types -- must never reconcile, this is the SEPARATE generic-arity-collision gap, out of scope for this fix");
        }

        /// Case 7 (required): partial generic class — both fragments
        /// of a GENERIC partial type still reconcile, as long as arity
        /// agrees across all fragments. Uses a NON-generic base
        /// (`Base`, not `Base<T>`) deliberately: a generic base type in
        /// a base clause (`Base<T>`) is a pre-existing, SEPARATE gap in
        /// `collect_csharp_base_list` (it only recognizes a bare
        /// `identifier` child of `base_list`, never a `generic_name`
        /// child — confirmed by direct grammar probe, present
        /// identically before and after this fix, exercised by no
        /// prior test in this file). This test isolates B2-ID-010's own
        /// claim (partial Foo<T> fragments reconcile when arity agrees)
        /// from that unrelated, pre-existing gap — not in this fix's
        /// scope to address.
        #[test]
        fn partial_generic_class_reconciles_when_arity_agrees() {
            let source = "class Base {}\npartial class Foo<T> : Base\n{\n}\npartial class Foo<T>\n{\n}\n";
            let (snapshots, contexts) = real_snapshots_and_contexts(repo(), &[("src/Generic.cs", source)]);
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let base = find(&snapshots, "Base");

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Foo"], "partial Foo<T> fragments (same arity) must still reconcile toward Base");
        }

        /// Case 8 (required): ordinary duplicate NON-partial
        /// declarations remain ambiguous — the baseline invariant this
        /// fix must never weaken.
        #[test]
        fn ordinary_non_partial_duplicate_names_remain_ambiguous() {
            // Two non-partial Foo classes (a genuine naming collision,
            // not a partial-class split) plus a Derived class naming
            // "Foo" as its own base -- the ambiguity to check is
            // "what does Derived extend", since Base itself is unique
            // and resolves correctly regardless (that would not
            // exercise this invariant at all).
            let source = "class Foo {}\nclass Foo {}\nclass Derived : Foo {}\n";
            let (snapshots, contexts) = real_snapshots_and_contexts(repo(), &[("src/Dup.cs", source)]);
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let derived = find(&snapshots, "Derived");

            let derived_extends: Vec<_> = edges.iter().filter(|e| e.from == derived.id()).collect();
            assert!(derived_extends.is_empty(), "two non-partial classes sharing the bare name Foo must remain genuinely ambiguous -- the reconciliation fix must not weaken this baseline invariant");
        }

        /// Case 9 (required): with empty contexts (the legacy-wrapper
        /// path), behavior is byte-for-byte identical to the
        /// non-reconciled result — confirms the compatibility wrapper's
        /// own guarantee directly against the real Polly-shaped
        /// ambiguity, not just a synthetic case.
        #[test]
        fn empty_contexts_produce_the_same_result_as_the_legacy_wrapper() {
            let (snapshots, contexts) = real_snapshots_and_contexts(
                repo(),
                &[
                    ("src/Policy.cs", "public partial class Policy {}\n"),
                    ("src/Policy.TResult.cs", "public partial class Policy { }\n"),
                    ("src/BulkheadPolicy.cs", "public class BulkheadPolicy : Policy {}\n"),
                ],
            );
            let _ = contexts;

            let via_wrapper = resolve_extends_implements(&snapshots).unwrap();
            let via_empty_contexts = resolve_extends_implements_with_csharp_contexts(&snapshots, &[]).unwrap();
            assert_eq!(via_wrapper, via_empty_contexts, "the legacy wrapper must be byte-for-byte equivalent to calling the new function with empty contexts");

            let bulkhead = find(&snapshots, "BulkheadPolicy");
            let bulkhead_extends: Vec<_> = via_wrapper.iter().filter(|e| e.from == bulkhead.id()).collect();
            assert!(bulkhead_extends.is_empty(), "WITHOUT contexts, the real Polly-shaped ambiguity must still produce no edge -- confirms this fix changes nothing unless contexts are actually supplied");
        }

        /// Case 10 (required): no DUPLICATE Extends/Implements edge is
        /// produced for the SAME (from, to, kind) triple — the
        /// pre-existing B2-ID-002 dedup guarantee, confirmed to still
        /// hold for a partial fragment whose own base clause names the
        /// SAME base name twice within its own one declaration (e.g. a
        /// syntactically odd `partial class Foo : Base, Base`). This is
        /// a DIFFERENT case from two DISTINCT fragments each separately
        /// naming Base in their OWN base clause — that legitimately
        /// produces two edges (one per real `from` fragment), which is
        /// correct, not a duplicate: each fragment really does declare
        /// its own, textually separate inheritance relationship.
        #[test]
        fn same_base_named_twice_in_one_partial_fragments_own_clause_produces_one_edge_not_two() {
            let source = "class Base {}\npartial class Foo : Base, Base\n{\n}\npartial class Foo\n{\n}\n";
            let (snapshots, contexts) = real_snapshots_and_contexts(repo(), &[("src/RepeatedBase.cs", source)]);
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let base = find(&snapshots, "Base");

            let to_base: Vec<_> = edges.iter().filter(|e| e.to == base.id()).collect();
            assert_eq!(to_base.len(), 1, "Base named twice in ONE fragment's own base clause must still collapse to one edge, per the pre-existing B2-ID-002 dedup rule");
        }

        /// Two DISTINCT partial fragments, each independently naming
        /// the same base in their OWN base clause, correctly produce
        /// TWO edges (one per real fragment) — confirming this is not
        /// mistaken for a duplicate by this fix's own reconciliation
        /// logic (reconciliation only ever changes which TARGET an
        /// ambiguous lookup resolves to; it never collapses two
        /// genuinely separate `from` declarations into one).
        #[test]
        fn two_distinct_fragments_each_naming_the_same_base_produce_two_real_edges() {
            let source = "class Base {}\npartial class Foo : Base\n{\n}\npartial class Foo : Base\n{\n}\n";
            let (snapshots, contexts) = real_snapshots_and_contexts(repo(), &[("src/TwoFragments.cs", source)]);
            let edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let base = find(&snapshots, "Base");

            let to_base: Vec<_> = edges.iter().filter(|e| e.to == base.id()).collect();
            assert_eq!(to_base.len(), 2, "two DISTINCT partial fragments, each with their own real base clause naming Base, must each produce their own edge -- not a duplicate, and not reconciled away");
            let from_ids: std::collections::HashSet<_> = to_base.iter().map(|e| e.from).collect();
            assert_eq!(from_ids.len(), 2, "the two edges must come from the two DIFFERENT fragment snapshots");
        }

        /// Determinism: the reconciliation's own representative choice
        /// must not depend on snapshot pool ordering.
        #[test]
        fn reconciliation_result_is_deterministic_regardless_of_snapshot_order() {
            let (snapshots, contexts) = real_snapshots_and_contexts(
                repo(),
                &[
                    ("src/Base.cs", "class Base {}\n"),
                    ("src/Foo.A.cs", "partial class Foo : Base\n{\n}\n"),
                    ("src/Foo.B.cs", "partial class Foo\n{\n}\n"),
                ],
            );
            let base = find(&snapshots, "Base");

            let forward_edges = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let mut reversed = snapshots.clone();
            reversed.reverse();
            let reversed_edges = resolve_extends_implements_with_csharp_contexts(&reversed, &contexts).unwrap();

            assert_eq!(sources_of(&forward_edges, base, &snapshots), sources_of(&reversed_edges, base, &reversed));
            let forward_target = forward_edges.iter().find(|e| e.to == base.id()).map(|e| e.from);
            let reversed_target = reversed_edges.iter().find(|e| e.to == base.id()).map(|e| e.from);
            assert_eq!(forward_target, reversed_target, "the chosen representative fragment must be the same regardless of snapshot pool ordering");
        }

        /// No snapshot identity change: reconciliation never alters,
        /// merges, or removes any `SymbolSnapshotId` — every fragment
        /// remains a fully independent snapshot before and after this
        /// resolver runs.
        #[test]
        fn reconciliation_never_changes_snapshot_identity() {
            let (snapshots, contexts) = real_snapshots_and_contexts(
                repo(),
                &[
                    ("src/Policy.cs", "public partial class Policy {}\n"),
                    ("src/Policy.TResult.cs", "public partial class Policy { }\n"),
                    ("src/BulkheadPolicy.cs", "public class BulkheadPolicy : Policy {}\n"),
                ],
            );
            let ids_before: std::collections::HashSet<SymbolSnapshotId> = snapshots.iter().map(|s| s.id()).collect();
            let _ = resolve_extends_implements_with_csharp_contexts(&snapshots, &contexts).unwrap();
            let ids_after: std::collections::HashSet<SymbolSnapshotId> = snapshots.iter().map(|s| s.id()).collect();
            assert_eq!(ids_before, ids_after, "resolve_extends_implements_with_csharp_contexts must never mutate or merge any snapshot's own identity");
            assert_eq!(snapshots.len(), 3, "all 3 original fragments/types remain distinct, independent snapshots");
        }
    }
}
