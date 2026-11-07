//! B2 Task 7: `Calls` edges. Walks each caller `SymbolSnapshot`'s own
//! `signature_text` (which, per `extract`'s per-language extractors,
//! captures the WHOLE construct — including its body — not just a
//! parameter list) by re-parsing it as a standalone fragment with the
//! same language's `LanguageParser`, and looks for call-expression nodes
//! whose callee name resolves to another snapshot in the given pool.
//!
//! # Resolved vs. heuristic vs. unresolved
//!
//! - A call whose callee name matches EXACTLY ONE other snapshot's name
//!   is `StaticAnalysis`-resolved — not because the whole program's
//!   call graph is statically analyzed (it is not: this pipeline does
//!   not do type inference, overload resolution, or virtual-dispatch
//!   analysis), but because, within the closed set of candidate
//!   snapshots this resolver is given, the name uniquely identifies one
//!   target with no ambiguity left for this resolver to guess at. A
//!   dotted/member call (Python's `self.encoder(x)`, matched by its
//!   final segment `encoder`) is resolved the SAME way once the name
//!   is unique — the pipeline does not need to know `self` is an
//!   instance to usefully record "this method calls something named
//!   `encoder`."
//! - A call whose callee name matches MORE THAN ONE candidate (e.g. two
//!   different types both define a method named `run`) is
//!   `NameHeuristic`, `Confidence::Speculative` — explicitly never
//!   picked arbitrarily. Per the task's own scope, this resolver does
//!   NOT attempt overload/type-based disambiguation; it records the
//!   ambiguity honestly rather than overclaiming a single target.
//! - A call whose callee name matches NO candidate in the given pool
//!   produces no edge at all — the callee is outside what this resolver
//!   was given to work with (e.g. a standard-library function, or a
//!   symbol in a file not included in this resolution pass), not a
//!   relationship this pipeline can honestly assert anything about.
//!
//! # Known gap: calls inside a macro invocation (Rust)
//!
//! A call wrapped inside a Rust macro invocation (e.g.
//! `assert_eq!(add(1, 2), 3)`) is NOT found by this resolver: confirmed
//! empirically against a real parse, `add(1, 2)` inside
//! `assert_eq!(...)`'s argument list is an opaque `token_tree` to
//! tree-sitter, not a real `call_expression` node — tree-sitter does not
//! expand macros, so it has no way to know `assert_eq!`'s arguments are
//! themselves Rust expressions rather than arbitrary tokens. This
//! resolver therefore cannot see calls made from inside macro-heavy test
//! code (a common shape, since `assert_eq!`/`assert!` are exactly where
//! a test's real assertions live) without a macro-aware token-tree
//! heuristic this task's scope does not include. Recorded here as a
//! real, named limitation — not silently masked by a test fixture that
//! happens to avoid it — and a candidate for the real-repository
//! validation pass this plan's section 8b requires before B2's closure.

use codegraph_domain::{Confidence, EdgeKind, SymbolKind};

use super::resolution::{heuristic_evidence, resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
pub(crate) use crate::parse::parser_for;
use crate::parse::Language;
use crate::snapshot::SymbolSnapshot;

/// Resolves `Calls` edges from every snapshot in `snapshots` to every
/// other snapshot in the SAME pool whose name matches a call found
/// inside the caller's own signature text. `snapshots` plays both the
/// "source" and "candidate target" role simultaneously — a caller and
/// its potential callees are expected to be given together, unlike
/// Task 6's lineage resolver's `prev`/`next` split (lineage is
/// inherently directional across commits; calls are not).
pub fn resolve_calls(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    for caller in lookup.snapshots() {
        // B2-ID-006, mechanism 1 of 2 (caller eligibility): a snapshot
        // whose own `SymbolKind` is a structural container (`Class`,
        // `Struct`, `Interface`, ...) is never itself a real caller —
        // see `SymbolKind::can_originate_calls`'s own doc comment for
        // the full, per-variant justification. Skipping here, before
        // ever re-parsing `caller.signature()`, is both correct (a
        // container's own body is never itself "executed") and cheaper
        // than walking its full body only to find nothing new once
        // mechanism 2 below would otherwise exclude every call inside
        // it anyway.
        if !caller.kind().can_originate_calls() {
            continue;
        }

        let Some(parser) = parser_for(caller.file()) else {
            // This resolver only recognizes languages whose
            // FileIdentity-independent kind it can determine from the
            // caller's own promoted SymbolKind/signature shape — in
            // practice, every language Task 4 extracts already has a
            // LanguageParser; see `parser_for`'s own doc comment for
            // why HTML/CSS snapshots never reach a parser here.
            continue;
        };

        let Ok(tree) = parser.parse(caller.signature().as_str().as_bytes()) else {
            continue;
        };

        // B2-ID-002: `call_names_in_tree` returns one entry per
        // SYNTACTIC call-site occurrence (e.g. `self.forward(x)` called
        // twice in the same body yields two entries), but `CodeGraph`
        // represents a SEMANTIC/topological `(from, to, EdgeKind)`
        // relation — at most one edge per pair, never one per
        // occurrence. `per_target` collapses every occurrence resolving
        // to the SAME target into exactly one `Edge`, chosen
        // deterministically by `prefer_edge` (see its own doc comment)
        // rather than by iteration order, before this caller's edges
        // are appended to the final output. This never merges two
        // DISTINCT targets — only repeated occurrences of the same
        // `(from, to)` pair.
        let mut per_target: std::collections::HashMap<codegraph_domain::SymbolSnapshotId, Edge> = std::collections::HashMap::new();

        for call_site in call_sites_in_tree(&tree) {
            // B2-ID-006, mechanism 2 of 2 (nested-scope boundary): this
            // call site's ABSOLUTE file position is `caller`'s own span
            // start plus the call's position within the re-parsed
            // fragment (`caller.signature()`'s text starts exactly at
            // `caller.span().start` — see `SymbolSnapshot::span`'s own
            // doc comment). If some OTHER caller-eligible snapshot,
            // strictly nested inside `caller`'s own span in the same
            // file, ALSO strictly contains this absolute position, that
            // nested snapshot — not `caller` — is this call site's real
            // owner (it will itself be visited as its own `caller` in a
            // later iteration of this same outer loop, and will
            // correctly attribute this call site to itself there).
            // Skipping it here is what prevents an outer Function/Class
            // from rediscovering a call that genuinely belongs to a
            // nested Function/Method — the SAME span-nesting fact
            // `resolve_contains` already uses for `Contains` edges,
            // reused here rather than inventing a second nesting
            // mechanism or any synthetic scope name.
            let absolute_call_site_position = caller.span().start + call_site.fragment_relative_start_byte;
            if call_site_belongs_to_a_more_nested_caller(lookup.snapshots(), caller, absolute_call_site_position) {
                continue;
            }

            let callee_name = call_site.callee_name;
            let all_candidates = lookup.find_by_name(&callee_name, caller.id());

            // Task 9.5, Priority 5(A), Case A: a `self.field.method()`-
            // shaped call (receiver is the caller's OWN instance, and
            // `field` is a known member of the caller's enclosing
            // class) must not be matched against an UNRELATED
            // same-named candidate elsewhere in the corpus just because
            // the bare method name happens to collide (e.g.
            // `self.src_mask.size(0)` — a call on a `Field` named
            // `src_mask` — must never resolve to an unrelated class's
            // unrelated `size()` method). When the receiver's field is
            // a real member of the caller's own class (confirmed
            // structurally via span-containment, the same signal
            // `resolve_contains` uses — never a name hardcoded to this
            // corpus), restrict the candidate pool to members of THAT
            // SAME class; if that class has no matching member,
            // "unresolved" is preferred over a cross-class false match,
            // per the task's explicit instruction, so no candidates
            // survive the restriction and no edge is produced for this
            // occurrence. When the receiver field is NOT a member of
            // the caller's class (or the caller has no enclosing class,
            // or no receiver was present at all), candidate selection is
            // unchanged from before this fix.
            let candidates = match &call_site.self_receiver_field {
                Some(field_name) if receiver_field_is_a_known_member(lookup.snapshots(), caller, field_name) => {
                    restrict_to_callers_own_class(lookup.snapshots(), caller, &all_candidates)
                }
                _ => all_candidates,
            };

            if candidates.len() == 1 {
                let evidence = resolved_evidence(&format!("call to {callee_name:?} resolves to exactly one candidate in the given snapshot pool"));
                let edge = Edge {
                    from: caller.id(),
                    to: candidates[0].id(),
                    kind: EdgeKind::Calls,
                    evidence,
                };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
                continue;
            }
            // Zero candidates: the callee is outside what this resolver
            // was given to work with (see this module's doc comment) —
            // "unresolved: no candidate exists" (tracked as a distinct
            // outcome from ambiguity, per the Evidence-before-claims
            // Experimental Policy), and the logic below naturally
            // produces no edges for an empty `candidates`.
            //
            // More than one candidate: Task 9.5, Priority 5(A), Case B.
            // Previously (CALL-RES-A0, Task 7's original design — see
            // `docs/research/EXPERIMENTAL_POLICY.md`), this resolver
            // recorded ONE Hypothesized edge per ambiguous candidate,
            // corpus-wide, with no scope narrowing at all. Measured on a
            // real 97-file corpus (B2-EVAL-001), this produced a 19-for-
            // 19 false-positive rate asking "who calls this `forward`"
            // on a commonly-named method — every one of 19 incoming
            // edges was confirmed, by manual source inspection, to be an
            // unrelated same-named method in a different file/class.
            //
            // The fix (CALL-RES-A1 + A2): narrow `candidates` through a
            // scope hierarchy — same enclosing class as `caller`, then
            // same file as `caller` — BEFORE deciding. At whichever
            // level is narrowest while still non-empty: exactly one
            // candidate there is accepted as a scoped heuristic edge
            // (still not `StaticAnalysis` — this is still a name-based
            // guess, just a much better-scoped one); more than one
            // candidate, even at the narrowest non-empty level, is
            // recorded as "unresolved: ambiguity remains" and produces
            // NO edge — never an arbitrary pick, and never N Hypothesized
            // edges for targets this resolver cannot actually
            // distinguish. This is itself an experimental configuration
            // (see the policy doc), not a provably optimal rule.
            if let Some((scoped_candidate, scope_description)) = narrow_by_scope(lookup.snapshots(), caller, &candidates) {
                let (evidence, _confidence) = heuristic_evidence(
                    codegraph_domain::ResolutionMethod::NameHeuristic,
                    Confidence::Speculative,
                    &format!("call to {callee_name:?} resolves to exactly one candidate once narrowed to {scope_description}"),
                );
                let edge = Edge {
                    from: caller.id(),
                    to: scoped_candidate.id(),
                    kind: EdgeKind::Calls,
                    evidence,
                };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
            }
            // Else: ambiguity remains even at the narrowest non-empty
            // scope level — no edge is produced for this occurrence,
            // per CALL-RES-A1's "no edge beats N speculative edges" rule.
        }

        let mut caller_edges: Vec<Edge> = per_target.into_values().collect();
        caller_edges.sort_by(|a, b| a.to.as_bytes().cmp(b.to.as_bytes()));
        edges.extend(caller_edges);
    }

    Ok(edges)
}

/// Task 9.5, Priority 5(A): the `Class` snapshot that structurally
/// contains `symbol`, by the SAME span-nesting rule `resolve_contains`
/// (this crate's own `Contains` edge resolver) uses: same file,
/// `Class`-kind, `symbol`'s span strictly nested inside the class's
/// span, innermost (smallest) class chosen when more than one
/// candidate class contains it. Deliberately re-derived here rather
/// than depending on `resolve_contains`'s own `Edge` output: resolvers
/// in this module each take a flat snapshot pool and produce `Edge`s
/// independently (see `TargetLookup`'s own doc comment — no resolver
/// may depend on another resolver's edges, only on the shared snapshot
/// pool and `SymbolSnapshot`'s own structural fields), so this stays a
/// pool-level query rather than introducing a cross-resolver ordering
/// dependency.
fn enclosing_class_of<'a>(snapshots: &'a [SymbolSnapshot], symbol: &SymbolSnapshot) -> Option<&'a SymbolSnapshot> {
    let mut innermost: Option<&SymbolSnapshot> = None;
    for candidate_class in snapshots {
        if candidate_class.id() == symbol.id() || candidate_class.kind() != SymbolKind::Class {
            continue;
        }
        if !same_file(candidate_class, symbol) || !strictly_contains(candidate_class, symbol) {
            continue;
        }
        innermost = Some(match innermost {
            None => candidate_class,
            Some(current) if span_len(candidate_class) < span_len(current) => candidate_class,
            Some(current) => current,
        });
    }
    innermost
}

/// B2-ID-006, mechanism 2 of 2: `true` if some snapshot OTHER than
/// `caller` is (a) caller-eligible, (b) in the same file as `caller`,
/// (c) strictly nested inside `caller`'s own span, and (d) itself
/// strictly contains `absolute_call_site_position` — meaning THAT
/// nested snapshot, not `caller`, is the call site's real nearest
/// executable owner. When more than one such nested snapshot exists
/// (a doubly-nested function, for instance), any one of them being
/// found is sufficient to exclude `caller`: `caller` is never the
/// nearest owner once ANY more deeply nested eligible snapshot also
/// contains the position, regardless of how many levels of nesting
/// separate them — the INNERMOST one will itself independently decide
/// whether IT has an even-more-nested owner, in its own iteration of
/// `resolve_calls`'s outer loop.
fn call_site_belongs_to_a_more_nested_caller(snapshots: &[SymbolSnapshot], caller: &SymbolSnapshot, absolute_call_site_position: usize) -> bool {
    snapshots.iter().any(|candidate| {
        candidate.id() != caller.id()
            && candidate.kind().can_originate_calls()
            && same_file(candidate, caller)
            && strictly_contains(caller, candidate)
            && span_contains_position(candidate.span(), absolute_call_site_position)
    })
}

pub(super) fn span_contains_position(span: crate::extract::ByteSpan, position: usize) -> bool {
    span.start <= position && position < span.end
}

pub(super) fn same_file(a: &SymbolSnapshot, b: &SymbolSnapshot) -> bool {
    a.file().repository() == b.file().repository() && a.file().commit() == b.file().commit() && a.file().relative_path() == b.file().relative_path()
}

pub(super) fn strictly_contains(outer: &SymbolSnapshot, inner: &SymbolSnapshot) -> bool {
    let outer_span = outer.span();
    let inner_span = inner.span();
    outer_span.start <= inner_span.start && inner_span.end <= outer_span.end && (outer_span.start, outer_span.end) != (inner_span.start, inner_span.end)
}

fn span_len(snapshot: &SymbolSnapshot) -> usize {
    snapshot.span().end - snapshot.span().start
}

/// `true` if `field_name` names a real member (any kind — `Field`,
/// `Method`, or `Function`, matching `resolve_contains`'s own
/// containment scope) of `caller`'s own enclosing class. This is the
/// gate for applying Case A's receiver-aware restriction at all: a
/// `self.X` reference where `X` is NOT a real member of the caller's
/// class is not something this resolver can honestly reason about
/// structurally (e.g. a dynamically-added attribute, or simply wrong
/// input), so no restriction is applied for it — "no behavior change"
/// is the safe default when the structural premise does not hold.
fn receiver_field_is_a_known_member(snapshots: &[SymbolSnapshot], caller: &SymbolSnapshot, field_name: &str) -> bool {
    let Some(class) = enclosing_class_of(snapshots, caller) else {
        return false;
    };
    snapshots.iter().any(|s| {
        s.id() != class.id()
            && matches!(s.kind(), SymbolKind::Field | SymbolKind::Method | SymbolKind::Function)
            && s.name().as_str() == field_name
            && same_file(class, s)
            && strictly_contains(class, s)
    })
}

/// Restricts `candidates` (every snapshot matching the called method's
/// bare name, corpus-wide) down to the ones that are themselves members
/// of `caller`'s own enclosing class. If `caller` has no enclosing
/// class, or none of `candidates` belong to it, returns an EMPTY list
/// deliberately — per the task's explicit "unresolved is preferable to
/// false positive" instruction, this is not a fallback to the
/// unrestricted set; a `self.field.method()` call whose matching
/// methods are all OUTSIDE the caller's own class is exactly the false-
/// positive shape this fix exists to prevent (e.g. `self.src_mask.size(0)`
/// must not fall back to matching two unrelated C++ `size()` methods
/// just because no `size` member exists on the caller's own class —
/// that absence is itself evidence the call is on an external/
/// unresolvable type, not a reason to guess more broadly).
fn restrict_to_callers_own_class<'a>(snapshots: &[SymbolSnapshot], caller: &SymbolSnapshot, candidates: &[&'a SymbolSnapshot]) -> Vec<&'a SymbolSnapshot> {
    let Some(class) = enclosing_class_of(snapshots, caller) else {
        return Vec::new();
    };
    candidates.iter().copied().filter(|c| same_file(class, c) && strictly_contains(class, c)).collect()
}

/// Task 9.5, Priority 5(A), Case B: progressively narrows an ambiguous
/// (`len() > 1`) `candidates` list down a scope hierarchy — same
/// enclosing class as `caller`, then same file as `caller` — returning
/// the single surviving candidate (and a short description of which
/// scope level resolved it, for the edge's own evidence note) the
/// FIRST time exactly one candidate remains at a non-empty level.
///
/// Per the Evidence-before-claims Experimental Policy
/// (`docs/research/EXPERIMENTAL_POLICY.md`), this is CALL-RES-A2: an
/// experimental scoping configuration, not a proven-optimal rule. Each
/// level is tried in order; a level with ZERO candidates falls through
/// to the next (it narrowed too far, not far enough), while a level
/// with MORE THAN ONE candidate stops the search entirely — ambiguity
/// within the narrowest scope that has any candidates is not resolved
/// by trying an even broader scope (that would only ever make the
/// candidate set larger, never smaller). `None` means every level tried
/// either was empty or stayed ambiguous — the caller must then record
/// "unresolved: ambiguity remains" and produce no edge.
///
/// Import/visibility-aware narrowing (Priority 6 and beyond,
/// CALL-RES-A4) is a planned FUTURE level between "same file" and
/// "repo-wide," not implemented here — no import-resolution data exists
/// in this resolver yet to narrow by.
fn narrow_by_scope<'a>(snapshots: &[SymbolSnapshot], caller: &SymbolSnapshot, candidates: &[&'a SymbolSnapshot]) -> Option<(&'a SymbolSnapshot, &'static str)> {
    if let Some(class) = enclosing_class_of(snapshots, caller) {
        let same_class: Vec<&SymbolSnapshot> = candidates.iter().copied().filter(|c| same_file(class, c) && strictly_contains(class, c)).collect();
        match same_class.len() {
            0 => {} // Narrowed too far — fall through to the next level.
            1 => return Some((same_class[0], "the caller's own enclosing class")),
            _ => return None, // Ambiguous even within the caller's own class.
        }
    }

    let same_file_candidates: Vec<&SymbolSnapshot> = candidates.iter().copied().filter(|c| same_file(caller, c)).collect();
    // The `0` arm is written explicitly (naming "no same-file
    // candidate at all" distinctly from "ambiguous among several"),
    // even though both it and `_` currently return the same `None` —
    // kept apart for readability, not because mutation testing could
    // distinguish them. (Task 9.5's closing mutation gate confirmed
    // deleting the `0` arm is an EQUIVALENT mutant BY CONSTRUCTION: with
    // it gone, `0` falls into `_ => None`, the exact same return value,
    // so no input can ever tell the two arms apart as written today.
    // Left this way deliberately — do not "simplify" by merging `0` and
    // `_`; doing so would only remove a semantic distinction this
    // function may want to keep observable if a future caller needs to
    // tell "no candidate" apart from "ambiguous.")
    match same_file_candidates.len() {
        0 => None, // No candidate even shares the caller's file — repo-wide ambiguity is never auto-resolved (CALL-RES-A1).
        1 => Some((same_file_candidates[0], "the caller's own file")),
        _ => None, // Ambiguous even within the caller's own file.
    }
}

/// Inserts `edge` into `per_target` (keyed by `edge.to`), or, if a
/// different occurrence already produced an edge to the SAME target,
/// keeps whichever one `prefer_edge` ranks higher — deterministically,
/// never by insertion/iteration order. This is the one place
/// `resolve_calls` collapses repeated call-site occurrences of the same
/// `(from, to)` pair into a single stored `Edge` (B2-ID-002).
fn upsert_preferring_stronger_evidence(per_target: &mut std::collections::HashMap<codegraph_domain::SymbolSnapshotId, Edge>, edge: Edge) {
    match per_target.get(&edge.to) {
        Some(existing) if !prefer_edge(&edge, existing) => {}
        _ => {
            per_target.insert(edge.to, edge);
        }
    }
}

/// Deterministic edge-preference rule for B2-ID-002's deduplication:
/// `true` if `candidate` should replace `current` for the same `(from,
/// to)` pair. `StaticAnalysis` always outranks a heuristic method
/// (resolved-unique beats ambiguous, which can only happen if the SAME
/// callee name resolved uniquely on one occurrence but, implausibly,
/// ambiguously on another within the same caller — not reachable today
/// since a given name's candidate set does not change within one
/// `resolve_calls` call, but handled explicitly rather than left
/// unreachable-and-unproven). When both sides have the same
/// `ResolutionMethod`, this scope's `Edge`/`Evidence` carry no other
/// field meaningfully distinguishing "better," so the choice is a
/// stable, explicit tie-break (note text, byte-for-byte) rather than
/// "whichever happened to be inserted first" — never a silent, order-
/// dependent pick.
fn prefer_edge(candidate: &Edge, current: &Edge) -> bool {
    let candidate_is_static = candidate.evidence.method() == codegraph_domain::ResolutionMethod::StaticAnalysis;
    let current_is_static = current.evidence.method() == codegraph_domain::ResolutionMethod::StaticAnalysis;
    if candidate_is_static != current_is_static {
        return candidate_is_static;
    }
    candidate.evidence.note() < current.evidence.note()
}

/// Every callee name found anywhere inside `tree`'s root, by walking for
/// the per-language call-expression node kind and extracting its own
/// callee child's name — `identifier` for a free/simple call, or the
/// LAST segment of an `attribute`/member-access chain for a method-call
/// shape (`self.encoder(x)` -> `"encoder"`, confirmed against this
/// workspace's real `tree_sitter_python` output before writing this
/// logic, not assumed).
pub(super) fn call_names_in_tree(tree: &crate::parse::ParseTree) -> Vec<String> {
    call_sites_in_tree(tree).into_iter().map(|site| site.callee_name).collect()
}

/// One syntactic call-site occurrence: the callee name being called,
/// plus (Task 9.5, Priority 5(A)) the `self`/`this`-qualified receiver
/// FIELD name, when the call is of the shape `self.field.method(...)`/
/// `this.field.method(...)` — `None` for a bare call (`helper()`), a
/// single-segment receiver call (`self.method()`), or a call through a
/// receiver this pipeline cannot resolve the type of (a plain local
/// variable, parameter, or module alias — no type inference here, so no
/// guess is made; see `self_or_this_receiver_field_name`'s own doc
/// comment for why this is deliberately conservative).
pub(super) struct CallSite {
    pub(super) callee_name: String,
    pub(super) self_receiver_field: Option<String>,
    /// B2-ID-006: the call expression's own start byte, relative to the
    /// re-parsed FRAGMENT text (`caller.signature()`'s own bytes, NOT
    /// the whole file) — i.e. exactly `tree_sitter::Node::start_byte()`
    /// on the fragment's own parse tree. `resolve_calls` converts this
    /// to an absolute file position by adding `caller.span().start`
    /// before comparing it against any other snapshot's own absolute
    /// `ByteSpan`, since every `SymbolSnapshot::span()` is a whole-file
    /// offset (see `extract::candidate::ByteSpan`'s own doc comment)
    /// while this fragment-relative offset is only meaningful paired
    /// with the specific `caller` it was produced from.
    pub(super) fragment_relative_start_byte: usize,
}

pub(super) fn call_sites_in_tree(tree: &crate::parse::ParseTree) -> Vec<CallSite> {
    let mut sites = Vec::new();
    walk_for_calls(tree.tree().root_node(), tree.source(), tree.language(), &mut sites);
    sites
}

fn walk_for_calls(node: tree_sitter::Node, source: &[u8], language: Language, out: &mut Vec<CallSite>) {
    let is_call_node = match language {
        Language::Rust => node.kind() == "call_expression",
        Language::Python => node.kind() == "call",
        Language::CSharp => node.kind() == "invocation_expression",
        Language::Cpp => node.kind() == "call_expression",
        Language::JavaScript | Language::TypeScript | Language::Tsx => node.kind() == "call_expression",
        Language::Html | Language::Css => false,
    };
    if is_call_node {
        if let Some(callee_name) = callee_name(node, source) {
            let self_receiver_field = node.child(0).and_then(|callee| self_or_this_receiver_field_name(callee, source));
            out.push(CallSite {
                callee_name,
                self_receiver_field,
                fragment_relative_start_byte: node.start_byte(),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_calls(child, source, language, out);
    }
}

/// `call_node`'s own first named child is the callee expression: an
/// `identifier` for a simple call, or an `attribute`/`field_expression`/
/// `member_expression`/`member_access_expression`-shaped member access
/// for a method call, whose own LAST `identifier`/`property_identifier`/
/// `field_identifier` child is the method name actually being called
/// (`self.encoder` -> `"encoder"`, not `"self"`). `member_access_expression`
/// (C#'s own node kind for `this.srcMask.Size`, confirmed via a direct
/// `tree-sitter-c-sharp` parse dump during Task 9.5's Priority 5(A)
/// investigation) was previously missing from this match arm — every
/// other language's equivalent node kind was already handled, so a C#
/// receiver-qualified call silently extracted no callee name at all,
/// confirmed as a real, pre-existing gap, not hypothetical.
fn callee_name(call_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let callee = call_node.child(0)?;
    match callee.kind() {
        "identifier" => Some(text_of(callee, source)),
        "attribute" | "field_expression" | "member_expression" | "member_access_expression" => {
            let mut cursor = callee.walk();
            let last_name_child = callee
                .children(&mut cursor)
                .filter(|c| matches!(c.kind(), "identifier" | "property_identifier" | "field_identifier"))
                .last()?;
            Some(text_of(last_name_child, source))
        }
        _ => None,
    }
}

/// `true` if `callee`'s node kind is one of the receiver-qualified
/// attribute/member-access shapes this module recognizes (Python's
/// `attribute`, Rust/C++'s `field_expression`, JS/TS's
/// `member_expression`, C#'s `member_access_expression`) — i.e. the
/// call has a RECEIVER at all, as opposed to a bare `identifier` call
/// with no receiver.
fn is_receiver_qualified(callee: tree_sitter::Node) -> bool {
    matches!(callee.kind(), "attribute" | "field_expression" | "member_expression" | "member_access_expression")
}

/// For a receiver-qualified callee node (`recv.field.method(...)`'s
/// `recv.field.method` node), returns the RECEIVER segment's own name —
/// `"src_mask"` for `self.src_mask.size`, the segment immediately before
/// the final (method-name) segment — if, and only if, the receiver is
/// literally `self` (Python/Rust) or `this` (C#/C++/JS/TS). Confirmed
/// via real parse dumps across all 6 grammars (Task 9.5, Priority 5(A))
/// that a receiver-qualified callee with more than one segment is
/// ALWAYS a nested attribute/member node one level down (e.g. Python:
/// outer `attribute["self.src_mask.size"]` containing inner
/// `attribute["self.src_mask"]`), never a flat multi-child list — so the
/// receiver segment is this inner node, and its own `self`/`this`-ness
/// is checked on ITS first child.
///
/// Returns `None` for a non-`self`/`this` receiver (a plain local
/// variable or module alias) — deliberately: this pipeline has no type
/// inference, so "what class does this local variable's value belong
/// to" is not an honestly answerable question yet, and inventing an
/// answer would be exactly the kind of guess Task 9.5 was told to avoid.
/// Only the `self`/`this` case is resolvable today, because it names the
/// CALLER's OWN enclosing scope, which this resolver can determine
/// directly from the snapshot pool's own byte spans (see
/// `enclosing_class_of`).
fn self_or_this_receiver_field_name(callee: tree_sitter::Node, source: &[u8]) -> Option<String> {
    if !is_receiver_qualified(callee) {
        return None;
    }
    let receiver_node = callee.child(0)?;
    if !is_receiver_qualified(receiver_node) {
        // A single-segment receiver (`self.method()`, no intermediate
        // field) has no distinct field name to extract — not this
        // function's concern; `callee_name` already resolves that case
        // correctly on its own.
        return None;
    }
    let mut cursor = receiver_node.walk();
    let mut children = receiver_node.children(&mut cursor);
    let base = children.next()?;
    // Rust has a dedicated `self` node kind; C#/C++/JS/TS have a
    // dedicated `this` node kind (all confirmed via real parse dumps,
    // Task 9.5 Priority 5(A)). Python has NEITHER — `self` is an
    // ordinary parameter name with no special grammar status, so it
    // parses as a plain `identifier` node whose TEXT happens to be
    // "self" (confirmed the same way: a real `tree_sitter_python` parse
    // dump of `self.src_mask.size` shows `identifier ["self"]`, not a
    // distinct node kind). Checking both the node kind (for the
    // languages that have one) and the literal text "self" (for
    // Python, where that is the only signal available) covers every
    // language this resolver supports without assuming Python works
    // like the others.
    let is_self_or_this = matches!(base.kind(), "self" | "this") || (base.kind() == "identifier" && text_of(base, source) == "self");
    if !is_self_or_this {
        return None;
    }
    let mut cursor = receiver_node.walk();
    receiver_node
        .children(&mut cursor)
        .filter(|c| matches!(c.kind(), "identifier" | "property_identifier" | "field_identifier"))
        .last()
        .map(|n| text_of(n, source))
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

    /// Regression for B2-ID-003's investigation: a chained call
    /// (`self._generate_square_subsequent_mask(len(src)).to(device)`)
    /// must resolve the inner, chained call to its target exactly like
    /// any other call site. B2-ID-003 confirmed this was ALREADY
    /// correct in `resolve_calls` itself — the apparent false negative
    /// originally reported was an evaluation-harness subject-selection
    /// bug, not a defect here — but no permanent regression test
    /// existed to pin that down, so this one does.
    #[test]
    fn a_chained_call_resolves_the_inner_call_to_its_target() {
        let forward_body = "def forward(self, src, has_mask=True):\n        if has_mask:\n            device = src.device\n            if self.src_mask is None or self.src_mask.size(0) != len(src):\n                mask = self._generate_square_subsequent_mask(len(src)).to(device)\n                self.src_mask = mask\n        else:\n            self.src_mask = None\n\n        src = self.input_emb(src) * math.sqrt(self.ninp)\n        src = self.pos_encoder(src)\n        output = self.encoder(src, mask=self.src_mask)\n        output = self.decoder(output)\n        return F.log_softmax(output, dim=-1)";
        let forward = test_snapshot("src/model.py", "forward", "function_definition", forward_body);
        let gssm = test_snapshot(
            "src/model.py",
            "_generate_square_subsequent_mask",
            "function_definition",
            "def _generate_square_subsequent_mask(self, sz):\n        mask = (torch.triu(torch.ones(sz, sz)) == 1).transpose(0, 1)\n        mask = mask.float().masked_fill(mask == 0, float('-inf')).masked_fill(mask == 1, float(0.0))\n        return mask",
        );
        let snapshots = vec![forward.clone(), gssm.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(
            edges.iter().any(|e| e.from == forward.id() && e.to == gssm.id()),
            "the chained call's inner callee must resolve to its target"
        );
    }

    /// Like `test_snapshot`, but all snapshots built from the SAME
    /// `repo` the caller passes in — needed when a test actually
    /// inserts into a `CodeGraph`, which rejects a cross-repository
    /// edge (Task 8's own invariant); `test_snapshot` alone mints a
    /// fresh, unrelated `RepositoryId` on every call, which is fine for
    /// every OTHER test in this file (none of which touches
    /// `CodeGraph`) but would trip that invariant here.
    fn test_snapshot_in_repo(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    /// Like `test_snapshot_in_repo`, but with an explicit `ByteSpan` —
    /// needed for Priority 5(A)'s receiver-aware tests, which depend on
    /// real span nesting (via `enclosing_class_of`) to determine which
    /// class a caller/field/method belongs to; `test_snapshot`/
    /// `test_snapshot_in_repo` both hardcode the same `[0, 10)` span for
    /// every snapshot, which can never nest.
    fn spanned_snapshot_in_repo(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str, start: usize, end: usize) -> SymbolSnapshot {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start, end }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    #[test]
    fn a_call_to_a_unique_candidate_is_resolved() {
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { helper(); }");
        let callee = test_snapshot("src/lib.rs", "helper", "function_item", "fn helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, caller.id());
        assert_eq!(edges[0].to, callee.id());
        assert_eq!(edges[0].kind, EdgeKind::Calls);
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn a_repo_wide_ambiguous_call_with_no_scope_signal_produces_no_edge() {
        // Task 9.5, Priority 5(A), Case B (CALL-RES-A1+A2 — see
        // docs/research/EXPERIMENTAL_POLICY.md): this is the EXACT shape
        // that produced B2-EVAL-001's 19-for-19 false-positive rate on a
        // real 97-file corpus — a bare, unqualified call whose name
        // matches more than one candidate, with no scope signal (no
        // enclosing class for the caller, candidates in different files
        // from the caller and from each other) to narrow by. Previously
        // (CALL-RES-A0) this resolver recorded one Hypothesized edge per
        // candidate; the new policy prefers no edge over an arbitrary
        // or over-broad guess once narrowing is exhausted.
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { run(); }");
        let candidate_a = test_snapshot("src/a.rs", "run", "function_item", "fn run() {}");
        let candidate_b = test_snapshot("src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "ambiguity with no scope signal to narrow by must produce no edge, not an arbitrary or over-broad guess: got {edges:?}");
    }

    #[test]
    fn an_ambiguous_call_narrowed_to_the_callers_own_file_produces_one_heuristic_edge() {
        // Same bare-name ambiguity, but now ONE of the two same-named
        // candidates shares the caller's own file — same-file narrowing
        // (CALL-RES-A2) must resolve this to that one candidate, with
        // heuristic (never StaticAnalysis) evidence, since the file-
        // level match is still a name-based guess, just a scoped one.
        let repo = test_repo_calls();
        let caller = test_snapshot_in_repo(repo, "src/lib.rs", "foo", "function_item", "fn foo() { run(); }");
        let same_file_candidate = test_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() {}");
        let unrelated_candidate = test_snapshot_in_repo(repo, "src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), same_file_candidate.clone(), unrelated_candidate.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1, "exactly one candidate shares the caller's file; that must resolve uniquely: got {edges:?}");
        assert_eq!(edges[0].from, caller.id());
        assert_eq!(edges[0].to, same_file_candidate.id());
        assert_ne!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis, "a scoped-but-still-name-based match must never be StaticAnalysis");
    }

    #[test]
    fn an_ambiguous_call_with_two_same_file_candidates_still_produces_no_edge() {
        // Narrowing to "same file as caller" still leaves TWO
        // candidates — genuine same-scope ambiguity. Per the user's
        // explicit final decision for Case B, this must NOT fall back
        // to emitting heuristic edges for both; it must produce no edge
        // at all, exactly like the repo-wide case.
        let repo = test_repo_calls();
        let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "foo", "function_item", "fn foo() { run(); }", 0, 20);
        let candidate_a = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* a */ }", 30, 50);
        let candidate_b = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* b */ }", 60, 80);
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "two indistinguishable candidates in the caller's own file must still produce no edge, not an arbitrary pick: got {edges:?}");
    }

    #[test]
    fn an_ambiguous_call_narrowed_to_the_callers_own_class_produces_one_heuristic_edge_even_with_another_same_file_candidate() {
        // The scope hierarchy's priority order: same-class is tried
        // BEFORE same-file. ClassA.forward calls run(); ClassA itself
        // contains a `run` method (unqualified call, no receiver, but
        // still within ClassA's own span) — wait: an unqualified call
        // has no receiver to tie it to a class syntactically, so this
        // test instead verifies the class-level narrowing uses span
        // containment of the CANDIDATE within the caller's class, which
        // is the real signal `enclosing_class_of`/`narrow_by_scope` use:
        // a `run` method nested inside the SAME class as the caller
        // narrows correctly even when a second, same-file-but-different-
        // class `run` also exists.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 200);
        let forward = spanned_snapshot_in_repo(repo, "src/m.py", "forward", "function_definition", "def forward(self):\n    run()", 10, 90);
        let run_in_class_a = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(self): pass", 100, 190);
        let run_outside_any_class = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(): pass", 250, 280);
        let snapshots = vec![class_a.clone(), forward.clone(), run_in_class_a.clone(), run_outside_any_class.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1, "exactly one `run` is a member of the caller's own enclosing class; that must resolve uniquely: got {edges:?}");
        assert_eq!(edges[0].to, run_in_class_a.id());
    }

    #[test]
    fn a_call_to_an_unknown_target_produces_no_edge() {
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { not_in_the_pool(); }");
        let snapshots = vec![caller];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "a call to a name absent from the given pool must produce no edge");
    }

    #[test]
    fn a_method_call_resolves_by_its_final_segment_name() {
        // The forward()-calls-encoder() deep-learning-relevant shape:
        // self.encoder(x) must resolve to a snapshot named "encoder",
        // not "self".
        let caller = test_snapshot(
            "src/model.py",
            "forward",
            "function_definition",
            "def forward(self, x):\n    return self.encoder(x)",
        );
        let encoder = test_snapshot("src/model.py", "encoder", "function_definition", "def encoder(x):\n    return x");
        let snapshots = vec![caller.clone(), encoder.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, encoder.id());
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn a_cross_file_call_is_still_resolved() {
        let caller = test_snapshot("src/a.rs", "foo", "function_item", "fn foo() { helper(); }");
        let callee = test_snapshot("src/b.rs", "helper", "function_item", "fn helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_recursive_call_does_not_self_link() {
        let recursive = test_snapshot("src/lib.rs", "factorial", "function_item", "fn factorial(n: u32) -> u32 { factorial(n - 1) }");
        let snapshots = vec![recursive.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "a function calling itself by name must not produce a self-referential edge from this resolver");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let snapshot = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() {}");
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_calls(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    #[test]
    fn a_cpp_call_is_resolved() {
        let caller = test_snapshot("src/a.cpp", "foo", "function_definition", "void foo() { helper(); }");
        let callee = test_snapshot("src/a.cpp", "helper", "function_definition", "void helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_header_extension_is_recognized_as_cpp() {
        let caller = test_snapshot("src/a.hpp", "foo", "function_definition", "void foo() { helper(); }");
        let callee = test_snapshot("src/a.hpp", "helper", "function_definition", "void helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_csharp_call_is_resolved() {
        let caller = test_snapshot("src/a.cs", "M", "method_declaration", "void M() { Helper(); }");
        let callee = test_snapshot("src/a.cs", "Helper", "method_declaration", "void Helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_tsx_call_is_resolved() {
        let caller = test_snapshot("src/a.tsx", "Comp", "function_declaration", "function Comp() { return helper(); }");
        let callee = test_snapshot("src/a.tsx", "helper", "function_declaration", "function helper() { return 1; }");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_jsx_extension_is_recognized_as_tsx() {
        let caller = test_snapshot("src/a.jsx", "Comp", "function_declaration", "function Comp() { return helper(); }");
        let callee = test_snapshot("src/a.jsx", "helper", "function_declaration", "function helper() { return 1; }");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    // --- B2-ID-002: occurrence-vs-relation deduplication ---

    #[test]
    fn calling_the_same_resolved_target_twice_in_one_body_produces_one_edge() {
        let caller = test_snapshot("src/lib.rs", "caller", "function_item", "fn caller() { callee(); callee(); }");
        let callee = test_snapshot("src/lib.rs", "callee", "function_item", "fn callee() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let to_callee: Vec<_> = edges.iter().filter(|e| e.from == caller.id() && e.to == callee.id()).collect();
        assert_eq!(to_callee.len(), 1, "two occurrences of the same call must collapse to one (from,to,Calls) edge");
    }

    #[test]
    fn calling_the_same_resolved_target_many_times_still_produces_one_edge() {
        let caller = test_snapshot("src/lib.rs", "caller", "function_item", "fn caller() { callee(); callee(); callee(); callee(); callee(); }");
        let callee = test_snapshot("src/lib.rs", "callee", "function_item", "fn callee() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let to_callee: Vec<_> = edges.iter().filter(|e| e.from == caller.id() && e.to == callee.id()).collect();
        assert_eq!(to_callee.len(), 1, "five occurrences of the same call must still collapse to one edge");
    }

    #[test]
    fn calling_two_different_targets_from_one_caller_produces_two_edges() {
        let caller = test_snapshot("src/lib.rs", "caller", "function_item", "fn caller() { a(); b(); }");
        let a = test_snapshot("src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot("src/lib.rs", "b", "function_item", "fn b() {}");
        let snapshots = vec![caller.clone(), a.clone(), b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.iter().filter(|e| e.from == caller.id()).count(), 2, "two genuinely distinct targets must not be collapsed into one edge");
    }

    #[test]
    fn two_different_callers_calling_the_same_target_each_get_their_own_edge() {
        let caller_a = test_snapshot("src/a.rs", "caller_a", "function_item", "fn caller_a() { shared(); }");
        let caller_b = test_snapshot("src/b.rs", "caller_b", "function_item", "fn caller_b() { shared(); }");
        let target = test_snapshot("src/lib.rs", "shared", "function_item", "fn shared() {}");
        let snapshots = vec![caller_a.clone(), caller_b.clone(), target.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let to_target: Vec<_> = edges.iter().filter(|e| e.to == target.id()).collect();
        assert_eq!(to_target.len(), 2, "two distinct callers of the same target must each have their own edge, not be deduplicated against each other");
        assert!(to_target.iter().any(|e| e.from == caller_a.id()));
        assert!(to_target.iter().any(|e| e.from == caller_b.id()));
    }

    #[test]
    fn an_ambiguous_call_narrowed_to_one_candidate_and_repeated_in_one_body_produces_no_duplicate_from_to_calls_triples() {
        // B2-ID-002's original concern (repeated occurrences of the
        // same ambiguous call must collapse to one edge, not one per
        // occurrence) still applies under the new Case B scoping — now
        // exercised via a scenario where same-file narrowing resolves
        // the ambiguity to exactly one candidate, called TWICE by the
        // same caller.
        let repo = test_repo_calls();
        let caller = test_snapshot_in_repo(repo, "src/caller.rs", "caller", "function_item", "fn caller() { run(); run(); }");
        let same_file_candidate = test_snapshot_in_repo(repo, "src/caller.rs", "run", "function_item", "fn run() {}");
        let unrelated_candidate = test_snapshot_in_repo(repo, "src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), same_file_candidate.clone(), unrelated_candidate.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let mut seen = std::collections::HashSet::new();
        for edge in &edges {
            assert!(seen.insert((edge.from, edge.to, edge.kind)), "duplicate (from,to,kind) triple found: repeated occurrences of the same scoped-ambiguous call must not produce more than one edge");
        }
        assert_eq!(edges.len(), 1, "exactly one edge, to the same-file candidate, regardless of how many times the ambiguous name was called in this body");
        assert_eq!(edges[0].to, same_file_candidate.id());
    }

    #[test]
    fn an_ambiguous_call_with_no_scope_resolution_repeated_in_one_body_still_produces_no_edge() {
        // The repo-wide-ambiguity-with-no-scope-signal case (previous
        // test) must also stay edge-free even when the ambiguous name
        // is called MORE THAN ONCE in the same body — repetition must
        // never turn "no edge" into "an edge after all."
        let caller = test_snapshot("src/caller.rs", "caller", "function_item", "fn caller() { run(); run(); }");
        let candidate_a = test_snapshot("src/a.rs", "run", "function_item", "fn run() {}");
        let candidate_b = test_snapshot("src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "repetition of an unresolvable ambiguous call must not manufacture an edge: got {edges:?}");
    }

    #[test]
    fn evidence_selection_between_duplicate_occurrences_is_deterministic_regardless_of_body_order() {
        // Two callers with the SAME two call occurrences but written in
        // OPPOSITE order in the source body must still resolve to
        // exactly the same final edge (same evidence note), proving the
        // dedup's tie-break does not depend on which occurrence tree-
        // sitter/this walk visits first.
        let callee = test_snapshot("src/lib.rs", "callee", "function_item", "fn callee() {}");
        let caller_order_1 = test_snapshot("src/a.rs", "caller_order_1", "function_item", "fn caller_order_1() { callee(); callee(); }");
        let caller_order_2 = test_snapshot("src/b.rs", "caller_order_2", "function_item", "fn caller_order_2() { callee(); callee(); }");
        let snapshots = vec![callee.clone(), caller_order_1.clone(), caller_order_2.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let edge_1 = edges.iter().find(|e| e.from == caller_order_1.id()).unwrap();
        let edge_2 = edges.iter().find(|e| e.from == caller_order_2.id()).unwrap();
        assert_eq!(edge_1.evidence.note(), edge_2.evidence.note(), "identical repeated occurrences must resolve to byte-identical evidence regardless of caller");
    }

    #[test]
    fn b2_id_002_regression_a_caller_calling_the_same_ambiguous_name_many_times_does_not_panic_on_graph_insertion() {
        // Direct regression reproduction of the real failure mode: a
        // caller invoking the SAME ambiguous name repeatedly (common in
        // real code, e.g. several calls to a same-named helper across
        // overloaded candidates) must never produce a (from,to,kind)
        // duplicate that CodeGraph::insert_edge would reject.
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let caller = test_snapshot_in_repo(
            repo,
            "src/caller.rs",
            "caller",
            "function_item",
            "fn caller() { forward(); forward(); forward(); forward(); forward(); }",
        );
        let candidate_a = test_snapshot_in_repo(repo, "src/a.rs", "forward", "function_item", "fn forward() {}");
        let candidate_b = test_snapshot_in_repo(repo, "src/b.rs", "forward", "function_item", "fn forward() {}");
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let mut graph = crate::graph::CodeGraph::new();
        for snapshot in &snapshots {
            graph.insert_snapshot(snapshot.clone()).unwrap();
        }
        for edge in edges {
            graph.insert_edge(edge).unwrap();
        }
    }

    // --- Direct unit tests on `prefer_edge` itself: mutation testing
    // flagged this function's actual decision logic as unexercised by
    // the end-to-end tests above (which only ever compare two
    // IDENTICAL-evidence occurrences, never two with genuinely
    // different ResolutionMethod or different note text) ---

    fn dummy_snapshot_id() -> codegraph_domain::SymbolSnapshotId {
        test_snapshot("src/x.rs", "x", "function_item", "fn x() {}").id()
    }

    fn static_edge(note: &str) -> Edge {
        Edge {
            from: dummy_snapshot_id(),
            to: dummy_snapshot_id(),
            kind: EdgeKind::Calls,
            evidence: resolved_evidence(note),
        }
    }

    fn heuristic_edge(note: &str) -> Edge {
        let (evidence, _) = heuristic_evidence(codegraph_domain::ResolutionMethod::NameHeuristic, Confidence::Speculative, note);
        Edge {
            from: dummy_snapshot_id(),
            to: dummy_snapshot_id(),
            kind: EdgeKind::Calls,
            evidence,
        }
    }

    #[test]
    fn prefer_edge_prefers_static_analysis_over_heuristic() {
        let static_candidate = static_edge("resolved");
        let heuristic_current = heuristic_edge("ambiguous");
        assert!(prefer_edge(&static_candidate, &heuristic_current), "a StaticAnalysis candidate must replace a heuristic current edge");
    }

    #[test]
    fn prefer_edge_never_replaces_static_analysis_with_heuristic() {
        let heuristic_candidate = heuristic_edge("ambiguous");
        let static_current = static_edge("resolved");
        assert!(!prefer_edge(&heuristic_candidate, &static_current), "a heuristic candidate must never replace an already-StaticAnalysis current edge");
    }

    #[test]
    fn prefer_edge_breaks_ties_between_two_static_edges_by_note_text() {
        let a = static_edge("aaa");
        let b = static_edge("bbb");
        assert!(prefer_edge(&a, &b), "the lexicographically smaller note must win the tie-break (a < b)");
        assert!(!prefer_edge(&b, &a), "the tie-break must be antisymmetric: b must not also beat a");
    }

    #[test]
    fn prefer_edge_breaks_ties_between_two_heuristic_edges_by_note_text() {
        let a = heuristic_edge("aaa");
        let b = heuristic_edge("bbb");
        assert!(prefer_edge(&a, &b));
        assert!(!prefer_edge(&b, &a));
    }

    #[test]
    fn prefer_edge_is_false_for_byte_identical_edges() {
        // Neither side should "win" against its own exact duplicate —
        // pins the strict `<` (not `<=`) comparison specifically.
        let a = static_edge("same");
        let b = static_edge("same");
        assert!(!prefer_edge(&a, &b), "an edge identical to the current one must not be reported as preferred (strict < , not <=)");
    }

    #[test]
    fn upsert_preferring_stronger_evidence_keeps_the_static_analysis_edge_regardless_of_insertion_order() {
        let mut per_target = std::collections::HashMap::new();
        let target = dummy_snapshot_id();
        let heuristic = Edge {
            to: target,
            ..heuristic_edge("ambiguous")
        };
        let resolved = Edge { to: target, ..static_edge("resolved") };

        // Heuristic inserted FIRST, then the StaticAnalysis one — the
        // stronger one must still win even though it arrives second.
        upsert_preferring_stronger_evidence(&mut per_target, heuristic.clone());
        upsert_preferring_stronger_evidence(&mut per_target, resolved.clone());
        assert_eq!(per_target.get(&target).unwrap().evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);

        // Reverse order: StaticAnalysis first, then heuristic — must
        // still end up StaticAnalysis, proving this is not merely
        // "last write wins."
        let mut per_target_reversed = std::collections::HashMap::new();
        upsert_preferring_stronger_evidence(&mut per_target_reversed, resolved);
        upsert_preferring_stronger_evidence(&mut per_target_reversed, heuristic);
        assert_eq!(per_target_reversed.get(&target).unwrap().evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    // --- Task 9.5, Priority 5(A), Case A: receiver-aware common-name
    // call resolution (`self.field.method()`) ---
    //
    // Root cause confirmed in B2-EVAL-001/this investigation:
    // `self.src_mask.size(0)` matched an UNRELATED C++ `size()` method
    // elsewhere in a 97-file real corpus, because `callee_name`
    // extracted only the bare final segment (`"size"`) with no
    // awareness that `src_mask` (the receiver) names a real `Field` on
    // the caller's OWN class. These tests pin the fix: a receiver-
    // qualified call through `self`/`this` onto a KNOWN member of the
    // caller's own class restricts candidate resolution to that same
    // class; cross-class candidates are rejected rather than guessed.

    #[test]
    fn a_receiver_qualified_call_resolves_only_within_the_callers_own_class_even_with_a_same_named_method_elsewhere() {
        let repo = test_repo_calls();
        // ClassA.src_mask is a Field; ClassA has NO method named `size`
        // itself, but an unrelated ClassB (different file, standing in
        // for the real corpus's unrelated C++ `size()` methods) does.
        // The call `self.src_mask.size(0)` inside ClassA::forward must
        // NOT resolve to ClassB::size.
        let class_a = spanned_snapshot_in_repo(repo, "src/model.py", "ClassA", "class_definition", "class ClassA:\n    ...", 0, 200);
        let src_mask = spanned_snapshot_in_repo(repo, "src/model.py", "src_mask", "self_attribute_assignment", "self.src_mask = None", 10, 40);
        let forward = spanned_snapshot_in_repo(repo, "src/model.py", "forward", "function_definition", "def forward(self):\n    self.src_mask.size(0)", 50, 150);
        let unrelated_size = spanned_snapshot_in_repo(repo, "src/other.cpp", "size", "function_definition", "size_t size() { return 0; }", 0, 50);
        let snapshots = vec![class_a.clone(), src_mask.clone(), forward.clone(), unrelated_size.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(
            edges.iter().all(|e| e.to != unrelated_size.id()),
            "a receiver-qualified call on a known Field of the caller's own class must never resolve to an unrelated cross-class/cross-file same-named method"
        );
    }

    #[test]
    fn a_receiver_qualified_call_still_resolves_when_the_matching_method_is_on_the_callers_own_class() {
        // self.helper.run() where `helper` IS a Field of the caller's
        // class, and the caller's OWN class also contains a method
        // named `run` — this must still resolve (the restriction keeps
        // in-class matches, it only drops cross-class ones).
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/model.py", "ClassA", "class_definition", "class ClassA:\n    ...", 0, 300);
        let helper_field = spanned_snapshot_in_repo(repo, "src/model.py", "helper", "self_attribute_assignment", "self.helper = Helper()", 10, 40);
        let run_method = spanned_snapshot_in_repo(repo, "src/model.py", "run", "function_definition", "def run(self): pass", 50, 90);
        let forward = spanned_snapshot_in_repo(repo, "src/model.py", "forward", "function_definition", "def forward(self):\n    self.helper.run()", 100, 250);
        let snapshots = vec![class_a.clone(), helper_field.clone(), run_method.clone(), forward.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(
            edges.iter().any(|e| e.from == forward.id() && e.to == run_method.id() && e.evidence.method() == codegraph_domain::ResolutionMethod::StaticAnalysis),
            "a receiver-qualified call must still resolve, with StaticAnalysis evidence, when the matching method genuinely belongs to the caller's own class"
        );
    }

    #[test]
    fn two_classes_in_the_same_file_each_with_their_own_same_named_method_do_not_cross_resolve_via_receiver() {
        // ClassA.forward calls self.helper.run(); ClassB ALSO has a
        // `run` method, but ClassB is a different class in the SAME
        // file. ClassA's own `helper` field does not belong to ClassB,
        // so ClassA's call must only ever consider ClassA's own `run`.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 200);
        let helper_field = spanned_snapshot_in_repo(repo, "src/m.py", "helper", "self_attribute_assignment", "self.helper = Helper()", 10, 30);
        let run_a = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(self): pass", 40, 60);
        let forward = spanned_snapshot_in_repo(repo, "src/m.py", "forward", "function_definition", "def forward(self):\n    self.helper.run()", 70, 190);
        let class_b = spanned_snapshot_in_repo(repo, "src/m.py", "ClassB", "class_definition", "class ClassB: ...", 300, 500);
        let run_b = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(self): pass", 320, 340);
        let snapshots = vec![class_a.clone(), helper_field.clone(), run_a.clone(), forward.clone(), class_b.clone(), run_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.iter().any(|e| e.from == forward.id() && e.to == run_a.id()), "ClassA's own run method must still resolve");
        assert!(edges.iter().all(|e| e.to != run_b.id()), "ClassB's run method, a different class in the same file, must never be reached through ClassA's own receiver-qualified call");
    }

    #[test]
    fn an_unqualified_call_with_a_target_in_the_same_scope_is_unaffected_by_the_receiver_restriction() {
        // A bare, non-receiver call (`helper()`, not `self.helper()`)
        // must behave exactly as before this fix — the restriction only
        // ever applies to a `self`/`this`-qualified receiver chain.
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { helper(); }");
        let callee = test_snapshot("src/lib.rs", "helper", "function_item", "fn helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_self_qualified_call_whose_field_is_not_a_known_member_falls_back_to_unrestricted_matching() {
        // self.not_a_real_field.run() where `not_a_real_field` is NOT a
        // real member of the caller's class at all (no such Field or
        // method exists). The structural premise for the restriction
        // does not hold, so behavior is unchanged from before this fix:
        // ordinary name-based resolution still applies.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 200);
        let forward = spanned_snapshot_in_repo(repo, "src/m.py", "forward", "function_definition", "def forward(self):\n    self.not_a_real_field.run()", 10, 190);
        let run_elsewhere = spanned_snapshot_in_repo(repo, "src/other.py", "run", "function_definition", "def run(self): pass", 0, 20);
        let snapshots = vec![class_a.clone(), forward.clone(), run_elsewhere.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(
            edges.iter().any(|e| e.from == forward.id() && e.to == run_elsewhere.id()),
            "when the receiver field is not a known member of the caller's class, resolution must fall back to the pre-existing unrestricted behavior"
        );
    }

    #[test]
    fn receiver_restriction_is_deterministic_regardless_of_snapshot_order() {
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/model.py", "ClassA", "class_definition", "class ClassA:\n    ...", 0, 200);
        let src_mask = spanned_snapshot_in_repo(repo, "src/model.py", "src_mask", "self_attribute_assignment", "self.src_mask = None", 10, 40);
        let forward = spanned_snapshot_in_repo(repo, "src/model.py", "forward", "function_definition", "def forward(self):\n    self.src_mask.size(0)", 50, 150);
        let unrelated_size = spanned_snapshot_in_repo(repo, "src/other.cpp", "size", "function_definition", "size_t size() { return 0; }", 0, 50);

        let forward_order = vec![class_a.clone(), src_mask.clone(), forward.clone(), unrelated_size.clone()];
        let reverse_order = vec![unrelated_size.clone(), forward.clone(), src_mask.clone(), class_a.clone()];

        let edges_a = resolve_calls(&forward_order).unwrap();
        let edges_b = resolve_calls(&reverse_order).unwrap();

        assert_eq!(edges_a.len(), edges_b.len(), "snapshot pool ordering must never change the resolved edge count");
        assert!(edges_a.iter().all(|e| e.to != unrelated_size.id()));
        assert!(edges_b.iter().all(|e| e.to != unrelated_size.id()));
    }

    fn test_repo_calls() -> codegraph_domain::RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    // --- Direct, white-box tests for `enclosing_class_of`/`span_len`,
    // added closing Task 9.5's mutation-testing gate. ---

    #[test]
    fn enclosing_class_of_excludes_a_candidate_with_the_symbols_own_id_even_if_it_is_a_class() {
        // Pins the `candidate_class.id() == symbol.id()` half of the
        // `||` skip-condition: a symbol can never be its OWN enclosing
        // class, even in the degenerate case where `symbol` itself is
        // Class-kind and would otherwise structurally "contain" itself
        // (a span trivially contains itself, but `strictly_contains`
        // already excludes equal spans — this test isolates the id-
        // equality check specifically, independent of that).
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 100);

        let pool = [class_a.clone()];
        let result = enclosing_class_of(&pool, &class_a);

        assert_eq!(result, None, "a symbol must never be reported as its own enclosing class");
    }

    #[test]
    fn enclosing_class_of_excludes_a_non_class_candidate_even_if_it_structurally_contains_the_symbol() {
        // Pins the `candidate_class.kind() != SymbolKind::Class` half
        // of the `||` skip-condition: a non-Class snapshot (e.g. a
        // Function) whose span happens to structurally contain
        // `symbol`'s span must never be reported as its enclosing
        // class, even though span-containment alone would suggest it.
        let repo = test_repo_calls();
        let outer_function = spanned_snapshot_in_repo(repo, "src/m.py", "outer", "function_definition", "def outer(): ...", 0, 100);
        let inner_method = spanned_snapshot_in_repo(repo, "src/m.py", "inner", "function_definition", "def inner(): ...", 10, 50);

        let pool = [outer_function, inner_method.clone()];
        let result = enclosing_class_of(&pool, &inner_method);

        assert_eq!(result, None, "a non-Class container must never be reported as an enclosing class, regardless of span containment");
    }

    #[test]
    fn enclosing_class_of_excludes_a_class_in_a_different_file_even_if_spans_overlap() {
        // Pins the `!same_file(...)` half of the second `||`
        // skip-condition, isolated from `strictly_contains`: same byte
        // range, different file.
        let repo = test_repo_calls();
        let class_in_other_file = spanned_snapshot_in_repo(repo, "src/a.py", "ClassA", "class_definition", "class ClassA: ...", 0, 100);
        let method = spanned_snapshot_in_repo(repo, "src/b.py", "method", "function_definition", "def method(): ...", 10, 50);

        let pool = [class_in_other_file, method.clone()];
        let result = enclosing_class_of(&pool, &method);

        assert_eq!(result, None, "a same-span class in a DIFFERENT file must never be reported as the enclosing class");
    }

    #[test]
    fn enclosing_class_of_excludes_a_same_file_class_that_does_not_structurally_contain_the_symbol() {
        // Pins the `!strictly_contains(...)` half of the second `||`
        // skip-condition, isolated from `same_file`: same file, but the
        // class's span does not contain the symbol's span at all.
        let repo = test_repo_calls();
        let unrelated_class = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 50);
        let method = spanned_snapshot_in_repo(repo, "src/m.py", "method", "function_definition", "def method(): ...", 100, 150);

        let pool = [unrelated_class, method.clone()];
        let result = enclosing_class_of(&pool, &method);

        assert_eq!(result, None, "a same-file class whose span does not contain the symbol must never be reported as its enclosing class");
    }

    #[test]
    fn enclosing_class_of_picks_the_smaller_of_two_nested_containing_classes() {
        // Pins the `span_len(candidate_class) < span_len(current)`
        // match guard directly: TWO classes both structurally contain
        // `method` (Outer, then the smaller Inner nested within it) —
        // the innermost (smallest span) must win, confirmed by checking
        // BOTH insertion orders so iteration order cannot be the real
        // reason this passes.
        let repo = test_repo_calls();
        let outer = spanned_snapshot_in_repo(repo, "src/m.py", "Outer", "class_definition", "class Outer: ...", 0, 200);
        let inner = spanned_snapshot_in_repo(repo, "src/m.py", "Inner", "class_definition", "class Inner: ...", 50, 150);
        let method = spanned_snapshot_in_repo(repo, "src/m.py", "method", "function_definition", "def method(): ...", 80, 120);

        let forward_order = [outer.clone(), inner.clone(), method.clone()];
        let reverse_order = [inner.clone(), outer.clone(), method.clone()];

        assert_eq!(enclosing_class_of(&forward_order, &method), Some(&inner), "the smaller (innermost) containing class must win regardless of insertion order");
        assert_eq!(enclosing_class_of(&reverse_order, &method), Some(&inner));
    }

    #[test]
    fn enclosing_class_of_keeps_the_first_innermost_class_found_when_a_later_candidate_is_not_strictly_smaller() {
        // Pins the `Some(current) => current` (keep, don't replace) arm
        // directly: a SECOND candidate class with a span size EQUAL to
        // (not smaller than) the current innermost's must never replace
        // it — only a STRICTLY smaller span may.
        let repo = test_repo_calls();
        let first_class = spanned_snapshot_in_repo(repo, "src/m.py", "FirstClass", "class_definition", "class FirstClass: ...", 0, 200);
        // Same span length as first_class (100 bytes), structurally
        // nested at a DIFFERENT position so both genuinely contain
        // `method` without being identical spans (which would instead
        // trip the duplicate-snapshot-id rejection elsewhere).
        let second_class = spanned_snapshot_in_repo(repo, "src/m.py", "SecondClass", "class_definition", "class SecondClass: ...", 0, 200);
        let method = spanned_snapshot_in_repo(repo, "src/m.py", "method", "function_definition", "def method(): ...", 80, 120);

        let pool = [first_class.clone(), second_class.clone(), method.clone()];
        let result = enclosing_class_of(&pool, &method);

        // Both classes tie on span length — the match guard's `<` means
        // neither replaces an already-set innermost once one is picked
        // first; this test fixes the ORDER so the result is
        // deterministic and verifiably matches the `<` (never `<=`)
        // semantics: the FIRST one encountered during iteration wins a
        // tie, confirming `<` rather than `<=` governs replacement.
        assert_eq!(result, Some(&first_class));
    }

    #[test]
    fn self_or_this_receiver_field_name_recognizes_rusts_dedicated_self_node_kind() {
        // Pins the LEFT half of `is_self_or_this`'s `||`:
        // `matches!(base.kind(), "self" | "this")` must independently
        // be sufficient to recognize a receiver as self/this, WITHOUT
        // relying on the Python-specific identifier-text fallback on
        // the right. Rust's `self` parses as a dedicated `self` node
        // kind (confirmed via a real parse dump, Task 9.5 Priority
        // 5(A)) — if the `||` were mutated to `&&`, this would always
        // be false (a node's kind cannot simultaneously be `"self"` and
        // `"identifier"`), so this test fails on that mutant while
        // every existing Python-only test does not exercise this path
        // at all.
        let source = b"fn forward(&self) { self.src_mask.size(0); }";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::RustParser, source).unwrap();
        let root = tree.tree().root_node();
        let call_node = find_first_node_of_kind(root, "call_expression").expect("expected a call_expression node");
        let callee = call_node.child(0).expect("call node must have a callee child");

        let result = self_or_this_receiver_field_name(callee, source);

        assert_eq!(result, Some("src_mask".to_string()), "Rust's dedicated `self` node kind must be recognized without needing the Python-specific text fallback");
    }

    #[test]
    fn self_or_this_receiver_field_name_rejects_a_python_identifier_receiver_whose_text_is_not_self() {
        // Pins the INNER `&&` of `is_self_or_this`'s RIGHT disjunct
        // directly: `base.kind() == "identifier" && text_of(base,
        // source) == "self"` requires BOTH the kind check AND the text
        // check. `other_obj` is a plain Python identifier (kind
        // "identifier", exactly like `self` would be — Python has no
        // dedicated `self` node kind) whose TEXT is NOT "self". If this
        // inner `&&` were mutated to `||`, `base.kind() == "identifier"`
        // alone would already be true (since EVERY Python receiver base
        // parses as an `identifier` node), making `is_self_or_this` true
        // for ANY receiver, not just `self` — exactly the false-positive
        // shape the whole Case A fix exists to prevent. The existing
        // Rust-based test above exercises the OUTER `||`'s left disjunct
        // (`"self" | "this"` node kind) but never reaches this inner
        // expression at all, since Rust's dedicated `self` kind short-
        // circuits before it.
        let source = b"def forward(other_obj):\n    other_obj.src_mask.size(0)\n";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::PythonParser, source).unwrap();
        let root = tree.tree().root_node();
        let call_node = find_first_node_of_kind(root, "call").expect("expected a call node");
        let callee = call_node.child(0).expect("call node must have a callee child");

        let result = self_or_this_receiver_field_name(callee, source);

        assert_eq!(result, None, "a plain identifier receiver whose text is not literally \"self\" must never be treated as a self/this-qualified receiver");
    }

    #[test]
    fn self_or_this_receiver_field_name_returns_none_for_a_bare_identifier_callee() {
        // Pins `is_receiver_qualified`'s real effect at its FIRST call
        // site in self_or_this_receiver_field_name: a bare, non-
        // receiver callee (`helper()`, callee node kind `identifier`)
        // must return None. A plain `identifier` node has zero
        // children, so `callee.child(0)` alone already yields `None`
        // regardless of `is_receiver_qualified`'s own result for THIS
        // specific case — this test exists to pin the observable
        // behavior precisely (None), even though it does not by itself
        // distinguish `is_receiver_qualified` from an always-true stub
        // for a childless node.
        let source = b"def forward(): helper()";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::PythonParser, source).unwrap();
        let root = tree.tree().root_node();
        let call_node = find_first_node_of_kind(root, "call").expect("expected a call node");
        let callee = call_node.child(0).expect("call node must have a callee child");
        assert_eq!(callee.kind(), "identifier");

        let result = self_or_this_receiver_field_name(callee, source);

        assert_eq!(result, None, "a bare identifier callee must never be read as a self/this-qualified receiver field");
    }

    #[test]
    fn is_receiver_qualified_is_false_for_a_bare_identifier_node() {
        // Direct, white-box pin of `is_receiver_qualified` itself
        // (distinct from the end-to-end test above): a plain
        // `identifier` node must report `false`, not the stubbed
        // `true` a mutation would otherwise substitute.
        let source = b"def forward(): helper()";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::PythonParser, source).unwrap();
        let root = tree.tree().root_node();
        let call_node = find_first_node_of_kind(root, "call").expect("expected a call node");
        let callee = call_node.child(0).expect("call node must have a callee child");

        assert!(!is_receiver_qualified(callee), "a bare identifier node must never be reported as receiver-qualified");
    }

    fn find_first_node_of_kind<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_first_node_of_kind(child, kind) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn narrow_by_scope_falls_through_to_same_file_when_the_callers_own_class_has_zero_candidates() {
        // Pins the `0 => {}` (fall-through, not an early return) arm in
        // narrow_by_scope's class-level check directly: the caller HAS
        // an enclosing class, but NONE of the ambiguous candidates
        // belong to it (same_class.len() == 0) — this must fall
        // through to the same-file check below, not short-circuit to
        // None, so a same-file (but different-class) candidate can
        // still resolve.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 90);
        let caller = spanned_snapshot_in_repo(repo, "src/m.py", "caller", "function_definition", "def caller(): ...", 10, 80);
        // same_file_candidate is in the SAME FILE as caller, but NOT
        // inside ClassA's span (so same_class is empty for it).
        let same_file_candidate = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(): ...", 100, 150);

        let candidates = [&same_file_candidate];
        let result = narrow_by_scope(&[class_a, caller.clone(), same_file_candidate.clone()], &caller, &candidates);

        assert_eq!(
            result.map(|(s, _)| s),
            Some(&same_file_candidate),
            "zero same-class candidates must fall through to the same-file check, not short-circuit to None"
        );
    }

    #[test]
    fn narrow_by_scope_returns_none_when_the_callers_own_class_has_more_than_one_candidate() {
        // Pins the `_ => return None` arm for the class-level check:
        // TWO candidates both belong to the caller's own class —
        // genuine same-class ambiguity must return None immediately,
        // never fall through to try the (necessarily broader) same-
        // file level.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 200);
        let caller = spanned_snapshot_in_repo(repo, "src/m.py", "caller", "function_definition", "def caller(): ...", 10, 90);
        let candidate_a = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(): ...", 100, 130);
        let candidate_b = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run2(): ...", 140, 170);

        let candidates = [&candidate_a, &candidate_b];
        let pool = [class_a, caller.clone(), candidate_a.clone(), candidate_b.clone()];
        let result = narrow_by_scope(&pool, &caller, &candidates);

        assert_eq!(result, None, "two candidates both in the caller's own class must be ambiguous, never resolved by falling through to a broader scope");
    }

    #[test]
    fn span_len_computes_the_difference_not_the_sum_or_a_constant() {
        let repo = test_repo_calls();
        let short = spanned_snapshot_in_repo(repo, "src/m.py", "a", "function_definition", "a", 10, 15);
        let long = spanned_snapshot_in_repo(repo, "src/m.py", "b", "function_definition", "b", 10, 115);

        assert_eq!(span_len(&short), 5);
        assert_eq!(span_len(&long), 105);
        assert_ne!(span_len(&short), span_len(&long));
    }

    // B2-ID-006 regression tests. Every snapshot below is built from
    // REAL extraction (`extract_symbols_reporting_rejections` + the
    // real `promote`), not a hand-written placeholder `signature_text`
    // — the whole point of this finding is that a placeholder body
    // structurally cannot exercise this bug (see B2-ID-006's own
    // "Observed" section), so these regression tests must not repeat
    // that mistake.
    mod b2_id_006_nested_scope_ownership {
        use super::*;
        use crate::extract::extract_symbols_reporting_rejections;
        use crate::parse::parser_for;

        /// Extracts and promotes every real candidate in `source`
        /// (parsed as `path`'s own language), returning the resulting
        /// snapshots. Mirrors exactly what the real production
        /// pipeline does (parse -> extract -> promote), never
        /// hand-constructing a `SymbolSnapshot` with an invented span
        /// or placeholder signature text.
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

        fn callers_of<'a>(edges: &'a [Edge], target: &SymbolSnapshot, snapshots: &'a [SymbolSnapshot]) -> Vec<&'a str> {
            let mut names: Vec<&str> = edges.iter().filter(|e| e.to == target.id()).filter_map(|e| snapshots.iter().find(|s| s.id() == e.from)).map(|s| s.name().as_str()).collect();
            names.sort_unstable();
            names
        }

        #[test]
        fn python_class_single_method_single_call_is_owned_by_the_method_not_the_class() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/a.py",
                "class A:\n    def f(self):\n        target()\n\n\ndef target():\n    pass\n",
            );
            let target = find(&snapshots, "target");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, target, &snapshots), vec!["f"], "exactly f -> target, never A -> target");
        }

        #[test]
        fn python_class_two_sibling_methods_calling_the_same_target_produce_two_real_caller_edges_no_class_duplicate() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/svc.py",
                "class Svc:\n    def a(self):\n        target()\n\n    def b(self):\n        target()\n\n\ndef target():\n    pass\n",
            );
            let target = find(&snapshots, "target");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, target, &snapshots), vec!["a", "b"], "both sibling methods own their own call; the class must not also appear");
        }

        #[test]
        fn python_nested_function_call_is_owned_by_the_inner_function_not_the_outer() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/nested.py", "def outer():\n    def inner():\n        target()\n    return inner\n\n\ndef target():\n    pass\n");
            let target = find(&snapshots, "target");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, target, &snapshots), vec!["inner"], "only inner calls target; outer must not also appear just because inner's span is nested inside it");
        }

        #[test]
        fn python_outer_own_call_plus_nested_call_are_each_owned_by_their_real_caller() {
            // outer calls `first` directly, in its OWN body, outside
            // inner's span; inner (nested inside outer) calls `second`.
            // Expected: outer -> first (real, outer's own direct call),
            // inner -> second (real), and NEVER outer -> second (which
            // would mean outer's own re-parsed body rediscovered a call
            // that only textually exists because inner's body happens
            // to be nested inside outer's span).
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/mixed.py",
                "def outer():\n    first()\n\n    def inner():\n        second()\n    return inner\n\n\ndef first():\n    pass\n\n\ndef second():\n    pass\n",
            );
            let first = find(&snapshots, "first");
            let second = find(&snapshots, "second");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, first, &snapshots), vec!["outer"], "outer's own direct call to first must still resolve");
            assert_eq!(callers_of(&edges, second, &snapshots), vec!["inner"], "second is owned by inner alone, never also by outer");
        }

        #[test]
        fn rust_outer_call_immediately_adjacent_to_a_nested_functions_closing_brace_still_resolves() {
            // Pins the end-exclusive ByteSpan boundary
            // (`span_contains_position`'s `position < span.end`, not
            // `<=`) against the exact real byte-adjacency case where
            // the two operators diverge: `fn outer(){fn inner(){}other();}`
            // — `inner`'s own span is `[11, 23)` (ends exactly at its
            // closing `}`), and `other()`'s call expression starts at
            // EXACTLY byte 23 — i.e. `absolute_call_site_position ==
            // inner.span().end` precisely, the one position where `<`
            // and `<=` disagree. With the correct `<`, position 23 is
            // NOT inside `inner`'s `[11, 23)` span, so `outer` (whose
            // own `[0, 32)` span DOES legitimately contain position 23)
            // keeps ownership of this call, exactly as it should: this
            // is `outer`'s OWN direct call, not something `inner`
            // contains. A `<=` mutant would wrongly treat position 23
            // as still "inside" `inner`, incorrectly excluding `outer`
            // from owning a call it genuinely makes.
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/zero_gap.rs", "fn outer(){fn inner(){}other();}\n\nfn other(){}\n");
            let other = find(&snapshots, "other");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(
                callers_of(&edges, other, &snapshots),
                vec!["outer"],
                "outer's own call to other, starting at the exact byte inner's span ends, must resolve to outer — the end-exclusive boundary must not misattribute it to inner or drop it"
            );
        }

        #[test]
        fn python_outer_own_call_immediately_after_a_nested_function_definition_still_resolves() {
            // Pins the end-exclusive ByteSpan boundary
            // (`span_contains_position`'s `position < span.end`, not
            // `<=`) against a real adjacency case: `outer` defines
            // `inner` FIRST, then immediately makes its OWN direct call
            // (`after_inner()`) on the very next line, right after
            // `inner`'s own span ends. If the nested-scope boundary
            // check used `<=` instead of `<`, a call site landing
            // exactly at (or the byte comparison treating as "still
            // inside") `inner`'s own `span.end` could be wrongly
            // excluded from `outer`'s own attribution, misattributing
            // (or losing) a real call `outer` genuinely makes — the
            // opposite failure mode from B2-ID-006's original
            // over-attribution bug, but the same boundary this fix's
            // own span check must get exactly right.
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/adjacent.py",
                "def outer():\n    def inner():\n        pass\n    after_inner()\n\n\ndef after_inner():\n    pass\n",
            );
            let after_inner = find(&snapshots, "after_inner");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(
                callers_of(&edges, after_inner, &snapshots),
                vec!["outer"],
                "outer's own direct call, immediately after inner's definition ends, must still resolve to outer — not be lost or misattributed at the inner/outer span boundary"
            );
        }

        #[test]
        fn python_doubly_nested_function_call_is_owned_by_the_innermost_function_only() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/deep.py",
                "def outer():\n    def middle():\n        def inner():\n            target()\n        return inner\n    return middle\n\n\ndef target():\n    pass\n",
            );
            let target = find(&snapshots, "target");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, target, &snapshots), vec!["inner"], "neither outer nor middle must appear — only the innermost real caller");
        }

        #[test]
        fn rust_trait_default_method_call_is_owned_by_the_method_not_the_trait() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/trait_default.rs", "trait Greeter {\n    fn greet(&self) {\n        helper();\n    }\n}\n\nfn helper() {}\n");
            let helper = find(&snapshots, "helper");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, helper, &snapshots), vec!["greet"], "the default method owns its own call; the trait/Interface snapshot must not also appear");
        }

        #[test]
        fn csharp_class_method_call_is_owned_by_the_method_not_the_class() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/A.cs", "class A {\n    void F() {\n        Target();\n    }\n}\n\nclass Target2 {\n    void Target() {}\n}\n");
            // C#'s method_declaration requires a containing structure;
            // the free-standing `Target()` call resolves by bare name
            // against whatever same-named candidate exists in the pool
            // — this test only asserts the CLASS `A` never appears as a
            // caller for whichever edge(s) are produced, which is the
            // one thing B2-ID-006 is actually about (not re-testing
            // C#'s own name-resolution behavior here).
            let edges = resolve_calls(&snapshots).unwrap();
            let class_a = find(&snapshots, "A");
            assert!(edges.iter().all(|e| e.from != class_a.id()), "the C# class snapshot must never be a Calls caller");
        }

        #[test]
        fn cpp_class_method_call_is_owned_by_the_method_not_the_class() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/a.cpp", "void helper() {}\n\nclass A {\n    void f() {\n        helper();\n    }\n};\n");
            let helper = find(&snapshots, "helper");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, helper, &snapshots), vec!["f"], "the C++ class must not also appear as a caller of helper");
        }

        #[test]
        fn javascript_class_method_call_is_owned_by_the_method_not_the_class() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/a.js", "function helper() {}\n\nclass A {\n    f() {\n        helper();\n    }\n}\n");
            let helper = find(&snapshots, "helper");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, helper, &snapshots), vec!["f"], "the JS class must not also appear as a caller of helper");
        }

        #[test]
        fn method_signature_with_no_body_never_originates_a_call() {
            // Rust trait method SIGNATURE (no body) alongside a real
            // default method that DOES call something — confirms
            // MethodSignature is excluded without needing a body to
            // exclude it from (there is none to walk), and does not
            // interfere with the real default method's own edge.
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/sig.rs", "trait T {\n    fn no_body(&self);\n\n    fn with_body(&self) {\n        helper();\n    }\n}\n\nfn helper() {}\n");
            let helper = find(&snapshots, "helper");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, helper, &snapshots), vec!["with_body"], "only the real default method body owns the call");
        }

        #[test]
        fn result_is_deterministic_regardless_of_input_snapshot_order() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/order.py",
                "class Svc:\n    def a(self):\n        target()\n\n    def b(self):\n        target()\n\n\ndef target():\n    pass\n",
            );
            let target = find(&snapshots, "target");

            let forward_edges = resolve_calls(&snapshots).unwrap();
            let mut reversed = snapshots.clone();
            reversed.reverse();
            let reversed_edges = resolve_calls(&reversed).unwrap();

            assert_eq!(callers_of(&forward_edges, target, &snapshots), callers_of(&reversed_edges, target, &reversed), "caller attribution must not depend on snapshot pool ordering");
        }
    }
}
