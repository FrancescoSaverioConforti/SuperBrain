//! SuperBrain M6.2B: pure, domain-agnostic hash-chain and record-envelope
//! primitives, extracted from `brain-api::audit_chain`/`audit_record`
//! (M5 Tasks 1 and 4) after M6.2A's golden-vector baseline proved the
//! extraction could be made byte-for-byte behavior-preserving.
//!
//! **Scope discipline (unchanged from the pre-extraction modules this
//! crate was extracted from)**: no file I/O, no locking, no DPAPI, no
//! network, no Tokio -- `cargo tree -p audit-chain-core` has no such
//! dependency, by construction (see this crate's own `Cargo.toml`).
//! Event-specific interpretation and domain types (`AuditEvent`,
//! `AuditEventKind`, `AuditSink`) stay in `brain-api` -- this crate knows
//! nothing about what an event MEANS, only how to hash-chain and
//! serialize/parse an opaque, serializable value.
//!
//! **Why extract now**: `brain-api-audit-worker`'s own future sibling
//! for MCP Client audit payloads needs this same hash-chain/record
//! format without depending on `brain-api`, `storage` or `mcp-server`
//! (B5.10's architecture test keeps `mcp-client` dependency-isolated;
//! this crate preserves that same isolation for a future audit payload
//! type, per M6.2B's own requirement 6).
//!
//! **Design basis**, reusing an established pattern already proven in
//! this codebase (`crates/vault-security/src/deletion_log.rs`'s own
//! `chain_hash = sha256(prev_chain_hash || generation || note_id)`
//! formula and genesis convention), not inventing a new one:
//! `record_hash = sha256(prev_record_hash || sequence || content_bytes)`,
//! with genesis `prev_record_hash` = 64 ASCII `'0'` characters -- the
//! exact same genesis convention `deletion_log.rs` already uses.
pub mod chain;
pub mod record;

pub use chain::{
    compute_record_hash, may_resume_without_recovery, scan_chain, verify_link, verify_record_hash,
    ChainLinkError, ChainRecord, LineOutcome, ScanResult, ScanStopReason, GENESIS_HASH,
};
pub use record::{parse_event, parse_line, serialize_record, AuditRecordParseError, AUDIT_RECORD_FORMAT_VERSION};
