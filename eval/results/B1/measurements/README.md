# Authorized post-fix scalability sample

Core implementation: `d798c74`. The user authorized **100 queries at each of
100, 1,000, 10,000 and 100,000 notes**, with 40 exact, 40 BM25 and 20 filtered.
This is an explicit deviation from the original N-dependent query budget.
The original full protocol is not claimed as executed.

This directory is incomplete until the measurement supervisor produces all raw
files and `manifest.json`. Completion of that manifest means the authorized
sample passed mechanical validation, **not** that the final B1 review is done.
Missing values must stay missing; no extrapolation or imputed percentile is used.

The current partial checkpoint includes actual raw samples for N=100, 1,000 and
10,000, plus the executable/source build hashes. N=100,000 remains in progress.
There is deliberately no completed all-scale manifest or aggregate CSV yet.

The local supervisor configuration/status/logs are under:

```text
.superpowers/sdd/2026-10-30-b1-deterministic-search/perf-postfix/
```

`job-status.json` records the current stage; `driver.pid` identifies the
supervisor. While a native scale process runs, inspect its `process-N.stderr.log`
and `latencies-N.csv` for in-stage progress. Status timestamps are stage updates,
not a claim that an uncompleted sample has been validated. The supervisor holds
an exclusive file lock. Before relaunching after an external kill, verify that
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
estimate of improvement. Final interpretation belongs in B1 evidence after all
samples and independent numerical review are complete.
