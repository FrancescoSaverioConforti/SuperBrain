//! B2 Task 7: typed, non-lineage edges between `SymbolSnapshot`s —
//! `Calls`, `Imports`, `Implements`, `Extends`, `References`, `TestsFor`.
//! No graph storage (Task 8), no persistence, no API, no MCP, no Task 9
//! incremental indexing.
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
mod extends_implements;
mod imports;
mod resolution;
mod tests_for;

pub use calls::resolve_calls;
pub use extends_implements::resolve_extends_implements;
pub use imports::resolve_imports;
pub use resolution::{Edge, EdgeResolutionError};
pub use tests_for::resolve_tests_for;
