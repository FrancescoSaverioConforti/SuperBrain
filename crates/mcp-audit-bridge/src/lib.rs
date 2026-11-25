//! SuperBrain M7: the sole cross-boundary convergence point between the
//! MCP client audit chain (`mcp-client-audit-worker`) and the Brain
//! audit chain (`brain-api`/`brain-api-audit-worker`).
//!
//! **Trust boundary this crate exists to draw**: `mcp-client`,
//! `mcp-client-audit-worker` and `brain-api` all stay dependency-isolated
//! from each other (verified by their own `Cargo.toml`s and architecture
//! tests) -- this crate is the ONLY place in the workspace permitted to
//! depend on both sides at once, and it is a pure translation/import
//! layer: it defines no new execution path, no new write authority, and
//! no new persisted chain format of its own.
//!
//! **Mode (M7, frozen)**: batch/offline only. This crate runs to
//! completion once per invocation over whatever new source records exist
//! since its last successful import, then stops -- never a long-lived
//! concurrent process racing the real MCP client or Brain audit writers.
//!
//! **Semantics (M7, frozen)**: at-least-once source processing with
//! idempotent destination dedup -- never "exactly-once" in the
//! distributed-transaction sense, since there is no atomic cross-chain
//! commit. The externally observable effect is "at most one Brain fact
//! per source record," achieved by verifying, before every append,
//! whether that exact source record has already been durably
//! materialized in the destination chain.
pub mod anti_bypass;
pub mod bridge_lock;
pub mod checkpoint;
pub mod dedup;
pub mod destination;
pub mod identity;
pub mod mapping;
pub mod registration;
pub mod run;

pub use anti_bypass::{guard_against_segmented_destination, SegmentedDestinationGuardError};
pub use bridge_lock::{BridgeLock, BridgeLockError};
pub use checkpoint::{BridgeCheckpoint, CheckpointError};
pub use dedup::{scan_destination_for_imported_identities, DestinationAnomaly};
pub use destination::{DestinationAccess, DestinationAccessError, DestinationHistory, HistoryCompleteness, SingleVaultDestination};
pub use identity::{RelocationError, SourceLogId, SourceLogRegistration, SourceRecordIdentity};
pub use mapping::translate;
pub use registration::{RegistrationError, RegistrationFile, RegistrationRecoveryError};
pub use run::{register_source, run_once, run_with_destination, RegisterSourceError, RunConfig, RunFailure, RunOutcome, RunReport, SourceConfig};
