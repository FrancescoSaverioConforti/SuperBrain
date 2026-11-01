---
schema: 1
id: n2io
type: next_action
title: >-
  B1/B2/C2 plans must re-audit the sensitive-classification confidentiality
  boundary against index, embedding, summary, attachment, and cache surfaces
  once those subsystems exist
status: open
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T08:21:18.678Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

A3's Task 3 (crates/storage/tests/a3_confidentiality_boundary.rs) explicitly defers 5 of C3's 10 listed surfaces (indici, embedding, riassunti, allegati, cache) because no such subsystem exists yet in the codebase. File content, metadata, log output, generated/temp files, and backup are covered by A3's stubs. This open item is the reminder to close that gap when B1/B2 (index) or C2 (summaries) land.

## Partial update - B1 Task 4 (recorded 2026-10-30)

Verified against docs/handoffs/B1-to-codex.md section 8 and the reviewer-verified Task 4 report. Status remains open; summary and attachment surfaces also remain open.

> **[n2io] — index surface, partial closure (B1 Task 4).** The in-memory `TrigramIndex`/`Bm25Index` postings structures and the `RealIndexer::mark_current` log/error-output path are now confidentiality-audited: a Sensitive note's `source` field and `relations` targets (the two allowlist-excluded fields, alongside the body already covered by Tasks 2/3) are proven, via `crates/index/tests/b1_confidentiality_boundary.rs`, to never reach either postings structure's query results — tested both via direct `Document` construction and via the full `propose_and_commit` → `RealIndexer::mark_current` production write path. The log/error-output surface (`Document::from_record_bytes`'s error path, `storage::SplitRecordError`'s `Debug`/`Display`, and `RealIndexer::mark_current`'s absorption of both a malformed record and a `poison_next_mark_current`-injected fault) is confirmed to never echo note content, including a distinctive marker standing in for sensitive content. **This closes [n2io] only for the surfaces that exist today.** The `SearchResponse`/search-API-response surface named in the original item remains **open**, pending Task 6 (which defines that type) — do not treat [n2io] as fully closed until that surface is also audited. Embedding and cache surfaces remain open as before, unrelated to this task: no such subsystems exist anywhere in this codebase yet.

## Partial update - B1 Task 6 (2026-10-30)

The core SearchResponse surface deferred by Task 4 is now implemented and audited
in crates/index/tests/b1_search.rs: separate production-path canary tests prove
Sensitive body, source and relations never appear in results or Debug, while
structured-field matches remain available. Reclassification Internal -> Sensitive
between indexing and search re-applies the current record's allowlist. Every
candidate is read through Vault::read_note, re-derived, query-matched, filtered
and rescored from the current committed version. Deleted/missing/inconsistent
candidates are excluded; operational/deletion-log failures produce a sanitized
ReadUnavailable error rather than a successful NotPresent response. Independent
review GO after one Important finding was fixed and re-reviewed. See
eval/results/B1/task-6.md for evidence and remaining limitations.

The B1 core index/postings/log/SearchResponse surfaces are covered. This does not
close n2io wholesale: embedding, summary, attachment and cache remain open for
future phases, and future API/transport wrappers must preserve the audited core
behavior. Direct retrieve legitimately retains full pointer-record frontmatter;
this search-only allowlist does not impose new retrieve redaction.

## Qualification — final B1 review (2026-11-01)

Status remains open. The previous body/source/relations exclusion checks still
pass, but the final independent review confirmed Important B1-SEC-001: arbitrary
String values in created/updated/event_period pass schema and gate and enter
Sensitive searchable text. The approved allowlist's assumption that these fields
are constrained dates is not enforced. A standalone schema/gate/Document/index
reproduction is retained in eval/results/B1/sensitive-date-metadata-repro.rs;
the scope and NO-GO verdict are in eval/results/B1/evidence.md. No claim of full
metadata confidentiality closure is made. Validation/legacy-value handling or
a narrower Sensitive search surface requires an explicit decision; no fix or
canonical-spec amendment was applied.
