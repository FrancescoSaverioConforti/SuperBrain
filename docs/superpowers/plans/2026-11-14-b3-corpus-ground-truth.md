# B3.1/B3.2 — Corpus, Query Set & Ground Truth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task (approved execution approach: Native — implement in the current session, then one independent review of the whole B3.1/B3.2 result after Task 8, per this plan's closing section). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build one frozen, source-faithful corpus of real-derived notes
under `eval/rag/corpus/notes/`, plus dev/calibration/final-test query and
label files that query that single corpus, with an extended label schema
supporting graded relevance, abstention, citation spans, category and
difficulty — all isolated from the production vault per the existing
contamination-control contract.

**Architecture:** Corpus notes are authored as real `storage`/`domain`
frontmatter Markdown files, validated by committing each one through
`storage::propose_and_commit` into a throwaway vault inside a build/test
harness (never into the real vault, which stays empty). Query/label files
are hand-authored JSONL, loaded by an extended `eval/rag/runner.py` whose
schema validation is covered by Python unit tests. A Rust integration test
(extending the existing `held_out_labels` test) proves the corpus and each
split's labels survive two cold index rebuilds without the labels/queries
themselves leaking into the index.

**Tech Stack:** Rust (`storage`, `domain`, `index`, `api` crates, existing),
Python 3 (`eval/rag/runner.py`, stdlib `json`/`unittest`, existing).

**Spec:** `docs/superpowers/specs/2026-11-14-b3-corpus-ground-truth-design.md`
(this plan implements that design's B3.1 + B3.2 sections only; read both).

## Global Constraints

- B3 is text-only: no `graph_query`/`get_code_context`, no CodeGraph
  integration, no MCP server, no Task 10 — none of this plan touches the
  code graph or builds any server.
- Corpus is source-faithful: every note is real, non-fabricated, carries no
  claim unsupported by its source, and records explicit provenance.
- Queries are independently formulated: never a verbatim substring of their
  target note's body.
- One frozen corpus (`eval/rag/corpus/notes/`); dev/calibration/final-test
  differ only in query/label files, never in the document collection.
- `controllore` is read from `C:\Users\fsave\Desktop\Python_code\controllore`
  pinned at commit `ec274f4680c19f5b335a01a7a563c8f64546e7e2`, read-only —
  no write of any kind to that repo.
- Production vault is untouched (there is none populated) — no change to
  `crates/storage`, `crates/index`, `crates/api`, or `crates/domain`
  production code; only `eval/rag/` tooling changes.
- No merge/tag/push in this plan; work happens on the current branch/worktree
  as directed at execution time.
- Every corpus note and label file is a fixture — the plan's Rust test
  changes touch only `crates/api/tests/b1_api.rs` and (if needed)
  `crates/index/tests/b1_search.rs`, never production modules.

## Review Focus

- A query whose wording accidentally contains a verbatim substring of its
  target note's body (the thing B3.1 explicitly forbids) — Task 3's
  authoring step and Task 5's validation script must catch this before it
  ships, not just rely on the author's memory.
- A `graded_relevance` map naming a `note_id` that doesn't exist in the
  frozen corpus, or a `relevant_note_ids` entry missing from
  `graded_relevance` (an inconsistent label the runner should reject, not
  silently accept) — Task 4's extended `load_labels` must validate this.
- An `abstain_expected: true` row that still carries a nonempty
  `relevant_note_ids` (contradicts its own abstention claim) — explicitly
  in the design's protocol; Task 4's loader must reject it. Abstention rows
  are NOT exempt from the rest of the schema: they still require
  `category` and `difficulty` (so per-category/difficulty abstention can be
  checked), and must carry `relevant_note_ids: []`, `graded_relevance: {}`,
  `citation_spans: []` explicitly (not omitted).
- A code-derived note whose `repository`/`commit`/`path` fields don't match
  the actually-frozen `controllore` commit (provenance drift between
  authoring time and the final corpus manifest) — Task 6's manifest
  generation must be built from the same frozen commit used in Task 2, and
  cross-checks every corpus note file against the explicit
  `corpus-provenance.json` map (never an id-prefix heuristic), failing
  loudly on any mismatch.
- The existing `held_out_labels` Rust test (`crates/api/tests/b1_api.rs`)
  silently rotting because it still points at the old
  `eval/rag/test/notes/isolation-smoke.md` single-note path after the
  corpus moves to `eval/rag/corpus/notes/` — Task 8 updates it to the new
  layout and confirms it still proves the same isolation property.

---

## File Structure

- Create: `eval/rag/corpus/notes/*.md` — the frozen corpus (NoteType-typed
  Markdown fixtures), one file per note.
- Create: `eval/rag/corpus-manifest.json` — repo/commit hashes, note counts
  per category, build date, build commit.
- Create: `eval/rag/annotation-notes.md` — audit trail of annotation
  rationale for borderline relevance calls (not read by the runner).
- Modify: `eval/rag/dev/labels.jsonl`, `eval/rag/calibration/labels.jsonl`,
  `eval/rag/test/labels.jsonl` — replace mechanical fixture rows with real
  query/label rows for their split.
- Modify: `eval/rag/runner.py` — extend `load_labels` to validate the new
  required fields and cross-field consistency rules.
- Modify: `eval/rag/test_runner.py` — add unit tests for the extended
  schema validation.
- Modify: `eval/rag/README.md` — document the new single-corpus layout and
  the "note-based retrieval, not code-graph" scope sentence.
- Modify: `crates/api/tests/b1_api.rs` — update `held_out_labels` to the new
  corpus path; keep its isolation assertions intact.
- Delete: `eval/rag/test/notes/isolation-smoke.md` (superseded by the real
  corpus; its mechanical role is absorbed by Task 8's updated test, which
  uses a dedicated canary row instead of a whole fixture note).

---

### Task 1: Extract and freeze source material inventory

**Files:**
- Create: `eval/rag/.build/source-inventory.md` (scratch working doc, not
  shipped to the runner — a plain list, deleted or kept at your discretion
  after Task 7; not read by any code)

**Interfaces:**
- Produces: a frozen list of (source document/commit, excerpt, proposed
  note type, proposed category) tuples that Task 2 and Task 3 draft from.
  No code interface — this is a content-curation task.

- [ ] **Step 1: List SuperBrain source material for the notes/decisions/bugs/patterns strata**

Run `git -C . log --oneline -40` and read `docs/Spec_mvp.md`,
`docs/technical-debt.md`, and the continuity decisions already captured in
this session (ids `d188qf`…`d197zq`) to pick ~15-25 real, citable facts
(a decision, a bug/fix, a pattern, a past session/handoff) that can become
one atomic note each. For any fact sourced from a continuity decision,
copy its exact title and body text into the inventory line now — this
inventory, not a later live continuity lookup, is what Task 6's
`corpus-provenance.json` will copy from, so the mapping stays
reproducible even if continuity state later changes. Record each as one
line: `source ref -> one-sentence fact -> proposed NoteType -> proposed
category` in `eval/rag/.build/source-inventory.md`.

- [ ] **Step 2: List controllore + SuperBrain code material for the code/identifier/cross-file strata**

In `controllore` (read-only, pinned at
`ec274f4680c19f5b335a01a7a563c8f64546e7e2`) and in SuperBrain's own
`crates/`, pick ~15-25 real functions/modules/concepts whose signature or a
short real excerpt can anchor a `NoteType::Code` note (e.g. a specific
function signature, a specific module's responsibility, a specific
cross-file call relationship visible by reading the code — not inferred).
Record each as `repo@commit:path[:symbol] -> one-sentence fact -> proposed
category` in the same inventory file.

- [ ] **Step 3: Tag each inventory entry with a target month/period for the temporal stratum**

For entries with a real, known date (commit date, decision date from
continuity, doc date), add that date to the inventory line. Confirm at
least 3 distinct real months are represented, so temporal queries ("ad
agosto", "il mese scorso" relative to a fixed reference date used in
labels) have genuine targets.

- [ ] **Step 4: Commit the inventory as a working artifact**

```bash
git add eval/rag/.build/source-inventory.md
git commit -m "docs: B3.1 source material inventory for corpus curation"
```

---

### Task 2: Author the frozen corpus notes

**Files:**
- Create: `eval/rag/corpus/notes/*.md` (one file per note, named
  `<note-id>.md` matching each note's `id` field)
- Test: `eval/rag/.build/corpus-smoke.rs` is NOT created here — validation
  that these parse happens in Task 7 against the real `domain`/`storage`
  crates; this task only authors content.

**Interfaces:**
- Consumes: Task 1's `source-inventory.md`.
- Produces: `eval/rag/corpus/notes/*.md`, each a valid frontmatter+body
  Markdown note per `domain::NoteMetadata`'s field order (`id`,
  `schema_version`, `type`, `status`, `trust`, `classification`, `project`,
  `source`, `created`, `updated`, `event_period`, `relations`), consumed by
  Task 3 (query/label authoring references these `id`s) and Task 7
  (validation).

- [ ] **Step 1: Author ~40-45 notes/decisions/bug/pattern notes**

For each selected inventory entry, write one Markdown file with:
- `id`: a unique id in a valid `storage::NoteId` shape (check
  `crates/storage/src/propose.rs`'s `parse_note_id` for the accepted
  syntax before naming — reuse the pattern from existing fixtures such as
  `eval/rag/test/notes/isolation-smoke.md`'s `id`, e.g.
  `decision-2026-10-26-b1-baseline`).
- `schema_version: 1`, `type` one of `decision|bug|pattern|session`,
  `status` a real status (`draft`/`reviewed`/`verified` as honestly
  applicable — don't invent `verified` without the evidence C3's criteria
  require), `trust: internal`, `classification: internal` (never
  `sensitive`/`secret` — this is a public benchmark corpus),
  `project: eval-b3`, `source` pointing at the real originating doc/commit
  (e.g. `docs/Spec_mvp.md` or a commit hash), real `created`/`updated`
  dates from the source, `event_period` matching the real event date,
  `relations: {}` unless a genuine cross-note relation exists in the
  source material (e.g. a decision that really supersedes another in this
  corpus — only link `supersedes`/`derived_from` etc. if both notes exist
  in this corpus and the relation is real).
- Body: the fact, condensed to one atomic claim, faithful to the source,
  no invented detail.

- [ ] **Step 2: Author ~15-25 code/identifier/cross-file notes**

(Combined Step 1 + Step 2 total must land at 60 notes ±10 — see Step 3's
count check below.)

For each selected code inventory entry, write one Markdown file with
`type: code`, same metadata conventions as Step 1, plus in the body: the
real signature/snippet verbatim, and explicit provenance lines (plain
text, not new frontmatter fields): `repository: <name>`,
`commit: <hash>`, `path: <original path>`, and `symbol: <name>` when
applicable.

- [ ] **Step 3: Confirm category, count, and temporal coverage across the whole corpus**

Tally notes against the 6 categories (notes-decisions, bugs-fixes,
patterns-knowledge, temporal, identifier-code, cross-file-concept) and
confirm every category has at least 5 corpus notes to draw queries from,
the corpus totals 60 notes ±10 (50-70), and at least 3 distinct real
months are represented in `event_period` values. Adjust Step 1/2 counts if
a category is short or the total is outside 50-70.

- [ ] **Step 4: Commit the corpus**

```bash
git add eval/rag/corpus/notes/
git commit -m "feat(eval): author frozen B3 corpus notes"
```

---

### Task 3: Author dev/calibration/final-test queries (content only, no labels yet)

**Files:**
- Create: `eval/rag/.build/queries-draft.jsonl` (scratch: id, question,
  category, difficulty, intended target note id(s) — not the shipped
  label file; Task 4 turns this into the real `labels.jsonl` files with
  the full schema)

**Interfaces:**
- Consumes: Task 2's corpus (`eval/rag/corpus/notes/*.md`, specifically
  each note's `id` and body content).
- Produces: `eval/rag/.build/queries-draft.jsonl` rows consumed by Task 4.

- [ ] **Step 1: Draft ~100 answerable queries across the 6 categories and 3 difficulties**

For each corpus note selected as a query target, write one independently
formulated question (paraphrase/reformulation, verified by eye to share no
verbatim substring ≥ 4 consecutive words with the target note's body) at
one of three difficulties: `easy` (shares distinctive terms), `medium`
(some vocabulary mismatch), `hard` (substantial mismatch, still answerable
from that one note alone). Target roughly 16-17 queries per category,
spread across difficulties, so that answerable + abstention queries
(Step 2) total ~120.

- [ ] **Step 2: Draft ~15-20 abstention queries, filling the ~120 total**

For each, pick a real, specific, plausible question about a topic
adjacent to the corpus but deliberately not covered by any corpus note
(verified by checking it matches no note's content) — never a nonsense
question. Assign each a real `category` and `difficulty` (abstention
queries still belong to one of the 6 categories and one difficulty level,
per the loader's requirement — see Task 4's corrected schema). Mark these
with no target note id.

- [ ] **Step 3: Verify no query is a verbatim substring of its target note**

For every drafted query with a target note, confirm by inspection that no
4+-consecutive-word run from the query appears verbatim in the target
note's body. Rewrite any that fail this check.

- [ ] **Step 4: Split query ids into dev/calibration/final-test (50/25/25)**

Stratify so every category and every difficulty appears in all three
splits in a roughly comparable ratio. Record the split assignment as a
column in `eval/rag/.build/queries-draft.jsonl`.

- [ ] **Step 5: Commit the draft**

```bash
git add eval/rag/.build/queries-draft.jsonl
git commit -m "docs: B3.2 query draft with split assignment"
```

---

### Task 4: Extend the label schema and loader, write labels for all three splits

**Files:**
- Modify: `eval/rag/runner.py:5` (`load_labels` function)
- Modify: `eval/rag/test_runner.py` (add new test cases)
- Create: `eval/rag/dev/labels.jsonl`, `eval/rag/calibration/labels.jsonl`,
  `eval/rag/test/labels.jsonl` (overwrite the existing mechanical-fixture
  rows with real rows; keep each split's file to that split's queries only)

**Interfaces:**
- Consumes: Task 3's `eval/rag/.build/queries-draft.jsonl`; Task 2's
  corpus note ids (to populate `relevant_note_ids`/`graded_relevance`/
  `citation_spans`).
- Produces: `load_labels(eval_root, split, vault_root) -> list[dict]`,
  same signature as today, each dict now ALWAYS required (abstain and
  non-abstain rows alike) to carry `category: str` (nonempty),
  `difficulty: str` (one of `"easy"|"medium"|"hard"`),
  `abstain_expected: bool`, `relevant_note_ids: list[str]`,
  `graded_relevance: dict[str, int]` (keys == `relevant_note_ids`, values
  in `{0,1,2}`), `citation_spans: list[dict]` (each `{"note_id": str,
  "excerpt_substring": str}`) — for `abstain_expected: true` rows,
  `relevant_note_ids`, `graded_relevance`, and `citation_spans` must be
  present and exactly empty (`[]`/`{}`/`[]`), not omitted; for
  `abstain_expected: false` rows they must be nonempty/consistent as
  before. Consumed by Task 5's tests and Task 7's Rust isolation test
  (which only checks the file is loadable and canary-free, not full
  schema — the Python loader is the schema's single source of truth).

- [ ] **Step 1: Write the failing test for required-field validation**

```python
def test_rejects_row_missing_new_required_fields(self):
    self.eval_root... # existing fixture pattern from LabelIsolationTests
    row = {"id": "q1", "question": "...", "answer": "...",
           "abstain_expected": False}  # missing category/difficulty/relevant_note_ids/graded_relevance/citation_spans
    (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
    with self.assertRaises(ValueError):
        load_labels(self.eval_root, "test", self.vault)
```

- [ ] **Step 2: Write the failing test for abstain/relevant_note_ids consistency**

```python
def test_rejects_abstain_row_with_nonempty_relevant_note_ids(self):
    row = {"id": "q1", "question": "...", "answer": "...", "category": "temporal",
           "difficulty": "easy", "abstain_expected": True,
           "relevant_note_ids": ["some-note"], "graded_relevance": {"some-note": 2},
           "citation_spans": []}
    (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
    with self.assertRaises(ValueError):
        load_labels(self.eval_root, "test", self.vault)

def test_rejects_abstain_row_missing_category_or_difficulty(self):
    row = {"id": "q1", "question": "...", "answer": "...", "abstain_expected": True,
           "relevant_note_ids": [], "graded_relevance": {}, "citation_spans": []}
    (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
    with self.assertRaises(ValueError):
        load_labels(self.eval_root, "test", self.vault)
```

- [ ] **Step 3: Write the failing test for graded_relevance/relevant_note_ids key mismatch**

```python
def test_rejects_graded_relevance_keys_not_matching_relevant_note_ids(self):
    row = {"id": "q1", "question": "...", "answer": "...", "category": "temporal",
           "difficulty": "easy", "abstain_expected": False,
           "relevant_note_ids": ["note-a"], "graded_relevance": {"note-b": 2},
           "citation_spans": []}
    (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
    with self.assertRaises(ValueError):
        load_labels(self.eval_root, "test", self.vault)
```

- [ ] **Step 4: Run the three new tests to verify they fail**

Run: `python -B -m unittest eval.rag.test_runner -k new_required_fields -v`
(and the other two test names)
Expected: FAIL — `load_labels` does not yet check these fields, so no
`ValueError` is raised (existing minimal validation only checks
`id`/`question`/`answer`).

- [ ] **Step 5: Implement the extended validation in `load_labels`**

In `eval/rag/runner.py`, after the existing `id`/`question`/`answer`
string check, for EVERY row (abstain or not): require `category` (str,
nonempty) and `difficulty` (one of `easy`/`medium`/`hard`) and
`abstain_expected` (bool) to be present. Then: if `abstain_expected is
True`, require `relevant_note_ids == []` and `graded_relevance == {}` and
`citation_spans == []` exactly (present, not omitted, and empty); if
`abstain_expected is False`, require `relevant_note_ids` (nonempty list of
str), `graded_relevance` (dict whose key set equals
`set(relevant_note_ids)`, values each in `{0,1,2}`), `citation_spans`
(nonempty list of dicts each with string `note_id`/`excerpt_substring`).
Raise `ValueError` on any violation, same style as the existing checks (no
new exception types).

- [ ] **Step 6: Run the three new tests plus the existing suite to verify all pass**

Run: `python -B -m unittest discover -s eval/rag -p test_runner.py -v`
Expected: PASS (all existing `LabelIsolationTests` plus the three new
tests).

- [ ] **Step 7: Transcribe Task 3's draft into the three real `labels.jsonl` files**

For each split, write one JSON line per query assigned to that split in
`eval/rag/.build/queries-draft.jsonl`, in the full schema from the
Interfaces block above, with `answer` filled honestly (a short factual
answer derivable from the target note, or an empty string for abstention
rows), `citation_spans` pointing at a real verbatim substring of the
target note's body (recorded by re-reading the actual corpus file — do not
guess the substring), and for abstention rows explicitly writing
`"relevant_note_ids": [], "graded_relevance": {}, "citation_spans": []`
(never omitted).

- [ ] **Step 8: Validate all three split files load cleanly**

Run (from a Python REPL or a throwaway script):
```python
from eval.rag.runner import load_labels
from pathlib import Path
for split in ["dev", "calibration", "test"]:
    rows = load_labels(Path("eval/rag"), split, Path("eval/rag/_no_such_vault"))
    print(split, len(rows))
```
Expected: no exception, three nonzero counts printed.

- [ ] **Step 9: Commit**

```bash
git add eval/rag/runner.py eval/rag/test_runner.py eval/rag/dev/labels.jsonl eval/rag/calibration/labels.jsonl eval/rag/test/labels.jsonl
git commit -m "feat(eval): extend label schema for graded relevance, abstention, citation spans"
```

---

### Task 5: Cross-corpus validation script (catches the Review Focus risks)

**Files:**
- Create: `eval/rag/validate_corpus.py`
- Test: `eval/rag/test_validate_corpus.py`

**Interfaces:**
- Consumes: `eval/rag/corpus/notes/*.md` (note ids and bodies, parsed by a
  minimal hand-written frontmatter reader — split the file on the first
  two `---`-only lines to get the frontmatter block and body, then parse
  the frontmatter block as flat `key: value` lines, needing only the `id:`
  value here; no PyYAML or other new dependency, no dependency on the Rust
  crates), and `load_labels` output for each split.
- Produces: `validate_corpus(corpus_dir: Path, eval_root: Path) ->
  list[str]` returning a list of human-readable problem descriptions
  (empty list == clean). Consumed by Task 7's report and by CI-style
  invocation in Task 8.

- [ ] **Step 1: Write the failing test for a dangling `graded_relevance` note id**

```python
def test_flags_note_id_not_present_in_corpus(self, tmp_path):
    # build a tiny corpus dir with one real note file, a labels dir whose
    # row references a note id that doesn't exist in that corpus
    ...
    problems = validate_corpus(corpus_dir, eval_root)
    assert any("unknown note id" in p for p in problems)
```

- [ ] **Step 2: Write the failing test for a query substring collision**

```python
def test_flags_query_that_is_verbatim_substring_of_its_target_note(self, tmp_path):
    # note body contains "the quick brown fox jumps"; label's question
    # contains the same 5-word run verbatim
    ...
    problems = validate_corpus(corpus_dir, eval_root)
    assert any("verbatim substring" in p for p in problems)
```

- [ ] **Step 3: Run both tests to verify they fail (function doesn't exist yet)**

Run: `python -B -m unittest eval.rag.test_validate_corpus -v`
Expected: FAIL with `ImportError` or `AttributeError`.

- [ ] **Step 4: Implement `validate_corpus`**

Parse every `*.md` in `corpus_dir` for its `id` and body using a small
hand-written splitter (no PyYAML): split the file text on lines that are
exactly `---`, take the frontmatter block as the text between the first
and second such line, extract `id:`'s value by a simple `line.split(":",
1)` scan (every corpus note's `id` line has no nested structure — this is
sufficient, unlike the full `domain::parse_frontmatter` grammar, which
this script does not need and must not reimplement), and take the body as
everything after the second `---` line. For each split's labels (via
`load_labels`): for every `relevant_note_ids`/`graded_relevance`/
`citation_spans[].note_id` entry, confirm the id exists in the parsed
corpus set; for every non-abstain row, scan whether any 4-consecutive-word
run of `question` appears verbatim (case-insensitive) in its target
note(s)' body; collect and return all violations as strings.

- [ ] **Step 5: Run both tests to verify they pass**

Run: `python -B -m unittest eval.rag.test_validate_corpus -v`
Expected: PASS.

- [ ] **Step 6: Run `validate_corpus` against the real Task 2/4 artifacts**

```bash
python -c "from eval.rag.validate_corpus import validate_corpus; from pathlib import Path; print(validate_corpus(Path('eval/rag/corpus'), Path('eval/rag')))"
```
Expected: `[]`. If not, fix the flagged corpus/label files (go back to
Task 2 Step 1/2 or Task 4 Step 7) before proceeding.

- [ ] **Step 7: Commit**

```bash
git add eval/rag/validate_corpus.py eval/rag/test_validate_corpus.py
git commit -m "test(eval): add cross-corpus label consistency validator"
```

---

### Task 6: Build the corpus manifest, with an explicit per-note provenance map

**Files:**
- Create: `eval/rag/corpus-manifest.json`
- Create: `eval/rag/corpus-provenance.json` (NEW: the explicit, hand-built
  `note_id -> {source, provenance, stratum, category}` map — the single
  reproducible source of truth for "which note came from where," never
  inferred)
- Create: `eval/rag/build_manifest.py` (small script, run once; its output
  is the manifest file, not a long-lived service)

**Interfaces:**
- Consumes: `eval/rag/corpus-provenance.json` (authored in this task, by
  hand, from Task 1's `source-inventory.md` and Task 2's actual corpus
  files — NOT inferred from note id prefixes or any other heuristic), the
  frozen `controllore` commit `ec274f4680c19f5b335a01a7a563c8f64546e7e2`,
  the current SuperBrain `HEAD` commit at build time.
- Produces: `eval/rag/corpus-provenance.json` as a flat JSON object keyed
  by every corpus note's `id`, each value
  `{"category": str, "stratum": "project-history"|"code", "source":
  str}` where `source` is a frozen, independently verifiable reference —
  for `stratum: "project-history"` notes, a path under `docs/` plus
  (where the fact came from a continuity decision) that decision's id
  AND the exact decision title/body text copied verbatim into this file
  at build time (so the mapping does not depend on continuity's live,
  mutable session state — it is a frozen snapshot, not a live lookup); for
  `stratum: "code"` notes, `{"repository": "controllore"|"superbrain",
  "commit": str, "path": str, "symbol": str|null}`. Also produces
  `eval/rag/corpus-manifest.json` with keys: `build_date` (ISO date),
  `build_commit` (SuperBrain HEAD at build time), `sources`
  (`{"superbrain": {"commit": str}, "controllore": {"commit":
  "ec274f4680c19f5b335a01a7a563c8f64546e7e2", "path": str}}`),
  `note_count_by_category` (`dict[str, int]`, computed from
  `corpus-provenance.json`'s `category` field — never from id-prefix
  inference), `note_count_total` (int),
  `query_count_by_split_category_difficulty` (nested
  `dict[str, dict[str, dict[str, int]]]`, filled from Task 4's three label
  files). Consumed by Task 7's report.

- [ ] **Step 1: Author `eval/rag/corpus-provenance.json` by hand**

For every note file created in Task 2, add one entry keyed by that note's
real `id`, with the category you actually assigned it during authoring
and the source reference you actually used (a real `docs/` path and/or
continuity decision id with its text copied in, for project-history notes;
real `repository`/`commit`/`path`/`symbol` for code notes — these should
already be sitting in the note body from Task 2 Step 2, just duplicated
here in structured form for programmatic use). This file is the single
place that says what maps to what; nothing downstream infers category
from the note id string.

- [ ] **Step 2: Implement `build_manifest.py`**

Reads `eval/rag/corpus-provenance.json` for category tallies (never
parsing or pattern-matching note ids), cross-checks every corpus note file
in `eval/rag/corpus/notes/` has a matching entry (and vice versa) and
exits with an error listing any mismatch, reads all three label files via
`load_labels`, tallies `(split, category, difficulty)` counts, reads `git
rev-parse HEAD` for SuperBrain, writes the manifest JSON with the frozen
`controllore` commit hardcoded (it is pinned, not re-queried).

- [ ] **Step 3: Run it and inspect the output**

Run: `python eval/rag/build_manifest.py > eval/rag/corpus-manifest.json`
Expected: valid JSON, no provenance-mismatch error, `note_count_total`
matches Task 2's actual file count, `controllore` commit matches
`ec274f4680c19f5b335a01a7a563c8f64546e7e2` exactly.

- [ ] **Step 4: Commit**

```bash
git add eval/rag/corpus-provenance.json eval/rag/build_manifest.py eval/rag/corpus-manifest.json
git commit -m "feat(eval): generate B3 corpus manifest with explicit per-note provenance map"
```

---

### Task 7: Rust isolation test on the real corpus (replaces the single-fixture test)

**Files:**
- Modify: `crates/api/tests/b1_api.rs:286-320` (the `held_out_labels` test)
- Modify: `eval/rag/test/notes/isolation-smoke.md` → delete (superseded)

**Interfaces:**
- Consumes: `storage::propose_and_commit` (existing signature, unchanged —
  see `crates/storage/src/propose.rs:179`), `index::Index`,
  `index::indexer_impl::RealIndexer`, the real corpus files at
  `eval/rag/corpus/notes/*.md`, the real `eval/rag/test/labels.jsonl`.
- Produces: an updated `held_out_labels` test proving: every corpus note
  commits successfully through the real gate/vault path, and canary
  markers embedded in `eval/rag/test/labels.jsonl` (keep the existing
  `HELDOUT_QUESTION_CANARY_901`/`HELDOUT_ANSWER_CANARY_902` convention on
  at least one `test` split row) never surface from two cold index
  rebuilds, exactly as the current test proves for the single fixture note.

- [ ] **Step 1: Update the test to commit every corpus note, not one fixture**

Replace the single `include_str!("../../../eval/rag/test/notes/isolation-smoke.md")`
call with a loop over `std::fs::read_dir("../../../eval/rag/corpus/notes")`
(or a build-time `glob!`-free directory walk — plain `std::fs::read_dir`
is sufficient here), calling `storage::propose_and_commit` for each `*.md`
file's contents in turn, asserting each returns
`storage::CommitResult::Committed(_)`.

- [ ] **Step 2: Keep one real `test`-split labels row carrying a canary marker**

Add one extra query to `eval/rag/test/labels.jsonl` (full schema per
Task 4) whose `question`/`answer` text embeds
`HELDOUT_QUESTION_CANARY_901`/`HELDOUT_ANSWER_CANARY_902` (this row is a
mechanical isolation probe, counted separately from the ~22-ish real
final-test queries — note it in `eval/rag/README.md` per Task 9).

- [ ] **Step 3: Run the test to verify it still fails/passes as expected before adversarial setup**

Run: `cargo test -p api held_out_labels -- --exact --nocapture`
Expected: PASS through the "every corpus note commits" assertions; the
existing adversarial-placement and two-rebuild assertions below are
unchanged in logic, just now scoped to the real corpus/labels files.

- [ ] **Step 4: Run the full test to verify the adversarial/no-leak assertions still pass**

Run: `cargo test -p api held_out_labels -- --exact`
Expected: PASS — canary markers absent from both rebuilt indexes' search
results, same as today's test already proves for the single fixture.

- [ ] **Step 5: Delete the superseded single-fixture note**

```bash
git rm eval/rag/test/notes/isolation-smoke.md
```

- [ ] **Step 6: Commit**

```bash
git add crates/api/tests/b1_api.rs eval/rag/test/labels.jsonl
git commit -m "test(eval): point held_out_labels isolation test at the real B3 corpus"
```

---

### Task 8: Update documentation and run the full isolation/validation suite

**Files:**
- Modify: `eval/rag/README.md`

**Interfaces:**
- Consumes: nothing new.
- Produces: nothing consumed by later tasks — this is the plan's closing
  documentation + verification task.

- [ ] **Step 1: Rewrite `eval/rag/README.md`**

Document: the single-corpus/split-queries-only layout, the extended label
schema (list the new fields), the "B3 validates the MVP's note-based
retrieval; raw-repository retrieval and graph+text retrieval remain
separate future capabilities" sentence verbatim (per the design doc), and
the existing isolation rules (unchanged in spirit — runner-only loading,
no runtime crate dependency, contamination checks).

- [ ] **Step 2: Run the full Python test suite**

Run: `python -B -m unittest discover -s eval/rag -p "test_*.py" -v`
Expected: PASS (all of `test_runner.py` and `test_validate_corpus.py`).

- [ ] **Step 3: Run the full Rust isolation test and the existing B1 search/API suites**

Run: `cargo test -p api -p index -p storage`
Expected: PASS — no regression in any existing B1 test.

- [ ] **Step 4: Run `validate_corpus` one final time against the committed state**

Run: `python -c "from eval.rag.validate_corpus import validate_corpus; from pathlib import Path; print(validate_corpus(Path('eval/rag/corpus'), Path('eval/rag')))"`
Expected: `[]`.

- [ ] **Step 5: Commit**

```bash
git add eval/rag/README.md
git commit -m "docs(eval): document the B3 single-corpus layout and scope"
```

---

### Task 9: Independent review of the full B3.1/B3.2 result

Not a subagent dispatch — per the approved Native execution approach, this
is one independent review pass (fresh eyes on the finished result, run via
`superpowers:requesting-code-review` or an equivalent fresh review) covering
specifically:

- **Source fidelity** — every corpus note traces to a real source with no
  added/unsupported claims (spot-check a sample against
  `corpus-provenance.json` and the original source).
- **Provenance** — every note in `corpus-provenance.json` matches a real
  corpus file and vice versa; code notes' repository/commit/path match the
  frozen `controllore` commit exactly.
- **Query leakage** — no query contains a verbatim 4+-word run from its
  target note (re-run `validate_corpus.py`, then spot-check manually).
- **Relevance labels** — graded relevance values are defensible on
  rereading target notes; no note is marked relevant without actually
  answering the query.
- **Abstention** — abstention queries are genuinely unanswerable from the
  corpus, not disguised trick or nonsense questions; abstain rows carry
  the full required schema (category, difficulty, explicit empty fields).
- **Citation spans** — every `excerpt_substring` is a real, exact substring
  of its cited note's body.
- **Split stratification** — dev/calibration/test query counts are
  roughly 50/25/25, every category and difficulty appears in all three
  splits at a comparable rate.
- **Corpus isolation** — `cargo test -p api held_out_labels` and the full
  Python suite pass; no production vault or `controllore`/SuperBrain
  source files were modified.
- **Manifest reproducibility** — `build_manifest.py` re-run produces an
  identical `corpus-manifest.json` (byte-for-byte except `build_date`).

- [ ] **Step 1: Run the review and collect findings**
- [ ] **Step 2: Fix any confirmed finding, re-run the affected
  task's validation commands, and commit the fix**
- [ ] **Step 3: Report final B3.1/B3.2 results to the user per the
  design doc's acceptance-criteria report format, then STOP — no
  merge/tag/push, no further B3 steps without explicit authorization**

---

## Self-Review Notes

- **Spec coverage**: design doc's corpus construction, strata/sourcing,
  size/split, label schema, annotation protocol, and acceptance criteria
  each map to Tasks 1-8. The design's "what's left to the builder" items
  (exact wording, exact split %, exact difficulty criteria) are left open
  in Task 3/Step 1 as judgment calls, per the design doc's own framing.
- **Review Focus** risks each have an owning task: verbatim-substring
  queries → Task 3 Step 3 (authoring-time) + Task 5 (automated,
  catches what authoring missed); dangling/inconsistent label ids →
  Task 4 Steps 1-3 (loader) + Task 5 (cross-corpus); abstain/relevant_ids
  contradiction (including abstain rows still needing
  category/difficulty/explicit-empty fields) → Task 4 Steps 2 and 5;
  provenance drift → Task 6 (explicit `corpus-provenance.json` map,
  pinned to the frozen commit, cross-checked against the actual corpus
  files — never inferred from note id strings) + Task 5's note-id
  cross-check; stale `held_out_labels` path → Task 7.
- **Proportion**: this plan is data-authoring-heavy by nature of the work
  (curating ~90 notes and ~120 queries is mechanical content work, not
  something a task step can specify line-by-line) — code changes are
  confined to Tasks 4, 5, 6, 7, each with real signatures and real test
  names, no invented types.
