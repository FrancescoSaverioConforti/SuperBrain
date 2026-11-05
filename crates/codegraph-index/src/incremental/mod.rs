//! B2 Task 9: incremental, in-memory re-indexing between commits.
//! Builds on Task 2's `RepositorySource` seam, Tasks 4-5's parse/
//! extract/promote pipeline, Task 6's lineage resolver, Task 7's typed-
//! edge resolvers, and Task 8's `CodeGraph` — wiring them into one
//! real, atomically-published pipeline for the first time ([`pipeline`]
//! module). No persistence, no exposed API, no MCP. Gate S1 (write-
//! path containment) continues to block any persistent write path;
//! nothing in this module writes to disk.

mod indexer;
mod pipeline;

pub use indexer::{CodeGraphIndexer, IndexMetrics, ObserveError, ObserveOutcome};
pub use pipeline::{index_files, PipelineError, PipelineOutput};
