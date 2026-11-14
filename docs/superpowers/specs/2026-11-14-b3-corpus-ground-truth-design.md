# B3.1/B3.2 — Corpus, Query Set & Ground Truth: Design

Status: **approved with changes** (see continuity `d197xx`). This revision
incorporates those changes. Nothing has been built yet.

## Intent and why

B2 (code graph) closed. The B3 scope reconciliation (see continuity decisions
`d192dq`, `c26kq`, `c27dm`, `d193wx`, `d195ou`, `d196qp`) settled B3 as
**text-only retrieval evaluation**, per `docs/Spec_mvp.md` Fase B's acceptance
criteria (Recall@10/nDCG@10, citation correctness, abstention, latency, real
use). This doc covers only the first two of the four authorized B3 steps:

- **B3.1** — build the corpus and query set.
- **B3.2** — build ground truth and the annotation protocol.

B3.3 (metric runner) and B3.4 (calibration/threshold freeze) come after this
doc is approved and B3.1/B3.2 are built, per `superpowers:writing-plans`.

## What B3 actually evaluates (the key constraint)

Verified against the real code (`crates/api/src/search.rs`, `crates/index/src/document.rs`):
`api::search` / `index` today reads **only vault Markdown notes** — parsed
`NoteMetadata` plus a classification-gated `text_searchable` string, searched
via trigram + BM25. There is no tree-sitter indexer wired to this API. The B2
code graph lives in an unmerged worktree and is explicitly out of scope
(B3.0, B3's second scope decision).

**B3 validates the MVP's note-based retrieval; raw-repository retrieval and
graph+text retrieval remain separate future capabilities.** This sentence
goes verbatim into the corpus README so nobody later mistakes this benchmark
for a code-indexer evaluation.

Consequence: both corpus strata — vault-style notes and "code" content — are
represented as the **same thing the system actually indexes**: `NoteType`-typed
Markdown notes with frontmatter, committed into a dedicated eval vault tree,
never the production vault (which is empty today; there is no existing vault
content to evaluate against).

## Goals

- A frozen, stratified corpus of real-derived notes, large enough to give
  Recall@10/nDCG@10 a meaningful signal, split into dev/calibration/test.
- Query set stratified across 6 categories (per your B3.1 decision):
  notes/decisions, bugs/fixes, patterns/knowledge, temporal search,
  identifier/code, cross-file technical concept.
- Ground truth that supports Recall@10/nDCG@10 (graded relevance, not just a
  single "the answer"), citation-correctness annotation, and abstention
  annotation (queries with **no** correct note — NotPresent must be correct).
- Full provenance: repository, commit, original path/symbol for code-derived
  notes; source document for note-derived content.
- Everything built sits under `eval/rag/`, isolated from the real vault, per
  the existing isolation contract (`eval/rag/README.md`, `test_runner.py`,
  `cargo test -p api held_out_labels`).

## Non-goals / anti-goals

- Not building a tree-sitter or code-graph retrieval path.
- Not touching `crates/index`, `crates/api`, or `crates/storage` production
  code in this step (B3.1/B3.2 produce data + a runner, not core changes;
  the labels schema extension described below is additive, in `eval/rag/`
  tooling only, not in the production note schema).
- Not running the final held-out test (B3.5) or building the MCP server.
- Not modifying `controllore` or `SuperBrain` source repos — read-only
  extraction only.
- Not creating separate document collections per split (corrected below —
  one frozen corpus, split only at the query/labels level).
- Not artificially paraphrasing the corpus itself — notes stay
  source-faithful (corrected below). Paraphrasing applies only to queries.
- A failure mode to explicitly avoid: queries copied verbatim from source
  text (your B3.1 requirement) — every query is a paraphrase/reformulation,
  never a substring of the note it targets.

## Corpus construction

**Correction from the first draft**: there is **one frozen corpus**, not
per-split document collections. Dev/calibration/test differ only in which
*queries and labels* they contain; all three query against the same
`eval/rag/corpus/notes/`. This measures whether query formulation and
retrieval configuration generalize, without also varying the document
collection underneath — a confound the original per-split-corpus design
would have introduced. The held-out final test stays fully held-out at the
query/labels level: its questions and relevance judgments are never read
until B3.5.

### Strata and sourcing

Notes are **source-faithful**: curated into atomic notes, staying true to
the source, adding no unsupported claims. Paraphrasing is reserved for
*queries*, never applied to the corpus itself.

| Stratum | Source | How it becomes a note |
|---|---|---|
| decision / pattern / bug-fix / session-like | Real material from *this* project's history: `git log` on SuperBrain itself, `docs/Spec_mvp.md`, `docs/technical-debt.md`, existing continuity decisions, past plan/handoff docs under `docs/superpowers/` and `docs/handoffs/` | Rewritten into a `NoteType::{Decision,Bug,Pattern,Session}` atomic note, frontmatter filled honestly (real dates from source, `trust: internal`), body faithful to the source material (condensed to one atomic claim per note, not paraphrased into something new), `source` field pointing at the originating doc/commit |
| code / identifier / cross-file concept | `controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2` and `SuperBrain` (code history, not the eval corpus itself) | A `NoteType::Code` note per selected function/module/concept: frontmatter + body containing the real signature/snippet verbatim, plus explicit `repository`, `commit`, `path`, and optional `symbol` fields recorded in the note body (curation metadata, carried as plain text in the note, not new `domain::NoteMetadata` fields) |
| temporal | Drawn from both strata above, selecting notes whose real `created`/`event_period` spans known months, so "ad agosto"-style queries have genuine stratified dates to hit | Same note objects as above; temporal stratification is a selection criterion over the corpus, not a separate content source |

Both source repos are read-only inputs; nothing is written back to them.
Commit hashes are frozen in the corpus manifest (see below).

Queries, by contrast, are always independently formulated: never a
substring of the target note, always a realistic paraphrase/reformulation —
this is what's being measured, so it's the one thing that must not be
source-copied.

### Corpus size and query split (proposed, open to your adjustment)

- Target **~60–100 notes total** in the single frozen corpus — large enough
  that Recall@10 is not trivially 1.0 or 0.0 by accident, small enough to
  hand-curate and hand-verify in this step.
- Queries split **dev 50% / calibration 25% / final-test 25%**, stratified
  across all 6 categories (no category exists only in one split) **and**
  across a new `difficulty: easy | medium | hard` dimension, kept at a
  roughly comparable distribution across splits (e.g. each split gets a
  similar easy/medium/hard mix per category, not concentrated in one split).
- "Easy" = query shares distinctive terms with the target note (BM25 should
  find it readily); "medium" = paraphrased with some vocabulary mismatch;
  "hard" = paraphrased with substantial vocabulary mismatch or requires
  combining cues (still answerable from the note, not a trick question).
- The corpus itself is not split — every query in every split is evaluated
  against the same full corpus.

### Where it lives

```
eval/rag/
  corpus/notes/*.md        (single frozen corpus, NoteType-typed fixtures,
                             committed via storage::propose_and_commit into
                             the eval vault, per the existing isolation rule)
  dev/labels.jsonl
  calibration/labels.jsonl
  test/labels.jsonl
  corpus-manifest.json     (repo/commit hashes, note counts per
                            stratum/category, build date, build commit)
```

This replaces the original per-split `notes/` subfolders; the existing
`test/notes/isolation-smoke.md` mechanical fixture is superseded by the real
corpus (kept or removed per whatever `test_runner.py` isolation tests
require once extended).

## Ground truth and annotation protocol (B3.2)

### Label schema (extends the current one)

Today's loader (`eval/rag/runner.py::load_labels`) requires only
`{id, question, answer}` strings — built for a single expected answer, not
for recall/nDCG over a ranked list. B3.2 proposes extending the schema
**additively** (old fields kept, loader's strict validation extended, not
loosened):

```json
{
  "id": "q-dec-003",
  "question": "perché abbiamo scelto BM25 invece di un reranker nella fase B?",
  "category": "notes-decisions",
  "difficulty": "medium",
  "relevant_note_ids": ["decision-2026-10-26-b1-baseline"],
  "graded_relevance": {"decision-2026-10-26-b1-baseline": 2},
  "answer": "Perché B1 deve passare con la sola baseline deterministica...",
  "abstain_expected": false,
  "citation_spans": [{"note_id": "decision-2026-10-26-b1-baseline", "excerpt_substring": "sola baseline deterministica"}]
}
```

- `relevant_note_ids` / `graded_relevance` (0=irrelevant, 1=partially,
  2=fully relevant) feed Recall@10/nDCG@10.
- `abstain_expected: true` marks queries with **no** relevant note in that
  split's corpus — these test the NotPresent path and must never have a
  nonempty `relevant_note_ids`.
- `citation_spans` feeds the citation-correctness protocol: a returned note
  is "correct" only if the excerpt actually contains the cited content, not
  merely the same topic (per spec's explicit requirement).
- `category` is one of the 6 agreed strata, used to check per-category
  recall/abstention separately so no threshold is met by always returning
  zero results.
- `difficulty` (`easy`/`medium`/`hard`) lets B3.4 inspect whether thresholds
  hold up across harder paraphrases, not just the easy cases.

`load_labels` gets extended to validate the new required keys per non-abstain
row and reject rows mixing `abstain_expected: true` with a nonempty
`relevant_note_ids` — a dedicated test added alongside the existing
`LabelIsolationTests`.

### Annotation protocol

1. Each query is authored first (paraphrased from source material, never
   copied), **then** relevance is assigned by rereading the candidate notes
   — not the other way around, to avoid writing queries that trivially match
   their own wording.
2. Graded relevance (0/1/2) assigned by one pass of manual review against the
   corpus manifest; borderline cases (same topic, wrong specifics) default to
   0 and get a one-line rationale comment kept in a parallel
   `annotation-notes.md` (not shipped to the runner, audit trail only).
3. Citation spans are recorded as exact substrings of the target note's body
   at authoring time, so correctness checking later is a substring test, not
   a judgment call.
4. Abstention queries are authored by picking real questions about adjacent
   but absent topics (e.g. asking about a decision deliberately not included
   in that split's corpus), never by inventing nonsense questions — abstention
   must be hard, not trivial.
5. This protocol, the label schema, and the category definitions are frozen
   as of B3.2's completion — any later change to the protocol itself (not the
   thresholds) is a scope change back through you, same as any other spec
   change.

## What's left to the builder (open calls I'll make unless corrected)

- Exact note content and query wording (I will draft all of it; it is
  mechanical, not a design decision).
- Exact query split percentages (50/25/25) — adjustable if you want a
  different dev/calibration/final-test balance.
- Corpus size (~60–100) — adjustable; smaller risks noisy metrics, larger
  costs more curation time in this step.
- Exact easy/medium/hard criteria (drafted above) — adjustable if you want a
  different operational definition.
- `corpus-manifest.json` exact shape — I will design it to satisfy the
  "commit/hash/version of repos+vault recorded" requirement; happy to show it
  before committing if you want to review the format first.

## Acceptance criteria for this step (B3.1 + B3.2 only)

- Single frozen corpus built at `eval/rag/corpus/notes/`; dev/calibration/test
  each have disjoint query/labels files querying that same corpus.
- Every note is source-faithful (real, non-fabricated, no unsupported claims
  added) with recorded provenance (repo+commit+path for code notes, source
  doc for others); every query is independently formulated, never a
  substring of its target note's content.
- Labels validate under an extended `load_labels` with new required fields
  (`category`, `difficulty`, `relevant_note_ids`, `graded_relevance`,
  `abstain_expected`, `citation_spans`); existing isolation tests
  (`test_runner.py`, `cargo test -p api held_out_labels`) still pass
  unmodified in spirit (extended, not weakened).
- `corpus-manifest.json` records SuperBrain and controllore commit hashes,
  note counts per category, query counts per split/category/difficulty,
  build date and build commit.
- No modification to `controllore`, no modification to production vault
  (there is none), no merge/tag/push, no MCP, no graph_query/get_code_context,
  no Task 10.
- A short report back to you: corpus note count, query counts per
  split/category/difficulty, provenance coverage, annotation protocol as
  actually applied, any deviation from this design and why, test/isolation
  results.
