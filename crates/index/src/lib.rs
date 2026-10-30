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
//!   embedding, summary, attachment and cache surfaces. Its index-surface
//!   portion is being closed specifically by Task 4 of the B1
//!   deterministic-search plan. Until that task lands, the index surface
//!   must not be treated as audited; the other four surfaces remain open
//!   for B2/C2/future phases.
//!
//! # B1 Task 1
//!
//! [`indexer_impl`] adds the first concrete piece: a skeleton
//! `storage::Indexer` implementation (no real indexing logic — that is
//! Tasks 2/3) that proves the integration seam Task 1 designed is usable
//! from outside `storage`. See that module's doc comment for the full
//! design note, including the Step 5 scalability-benchmark protocol Task 9
//! will execute once real indexes exist.
//!
//! # B1 Task 2
//!
//! [`document`] defines `Document`, the shared shape built from a parsed
//! note's metadata (plus its already-split body) that both `TrigramIndex`
//! (this task) and `Bm25Index` (Task 3) consume. [`trigram`] defines
//! `TrigramIndex` itself: a standalone insert/remove/exact-query postings
//! structure over `Document::text_searchable`, with no knowledge of
//! structured metadata, classification, or filters. Neither module wires
//! into `SkeletonIndexer` yet — that real-indexer wiring is Task 3's job.

pub mod document;
pub mod indexer_impl;
pub mod trigram;
