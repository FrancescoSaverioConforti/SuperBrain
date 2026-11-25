//! SuperBrain M7.5/M7.6: audit log segment lifecycle (rotation), a
//! dedicated-Vault retention ledger, and the 5-phase retention protocol
//! — built entirely on top of the existing, unmodified
//! `storage::audit_writer`.
//!
//! **Frozen invariants, unchanged by this crate**:
//! `audit_chain_core::AUDIT_RECORD_FORMAT_VERSION`, `ChainRecord`,
//! `storage::audit_writer`'s own semantics, the `.audit/audit.log` +
//! `head.ref` contract, and M5/M6's own append/recovery semantics. This
//! crate introduces no new persisted record format for an audit
//! segment or the retention ledger — both are ordinary, unmodified
//! `storage::Vault`s, written to exclusively through
//! `Vault::open_audit_writer`/`AuditWriterHandle::append`, exactly as
//! every other composition root in this workspace already does.
//!
//! **`segment-manifest.json` is an accelerator, never an authority**
//! (the frozen rule) — see `domain::SegmentManifest`'s own doc comment.
//!
//! **Sibling, not child, of `mcp-audit-bridge`**: this crate owns
//! segment lifecycle, rotation, the retention ledger, retention/
//! recovery, and end-to-end segment verification. It has no dependency
//! on `mcp-client`, `mcp-client-audit-worker`, `mcp-audit-bridge`, or
//! `mcp-server` — MCP import/translation is `mcp-audit-bridge`'s own,
//! separate responsibility.
pub mod adoption;
pub mod domain;
pub mod ledger;
pub mod lifecycle_lock;
pub mod manifest_store;
pub mod pin_store;
pub mod recovery;
pub mod retention;
pub mod rotation;
pub mod verifier;

pub use adoption::{adopt_existing_vault, AdoptionError};
pub use domain::{OwnerClass, OwnerClassError, PinAcquired, PinReason, PinReleased, RetentionReceipt, SegmentId, SegmentIdError, SegmentManifest, SegmentManifestEntry, SegmentManifestError, SegmentStatus, SEGMENT_MANIFEST_FORMAT_VERSION};
pub use ledger::{LedgerError, VerifiedLedgerState};
pub use lifecycle_lock::{LifecycleLock, LifecycleLockError};
pub use manifest_store::ManifestStoreError;
pub use pin_store::{PinStoreError, PinStoreRecord, VerifiedPinStoreState};
pub use recovery::{classify, refine_with_manifest_projection, RecoveryState};
pub use retention::{retire_segment, RetentionConfig, RetentionError, RetentionReport};
pub use rotation::{rotate, RotationConfig, RotationError, RotationReport};
pub use verifier::{verify, VerificationFinding, VerificationReport, VerifierInput};
