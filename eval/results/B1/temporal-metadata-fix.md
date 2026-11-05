# Authorized B1-SEC-001 temporal metadata fix

2026-11-01; baseline `338fbf1`. The user explicitly chose separation of
structured metadata and searchable temporal text, without changing the canonical
schema, storage write gate or MVP specification. Independent verdict: **GO on
the fix**, no Critical/Important residual within the authorized scope. This does
not declare the interrupted 100-query sample or Task 9 complete.

## Cause and decision

The previous implementation followed the original ten-field Sensitive allowlist,
whose premise that the three date fields were constrained types was false.
Domain parsing accepts Strings. A field containing a distinctive marker passed
the real gate and entered both postings, then appeared in a Sensitive excerpt.
The independent historical reproduction remains in sensitive-date-metadata-repro.rs;
it intentionally describes the pre-fix finding, not a passing regression test.

The authorized fix makes all three temporal fields filter-only for Sensitive,
including valid dates. There is no malformed-value fallback to text. Seven
non-temporal fields retain their approved text behavior; this fix makes no new
claim that arbitrary project names or identifiers are inherently non-identifying.
Body, source and relation exclusions remain unchanged. Internal/Public searchable
text behavior is preserved. Direct retrieve still returns the authorized pointer
record and is not silently redefined by a search-only decision.

## Implementation and compatibility

`Document.created`/`updated` are now `Option<DateTime<Tz>>`, and `event_period`
is `Option<EventPeriod>` with Point/Interval variants. Construction parses
canonical YYYY-MM-DD dates or RFC3339 instants, normalizes into Europe/Rome,
and accepts event start/end only when both endpoints are valid and start < end.
Invalid dates, free text, suffixes, incomplete/reversed/zero-length intervals
become None. No raw invalid temporal string remains in a Sensitive Document,
its Debug representation or its searchable text.

Search's C4 filters consume those validated representations directly, retaining
half-open point membership and interval overlap. Any still means any valid
temporal field; an invalid field does not invalidate another valid field.
Parsing moved out of the filter's raw-string path; it is shared by live indexing,
cold rebuild and current-record revalidation through Document construction.
Non-zero-padded date spellings are rejected to match the documented YYYY-MM-DD
grammar. Storage/domain records are not rewritten or newly rejected; malformed
legacy values simply cannot satisfy their temporal filter. A canonical date can
still match text if independently present in an allowed id/project field; the
fix excludes temporal-field contributions, not coincidentally identical tokens.

No version/coordinator/lock protocol, Vault gate, DeletionLog, schema, dependency,
storage persistence or index persistence change was made. Existing versioned
atomic publication protects both postings and the new typed metadata together.

## TDD and regressions

Actual RED runs before changing production code:

- `cargo test -p index --locked sensitive_temporal -- --nocapture`: the new
  Document unit test fails with `created trigram marker leak`.
- `cargo test -p index --locked --test b1_search sensitive_temporal -- --nocapture`:
  three failures prove live postings leakage, searchable valid temporal text,
  and marker exposure after Internal -> Sensitive with a stale index.

GREEN verification covers eight additional tests in total: per-field marker
exclusion in both indexes; valid temporal values excluded from Sensitive text;
typed point/interval values; malformed/calendar/interval rejection; real
propose_and_commit live updates and repeated rebuilds; paused-index current-record
reclassification; valid C4 filters; and a deterministic barrier-controlled stale
Internal rebuild v1 racing a Sensitive live v2. The last test checks both
postings, represented version and Current, then rebuilds again. No sleeps.

The old ten-token golden expectation is deliberately updated to the seven
non-temporal tokens specified by the new user ruling. It does not remove coverage
of structured dates; typed assertions and real C4 searches cover that separately.

Checks on the patched runtime:

- `cargo test --workspace --locked --quiet`: **518 passed, 0 failed/ignored**,
  default parallel scheduling. Index84 unit +6 confidentiality +31 search; API11;
  all other workspace suites unchanged and passing.
- `cargo clippy --workspace --locked -- -D warnings`: clean.
- `cargo deny check --hide-inclusion-graph`: all four categories OK; existing
  duplicate-version/unused-license warnings only.
- Targeted mutation run: **77 tested, 65 caught, 12 unviable, 0 missed/timeouts**,
  successful unmutated baseline and exit 0. Command: `cargo mutants -p index
  --file crates/index/src/document.rs --file crates/index/src/search.rs
  --test-workspace false --jobs 2 --timeout 60`. Exact lists are retained in
  mutations/temporal-fix-caught.txt and temporal-fix-unviable.txt. This scoped
  pass is not represented as a new full-branch mutation pass.

Independent reviewer personally passed index 84+6+31, API 11 and workspace Clippy.
They confirmed the typed-only C4 path, exclusions even for valid dates, None/no
Debug fallback for invalid fields, unchanged Internal/Public text behavior,
paused-index classification revalidation and barrier-controlled both-postings
race. Whole-branch re-review found no additional Critical/Important. The same
reviewer independently checked all 19 interrupted benchmark rows, composition,
phase/count consistency and exact SHA; this is PASS for partial evidence only.

## Benchmark interruption is separate

The user authorized evaluating a controlled interruption when residual cost
became disproportionate. N=100,000 completed setup/rebuild and 19 query rows;
one broad query took 2416827.1975 ms (about 40 minutes) and 100000 authoritative
reads. Identity and completed rebuild/query phase were verified before stopping
the sole native worker. Source inspection confirms this phase only reads Vault
records and writes measurement output; no fixture mutation/commit was in flight.
The supervisor then exited through its normal nonzero-exit handling. No second
supervisor, restart, vault deletion/repair or database mutation was performed.

All 19 complete rows, generated queries, corpus hash and rounded rebuild/RSS/peak
log are retained under measurements/. The unfinished twentieth query is not
timed or counted. There is no 100-query P50/P95, RSS baseline/delta or extrapolation.
The native status is failed_review_required due to the intentional stop, not an
observed spontaneous storage failure. benchmark-outcome.json records that cause.
The measured core is d798c74, before this temporal fix; no final-core benchmark
performance claim is made. B1-PERF-001 and its separate optimization proposal
remain open. No performance optimization is included in this security fix.
