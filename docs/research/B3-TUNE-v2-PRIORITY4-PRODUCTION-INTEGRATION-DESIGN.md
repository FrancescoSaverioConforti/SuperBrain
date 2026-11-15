# Dense Retrieval v2 — Production Integration Design

**Status: DESIGN ONLY. No implementation in this document or as a
result of it. No code has been written against `crates/index` or
`crates/api/src` for this design.**

**Date:** 2026-11-15 (revised same day, post-review). **Follows:**
`B3-TUNE-v2-CLOSURE-SUMMARY.md` — `all-MiniLM-L6-v2 dense-only` is the
accepted v2 configuration, backed by the frozen
`eval/results/B3-TUNE-v2/frozen-config-v2.json`. This document is the
first step toward eventually bringing that *evaluation-harness*
configuration into the *production* `index`/`api` crates — it is a
design to review, not a plan to execute yet.

**Revision note:** this design was reviewed and returned **APPROVE
WITH CHANGES** — the strategic architecture (index lifecycle, storage,
privacy, runtime, rollback) was approved as sound, but three
implementation-readiness gaps were identified and are now resolved
below, each frozen as an explicit contract rather than left generic:
(1) Section 3's search API contract (exact `RetrievalMode` enum, not a
generic "retrieval mode" placeholder); (2) Section 7's parity-test
semantics (two explicit layers — synthetic-harness parity, and a
required real-vault production-path integration test — not a single
ambiguous "parity test"); (3) a new Section 6a covering the production
promotion of `ort`/`tokenizers`/`ndarray` from today's dev-only/
example-only dependencies to real production dependencies, with its
own build/packaging/CI questions named explicitly. See
`docs/research/B3-TUNE-v2-PRIORITY4-DESIGN-REVIEW.md` for the full
review and the resulting B4 task breakdown.

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

**Gap 1 resolution — exact search API contract, frozen now, not left
generic.** `Index::search(&self, query: &str, filters: SearchFilters,
k: usize) -> Result<SearchResponse, SearchError>` is the real, current
signature (`crates/index/src/search.rs:285-292`) — no retrieval-mode
parameter exists today. `Index` already has precedent for additive,
default-off ablation fields set via `with_*` builders
(`identifier_boost: bool`, `status_tiebreak: bool`,
`crates/index/src/search.rs:192-209`), but a boolean is the wrong
shape here: dense retrieval is not a binary ablation toggle, it is a
genuine mode selection with its own fallback semantics (Section
"Behavior when model/index is unavailable" below), so a boolean would
force an awkward third implicit state ("requested but unavailable")
onto a two-valued type. Per explicit instruction, this is frozen as an
enum, not a boolean flag:

```rust
/// Frozen contract (B4 Task 1) -- not yet implemented.
pub enum RetrievalMode {
    /// Default. Identical behavior to every existing call site today
    /// -- B1 trigram+BM25 only. Zero behavior change for any caller
    /// that does not explicitly opt in.
    Lexical,
    /// Opt-in. Dense-only (`v2 MiniLM dense-only`, per
    /// `frozen-config-v2.json`). Falls back to `Lexical` behavior,
    /// with a logged warning, if the dense model/index is
    /// unavailable (Section "Behavior when model/index is
    /// unavailable") -- never a hard error for this reason alone.
    DenseV2,
}

impl Default for RetrievalMode {
    fn default() -> Self { RetrievalMode::Lexical }
}
```

- **Parameter name:** `retrieval_mode`.
- **Rust type:** `RetrievalMode` enum (above), not `bool`, not
  `Option<T>` — an explicit two-variant enum names the current and
  only accepted modes without implying a boolean's natural "on/off"
  reading, and leaves room to add a future `Hybrid` variant later
  without a breaking signature change (adding an enum variant is
  additive at the type level in the same spirit as every other lever's
  own additive discipline, though it does require every `match` on
  `RetrievalMode` to be updated — a compile-time-enforced reminder,
  not a silent gap).
- **Default:** `RetrievalMode::Lexical` (via `Default`), identical to
  every existing call site's behavior today.
- **Semantic contract:** `RetrievalMode::Lexical` is byte-identical to
  today's `Index::search` behavior — this is the regression contract
  Section 7's "Regression against B1 behavior" test enforces.
  `RetrievalMode::DenseV2` replaces candidate generation and scoring
  with the frozen v2 dense pipeline (Section 1), but reuses B1's
  existing filtering/privacy/redaction stages unchanged (Section 4) —
  dense retrieval is a new candidate-generation-and-scoring source,
  not a parallel filtering pipeline.
- **Interaction with existing `query`/`filters`/`k`:** unchanged in
  shape and meaning across both modes — `filters` (project/type/
  status/trust/time) and `k` apply identically regardless of
  `retrieval_mode`; only how candidates are generated and scored
  differs.
- **Behavior when dense runtime/index is unavailable:** the three
  sub-cases below (unchanged from this design's original Section 3)
  all resolve to the same outcome — silent-to-the-caller-as-an-error,
  but explicitly logged, fallback to `Lexical` behavior for that
  request. `DenseV2` never returns `SearchError` merely because the
  dense path is unavailable; it degrades to `Lexical`'s own existing
  error semantics instead.
- **Backward compatibility:** every existing call site of
  `Index::search`/`Index::search_profiled` must add the new parameter
  via `Default::default()` (or an explicit `RetrievalMode::Lexical`)
  to compile unchanged in meaning — this is a source-breaking,
  behavior-preserving signature change, not a behavior-breaking one.
  (Alternative considered and rejected: a separate
  `Index::search_with_mode` method, leaving `Index::search` untouched.
  Rejected because it would let a caller silently keep calling the old
  method forever and never see dense retrieval even exist as an
  option, and because every existing Priority 2/3 lever instead
  extended the *same* method's behavior via `Index` construction-time
  builders, not a parallel method — this keeps that precedent, at the
  cost of a one-time signature update at every call site.)
- **Deterministic fallback behavior:** the fallback decision itself
  (available vs. unavailable) must be deterministic for a given
  process state — not flip between `Lexical` and `DenseV2` behavior
  on retries of the identical request under unchanged index/model
  state. A transient failure during fallback evaluation itself (e.g. a
  filesystem error reading the index) is a `SearchError`, not a silent
  `Lexical` fallback — only "dense path confirmed structurally
  unavailable" (not yet built, hash-mismatched, runtime failed to
  init) triggers the fallback; an ambiguous/transient state fails
  closed as an error, consistent with B1's own existing storage-error
  handling discipline (ranking-pipeline audit Section 3: "Any other
  storage error → the whole search fails closed... never silently
  degrading").

This contract is frozen as of this revision — B4's first task is to
implement exactly this signature, not to re-derive or renegotiate its
shape mid-implementation.

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

## 6a. Production dependency migration — Gap 3 resolution

**Current real state, verified against the actual `Cargo.toml` files
(not assumed):** `ort` (`2.0.0-rc.15`), `tokenizers` (`0.23.2`), and
`ndarray` are currently `[dev-dependencies]` of the `api` crate only
(`crates/api/Cargo.toml`), explicitly scoped to the Priority 4
evaluation example binaries — never a dependency of `crates/index` at
all, and never a `[dependencies]` (production) entry of `api` either.
Production code today has zero exposure to any embedding/ONNX crate.
**Promoting `RetrievalMode::DenseV2` from design to shipped code
requires moving some subset of these from dev-only/example-only to a
real production dependency — this is not a detail to discover
mid-implementation; it must be planned explicitly, since dependency
promotion changes the production binary's build, size, and platform
support surface.**

**Feature flags.** Candidate approach: gate the new production
dependency behind a Cargo feature (e.g. `dense-retrieval`), default
**off**, so a build that doesn't want the ONNX Runtime dependency at
all (smaller binary, fewer platform-support questions) can opt out at
compile time, not just at runtime via `RetrievalMode::Lexical`. This
is a stronger guarantee than the runtime default: a user/build that
never enables the feature never links the dependency at all. Needs a
decision on whether this project's existing build/release process
already uses Cargo features for anything comparable, or whether this
would be the first.

**Target/platform support.** `ort`'s `download-binaries` feature (used
by the evaluation harness) fetches a pre-built ONNX Runtime shared
library for the host platform at build/first-run time — this needs an
explicit decision for production: continue relying on that mechanism
(simpler, but reintroduces a one-time network fetch dependency at
build or first-run time, which needs to be reconciled with Section
6's "deliberate, user-visible setup step" requirement rather than a
silent `cargo build`-time fetch) or vendor/bundle the ONNX Runtime
binary per supported platform directly (more packaging work, zero
build-time network dependency). **Not decided here — flagged as the
single packaging question most likely to have platform-specific
surprises.**

**Windows build.** This entire evaluation cycle built and ran
successfully on Windows via `ort`'s `download-binaries` feature
(confirmed: the DEV/calibration/held-out harnesses all compiled and
ran on this exact machine) — but that confirms only that the
*evaluation* build path works on Windows, not that whatever production
packaging strategy is chosen for Section "Target/platform support"
above will. This needs its own explicit verification once a packaging
strategy is chosen, not an inference from the evaluation harness alone.

**Dynamic/static runtime requirements.** The ONNX Runtime shared
library is a native (non-Rust) dependency — whether it is dynamically
linked at runtime (requiring the `.dll`/`.so` to be present and
discoverable alongside the production binary) or statically linked
(larger binary, no separate runtime artifact to lose/misplace) is an
`ort` build-configuration choice that needs to be made explicitly for
production, not left at whatever the evaluation harness's default
happened to be.

**ONNX Runtime packaging.** Whatever choice is made in "Target/
platform support" above directly determines this: either the ONNX
Runtime binary ships as part of the production distribution (vendored/
bundled) or production packaging documents and automates its
acquisition as part of the same one-time setup step that fetches the
model weights (Section 6) — these two artifacts (model weights, ONNX
Runtime binary) should likely be acquired and verified together, in
one setup step, not as two separate, independently-timed download
events a user could partially complete.

**Artifact/model discovery.** Production code needs a defined,
documented path convention for where the model/tokenizer/ONNX-Runtime
artifacts live on disk once acquired (the evaluation harness's own
`.priority4-model-cache/` convention is a starting point, not
necessarily the final production path) — and must fail with a clear,
actionable error (not a cryptic ONNX Runtime internal error) if that
path is missing or incomplete at startup.

**Offline installation.** Unchanged in principle from Section 6's
existing "Offline setup" requirement — restated here specifically
because dependency promotion is the mechanism that makes "offline
after first fetch" either true or false in practice: if `ort`'s
`download-binaries` feature is kept as-is for production, "offline
after first fetch" depends on that feature's own fetch happening
during the same explicit setup step as model acquisition, not
separately and invisibly during a later `cargo build`/binary update.

**Release pipeline impact.** If this project has any existing CI/
release automation (not confirmed by this design — needs checking),
promoting these dependencies to production likely changes build time
(native dependency compilation/linking) and may require
platform-specific build matrix entries that don't exist today. Flagged
as a question for whoever owns the release pipeline, not answered
here.

**Binary size.** `ort`/`tokenizers`/`ndarray` plus the ONNX Runtime
native library (if bundled rather than fetched) will measurably
increase the production binary/distribution size beyond today's
footprint (`index`/`api` currently have zero such dependencies) — the
actual delta should be measured once a packaging strategy (vendored
vs. fetched) is chosen, not assumed negligible.

**Licensing/attribution.** Unchanged from Section 6's existing
requirement for the model weights (Apache-2.0) — restated here because
`ort`, `tokenizers`, and `ndarray` each carry their own license terms
as Rust crate dependencies, independent of the model's own license,
and must be checked/attributed per the project's existing dependency-
licensing practice (if one exists) before promotion to production,
not assumed clear by analogy to the model weights' own license.

**CI implications.** If CI currently builds/tests `api`'s examples
(where these are dev-dependencies today) differently from how it would
build/test `api`'s own production library surface (where they would
become production dependencies), this promotion likely changes what
CI exercises and how long it takes — not quantified here, needs
checking against the actual CI configuration once one is identified.

**Failure mode when runtime/model artifact is missing.** This is the
same case already covered functionally in Section 3's "Behavior when
model/index is unavailable" (ONNX Runtime fails to initialize) — restated
here as a packaging-specific instance: a production build with the
`dense-retrieval` feature enabled, but missing its acquired model/
runtime artifacts (e.g. the one-time setup step was never run), must
fail into the same `RetrievalMode::Lexical` fallback with a loud,
actionable startup warning — never a hard crash, and never a silent,
unexplained absence of dense retrieval that looks like a bug rather
than an unfinished setup step.

**No dependency is promoted "accidentally" during implementation** —
per explicit instruction, this entire section's questions must be
answered (even if some answers remain "defer to B4 Task N" rather than
resolved here) before any `Cargo.toml` change moves `ort`/
`tokenizers`/`ndarray` out of `[dev-dependencies]`.

## 7. Testing

**Parity against frozen evaluation implementation — Gap 2 resolution.**
Production dense retrieval's output (given the same corpus, same
query, same frozen model) must match the evaluation harness's own
output exactly — same embeddings, same similarity scores, same top-k
ordering. This must be demonstrated at two distinct layers, not one,
because a parity test that only compares two synthetic harnesses
against each other proves nothing about the real production path:

1. **Evaluation-harness-vs-evaluation-harness parity (weakest, but
   cheap and already partially true).** The evaluation harnesses
   (`b3_tune_v2_priority4_dense_dev`,
   `..._dense_calibration`, `..._dense_v2_heldout`) already share the
   same `priority4_common/embedding.rs` module and the same
   `VaultFixture` pattern — a synthetic, in-memory `git2`-backed vault,
   freshly committed from `eval/rag/corpus/notes/*.md` on every run,
   never the real on-disk production vault. This is the harness every
   Priority 4 quality number (DEV/calibration/held-out) was measured
   against. **What this fixture replaces:** the real `storage::Vault`'s
   persistent on-disk journal, its crash-recovery state machine, and
   its PII store's real key material — `VaultFixture` uses a
   throwaway temp directory and a freshly generated key every run,
   exactly so no evaluation run can accidentally touch real project
   data. **What this fixture does NOT cover:** anything about the real
   vault's persistence, concurrent access, crash recovery, or its
   actual historical note content/scale (`VaultFixture` always builds
   from the same 53-note `eval/rag/corpus/notes/` set, never the real,
   larger, continuously-growing project vault) — a parity test that
   only exercises this fixture proves the embedding/scoring *logic* is
   faithfully reproduced, nothing about how that logic behaves against
   the real vault's actual scale, content, or storage behavior.

2. **Production-path integration test against the real vault
   (required, not optional).** At least one integration test must run
   the real `Index::search(..., RetrievalMode::DenseV2, ...)` call —
   the actual production code path, not a reimplementation of it in an
   example binary — against a real `storage::Vault` instance (using
   the project's own existing vault-construction test helpers, the
   same ones B1's own index tests already use, not a new fixture
   invented for this purpose) and assert that its results match what
   the frozen evaluation harness produces for the identical corpus and
   query. This is the test that actually validates "production
   implements what was evaluated" — layer 1 alone cannot make that
   claim, since it never touches `Index::search` itself.

**Invariants this production-path test must preserve, explicitly:**
- Candidate generation/scoring for `RetrievalMode::DenseV2` must
  produce the same ranked note-id list, for a fixed query against a
  fixed corpus snapshot, as the frozen evaluation harness — this is
  the representation-semantics parity itself (Section 4b of the
  hybrid design doc: exact text embedded, prefix, truncation, pooling,
  normalization, metric — all must be implemented identically in
  production, not approximately).
- **Privacy/searchable-surface parity between the evaluation harness
  and production** is itself a thing to test, not assume: the
  evaluation harness's `Sensitive`-note privacy test (Section 5.4 of
  the closure) must be re-run against the real production
  `Index::search(..., RetrievalMode::DenseV2, ...)` path specifically
  — not only against the evaluation harness's own standalone
  embedding call — to confirm the real integration point (not just the
  embedding module in isolation) never embeds a `Sensitive` note's
  redacted-away content. A passing evaluation-harness privacy test
  does not, by itself, prove the production integration point wires
  redaction through correctly; this is exactly the kind of gap a
  parity test at layer 1 alone would miss.
- B1's own existing behavior (`RetrievalMode::Lexical`, filters,
  `k`) must remain provably unaffected by `DenseV2`'s mere existence —
  this is Section "Regression against B1 behavior" below, not
  conflated with parity testing itself.

**This is the single most important test category** in this entire
design, since every Priority 4 quality number (DEV/calibration/
held-out) is only trustworthy for production if production actually
implements what was evaluated — and that claim requires layer 2's
real-vault, real-API test, not layer 1's synthetic-harness-vs-
synthetic-harness comparison alone.

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

- Does not implement any component above — including the
  `RetrievalMode` enum itself (Section 3) and the dependency
  promotion (Section 6a): both are frozen *contracts*, not yet
  code.
- Does not choose a final vector-index storage format (Section 2) —
  explicitly deferred to implementation time.
- Does not decide whether v2 becomes the new default over B1 — a
  separate product decision.
- Does not propose solving Priority 1's abstention gap.
- Does not reopen any Priority 1-3 lever.
- Does not touch the now-CONSUMED v2 held-out set.
- Does not merge, tag, or push anything.
- Does not resolve every open question in Section 6a (release
  pipeline impact, CI implications, vendored-vs-fetched ONNX Runtime)
  — those are explicitly named as B4 task-level decisions, not
  resolved speculatively here.

## 10. STOP

This design, revised per the structured review in
`docs/research/B3-TUNE-v2-PRIORITY4-DESIGN-REVIEW.md`, is complete as
requested. No implementation follows until separately authorized. A
real git-state report and the B4 task breakdown follow as separate
documents/steps.
