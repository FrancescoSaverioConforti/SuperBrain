# B3-TUNE-v2 Priority 2 — Lexical Tuning: Design

**Status:** design-only. No code, no tuning, no new runs are
authorized by this document. Implementation requires a separate,
approved plan (per `superpowers:writing-plans`), the same two-gate
process Priority 1 used.

## Why this exists

B3-TUNE-v2's design doc
(`docs/superpowers/specs/2026-11-14-b3-tune-v2-design.md`) names lexical
tuning as Priority 2, to be attempted after Priority 1. Priority 1
(abstention thresholding) was measured and **closed as REJECT**
(`docs/research/B3-TUNE-v2-PRIORITY1-CLOSURE.md`, 2026-11-14): no single
global score-derived threshold on the `ratio` signal cleared the
required guardrails on calibration. Priority 2 starts fresh from the
**official B1 baseline** (`eval/results/B3/frozen-config.json`, commit
`52d42ea`) — not from any Priority 1 artifact, per explicit user
instruction.

**The problem Priority 2 targets**: B1 is pure trigram+BM25 with
default-ish parameters, no stemming, and no synonym handling. Some of
B3's lower-scoring answerable queries (particularly `difficulty: hard`
paraphrases) may be recoverable through better lexical matching alone,
without touching ranking architecture or introducing embeddings — which
stays inside the MVP spec's "deterministic, no embeddings" contract for
B1 (`docs/Spec_mvp.md` Fase B).

## Audit of what already exists (prerequisite, done before this design)

A full read of `crates/index/src/{bm25,trigram,document,search}.rs` and
a grep of `crates/index/src` + `crates/domain/src` for
synonym/alias/stem/expand established the current state exactly, so
this design proposes changes against real gaps, not re-implementations
of things already present:

- **Tokenization**: `bm25::tokenize` — `text.split_whitespace().map(str::to_lowercase)`.
  Whitespace-delimited, case-folded (Unicode-aware), **no punctuation
  stripping** — this is an explicit, documented design choice in the
  current code (a token like `"BM25,"` keeps its trailing comma). This
  same function is imported directly into `search.rs` and used for both
  index-build-time and query-time tokenization — confirmed symmetric by
  construction (shared function, not independently-written parallel
  logic), so there is no index/query tokenization drift to fix.
- **Normalization**: case-folding only. No stemming, no lemmatization,
  no Unicode NFC/NFKD normalization, no diacritic stripping.
- **BM25 parameters**: `K1 = 1.2`, `B = 0.75` — hardcoded module-level
  `const`s in `crates/index/src/bm25.rs`, no runtime configurability at
  all today. Standard Robertson/Sparck-Jones formula, non-negative IDF,
  no smoothing floor (a zero-overlap document scores exactly `0.0`,
  which the existing code comments call load-bearing for
  citation-correctness logic elsewhere).
- **Synonym/alias handling**: absent. Confirmed by grep across the
  whole `index` and `domain` crates.
- **Indexed fields**: no `title`/`tags` field exists in the note schema
  at all (`domain::NoteMetadata` has no such fields). What's indexed is
  structured metadata (id/type/status/trust/project, always) plus body/
  source/relations/temporal strings for `Internal`/`Public` notes only
  (`Sensitive` notes exclude all of those, no fallback) — `document.rs`'s
  `build_text_searchable`.
- **Noted, not a defect**: `TrigramIndex::query_exact`'s substring check
  and `search.rs::query_match`'s own substring check are two independent
  implementations of the same "case-insensitive contains" logic. Not a
  bug today, but worth being aware of if any lexical change touches
  either path, since they could silently diverge.

## Goals

1. Measure whether BM25 parameter tuning (`k1`/`b`), light normalization
   beyond what exists, and a small deterministic synonym/alias table
   each improve Recall@10/nDCG@10 on dev/calibration, as **separate**
   ablations against the official B1 baseline.
2. Measure, as a secondary/diagnostic question (not a gating criterion),
   whether any accepted lexical change also improves the `ratio` signal's
   separation between answerable and abstain-expected queries — i.e.
   whether sharpening the score distribution lexically has a side effect
   on Priority 1's abstention-signal problem, re-opening that question
   with better evidence if so (not re-attempting Priority 1's threshold
   search itself — that remains closed).
3. Keep every change deterministic and auditable — no learned models,
   no embeddings, consistent with the "deterministic, no embeddings"
   contract B1 itself was measured against.

## Non-goals

- Replacing or redesigning the trigram index's role (candidate
  pre-filter only) — out of scope here.
- Any change to `index::search`'s ranking logic itself (reranking is
  Priority 3, separately scoped).
- Any abstention-threshold work — Priority 1 is closed; this phase does
  not reopen it except via the diagnostic measurement in Goal 2.
- Runtime-configurable BM25 parameters as a product feature (e.g. a
  config file or CLI flag for end users) — if `k1`/`b` tuning is
  accepted, the new values simply replace the current hardcoded
  constants; making them end-user-configurable is a separate, unrelated
  decision not implied by this tuning work.
- Any new note schema fields (`title`/`tags`) — the audit found these
  don't exist; adding them would be a `domain`-crate schema change, well
  outside "lexical tuning" scope, and is not proposed here.

## Anti-goals

- A synonym/alias table that grows into an uncontrolled, hard-to-audit
  list — the design below caps it at a small, explicit, hand-reviewed
  set with a stated rationale per entry, not an auto-mined list.
- Accepting a change that measurably improves Recall@10/nDCG@10 on dev
  but is secretly fit to dev's own specific 60 queries — every
  acceptance decision here is checked against calibration before
  proposal, same discipline B3/Priority 1 used.
- Re-opening per-category thresholds or any Priority 1 abstention work
  under a different name.

## Constraints (carried over, unchanged)

- No MCP, no merge/tag/push.
- The new v2 held-out set (`eval/rag/v2-heldout/`) stays completely
  unread. It is not read, run, or referenced by anything in Priority 2.
  It was built once (Task 0, B3-TUNE-v2 Priority 1) and nothing in this
  phase changes that status.
- No new held-out final-test run of any kind.
- Every ablation is measured and reported exactly once per candidate
  configuration — dev first (diagnostic), then calibration (the decision
  split) — same as B3 and Priority 1's own discipline.
- Every ablation is logged per the design doc's experimental logging
  rule: full config diff, commit, corpus/labels hashes, Recall@10 (+CI),
  nDCG@10 (+CI), abstention confusion matrix (even though this phase
  isn't tuning abstention, B1's own non-abstaining behavior must still
  be tracked so a lexical change doesn't accidentally start producing
  `NotPresent` results as a side effect — that would be a correctness
  change, not a tuning win, and needs to be caught), latency/resource
  delta or "not measured," delta vs. the **official** B1 baseline, and
  an explicit KEEP/REJECT decision — same shape Priority 1 used.
- Each of the three candidate approaches (BM25 `k1`/`b`, normalization,
  alias table) is its own **independent** ablation against the official
  B1 baseline — never bundled together in one combined variant without
  first measuring each alone. (Per the user's explicit instruction this
  session: "ogni variante come ablation separata.")
- No per-category thresholds, no abstention-threshold tuning of any
  kind — Priority 1 stays closed.

## Approaches considered (for each of the 3 candidate levers)

### Lever A — BM25 `k1`/`b` tuning

- **Option A1 (grid search over a small, fixed candidate set)**: pick a
  handful of `k1`/`b` combinations around the current defaults (e.g.
  `k1 ∈ {0.9, 1.2, 1.5, 1.8}`, `b ∈ {0.4, 0.6, 0.75, 0.9}` — small,
  interpretable, not an exhaustive sweep) and measure each on dev, carry
  forward only genuinely promising candidates to calibration. **Recommended**:
  bounded, auditable, directly answers "do the current defaults fit this
  corpus's actual document-length distribution," and reuses the same
  "deterministic sweep over a small candidate set, not a continuous
  optimizer" pattern Priority 1's threshold sweep already established
  and the user is now familiar with.
- **Option A2 (derive k1/b analytically from the corpus's length
  distribution)**: compute the corpus's actual document-length
  statistics (mean/variance of `avgdl`'s inputs) and derive a
  theoretically-motivated `b` from them, rather than grid-searching.
  Rejected for this phase: BM25's `k1`/`b` don't have a closed-form
  "optimal" derivation from length statistics alone — it depends on the
  actual relevance judgments too, which is exactly what a small grid
  search over real Recall@10/nDCG@10 measures directly, more reliably
  than a proxy statistic would.
- **Option A3 (continuous optimization, e.g. gradient-free search over
  k1/b to directly maximize calibration Recall@10)**: rejected — this is
  the textbook overfitting trap Priority 1 explicitly avoided ("con n=6
  abstention-positive in calibration il rischio di overfitting sarebbe
  troppo alto" — the same concern applies here: calibration has only 30
  queries total, continuous optimization against it would fit noise).

### Lever B — Normalization

- **Option B1 (light, auditable normalization additions)**: add
  punctuation stripping (currently absent, confirmed by audit) to
  `bm25::tokenize` specifically — e.g. strip a token's leading/trailing
  non-alphanumeric characters before lowercasing, so `"BM25,"` tokenizes
  identically to `"BM25"`. **Recommended**: small, deterministic, targets
  a real documented gap (not a re-implementation), directly testable in
  isolation from Lever A/C.
- **Option B2 (stemming, e.g. Porter or Snowball)**: considered and
  rejected for this phase — stemming is a bigger behavior change
  (affects every token, not just punctuation-adjacent ones), harder to
  audit by hand (stemmer output isn't always intuitive), and the corpus
  is technical/code-heavy prose where aggressive stemming risks
  conflating distinct identifiers (e.g. stemming `"resolver"` and
  `"resolve"` together might help, but stemming `"Trigram"` and
  `"trigrams"` together could also collide with unrelated short tokens).
  If B1 (punctuation stripping) and Lever C (alias table) together don't
  close enough of the gap, stemming becomes a candidate for a **future**
  round, not this one.
- **Option B3 (Unicode NFC/NFKD normalization + diacritic stripping)**:
  considered and deprioritized — the corpus audit (B3.1/B3.2) didn't
  surface diacritic-sensitive query failures as a measured problem, so
  this would be solving a gap not yet shown to exist. Worth a quick
  diagnostic check (does any dev/calibration query actually contain
  diacritics the corpus's notes don't match?) before building it; if the
  diagnostic shows zero affected queries, this lever is dropped from
  Priority 2 entirely rather than implemented speculatively.

### Lever C — Deterministic synonym/alias table

- **Option C1 (small, hand-curated, auditable table)**: a short, explicit
  list of domain-specific alias pairs this corpus actually repeats (the
  original design doc's own examples: `"SEC-001"` ↔ `"symlink TOCTOU
  bug"`, `"B2-ID-018"` ↔ `"TypeScript interface extraction gap"`), each
  entry justified by a real query/note pair from the existing corpus
  where the alias would help, applied as query-time expansion (not
  index-time — expanding at index time would bloat every document's
  token set; expanding at query time only touches the query actually
  being searched for). **Recommended**: small, reversible, every entry
  individually auditable and justified, matches the original design
  doc's own framing of this lever ("cheap, auditable, and reversible,
  unlike a learned embedding").
- **Option C2 (auto-mined synonym table, e.g. from co-occurrence
  statistics)**: rejected — this corpus (53 notes) is far too small for
  co-occurrence-based synonym mining to produce anything but noise, and
  it reintroduces exactly the "uncontrolled, hard-to-audit list"
  anti-goal above.
- **Option C3 (no alias table at all this round)**: a legitimate fallback
  if the diagnostic step (identifying real alias candidates from actual
  dev/calibration query/note pairs) doesn't surface enough genuine cases
  to justify even a small table — this lever is only built if there's
  real evidence for specific entries, not built speculatively and then
  populated.

## Chosen approach (this design)

**A1 + B1 + C1**, each as its own fully separate ablation against the
official B1 baseline, in that order (A then B then C), with each
accepted change becoming the new reference baseline for the next lever's
ablation — same "sequential accepted-baseline chaining" rule the parent
B3-TUNE-v2 design already established for priorities generally, now
applied within Priority 2's own three levers.

Rationale: each of A1/B1/C1 is the version of its lever that is
small, deterministic, auditable, and testable in isolation, which is
what lets each stay a genuinely separate ablation rather than a bundled,
hard-to-attribute change. The rejected alternatives in each lever either
reach for more complexity than this corpus's size (53 notes, 30-60
queries per split) can support without overfitting, or solve a problem
not yet shown to exist.

## What's measured for each ablation

For every one of the three levers' ablations (dev, then calibration):

- Recall@10 (+ CI), nDCG@10 (+ CI) — same frozen formulas B3 established,
  unchanged.
- Abstention confusion matrix (TP/TN/FP/FN) — **diagnostic only** in this
  phase (Priority 1 is closed; this is a correctness tripwire, not a
  tuning target): if a lexical change causes B1 to start returning
  `NotPresent` where it previously didn't (or vice versa), that is
  flagged explicitly, since it would mean the lexical change altered
  candidate-retrieval behavior in a way that needs its own scrutiny
  before being accepted as a pure "ranking quality" improvement.
- **Signal separation diagnostic** (Goal 2, secondary): rerun
  `b3_tune_v2_signal_analysis`'s existing `ratio`-signal computation
  under each accepted lexical configuration, and report whether
  `separation_gap` moves (toward less negative = less overlap = better
  separation, or more negative = worse) relative to the official B1
  baseline's already-measured gap (-0.445 on calibration). This is
  reported for information only — it does not gate KEEP/REJECT for the
  lexical change itself, and it does not reopen Priority 1's threshold
  search even if the gap improves; it only informs whether a *future*,
  separately-authorized abstention round would have better raw material
  to work with.
- Delta vs. the official B1 baseline (`eval/results/B3/frozen-config.json`,
  not the previous lever's ablation, so drift across A→B→C stays visible
  — same rule B3-TUNE-v2's parent design already states).
- KEEP/REJECT per ablation, with rationale, before moving to the next
  lever.

## What this design does NOT decide

- The exact `k1`/`b` grid values beyond the illustrative set above — the
  implementation plan fixes the final small candidate list.
- Whether Option B3 (Unicode normalization) is built at all — gated on
  the quick diagnostic described above; if that diagnostic (part of the
  plan's own early steps, not this design) finds zero affected queries,
  B3 is dropped without further work.
- The exact alias-table entries — the plan's own diagnostic step
  identifies real candidates from dev/calibration query/note pairs
  before any entry is added; this design does not pre-commit specific
  pairs beyond the two inherited examples from the parent design doc,
  which are illustrative, not a confirmed final list.
- Any acceptance tolerance beyond "measure against calibration, propose,
  STOP for approval, same two-stop discipline as B3.4 and Priority 1's
  own threshold proposal."

## What this document does NOT authorize

- No code changes, no new metric runs — all of that requires a separate,
  approved implementation plan (per `superpowers:writing-plans`/
  `superpowers:executing-plans`, the same process Priority 1's plan
  used).
- Priority 3 (reranking) and Priority 4 (hybrid retrieval) — remain
  design-only, untouched by this document.
- No modification to the B3 frozen config, runner, corpus, or labels.
- No read of the v2 held-out set.
- No MCP, no merge/tag/push.
