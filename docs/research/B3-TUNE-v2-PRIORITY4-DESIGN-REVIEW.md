# Dense Retrieval v2 — Production Integration Design: Structured Review

**Date:** 2026-11-15. **Reviews:**
`docs/research/B3-TUNE-v2-PRIORITY4-PRODUCTION-INTEGRATION-DESIGN.md`
(as revised in this same review cycle to resolve the three gaps found
below). **Status: review complete, verdict rendered below, merge NOT
yet executed.**

---

## 1. Checklist review

### Index lifecycle

Embedding-generation timing (lazy-on-read vs. background worker),
incremental update (content-hash check), delete handling (hook into
B1's existing removal path), stale-vector invalidation (content-hash
AND model-identity-hash, two independent triggers), model/index
versioning (frozen model hashes recorded in index metadata, compared
at startup), and rebuild rules (full vs. incremental, explicit
triggers) are all addressed with concrete candidate mechanisms and
explicit trade-offs named, not asserted away. **Assessment: sound.**
The lazy-vs-background trade-off is correctly left open rather than
prematurely decided — this is a genuine implementation-time call that
depends on usage patterns not yet known.

### Atomic storage / recovery

Write-to-temp-then-atomic-rename, explicitly tied to the project's own
existing journal/deletion-log crash-safety discipline rather than
inventing a separate pattern. **Assessment: sound in principle**, but
the design correctly stops short of specifying exact mechanics (which
existing `storage` primitive it would reuse, if any) — appropriately
deferred, since this requires reading `storage`'s actual atomic-write
code, not assumed from its existence.

### Invalidation and rebuild

Covered under "Index lifecycle" above — the two-trigger structure
(content change vs. model-identity change) is the correct
decomposition; a single "needs rebuild" boolean would have been a
regression from this precision.

### Model/index versioning

Explicit requirement to record HF repo, pinned revision, ONNX artifact
hash, and tokenizer hash in index metadata, compared against the
running model's own hashes at startup. This directly reuses
`frozen-config-v2.json`'s own evidence-provenance fields rather than
inventing a new hash set — **correct**, since those are already the
authoritative identity record for this exact model.

### Privacy parity

Explicitly requires production to embed only `text_searchable`, never
raw body, mirroring B1's own redaction — correctly framed as "the same
rule, carried forward" rather than a new rule invented for dense
retrieval. **Originally weak on verification** (see Gap 2 below,
resolved in the revision): the original Section 7 did not specify
*how* this parity would actually be tested against the real production
code path, only asserted the requirement.

### Sensitive-note handling

Specific, concrete requirement: `Sensitive`-note embedding input must
reflect B1's own existing withholding of `source`, never the
unredacted body — directly traceable to the evaluation's own passed
privacy test (Section 5.4 of the closure), not a new, untested
assumption. **Sound**, with the caveat (now resolved in the revision)
that the evaluation's privacy test needs re-running against the real
production integration point, not assumed to transfer by analogy.

### ONNX Runtime lifecycle

One `Environment` per process, initialized once — correctly mirrors
the evaluation harness's own pattern. Concurrency/threading is
correctly flagged as needing verification against the actual pinned
`ort` version's thread-safety guarantees, not assumed. **Sound, with
an appropriately-flagged open question** rather than a false certainty.

### Offline packaging

Covered thoroughly in the original Section 6 (model acquisition,
hash pinning, offline-after-fetch, license attribution, Windows
support) — but **originally silent on the production-dependency-
promotion question itself** (Gap 3, resolved in the revision via the
new Section 6a). The original design discussed packaging the *model*
correctly but did not address that `ort`/`tokenizers`/`ndarray`
themselves are not yet production dependencies at all — a structural
fact confirmed directly against `crates/api/Cargo.toml` during this
review, not previously verified in the design itself.

### Fallback B1

Explicit, three-sub-case breakdown (index not yet built; model-hash
mismatch; ONNX Runtime init failure) all resolving to the same
B1-fallback behavior, with a note that the init-failure case
specifically deserves a loud startup warning since it likely indicates
a packaging problem rather than a transient data issue. **Sound and
appropriately granular** — collapsing these three into one generic
"falls back to B1" statement would have hidden the real difference in
operator-actionable signal between them.

### API integration

**This was the weakest section before revision (Gap 1).** The original
text proposed "a `retrieval_mode` or similar flag" without
committing to an exact type, name, or interaction contract with the
real `Index::search(&self, query: &str, filters: SearchFilters, k:
usize)` signature (`crates/index/src/search.rs:285-292`, verified
directly during this review — no retrieval-mode parameter exists
today). This is now resolved in the revision: a frozen `RetrievalMode`
enum (`Lexical` default, `DenseV2` opt-in), with its name, type,
default, fallback semantics, and backward-compatibility impact all
specified, plus an explicit rejected alternative (a separate
`search_with_mode` method) with reasoning for the rejection. **Now
sound.**

### Deterministic behavior

Dense search's own tie-break (cosine-similarity-desc, then note-id-
lexicographic-asc) is explicitly required to mirror B1's own
existing, audited tie-break convention — correct, avoids introducing a
second, inconsistent tie-break rule into the same codebase. The
revision adds an explicit rule for the fallback decision's own
determinism (transient failures fail closed as errors, never flip
silently between modes on retry) — this was implicit before, now
explicit.

### Crash/recovery behavior

Explicitly tied to the project's own existing deletion-log crash-
safety test discipline (`docs/Spec_mvp.md`) as the pattern to mirror,
not reinvent — **sound**, consistent with every other "reuse existing
project discipline, don't invent a parallel one" instance elsewhere in
this design (atomic storage, delete-handling hook, tie-break
convention).

### Test strategy

**This was the second weakest section before revision (Gap 2).** The
original "parity test" language implied a single comparison without
specifying what it would actually prove or what layer of the system it
would exercise. The revision splits this explicitly into two layers:
synthetic-harness-vs-synthetic-harness parity (cheap, already
partially true, but proves only that the embedding/scoring *logic* is
reproduced) and a **required** real-vault, real-`Index::search`
integration test (the only layer that actually validates "production
implements what was evaluated"), plus a specific requirement to re-run
the `Sensitive`-note privacy test against the real production
integration point, not just the evaluation harness's own embedding
call in isolation. **Now sound.** Golden-embedding tests, index-
invalidation tests, crash/recovery tests, and the B1-regression test
were all already well-specified in the original draft and are
unchanged by this revision.

### Rollback

Feature flag (config switch, off by default) plus full-restore-to-B1
on disable — consistent with every prior lever's own additive-opt-in
discipline in this project. **Sound.** The revision's new
`dense-retrieval` Cargo feature flag (Section 6a) is a *build-time*
complement to this *runtime* rollback mechanism — the two are
distinct and both necessary (a Cargo feature controls whether the
dependency is even linked; the runtime config switch controls whether
an already-linked capability is used), and the revision makes this
distinction explicit rather than conflating them.

### What's production-worthy vs. evaluation/research-only

This question is answered implicitly throughout the design but is
worth stating explicitly here, as its own checklist item, since it is
the crux of what a merge decision actually turns on:

**Production-worthy (should eventually be ported into
`crates/index`/`crates/api/src`, in some form, once B4 actually
implements it):**
- The frozen model identity and representation semantics
  (`frozen-config-v2.json`'s own content) — these are facts about the
  accepted configuration, not evaluation-only artifacts.
- The `priority4_common/embedding.rs` module's *logic* (tokenization,
  pooling, normalization, cosine similarity) — not the file itself
  verbatim, but its representation-semantics decisions, which must be
  reproduced exactly in production per the parity requirement.
- The `Sensitive`-note privacy test's *logic* (structural + differential
  check) — ported into a permanent production test suite, per Section
  7's explicit requirement.
- The frozen quality/cost gates themselves, as documentation of what
  was validated — not as runtime-enforced thresholds (nothing in this
  design proposes runtime-checking Recall@10 in production; that
  would be a different, unrelated feature).

**Must remain evaluation/research-only, never promoted into
production as-is:**
- `VaultFixture` itself (the synthetic, throwaway, freshly-committed
  temp-vault pattern) — explicitly called out in the revision's Gap 2
  resolution as something the real production integration test must
  NOT rely on exclusively; production code integrates with the real
  `storage::Vault`, never a synthetic fixture.
- The example-binary harnesses themselves
  (`b3_tune_v2_priority4_dense_dev`/`..._calibration`/`..._v2_heldout`
  and their `cost`/`hybrid` siblings) — these are Priority 4's
  evaluation instruments, not production code to be lifted wholesale;
  production needs its own implementation against the real
  `Index`/`api` surface, informed by but not copy-pasted from these.
- The `.priority4-model-cache/` path convention — an evaluation-only
  convention; production needs its own documented path (Section 6a).
- Every REJECTED or NOT-EVALUATED/NO-OP lever from Priorities 1-3
  (abstention threshold, BM25 tuning, punctuation normalization,
  alias table, identifier boost, status/recency/trust tie-breaks) and
  the REJECTED hybrid-RRF fusion strategy — none of these are
  candidates for production integration; only `v2 MiniLM dense-only`
  itself is.
- The now-CONSUMED v2 held-out set and its specific label content —
  evaluation evidence, not a production artifact of any kind.
- The two fabricated subagent reports from Priority 4's calibration/
  held-out execution — already formally discarded per the closure
  document; restated here only to be unambiguous that they have zero
  bearing on what gets merged.

## 2. Verdict

**Strategic design: APPROVED.** The architecture (index lifecycle,
storage discipline, privacy/security model, runtime model, rollback
mechanism) is sound, correctly reuses existing project discipline
rather than inventing parallel mechanisms, and correctly defers
genuine implementation-time decisions (vector-index format choice,
lazy-vs-background embedding generation) rather than deciding them
speculatively.

**Implementation readiness: APPROVE WITH CHANGES.** Three gaps (API
contract specificity, parity-test layer specificity, production-
dependency-migration planning) were real and are now resolved in the
revised design document — not by weakening the review's standard, but
by freezing exactly the contracts the original draft left generic.
None of the three gaps invalidated the underlying architecture; all
three were implementation-readiness gaps, not design-soundness gaps.

## 3. Blocking gaps — status after this revision

| Gap | Status |
|---|---|
| 1 — Exact search API contract | **Resolved.** `RetrievalMode` enum frozen in the revised design's Section 3, with name/type/default/semantics/compat/fallback-determinism all specified. |
| 2 — Parity-test semantics | **Resolved.** Two-layer parity requirement frozen in the revised design's Section 7, including the required real-vault integration test and the production-path privacy re-test. |
| 3 — Production dependency migration | **Resolved as a planning document, not yet executed.** New Section 6a names every question (feature flag, platform support, Windows build, linking, packaging, discovery, offline behavior, release/CI impact, binary size, licensing, failure mode) explicitly. Several of these are correctly left as "B4 task-level decision" rather than resolved here, since resolving them requires information (actual CI config, actual release process) this design review does not have access to — **not a gap in the review, a correctly-scoped deferral.** |

**No gap remains blocking at the design-document level.** The
remaining open items inside Section 6a (vendored-vs-fetched ONNX
Runtime, feature-flag naming convention, CI impact) are B4 **task**
inputs, not design defects — B4's own task breakdown (Section 5 below)
assigns each to a specific task with its own acceptance criterion.

## 4. Exact list — what should merge into `main`

**Recommendation: merge the following, and only the following, from
`b3-retrieval-eval` into `main`, once the user authorizes the merge
itself** (not yet done — this is a list for that future action, not an
action taken now):

- All `docs/research/B3-TUNE-v2-*.md` closure and design documents
  (Priority 1-4 closures, the ranking-pipeline audit, the hybrid
  design, the model-shortlist audit, the calibration/held-out results,
  this production-integration design and its revision, this review
  document itself).
- All `docs/superpowers/plans/2026-11-14-b3-tune-v2-*.md` and
  `docs/superpowers/specs/2026-11-14-b3-tune-v2-*.md` plan/spec
  documents.
- All `eval/results/B3-TUNE-v2/*.json` and `*.md` result artifacts
  (DEV/calibration/held-out reports for all four priorities,
  `frozen-config-v2.json`).
- The Priority 1-4 evaluation example binaries under
  `crates/api/examples/b3_tune_v2_*` and
  `crates/api/examples/priority4_common/` — these are legitimate,
  permanent evaluation-harness code (reproducibility of the
  DEV/calibration/held-out numbers depends on them existing), not
  throwaway scratch work.
- The `ort`/`tokenizers`/`ndarray` **dev-dependency** entries in
  `crates/api/Cargo.toml` (as dev-dependencies only — this is their
  current, already-correct scope; nothing about this list promotes
  them to production dependencies).
- `eval/rag/v2-heldout/manifest.json` and `eval/rag/v2-heldout/labels.jsonl`
  (the held-out set itself, now CONSUMED but still a legitimate,
  permanent evaluation artifact for reproducibility/audit purposes —
  not deleted, not hidden, simply never reusable as independent
  evidence again).
- Any additive, default-off `index`/`api` production code from
  Priority 2/3's accepted or evaluated levers that already exists on
  this branch (e.g. the `with_bm25_params`, `with_identifier_boost`,
  `with_status_tiebreak` builders) — these are real production code
  additions, already reviewed, already additive/opt-in, already
  default-off; their presence on `main` causes no behavior change for
  any existing caller regardless of whether their own lever was
  accepted or rejected.

## 5. Exact list — what must NOT enter the production path

(Distinct from "merge into `main`" above — these items may legitimately
exist on `main` as evaluation/research artifacts per Section 4, but
must never be treated as, or wired into, `index`/`api`'s own
*production* search behavior.)

- `VaultFixture` and any other evaluation-harness synthetic-vault
  pattern — never a substitute for the real `storage::Vault` in
  production code paths.
- The evaluation example binaries themselves — not to be imported,
  `include!`d, or otherwise referenced from `crates/index/src/` or
  `crates/api/src/`; production code must be its own implementation,
  informed by but not dependent on the evaluation harnesses.
- Any REJECTED or NOT-EVALUATED/NO-OP lever's logic (Priority 1's
  abstention threshold, Priority 2's BM25 `k1`/`b` retuning and
  punctuation normalization, Priority 3's recency/trust tie-breaks —
  the latter two were never even implemented) — none of these should
  be resurrected into production without a fresh, separately-
  authorized evaluation cycle.
- Hybrid-RRF fusion logic — rejected at calibration; must not be wired
  into `RetrievalMode` as a third variant without new evidence from a
  fresh evaluation cycle.
- `.priority4-model-cache/`'s specific path/layout convention — an
  evaluation-only convention, not to be hardcoded into production
  packaging without its own, separate decision (Section 6a).
- `ort`/`tokenizers`/`ndarray` as **production** dependencies — until
  B4's Section 6a questions are actually answered and a `Cargo.toml`
  change is explicitly authorized; their current dev-dependency scope
  must not be silently widened as a side effect of some other change.
- Any numeric threshold proposed in `frozen-config-v2.json` as a
  **runtime-enforced** gate — those numbers describe what was
  validated; nothing in this design proposes checking Recall@10 live
  in production, and nothing should, without its own separate
  justification.

## 6. Proposed plan — B4: Dense Retrieval Production Integration

This is a **proposed task breakdown**, not an authorization to begin
any of it. Each task names its dependency on prior tasks and a
concrete acceptance criterion. No task in this list may begin until
the user separately authorizes B4 itself.

### B4.0 — Freeze remaining Section 6a questions (prerequisite, no code)

**Depends on:** nothing (can start immediately once B4 is authorized).
**Acceptance criterion:** a short written decision record (mirroring
this project's own `docs/superpowers/specs/` convention) answering, at
minimum: Cargo feature name and default state; vendored-vs-fetched
ONNX Runtime strategy; dynamic-vs-static linking; artifact/model
discovery path convention for production (distinct from
`.priority4-model-cache/`). Everything else in Section 6a (release
pipeline, CI, binary size, licensing) may be answered here too if
information is available, or explicitly deferred to a named later
task if not.

### B4.1 — Implement the `RetrievalMode` enum and `Index::search` signature change

**Depends on:** B4.0 (needs no input from it directly, but should not
start before the team has committed to *some* dependency strategy, to
avoid designing the enum around an assumption Section 6a later
invalidates). **Acceptance criterion:** `RetrievalMode::Lexical` is
provably byte-identical to today's `Index::search` behavior (the
B1-regression test from Section 7) for every existing call site;
`RetrievalMode::DenseV2` compiles and returns a structurally valid
(even if not yet dense-powered) response — this task may legitimately
land with `DenseV2` behaving identically to `Lexical` under the hood
temporarily, as a pure API-surface change, with the real dense pipeline
wired in by a later task.

### B4.2 — Production dependency promotion

**Depends on:** B4.0's decisions. **Acceptance criterion:** `ort`/
`tokenizers`/`ndarray` (or whichever subset B4.0 decided) are
production dependencies of `crates/api` (or `crates/index`, per
B4.0's own architectural call — not decided here), gated behind the
B4.0-named Cargo feature, default off; a build with the feature
disabled has zero new dependency surface versus today; a build with
the feature enabled compiles and links successfully on Windows (the
platform this entire evaluation cycle ran on) at minimum.

### B4.3 — Index lifecycle: embedding generation, incremental update, delete handling

**Depends on:** B4.1 (needs `RetrievalMode` to exist), B4.2 (needs the
embedding runtime to exist as a real dependency). **Acceptance
criterion:** a note's vector is generated/updated/removed correctly
across all cases named in the design's Section 1 (new note, content
change, metadata-only change — no re-embed, delete, model-identity
change — full rebuild), each covered by the index-invalidation tests
from Section 7.

### B4.4 — Storage: vector index format, atomic write, corruption detection

**Depends on:** B4.3 (needs something to store). **Acceptance
criterion:** the chosen format (per the design's Section 2, still
deferred to this task) survives the crash/recovery test discipline
from Section 7 (crash mid-write recovers to old-or-new-complete-index,
never partial); a deliberately corrupted index file is detected and
triggers rebuild, not silent misbehavior.

### B4.5 — Dense scoring and `DenseV2` wiring into `Index::search`

**Depends on:** B4.1, B4.3, B4.4. **Acceptance criterion:** the
two-layer parity test from the design's revised Section 7 passes —
both the synthetic-harness-vs-synthetic-harness layer and, critically,
the real-vault `Index::search(..., RetrievalMode::DenseV2, ...)`
integration test, producing the same ranked note-id list as the
frozen evaluation harness for an identical corpus/query.

### B4.6 — Privacy/security production test suite

**Depends on:** B4.5 (needs the real integration point to test
against). **Acceptance criterion:** the `Sensitive`-note structural +
differential privacy test passes against the real production
`Index::search(..., RetrievalMode::DenseV2, ...)` path specifically
(not only the evaluation harness), per the design's Gap 2 resolution.

### B4.7 — Fallback, rollback, and feature-flag wiring end-to-end

**Depends on:** B4.1-B4.6 (needs all failure-mode-producing
components to exist to test fallback against them). **Acceptance
criterion:** each of the three unavailability sub-cases (index not
built, model-hash mismatch, ONNX Runtime init failure) correctly
degrades to `Lexical` behavior with an appropriately-leveled log
message; disabling the config switch/feature fully restores
`Lexical`-only behavior with no partial/mixed state observable.

### B4.8 — Packaging and offline setup

**Depends on:** B4.2 (needs the dependency strategy settled), B4.0's
model-acquisition-path decision. **Acceptance criterion:** a
documented, explicit setup step acquires and hash-verifies model +
tokenizer + (if not vendored) ONNX Runtime artifacts in one pass;
subsequent operation is confirmed fully offline (no network call
observed during normal search operation); Windows is explicitly
covered by this verification, not inferred from the evaluation
harness's own Windows success.

### Task-order summary

```
B4.0 (freeze decisions, no code)
  |
  +--> B4.1 (RetrievalMode + search signature) --+
  |                                               |
  +--> B4.2 (dependency promotion) ---------------+--> B4.3 (index lifecycle)
                                                               |
                                                               v
                                                        B4.4 (storage)
                                                               |
                                                               v
                                                        B4.5 (dense scoring + wiring)
                                                          |          |
                                                          v          v
                                                   B4.6 (privacy)  B4.7 (fallback/rollback)
                                                                      |
                                                                      v
                                                               B4.8 (packaging)
```

## 7. Updated merge recommendation

**Do not merge yet.** Per the user's own explicit instruction, this
review stops here. Once the user separately authorizes:
(a) the merge of `b3-retrieval-eval` into `main` (scoped exactly to
Section 4's list, excluding Section 5's list), and/or
(b) the start of B4 itself (Section 6's task breakdown, starting at
B4.0),
those become their own, separately-executed actions — neither is
performed as part of this review.

## 8. STOP

Review complete. No merge, no tag, no production code, no
implementation of any B4 task. Next step is the user's decision on
whether/when to authorize the merge and/or B4.
