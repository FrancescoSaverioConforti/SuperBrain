//! SuperBrain M7.7: composition layer connecting
//! `mcp-audit-bridge`'s own `DestinationAccess` core to
//! `audit-segmentation`'s segment lifecycle/retention-pin machinery.
//!
//! **The only crate permitted to depend on both siblings** — neither
//! `mcp-audit-bridge` nor `audit-segmentation` depends on this crate
//! or on each other. This crate reimplements NONE of either sibling's
//! own domain logic: it resolves segments, acquires/releases pins, and
//! adapts the result to `mcp_audit_bridge::DestinationAccess` — the
//! bridge's own `run_with_destination` is the ONE place mapping/dedup/
//! registration/checkpoint logic lives, called here, never duplicated.

pub mod destination;
pub mod pinned_access;
pub mod trusted_root;

pub use destination::{ResolvedSegment, SegmentedDestination, SegmentedDestinationError};
pub use pinned_access::{SegmentedBridgeRun, SegmentedRunError};
pub use trusted_root::TrustedSegmentationRoot;
