# B1 mutation testing and survivor triage

2026-11-01. All runs use cargo-mutants 27.1.0 in isolated copies, without exclusions
of production files. Full passes select index+api; the coordinator pass narrows
to indexer_impl.rs; an extra policy pass runs API tests against ReadPolicy.

| Pass | considered | caught | unviable | timeout | missed |
| --- | --- | --- | --- | --- | --- |
| Initial pre-hardening index+api | 209 | 108 | 53 | 0 | 48 |
| After initial hardening, pre-coordination-fix | 218 | 162 | 54 | 0 | 2 |
| Authorized coordinator fix, targeted | 35 | 24 | 11 | 0 | 0 |
| Complete index+api coordinator fix, d798c74 | 237 | 177 | 58 | 0 | 2 equivalent |
| Additional ReadPolicy, API tests | 2 | 2 | 0 | 0 | 0 |
| Sensitive temporal fix, Document/search, 3c6b1bb | 77 | 65 | 12 | 0 | 0 |

The last row is a scoped post-temporal-fix run over document.rs/search.rs,
with an unmutated baseline and exit0. Exact lists are
mutations/temporal-fix-caught.txt and temporal-fix-unviable.txt. Command:
`cargo mutants -p index --file crates/index/src/document.rs --file
crates/index/src/search.rs --test-workspace false --jobs 2 --timeout 60`.
It covers the changed parser, searchable-field assembly and structured filter
path; it is not a new full index+api pass. Earlier full results remain historical
for d798c74, not silently relabelled as a full run of the patched core.

Baseline scenarios passed separately and are not counted as mutants. Unviable
means the mutant did not compile, not a passing test. cargo-mutants exits1 when
it reports missed mutants; the full pass exit1 is recorded honestly and accepted
only after the equivalence reasoning below, not presented as a zero-survivor run.
An earlier targeted attempt was interrupted after hook-removing mutants hung on
arrival barriers. Tests now use a bounded arrival watchdog plus resume barrier;
this changes liveness diagnostics, not timing-based race scheduling. Final runs
above have zero timeouts. Source files and results were not altered by in-place
mutation: scratch copies only.

## Initial 48 survivors: complete disposition

- Cheap-to-close untested-boundary class (18): ApiError Display 1, retrieve relation
  booleans 4, canonical Document enum-string mappings 8, SearchError Display 1,
  TimeResolutionError Display 1, event-period zero-length boundary 1, exactly-three-
  character trigram boundary 2. Closed with nonempty sanitized diagnostics, all-five-
  relation-kind tests, ten-field golden Sensitive metadata, empty/reversed/touching
  period cases, and the three-character boundary. All absent from final misses.
- Distinct-logic-gap class (28): BM25 numeric arithmetic and IDF variants. Previous
  tests pinned ranking/zero-overlap but not numeric scores. A hand-calculated
  three-document corpus (including an empty doc) pins N/df/avgdl/tf/length
  normalization with expected 1.6270842432246126 and 0.3335509626905221. These are
  rejected by final tests, without changing the already-correct BM25 formula.
- Equivalent-mutant class (2): exact same pair retained in the final full pass,
  proved below. No security-relevant survivor was accepted as merely a limit.
- Hard-to-close IO-guard class: no remaining missed mutant assigned here.

Additional security-relevant-found-and-closed logic was discovered during audit,
not hidden among equivalent survivors: a failed newer update retained Current;
independent whole-branch review then reproduced rebuildv1 overwriting livev2.
The authorized versioned-coordination fix closes both. Twelve deterministic tests
exercise real commits, pending/faulted versions, both postings, metadata,
classification/deletion gates, rollback, poisoned mutex and duplicate delivery.
See coordination-fix.md; the dedicated post-fix 35 pass has no missed mutants.

## Remaining two equivalents (output semantics under maintained invariants)

1. Bm25Index::query_ranked initial guard `empty_query || empty_corpus` -> `&&`.
   If query terms are empty, the scoring loop never inserts a score. If the corpus
   is empty, its document-frequency map is empty and every term is skipped.
   Either case still returns the identical empty vector. Both-empty still returns
   early. The guard's shortcut changes redundant work, not retrieval output.
2. Positive-score filter `score > 0` -> `score >= 0`. A score-map entry is created
   only for a present term with tf>=1 and1<=df<=N. The pinned IDF is
   ln(1+(N-df+0.5)/(df+0.5))>0; the tf contribution has positive numerator and
   denominator (k1=1.2,b=0.75). Entries are sums of positive contributions;
   zero-overlap docs have no map entry. Thus no zero score can reach this filter.

The independent final reviewer verified both arguments against the actual loop.
If smoothing, IDF, tokenization or map-population invariants change, re-evaluate
these dispositions; do not carry this reasoning forward blindly.

Commands: cargo mutants -p index -p api --test-workspace false --jobs 3 --timeout 60;
coordinator: -p index --file crates/index/src/indexer_impl.rs --jobs 2;
policy: -p policy --file crates/policy/src/read.rs --test-package api --jobs 1.
Output directories are recorded in the local SDD ledger. Exact relative-source
mutation lists are committed alongside this report. No benchmark completion or
quality-threshold claim follows from mutation results.
