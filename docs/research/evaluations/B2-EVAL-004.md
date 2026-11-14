# B2-EVAL-004: multi-codebase validation, Frontend/UI TypeScript/TSX domain

**Evaluation ID:** B2-EVAL-004
**Status:** **COMPLETE.**
**Date:** 2026-11-07.
**SuperBrain commit:** working tree atop `7c7d57e` on branch
`b2-code-graph`, same post-B2-ID-006/007 state B2-EVAL-003 used (fixes
implemented, independently reviewed CLOSED, not yet committed/merged).
No production code was changed during this evaluation — only a new,
domain-specific throwaway harness was added
(`crates/codegraph-index/examples/b2_smoke_test_frontend_ts.rs`).
`cargo test --release -p codegraph-index`: unchanged from B2-EVAL-003's
own last confirmed run (378/378 passing), re-confirmed immediately
before this evaluation's run.
**External repository:** `yoonge/conduit-react`
(https://github.com/yoonge/conduit-react), MIT license (confirmed by
direct read of the repository's own `LICENSE` file, root of the
repository, "Copyright (c) 2023 PDM"). A real React 18 + TypeScript
frontend implementing the RealWorld/Conduit spec (a forum-style app:
topics, comments, tags, auth), chosen per the user's explicit
Frontend/UI TypeScript domain requirements: real `.tsx` function
components (`React.FC<Props>`), real custom-hook-shaped state access
(via the `hox` library's `createStore`/`createGlobalStore`), real
props/children composition (`Homepage` -> `ListPage` -> `ListView` ->
`ListItem`), real multi-file imports, and genuinely more than a toy
line count (36 real TS/TSX files parsed, 71 files in the tree total,
86 real commits of actual history). Not a component-library demo or a
single-file tutorial repository. Cloned read-only into a fresh,
isolated scratchpad directory; deleted after this evaluation's data
was captured (confirmed deleted, no residual clone left on disk).
**External commits:** commit A `b88088b2e5d879fe6f049dce483fe0d1c319efc9`
("Merge pull request #28 from yoonge/dev"), commit B
`56e1aa36b97418cbcfc19cf2c40136093a496af7` ("feat: implement `Tags
Cloud` feature") — a real, substantial multi-file commit (10 files
changed: `app/components/Header`, `ListItem`, `main.tsx`, `TagPage`,
a brand-new `src/pages/Tags/index.less` stylesheet, plus
`package.json`/`pnpm-lock.yaml`/`README.md`), chosen to exercise
incremental indexing against a genuine cross-component feature
addition, not a trivial one-line diff.
**Configuration:** the frozen baseline configuration, unchanged from
B2-EVAL-003 (CALL-RES-A1+A2+A3 for `Calls`, post-B2-ID-006/007;
`Contains`/`Instantiates` as introduced by Task 9.5; `ImportFact`-based
Imports per [B2-ID-005](../findings/B2-ID-005.md)). No algorithm was
changed to produce this evaluation's numbers — this is the FIRST
real-repository exercise of this exact configuration against TypeScript/
Tsx source specifically (B2-EVAL-001/002 were Python/C++; B2-EVAL-003
was Python).
**Methodology:** drove the real production pipeline (`RepositorySource`
-> parse -> extract -> promote -> lineage -> edges -> `CodeGraph` ->
incremental reindex) via a new, domain-specific throwaway harness,
`crates/codegraph-index/examples/b2_smoke_test_frontend_ts.rs` —
structurally identical to `b2_smoke_test.rs`/`b2_smoke_test_backend_api.rs`
for every generic measurement section, with the Q&A section (H)
rewritten for this repository's own component/hook/store architecture,
and a new Section C1 added specifically for this domain: a raw
tree-sitter node-kind scan (independent of the extraction/promotion
pipeline) counting constructs the pipeline does NOT currently promote
as a symbol, so "zero promoted" can be told apart from "zero present in
the corpus" for TS-specific constructs (`interface_declaration`,
`type_alias_declaration`, JSX element references, decorators,
namespaces, export shapes). All numbers below are **Measured** unless
explicitly marked otherwise. Raw output preserved at
`docs/research/evidence/b2-eval-004-conduit-react-run.txt`.

## A. Symbol / structural coverage

| Metric | Value |
|---|---:|
| Total files in tree | 71 |
| Files supported by parser | 36 |
| Files parsed | 36 |
| Parse failures | 0 |
| Files with tree-sitter error nodes | 0 |
| Raw candidates extracted | 52 |
| Extraction rejections (oversized name/signature) | 6 |
| Snapshots promoted | 52 |
| Promotion failures | 0 |
| `Function` count | 50 |
| `Method` count | 2 |
| TS/Tsx files with ≥1 import statement | 28 |

No parse failures, no promotion failures, no duplicate
`SymbolSnapshotId` or duplicate `(from,to,kind)` edge triples detected.

**Immediately visible coverage gap (Measured):** NO `Class`, `Struct`,
`Interface`, `Field`, or `MethodSignature` snapshot was produced
anywhere in this corpus, despite the corpus containing real TypeScript
`interface` declarations (11, confirmed in Section C1) and real class-
shaped component patterns in the broader React ecosystem (this specific
repository happens to use only functional components, confirmed by
direct inspection — zero `class Foo extends React.Component` anywhere
in `src/`). This is NOT evidence the pipeline is broken for class-based
TS/Tsx — it reflects this one corpus's own real coding style (100%
functional components, 0% class components), consistent with the
dominant modern React idiom. The 2 `Method` snapshots are real class
methods (confirmed: `AbortController`-adjacent or similar small class
usages are absent; the 2 methods trace to `src/utils/axios.ts`'s own
interceptor setup, not a user-authored component class — not sampled in
Section D, noted here as structural coverage only).

## B. TS/Tsx-specific semantic coverage — what the current `SymbolKind` set represents well, and what it does not

| Construct | Representable today? | Evidence |
|---|---|---|
| Function components (`const X = () => {...}` / `const X: React.FC<P> = (...) => {...}`) | **Yes** — extracted as `Function` via the arrow-function-assigned-to-a-variable-declarator shape | 50 `Function` snapshots, including every sampled component in Section D/H |
| Named functions (`function f() {...}`) | **Yes** | Covered by the same `function_declaration` path used for JS/TS in B2-EVAL-001/002/003 |
| Class methods | **Yes, where a class exists** | 2 `Method` snapshots found; this corpus has very few classes to exercise it further |
| Props/interfaces declaring a component's props shape (`interface FooProps {...}`) | **No** — `interface_declaration` is parsed by tree-sitter but never promoted to any `SymbolKind` for TypeScript. **Root cause, corrected during independent review:** `kind_mapping.rs`'s own `map_kind_hint` function places NO language gate on `"interface_declaration"` at all (it maps unconditionally to `SymbolKind::Interface`, the same arm C#'s `interface_declaration` also hits) — the real gap is one step earlier, in `extract/javascript.rs`'s own node-kind walk (the shared JS/TS/Tsx extractor), which never visits or emits an `interface_declaration` node as a `RawSymbolCandidate` in the first place, so no `kind_hint` for it is ever produced for this extractor to pass to the mapping table. An initial draft of this document misattributed the gap to the mapping table itself; the extractor's own node-kind walk is the correct and only place this gap lives | 11 real `interface` declarations present in this corpus (Section C1), confirmed by direct source reading to include real, non-trivial prop-shape declarations (`ListPageProps`, `ListViewProps`, `CommentFormProps`, `TopicListStoreProps`, others) — **zero** promoted |
| Type aliases (`type X = ...`) | **No** — same gap, `type_alias_declaration` is never promoted | 5 real `type_alias_declaration` nodes present (Section C1) |
| Hooks exported via simple `const useX = () => {...}` | **Yes** | Not exercised by name in this corpus's own hook exports (see next row) — confirmed representable in principle via the same arrow-function path as any other `const` export |
| **Hooks/stores exported via array-destructuring from a factory call** (`export const [useX, Provider] = createStore(...)`) | **No — a real, now-confirmed gap** | `useTopicListStore`/`TopicListStoreProvider` (`src/stores/topic.ts`) and `useAcountStore`/`getAcountStore` (`src/stores/auth.ts`) are NEVER extracted as symbols at all — the extractor's arrow-function-naming logic only recovers a name from the single-identifier `const name = (...) => ...` shape (`extract/javascript.rs`'s own documented design), and an array-destructuring left-hand side (`const [a, b] = ...`) has no single `identifier` sibling for that logic to find. This is the primary state-management idiom in this entire corpus (both of this repository's own global/scoped stores use this exact export shape) |
| Closures returned from inside a store factory (`const login = (...) => {...}` nested inside `createGlobalStore(() => {...})`) | **Yes, as a nested `Function`** | `login`, `logout`, `update` (`stores/auth.ts`), `fetchTopicList`, `removeTopicFromList`, `updateTopicList` (`stores/topic.ts`) are all correctly extracted as real, independently addressable `Function` snapshots — confirmed by their correct appearance as call TARGETS in Section D's manual sample |
| JSX component composition (`<ListView .../>` inside another component's JSX return) | **No — confirmed, not merely suspected** | `resolve_calls` does NOT treat a JSX element construction as a call site at all (confirmed empirically: Section H's Q1 shows `ListPage` has zero outgoing `Calls` edges to `Header`/`TabsComponent`/`ListView`/`PaginationComp`, all of which it genuinely renders as JSX children) — this pipeline's `Calls` resolver walks for `call_expression`-shaped nodes specifically, and tree-sitter's TSX grammar represents a JSX element as its own distinct node kind (`jsx_element`/`jsx_self_closing_element`), never as a `call_expression`. 313 real JSX element-reference occurrences exist in this corpus (Section C1) and NONE of them currently produces any typed edge of any kind |
| Namespaces (`namespace X {...}`) | **No extractor support** | **0 occurrences in this corpus** (confirmed via Section C1's own scan) — not a gap this corpus can demonstrate either way |
| Decorators | **No extractor support** | **0 occurrences in this corpus** — same caveat |
| Default/named exports, re-exports (`export *`) | **Not separately modeled as a relationship** (an export does not itself create a typed edge; only the exported symbol's own extraction matters) | 26 default exports, 9 named exports, 0 `export *` re-exports, all counted in Section C1; not itself classified as a defect, since this pipeline's model does not claim to represent "what a module exports" as a distinct relationship today (consistent with the Module/File gap, Section C2 below) |

**Severity classification of the two new gaps found (per the Research-
grade Findings Policy, Moderate/Minor handled here, not escalated):**

- **JSX-as-composition is unrepresented (Moderate):** this is the single
  most consequential gap for THIS domain specifically — component
  composition (`parent renders child`) is arguably the single most
  common "impact analysis" question a frontend coding agent would ask,
  and this pipeline currently cannot answer it at all, not even
  partially. Classified Moderate rather than Important/Critical because
  it is a known, deliberate scope boundary (this pipeline's `Calls`
  resolver was designed around `call_expression` nodes across all nine
  supported languages, and JSX is a TSX-grammar-specific construct with
  no equivalent in the other eight languages — not a regression or an
  oversight specific to this task, but a genuine, now-quantified
  absence worth flagging for the deferred structural-node decision this
  evaluation is also gathering evidence for). Not auto-fixed, per
  explicit instruction.
- **Array-destructuring hook/store exports are unrepresented (Moderate):**
  confirmed to make this repository's own two most-used hooks
  (`useAcountStore`, `useTopicListStore`) completely invisible to the
  graph as symbols — any "which components use this hook" or "what does
  this hook return" question about either one is unanswerable today
  not because the resolver gets the wrong answer, but because there is
  no symbol at all for it to reason about. Classified Moderate, not
  Important: it does not corrupt any OTHER measurement (it is a missed
  EXTRACTION, not a wrong RESOLUTION — no spurious edge results from
  it), and the closures returned from inside the factory ARE
  individually extracted correctly (Section D confirms `fetchTopicList`
  resolves correctly as a call target). Not auto-fixed, per explicit
  instruction.
- `interface_declaration`/`type_alias_declaration` non-promotion
  (Minor): consistent with this pipeline's own pre-existing, already-
  documented scope boundary (type-level constructs are not modeled as
  symbols in any language today, not just TypeScript) — not a new
  discovery so much as a now-quantified instance of an already-known
  boundary (11 interfaces, 5 type aliases present and unpromoted in this
  one corpus).

## C. Edge quality

| Metric | Value | Status |
|---|---:|---|
| Lineage edges (commit B alone, no prev) | 0 | Measured |
| **`Calls` edges** | **25** | Measured — see Section D for manual validation; this is the full corpus's own total `Calls` edge count, not a sample size |
| `Extends`/`Implements` edges | 0 | Measured — consistent with Section A's finding of zero class-based components in this corpus; correctly reports zero rather than inventing an edge (confirmed by Q7, Section H) |
| `TestsFor` edges | 0 | Measured — this repository has NO `tests/` directory at all (confirmed by direct file-tree inspection, unlike B2-EVAL-003's backend corpus, which had tests but a naming-convention mismatch); zero is the genuinely correct answer here, not a resolver gap |
| `Contains` edges | 0 | Measured — this corpus produced zero `Class` snapshots (Section A), and `Contains` in this pipeline currently only links a `Class`/container kind to its members; with no container symbols at all, zero `Contains` edges is the structurally correct result, not a defect |
| `Instantiates` edges | 0 | Measured — `resolve_instantiates` is gated on `Field`-kind snapshots (per B2-ID-007's own audit), and this corpus produced zero `Field` snapshots; same structural explanation as `Contains` |
| `StaticAnalysis` (resolution method) | 23 | Measured |
| `NameHeuristic` (resolution method) | 2 | Measured |
| Full index wall-clock | 116.1593ms | Measured, single run |

**Precision/recall/F1 on the manual sample:**

| Metric | Value | Basis |
|---|---:|---|
| Sample size | **25** (the FULL corpus's own `Calls` edge count — fewer than 30 exist in total, so the deterministic "first 30 by evidence note" selection captured the entire corpus, not a subset) | Deterministic, same selection methodology as prior evaluations; explicitly noted here rather than silently reporting a 30-edge sample that does not exist |
| TP (target-correct AND caller-correct) | 23 | Individually re-verified against real source at commit B (see Section D) |
| FP | **2** | Both are the SAME underlying mechanism: a prop destructured by name inside a function component, with an unrelated but same-named real function existing elsewhere in the corpus — `CommentForm::handleSubmit -> TopicDetail::handleCommentSubmit` and `-> TopicDetail::handleToastShow`. Opened as [CRC-002](../challenge-sets/call-resolution-challenges.md#case-crc-002-destructured-prop-with-the-same-name-as-an-unrelated-same-named-function-in-another-file) in the persistent challenge set |
| FN | Not exhaustively counted corpus-wide | Spot-checks found no missed call site among the functions directly inspected (`handleFavor`, `handleSubmit` variants, `TopicDetail`'s own internal calls) — a full exhaustive FN count across all 36 parsed files was out of scope, consistent with B2-EVAL-003's own stated limitation on this point |
| Precision (on sample) | 23/25 = **92.0%** | TP / (TP + FP) |
| Recall (scoped) | Not computed as a corpus-wide number | Same reasoning as B2-EVAL-003: no independently established denominator of ALL real call sites exists for this corpus |
| F1 | Not computed | Depends on an unavailable recall figure |

**This is the first evaluation in the B2 multi-codebase suite where a
real, methodologically-sound sample scored below 100% precision after
the B2-ID-006/007 fixes** — not because those fixes regressed anything
(CRC-002's mechanism is wholly unrelated to container-origination or
nested-scope span boundaries; it is a missing local-binding/shadowing
check in `resolve_calls`'s own candidate search, a different part of
the same resolver), but because this domain's own idiom (props
destructured by name, same name reused as a real function elsewhere)
exercises a scope-blindness the Python/C++/Backend corpora's own call
shapes did not happen to trigger.

## D. Manual call-quality sample — full detail

All 25 edges in the corpus were individually checked against real
source at commit B (`56e1aa3`):

- **23/25: target-correct, caller-correct.** Representative confirmed
  cases: `TopicForm::handleVditorInit`/`blur`/`input` all correctly
  call the real sibling function `checkContentValidity` (same file,
  confirmed at `src/components/TopicForm/index.tsx` lines 77/133/148/
  152/158); `ListPage::ListPage` correctly calls
  `stores/topic.ts::fetchTopicList` (a closure returned from
  `createStore`'s own factory, confirmed reachable and correctly
  resolved despite NOT being directly exported by name — the resolver
  found it as an ordinary nested `Function` snapshot); six independent
  call sites (`Login`/`Register`/`Settings`/`TopicInitiate`/
  `TopicUpdate`/`TopicDetail`'s own `handleSubmit`/`handleCommentSubmit`)
  all correctly call the shared utility `utils/loading.ts::loadingDelay`
  (confirmed: a simple top-level `export const loadingDelay = async
  (...) => {...}`, the ONE arrow-function export shape this extractor
  DOES recognize, unlike the array-destructuring shape documented in
  Section B); `login`/`logout`/`update` (all destructured from
  `useAcountStore()`) correctly resolve to their real definition sites
  inside `stores/auth.ts`'s own `createGlobalStore` factory —
  confirmed correct, but for a reason worth stating plainly: this
  works ONLY because the locally-destructured name happens to be
  spelled identically to the real closure's own name, the exact same
  coincidental-name-match mechanism that produces CRC-002's two false
  positives when the names collide with the WRONG symbol instead of
  the right one.
- **2/25: target-INCORRECT (false positive), caller-correct.**
  `CommentForm::handleSubmit -> TopicDetail::handleCommentSubmit` and
  `-> TopicDetail::handleToastShow` — see Section C and
  [CRC-002](../challenge-sets/call-resolution-challenges.md) for full
  detail. Both are props, not references to the same-named functions
  in `TopicDetail`, which genuinely exist (they are the functions
  `TopicDetail` passes INTO `CommentForm` as those very props) but are
  NOT what these two specific call sites actually invoke at runtime.
- No false negative was found among the specific call sites directly
  inspected during this manual check.

**Why the array-destructuring gap (Section B) does NOT show up as a
`Calls` sample defect:** `useTopicListStore`/`useAcountStore` themselves
are never call TARGETS or SOURCES in this sample because they are never
extracted as symbols at all (Section B) — their ABSENCE is invisible to
a sample built only from edges that exist. This is a real, separate
limitation from the FP/FN axis this sample measures, flagged explicitly
so it is not mistaken for "the hooks resolved correctly" by omission.

## E. Imports — extraction coverage vs. resolution accuracy

| Metric | Value |
|---|---:|
| Real import statements (line-based heuristic count, commit B, `.ts`/`.tsx`) | 189 |
| `ImportFact`s extracted (file-level) | 225 |
| Extraction coverage | 225/189 (119.0% — see interpretation below) |
| Total `ImportFact`s, all languages | 225 |
| Resolution: `Internal` | 7 |
| Resolution: `External` | 218 |
| Resolution: `Ambiguous` | 0 |
| Aliases captured | 66 |

**Interpretation (Inferred, same measurement-definition caveat
B2-ID-005/B2-EVAL-002/003 already documented):** extraction coverage
exceeds 100% for the same reason as the backend corpus — a single
multi-name `import { A, B, C } from 'x'` statement is one line-based
statement but multiple `ImportFact`s. **The External/Internal ratio
here (218:7, 96.9% external) is dramatically more external-heavy than
the Backend/API corpus's own 284:192 split (59.7% external)** — an
Inferred, domain-characteristic difference, not a defect: a small
frontend app imports from `react`, `react-router-dom`, `react-bootstrap`,
`axios`, `hox`, and its own `.less` stylesheets far more than it imports
across its OWN small `src/` tree, compared to a backend app's own
deeper internal `routes`/`services`/`repositories` layering. 66 aliases
captured (`import X as Y`-equivalent and default-import-renamed
patterns) is proportionally much higher than the backend corpus's own 6
— also domain-characteristic (TypeScript/JSX conventionally aliases far
more imports, e.g. renamed default exports per component file, than
Python's own `import x as y` idiom, which is comparatively rare). No
false positive on resolution (no `External`/`Ambiguous` import ever
carries an invented internal target) — confirmed by design per
`ImportResolution`'s own variants, same as prior evaluations.

## F. Module/File -> top-level-symbol gap (Frontend/UI domain stress case)

| Metric | Value |
|---|---:|
| Files with at least one extracted symbol | 25 |
| Total top-level symbols across all files (span-nesting definition) | 36 |
| Files with more than one top-level symbol | 6 |

Top files by top-level-symbol count: `src/pages/TopicDetail/index.tsx`
(4), `src/components/TopicForm/index.tsx` (3), `src/stores/auth.ts` (3),
`src/stores/topic.ts` (3), `src/pages/Register/index.tsx` (2),
`src/pages/Settings/index.tsx` (2).

**Interpretation (Inferred) — how this domain's gap differs from the
Backend/API domain's:** only 6/25 files (24%) have more than one
top-level symbol here, versus 39/67 (58.2%) in B2-EVAL-003's backend
corpus — a real, Measured, domain-characteristic difference: this
repository's own convention is overwhelmingly "one component/one
default-exported function per file" (confirmed directly: every
`src/components/*/index.tsx` and `src/pages/*/index.tsx` file holds
exactly one component), whereas the backend's `repositories.py`/
`schemas.py` files commonly bundle several related classes/functions
together. **This means the Module/File structural-container gap is
LESS acute by this one specific symptom (fewer multi-symbol files to
disambiguate) in a frontend codebase that follows this one-component-
per-file convention** — but the GREATER cost in THIS domain is
different: it is not "which of several symbols does this file contain"
but "which OTHER files/components does this file's own default export
compose with" (the JSX-composition gap, Section B), which a Module/File
node alone would not fix either — that specific limitation needs the
JSX-composition edge, not a structural-container node. **Both
B2-EVAL-003's and this evaluation's own Module/File evidence point to
the SAME underlying missing concept (a first-class container above
individual symbols) mattering for DIFFERENT reasons per domain** — this
is recorded as a cross-domain synthesis point for the still-deferred
Module/File-node decision, not a new implementation decision here.
Not implemented during this evaluation, per explicit instruction.

## G. Incremental indexing (A -> B, real multi-file commit)

| Metric | Value |
|---|---:|
| Changed files (`git diff A..B`) | 10 |
| Files reparsed | 10 |
| Snapshots removed | 8 |
| Snapshots added | 7 |
| Edges rebuilt | 30 |
| Raw import statements extracted (this call) | 49 |
| Raw import statements reused (this call) | 176 |
| Import facts resolved (always full recompute, by design) | 225 |
| Full index (commit A) wall-clock | 69.659ms |
| Incremental index (A->B) wall-clock | 43.7929ms |
| Final snapshot count | 57 |
| Final edge count | 25 |
| Final lineage edge count | 5 |

**Interpretation:** the incremental update correctly reused 176/225
(78.2%) of raw import statements despite 10 files changing, including a
genuinely new stylesheet file (`src/pages/Tags/index.less`, not itself
parsed for symbols but present in the changed-file list) and a
substantive `Header`/`ListItem` component refactor. No
`DuplicateSnapshotId`/`DuplicateEdge` panics, no stale-state warnings —
consistent with every prior B2 evaluation's own incremental-indexing
result.

## H. Frontend/UI coding-agent usefulness Q&A

| # | Question | Answer (observed) | Classification |
|---|---|---|---|
| Q1 | Which component renders/contains `ListView` as a child? | `ListPage` calls/references: `["stores/topic.ts::fetchTopicList"]` — ground truth (`src/components/ListPage/index.tsx`) is that `ListPage` DOES render `<Header/>`, `<TabsComponent .../>`, `<ListView .../>`, and `<PaginationComp/>` directly in its own JSX, confirmed by direct source reading, none of which appear in this answer | **Incorrect** (confidently wrong by omission — the pipeline reports a real but incomplete answer, with no indication the JSX-composition relationships are simply invisible to it, rather than genuinely absent). Root-caused in Section B: JSX element construction is not modeled as a `Calls` edge at all |
| Q2 | Which component(s) use the `useTopicListStore` hook? | `'useTopicListStore' NOT FOUND in src/stores/topic.ts` | **Not answerable**, root-caused in Section B: the array-destructuring export pattern means `useTopicListStore` is never extracted as a symbol at all, so there is no node in the graph for this question to even be asked about |
| Q3 | Where does `Header` get its active-link/auth state from? | `Header calls: []` — ground truth: `Header` calls `useAcountStore()` (internal, but itself unextracted per Q2's same root cause) and `useLocation()`/`useNavigate()` (external, `react-router-dom`) | **Partially answerable at best, arguably Not answerable**: the empty result is technically "correct" in the narrow sense that no callee happens to be a promoted symbol, but it gives a coding agent zero signal rather than surfacing "this calls an unextracted internal hook" — indistinguishable from "this function calls nothing," which is false |
| Q4 | If I modify `fetchTopicList`, which components are impacted? | `["components/ListPage/index.tsx::ListPage"]` (`fetchTopicList` confirmed found as a promoted symbol) | **Fully answerable**, confirmed correct against real source — `fetchTopicList` is the one exception where a closure INSIDE an unextracted hook-factory is still independently reachable and correctly resolved, because the resolver's candidate search is name-based, not container-aware |
| Q5 | What does `ListView`'s `handleClick` event handler call? | `calls: []` | **Correct, and correctly empty** — ground truth: `handleClick`'s only call targets are `e.preventDefault()` (a method on a parameter, not a resolvable top-level symbol) and either `navigate(...)` (external hook-returned function) or `handleTabSelect(...)` (a received prop) — there is genuinely no internal symbol-pool target to find here. This is the CORRECT negative, not a gap, and is recorded specifically to show the pipeline does not invent an edge just because a call expression exists |
| Q6 | Which tests cover the `ListItem` component? | `0` `TestsFor` edges in corpus | **Not answerable, correctly root-caused**: this repository has NO `tests/` directory anywhere (confirmed by direct file-tree inspection) — zero is the genuinely correct answer, not a resolver defect, structurally different from B2-EVAL-003's backend corpus (which HAD tests with a naming-convention mismatch) |
| Q7 | What does `ListPage` extend/implement? | `[]` (expected: empty) | **Fully answerable, and correctly empty** — `ListPage` is a functional component (`React.FC`), not a class; confirms the pipeline does not invent a spurious `Extends` edge for a corpus with zero class-based components |

**Summary (out of 7 questions):** 2 Fully answerable (Q4, Q7), 1
correctly-negative "answerable in the sense that empty is right" (Q5),
2 Not answerable with an honest root cause (Q2, Q6), 1 Partially
answerable/arguably Not answerable (Q3), 1 Incorrect by confident
omission (Q1). **This is the weakest Q&A result in the B2
multi-codebase suite so far** (B2-EVAL-001/002/003 each scored 6/7 or
better Fully-answerable) — directly attributable to the two Moderate
gaps in Section B (JSX composition, array-destructured hook exports)
both landing squarely on this domain's own dominant idioms, not to any
regression in the underlying resolvers (which, aside from CRC-002's
narrow prop-shadowing defect, behaved identically to their validated
behavior on the other two domains).

## I. Research integrity notes

- The 92.0% sample precision (23/25) and the 2/7 "fully answerable"
  Q&A result are both reported plainly, without softening, even though
  they are the weakest quantitative results in the B2 multi-codebase
  suite to date — consistent with the Evaluation & Paper Evidence
  Policy's own research-integrity requirement.
- The two new Moderate gaps (JSX composition, array-destructured hook
  exports) were found by direct, honest root-causing of UNEXPECTED
  empty/wrong Q&A answers (Q1, Q2, Q3), not invented as hypothetical
  concerns — each is backed by a specific file/line citation and a
  confirmed reproduction.
- CRC-002 was opened immediately upon confirming the false positive
  against real source, per the challenge-set framework's own
  "collected during real evaluations, not invented" standard.
- No repository/commit was excluded or re-chosen to produce a more
  favorable result; `conduit-react` was the first and only candidate
  evaluated in depth (a second candidate, an archived
  `react-redux-realworld-example-app` fork family, was considered via
  search but not cloned, since `conduit-react`'s MIT license, real
  TypeScript, non-toy size, and active hook/store usage made it a
  stronger fit on inspection of its own commit history and file tree
  alone).
- Precision is reported with its scope stated explicitly (25-edge
  full-corpus sample, not a subset of a larger population) and
  recall/F1 are explicitly reported as NOT computed, per the
  Evidence-before-claims Experimental Policy.

## J. Cleanup

The temporary clone (`scratchpad/b2-eval-004/repo`) was deleted after
this evaluation's harness output was captured and preserved. The
repository identity, commit pair, license, and SPDX reproducibility
data (above) are retained; the raw run output is preserved at
`docs/research/evidence/b2-eval-004-conduit-react-run.txt`.

## Residual limitations of this evaluation

- Domain/language coverage for the multi-codebase suite is now 3
  repositories (ML/Python+C++; Backend/API Python; Frontend/UI
  TypeScript/TSX) — still short of a fully general, cross-domain
  conclusion, though each new domain has now surfaced at least one
  domain-characteristic finding not visible in the others (B2-ID-006/007
  from the backend domain; CRC-002 and the two Moderate TS/TSX gaps
  from this one).
- This evaluation did not attempt to measure JSX-composition or
  array-destructured-hook-export fixes, since the user's explicit
  instruction was to document, not implement, Moderate/Minor gaps
  during this evaluation.
- The `Extends`/`Implements`/`Contains`/`Instantiates` = 0 results are
  all correctly explained by this ONE corpus's own all-functional,
  no-class, no-field-assignment coding style — this evaluation does NOT
  establish how this pipeline performs on a CLASS-based TypeScript/TSX
  corpus (e.g. older React class components, or a non-React OOP-heavy
  TypeScript codebase); that remains an open question for a future
  evaluation, not resolved here.
- CRC-002's mechanism (local-binding/shadowing blindness in
  `resolve_calls`'s candidate search) was found via this one 25-edge
  sample; its true prevalence across a larger TS/TSX corpus, or across
  the other eight supported languages (a Python function parameter
  shadowing a same-named top-level function elsewhere, for instance,
  would be the same underlying defect family) was not separately
  measured here and is flagged as a candidate for a future, focused
  audit, analogous to how B2-ID-007 systematically audited B2-ID-006's
  own defect family across resolvers rather than assuming it was
  `resolve_calls`-only.
