# RAG evaluation corpus (B3.1/B3.2 complete, not a benchmark result yet)

B3 validates the MVP's note-based retrieval; raw-repository retrieval and
graph+text retrieval remain separate future capabilities.

## Layout

One frozen corpus, three query/label splits:

```text
eval/rag/
  corpus/notes/*.md          one frozen corpus of 53 real, source-faithful
                              NoteType-typed notes (decision/bug/pattern/
                              session/code), shared by every split
  corpus-provenance.json     explicit, hand-authored note_id -> {category,
                              stratum, source} map; never inferred from
                              note id prefixes
  corpus-manifest.json       build date/commit, source repo commits, note
                              counts by category, query counts by
                              split/category/difficulty — regenerate with
                              build_manifest.py after any corpus or label
                              change
  dev/labels.jsonl           60 queries (50%)
  calibration/labels.jsonl   30 queries (25%)
  test/labels.jsonl          30 real queries + 1 mechanical isolation
                              canary row (31 lines total; see below) (25%)
```

Dev, calibration and test differ only in which queries/labels they
contain — all three query the SAME corpus in `corpus/notes/`. This
measures whether query formulation and retrieval configuration
generalize, without also varying the document collection underneath.
Only dev may inform retrieval changes; calibration freezes configuration;
the held-out test is read only after that freeze.

## Label schema

Every row in every split's `labels.jsonl` carries:

- `id`, `question`, `answer` (str; `answer` may be `""` only when
  `abstain_expected` is `true`)
- `category` (str; one of six strata: notes-decisions, bugs-fixes,
  patterns-knowledge, temporal, identifier-code, cross-file-concept)
- `difficulty` (`"easy"` | `"medium"` | `"hard"`)
- `abstain_expected` (bool)
- `relevant_note_ids` (list[str]; empty iff `abstain_expected` is `true`)
- `graded_relevance` (dict[note_id, 0|1|2]; keys equal `relevant_note_ids`)
- `citation_spans` (list of `{note_id, excerpt_substring}`; each
  `excerpt_substring` is a real, exact substring of that note's body;
  empty iff `abstain_expected` is `true`)

`runner.py::load_labels` validates all of this; it is the schema's single
source of truth — nothing else re-implements it. `validate_corpus.py`
additionally cross-checks every referenced `note_id` against the real
corpus and flags any query that shares a 4+-word verbatim run with its
own target note's body (queries must be independently formulated
paraphrases, never copied from the note they target).

## Isolation

`runner.py` alone loads labels. It has no runtime API role and requires
disjoint canonical evaluation/vault trees. A caller imports only note
fixtures into the vault through `storage::propose_and_commit`. Never
import labels, questions, answers or runner output into the vault, index,
sessions, summaries, logs or caches. Runtime crates have no reference to
this loader or label paths. The Rust API isolation test
(`held_out_labels` in `crates/api/tests/b1_api.rs`) commits every real
corpus note through the real gate/vault path, puts adversarial labels
under a nested vault path, and proves two cold rebuilds cannot retrieve
the canary markers while a real corpus note's own content is found. This
is a narrow path-isolation check, not a universal indirect-copy detector.

`test/labels.jsonl`'s 31st row (`id: canary-isolation-probe`,
`abstain_expected: true`) is this mechanical isolation fixture, not a
real evaluation query — it carries both `HELDOUT_QUESTION_CANARY_901`
and `HELDOUT_ANSWER_CANARY_902` markers that must never surface from a
rebuilt index. The other 30 test-split rows are real evaluation queries.

## Provenance

Every corpus note's provenance is in `corpus-provenance.json`: for
project-history notes (decision/bug/pattern/session), a `docs/` path
and/or a continuity decision id with its exact text copied in (frozen at
build time, not a live lookup); for code notes, the real `repository`,
`commit`, `path` and `symbol`. Code notes cite two frozen source repos:
SuperBrain itself, pinned at the commit recorded in each note's own
`corpus-provenance.json` entry (the last commit to touch the quoted
source file at authoring time — not `corpus-manifest.json`'s
`build_commit`, which only describes when the manifest itself was last
regenerated), and `controllore` pinned at
`ec274f4680c19f5b335a01a7a563c8f64546e7e2`. Neither source repo is ever
modified by this corpus.

## Checks

Run from the repository root:

```text
python -B -m unittest discover -s eval/rag -p "test_*.py" -v
cargo test -p api held_out_labels -- --exact
```

`validate_corpus.py` and `build_manifest.py` both import `runner` with a
bare `from runner import load_labels`, which only resolves when
`eval/rag/` itself is on `sys.path` — `eval/rag/` is not a package (no
`__init__.py`), so a dotted `eval.rag.validate_corpus` import fails with
`ModuleNotFoundError: No module named 'runner'`. Run either script from
inside `eval/rag/`, or add that directory to `sys.path` first:

```text
python -c "import sys; sys.path.insert(0, 'eval/rag'); from validate_corpus import validate_corpus; from pathlib import Path; print(validate_corpus(Path('eval/rag/corpus'), Path('eval/rag')))"
```

## What's still ahead (B3.3+)

B3 must still build a metrics runner (Recall@10/nDCG@10 with confidence
intervals), freeze thresholds and configuration on the dev/calibration
split before viewing test results, record commit/date/dataset/query
hashes with protocol and rationale, save raw measurements and
uncertainty intervals, evaluate retrieval/citation/abstention separately,
invalidate eval caches, keep sessions/output out of normal memory, and
repeat contamination checks after a prior benchmark run. A threshold
changed after seeing held-out results invalidates that run as
independent evidence. Nothing in `eval/rag/` as of B3.2 creates a
threshold or a final benchmark claim.
