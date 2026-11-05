//! B2 Task 8: the in-memory `CodeGraph` — the storage and query core that
//! owns every `SymbolSnapshot`, `LineageEdge` and typed `Edge` this
//! pipeline has resolved, and the traversal primitives a future
//! `get_code_context`/`graph_query` (out of this task's scope) will be
//! built on. No persistence, no API, no MCP, no Task 9 incremental
//! indexing — this module only holds what it is given, in memory, for
//! the lifetime of one `CodeGraph` value.
//!
//! # Invariants this graph enforces on insertion
//!
//! - **No duplicate snapshot id.** Inserting a `SymbolSnapshot` whose
//!   `SymbolSnapshotId` is already present is rejected
//!   ([`GraphError::DuplicateSnapshot`]) — never a silent overwrite, and
//!   never a silent no-op dedup. A caller that needs idempotent re-
//!   insertion of the exact same snapshot must check first; this graph
//!   does not infer that two insertions with the same id are "the same
//!   call happening twice" rather than a real bug upstream.
//! - **No dangling edge.** A `LineageEdge`/`Edge` whose `from` or `to`
//!   does not name a snapshot already present in the graph is rejected
//!   ([`GraphError::DanglingEdgeFrom`]/[`GraphError::DanglingEdgeTo`]).
//!   Snapshots must be inserted before any edge that references them —
//!   this graph never creates a placeholder node to make a dangling
//!   reference "work."
//! - **No duplicate edge.** The same typed edge (`from`, `to`, `kind`) —
//!   or the same lineage edge (`from`, `to`) — inserted twice is
//!   rejected ([`GraphError::DuplicateEdge`]). Two DIFFERENT edges
//!   between the same pair with different `EdgeKind`s (e.g. both
//!   `Calls` and `References` between the same two snapshots) are not
//!   duplicates of each other and are both accepted; this graph does not
//!   silently collapse them. `Evidence` is deliberately NOT part of the
//!   duplicate key — two attempts to insert the exact same `(from, to,
//!   kind)` triple are the same logical edge even if a caller
//!   constructed its `Evidence` slightly differently (e.g. a different
//!   note string) each time, so the second insertion is rejected as a
//!   duplicate rather than silently admitted as a "different" edge that
//!   would otherwise let the SAME relationship be recorded twice.
//! - **No cross-repository edge.** An edge whose `from` and `to`
//!   snapshots belong to different `RepositoryId`s is rejected
//!   ([`GraphError::CrossRepositoryEdge`]) — nothing in this pipeline's
//!   scope (no cross-repo resolution exists anywhere in Tasks 4–7) ever
//!   legitimately produces one, so admitting it here would hide an
//!   upstream bug rather than surface it.
//! - **Known repository only.** A `SymbolSnapshot` is accepted
//!   regardless of which specific repository/commit it names — this
//!   graph does not maintain its own separate allowlist of "known"
//!   repositories (that is `RepositoryAllowlist`'s job, a different
//!   layer). What this graph DOES enforce is that every edge's
//!   endpoints must already be snapshots IN THIS GRAPH (the dangling-edge
//!   rule above), which transitively means every edge's snapshots belong
//!   to whatever repositories/commits were actually inserted.
//!
//! # Cycle safety
//!
//! `A calls B` + `B calls A` is a legitimate, representable graph shape
//! (mutual/indirect recursion is common in real code) — this graph never
//! rejects a cycle on insertion. Every traversal primitive
//! ([`CodeGraph::reachable_from`]) carries its own visited-set and
//! therefore always terminates, regardless of how many cycles the
//! underlying edges form. This task does not add Task 13's final
//! traversal budgets (depth/node-count limits for a public query
//! surface) — only the structural guarantee that a cycle can never cause
//! non-termination in the primitives this task adds.
//!
//! # Determinism
//!
//! Every method that returns a collection returns it in a fixed,
//! explicit order — never raw `HashMap`/`HashSet` iteration order.
//! Snapshots are ordered by `SymbolSnapshotId`'s own byte representation
//! ([`SymbolSnapshotId::as_bytes`], the same total ordering
//! `lineage::resolver` already uses for its own tie-breaking); edges are
//! ordered by `(from bytes, to bytes, kind)`. Internally, lookup indexes
//! use `HashMap`/`HashSet` for O(1) membership/adjacency, but every
//! observable return value is sorted explicitly before being handed back
//! — the hash collections are an implementation detail, never a
//! visible source of non-determinism.
//!
//! # Evidence preservation
//!
//! This graph stores `LineageEdge`/`Edge` values as given, verbatim — it
//! never summarizes, drops, or re-derives their `Evidence`,
//! `Confidence`, or `LineageStatus`. A `Hypothesized` lineage retrieved
//! from this graph is still `Hypothesized`, with the same `Evidence` and
//! `Confidence` it was inserted with; a heuristic typed `Edge` is still
//! heuristic. This graph has no method that could "simplify" an edge
//! into a weaker or stronger status than it was given.
//!
//! # Known gaps carried forward from Task 7 (not fixed here)
//!
//! 1. Calls inside Rust macro invocations (e.g. `assert_eq!(f(x), y)`)
//!    are invisible to `calls.rs`'s tree-sitter walk — macro arguments
//!    parse as an opaque `token_tree`, never real `call_expression`
//!    nodes. This graph stores whatever `Calls` edges it is given; it
//!    cannot and does not attempt to recover edges `calls.rs` never
//!    produced.
//! 2. `Imports` edges essentially never fire against real Task-4-
//!    extracted data, because a symbol's `signature_text` never includes
//!    a file-level import statement outside that symbol's own span —
//!    only synthetic test fixtures that inline the import text exercise
//!    `imports.rs`'s logic. This graph will simply contain very few (or
//!    zero) `Imports` edges on a real repository until that extraction
//!    gap is closed in a future task.
//!
//! Both are recorded here again (not just in Task 7's own module docs)
//! because this is the module a future real-repository validation pass
//! (planned immediately after Task 9) will read first to understand
//! what the graph can and cannot yet represent.

mod deep_learning_fixture;
mod store;
mod traversal;

pub use store::{CodeGraph, GraphError};
pub use traversal::Direction;
