# B1 Task 6 - search, filters and citations

Date: 2026-10-30. Baseline: 80cab4498ba910a9220efcacc3d794fdad728a50 on
b1-deterministic-search; clean before the task. Task 7 has not started.

## Delivered behavior

`index::Index` holds a vault, real indexer, active project and explicit reference
date. `search(query, SearchFilters, k)` returns a typed Result. The response always
contains project scope and the applied temporal field, Rome interval, timezone,
reference date and calendar convention when filtering by time. Matches is
structurally nonempty (first + rest); NotPresent is distinct. A mandatory
completeness value states that indexed candidates can be incomplete.

Candidates are the untruncated union of trigram literal-substring hits and BM25
token hits. Only ids cross that boundary. Locks are released before any vault
read, preserving the no-callback-under-writer-lock architecture. For each id:

1. Read through Vault::read_note's committed-version/deletion/integrity gates.
2. Split with storage::split_record and construct a new Document from the CURRENT
   metadata and body. The id must match; the current classification is authoritative.
3. Recheck literal substring / shared BM25 tokenization against fresh allowlisted
   searchable text; skip a candidate that no longer matches.
4. Reapply project/type/status/trust/date filters against that fresh Document.
5. Score a new ephemeral BM25 corpus made only of fresh eligible candidates;
   derive the excerpt and provenance from the same record. Sort deterministically
   by score descending/id ascending; apply k only now.

No candidate can reach the response from cached content or scores. Sensitive
excerpts use only allowlisted structured metadata; source is None and relation
targets are never response fields. Public/Internal excerpts use the current body
when it matches, otherwise searchable metadata so the excerpt contains evidence
of the match. Direct retrieve is unaffected by search-only redaction.

## Tests and TDD

The initial 23 named production-path tests compiled and all failed on the
unimplemented search function (RED); all passed after implementation (GREEN). They cover
explicit absence/project scope, four distinct temporal modes, August events
created today, differing event/updated results and annotations, deletion through
tombstone propose_and_commit, current v2 body/version, stale v1 matches absent
from v2, current filters and fresh scoring/top-k, exact and ranked paths,
metadata-only evidence, malformed/missing records, date/interval boundaries,
Rome offsets, clarification, typed invalid input and deterministic ties.

Three separate canary tests exercise Sensitive body/source/relations through the
real propose_and_commit -> Vault -> RealIndexer -> SearchResponse path, checking
both results and Debug while confirming the note is still found via structured
metadata. Another test pauses index updates and reclassifies Internal to Sensitive,
proving old body/source/relation matches disappear and current provenance is safe.

Review added a 24th regression test: corrupt the deletion log after indexing a
note, confirm read_note returns DeletionLog, and require search to return an
error. It failed against the initial code, then passed after the fix. The 25th
test verifies honest C6 reference-state reporting from current metadata.

Final commands:

- cargo build --workspace --locked: passed.
- cargo test --workspace --locked --quiet: 479 passed, 0 failed, 0 ignored.
  Counts: domain 32; index unit 62; index confidentiality 6; index search 25;
  policy 14; storage unit 201; storage integration 4+6+14+24;
  vault-security 86; storage doctests 5.
- cargo clippy --workspace --locked -- -D warnings: passed, zero warnings.
- cargo deny check --hide-inclusion-graph: advisories/bans/licenses/sources OK,
  with existing duplicate-version and unused-allowance warnings.
- git diff --check and rustfmt of the two new Rust files: passed.

An attempted cargo-deny --disable-fetch option was unsupported by the installed
version; the standard command above was used successfully. No dependency or
lockfile changes. Mutation testing and scalability remain Task 9 work.

## Independent review

GO after one fix round, no Critical/Important residual findings. The independent
task6_review agent personally ran 62 unit + 6 confidentiality + 23 initial search
tests; after the fix they reran all 25 search tests and workspace Clippy clean.
They inspected the mandatory pipeline, current classification, confidentiality,
project/time context, nonempty response shape, ordering/top-k and fresh scoring.

Important finding fixed: blanket skipping of read errors incorrectly converted
IO/journal/deletion-log failure into Ok(NotPresent). Search now silently skips
only Deleted, NoteDeleted, NotFound and Inconsistent. Every other read error
returns sanitized SearchError::ReadUnavailable, without raw content/id/path.

## Explicit limits and integration choices

- Scores are BM25 within this request's fresh matching/filter-eligible candidates,
  not a stale full-vault corpus. Exact substring-only hits can score zero and
  carry MatchKind::ExactSubstring. Scores are comparable within a response,
  not across differently filtered queries. Ties use note id.
- Excerpts are the full matching body/searchable surface. B1 has no truncation,
  summarization, timeout or transport limits yet; later wrappers must document
  their own limits rather than imply these exist.
- Date metadata supports YYYY-MM-DD, RFC3339, and (for event_period) start/end
  half-open intervals in those forms, matched by overlap. Invalid date fields
  fail that field's filter; Any matches any valid selected field. Unbounded or
  other interval spellings are not guessed. Ambiguous expressions return C4's
  typed clarification; callers provide omitted-year coverage as in Task 5.
- Each result is a per-note committed read, not a cross-note snapshot or a
  guarantee the candidate index is complete. No result claims fully current.
- B1 NoteMetadata lacks repository_id/commit, so provenance explicitly reports
  None. C6 gate reference state is also not persisted: citations report Unlinked
  for zero relations, otherwise NotRevalidated. No fabricated Pending/Resolved
  state or new storage/schema machinery is introduced. Persisting that state
  belongs to the later provenance/state work; Task 7 must retain these limits.
- No writes, PiiStore plaintext reads, journal access, disk-backed index or
  generative model were added. SEC-001/002/003 remain deferred. MVP v6, approved
  plan, storage/domain/policy/vault-security and Cargo.lock remain unchanged.

## Continuity and next checkpoint

n2io remains open overall. The core SearchResponse confidentiality obligation
from Task 4 is now covered and recorded with evidence; embedding, summary,
attachment and cache surfaces remain deferred. Future API/transport wrappers
must preserve these guarantees. Next is Task 7 (api search/retrieve wrappers),
then Tasks 8-9; no merge, tag or push without explicit authorization.
