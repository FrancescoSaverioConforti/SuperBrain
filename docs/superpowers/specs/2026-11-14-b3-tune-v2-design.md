# B3-TUNE-v2 — Initial Design

Status: **initial draft only, per explicit user instruction** ("prepara
SOLO un design iniziale... Nessun tuning ancora"). Not approved for
implementation. No code, no tuning, no new runs authorized by this
document.

## Why this exists

B3 closed with the B1 deterministic trigram+BM25 baseline accepted
(`docs/research/B3-CLOSURE-SUMMARY.md`, continuity `d203i8`): Recall@10
and nDCG@10 both PASS their frozen thresholds, but abstention is a
confirmed structural limitation — the baseline never returns
`NotPresent`, on any of the 17 abstention-expected queries across
dev+calibration+test. B3-TUNE-v2 is the next macro-step that would
address this and any other improvement opportunity the B3 breakdowns
surfaced, **once separately authorized**.

## Hard constraint carried over from B3 (continuity `c28pk`)

**The B3 final-test (`eval/results/B3/test-20261114T132816Z-4a5f7a2.json`)
is CONSUMED.** It was read and reported as B3's own closing evidence. It
cannot be reused, re-read for its content, or treated as held-out
evidence for v2. Dev and calibration MAY be reused for v2 development and
diagnosis — they were never the held-out set. **Before any v2
configuration can be claimed better than the B1 baseline, a NEW,
never-before-read held-out final-test is required** — either a freshly
authored query/label split, or some other held-out set not yet read by
anyone in this project. Designing and building that new held-out set is
itself part of v2's own scope, not a precondition solved elsewhere.

## Priority order (per user instruction, not open to reordering without new authorization)

### Priority 1 — Abstention thresholding

**The problem**: B1's retrieval always returns *some* ranked result, even
when nothing in the corpus actually answers the query (`SearchOutcome`
is `NotPresent` only when BM25+trigram find literally zero candidate
matches at all — not when the best match is a poor one). The 17
abstention-expected queries across B3's splits all score low on
relevance but still come back as `Matches`, because "low-confidence
match" and "no match" are different states that B1's search path does
not currently distinguish downstream of candidate retrieval.

**Candidate approach**: introduce a score-based abstention threshold —
if the top result's BM25 score (or some normalized confidence derived
from it) falls below a tuned cutoff, report `NotPresent` instead of
`Matches`, even though a candidate technically exists. This requires:
- Reading the real score distribution of true-positive matches vs.
  abstention-expected "near miss" queries on dev (B3's own `per_query`
  data from the dev/calibration reports is the starting evidence — it
  already has `recall_at_10`/`ndcg_at_10` per query, though not the raw
  BM25 score; getting the raw score would need either a new field in
  the search response or a diagnostic script against `index::search`
  directly).
- A threshold is a new tunable parameter, calibrated the same way B1's
  Recall@10/nDCG@10 thresholds were: proposed from dev, frozen from
  calibration, verified on a new held-out set.
- Risk: lowering the "claims to have an answer" rate could reduce
  Recall@10 for genuinely answerable-but-lexically-distant queries
  (the `temporal` category's low n=2 at 0.50 Recall@10 in the final-test
  is exactly the kind of case a naive score cutoff could make worse, not
  better, if those queries already score low on legitimate matches).

**Open design question for the next authorization round**: should the
threshold apply uniformly across categories, or be allowed to vary
(since B3's breakdowns show real per-category spread in both recall and
score behavior)? This needs real score-distribution data before it can
be answered, not guessed now.

### Priority 2 — Lexical tuning

**The problem**: B1 is pure trigram+BM25 with no query expansion,
stemming beyond what's already in the existing tokenizer, or synonym
handling. Some of dev/calibration/test's lower-scoring queries (e.g.
paraphrases at `difficulty: hard`) may be recoverable purely through
better lexical matching, without touching ranking architecture.

**Candidate approaches**, roughly in increasing complexity:
- BM25 parameter tuning (`k1`/`b`) against the real corpus's length
  distribution — currently whatever `index::bm25::Bm25Index` defaults
  to; never tuned against this specific corpus.
- Light query normalization beyond what's already done (case folding,
  punctuation — verify what's already in place before assuming a gap).
- A small, explicit synonym/alias table for domain terms this corpus
  repeats (e.g. "SEC-001" ↔ "symlink TOCTOU bug", "B2-ID-018" ↔
  "TypeScript interface extraction gap") — cheap, auditable, and
  reversible, unlike a learned embedding.

**Why this is Priority 2, not Priority 3**: the design doc's own spec
constraint (`docs/Spec_mvp.md` Fase B) requires B1 to pass with the
deterministic baseline alone before embeddings are even considered —
lexical tuning stays inside that same "deterministic, no embeddings"
contract B3 already measured against, so it is the natural next lever
before reaching for anything heavier.

### Priority 3 — Reranking

**The problem**: BM25's ranking is a single-pass lexical score; a
second-pass reranker (even a simple deterministic one — e.g. a
cross-field boost, a recency/trust-level tiebreak already partially
present in `index::search`'s existing tie-breaking) could reorder
*already-retrieved* candidates without expanding what's considered a
candidate at all.

**Candidate approach**: a deterministic reranking step applied to
whatever BM25 already retrieves — not a learned reranker (out of scope
for a "deterministic, no embeddings" baseline family), but a rule-based
reordering using signals already in the index (e.g. `MatchKind::ExactSubstring`
vs `RankedTerms`, note type, trust level) that B1's current ranking may
not fully exploit. This needs a concrete audit of what `index::search`'s
current ranking logic already does with these signals before proposing
changes — avoid re-deriving signals it already uses.

### Priority 4 — Hybrid retrieval (only if justified)

**Explicitly conditional**, per both the user's instruction here and the
B3 design's own constraint (`docs/Spec_mvp.md`): CPU embeddings / hybrid
retrieval are introduced **only if** Priorities 1-3 are measured and
found insufficient against the new held-out final-test, and only if the
improvement is large enough to justify the added resource cost (the
spec's own language: "senza peggiorare latenza e RAM oltre i limiti").
This priority is not scoped further here — if reached, it gets its own
design pass once there is real evidence that lexical tuning and
reranking alone don't close the gap.

## v2 evaluation plan sketch (not a committed plan — subject to revision once v2 work is actually authorized)

1. Reuse the existing 53-note corpus and dev/calibration query sets for
   development and diagnosis of Priority 1/2/3 changes.
2. Each change is measured against dev first (same discipline as B3:
   dev never gates, it diagnoses), then calibration for threshold
   proposals.
3. Before any v2 configuration is declared better than the B1 baseline,
   build and freeze a **new** held-out final-test (new queries, new
   labels, or a genuinely new held-out partition — to be decided when
   this phase starts) that has never been read by anyone, and run it
   exactly once, under the same freeze/report discipline B3 used
   (`frozen-config.json` analog, single run, no post-hoc tuning).
4. A v2 configuration only replaces the B1 baseline as "official" if it
   passes its own frozen thresholds on that new held-out set — matching
   or exceeding B1's Recall@10/nDCG@10 is necessary but not sufficient;
   the whole freeze/final-test discipline must repeat, not be skipped
   because v2 "should obviously be better."

## Explicitly not decided here

- Exact abstention threshold value, BM25 parameter values, specific
  reranking rules, or whether hybrid retrieval is ever actually needed —
  all require real measurement against dev/calibration first.
- Whether the new v2 held-out final-test reuses this corpus with fresh
  queries, or needs an expanded/different corpus — a question for v2's
  own brainstorming/design phase, not pre-decided here.
- Timeline or task breakdown — this is a priorities-and-constraints
  sketch, not an implementation plan. A real plan (via
  `superpowers:writing-plans`) comes only after v2 work is separately
  authorized, same process B3.1 through B3.5 each followed.

## What this document does NOT authorize

- No code changes.
- No new metric runs (dev, calibration, or any new held-out set).
- No modification to the B3 frozen config, runner, corpus, or labels.
- No MCP, no merge/tag/push.
