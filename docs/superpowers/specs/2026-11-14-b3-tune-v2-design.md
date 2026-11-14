# B3-TUNE-v2 — Initial Design

Status: **approved with changes** (see continuity, this revision). This
document is still design-only for Priorities 2-4; a full implementation
plan exists separately for Priority 1 only
(`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority1-abstention.md`).
No code, no tuning, no new runs are authorized by this document itself —
only by that separate Priority 1 plan, once it is in turn approved.

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
diagnosis — they were never the held-out set.

### New v2 held-out final-test: build order (correction)

The new held-out final-test must be **built, versioned, hashed, and
frozen before v2's final configuration is chosen** — not designed or
adapted after seeing tuning results. Concretely, in this order:

1. Author the new held-out query/label set (new queries, new labels, or
   a genuinely new held-out partition) as one of Priority 1's own early
   tasks, **before** any abstention threshold is tuned against dev/
   calibration.
2. Commit it, compute and record its SHA-256 (same discipline as B3's
   `corpus_manifest_sha256`/`labels_sha256`), and freeze that hash in a
   v2-specific manifest — this is the "versioned, hashed, frozen" step,
   done early.
3. The held-out set's *content* is still never read for tuning purposes
   between that freeze and the final v2 evaluation run — only its
   existence, hash, and metadata (query count, category/difficulty
   distribution) are known during tuning, exactly as B3's own `test`
   split was known to exist but unread during B3.1-B3.4.
4. Only after Priority 1 (and, later, 2/3/4 if reached) converges on a
   proposed final configuration does anyone read this held-out set's
   content, by running the final-test exactly once — same discipline as
   B3.5.

This ordering, not just the "don't reuse the old one" rule, is the
correction: a held-out set designed *after* seeing tuning results is not
held-out in any meaningful sense, even if its content was never
literally read before — the act of shaping its queries around known
tuning outcomes contaminates it.

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

**Correction: do not assume raw BM25 top-1 score is globally calibratable
without first checking which signal actually discriminates.** Priority 1
is a measurement task before it is a tuning task. The first real step is
comparing candidate discriminating signals against each other on dev,
not picking one and tuning it blind:

- **top1 score** — the simplest signal, but BM25 scores are not
  naturally bounded or comparable across queries of different length/
  term rarity, so this may discriminate poorly on its own.
- **top1 − top2 margin** — how much the best candidate beats the
  second-best; a large margin suggests genuine confidence, a small one
  suggests the retrieval is guessing among similar-quality candidates.
- **top1/top2 ratio**, only when top2 exists and is nonzero (guard the
  same zero-denominator discipline B3's own metrics used — report
  `null`/undefined rather than a fabricated ratio when there's no
  second candidate).
- **query-local normalization**, if the real per-query score data
  supports it (e.g. normalizing by the query's own term count or by the
  corpus's score distribution for similar query lengths) — only pursued
  if the simpler signals above don't already discriminate well; not
  assumed necessary in advance.

None of this is available from B3's own `per_query` report data (it
has `recall_at_10`/`ndcg_at_10` per query, not raw scores) — getting it
requires either a new field in `index::search`'s response or a
diagnostic script calling `index::search` directly and reading
`SearchResult::score` for the top-2 results per query. This measurement
step is itself part of Priority 1's plan, not a precondition solved
elsewhere.

**Threshold granularity (correction): start with one global rule.**
Per-category thresholds are **not authorized yet** — B3's own category
breakdowns showed sample sizes too small (`n` as low as 2-4 in several
categories) to support a per-category threshold with any statistical
confidence. Priority 1 starts with a single, simple, auditable global
rule using whichever signal measurement shows discriminates best.
Per-category thresholds are a possible future refinement, requiring
their own evidence (a larger corpus/query set per category) before they
could be proposed — not something this phase pre-commits to.

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

### The v2 acceptance trade-off (correction)

**Not imposed**: "v2 must be ≥ B1 on every single metric." Gaining a
working abstention signal can justify a small, controlled loss on
Recall@10/nDCG@10 — a system that correctly says "I don't know"
sometimes is more useful than one that never does, even if that comes
with a slightly lower recall. The design instead carries forward a
**multi-objective acceptance criterion**:

- Abstention recall > B1's baseline (0.0, confirmed structural
  limitation) — any genuine improvement here is the point of Priority 1.
- Abstention precision sufficiently high — "sufficiently" is not fixed
  here; it is proposed from dev/calibration evidence, same as every
  other B3/v2 threshold.
- Recall@10 does not degrade beyond a frozen tolerance (a small
  allowed regression band around B1's 0.9259, not a hard floor equal
  to it).
- nDCG@10 does not degrade beyond a frozen tolerance (same treatment,
  around B1's 0.8061).

**The exact tolerance values are explicitly NOT decided here.** They
are proposed only after dev/calibration measurement (same two-stop
discipline B3.4 used: propose, then freeze only after approval) and
frozen before the new held-out final-test is read — never adjusted
after seeing that result.

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

1. Build, commit, hash, and freeze the new held-out final-test set
   FIRST (see "New v2 held-out final-test: build order" above) — before
   any tuning work begins, not after.
2. Reuse the existing 53-note corpus and dev/calibration query sets for
   development and diagnosis of Priority 1/2/3 changes.
3. **Each priority is a separate ablation against the previous accepted
   baseline, never an uncontrolled accumulation of changes (correction).**
   Concretely: Priority 1's abstention-thresholded configuration is
   measured against plain B1 as its own ablation; if accepted, it
   becomes the new reference baseline for Priority 2's lexical-tuning
   ablation; and so on. A later priority is never bundled with an
   earlier one's changes without first establishing that the earlier
   one's own ablation was measured in isolation.
4. Each ablation is measured against dev first (same discipline as B3:
   dev never gates, it diagnoses), then calibration for threshold/
   tolerance proposals.
5. Before any v2 configuration is declared better than the B1 baseline,
   run the new held-out final-test (built in step 1) exactly once,
   under the same freeze/report discipline B3 used (`frozen-config.json`
   analog, single run, no post-hoc tuning).
6. A v2 configuration only replaces the B1 baseline as "official" if it
   passes the multi-objective acceptance criterion above on that new
   held-out set — not a single-metric "beats B1 everywhere" bar.

### Experimental logging rule (correction)

For every variant tested (every ablation, at every stage — dev,
calibration, and the eventual final-test), record at least:

- The complete configuration (every parameter that differs from the
  previous accepted baseline, named explicitly — never "tuned a bit").
- The git commit the variant was run at.
- Corpus/labels SHA-256 hashes (same fields B3's reports already carry).
- Recall@10 (with CI).
- nDCG@10 (with CI).
- The abstention confusion matrix (TP/TN/FP/FN) and derived precision/
  recall/F1/accuracy, with the same null-on-undefined semantics B3
  established.
- Latency/resource delta versus the previous baseline, when available
  (not fabricated if not measured — recorded as "not measured" rather
  than omitted silently).
- The delta versus the **official** B1 baseline (not just the
  immediately preceding ablation), so drift across several accepted
  ablations stays visible.
- An explicit **KEEP / REJECT** decision for that variant, with the
  reason, before moving to the next one.

This is a logging discipline, not a new metric — it reuses B3's own
report shape and adds the KEEP/REJECT decision and the configuration
diff on top.

## Explicitly not decided here

- Exact abstention threshold value or which discriminating signal wins —
  Priority 1's own plan measures this; not pre-decided in this design.
- BM25 parameter values, specific reranking rules, or whether hybrid
  retrieval is ever actually needed — Priorities 2-4 remain
  design-only (per the user's explicit "non pianificare ancora Priority
  2/3/4 in dettaglio"); their own implementation plans come later, each
  separately authorized, after Priority 1's ablation is accepted or
  rejected.
- Whether the new v2 held-out final-test reuses this corpus with fresh
  queries, or needs an expanded/different corpus — settled by Priority
  1's own plan (which builds it first, per the correction above), not
  pre-decided in this design.
- The exact Recall@10/nDCG@10 degradation tolerances and the abstention
  precision bar — proposed only after dev/calibration measurement.

## What this document does NOT authorize

- No code changes, no new metric runs, no new held-out construction —
  all of that is authorized only by the separate Priority 1 plan
  (`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority1-abstention.md`),
  once that plan is itself approved.
- Priority 2, 3, and 4 work of any kind — design-only until each gets
  its own separate authorization round, the same way Priority 1 did.
- No modification to the B3 frozen config, runner, corpus, or labels
  (`eval/results/B3/frozen-config.json` and everything it pins remain
  the accepted B1 baseline's own record, untouched).
- No MCP, no merge/tag/push.
