# B1 — deterministic retrieval evidence

**Verdict at this checkpoint: NO-GO. Task 9 remains open.** Final independent
whole-branch review confirmed a new Important Sensitive-metadata issue requiring
a user decision (B1-SEC-001 below). The approved plan requires stopping rather
than fixing a new architectural finding autonomously. N=100,000 is still running
unchanged; its complete numeric review is pending. This is not a merge
authorization or a claim that the original benchmark protocol was executed.

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

All task reviews were independent of implementation. The fresh whole-branch
review found the rebuild/live race, reproduced it through public APIs and stopped
for the user's architectural ruling. After explicit authorization, the fix went
through deterministic TDD and independent re-review. Final closing review returns
NO-GO on the new B1-SEC-001 below; earlier GO on the concurrency fix is not a
whole-B1 closure. The fourth-scale numeric audit is still pending.

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
exclusions; B1-SEC-001 leaves allowed date metadata unresolved. Embedding,
summary, attachment and cache also remain open for future phases.
SEC-001/002/003 remain deferred as documented. `docs/Spec_mvp.md` is unchanged.

## Verification of the final runtime core

At runtime commit `d798c74`:

| Check | Actual outcome |
| --- | --- |
| `cargo test --workspace` | 510 passed, 0 failed, 0 ignored, default parallel run |
| Deterministic coordination tests | 12 passed; first eight reproduced old failures RED before fix |
| `cargo clippy --workspace --locked -- -D warnings` | clean |
| `cargo deny check --hide-inclusion-graph` | advisories/bans/licenses/sources OK; existing duplicate/unused-allowance warnings |
| Evaluation runner tests | 5 passed |
| Full post-fix index+api mutation pass | 237: 177 caught, 58 unviable, 2 equivalent, 0 timeouts |
| Targeted coordinator mutation pass | 35: 24 caught, 11 unviable, 0 missed/timeouts |
| Additional ReadPolicy mutation pass | 2 caught of 2 |

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
them. The final default-parallel 510-test run passed. Later changes affect only
benchmark harness, tooling and evidence, not the verified runtime core.

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

| Notes | Actual queries | Rebuild s | Populated RSS bytes | RSS delta bytes | P50 ms | P95 ms |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 100 | 0.029705 | 13328384 | 4370432 | 6.563300 | 24.685700 |
| 1000 | 100 | 0.360931 | 52875264 | 43458560 | 77.206200 | 287.053300 |
| 10000 | 100 | 17.270152 | 412983296 | 396247040 | 4285.351800 | 16355.083600 |
| 100000 | 17/100, partial snapshot | 1454.158 (rounded log) | 3727187968 (log) | unavailable | not measured completely | not measured completely |

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
Final fourth-scale review remains pending. The 2026-11-01 15:22 Europe/Rome
snapshot has rows 0–16 only: 8 exact / 6 BM25 / 3 filtered, not the authorized
completed 40/40/20 sample. Populated lifetime peak in the actual rebuild log is
3729625088 bytes. The longest completed query in that snapshot took
2061965.7653 ms and performed 100000 authoritative reads. These observations
are actual but partial; there is no completed-sample percentile or RSS baseline/
delta for N=100,000 yet. The native worker/supervisor remain active; the benchmark
has neither failed nor been interrupted at this checkpoint. See the retained
`latencies-100000.partial.csv`, `rebuild-100000.partial.log` and
[measurement scope](measurements/README.md). Live files may subsequently advance;
the partial snapshot is not substituted for the completed reducer input.

Independent partial-snapshot audit: PASS for row indices/composition, finite
non-overlapping phase times and count consistency, without asserting completion
of the authorized sample. Snapshot SHA256:
`922dccdc357b89d75b88b7159ea736d4226571d426223346345203dd750acfd8`.

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
an authorization to weaken gates or persist the index. N=100,000 findings must
be added only after actual completion/failure/interruption data are collected.

The separate [optimization proposal](../../../docs/superpowers/plans/2026-11-01-b1-search-performance-proposal.md)
describes attribution, conservative version-safe pruning, exact ranking proof
obligations and optional storage-owned batching. It requires separate approval
and implements nothing. No arbitrary top-k candidate cap, stale-metadata filter,
gate cache, persistent index or automatic architectural optimization is applied.

## Blocking finding B1-SEC-001 — free text in Sensitive date metadata

**Important, open; decision required.** The approved plan's allowlist is
implemented as written, but its safety premise is false for the current schema.
Plan lines 24–25 claim the ten fields cannot carry free text, unlike `source`.
`domain::parse_frontmatter` (`crates/domain/src/note.rs`, lines 123–125) only
requires Strings for `created`, `updated` and `event_period`; it does not enforce
date syntax. `Document::structured_fields_as_text` indexes these values verbatim,
and Sensitive search uses that text in its excerpt. Arbitrary identifying text
can therefore be discoverable under these ostensibly constrained fields.

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
To rerun, create an external temporary Cargo binary with this file as main.rs
and path dependencies on this checkout's domain, index, storage, policy and
vault-security crates; no benchmark vault access is needed.

The schema permissiveness predates B1; indexing it exposes the unsound allowlist
premise. The existing twelve-field exclusion tests did not exercise this input
class. Resolving it requires an explicit decision about validating/handling
legacy date values or restricting/sanitizing the Sensitive searchable surface.
Project/identifier text assumptions should be assessed in that decision rather
than automatically claimed safe. No broader schema change, migration, index fix,
specification amendment or acceptance of this residual risk is inferred here.

## Closing review and verdict

**NO-GO for B1 closure at this checkpoint.** The final independent whole-branch
review found B1-SEC-001 and the approved plan's stop rule applies before an
autonomous architectural fix. The previously authorized concurrency fix remains
approved; no new concurrency finding was reported. B1-PERF-001 remains open and
has a separate unimplemented optimization proposal.

Task 9 is not complete. The N=100,000 process continues under the existing
supervisor, unchanged; full-sample measurement and final numeric audit remain
pending. This verdict does not assert the benchmark failed or authorize stopping
it. Only actual completed/partial observations are recorded. No merge, tag or
push has been performed, and the canonical specification is unchanged.
