//! Deterministic search index over vault notes (scaffold).
//!
//! This crate currently contains no search logic; later B1 tasks add the
//! trigram, BM25 and query layers. It depends on `domain` for note types and
//! on `storage` for vault access, and reads note content only through
//! `Vault::read_note` / `Vault::current_version`.
//!
//! # Security and confidentiality status (documentation anchor, not a test)
//!
//! This module intentionally contains no `#[test]` and asserts nothing. It
//! records the status that future readers of the index surface need to know:
//!
//! - **SEC-001 / SEC-002 / SEC-003 are unaffected by this plan.** `index` never
//!   calls `record_path_checked` or `ensure_resolves_inside_root` directly. It
//!   only calls `Vault::read_note` and `Vault::current_version`, which already
//!   perform those checks internally. Any change to those checks is still
//!   governed by their own tests in `vault.rs`, not by anything in this crate.
//! - **[n2io] is not closed by this crate existing.** [n2io] is the deferred
//!   confidentiality audit (carried over from A3) covering the index,
//!   embedding and cache surfaces. Its index-surface portion is being closed
//!   specifically by Task 4 of the B1 deterministic-search plan. Until that
//!   task lands, the index surface must not be treated as audited.
