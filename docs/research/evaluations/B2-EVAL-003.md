# B2-EVAL-003: multi-codebase validation, Backend/API domain

**Evaluation ID:** B2-EVAL-003
**Status:** **COMPLETE.** Resumed after [B2-ID-006](../findings/B2-ID-006.md)
and [B2-ID-007](../findings/B2-ID-007.md) were both approved, implemented,
mutation-closed, and independently reviewed (CLOSED, 2026-11-06). All
`Calls`-derived metrics below are now measured against the corrected
pipeline (fix for B2-ID-006 in the working tree, not yet merged to
`main`). Per explicit instruction, the original contaminated values are
preserved in full alongside the corrected re-measurement, the finding
that invalidated them, the before/after difference, and an explicit
statement of which conclusions changed (none) — no experimental history
was deleted.
**Date:** 2026-11-06 (original pause), resumed and completed 2026-11-07.
**SuperBrain commit (original pause):** `7c7d57e` (B2-EVAL-002's own
closing commit; B2-ID-006/007 fixes not yet applied).
**SuperBrain commit (resumed, corrected run):** working tree atop
`7c7d57e` on branch `b2-code-graph`, containing the B2-ID-006 fix
(`SymbolKind::can_originate_calls` + nested-scope span boundary in
`resolve_calls`) and the B2-ID-007 fixes
(`resolve_extends_implements`'s node-kind boundary,
`resolve_tests_for`'s cross-snapshot span check) — both CLOSED by
independent review, not yet committed/merged. `cargo test --release -p
codegraph-index`: 378/378 passing, 0 failed, confirmed immediately
before the corrected run below.
**External repository:** `nsidnev/fastapi-realworld-example-app`
(https://github.com/nsidnev/fastapi-realworld-example-app), MIT license
(confirmed by direct read of the repository's own `LICENSE` file, root
of the repository, "Copyright (c) 2019 Nik Sidnev"). Chosen per the
user's explicit Backend/API domain requirements: real route → service
→ data-layer call chains (FastAPI `APIRouter` routes, a `services/`
layer, a `db/repositories/` layer over PostgreSQL via `asyncpg`),
dependency injection via FastAPI's own `Depends(...)`, Pydantic
request/response schemas, real multi-file package structure suited to
measuring the Module/File→top-level-symbol gap. Archived (no longer
actively maintained) — treated as a feature, not a defect, for a
frozen, stable evaluation corpus. Re-cloned read-only into a fresh,
isolated scratchpad directory for this resumed run; deleted immediately
after this evaluation's data was captured (confirmed deleted; no
residual clone left on disk).
**External commits (unchanged from the original pause, same protocol):**
commit A `c1f4042746c293af4bb70b351a858e52df2193f8` ("Bump alembic from
1.7.1 to 1.7.4 (#211)"), commit B
`fa56dbac0cdc7014cc51608702e3d370d73249b3` ("Use Pydantic's BaseSettings
for project configuration (#219)") — a real, substantial multi-file
commit pair (14 files changed, 203 insertions/71 deletions in `app/`,
including a brand-new `app/core/settings/` package), chosen specifically
to exercise incremental indexing against genuine file additions plus
cross-module changes, not a trivial one-line diff.
**Configuration:** the frozen baseline configuration from immediately
after Task 9.5 (CALL-RES-A1+A2+A3 for `Calls`; `Contains`/`Instantiates`
as introduced by Task 9.5; `ImportFact`-based Imports per
[B2-ID-005](../findings/B2-ID-005.md)), PLUS the B2-ID-006/007 fixes for
this resumed run. No OTHER algorithm was changed between the original
pause and this resumed run — the only code difference between the
"contaminated" and "corrected" columns below is the B2-ID-006/007 fix
itself.
**Methodology:** drove the real production pipeline
(`RepositorySource` → parse → extract → promote → lineage → edges →
`CodeGraph` → incremental reindex) via the same domain-specific
throwaway harness used at the original pause,
`crates/codegraph-index/examples/b2_smoke_test_backend_api.rs` (compiled
unmodified against the current, fixed working tree — no change was
needed to the harness itself, only to the production resolvers it
calls). All numbers below are **Measured** unless explicitly marked
otherwise. Raw output for the resumed, corrected run is preserved at
`docs/research/evidence/b2-eval-003-resumed-corrected-run.txt`.

## 0. Contamination disclosure — what changed between the original pause and this resumed evaluation, and why

This evaluation's own manual validation of a `Calls` sample at the
original pause (2026-11-06) discovered
[B2-ID-006](../findings/B2-ID-006.md): `resolve_calls` was attributing
real call sites to every enclosing container snapshot (e.g. a class),
not only the nearest real executable caller (the specific method). The
same systematic audit opened [B2-ID-007](../findings/B2-ID-007.md),
which found the identical structural defect pattern — unconditional
recursion into a re-parsed `signature_text` with no boundary at a
nested symbol's own span — also present in `resolve_extends_implements`
and `resolve_tests_for` (confirmed on Python/C#; this specific
repository's `TestsFor` = 0 result is NOT an instance of B2-ID-007's
trigger shape — see Section D below, unchanged from the original
pause).

Both findings were approved, implemented with 25 new regression tests
across the three affected resolvers, mutation-closed (0 genuine missed
mutants in any of the three resolvers' own fix code), and independently
reviewed with a combined APPROVE verdict (2026-11-06) before this
evaluation was authorized to resume.

**Per explicit instruction: no contaminated `Calls` count from the
original pause is reused as valid anywhere below.** Every metric this
defect could affect is shown as a **Before (contaminated) → After
(corrected)** pair, with the delta and an explicit statement of whether
any conclusion changed.

## A. Symbol / structural coverage — NOT affected by B2-ID-006/007

Unchanged between the original pause and this resumed run (symbol
extraction/promotion runs before any of the three affected resolvers;
B2-ID-006/007 touch `resolve_calls`/`resolve_extends_implements`/
`resolve_tests_for` only, never extraction or promotion).

| Metric | Value |
|---|---:|
| Total files in tree | 125 |
| Files supported by parser | 95 |
| Files parsed | 95 |
| Parse failures | 0 |
| Files with tree-sitter error nodes | 0 |
| Raw candidates extracted | 255 |
| Extraction rejections (oversized name/signature) | 1 |
| Snapshots promoted | 255 |
| Promotion failures | 0 |
| `Class` count | 58 |
| `Field` count | 12 |
| `Function` count | 185 |
| Python files with ≥1 import statement | 69 |

No parse failures, no promotion failures, no duplicate
`SymbolSnapshotId` or duplicate `(from,to,kind)` edge triples detected
in the corrected run either (the harness's own built-in diagnostic
checks printed no warnings).

## B. Module/File → top-level-symbol gap (Backend/API domain stress case) — NOT affected by B2-ID-006/007

Unchanged (depends only on extraction/span data, not on any of the
three affected resolvers).

| Metric | Value |
|---|---:|
| Files with at least one extracted symbol | 67 |
| Total top-level symbols across all files (span-nesting definition: a candidate with no OTHER candidate's span strictly containing it) | 196 |
| Files with more than one top-level symbol | 39 |

Top 10 files by top-level-symbol count:

| File | Top-level symbols |
|---|---:|
| `tests/test_api/test_routes/test_articles.py` | 18 |
| `app/db/repositories/articles.py` | 14 |
| `app/db/migrations/versions/fdf8821871d7_main_tables.py` | 11 |
| `tests/conftest.py` | 9 |
| `app/api/dependencies/authentication.py` | 7 |
| `app/db/queries/tables.py` | 7 |
| `tests/test_api/test_routes/test_profiles.py` | 7 |
| `tests/test_api/test_routes/test_users.py` | 7 |
| `app/models/schemas/articles.py` | 6 |
| `app/api/routes/articles/articles_resource.py` | 5 |

**Interpretation (Inferred, unchanged conclusion):** 39/67 files (58.2%
of files with at least one symbol) have more than one top-level symbol,
meaning any query shaped "what does file X contain" or "which symbols
live in module Y" cannot be answered by any existing graph edge today —
only by a file-path string match outside the graph model, for a
majority of files in a real backend repository with this architecture.
This is the first real, quantified evidence point toward the user's
requested Module/File-node decision, though a single backend repository
alone is not sufficient to close that decision generally. **This
conclusion is unaffected by B2-ID-006/007** — the gap is about missing
`Contains`-equivalent structure at the file level, not about any
`Calls`/`Extends`/`TestsFor` edge count.

## C. Edge quality — Before (contaminated) → After (corrected)

| Metric | Before (contaminated, 2026-11-06) | After (corrected, 2026-11-07) | Delta | Status |
|---|---:|---:|---:|---|
| **`Calls` edges** | 326 | **293** | **−33** | Corrected. All 33 confirmed spurious (container/nested-origin duplicates), zero new edges introduced — full edge-level diff, not a sample, preserved at `docs/research/evidence/b2-id-006-backend-before-after-edge-diff.txt` |
| `Extends`/`Implements` edges | 42 | 42 | 0 | Unaffected — this specific repository's inheritance graph has no nested-type-declaration shape for B2-ID-007's `resolve_extends_implements` fix to catch (confirmed: fix changed 0 edges on this corpus, though the resolver itself was still vulnerable in principle before the fix, per B2-ID-007's own Python/C# reproduction on synthetic source) |
| `TestsFor` edges | 0 | 0 | 0 | Unaffected — root cause is the naming-convention mismatch (Section D), not B2-ID-007's nested-declaration trigger shape, which this repository does not happen to contain |
| `Contains` edges | 45 | 45 | 0 | Confirmed structurally immune per B2-ID-007's audit (already innermost-only by design) |
| `Instantiates` edges | 6 | 6 | 0 | Confirmed structurally immune per B2-ID-007's audit (kind-filtered to `Field` only, never a container) |
| `StaticAnalysis` (resolution method, across all edge kinds) | 414 | **384** | −30 | Corrected. Tracks the `Calls` reduction (33 removed) minus a small shift in `NameHeuristic` count below — exact accounting: 414 − 384 = 30, vs. 33 Calls removed, because 3 of the removed edges were themselves `NameHeuristic`-resolved, not `StaticAnalysis` (see next row) |
| `NameHeuristic` (resolution method) | 5 | **2** | −3 | Corrected. 3 of the 33 removed container-originated `Calls` edges had been resolved via the heuristic path, not static analysis |
| Full index wall-clock | 232.394ms | 200.98ms | — | Both single-run measurements; within expected run-to-run variance for this harness (see B2-EVAL-002's own stated limitation on single-run timing) |

**Precision/recall/F1 on the manual sample — now computed (not
deferred), per this resumption's explicit instruction, methodologically
valid because the known contamination source has been fixed and
verified absent from this specific sample (see Section D):**

| Metric | Value | Basis |
|---|---:|---|
| Sample size | 30 | Deterministic, sorted by evidence note — same selection methodology as B2-EVAL-001/002 and the original pause |
| TP (target-correct AND caller-correct) | 30 | All 30 edges independently re-verified against real source at commit B (see Section D for the specific lines checked) |
| FP | 0 | Zero container-originated or otherwise spurious-caller edges found in this sample — the exact defect class this sample previously surfaced (B2-ID-006) is now confirmed absent from it |
| FN | 0 (within sample scope) | Spot-checked known receiver-qualified call chains outside the 30-sample itself (`check_username_is_taken`/`check_email_is_taken` → `repo.get_user_by_username`/`get_user_by_email`, both correctly resolved — see Q5 in Section H) found no missed call site; a full exhaustive FN count across the whole 293-edge corpus was not attempted (would require manually enumerating every real call site in all 95 parsed files, out of scope for a sample-based check) |
| Precision (on sample) | 30/30 = **100%** | TP / (TP + FP) |
| Recall (on sample, scoped) | Not computed as a corpus-wide number | Per the Evidence-before-claims Experimental Policy, recall requires a denominator of ALL real call sites, which this sample-based methodology does not establish; the 0 FN figure above is a scoped spot-check, not a corpus-wide recall measurement — reported honestly as not computed, rather than presented as a number it is not |
| F1 | Not computed | Depends on a valid recall figure, which is not available per above |

**Why precision is now methodologically valid where it was deliberately
withheld at the original pause:** the original pause explicitly declined
to compute precision because the manual sample itself was what
discovered the contamination — computing a number from a sample that
had just revealed a known defect would have presented a contaminated
metric as final. That defect is now fixed, verified fixed on THIS
specific sample (0/30 container-originated edges, vs. 2/30 at the
original pause), and independently reviewed closed. Computing
precision now, on the corrected pipeline, is not the same act the
pause's policy was protecting against.

## D. Manual call-quality sample — re-verified against the corrected pipeline

The same deterministic 30-edge sample selection (sorted by evidence
note) was re-generated against the corrected pipeline and re-checked
against the real source at commit B.

- **30/30 sampled edges: target-correct, caller-correct.** Every edge
  in the corrected sample was individually cross-checked against real
  source, including the two specific lines that previously produced
  the spurious `CommentsRepository -> Comment` and `UsersRepository ->
  EntityDoesNotExist` container-level duplicates
  (`app/db/repositories/comments.py` line 14, `class CommentsRepository`,
  and `app/db/repositories/users.py` lines 9-18, `class UsersRepository`
  through `get_user_by_username`, both confirmed by direct read): those
  two container-level entries are **confirmed absent**
  from the corrected sample; the real method-level edges
  (`_get_comment_from_db_record -> Comment`, `get_user_by_email/
  get_user_by_username -> EntityDoesNotExist`) are present and correct,
  unchanged from the original pause's own correct half of the sample.
- Representative re-checks performed directly against real source for
  this resumption (not merely inherited from the original pause):
  `_get_article_from_db_record` (confirmed a real method on
  `ArticlesRepository`, called from 4 sites including
  `get_article_by_slug`), `EntityDoesNotExist` (confirmed the real
  exception class, raised at both `comments.py`'s and `users.py`'s own
  not-found branches), `FakeAsyncPGPool.acquire` (confirmed a real
  method returning a real `FakePoolAcquireContent` construction),
  `check_username_is_taken`/`check_email_is_taken` →
  `repo.get_user_by_username`/`get_user_by_email` (confirmed real
  receiver-qualified call sites in `app/services/authentication.py`,
  `check_username_is_taken` at lines 5-11 and `check_email_is_taken` at
  lines 14-20, the same receiver-resolution shape as Q5 below).
- **No false target (wrong symbol resolved) was found anywhere in this
  30-edge sample**, same conclusion as the original pause — the fix
  changed WHICH snapshot owns a call, never WHICH target a call
  resolves to.

**`TestsFor` = 0, root cause unchanged by B2-ID-006/007 (Measured +
Manually validated, re-confirmed on the corrected pipeline):** this
repository DOES have a substantial `tests/` directory, yet zero
`TestsFor` edges are produced, identically before and after the fix.
Direct source reading confirms why: `resolve_tests_for`'s
naming-convention matcher requires a test named `test_<X>` to imply
target name exactly `<X>`
(`test_user_can_update_own_profile` → implied target
`user_can_update_own_profile`, which does not exist as a real symbol
anywhere in this corpus) — this repository's tests use descriptive,
BDD-style names, not the `test_add`→`add` convention the resolver's own
module doc comment describes as its design target. **This is NOT an
instance of B2-ID-007's nested-declaration trigger shape** (which
requires an outer container's OWN name to match the convention AND
something nested to call the exact implied name) — it is the
separate, already-identified naming-convention mismatch flagged at the
original pause. **Classified here as a genuine semantic-coverage gap,
Moderate severity**, unchanged classification from the original pause.
Per explicit instruction for this resumption ("do not auto-fix moderate
semantic gaps during this evaluation"), this gap is reported, not
fixed, and not newly opened as its own `B2-ID-xxx` — consistent with
the original pause's own decision not to open a separate finding file
for it mid-pause.

## E. Imports — extraction coverage vs. resolution accuracy — NOT affected by B2-ID-006/007

Unchanged between the original pause and this resumed run (Imports
resolution is a wholly separate code path from `resolve_calls`/
`resolve_extends_implements`/`resolve_tests_for`).

| Metric | Value |
|---|---:|
| Real import statements (line-based heuristic count, commit B) | 381 |
| `ImportFact`s extracted (file-level, Python) | 489 |
| Extraction coverage | 489/381 (128.3% — see interpretation below) |
| Total `ImportFact`s, all languages | 489 |
| Resolution: `Internal` | 192 |
| Resolution: `External` | 284 |
| Resolution: `Ambiguous` | 13 |
| Aliases captured | 6 |

**Interpretation (Inferred, unchanged, consistent with
[B2-ID-005](../findings/B2-ID-005.md)'s already-documented
measurement-definition mismatch):** extraction coverage exceeds 100%
because a single multi-line `from X import (A, B, C)` statement is one
"line-based statement" in the denominator but produces multiple
`ImportFact`s (one per imported name) in the numerator. Not a defect.
No false positives on resolution.

## F. Incremental indexing (A → B, real multi-file commit) — Before → After

| Metric | Before (contaminated) | After (corrected) | Delta |
|---|---:|---:|---:|
| Changed files (`git diff A..B`) | 22 | 22 | 0 (unaffected — file-change detection is independent of edge resolution) |
| Files reparsed | 22 | 22 | 0 |
| Snapshots removed | 29 | 29 | 0 (symbol-level, unaffected by B2-ID-006/007) |
| Snapshots added | 42 | 42 | 0 |
| Edges rebuilt | 448 | **415** | −33 (tracks the same `Calls` reduction as the full-index run) |
| Raw import statements extracted (this call) | 124 | 124 | 0 |
| Raw import statements reused (this call) | 365 | 365 | 0 |
| Import facts resolved (always full recompute, by design) | 489 | 489 | 0 |
| Full index (commit A) wall-clock | 170.4909ms | 155.4553ms | — (single-run timing, not benchmark-grade) |
| Incremental index (A→B) wall-clock | 89.3048ms | 83.9899ms | — (single-run timing) |
| Final snapshot count | 284 | 284 | 0 |
| Final edge count | 419 | **386** | −33 (same `Calls` correction, propagated through the incremental path) |
| Final lineage edge count | 29 | 29 | 0 |

**Interpretation:** the `Calls` correction propagates identically
through both the full-index and incremental-index code paths (−33 in
both), confirming — consistently with B2-ID-006's own independent
review finding on the `pytorch/examples` corpus — that
`incremental_reindex` always recomputes typed edges against the full
current snapshot pool rather than caching a stale, pre-fix edge set.
No `DuplicateSnapshotId`/`DuplicateEdge` panics, no stale-state
warnings, in either the original pause's run or this corrected run.

## G. Performance

| Metric | Value | Status |
|---|---:|---|
| Full index wall-clock | 200.98ms (corrected, this harness's own full pipeline including diagnostics) / 155.4553ms (via `CodeGraphIndexer::observe_commit`, commit A alone) | Measured, single run |
| Incremental index wall-clock | 83.9899ms | Measured, single run |
| Memory | 8,180 KB ≈ 7.99 MB (Windows `tasklist`, coarse) | Estimated |
| Graph size (final, commit B) | 284 snapshots, 386 edges, 29 lineage edges | Measured |
| `ImportFact`s (final) | 489 | Measured |

Single-run measurements, not multi-run averages — consistent with
B2-EVAL-002's own stated limitation, not benchmark-grade. The ~15%
wall-clock reduction (232ms→201ms full index; 170ms→155ms via the
indexer) is consistent with resolving and materializing 33 fewer edges,
not treated as a benchmark-grade performance claim given the single-run
methodology.

## H. Backend/API coding-agent usefulness Q&A — re-run against the corrected pipeline

All 7 questions re-asked against the corrected `CodeGraph`
(`observe_commit(B)`'s final graph, built from the fixed resolvers).

| # | Question | Answer (observed, corrected pipeline) | Compared against the original pause's own answer text (B2-EVAL-003.md, pre-resume revision) |
|---|---|---|---|
| Q1 | From which route does user registration start, and what does it call? (`app/api/routes/authentication.py::register`) | `["schemas/users.py::UserInResponse", "services/authentication.py::check_username_is_taken", "jwt.py::create_access_token_for_user", "repositories/users.py::create_user", "services/authentication.py::check_email_is_taken", "dependencies/database.py::get_repository", "schemas/users.py::UserWithToken"]` — **Fully answerable**, all 7 confirmed correct against real source (`app/api/routes/authentication.py` lines 62-93) | Same 7 callees as a set, order differs (`HashMap`/set iteration order, not a content change). **Caveat:** this comparison is against the prior revision's own quoted text carried over narratively while editing this document, not against a separately preserved raw-output artifact — no diffable file from the original pause's own run exists; treat "unchanged" here as the author's direct text comparison, not an independently reproducible diff |
| Q2 | What does the login route call, and does it reach the data layer? (`app/api/routes/authentication.py::login`) | `["schemas/users.py::UserInResponse", "repositories/users.py::get_user_by_email", "jwt.py::create_access_token_for_user", "models/domain/users.py::check_password", "dependencies/database.py::get_repository", "schemas/users.py::UserWithToken"]` — **Fully answerable**, confirmed correct, reaches the data layer via `get_user_by_email` | Same 6 callees as a set, order differs. Same caveat as Q1 — narrative text comparison, no preserved raw artifact from the original pause |
| Q3 | What does `UsersRepository` (users data-layer component) extend? | `["repositories/base.py::BaseRepository"]` — **Fully answerable**, confirmed correct | Identical. `Extends`/`Implements` edge count for this corpus is independently confirmed unchanged by Section C's measured edge count (42 → 42), not merely a text comparison |
| Q4 | If I modify `check_username_is_taken` (service layer), which route handlers are impacted? | `["routes/users.py::update_current_user", "routes/authentication.py::register"]` — **Fully answerable**, confirmed correct against real source (both call sites independently re-verified) | Identical as a set. Same narrative-comparison caveat as Q1/Q2 |
| Q5 | Does `check_username_is_taken` reach the database layer, and through which method? | `["repositories/users.py::get_user_by_username"]` — **Fully answerable**, confirmed correct, demonstrates receiver-qualified resolution on a NON-`self` receiver | Identical. Same caveat |
| Q6 | Which tests cover the users data layer or authentication routes? | `TestsFor` edges in corpus: 0; `TestsFor` edges incoming to `UsersRepository`: `[]` — **Not answerable**, root-caused in Section D (unchanged) | Identical. Independently confirmed unchanged by Section C's measured `TestsFor` edge count (0 → 0), not merely a text comparison |
| Q7 | Who depends on (calls) `get_current_user_authorizer` (the auth DI dependency)? | 16 real callers listed, spanning routes AND dependency functions — **Fully answerable**, re-spot-checked against real source, all confirmed; `Depends(get_current_user_authorizer())` correctly treated as a real call site | Same 16 callers as a set, order differs. Same narrative-comparison caveat as Q1/Q2/Q4/Q5 — no preserved raw artifact from the original pause's own Q7 run exists to diff against mechanically |

**Summary (out of 7 questions with a ground-truth-checkable answer):**
6 Fully answerable (Q1, Q2, Q3, Q4, Q5, Q7), 1 Not answerable but
root-caused (Q6), 0 Partially answerable, 0 Incorrect. Identical
classification to the original pause for every one of the 7 questions.
**Evidentiary note, added during independent review:** the original
pause's own per-question contamination check (its own Section H) had
already stated that none of these 7 specific answers were built from a
container-originated edge, and the "no change" cells above compare the
corrected run's output against that prior revision's own quoted answer
text directly. This is a real text comparison, not a fabricated claim —
but it is NOT backed by a separately preserved raw-output file from the
original pause's own run (no such file exists in
`docs/research/evidence/`), so it should be read as "unchanged by
direct comparison against the previously published answer text,"
not as "independently re-derived from a diffable artifact." Q3 and Q6
are the two exceptions with independent, measured confirmation (Section
C's edge counts), not merely text comparison.

## I. Research integrity notes

- This evaluation's own manual sampling at the original pause is what
  discovered B2-ID-006 — recorded, at the time and again here, as a
  POSITIVE outcome of the discipline the Evaluation & Paper Evidence
  Policy requires, not minimized because it paused the suite.
- Per this resumption's explicit instruction, no contaminated `Calls`
  count is reused as valid anywhere in this document — every affected
  metric is shown as an explicit Before→After pair with the finding
  that invalidated the Before value named directly.
- The `TestsFor` = 0 result is recorded in full, with its root cause,
  unchanged conclusion from the original pause, and an explicit check
  (Section D) confirming it is NOT an instance of B2-ID-007's own
  nested-declaration trigger shape — a different thing that could have
  been confused for the same root cause but was checked, not assumed.
- No repository/commit was excluded or re-chosen to produce a more
  favorable result; this resumed run used the EXACT same repository,
  commit pair, and harness as the original pause — the only variable
  changed was the production code itself (the approved, reviewed
  B2-ID-006/007 fixes).
- Precision (100% on the 30-edge sample) is reported with its scope
  stated explicitly (sample-based, not corpus-wide) and recall/F1 are
  explicitly reported as NOT computed, rather than estimated or
  assumed, per the Evidence-before-claims Experimental Policy.

## J. Cleanup

The temporary clone used for this resumed, corrected run
(`scratchpad/b2-eval-003-resume/repo`, a fresh, isolated, empty
directory, never shared with any script or other downloaded content)
was deleted after this evaluation's harness output was captured and
preserved. The repository identity, commit pair, license, and SPDX
reproducibility data (above) are retained; the raw corrected-run output
is preserved at
`docs/research/evidence/b2-eval-003-resumed-corrected-run.txt`.

## Residual limitations of this evaluation

- Domain/language coverage for the multi-codebase suite remains at 2
  repositories (ML/Python+C++ from B2-EVAL-001/002; Backend/API Python
  from this evaluation) — still short of the user's full required
  domain list; more domains are needed before any general,
  cross-domain conclusion.
- Recall and F1 on the manual call-quality sample are explicitly NOT
  computed (Section C) — only precision, on a 30-edge sample, is
  reported as methodologically valid; a corpus-wide recall figure would
  require manually enumerating every real call site across all 95
  parsed files, which was out of scope for this resumption.
- `resolve_extends_implements`'s and `resolve_tests_for`'s own
  B2-ID-007 fixes produced a measured delta of ZERO on this specific
  corpus (neither resolver's affected trigger shape happens to occur in
  this repository) — this evaluation does NOT independently re-confirm
  B2-ID-007's own cross-language fix correctness (that validation lives
  in B2-ID-007 itself, with its own dedicated regression tests and
  mutation campaigns); this evaluation only confirms that applying both
  fixes together changed this corpus's edge counts by exactly the
  amount B2-ID-006 alone predicts, with zero unexpected side effects
  from B2-ID-007's code being present in the same binary.
- The B2-ID-006/007 fixes are in the working tree on branch
  `b2-code-graph`, not yet committed or merged to `main` — this
  evaluation's own "corrected" column reflects that uncommitted state,
  consistent with [d104xe]/[d107ad]'s own recorded status (fixes
  implemented and independently reviewed CLOSED, but B2-EVAL-003 itself
  was the one explicitly gating further multi-codebase work, not a
  commit/merge decision, which remains separately pending).
- No new `B2-ID-xxx`/`B2-COV-xxx`/`CRC-xxx` finding was opened during
  this resumed evaluation — no new Critical/Important defect and no new
  significant FP/FN was found beyond what B2-ID-006/007 already
  disclosed, and the one pre-existing Moderate gap (`TestsFor` naming
  convention) was, per instruction, reported again rather than
  auto-fixed.
