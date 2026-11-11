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
mod resolution;
mod tests_for;

pub use calls::resolve_calls;
pub use contains::resolve_contains;
pub use extends_implements::{resolve_extends_implements, resolve_extends_implements_with_csharp_contexts};
pub use instantiates::resolve_instantiates;
pub use resolution::{Edge, EdgeResolutionError};
pub use tests_for::resolve_tests_for;

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
    let mut edges = Vec::new();
    edges.extend(resolve_calls(snapshots)?);
    edges.extend(resolve_extends_implements_with_csharp_contexts(snapshots, csharp_contexts)?);
    edges.extend(resolve_tests_for(snapshots)?);
    edges.extend(resolve_contains(snapshots)?);
    edges.extend(resolve_instantiates(snapshots)?);
    Ok(edges)
}
