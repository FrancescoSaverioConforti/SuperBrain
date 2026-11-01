# Authorized post-fix scalability sample

Measured core: `d798c74`, after the coordinator fix and before the Sensitive
temporal-metadata fix `3c6b1bb`. These are not measurements of the final patched
core. The user authorized **100 queries at each of
100, 1,000, 10,000 and 100,000 notes**, with 40 exact, 40 BM25 and 20 filtered.
This is an explicit deviation from the original N-dependent query budget.
The original full protocol is not claimed as executed.

The authorized sample is incomplete. N=100,000 was intentionally interrupted
under the user's cost-assessment authorization at 2026-11-01T14:52:54Z, after
completed setup/rebuild and 19 queries. There is no completion manifest or
all-scale aggregate, and no process remains active. Missing values stay missing;
no extrapolation or imputed percentile is used.

The current partial checkpoint includes actual raw samples for N=100, 1,000 and
10,000, plus executable/source build hashes. For N=100,000,
`latencies-100000.interrupted.csv` has rows 0–18 (8 exact / 8 BM25 / 3 filtered).
The unfinished query 19 is not timed or counted. Generated queries and the
corpus hash are retained. `benchmark-outcome.json` records rounded rebuild time,
populated RSS/peak, missing baseline/delta/P50/P95 and the intentional stop cause.
Independent numerical audit of all 19 rows: PASS as partial evidence only.

`latencies-100000.partial.csv` and `rebuild-100000.partial.log` are a historical
15:22 Europe/Rome checkpoint on 2026-11-01, with 17 completed queries and a
completed rebuild. They are not the completed 100-query input expected by the
reducer. The newer interrupted CSV supersedes that latency snapshot. The user
authorized the separate Sensitive-metadata fix; its independent review is GO.
See ../evidence.md for scope and the still-incomplete benchmark protocol.

The local supervisor configuration/status/logs are under:

```text
.superpowers/sdd/2026-10-30-b1-deterministic-search/perf-postfix/
```

`job-status.json` now records intentional interruption. `native-stop-status.json`
preserves the supervisor's native failure status after the verified worker was
stopped in the source-verified read-only query phase. The vault was retained;
there was no restart, fixture repair or database mutation. `driver.pid` is a
historical supervisor id, not a claim of current liveness.
While a native scale process runs, inspect its `process-N.stderr.log`
and `latencies-N.csv` for in-stage progress. Status timestamps are stage updates,
not a claim that an uncompleted sample has been validated. A running supervisor
holds an exclusive file lock. Before any separately authorized relaunch, verify that
no orphaned native worker owns the same synthetic vault.

The reviewed PowerShell driver runs scales serially, uses a single SHA256-pinned
release binary, captures native stdout/stderr separately and checks exit codes.
An absent note is created only on typed `NotFound`; an existing note must pass
`Vault::read_note` and match version 1/content byte-for-byte. All writes use
`propose_and_commit`. Setup is non-git (`JournalCommitted`/explicit `GitPending`),
and its recovery/population is outside rebuild/search measurement intervals.
No runtime storage/index optimization or disk-backed index is introduced.

The Python reducer accepts results only when every scale has exactly 100 query
rows and the authorized mix, validates phase/count/memory consistency and
recalculates nearest-rank P50/P95 from the raw data. It rejects missing scales,
non-finite measurements and Python optimization that could disable assertions.
Corpus/query/result hashes are retained; CSV/TSV/JSON files use UTF-8 LF.

Memory is process working set and lifetime high-water mark. The RSS difference
is approximate index attribution; allocator-retained/setup memory is not
perfectly isolated. Four timing phases are opt-in aggregate clocks on the same
validated search path: candidates, authoritative reads, revalidation, response
construction/ranking. Other setup/drop/timer overhead is a reported residual.
There is one measured run per scale, without confidence intervals or a pass/fail
threshold. Historical pre-fix pilots are separate and are not a controlled A/B
estimate of improvement. Interpretation of this incomplete run is in B1 evidence;
the interrupted prefix is not a balanced substitute for the authorized sample.
