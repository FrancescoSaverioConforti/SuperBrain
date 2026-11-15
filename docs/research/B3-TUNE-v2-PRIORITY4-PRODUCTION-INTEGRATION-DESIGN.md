# Dense Retrieval v2 — Production Integration Design

**Status: DESIGN ONLY. No implementation in this document or as a
result of it. No code has been written against `crates/index` or
`crates/api/src` for this design.**

**Date:** 2026-11-15. **Follows:** `B3-TUNE-v2-CLOSURE-SUMMARY.md` —
`all-MiniLM-L6-v2 dense-only` is the accepted v2 configuration, backed
by the frozen `eval/results/B3-TUNE-v2/frozen-config-v2.json`. This
document is the first step toward eventually bringing that
*evaluation-harness* configuration into the *production* `index`/`api`
crates — it is a design to review, not a plan to execute yet.

---

## 0. Scope and non-goals

**In scope:** how `v2 MiniLM dense-only` retrieval could be
integrated into the production `index`/`api` crates, as an addition
alongside B1, not a replacement decided here.

**Explicitly out of scope / not decided by this document:**
- Whether v2 *should* replace B1 as the default — that is a product
  decision for later, informed by this design but not made by it.
- Weighted score fusion or any new fusion strategy (still deferred).
- Any new evaluation cycle, model change, or held-out read.
- Actual implementation of any component below.

## 1. Index lifecycle

**When embeddings are generated.** Candidate approach: embeddings are
generated lazily, on first dense-search request after a note is
committed/updated, rather than synchronously inside
`storage::propose_and_commit`'s write path — this keeps the write path
free of a model-inference dependency, consistent with B1's own
candidate-generation design (audit Section 1: candidate generation
reads already-indexed state, never blocks a write on indexing work).
Alternative: a background indexing worker triggered by commit events,
closer to how a search engine's indexer typically decouples from
writes. **Trade-off not resolved here:** lazy-on-read risks a slow
first dense query after a burst of writes; a background worker adds a
process/thread to reason about. Needs a decision before implementation.

**Incremental update when a note changes.** A note's vector must be
re-embedded whenever its `text_searchable` changes — not on every
metadata change. Candidate approach: hash `text_searchable` itself (a
cheap, deterministic check) and only re-embed when that hash differs
from the vector index's stored hash for that note id. This avoids
re-embedding on, e.g., a `status` or `trust` change that doesn't touch
`text_searchable` at all.

**Delete handling.** When a note is deleted (per the existing
deletion-log protocol in `docs/Spec_mvp.md`), its vector entry must be
removed from the dense index in the same logical step, not left as a
dangling, searchable-but-orphaned vector. This needs to hook into
whatever mechanism already handles B1's own index removal on delete —
reuse that hook, don't invent a parallel one.

**Stale-vector invalidation.** Two independent triggers: (a) the
note's own content changed (handled by the hash check above), and (b)
the **embedding model itself changed** (a different model, or a new
revision/hash of the same model) — in this case, every vector in the
index is stale regardless of note content, and the only correct
response is a full rebuild, never a partial one (mixing vectors from
two model versions in one index is not a valid state — their
embedding spaces are not comparable).

**Model/version hash in index metadata.** The vector index's own
on-disk metadata must record the exact frozen model identity: HF repo,
pinned revision, ONNX artifact SHA-256, tokenizer artifact SHA-256 —
the same fields already captured in `frozen-config-v2.json`. On
startup, the running model's hashes must be compared against the
index's stored hashes; a mismatch triggers the full-rebuild path from
the paragraph above, never a silent continued use of a mismatched
index.

**Rebuild rules.** A full rebuild (every note re-embedded, a new
vector index written from scratch) is required when: the model
identity changes, the vector index format/schema version changes
(Section 2), or the index is found corrupted (Section 2). An
incremental update (only changed/new/deleted notes touched) is
sufficient when only note content changed. This distinction needs to
be enforced at startup, not left to be inferred by whichever code
happens to run first.

## 2. Storage

**Vector index format.** Not chosen here. Candidates to evaluate when
implementation starts: a flat, memory-mapped array of `f32` vectors
keyed by note id (simplest; matches this evaluation's own brute-force
approach, Section "Vector index structure" of the hybrid design doc,
which found brute-force adequate at 53 notes and flagged revisiting
only if corpus size grows); or a dedicated embedded vector-index
library, if/when corpus size growth makes brute-force the bottleneck.
**This choice is deferred to implementation time**, informed by actual
corpus-size growth, not decided speculatively here.

**Atomic write / recovery.** The vector index must never be left in a
partially-written state readable by a concurrent search. Candidate
approach: write-to-temp-file-then-atomic-rename, mirroring whatever
pattern `storage`'s own journal/vault already uses for crash-safety
(the project's own `docs/Spec_mvp.md` deletion-log protocol already
reasons carefully about exactly this class of problem for its own
journal — the vector index should follow the same discipline, not
invent a separate one).

**Corruption detection.** On load, verify the index file's own
integrity (e.g. a stored checksum of the vector data, checked against
a freshly computed one) before trusting it for search. A corrupted
index should fail closed into a rebuild, not silently serve wrong or
truncated vectors.

**Schema/versioning.** The on-disk index format itself needs a version
number, independent of the model-identity hash — so a future format
change (e.g. switching from flat-array to a real vector-index library)
can be detected and trigger a rebuild, distinct from a model change.

**Migration strategy.** Not designed in detail here — flagged as
needing its own sub-design once an actual format change is proposed,
following the same "audit before design before implementation"
discipline this whole B3-TUNE-v2 cycle has used throughout.

## 3. Search API

**How current `api::search` exposes dense retrieval.** Candidate
approach: an additive, opt-in parameter on the existing search
entrypoint (mirroring every Priority 2/3 lever's own
`with_*`-builder-style, default-off discipline) — e.g. a
`retrieval_mode` or similar flag, defaulting to B1-lexical-only, so
every existing call site is provably unaffected unless it explicitly
opts in. This preserves the exact "additive, never a silent behavior
change" pattern this entire project has used for every lever so far.

**Whether B1 remains fallback.** Yes, in this design's working
assumption — not a final decision: if the dense model/index is
unavailable (not yet built, corrupted, or the embedding runtime fails
to initialize), search should fall back to B1-lexical-only rather than
failing the request outright. This needs to be an explicit, logged
fallback, not a silent one, so an operator can tell dense retrieval is
degraded.

**Behavior when model/index is unavailable.** Three sub-cases to
design explicitly, not conflate: (a) index not yet built (first run,
before any embeddings exist) — fall back to B1, log once, do not
error; (b) index present but model-hash-mismatched (Section 1) —
treat as unavailable until rebuilt, same fallback; (c) ONNX Runtime
itself fails to initialize (missing native library, corrupted model
file) — same fallback, but this is also the case most worth a loud
startup warning, since it likely indicates a packaging problem
(Section 6), not a transient data issue.

**Deterministic ordering/ties.** Dense search's own ordering (cosine
similarity descending) needs the same kind of defined, deterministic
tie-break B1 already has (score desc, then note-id lexicographic asc,
per the ranking-pipeline audit Section 5) — ties in cosine similarity
are plausible at float precision and must not produce nondeterministic
output ordering between runs.

**`NotPresent` / abstention behavior remains explicitly unresolved
unless separately designed.** Dense retrieval does not change B1's own
unresolved abstention gap (Priority 1's REJECT verdict stands
unchanged — nothing in Priority 4 addresses "does this query have no
good answer at all"). This design does **not** propose solving
abstention via a dense-similarity threshold or any other mechanism;
doing so would reopen Priority 1 without new evidence, which is
explicitly not authorized by this closure.

## 4. Privacy/security

**Same searchable-surface parity as evaluation.** Production dense
retrieval must embed exactly `text_searchable`, the same
redaction-aware field B1 itself uses — never a raw note-body read that
bypasses classification-based redaction. This is not a new rule for
production; it is the same rule the evaluation harness's privacy test
(Section 5.4 of the closure) already verified and must carry forward
unchanged into production code, not be re-derived separately.

**`Sensitive`-note redaction.** Identical behavior to B1's own: a
`Sensitive` note's `source` is withheld from `text_searchable`
entirely; its embedding input must reflect that withholding, never the
unredacted body. The evaluation's `Sensitive`-note privacy test
(structural + differential check against an `Internal` control) should
become a production test suite requirement, not something re-verified
only once in an evaluation harness and then trusted forever after.

**Embeddings as derived sensitive artifacts.** The production vector
index is a derived sensitive artifact in exactly the sense the hybrid
design doc's Section 4c already established: local-only, no runtime
network, correctly invalidated on content/model-version change
(Section 1 above), and never permitting a `Sensitive` note's
redacted-away content to leak through similarity search (verified at
evaluation time; must remain true in production, not weaken once the
evaluation's own test suite isn't the only thing running).

**Local-only model/runtime, no runtime network.** Unchanged from the
project's own hard offline/local-first constraint
(`docs/Spec_mvp.md:27`) — the production embedding runtime must never
make a network call at inference time, model-load time, or any other
runtime point. Model weight acquisition (a one-time, explicit setup
step) is addressed separately in Section 6; it is not a "runtime"
network call in the sense this constraint forbids.

## 5. Runtime

**Model loading/caching.** The ONNX model and tokenizer should be
loaded once per process lifetime (not per-query) and cached in memory
for the process's duration — mirroring the evaluation harness's own
`EmbeddingModel::load` pattern, which already does this. Candidate
question for implementation time: whether a long-running service
process keeps the model loaded indefinitely, or a short-lived CLI
invocation pays the load cost every time — this depends on how
`brainCompany`/the agent runtime actually invokes search, which is
outside this design's own knowledge and needs clarifying before
implementation.

**ONNX Runtime lifecycle.** One `ort::Environment` per process,
initialized once (mirroring the evaluation harness's own
`ort::init().build()` call), not re-initialized per search request.

**CPU execution.** Unchanged from evaluation: CPU-only is the
baseline assumption (per the hybrid design's own embeddings audit,
Section 4) — no GPU requirement, consistent with this being a
local-first tool with no guaranteed GPU. An optional GPU execution
provider could be considered later as a pure performance optimization,
never a requirement, and is not designed here.

**Concurrency/threading.** Dense search's embedding-inference step is
CPU-bound; if multiple search requests can be concurrent (depends on
how `api::search` is actually called — not established by this
document), the ONNX session's own thread-safety properties (an `ort`
`Session` is typically safely shared behind a reference, but this must
be confirmed against the actual `ort` version pinned in
`frozen-config-v2.json` before relying on it) need to be confirmed, not
assumed, before any concurrent-access implementation.

**Memory ownership.** The loaded model (tens of MB, per Section 6's
footprint) and the in-memory vector index (small at current corpus
scale, per the evaluation's own measurements) are both long-lived,
process-scoped resources — ownership should live at whatever level
already owns B1's own persistent `RealIndexer`/`Bm25Index`/
`TrigramIndex` state (per the ranking-pipeline audit), not be
duplicated or independently lifecycled.

**Startup vs. lazy load.** Candidate approach: lazy-load the model on
first dense-search call, not at process startup — avoids paying the
model-load cost (155ms for MiniLM, per the evaluation's own cost
measurements) for processes/invocations that never actually use dense
search. Trade-off: first dense query in a session pays the load cost
inline, which needs to be acceptable or explicitly warmed, a decision
for implementation time informed by real usage patterns.

## 6. Packaging

**Model-weight acquisition.** A one-time, explicit setup step (not a
runtime behavior) that fetches the pinned model/tokenizer artifacts
from their source (e.g. the Hugging Face repository, exactly as this
evaluation's own harness did) and caches them locally
(`.priority4-model-cache/` was this evaluation's own convention; a
production equivalent needs its own, documented cache location). This
must be a deliberate, user-visible action (e.g. a setup command), not
something that silently happens on first use without the user knowing
a download occurred.

**Pinning/hash verification.** Production packaging must verify the
fetched model/tokenizer artifacts' SHA-256 against the exact hashes
recorded in `frozen-config-v2.json` before trusting them — the same
discipline the project's own stated principle requires
("allowlist, versioni e hash bloccati" per `docs/Spec_mvp.md:27`) for
its acquisition network generally. A hash mismatch at acquisition time
should hard-fail setup, not silently proceed with an unverified
artifact.

**Offline setup.** Once the one-time acquisition step completes, all
subsequent operation must be fully offline — no further network access
for model loading, inference, or index operations. This mirrors the
evaluation's own offline-after-first-fetch behavior exactly.

**License attribution.** `all-MiniLM-L6-v2`'s Apache-2.0 license
(confirmed in the shortlist audit) must be attributed per its license
terms in whatever production distribution/documentation accompanies
this feature — not assumed to need no attribution just because
Apache-2.0 is permissive.

**Windows support.** This entire evaluation cycle ran on Windows (this
machine), including discovering and resolving the `core.autocrlf`
CRLF/LF artifact (Section 5.6 of the closure) — production packaging
should explicitly test the acquisition/caching/loading path on
Windows, not assume Linux-developed packaging logic transfers
unchanged (the CRLF/LF lesson from this cycle is a concrete instance
of exactly this class of risk, applied to a different file than a
model artifact, but the same platform-dependency category).

## 7. Testing

**Parity against frozen evaluation implementation.** Production dense
retrieval's output (given the same corpus, same query, same frozen
model) must match the evaluation harness's own output exactly — same
embeddings, same similarity scores, same top-k ordering. A parity test
suite comparing production code's output against the evaluation
harness's already-committed reports (e.g. replaying DEV queries
through production code and diffing against
`priority4-dev-all-MiniLM-L6-v2.json`) is the most direct way to catch
a representation-semantics drift between the two implementations —
this is the single most important test category, since every
Priority 4 quality number is only trustworthy for production if
production actually implements what was evaluated.

**Golden embedding/query tests.** A small, fixed set of known
inputs → known-expected-embedding-vector (or known-expected top-k
result) tests, to catch a silent regression from a dependency upgrade
(e.g. a new `ort` or `tokenizers` version subtly changing numeric
output) that a parity-only test might not isolate as clearly.

**Index invalidation tests.** Cover each trigger from Section 1:
note-content change triggers re-embed of that note only; model-hash
mismatch triggers full rebuild; corrupted-index detection (Section 2)
triggers rebuild, not silent continued use.

**Crash/recovery tests.** Mirroring the project's own existing
discipline for its journal/vault crash-safety testing
(`docs/Spec_mvp.md`'s own test list for the deletion-log protocol):
crash mid-write of the vector index, verify recovery lands on either
the old or the new complete index, never a partial one.

**Privacy tests.** The evaluation's own `Sensitive`-note structural +
differential test (Section 5.4 of the closure) becomes a required,
permanent production test — run on every change to the embedding
input-construction code path, not just once during evaluation.

**Regression against B1 behavior where relevant.** Enabling dense
retrieval (opt-in, per Section 3) must not change B1's own behavior
when dense retrieval is NOT requested — a regression test asserting
B1-only call sites produce byte-identical results before and after
this feature's introduction.

## 8. Rollback

**Feature flag/config switch.** Dense retrieval must be gated behind
an explicit, off-by-default configuration switch at the point of
production introduction — consistent with every prior lever in this
project's own additive-opt-in discipline, not a special exception for
this one.

**Ability to fall back to B1 lexical retrieval if dense runtime/index
fails.** Already addressed functionally in Section 3 ("Behavior when
model/index is unavailable") — restated here as the rollback
mechanism specifically: if dense retrieval causes a production
problem after being enabled, turning the feature flag off must fully
and immediately restore B1-only behavior, with no partial/mixed state
possible in between.

## 9. What this design does not do

- Does not implement any component above.
- Does not choose a final vector-index storage format (Section 2) —
  explicitly deferred to implementation time.
- Does not decide whether v2 becomes the new default over B1 — a
  separate product decision.
- Does not propose solving Priority 1's abstention gap.
- Does not reopen any Priority 1-3 lever.
- Does not touch the now-CONSUMED v2 held-out set.
- Does not merge, tag, or push anything.

## 10. STOP

This design is complete as requested. No implementation follows until
separately authorized. Per instruction, a real git-state report
follows this document as a separate, final step.
