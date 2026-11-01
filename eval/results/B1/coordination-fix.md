# B1 - authorized rebuild/live concurrency fix

Date: 2026-11-01. Baseline task checkpoint: ff6ddf1. The user explicitly authorized
this coordination/versioning fix after the final independent review found the
Important race. This document closes that finding, not the whole B1 protocol.

## Cause and reproduction

The original RealIndexer ignored the callback version. Rebuild read a committed
VaultRecord, discarded record.version, then separately updated trigram, BM25 and
state. A delayed v1 rebuild could therefore run after a successful v2 write,
replace its postings and still report Current. The independent reviewer
reproduced twice through real public APIs:

    committed_version=2 old_hits=1 new_hits=0 state=Current
    new_term_search=NotPresent

Search's authoritative revalidation prevented wrong excerpts from being returned,
but could not recover v2 terms missing from the candidate set. The original
sleep-based diagnostic harness is retained as historical evidence in
rebuild-race-repro.rs; it is NOT the regression test or part of CI.

## Fix and linearization points

RealIndexer now has one Mutex<IndexData> covering both postings, retained
allowlisted Document metadata, expected version and published version. All index
observers use this same lock. The version check and publication are synchronized,
not a check-then-act across independently locked structures.

For each note, expected_version is the highest committed version observed through
a live callback or authoritative rebuild read. The published snapshot contains
its own version and Document. Current is derived only when those versions agree.
An observed but unrepresented newer version is Stale, including indexing failure.
An older delivery cannot decrease the expected version or restore Current.

1. Observation linearizes under the shared lock, before parsing. Learning v2
   makes a previously published v1 Stale immediately.
2. Parsing happens outside the lock. Rebuild preserves VaultRecord.version and
   uses only the public authoritative read API.
3. Publication checks the expected version again under the shared lock, updates
   both postings, then publishes Document/version before releasing that lock.
   A delayed v1 publication is rejected after any observation of v2.
4. A publication unwind rolls back both postings while observers remain excluded.
   The old snapshot remains stale and can be repaired by a successful same-version
   retry/rebuild. No indexing unwind escapes mark_current into the storage commit.
5. A genuinely poisoned publication mutex resets derived postings/snapshots
   coherently to Stale while preserving version high-water marks. A later valid
   live update or rebuild repairs it. Process-aborting allocation failure is not
   claimed recoverable by Rust catch_unwind.

Current is relative to the latest notification/authoritative version OBSERVED by
this in-memory index. This does not claim visibility of a storage event before
its notification, or a global cross-note storage snapshot. Search still declares
possible candidate incompleteness and re-reads/revalidates all returned records.
Immutable committed versions are supplied by the existing trusted storage seam;
contradictory callback version headers or note ids are rejected.

## Lock ordering and scope

- Live write: existing Vault per-note lock -> index publication lock. No indexer
  callback into Vault, DeletionLog or PiiStore, and no new storage write path.
- Rebuild: Vault::read_note acquires/releases its own note lock first; only after
  it returns can the index observation/publication lock be acquired.
- Search: candidate generation releases the index lock before Vault::read_note.
- Test-only hooks are cloned outside their own mutex before invocation; only the
  AfterTrigram hook deliberately executes inside the publication transaction.

This preserves concurrent rebuild and the in-memory-only design. No canonical
specification, storage gate, DeletionLog, classification allowlist, dependency or
persistence format was changed. Retaining a Document and sharing a publication
lock can affect memory/contention; no architectural performance optimization was
introduced and pre-fix benchmarks are not measurements of this implementation.

## Deterministic TDD

Eight original tests compiled and failed against the old implementation (RED),
including actual stale overwrite, false Current after a failed newer write,
reclassification/deletion and an indexing unwind escaping a storage commit.
All eight became GREEN after the fix. Four additional tests close metadata,
contradictory-input, duplicate-delivery and poison-recovery boundaries.

All 12 tests in indexer_impl/concurrency_tests.rs use explicit test hooks and
handshakes; no sleep controls ordering. A channel watchdog only fails a missing
hook promptly (for example when a mutant removes it); the resume barrier controls
the interleaving. No timing delay is used to cause the race.

Covered cases:

- v1 rebuild paused after read, real v2 propose_and_commit wins, v1 cannot overwrite;
- observed v2 held before publication: old v1 remains Stale, never premature Current;
- faulted live v2 followed by late rebuild v1 stays Stale until authoritative v2 repair;
- tombstone deletion during rebuild cannot restore old terms or bypass read gates;
- Internal -> Sensitive during rebuild cannot republish body/source markers;
- readers cannot observe the transaction between trigram and BM25 publication;
- fault between postings rolls back both and the storage commit still succeeds;
- old/malformed delivery cannot decrease the high-water mark;
- status/project/classification metadata and version publish together;
- mismatched header/id is rejected and a valid retry of the expected version repairs;
- identical Current-version redelivery is idempotent;
- real mutex poisoning resets coherently and the next real commit still succeeds.

The earlier failed-update fixture now starts a fresh index for each independent
failure. Replaying v1 after an observed failed v2 can no longer reset version
ordering; that old fixture assumption contradicted the newly authorized contract.
Assertions that failed updates are Stale and preserve old postings remain.

## Verification and independent review

Final working-tree validation:

- cargo test -p index concurrency_tests --lib --quiet: 12 passed.
- cargo test --workspace --locked --quiet: 510 passed, 0 failed, 0 ignored.
  Counts: api11; domain32; index unit79; index confidentiality6; index search28;
  policy14; storage unit201; storage integration4+6+14+24; vault-security86;
  storage doctests5. Default parallel scheduling passed this run; known unrelated
  pre-B1 restore/deletion flakes remain recorded in Task7 evidence.
- cargo clippy --workspace --locked -- -D warnings: clean.
- cargo deny check --hide-inclusion-graph: advisories/bans/licenses/sources OK;
  existing duplicate-version/unused-license-allowance warnings only.
- git diff --check: clean. Canonical spec/storage/dependency graph unchanged.

Independent b1_final_review verdict: GO, no Critical/Important residual. The
reviewer inspected monotonicity, publication/observation linearization points,
lock ordering, rollback and no-callback/read-gate discipline; personally reran
75 index unit tests (8 initial interleavings),6 confidentiality,28 search,11 API,
and workspace Clippy. They then read the four added tests and personally reran
all 12 coordinator tests, confirming the nonblocking coverage notes were closed.

Targeted post-fix mutation pass: cargo mutants -p index --file crates/index/src/indexer_impl.rs --test-workspace false --jobs 2 --timeout 60. Result: 35 tested, 24 caught, 11 unviable, 0 missed, 0 timeouts. The exact lists are saved in coordination-mutants-caught.txt and coordination-mutants-unviable.txt. This targeted result does not claim a new full-workspace mutation pass.
The first attempt was stopped because mutants removing hooks caused indefinite
arrival-barrier waits; the final pass uses the bounded watchdog described above.
This is test-harness liveness, not an observed baseline deadlock.

## B1 completion status

The authorization-gated Important finding is fixed and independently approved.
Task9 as a whole is NOT declared complete. Earlier mutation/scalability data are
pre-fix; the original N-dependent full benchmark protocol was not executed.
User-authorized replacement remains 100 queries at EACH N=100/1,000/10,000/100,000,
with 40 exact / 40 BM25 / 20 filtered, no fabricated or extrapolated metrics.

No live benchmark process remains on resume. N=100,1,000,10,000 pre-fix runs have
complete CSV results; N=100,000 has only incomplete setup/corpus output, no
rebuild/latency summary. The old process session19915 no longer exists. Final
post-fix scalability measurements and remaining whole-B1 evidence must still be
completed before closing B1. No merge, tag or push performed.

## Follow-on Task 9 execution (2026-11-01)

The full post-fix index+api mutation run has completed:237 considered,177 caught,
58 unviable,2 equivalent,0 timeouts. The additional read-policy pass caught both
mutants. See mutation-triage.md and the committed relative-source lists. These
supersede the earlier pre-fix mutation totals for the current core.

The benchmark is now supervised automatically with the core from d798c74 and a
separately reviewed resumable fixture harness. It serializes scale runs, checks
the executable SHA256, separates native stdout/stderr, rejects native failures,
and validates all four actual100-query samples before publishing the aggregate.
The exact count is40 exact/40 BM25/20 filtered at each scale. Python optimization
cannot disable validation. No Git operation is performed by the measurement job.

This is still NOT a claim that B1 or the original full protocol is complete.
Current progress is in the local ignored perf-postfix/job-status.json; process
logs and partial latency CSVs remain available while a scale runs. The supervisor
updates status at stage boundaries; raw process stderr/CSV shows in-stage progress.
Final artifacts will appear under eval/results/B1/measurements only as measured
results, with a manifest flag explicitly denying execution of the original full
N-dependent protocol. Final independent numerical review remains required.
