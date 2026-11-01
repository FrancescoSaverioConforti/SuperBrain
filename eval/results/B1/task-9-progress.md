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

## Current continuation: post-fix measurements, 2026-11-01

The authorized Important fix is committed as d798c74; related prior hardening and
diagnostics are separated in c8a1974. Workspace510 Rust tests,5 Python runner
tests, Clippy and cargo-deny passed. Full post-fix mutation237:177 caught,
58 unviable,2 independently justified equivalents,0 timeouts; extra read policy2/2.

A reviewed hidden supervisor is now running the100-query sample at every scale,
serially, without further approvals or Git operations. Current local status:
.superpowers/sdd/2026-10-30-b1-deterministic-search/perf-postfix/job-status.json.
The supervisor PID is recorded alongside that file in driver.pid; do not launch a
second supervisor while it holds coordinator.lock. Before restarting after an
external kill, inspect native workers to avoid concurrent recovery of one vault.

Two operational harness issues were found and closed before unattended execution:
PowerShell native stderr under Stop is now handled through Start-Process with
separate files and explicit ExitCode; and absent synthetic notes are created only
on typed NotFound, while existing notes require a full authoritative read with
exact version/content. Other errors fail closed. Native stderr/exit-code smoke
checks, spaced paths, reducer percentile anchors, missing-data rejection and
Python-O rejection passed; independent reviewer confirmed GO on these changes.

Post-fix complete actual samples already observed at this checkpoint:

| N | measured queries | rebuild s | populated RSS bytes | P50 ms | P95 ms |
| --- | --- | --- | --- | --- | --- |
|100|100|0.029705|13328384|6.563300|24.685700|
|1000|100|0.360931|52875264|77.206200|287.053300|

The larger scales are still running/queued. No estimate replaces an unmeasured
value. Differences from historical pilots are not a controlled causal estimate
of optimization: workload count, cache and competing activity differ. The prior
Important scalability observations remain recorded; no runtime performance
optimization or disk-backed index was introduced. B1 remains in progress until
all four samples and the final numerical review are complete.

### Completed 10,000-note post-fix measurement (2026-11-01)

The third scale has now completed all 100 measurements (40 exact / 40 BM25 /
20 filtered). Actual rebuild: 17.270152 s; populated process RSS: 412983296
bytes; RSS baseline: 16736256 bytes; delta: 396247040 bytes; lifetime peak:
412983296 bytes. Nearest-rank P50: 4285.351800 ms; P95: 16355.083600 ms.
Completed raw CSV/TSV files for the first three scales are retained under
measurements/; this remains a partial checkpoint, without the all-scale manifest.

Across the 100 queries, measured wall time totals 630058.9399 ms. Non-overlapping
phase totals are candidate generation 632.9751 ms (0.100%), authoritative
Vault::read_note 614559.0314 ms (97.540%), revalidation 12868.6833 ms (2.042%),
and response construction 1510.1431 ms (0.240%). Remaining timer/other overhead
is approximately 488.1070 ms (0.077%). Percentages use aggregate wall time,
not a decomposition of the P95 query.

This is a significant observed scalability limitation: retrieval at this scale
already takes seconds, and authoritative reads dominate measured query time.
The phase timer includes the entire authoritative read path; it does not isolate
SQL, filesystem, integrity checks or locking. A source inspection hypothesis
about repeated journal lookups is not a measured causal attribution. No storage
optimization, architectural redesign or persistent index has been introduced.
N=100,000 is running; its rebuild/RSS/percentiles remain unmeasured until actual
artifacts complete. No extrapolation is reported as measurement.

Independent numerical review: PASS for all three completed post-fix samples.
The reviewer independently checked query mix/index/category alignment, phase
budgets, candidate/read/eligible/result counts, nearest-rank percentiles and
RSS/peak/delta consistency. At N=10,000 the exact subset P95 is 15.0333 ms
(one authoritative read per query), BM25 P95 is 16662.6147 ms and filtered P95
is 16155.7740 ms. Ranked and filtered queries fan out to 2,500-10,000 reads.
These measured differences explain why the mixed P95 is poor while exact lookup
remains much faster. They do not bypass or justify weakening authoritative gates.
The fourth scale and final whole-B1 review remain pending.

## Independent whole-branch review: NO-GO (2026-11-01)

The reviewer confirmed a new Important, B1-SEC-001: `created`, `updated` and
`event_period` only require String values in the domain schema, while the plan
assumes date-shaped values cannot carry identifying free text. A minimal
reproduction for each field passes schema and the real gate and makes a canary
searchable in both indices for a Sensitive pointer record. The implemented field
list matches the approved plan; the faulty security premise requires an explicit
decision about validation/legacy values or restricting the search surface.
No fix or spec change was applied. See evidence.md and the retained standalone
sensitive-date-metadata-repro.rs. The concurrency fix remains approved; the
reviewer found no additional coordinator/lock/gate/API/C4 regression.

Per Task9Step4, stop for this new Important rather than make an autonomous
architectural decision. Task9/B1 remain open; evidence.md records NO-GO and
separates the security blocker from B1-PERF-001. The optimization proposal under
docs/superpowers/plans/2026-11-01-b1-search-performance-proposal.md is unapproved
and implements nothing.

The existing N=100,000 supervisor5500/native30872 continues unchanged. Snapshot
at 15:22 Europe/Rome:17/100 actual queries completed (8 exact/6 BM25/3 filtered),
rebuild log1454.158s, populated RSS3727187968 bytes, peak3729625088 bytes. Raw
partial latency/log snapshots are retained separately; no full-sample P50/P95,
baseline/delta or fourth-scale numeric PASS is asserted. No second supervisor,
interruption, runtime optimization, canonical-spec change, merge, tag or push.
