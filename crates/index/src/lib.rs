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
//! - **[n2io] remains partially open.** Task 4 audited the in-memory
//!   trigram/BM25 postings and indexing log/error-output paths. The search
//!   response surface remains open for Task 6's dedicated body/source/relations
//!   tests. Embedding, summary, attachment and cache surfaces remain open for
//!   their future phases. See `.continuity/claims/n2io.md` and the B1 handoff.
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
//! structured metadata, classification, or filters. Neither module wired
//! into a real indexer yet — that real-indexer wiring was Task 3's job.
//!
//! # B1 Task 3
//!
//! [`bm25`] defines `Bm25Index`: the second (ranked) postings structure
//! over `Document::text_searchable`, with the same text-only contract
//! `TrigramIndex` has. [`indexer_impl`] now also defines `RealIndexer`, the
//! first real `storage::Indexer` implementor: it wires both `TrigramIndex`
//! and `Bm25Index` behind `mark_current`/`state_for`, and
//! `indexer_impl::rebuild_index` is the cold-start rebuild function that
//! populates a fresh `RealIndexer` from every live note a `Vault` holds,
//! using `Vault::list_note_ids` + `Vault::read_note` outside any writer's
//! lock. `SkeletonIndexer` is kept alongside `RealIndexer`, unchanged, as
//! the Task 1 seam-proof it always was — it is not used in production.

pub mod bm25;
pub mod document;
pub mod indexer_impl;
pub mod time_range;
pub mod trigram;
