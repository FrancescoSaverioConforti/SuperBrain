//! B2 Task 7: typed, non-lineage edges between `SymbolSnapshot`s —
//! `Calls`, `Implements`, `Extends`, `References`, `TestsFor`. No graph
//! storage (Task 8), no persistence, no API, no MCP, no Task 9
//! incremental indexing.
//!
//! # `Imports` — removed, see B2-ID-005
//!
//! Task 7 originally added a symbol-to-symbol `EdgeKind::Imports`
//! (`resolve_imports` in this module), resolved by re-parsing each
//! `SymbolSnapshot`'s own `signature_text` for import-shaped nodes.
//! Task 9.5's Priority 6 real-repository evaluation
//! ([B2-EVAL-001](../../../../docs/research/evaluations/B2-EVAL-001.md))
//! empirically confirmed this captured 0 of 589 real import statements
//! in a 97-file corpus: an import statement is a FILE/module-level
//! fact, written above the symbols it applies to, and never inside any
//! one symbol's own byte span — so it was never present in the text
//! this design re-parsed. The granularity itself (symbol-to-symbol)
//! was the defect, not a fixable bug in the matching logic, so it was
//! REMOVED rather than patched. See
//! [B2-ID-005](../../../../docs/research/findings/B2-ID-005.md) for the
//! full analysis and its replacement, `crate::imports::ImportFact` — a
//! separate, file-level fact model, deliberately NOT expressed as a
//! `SymbolSnapshot`-to-`SymbolSnapshot` `Edge` in this module.
//!
//! # The one rule that governs everything in this module
//!
//! A heuristic relationship must never be indistinguishable from a
//! resolved one. Every [`Edge`] carries [`Evidence`], and this module's
//! only constructor of `StaticAnalysis`-method evidence
//! ([`resolved_evidence`]) is used exclusively when a target symbol was
//! found with syntactic certainty (an exact-name match within the
//! snapshots this resolver was given, for a syntactically unambiguous
//! relationship: `impl Trait for Type`, `class X extends Y`,
//! `class X implements Y`, a call to a name resolving to exactly one
//! candidate). Every other case — an ambiguous target, a dynamically-
//! dispatched call, a target not found among the known snapshots, a
//! same-name-multiple-candidates situation, or a convention-based guess
//! (e.g. `TestsFor` from a naming pattern alone) — uses
//! [`heuristic_evidence`] with an explicit [`Confidence`], never
//! `StaticAnalysis`. No edge is constructed at all when the evidence
//! does not even clear a minimum plausibility bar — "no edge beats a
//! false edge," the same principle Task 6's lineage resolver already
//! applies.
//!
//! # Deep-learning repository forward-compatibility
//!
//! This task does not add a `Field`/`Attribute` `SymbolKind` (out of
//! scope per the B2 plan's section 8b coverage policy — no real need
//! within Task 7's own scope forces it). But [`Edge`]'s shape (a
//! `from`/`to` pair of `SymbolSnapshotId`s plus `EdgeKind`/`Evidence`,
//! with the resolver side looking up targets by name among whatever
//! snapshots it was given) does not assume `from`/`to` must be
//! functions/classes specifically — nothing here prevents a FUTURE
//! `Field`/`Attribute` snapshot from participating in a `References`
//! edge the same way a function does, once that `SymbolKind` variant
//! exists. Concretely, `forward calls encoder`/`forward calls
//! classifier` (a method calling what is, today, an unresolved
//! attribute access `self.encoder(...)`) already resolves as a `Calls`
//! edge to whatever snapshot's name matches `encoder`/`classifier`
//! among the known snapshots — if a future `Field` snapshot for
//! `self.encoder = TransformerEncoder(...)` exists with that name, this
//! same resolution logic finds it without any change to this module.

mod calls;
mod contains;
mod extends_implements;
mod instantiates;
mod js_prototype_ownership;
mod resolution;
mod tests_for;

pub use calls::{resolve_calls, resolve_calls_with_extraction_rejections, resolve_calls_with_imports, resolve_calls_with_namespace_context, resolve_calls_with_negative_knowledge};
pub use contains::resolve_contains;
pub use extends_implements::{resolve_extends_implements, resolve_extends_implements_with_csharp_contexts};
pub use instantiates::resolve_instantiates;
pub use js_prototype_ownership::resolve_js_prototype_ownership;
pub use resolution::{Edge, EdgeResolutionError};
pub use tests_for::resolve_tests_for;

use crate::imports::ImportFact;
use crate::snapshot::SymbolSnapshot;

/// Runs EVERY typed-edge resolver this module owns against the same
/// snapshot pool, in one place. Task 9.5 introduced this after finding
/// the 4-resolver call block independently duplicated between
/// `incremental::pipeline::index_files` and
/// `incremental::indexer::incremental_reindex` — a real risk that a
/// future new resolver (like this task's own `Contains`/`Instantiates`)
/// would be wired into one call site and silently omitted from the
/// other, making the incremental path quietly diverge from a full
/// reindex. Both call sites now go through this single function
/// instead of their own independent lists.
pub fn resolve_all_typed_edges(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    resolve_all_typed_edges_with_csharp_contexts(snapshots, &[])
}

/// B2-ID-010: identical to [`resolve_all_typed_edges`] except
/// `Extends`/`Implements` resolution also receives `csharp_contexts`,
/// enabling the partial-class reconciliation
/// [`extends_implements::resolve_extends_implements_with_csharp_contexts`]
/// implements. With an empty `csharp_contexts` slice this is
/// byte-for-byte identical to [`resolve_all_typed_edges`] — confirmed
/// by a dedicated equivalence test.
pub fn resolve_all_typed_edges_with_csharp_contexts(snapshots: &[SymbolSnapshot], csharp_contexts: &[crate::csharp_declarations::CSharpTypeDeclarationContext]) -> Result<Vec<Edge>, EdgeResolutionError> {
    resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge(snapshots, csharp_contexts, &[], &[]).map(|(edges, _attempts)| edges)
}

/// Semantic Resolution & Reliability Checkpoint, Priority 1 + 3; CALL-RES-A4
/// wiring: identical to [`resolve_all_typed_edges_with_csharp_contexts`]
/// except `Calls` resolution also returns its own negative-knowledge
/// `CallAttempt` records, and now runs through
/// [`calls::resolve_calls_with_namespace_context`] — CALL-RES-A4's own
/// import-alias-aware (Python) and namespace-aware (C#) resolver —
/// rather than the pre-A4 [`resolve_calls_with_extraction_rejections`].
/// `resolve_calls_with_namespace_context` itself falls back to
/// byte-identical pre-A4 behavior whenever `csharp_contexts` is empty
/// (which falls back again to byte-identical behavior whenever
/// `import_facts` is empty or has no matching alias for a given call) —
/// so passing `&[]` for either or both of `import_facts`/`csharp_contexts`
/// reproduces [`resolve_calls_with_extraction_rejections`]'s own output
/// exactly, confirmed by a dedicated equivalence test, mirroring exactly
/// how every prior "_with_X" extension in this module (B2-ID-010's own
/// `_with_csharp_contexts`) preserved its own predecessor's behavior.
pub fn resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge(
    snapshots: &[SymbolSnapshot],
    csharp_contexts: &[crate::csharp_declarations::CSharpTypeDeclarationContext],
    signature_too_long_names: &[String],
    import_facts: &[ImportFact],
) -> Result<(Vec<Edge>, Vec<crate::negative_knowledge::CallAttempt>), EdgeResolutionError> {
    let mut edges = Vec::new();
    let (calls_edges, call_attempts) = resolve_calls_with_namespace_context(snapshots, signature_too_long_names, import_facts, csharp_contexts)?;
    edges.extend(calls_edges);
    edges.extend(resolve_extends_implements_with_csharp_contexts(snapshots, csharp_contexts)?);
    edges.extend(resolve_tests_for(snapshots)?);
    edges.extend(resolve_contains(snapshots)?);
    edges.extend(resolve_instantiates(snapshots)?);
    edges.extend(resolve_js_prototype_ownership(snapshots)?);
    Ok((edges, call_attempts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, EdgeKind, FileIdentity, RepositoryRegistry, RepositoryRoot};

    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::promote;

    fn test_repo() -> codegraph_domain::RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    fn test_snapshot(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str, start: usize, end: usize) -> SymbolSnapshot {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start, end }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    /// Neither `resolve_all_typed_edges` nor
    /// `resolve_all_typed_edges_with_csharp_contexts` had any direct
    /// caller anywhere in this crate (production code goes straight to
    /// `resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`)
    /// — confirmed by `cargo mutants` finding both thin wrappers'
    /// entire bodies replaceable with `Ok(vec![])` without a single
    /// test failing. These two tests close that real test-coverage gap
    /// directly: a hand-built class/method pair produces a real
    /// `Contains` edge via `resolve_contains`, which both wrappers
    /// include — proving each one genuinely calls through to its
    /// documented richer counterpart rather than returning an empty
    /// Vec unconditionally.
    #[test]
    fn resolve_all_typed_edges_actually_resolves_a_real_contains_edge() {
        let repo = test_repo();
        let class = test_snapshot(repo, "src/m.py", "MyModel", "class_definition", "class MyModel:\n    def forward(self): pass\n", 0, 100);
        let method = test_snapshot(repo, "src/m.py", "forward", "function_definition", "def forward(self): pass", 20, 90);
        let snapshots = vec![class.clone(), method.clone()];

        let edges = resolve_all_typed_edges(&snapshots).unwrap();

        assert!(
            edges.iter().any(|e| e.kind == EdgeKind::Contains && e.from == class.id() && e.to == method.id()),
            "resolve_all_typed_edges must actually run resolve_contains, not silently return an empty edge set"
        );
    }

    #[test]
    fn resolve_all_typed_edges_with_csharp_contexts_actually_resolves_a_real_contains_edge() {
        let repo = test_repo();
        let class = test_snapshot(repo, "src/m.py", "MyModel", "class_definition", "class MyModel:\n    def forward(self): pass\n", 0, 100);
        let method = test_snapshot(repo, "src/m.py", "forward", "function_definition", "def forward(self): pass", 20, 90);
        let snapshots = vec![class.clone(), method.clone()];

        let edges = resolve_all_typed_edges_with_csharp_contexts(&snapshots, &[]).unwrap();

        assert!(
            edges.iter().any(|e| e.kind == EdgeKind::Contains && e.from == class.id() && e.to == method.id()),
            "resolve_all_typed_edges_with_csharp_contexts must actually run resolve_contains, not silently return an empty edge set"
        );
    }
}
