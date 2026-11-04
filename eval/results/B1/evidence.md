# B1 — deterministic retrieval evidence

**GO — B1 is formally closed at HEAD `3ca2203`.** The user resolved the
Sensitive temporal-metadata decision (B1-SEC-001, fixed at `3c6b1bb`); the
separate P0 DeletionLog fail-closed increment then landed on this branch
(`e92dc78`/`3ca2203`) without regressing B1 (fixture-only diff to
`index`/`api`); and a final independent whole-branch review at HEAD `3ca2203`
returned GO with no residual Critical/Important finding. N=100,000 was
intentionally interrupted after 19 complete queries under the user's
cost-assessment authorization and stays recorded as partial, not complete.
B1-PERF-001 (performance) and the listed security residuals remain explicitly
OPEN/deferred — see "B1 verdict" below. No merge, tag, or push authorized.

## Scope and implementation

B1 adds in-memory trigram exact-substring and BM25 ranked retrieval, C4 calendar
time resolution, authoritative result revalidation/citations, project/type/
status/trust/time filters, and policy-checked read-only `api::search`/`retrieve`.
The evaluation split and runner isolation are scaffolding for B3, not a completed
held-out quality benchmark. Embeddings, code graph, transport and chunk retrieval
remain outside B1; chunks return a typed unsupported-target error.

The branch starts at merge-base `55d1721`. Handoff checkpoint `f1c2abf` included
Tasks 0–4. The implementation/review record is:

| Work | Commit(s) | Evidence |
| --- | --- | --- |
| Index/Vault seam, Document, exact/BM25, confidentiality | edaf0fe through 94c59b4 | handoff; local Task 0–4 reports; confidentiality tests |
| C4 Europe/Rome calendar expressions | 80cab44 | [Task 5](task-5.md) |
| Revalidated search, filters, citations, typed errors | 0ce5543 | [Task 6](task-6.md) |
| Default-deny read-only API | 8ca272b | [Task 7](task-7.md) |
| Evaluation isolation scaffold | ff6ddf1 | [Task 8](task-8.md) |
| Mutation hardening and opt-in four-phase diagnostics | c8a1974 | [Mutation triage](mutation-triage.md) |
| Authorized rebuild/live ordering fix | d798c74 | [Coordination fix](coordination-fix.md) |
| Resumable measurement tooling and first-three-scale audit | 75abcb4, 5080a41 | [Checkpoint](task-9-progress.md), [raw measurements](measurements/README.md) |
| Sensitive temporal text exclusion and validated filter metadata | 3c6b1bb | [Temporal fix](temporal-metadata-fix.md) |

All task reviews were independent of implementation. The fresh whole-branch
review found the rebuild/live race, reproduced it through public APIs and stopped
for the user's architectural ruling. After explicit authorization, the fix went
through deterministic TDD and independent re-review. Closing review then found
B1-SEC-001 and returned NO-GO. After the user's explicit temporal-surface ruling,
the new fix also passed TDD and independent re-review: GO, no new Critical/Important.
The interrupted fourth-scale numeric evidence is verified as partial only.

## Architecture, safety and result contracts

- `storage` does not depend on `index`/`api`. Its public `Indexer` seam passes
  already committed record bytes/version while holding the existing per-note
  lock. The indexer never calls back into Vault. `propose_and_commit` remains
  the single public storage write entry point; cold rebuild uses public listing
  and `Vault::read_note`, without direct journal access from index/api.
- One publication mutex coordinates both postings, allowlisted Document metadata,
  expected-version high-water marks and published versions. Observation precedes
  parsing; publication rechecks under the same mutex. A late rebuild v1 cannot
  overwrite an observed live v2 or restore `Current`. Faults leave the note
  `Stale`; rollback/poison recovery remain coherent and indexing unwinds do not
  fail the already committed write. Abort-on-OOM is not claimed recoverable.
- Rebuild/read and candidate generation release their respective locks before
  entering the other subsystem. No index-to-Vault callback or inverse lock order
  was introduced. Rebuild remains concurrent and the index stays in memory.
- Search reads every candidate through `Vault::read_note`, reconstructs current
  allowlisted content, re-matches and filters it, then rescores/ranks the current
  eligible corpus before applying k. Citations carry the version actually read.
  No global cross-note snapshot or complete/current candidate-set claim is made.
- Sensitive body, source and relation targets are excluded from postings/search
  responses and diagnostic formatting; current reclassification is reapplied.
  Created/updated/event-period fields are also excluded from Sensitive text,
  including valid values. Document stores validated optional dates/instants and
  point/nonempty intervals for C4; invalid values become None without text fallback.
  Deleted/missing/inconsistent candidates are excluded. Operational/gate failures
  return sanitized `ReadUnavailable`, rather than successful `NotPresent`.
- Active-project scoping is default; explicit other-project requests are visible
  in the response. API permissions are checked on every operation, default deny.
  Retrieve returns the authoritative pointer record; its legitimate full
  frontmatter is not the restricted search excerpt surface.
- C4 reference dates are explicit. Calendar ranges use Europe/Rome and `[start,
  end)` boundaries, including the real 2026 DST transition. Ambiguous expressions
  return typed ambiguity; no implicit current clock or fabricated dates.
- Repository/commit provenance unavailable in note records remains `None`.
  Reference state is honestly `Unlinked`/`NotRevalidated`, not invented C6 gate
  resolution. There is no transport, network identity, deadline scheduler or
  cross-query authorization cache in this callable core.

The exact partial `[n2io]` text was checked against handoff section 8 before being
recorded. Postings/log and core SearchResponse surfaces are audited; the overall
claim remains open. Those audits cover the tested body/source/relations
exclusions and the new temporal-field regressions; B1-SEC-001 is closed within
its authorized search-surface scope. Embedding,
summary, attachment and cache also remain open for future phases.
SEC-001/002/003 remain deferred as documented. `docs/Spec_mvp.md` is unchanged.

## Verification of the final runtime core

At patched runtime commit `3c6b1bb` (historical full mutation results separately labelled):

| Check | Actual outcome |
| --- | --- |
| `cargo test --workspace --locked --quiet` | 518 passed, 0 failed, 0 ignored, default parallel run |
| Deterministic coordination tests | 13 passed, including temporal-marker race; first eight originally RED before coordination fix |
| `cargo clippy --workspace --locked -- -D warnings` | clean |
| `cargo deny check --hide-inclusion-graph` | advisories/bans/licenses/sources OK; existing duplicate/unused-allowance warnings |
| Evaluation runner tests | 5 passed |
| Full index+api mutation pass at earlier d798c74 | 237: 177 caught, 58 unviable, 2 equivalent, 0 timeouts |
| Targeted coordinator mutation pass at earlier d798c74 | 35: 24 caught, 11 unviable, 0 missed/timeouts |
| Additional ReadPolicy mutation pass | 2 caught of 2 |
| Targeted temporal-fix mutation pass at 3c6b1bb | 77: 65 caught, 12 unviable, 0 missed/timeouts, exit0 |

The two full-pass survivors are explicitly justified equivalent BM25 mutations,
not hidden failures: the empty query/corpus shortcut and accepting zero scores
where the score map contains only strictly positive contributions. Commands,
initial 48-survivor disposition and exact lists are in
[mutation-triage.md](mutation-triage.md). cargo-mutants' exit 1 for survivors is
recorded, not reported as a zero-survivor exit. Initial hook-removing mutants
exposed unbounded test waits; the corrected arrival watchdog keeps barrier-driven
interleavings and the final runs have no timeouts.

Earlier default-parallel workspace runs hit pre-existing restore/deletion
`ReferenceCorrupt` scheduling flakes. The named failures and serial successful
rerun are retained in Task 7; storage recovery/restore were not changed to hide
them. Both the previous 510-test run and patched default-parallel 518-test run
passed. The temporal fix adds eight tests, preserves the storage gates and
coordinator protocol, and is not a performance optimization.

## Authorized scalability measurements

**Explicit protocol deviation approved by the user:** 100 queries at each of
100 / 1,000 / 10,000 / 100,000 notes, exactly 40 exact / 40 BM25 / 20 filtered,
k=10. The original N-dependent full protocol was not executed. Missing values
are not estimates. There is one run per scale and no confidence interval or
invented pass/fail threshold.

Fixed seed `0xB120261030`, varied synthetic bug/decision/pattern/research notes,
frequency-weighted two-to-four-term queries, filtered Bug/date cases and exact
ids are generated by `b1_scalability.rs`. Setup uses public writes and
authoritative existing-record checks in an isolated non-git synthetic vault;
`JournalCommitted`/explicit `GitPending` is intentional. Setup/recovery time is
outside rebuild/search measurements. Original historical pilots remain separate
and are not a controlled A/B performance comparison.

All measurement rows below belong to core `d798c74`, before the temporal fix.
No scalability measurement of final core `3c6b1bb` is claimed. Moving date parsing
into Document is not assumed performance-neutral or credited with an improvement.

| Notes | Actual queries | Rebuild s | Populated RSS bytes | RSS delta bytes | P50 ms | P95 ms |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 100 | 0.029705 | 13328384 | 4370432 | 6.563300 | 24.685700 |
| 1000 | 100 | 0.360931 | 52875264 | 43458560 | 77.206200 | 287.053300 |
| 10000 | 100 | 17.270152 | 412983296 | 396247040 | 4285.351800 | 16355.083600 |
| 100000 | 19/100, interrupted | 1454.158 (rounded log) | 3727187968 (log) | unavailable | not measured completely | not measured completely |

RSS/peak are process working set/lifetime high-water marks; delta is approximate
attribution, not isolated index allocator usage. Raw data retains baselines/peaks.
The Windows host has i9-13900H, 20 logical processors and 33999495168 bytes RAM;
rustc/cargo 1.99.1, cargo-mutants 27.1.0, Python 3.13.15. The benchmark release
binary/source/core/Cargo.lock hashes are pinned in the build manifest. Live
status is local `perf-postfix/job-status.json`; stage timestamps are not liveness
or validation claims. Only completed CSVs enter the all-scale reducer, which
checks query composition, finite/non-overlapping times, counters, nearest-rank
percentiles and memory consistency. The original protocol flag stays false.

Independent numerical review of the first three completed samples: PASS.
The interrupted fourth-scale evidence also passed independent numeric audit,
as partial data only: 19 rows 0–18, mix 8 exact / 8 BM25 / 3 filtered, finite times,
phase budgets and counters coherent. At 2026-11-01T14:52:54Z the sole worker was
stopped after verified completed setup/rebuild, in its read-only query phase,
under the user's cost-assessment authorization. The supervisor exited through
native nonzero-exit handling. There was no second supervisor, restart, fixture
deletion/repair or storage mutation; the vault is retained. The unfinished query
19 has no recorded time and is excluded. No process remains active.

Actual populated lifetime peak is 3729625088 bytes (log). The longest complete
query, index 17, took 2416827.1975 ms with 100000 reads. RSS baseline/delta and the
100-query P50/P95 are unavailable, not estimated. See
`latencies-100000.interrupted.csv`, `benchmark-outcome.json` and
[measurement scope](measurements/README.md). Final interrupted CSV SHA256:
`0329b1d577c9488d75983d346b914ad07c710dadd57296dd16c7d69f41191f51`.
The previous 17-row snapshot remains historical evidence, not reducer input.
Neither interrupted file substitutes for the missing full-sample summary or
creates an all-scale completion manifest.

## Open finding B1-PERF-001 — authoritative-read fan-out

At N=10,000, BM25 and filtered queries read 2,500–10,000 candidate notes despite
k=10. Exact P95 is 15.0333 ms (one read); BM25 P95 is 16662.6147 ms and filtered
P95 is 16155.7740 ms. Aggregate measured phase totals across 100 queries:

| Phase | ms | Share of total wall time |
| --- | ---: | ---: |
| Candidate generation | 632.9751 | 0.100% |
| `Vault::read_note` | 614559.0314 | 97.540% |
| Revalidation | 12868.6833 | 2.042% |
| Response construction/ranking | 1510.1431 | 0.240% |
| Residual overhead | 488.1070 | 0.077% |

Total wall time is 630058.9399 ms. These are aggregate phase shares, not a
decomposition of the P95 query. The timer includes journal, locks, integrity,
classification/deletion gates and I/O; a journal-scan hypothesis is not a measured
causal attribution. This is a significant observed sustainability limit, not
an authorization to weaken gates or persist the index. At N=100,000 the 19
completed rows include broad queries with 100000 authoritative reads; one took
2416827.1975 ms (about 40 minutes). This is actual interrupted-run evidence,
not a complete-sample percentile or a prediction of unmeasured queries.

The separate [optimization proposal](../../../docs/superpowers/plans/2026-11-01-b1-search-performance-proposal.md)
describes attribution, conservative version-safe pruning, exact ranking proof
obligations and optional storage-owned batching. It requires separate approval
and implements nothing. No arbitrary top-k candidate cap, stale-metadata filter,
gate cache, persistent index or automatic architectural optimization is applied.

## Closed finding B1-SEC-001 — free text in Sensitive date metadata

**Important, fixed at `3c6b1bb` after explicit user ruling; independent GO.**
The original approved allowlist was implemented as written, but its safety
premise was false for the current schema.
Plan lines 24–25 claim the ten fields cannot carry free text, unlike `source`.
`domain::parse_frontmatter` (`crates/domain/src/note.rs`, lines 123–125) only
requires Strings for `created`, `updated` and `event_period`; it does not enforce
date syntax. The old Document indexed these values verbatim, and Sensitive
search used that text in its excerpt. Arbitrary identifying text was therefore
discoverable under these ostensibly constrained fields.

The independent reviewer ran a minimal reproduction for each of the three
fields using actual domain parsing, `Gate::evaluate`, a Sensitive pointer record,
`Document::from_record_bytes`, TrigramIndex and Bm25Index. Each produced:

```text
schema=accepted gate=Pass class=Sensitive canary_searchable=true trigram_hits=1 bm25_hits=1
```

The synthetic source marker and `pii://` body pointer stayed excluded. This is
not a regression of those existing exclusions, nor a claim of an end-to-end
encrypted-store commit reproduction: the minimal harness describes store
availability to the real gate and constructs the pointer record directly.
Source is retained in [sensitive-date-metadata-repro.rs](sensitive-date-metadata-repro.rs).
It is a standalone diagnostic, not a newly installed workspace test or fix.
To reproduce the old finding, create an external temporary Cargo binary with
this file as main.rs and path dependencies on domain/index/storage/policy/
vault-security from pre-fix `338fbf1`. Its canary-presence assertion intentionally
fails against the patched implementation. No benchmark vault access is needed.

The schema permissiveness predates B1; indexing exposed the unsound allowlist
premise. The original twelve-field tests did not exercise this input class.
The user chose filter-only structured temporal values for Sensitive rather than
a domain-schema migration. Eight added tests cover both postings, valid C4,
invalid values, reclassification, live updates and a barrier-controlled stale
rebuild. 518 workspace tests and 77 targeted mutants passed as documented above.
Independent re-review: GO, no new Critical/Important. See
[temporal-metadata-fix.md](temporal-metadata-fix.md). Other allowlisted fields
retain approved behavior; this narrow fix does not assert that arbitrary
project/identifier strings cannot contain identifying text. No broader schema
change, migration or canonical-spec amendment was introduced.

## Prior closing review (at `3c6b1bb`, before the P0 DeletionLog landing)

**Independent technical review: GO on the authorized fix and branch re-review,
no residual Critical/Important within scope.** The previous security NO-GO is
resolved by the explicit user decision and `3c6b1bb`. The reviewer personally
passed 84 index unit + 6 confidentiality + 31 search + 11 API and Clippy, and
audited all 19 interrupted rows. Targeted mutations: 77 tested, 65 caught,
12 unviable, 0 missed/timeouts. The full workspace passed 518/0/0.
No new concurrency finding was reported.

## P0 DeletionLog landing on this branch (`e92dc78`/`3ca2203`)

The separate P0 DeletionLog fail-closed increment (DL-02/DL-03) landed on this
same branch after the above review, closed with its own independent GO — see
`docs/handoffs/P0-deletionlog-handoff.md`. P0 is a `storage`/`vault-security`
security increment, not a B1 task; it is recorded here only to confirm it did
not regress B1's search/retrieve behavior.

`git diff 3c6b1bb..3ca2203 -- crates/index crates/api` contains exactly seven
added `pii.bootstrap_deletion_log().unwrap()` fixture lines (adapting test
fixtures to the new fail-closed DeletionLog lifecycle) and one doc-comment
rewording in `document.rs`. No search/trigram/BM25/citation logic changed.
Workspace tests at HEAD `3ca2203`: **556 passed, 0 failed, 0 ignored** (up from
518 at `3c6b1bb`, entirely accounted for by P0's own `vault-security`/`storage`
tests). B1's prior mutation evidence (77 targeted mutants at `3c6b1bb`) remains
applicable: P0 introduced no logic change to the `index`/`api` crates.

## Final B1 closure review (at HEAD `3ca2203`)

**Independent whole-branch review, dispatched fresh and uninvolved in any
task's implementation: GO.** The reviewer re-verified, rather than trusted,
all of the following against the actual code at HEAD `3ca2203`:

- Task 1's Indexer seam: `RealIndexer::mark_current` works only on the bytes
  the write path already holds; it never calls back into `Vault::read_note`.
  Only `rebuild_index` reads the vault directly, always outside the index
  lock.
- Task 6's mandatory five-point re-validation discipline (`crates/index/src/search.rs`)
  is structural, not a bypassable convention: every candidate passes through
  a fresh `read_note`, a fresh `Document` re-derivation under current
  classification, a fresh query-match re-check, every filter re-applied, and a
  score recomputed from a per-request BM25 index built only from the eligible
  fresh documents. There is no code path that reaches a `SearchResult` without
  all five steps.
- The P0 landing's diff to `crates/index`/`crates/api` is fixture-only, as
  stated above; confirmed independently by the reviewer's own diff.
- Sensitive-classification confidentiality (body/source/relations excluded
  from both postings; B1-SEC-001's temporal-field exclusion) is in the code,
  not just documented, with reclassification and live-update cases covered.
- C4 date resolution uses real `chrono_tz::Europe::Rome` arithmetic, verified
  against the actual 2026-10-25 and 2026-03-29 DST transitions, not naive UTC
  math.
- The rebuild/live coordination fix matches `coordination-fix.md`: one mutex
  covers both postings plus the expected-version high-water mark; version
  checks prevent a stale rebuild snapshot from overwriting a newer live write;
  a poisoned lock resets affected notes to `Stale` rather than serving wrong
  content.
- `eval/results/B1/evidence.md` (this file) does not overstate the N=100,000
  benchmark as complete and does not claim B1-PERF-001 resolved.
- No undeclared `TODO`/`FIXME`/`XXX`/`unimplemented!`/`todo!()` in
  `crates/index`/`crates/api`.
- Independent re-run at HEAD `3ca2203`: `cargo test --workspace --locked`
  556/0/0; `cargo clippy --workspace --locked -- -D warnings` clean;
  `cargo deny check` advisories/bans/licenses/sources ok.

No Critical or Important findings. Four non-blocking suggestions were noted:
this evidence file's verification table predates the P0 landing's test-count
change (518 vs. 556 at HEAD, fully explained by P0's own tests, now corrected
above); `rebuild_index` skips per-note `read_note` failures during rebuild
(already documented as `Completeness::IndexedCandidatesMayBeIncomplete`, not a
correctness or confidentiality issue); P0 changed one internal error-kind
mapping in `list_note_ids` (`Io` to `DeletionLog`) with no effect on B1's own
error handling; and `id`/`project` remain text-searchable for Sensitive notes
under the approved allowlist, which this file already states does not assert
identifier/project strings are free of identifying text.

## Task 9 status

**Complete, on the evidence above.** All of Task 9's required checks ran
clean at HEAD `3ca2203`: full workspace tests (556/0/0), Clippy, cargo-deny,
the scalability benchmark protocol (executed at three full scales plus a
documented partial fourth scale), and a final whole-branch independent review
returning GO with no new Critical/Important finding. Mutation evidence from
`3c6b1bb` carries forward unchanged, since P0's touch of `index`/`api` was
fixture-only.

## B1 verdict: GO, with two findings left intentionally open

**GO to close B1 as implemented**, on the following explicit terms:

- **B1-PERF-001 (performance) stays OPEN/deferred.** The N=100,000 scalability
  sample is incomplete (19/100 queries, intentionally and authorizedly
  interrupted); the N=10,000 sample shows authoritative-read fan-out
  dominating BM25/filtered query latency. Correctness is not affected. An
  unimplemented optimization proposal exists
  (`docs/superpowers/plans/2026-11-01-b1-search-performance-proposal.md`) and
  requires its own separate approval before any implementation work begins.
- **The listed security residuals stay OPEN/deferred**, exactly as enumerated
  in this handoff's instructions (temporal/schema validation debt beyond the
  narrow Sensitive-search fix, PiiStore raw API confinement, AEAD/AAD binding,
  backup provenance, SEC-001/002/003 filesystem hardening, DoS/resource
  budgeting, future MCP/agent surfaces). B1 does not claim any of these
  resolved. P0 DeletionLog (DL-02/DL-03) is the one exception: closed, GO,
  confirmed on this branch's history.

No runtime optimization, disk-backed index, schema migration, or canonical
specification change was introduced to reach this verdict. No merge, tag, or
push performed or authorized by this closure.
