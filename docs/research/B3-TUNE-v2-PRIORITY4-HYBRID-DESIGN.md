# B3-TUNE-v2 Priority 4 — Hybrid Retrieval: Design + Initial Audit

**Phase:** B3-TUNE-v2, Priority 4 (hybrid lexical + dense retrieval).
**Date:** 2026-11-14.
**Status: DESIGN ONLY. No implementation, no model download, no
embeddings benchmark has happened or is authorized by this document.**

This document satisfies the user's request for (1) an audit of the
hybrid/vector options and (2) a complete Priority 4 design, and
nothing past that. It follows directly from
`docs/research/B3-TUNE-v2-PRIORITY3-CLOSURE.md`: Priority 3 closed
with no accepted deterministic reranking configuration, so official B1
(`eval/results/B3/frozen-config.json`, commit `52d42ea`) remains the
only active baseline going into this design.

---

## 1. Why hybrid retrieval, and why not yet accepted a priori

B1 is pure lexical: candidate generation is trigram-substring ∪ BM25
term-overlap, final scoring is per-query candidate-scoped BM25, no
semantic signal anywhere in the pipeline
(`docs/research/B3-TUNE-v2-PRIORITY3-RANKING-AUDIT.md`, section 1-2).
This means B1 is structurally blind to queries that are semantically
on-target but share no tokens with the relevant note — paraphrase,
synonym, or conceptual-neighbor queries. Priority 1-3 all operated
strictly within the lexical pipeline (abstention thresholding,
lexical tuning, deterministic reranking) and none could address this
class of miss, because none of them can add a candidate that lexical
matching never surfaced in the first place. Dense/hybrid retrieval is
the first Priority in this project that can change **candidate
membership**, not just filtering, scoring weights, or ordering among
already-lexically-matched candidates.

This is also exactly why it must clear a higher bar than Priority 1-3:
every prior lever was additive and reversible with a single flag, at
effectively zero runtime/storage/licensing cost. A vector index is
none of those — it is a new model dependency, a new index artifact, a
new latency floor, and (per Section 4) a new offline/licensing
surface. The evaluation bar (Section 6) exists because "hybrid
measures better on nDCG@10" alone does not justify that cost; the
improvement has to be large enough to be worth it.

## 2. Three configurations to compare

1. **B1 lexical baseline, invariant.** Official frozen config,
   unmodified, exactly as measured in Priority 1-3. This is not a new
   measurement — it is the existing number, reused as the fixed
   comparison point so the hybrid numbers are read against the same
   baseline the whole B3-TUNE-v2 project has used throughout.
2. **Dense/vector retrieval, standalone.** Candidate generation and
   scoring both replaced by nearest-neighbor search over note
   embeddings — no BM25/trigram candidate generation at all in this
   configuration. Useful as a diagnostic upper/lower bound on "what
   can semantic similarity alone do on this corpus," not expected to
   be the shipped configuration, but necessary to interpret whether
   hybrid's gain (if any) is coming from the dense signal or just from
   having a larger candidate union.
3. **Lexical + dense hybrid.** B1's existing candidate generation
   (trigram ∪ BM25) unioned with a dense top-N nearest-neighbor
   candidate set, combined via a fusion strategy (Section 3). This is
   the configuration actually under consideration for promotion.

All three must run through the **real** search pipeline (not a
standalone scorer) for their final-decision measurement, per the
lesson Priority 2 Lever B established: a standalone scorer's
whole-corpus IDF/avgdl assumptions do not reproduce the real
pipeline's per-query candidate-scoped BM25, and the same risk of
divergence applies even more sharply here, since a standalone
embedding-similarity script would not reproduce the real pipeline's
filtering, redaction, and tie-break stages either.

## 3. Fusion strategy: weighted score fusion vs. RRF

**Weighted score fusion** (`final = α·bm25_norm + (1-α)·cosine_sim`)
requires BM25 and cosine-similarity scores to be on comparable scales.
They are not, natively: BM25 is unbounded above and corpus/query-size
dependent (Priority 2 Lever B's own real-pipeline finding — IDF/avgdl
are rescoped per query, so BM25 magnitude is not even stable across
queries within B1 itself); cosine similarity is bounded `[-1, 1]` (or
`[0, 1]` depending on normalization). Making these comparable requires
a normalization choice (min-max per query, softmax, z-score against a
reference distribution) — and that normalization choice is itself a
tunable parameter with no principled default, which reopens exactly
the "which cutoff looks best after seeing calibration" risk Priority 1
was rejected over and Priority 2 Lever B's checkpoint was built to
prevent. Weighted fusion is **not ruled out**, but it is the
higher-risk option precisely because of this calibration surface, and
should only be considered if RRF's simplicity turns out to cost too
much quality in practice.

**Reciprocal Rank Fusion (RRF)** — `score(d) = Σ 1/(k + rank_i(d))`
over each ranked list `i` the document appears in (standard constant
`k=60`, from the original Cormack/Clarke/Buettcher 2009 paper) — never
touches BM25 or cosine magnitude at all, only each list's **rank**
position. This sidesteps the scale-comparability problem entirely:
BM25's rank-1 and dense's rank-1 contribute identically regardless of
what their respective score distributions look like. The one free
parameter is `k` itself, and the standard literature value is a
defensible, non-tuned default — it does not need to be swept against
calibration to be used, which keeps this deterministic/auditable in
the same spirit as Priority 1-3's own levers.

**Recommendation: RRF as the primary candidate**, exactly for the
reason the user already flagged — it does not require BM25/vector
scores to be directly comparable. Weighted fusion is documented here
as the fallback to evaluate only if RRF's results are promising but
insufficient, not as a parallel first-class candidate, to avoid
doubling the evaluation surface before there is any evidence hybrid
helps at all.

**Risk common to both:** fusion only reorders/merges two already-
generated candidate lists; it cannot fix a case where neither BM25 nor
the dense retriever's top-N surfaced the relevant note at all. The
"recall ceiling" question (Section 6) has to be checked before fusion
strategy is even relevant.

## 4. Embeddings: options audit (no model chosen)

The project's `index` crate currently has zero ML/tensor dependencies
(`crates/index/Cargo.toml`: `chrono`, `chrono-tz`, `domain`, `storage`
only). Any embedding option is a wholly new dependency surface. The
project's own stated constraint
(`docs/Spec_mvp.md:27`): *"elaborazione dei dati solo locale... il
runtime dell'agente resta offline"* — the agent runtime is local-only
and stays offline. This is a hard constraint on every option below,
not a preference: no option that requires a network call at query
time or at indexing time in the shipped runtime is in scope. "Local
model" below means the model weights ship with/alongside the binary or
are fetched once during an explicit, user-visible setup step — never a
runtime dependency on a remote embedding API.

| Dimension | What has to be decided | Why it matters here specifically |
|---|---|---|
| **Model** | A local sentence/text embedding model (e.g. small BERT-family or distilled sentence-transformer class model; exact candidates not selected here) | Must run fully offline, with weights obtainable once and then local |
| **Embedding dimension** | Typically 384-1024 for small local models | Directly drives index size on disk and query-time NN search cost |
| **CPU vs GPU** | This is a local-first CLI/API tool, not a server with guaranteed GPU | Must assume CPU-only is the common case; a GPU path can be an optional accelerator, never a requirement |
| **RAM/disk footprint** | Model weights (tens of MB to low GB depending on model) + one vector per note (53 notes today, but must scale) | On a 53-note corpus this is negligible; the real question is the model weights' footprint on a local-first/offline tool, not the index |
| **Indexing time** | One embedding forward-pass per note, once per index build/update | At 53 notes, trivially fast with any local model; becomes a real cost only as corpus size grows — needs to be measured at the actual corpus scale, not assumed from the current 53-note sample |
| **Query latency** | One embedding forward-pass per query + nearest-neighbor search over the index | This is the dimension most likely to regress user-visible latency, since B1 today has no model inference in its query path at all — any embedding model adds a CPU inference step to every query |
| **Licensing/distribution** | Model weights carry their own license, separate from this project's own Apache-2.0 | Must be compatible with redistributing/bundling with a privately-developed, non-crates.io-published local-first tool; some popular embedding models carry licenses with attribution or field-of-use terms that need to be checked specifically before any choice, not assumed permissive by default |
| **Offline behavior** | Weights must be fully resident locally after one-time setup; zero runtime network calls | Directly required by `docs/Spec_mvp.md:27`; any option whose normal operation calls a hosted embeddings API is out of scope entirely, not just de-prioritized |

**No model is selected in this document.** The axes above are the
comparison criteria for a future, explicitly-authorized benchmarking
step — not yet run, per the user's STOP instruction.

**Vector index structure** is also undecided and out of scope to
choose here, but is worth flagging as a second decision stacked on top
of the model choice: at 53 notes, brute-force cosine similarity over
all vectors is almost certainly fast enough that no approximate index
(HNSW, IVF, etc.) is needed at all — adding one now would be
premature complexity for this corpus size. This should be revisited
only if/when corpus size grows enough that brute-force search becomes
the latency bottleneck; that threshold is not estimated here.

## 5. What this design deliberately does not do

- **No model is downloaded, loaded, or benchmarked.**
- **No embeddings are generated for any note.**
- **No vector index is built.**
- **No fusion code is written.**
- **No composition with any Priority 3 lever** — Priority 3 closed
  with nothing to compose with (Section 6 of
  `B3-TUNE-v2-PRIORITY3-CLOSURE.md`), so hybrid retrieval is designed
  here strictly on top of unmodified B1, not on top of any rejected or
  no-op lever.
- **The new v2 held-out set (`eval/rag/v2-heldout/`) is not read.**

## 6. Evaluation plan (for when implementation is authorized)

Splits, unchanged from Priority 1-3's own discipline:

- **dev** — development/diagnosis only, never the decision split.
- **calibration** — the decision split; all KEEP/REJECT/promote
  judgments are made here, exactly as in Priority 1-3.
- **v2 held-out** (`eval/rag/v2-heldout/`) — remains completely
  unread through this entire design step and through implementation;
  only read at the final, separately-authorized validation stage, per
  the project's standing discipline.
- The old B3 test split remains **consumed** (already used, not
  reusable as a fresh decision split) — unchanged from its existing
  status.

Metrics to collect, at minimum, for each of the three configurations
in Section 2:

- **Recall@10** + confidence interval.
- **nDCG@10** + confidence interval.
- **Per-category and per-difficulty** breakdowns (same categories/
  difficulty bands used throughout B3-TUNE-v2's existing reports).
- **Latency P50/P95** (query-time, including embedding inference for
  the dense/hybrid configurations — not just the search call).
- **Index/build time** (time to produce the vector index from the
  current corpus).
- **RAM** (peak resident memory during indexing and during query).
- **Index size on disk** (vector index artifact size, separate from
  the existing lexical index).
- **Delta against official B1** for every metric above — B1's own
  numbers are already measured and frozen; no B1 re-measurement is
  needed, only a diff.

**A critical pre-condition check, before fusion strategy even
matters:** measure the dense-standalone configuration's own Recall@10
on calibration first. If dense retrieval alone cannot surface the
relevant note in its own top-N candidate set for queries B1 already
gets right, hybrid fusion cannot recover anything on those queries
either — fusion can only reorder/merge what both lists already
contain. This should be checked early in implementation, before
investing in fusion-strategy tuning, since a negative result here
would mean the model/embedding choice itself needs revisiting before
fusion is even the right next question.

## 7. Promotion bar — explicitly not "any metric improves"

Per the user's explicit instruction, hybrid retrieval is **not**
accepted merely because some metric improves — unlike the simple KEEP
rule used for Priority 2/3's low-cost, reversible levers, the bar here
has to account for the much larger cost being requested:

- a new embedding model dependency (with its own licensing surface,
  Section 4);
- a new vector index artifact (disk + build time);
- additional RAM footprint;
- additional per-query latency (an inference pass that does not exist
  in B1's query path today);
- additional architectural complexity (a second retrieval system to
  maintain, test, and keep correct alongside the existing lexical
  one).

The improvement has to be large enough, specifically on calibration,
to justify all five of these costs together — not evaluated item by
item, since a hybrid system that is barely better on nDCG@10 but adds
a new offline-model dependency and doubles query latency is not a
good trade even if "the metric went up." No numeric threshold for
"large enough" is fixed in this document; that judgment is reserved
for the user once real calibration numbers exist, consistent with how
Priority 1's cutoff and Priority 2's lever decisions were made by the
user against real measured numbers, not by a pre-committed formula.

## 8. Next step

This document and `B3-TUNE-v2-PRIORITY3-CLOSURE.md` together complete
the two deliverables requested: Priority 3 closure, and Priority 4
design + initial audit. Per explicit instruction, this session stops
here: no held-out read, no Priority 4 implementation, no model
download, no embeddings benchmark, no merge/tag/push. The next
authorized step, if and when given, would be selecting a short list
of local embedding models against the axes in Section 4 for actual
benchmarking — not yet started.
