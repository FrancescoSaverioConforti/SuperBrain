# B3-TUNE-v2 Priority 4 — Hybrid Retrieval: Design + Initial Audit

**Phase:** B3-TUNE-v2, Priority 4 (hybrid lexical + dense retrieval).
**Date:** 2026-11-14.
**Status: DESIGN ONLY. No implementation, no model download, no
embeddings benchmark has happened or is authorized by this document.**

**Revision note (2026-11-14, post-review):** this design was reviewed
and approved with 6 required changes. All six are incorporated below:
(1) model selection happens on DEV only, never calibration; (2)
retrieval/fusion hyperparameters are frozen before any calibration
run, diagnosed on DEV only; (3) embedding representation semantics are
frozen before benchmarking, not adjusted after seeing results; (4) a
hard privacy/searchable-surface parity requirement is added, with
explicit `Sensitive`-note handling; (5) cost benchmarking (latency,
RAM, build time) reuses B1 in the same harness as a cost-control
measurement, never a requality-evaluation; (6) promotion criteria are
frozen and user-approved BEFORE calibration is touched, not decided
after seeing calibration numbers. The corrected workflow throughout
this document is: **frozen shortlist → DEV benchmark → frozen
hyperparameters/representation → frozen promotion criteria → user
approval (STOP) → calibration → mechanical KEEP/REJECT.** Any earlier
wording in this document that referred to calibration as a
model-selection or criteria-setting step was in error and is corrected
below; see Section 6a/6b/8 for the frozen-criteria gate specifically.

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

**Recommendation: RRF remains the primary and, for this first round,
the only fusion strategy implemented.** Weighted score fusion is
**deferred and will not be implemented in this first round** — it
stays documented here (above) as a fallback to consider only if RRF
turns out promising but insufficient, and only as a separately
authorized follow-up, not a parallel candidate benchmarked alongside
RRF now.

**Risk common to both:** fusion only reorders/merges two already-
generated candidate lists; it cannot fix a case where neither BM25 nor
the dense retriever's top-N surfaced the relevant note at all. The
"recall ceiling" question (Section 6) has to be checked, on DEV, before
fusion strategy is even relevant.

### 3a. Hyperparameters frozen before calibration, diagnosed on DEV only

The following retrieval/fusion hyperparameters must be frozen in
writing before any calibration run, with any tuning/diagnosis of them
done exclusively on DEV:

- **Lexical candidate depth** — how many top-ranked B1 lexical
  candidates feed into the fused list.
- **Dense candidate depth** — how many top-N nearest-neighbor dense
  candidates feed into the fused list.
- **Final top-k** — how many fused results are returned (consistent
  with B1's own `@10` metric convention unless a reasoned deviation is
  frozen explicitly).
- **RRF constant `k`** — the standard literature default (`k=60`,
  Section 3) is the frozen starting point; if DEV diagnosis suggests a
  different value, that becomes the new frozen value, decided on DEV,
  never tuned against calibration.
- **Deduplication semantics** — how a note that appears in both the
  lexical and dense candidate lists is merged into one fused entry
  (e.g. is it one RRF computation using both ranks, as the fusion
  formula requires, or could it erroneously double-count — must be
  specified and tested).
- **Ranking/tie semantics** — how ties in the fused RRF score are
  broken (e.g. falling back to B1's existing note-id lexicographic
  tie-break, per the audit's Section 5 convention, for consistency).
- **Dense similarity metric** — restated here from Section 4b because
  it is also a fusion-relevant hyperparameter: whichever metric is
  frozen for the chosen model must be the same one used to rank dense
  candidates feeding into fusion.

None of these may be adjusted after a calibration run has been read,
for the same reason the embedding representation semantics (Section
4b) cannot be: doing so would let calibration results silently steer a
choice that is supposed to be fixed before calibration is ever touched.

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
step — not yet run, per the user's STOP instruction. A concrete
2-4-candidate shortlist against these axes, without downloading
anything, is the separate deliverable requested alongside this design
(see "Model-shortlist audit" deliverable, tracked outside this
document per the user's next-steps instruction).

**Vector index structure** is also undecided and out of scope to
choose here, but is worth flagging as a second decision stacked on top
of the model choice: at 53 notes, brute-force cosine similarity over
all vectors is almost certainly fast enough that no approximate index
(HNSW, IVF, etc.) is needed at all — adding one now would be
premature complexity for this corpus size. This should be revisited
only if/when corpus size grows enough that brute-force search becomes
the latency bottleneck; that threshold is not estimated here.

## 4a. Model-selection workflow — DEV only, never calibration

**Required correction:** model selection among shortlist candidates
happens strictly on **DEV**. Calibration must never be used to choose
between multiple candidate models.

Frozen workflow:

```
frozen shortlist (2-4 models, no downloads yet)
    -> benchmark each candidate on DEV only
    -> apply a deterministic selection rule (defined before benchmarking,
       not chosen after seeing DEV numbers)
    -> freeze exactly ONE model
    -> only then: calibration run, with the single frozen model
```

**Explicitly prohibited:** "run model A/B/C on calibration, see which
wins." That pattern uses the decision split to pick among alternatives,
which is a form of the same post-hoc-selection risk Priority 1's
rejected threshold-shopping and Priority 2 Lever B's checkpoint both
exist to prevent — calibration is for judging ONE frozen
configuration against frozen criteria, never for choosing among several.

The "deterministic selection rule" itself (e.g. "highest DEV
Recall@10 among candidates within the CPU/RAM budget," or a weighted
combination of DEV quality and cost axes) must itself be written down
and frozen before DEV benchmarking starts, for the same reason: a rule
chosen after seeing DEV results is no longer a rule, it's a
post-hoc rationalization of whichever candidate looked best.

## 4b. Representation semantics — frozen before any benchmark

Before any model is benchmarked on DEV, the following must be
specified in writing and then held fixed through DEV benchmarking,
model selection, and calibration — none of these may be adjusted after
seeing results at any stage:

- **Exact document text embedded** — e.g. the note's full
  `text_searchable` as already assembled by B1's indexing
  (`document.rs`), or some other defined subset; must be stated
  explicitly, not left to the embedding call's default input.
- **Exact query representation** — the raw query string, or a
  normalized/preprocessed form; must match whatever was actually used
  during benchmarking, not assumed equivalent to the document-side
  processing.
- **Model-specific query/document prefixes**, if the chosen model
  family requires them (some embedding models expect a literal
  `"query: "` / `"passage: "` prefix to produce correct embeddings) —
  must be identified per shortlisted model, not discovered after
  benchmarking already shows poor results.
- **Max sequence length** — the model's hard input token limit.
- **Truncation policy** — what happens to a note's text that exceeds
  the max sequence length (truncate head/tail, error, or chunk — see
  next item).
- **Chunking policy** — whether long notes are split into multiple
  embedded chunks at all, and if so, the exact splitting rule (by
  token count, by section boundary, etc.).
- **Chunk overlap**, if chunking is used — the exact overlap amount,
  stated as a fixed number, not tuned after seeing results.
- **Pooling** — how a model's per-token output vectors are reduced to
  one embedding (mean pooling, `[CLS]`-token, last-token, or a
  model-specific pooling the model card specifies).
- **Multi-chunk note aggregation** — if a note was split into multiple
  chunks, how their similarity scores or embeddings are combined back
  into one note-level signal for ranking (max-over-chunks,
  mean-over-chunks, or treating each chunk as its own candidate and
  deduplicating by note afterward) — must be decided here, not
  improvised once multi-chunk notes are actually observed.
- **Embedding normalization** — whether vectors are L2-normalized
  before storage/comparison (this interacts with the similarity metric
  choice below: cosine similarity is invariant to L2 normalization,
  dot product is not).
- **Similarity metric** — cosine similarity vs. dot product vs.
  Euclidean distance; must be the metric the chosen model was actually
  trained/evaluated with, per its model card, not assumed.

**These are frozen before DEV benchmarking begins and must not change
after seeing DEV or calibration results.** A change discovered to be
necessary mid-benchmarking (e.g. "this model needs a query prefix we
didn't specify") requires re-running the affected benchmarks under the
corrected, newly-frozen specification — not a silent fix applied only
where it happened to help.

## 4c. Privacy / searchable-surface parity — hard requirement

Dense retrieval **must respect the same visibility/redaction/
searchable-surface policy as the existing B1 pipeline.** This is a
hard requirement, not a quality preference, and is added here per
explicit instruction.

**The core rule: never embed content that B1 does not already render
searchable.** Concretely, per the ranking-pipeline audit
(`B3-TUNE-v2-PRIORITY3-RANKING-AUDIT.md`, Section 6): a `Sensitive`
note's `source` field is withheld from `text_searchable` entirely
(`document.rs:189-190`, confirmed never appended for `Sensitive`,
only for `Internal`/`Public`) and its body is excluded from excerpts.
Any embedding pipeline built for Priority 4 must embed exactly the
same text B1 treats as searchable for that note's classification —
**never** a separate "full note body" extraction that bypasses B1's
own redaction logic. Concretely:

- A `Sensitive` note's embedding input must be built from the same
  redacted `text_searchable` (or equivalent) B1 itself would expose
  for that note — not from the raw note body read directly from
  storage, which would reintroduce exactly the content B1 deliberately
  withholds.
- The vector embedding and vector index are themselves **derived
  sensitive artifacts** and must be treated with the same handling
  discipline as the rest of the project's local-first security model:
  - **Local-only** — generated and stored entirely on-disk, locally;
    no runtime network call for inference or storage.
  - **No runtime network** — consistent with Section 4's existing
    offline requirement (`docs/Spec_mvp.md:27`), restated here because
    it also has a privacy dimension: a hosted embeddings API would
    send note content off-device, which is a privacy violation
    independent of the offline-architecture violation already noted.
  - **No leaking excluded note content** — a vector embedding is a
    lossy but information-carrying representation of its input text;
    a nearest-neighbor search over an index built from a `Sensitive`
    note's *un-redacted* content could leak that content's semantic
    presence (e.g. "a note very similar to this exists") even if the
    exact text is never returned verbatim. The redaction-before-
    embedding rule above exists specifically to prevent this.
  - **Correct invalidation/rebuild** when source content changes (a
    note is edited, re-classified, or deleted — the corresponding
    vector entry must be rebuilt or removed, not left stale) or when
    the embedding model version changes (vectors from two different
    model versions are not comparable to each other; the index must
    not silently mix them).

**Required test coverage:** this design requires dedicated tests,
once implementation is authorized, that specifically construct a
`Sensitive` note and verify (a) its embedding input matches its
redacted `text_searchable`, never its raw body; (b) a nearest-neighbor
query that would only match the note's redacted-away content does not
surface it with any materially higher score than chance; (c) deleting
or re-classifying a note invalidates its stale vector entry. These
tests do not exist yet — no code has been written — but are a
required part of any future implementation plan, not an optional
nice-to-have.

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
matters:** measure the dense-standalone configuration's own
candidate-recall behavior **on DEV first** (corrected from an earlier
draft of this document, which incorrectly named calibration here — DEV
is where all model/embedding diagnosis belongs, per Section 4a).
If dense retrieval alone cannot surface the relevant note in its own
top-N candidate set for DEV queries B1 already gets right, hybrid
fusion cannot recover anything on those queries either — fusion can
only reorder/merge what both lists already contain. This should be
checked early, on DEV, before investing in fusion-strategy tuning or
touching calibration at all; a negative result here means the model/
embedding choice itself needs revisiting on DEV, never papered over by
moving on to calibration anyway.

## 6a. Cost benchmarking — same harness, not a new B1 quality evaluation

Quality baseline comparison continues to use the official, already-
frozen B1 numbers (`eval/results/B3/frozen-config.json`) — B1's
Recall@10/nDCG@10 are not re-measured.

**However, for cost metrics specifically — latency P50/P95, RAM, and
indexing/build cost — B1 must be re-run as a cost-control measurement,
in the identical harness/machine/configuration used for the dense and
hybrid configurations.** This is necessary because B1's existing
frozen numbers were never measured for latency/RAM/build-time in the
first place (B3's own closure, `B3-CLOSURE-SUMMARY.md`, reports only
Recall@10/nDCG@10/abstention — no cost metrics at all), so there is no
existing B1 cost baseline to diff against; one has to be produced,
under matching conditions, or the dense/hybrid cost numbers would be
uninterpretable (a hybrid system might look "slow" only because it was
measured on a different machine/load than B1 was, not because it is
actually slower). This is explicitly **not** a new B1 quality
evaluation and does not touch Recall@10/nDCG@10/thresholds/verdict —
only wall-clock/memory/disk measurements of the same frozen B1
configuration, for comparability.

At minimum, separate out:

- **Startup/model-load time** — for dense/hybrid, this includes
  loading the embedding model into memory; for the B1 cost-control
  run, this is whatever B1's own existing startup cost is (expected
  near-zero, but must be measured, not assumed).
- **Warm query end-to-end latency** — the full round-trip for a single
  query after startup, measured identically across all three
  configurations.
- **Embedding inference time** — isolated separately from vector-
  search time, for dense/hybrid only (not applicable to B1).
- **Vector-search time** — the nearest-neighbor lookup itself, isolated
  from embedding inference.
- **Hybrid fusion time** — the RRF merge step itself, isolated from
  both retrieval times that feed into it (hybrid only).
- **Peak RAM** — during indexing and during query, for all three
  configurations.
- **Vector artifact disk size** — dense/hybrid only; B1's existing
  lexical index size is already known and does not need re-measurement
  for this comparison.
- **Build/index time** — full corpus index build, for all three
  configurations (B1's build time has also not been previously
  measured and must be captured here for the first time, under the
  same harness).

## 7. Promotion bar — explicitly not "any metric improves"; criteria frozen BEFORE calibration

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

**Required correction to this document's earlier wording:** an earlier
draft deferred the "how large is large enough" judgment to "once real
calibration numbers exist." That is corrected here — **the promotion
criteria themselves must be proposed, reviewed, and frozen BEFORE any
calibration run, not decided after seeing calibration.** The mandatory
workflow, mirrored from Sections 4a/3a's model/hyperparameter freezing:

```
1. benchmark/diagnose on DEV
2. propose concrete acceptance/cost criteria (numeric, not vague)
3. STOP -- present proposal for user approval, no further action until
   approved
4. freeze the approved criteria
5. only then -- run calibration, exactly once, with the frozen model +
   frozen hyperparameters + frozen criteria
6. mechanical KEEP/REJECT against the frozen criteria -- not a fresh
   judgment call made upon seeing the calibration numbers
```

This closes the gap the earlier wording left open: calibration must
never be the moment the bar itself gets decided, only the moment a
fully pre-committed bar gets mechanically checked. This mirrors
Priority 1's own lesson in the opposite direction — Priority 1 was
rejected specifically because no single cutoff satisfied criteria
that, in hindsight, were somewhat informally held; Priority 4's bar
must be unambiguous and numeric before calibration, precisely to avoid
re-litigating "is this good enough" after seeing the number.

The frozen criteria proposal (step 2 above) must cover, at minimum:

- **Recall@10 regression tolerance** — maximum acceptable drop vs. B1,
  if any dimension of hybrid regresses while others improve.
- **Required nDCG@10 improvement**, or an explicitly defined
  equivalent quality gain — stated as a concrete minimum delta, not
  "meaningfully better."
- **Latency P50/P95 budget** — maximum acceptable added latency vs.
  the B1 cost-control measurement (Section 6a), stated in concrete
  units (e.g. milliseconds or a multiplier).
- **Peak RAM budget** — maximum acceptable RAM vs. the B1 cost-control
  measurement.
- **Model-weight footprint** — maximum acceptable on-disk model size.
- **Vector-index disk footprint** — maximum acceptable index artifact
  size.
- **Indexing/build-time budget** — maximum acceptable added build time
  vs. the B1 cost-control measurement.

**This document does not itself propose numeric values for these
criteria** — per the frozen workflow above, that proposal comes after
DEV benchmarking (step 2), grounded in real DEV numbers and real
measured costs, and requires the user's explicit STOP-and-approve
before calibration, not as something decided unilaterally in this
design document ahead of any data.

## 8. Next step

This document and `B3-TUNE-v2-PRIORITY3-CLOSURE.md` together complete
the Priority 3 closure and Priority 4 design + initial audit
deliverables, now revised with the 6 required corrections above. Per
explicit instruction, the following remain outstanding and are
tracked as separate next deliverables, **none implemented yet**:

1. A **model-shortlist audit** (2-4 realistic candidates, no
   downloads) against the axes in Section 4.
2. A final shortlist recommendation for DEV benchmarking.
3. A **Priority 4 Stage 1 plan**, scoped only as far as: DEV
   benchmarking of the shortlist (Section 4a) + a promotion-criteria
   proposal (Section 7, step 2) — stopping before model download,
   embeddings generation, calibration, or held-out, exactly as
   instructed.

This session stops here: no held-out read, no Priority 4 execution,
no model download, no embeddings benchmark, no calibration run, no
MCP, no merge/tag/push.
