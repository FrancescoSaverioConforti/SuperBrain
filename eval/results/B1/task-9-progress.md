# B1 Task 9 - progress checkpoint, not completion evidence

**Update 2026-11-01:** the user authorized the Important coordination fix.
It is implemented and independently GO-reviewed; see coordination-fix.md for
TDD,12 deterministic tests and complete validation. Historical measurements
below are PRE-FIX. Benchmark processes are no longer active; N=100,000 remains
unmeasured (incomplete setup only). Task9/B1 are not declared complete.

2026-10-30. Last committed task: ff6ddf1 (Task 8), working changes below remain
uncommitted. Canonical spec and approved plan unchanged. No merge/tag/push.

## Authorized benchmark deviation

User explicitly authorized 100 queries at EACH N=100/1,000/10,000/100,000,
40 exact + 40 BM25 + 20 filtered. Full original N-dependent protocol has NOT
been executed and must not be claimed complete. Measurements below are actual
observations, never extrapolated. Early pilot results are kept separately.

Current measured completed profiled runs:

| N | queries | rebuild seconds | populated RSS bytes | RSS delta bytes | P50 ms | P95 ms |
| --- | --- | --- | --- | --- | --- | --- |
| 100 | 100 | 0.090635 | 17969152 | 4317184 | 14.516600 | 50.022300 |
| 1,000 | 100 | 0.519898 | 56901632 | 42242048 | 171.275200 | 664.092000 |

The earlier 1,000-note pilot used 250 queries and yielded P95 724.709700ms.
It is distinct from the final authorized 100-query measurement above.

10,000 notes: setup/rebuild measured (17.041s, populated RSS405876736 bytes),
but 100-query run is still in progress, so no final P50/P95 reported yet.
This run began before profiling support; its raw CSV has no phase timings.
100,000 notes: population still in progress, no measured rebuild/memory/latency
result yet. No missing value is replaced with an estimate.

Diagnostic timings (actual sum across the 100 profiled queries at N=1,000):

| phase | total ms | share of measured wall time |
| --- | --- | --- |
| candidate generation | 103.8384 | 0.470% |
| authoritative Vault reads | 19890.3349 | 90.020% |
| revalidation | 1831.3513 | 8.288% |
| response construction/ranking | 213.5857 | 0.967% |

Profiling uses the same search path with opt-in aggregate Instant timings; no
query text, content or ids retained in the profile. Revalidation includes parsing,
Document/query/filters and insertion into the fresh score corpus. Response
construction includes excerpts/provenance plus fresh scoring/sorting/envelope.
Unaccounted wall time is loop/setup/drop/measurement overhead. Runs overlap
mutation/build/setup work and are not isolated performance benchmarks; disclose
this limitation. No architectural optimization or disk index was introduced.

Runner: crates/index/examples/b1_scalability.rs, release build, deterministic
seed 0xB120261030, varying-length synthetic notes, actual frequency-weighted body
vocabulary, saved query sets/raw latency CSVs. Population uses only sanctioned
propose_and_commit into a non-git synthetic vault: durable JournalCommitted
content is returned as GitPending honestly. Setup is excluded from measured
rebuild/search timings; every retrieval still uses authoritative vault reads.
Memory uses Windows PROCESS_MEMORY_COUNTERS; baseline deltas are approximate
process-wide index attribution, not allocator-perfect isolated allocations.
CPU i9-13900H,20 logical CPUs; RAM33999495168 bytes. No source-data secrets used.

## Mutation/testing progress

Initial index+api mutation pass:209 tested,108 caught,53 unviable,48 missed,
zero timeouts. New tests close numeric BM25 formula/length normalization,
canonical metadata, all relation variants, temporal interval boundaries,
three-character trigram boundary and sanitized nonempty error diagnostics.
The separate ||->&& empty-corpus guard and >->>= positive-score filter survivors
are equivalent for the current scoring loop; full final triage still pending.

An audit regression independently proved a pre-existing state bug: indexing a
malformed/poisoned newer update after Current retained Current. Test RED then
GREEN after marking Stale at start; previous postings are retained. This fix
preserves existing architecture and was personally rechecked by the reviewer.

Final mutation run218 is still active; no final counts claimed here.
Current tests passed: index66 unit+6 confidentiality+28 search; api11 tests;
workspace Clippy clean after profiling. The reviewer personally reran
Task4 6/6, Task6 28/28, failed-update regression1/1, C4 25/25 and workspace Clippy.
Final full workspace validation remains pending, including known pre-B1
restore/deletion scheduling flakes documented in Task7.

## IMPORTANT: final independent review requires architectural ruling

Verdict CHANGES REQUIRED. Two real concurrent harness executions reproduced:

    committed_version=2 old_hits=1 new_hits=0 state=Current
    new_term_search=NotPresent

Interleaving: rebuild reads v1, writer commits/indexes v2, rebuild inserts its
old v1 snapshot and marks Current. indexer_impl.rs index_document uses separate
postings/state locks without a version check; mark_current ignores its version
argument; rebuild discards VaultRecord.version. Revalidation prevents false
returned content but cannot restore a v2 term missing from the candidate set.
The missing recall persists until another successful index update/rebuild.

Reproduction preserved in rebuild-race-repro.rs (copied unchanged from the fresh
reviewer's standalone harness); it uses real propose_and_commit/Vault/RealIndexer
and a large synthetic v1 source to widen the race, no source modifications.
The reviewer's original harness manifest is retained at
C:/Users/fsave/AppData/Local/Temp/superbrain-review-race-b71caebc1adf4f2facb99da21bf93ab7/Cargo.toml.

No coordination/versioning fix applied. Task9Step4 explicitly requires STOP and
report before resolving a new Critical/Important architectural decision. Proposed
ruling for the user: preserve concurrent rebuild by adding monotonic version
checks and coordinated postings/state updates shared by rebuild and live indexing,
without callbacks into Vault, new persistence, or MVP edits. Do not silently
switch to a quiescent-only rebuild (contradicts the approved concurrent contract).

This checkpoint is not final evidence.md and does not declare B1 complete.

## Active processes to resume

- mutation-final: exec session95582; output under
  .superpowers/sdd/2026-10-30-b1-deterministic-search/mutation-final/mutants.out.
- N10,000 benchmark: exec session73561;100-query limit already supplied;
  raw data under .superpowers/sdd/2026-10-30-b1-deterministic-search/perf-final.
- N100,000 profiled benchmark: exec session19915, setup ongoing; output under
  .superpowers/sdd/2026-10-30-b1-deterministic-search/perf-profiled.
- N100/N1,000 profiled runs: exec session35499; both measurement CSVs complete.

Already-authorized measurements/mutations may continue in the background. Do not
interpret an unfinished process as completion; inspect exit status and final
artifacts on resume. Architectural changes and final B1 closure await user ruling.
