# B1 search performance proposal — awaiting authorization

This is a separate proposal, not an approved implementation plan. No optimization
is implemented by this document. The canonical MVP specification is unchanged.
Keep the index in memory, concurrent rebuild and the existing public storage
write path. Final B1 benchmark/review status is recorded in
`eval/results/B1/evidence.md`, not inferred from this proposal.

The final whole-branch review found Important B1-SEC-001 (unvalidated free-text
date values in Sensitive searchable metadata). A separate user-authorized fix
at `3c6b1bb` excludes those fields from Sensitive text and validates their filter
representations; independent re-review is GO. That security fix implements no
performance optimization. Benchmark completion and B1 closure remain distinct.

## Measured problem

The completed post-fix N=10,000 sample has 100 queries, split 40 exact / 40 BM25 /
20 filtered, k=10. Mixed P50/P95 are 4285.3518/16355.0836 ms. Exact P95 is
15.0333 ms with one authoritative read; BM25 P95 is 16662.6147 ms and filtered
P95 is 16155.7740 ms. Ranked/filtered queries perform 2,500–10,000 authoritative
reads. The read phase consumes 614559.0314 of 630058.9399 ms (97.539927%).
This is an observed fan-out problem; the timer includes all of `Vault::read_note`,
not just disk I/O. The N=100,000 run was intentionally interrupted after 19
queries when residual cost became disproportionate. One measured query used
100,000 reads and took 2416827.1975 ms (about 40 minutes); complete-sample
percentiles are unavailable. All these measurements are from `d798c74`, before
the temporal fix, and are not final-core performance results.

`candidate_ids` unions all exact and positive BM25 candidates, without a k cap.
`search` reads each candidate, reconstructs the current allowlisted Document,
matches/filters it and builds request-local BM25 statistics before truncating to
k. Thus limiting the output to ten does not limit authoritative reads. This
ordering protects correctness; replacing it with an arbitrary indexed top-k
would lose valid lower-ranked results after deletion/reclassification/filtering
and can change the current BM25 score semantics.

## Work packages for a separately approved implementation

1. **Attribute read cost before changing its implementation.** Add opt-in,
   aggregate-only timers/counters inside storage for per-note lock wait, journal
   lookup, record I/O/integrity and deletion/classification checks. Do not log
   content, paths or note ids. Measure matched fresh fixtures with the same
   workload and cache/setup declaration. Inspect journal lookup/query-plan costs
   inside storage's own boundary; repeated scanning is currently a source-level
   hypothesis, not a measured SQL bottleneck. Index/query-plan changes, if
   justified, require their own migration/failure tests and authorization.
   Cheaper reads alone do not satisfy the goal of reducing unnecessary reads.

2. **Specify and prove conservative candidate pruning.** Explore project/type/
   status/trust/date preselection from the coordinated versioned Document
   snapshot. A cached mismatch is not by itself permission to omit a note: an
   outdated or concurrently changing note can become eligible. Stale, unknown,
   failed-index and uncertain candidates must fall back to authoritative reads.
   Even `Current` means latest observed notification, not guaranteed knowledge
   of a storage commit before notification. Before implementing exclusions,
   demonstrate a storage-backed freshness/snapshot validation protocol that
   covers that window, deletion and reclassification; index-only generation
   checks are insufficient. Validate/restart before publishing or fall back to
   the existing full scan whenever the proof fails. If the necessary seam is
   broader than existing coordination, stop for a separate architectural ruling.
   The expected gain is fewer reads of provably ineligible candidates, especially
   filtered/project-scoped queries. There is no promised gain for broad unfiltered
   queries matching the whole corpus.

3. **Evaluate exact ranked-search pruning separately.** Establish bounds under
   the actual request-local BM25 statistics and deterministic tie ordering.
   Prove that unread candidates cannot change eligibility, corpus statistics,
   scores or the winning k before terminating early. Indexed scores and fixed
   oversampling are not such a proof. If exact bounds cannot preserve the existing
   behavior, retain full validation and report the limit. A different scoring or
   completeness contract needs explicit approval; it is not a performance patch.

4. **Only if measurements justify it, consider a storage-owned batch read seam.**
   Reuse safe query-scoped work while preserving the same per-note deletion,
   classification, committed-version and integrity guarantees and error behavior.
   Batch reads may reduce repeated overhead but do not inherently reduce the
   number of candidates read. No direct journal/SQLite access from `index`/`api`,
   no serving cached bodies, no persisted index, no gate-result cache across
   queries. Lock acquisition and callback ordering require a separate review.

Each package has a review/authorization boundary. Do not bundle an unproved
candidate cap, concurrency rewrite or storage migration into instrumentation.

## Acceptance and regression obligations

- Use the existing full-validation search as an oracle on frozen fixtures:
  same ids, order, scores, versions, excerpts, provenance, project/time context,
  typed absence and sanitized operational errors. Compare exact floating scores
  or justify any explicitly approved numeric tolerance in advance.
- Deterministic hooks/barriers cover a candidate moving into/out of filters,
  Internal/Sensitive transitions, deletion before/after preselection, failed live
  indexing, callback lag after durable commit, concurrent rebuild v1/live v2,
  equal-score ties, broad common-term queries and all candidates rejected. No
  sleep-based race tests and no degradation to premature `Current`.
- Every returned citation/content/version still comes from an authoritative
  validated record. Preserve DeletionLog fail-closed behavior, Sensitive body/
  source/relations exclusion, and operational failure versus `NotPresent`.
  Do not rely on a bounded shortlist to mask unread errors silently.
- Measure actual candidates, authoritative reads, phase times, P50/P95, rebuild
  and process RSS at all four authorized scales. Record workload/hash/build and
  cache conditions. Report fallback rate and cases with no read reduction; never
  extrapolate an unmeasured improvement. Compare runs under controlled conditions.
- Require TDD, independent security/concurrency/ranking review, relevant mutation
  testing, full workspace tests, Clippy and Cargo Deny before accepting a change.

Success means a measured reduction of unnecessary authoritative reads with the
same result and gate contracts. Safe fallbacks are preferable to an unproved
claim that k results require only k reads. No merge, tag or push is authorized.
