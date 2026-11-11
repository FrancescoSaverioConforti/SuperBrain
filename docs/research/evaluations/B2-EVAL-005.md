# B2-EVAL-005: multi-codebase validation, Rust/systems domain

**Evaluation ID:** B2-EVAL-005
**Status:** **COMPLETE.** Resumed after [B2-ID-008](../findings/B2-ID-008.md)
(Rust `impl` blocks never represented as symbols, so `Extends`/`Implements`
coverage was structurally zero) was approved (Option B: `ImplFact` as a
non-symbol structural fact), implemented, fixed through a final-review
Critical bug (C1: incremental resolution used a stale snapshot pool,
silently dropping edges on ordinary edits), re-reviewed, and closed. Per
explicit instruction, every pre-fix number measured during the original
pause is preserved below exactly as it was recorded — none deleted,
none silently replaced — with the invalidating finding, the corrected
post-fix number, the before/after delta, and an explicit statement of
which conclusions changed, placed directly alongside each pre-fix value.
**Date:** 2026-11-07 (original pause), resumed and completed 2026-11-11.
**SuperBrain commit (original pause):** working tree atop `7c7d57e`
(pre-B2-ID-008). **SuperBrain commit (resumed, corrected run):** branch
`b2-code-graph` at commit `3d3e151` (B2-ID-008 fully implemented,
final-review fix wave applied and re-reviewed clean — `ImplFact`,
`resolve_implements_from_impl_facts`, the C1 incremental-resolution fix,
and the I1/I3 extraction/resolution-safety fixes are all included).
`cargo test --release --workspace`: green across every crate
immediately before this resumed run (confirmed by the B2-ID-008 final
re-review, not independently re-run as part of this evaluation's own
scope, since no production code was touched here).
**External repository:** `sharkdp/fd`
(https://github.com/sharkdp/fd), dual-licensed MIT
("Copyright (c) 2017-present The fd developers", `LICENSE-MIT`) /
Apache-2.0 (`LICENSE-APACHE`), confirmed by direct read of both license
files at the repository root — unchanged from the original pause.
**External commits:** commit A `b7a0a29d07c3d0e9e0a41155019acada08831a6c`,
commit B `23815624819c77ab665e19c8e6c28bddbcd96e1d` — the same pair the
original pause used, re-cloned fresh into a new, isolated scratchpad
directory for this resumed run (never reusing a path from any prior
clone); deleted after this evaluation's data was captured (confirmed
deleted, no residual clone left on disk).
**Configuration:** the frozen baseline configuration, unchanged except
for the one approved fix this resumption exists to validate ([B2-ID-008](../findings/B2-ID-008.md)'s
`ImplFact` mechanism). `Calls`/`Imports`/`Contains`/`Instantiates`
configurations are byte-for-byte the same as the original pause — no
other algorithm changed.
**Methodology:** drove the real production pipeline via the same
domain-specific harness, `crates/codegraph-index/examples/b2_smoke_test_rust_systems.rs`,
with two additive changes made as PART of this resumption (both
measurement-harness fixes, never production-semantics changes, per
explicit instruction): (1) Section H's Q1/Q2/Q5 now query
`EdgeKind::Implements` instead of the stale `EdgeKind::Extends` they
were querying at the original pause (a pre-existing harness display bug,
not a resolver defect — the production pipeline has always produced the
right `EdgeKind`; the harness was asking the graph the wrong question);
(2) a new, explicit full-vs-incremental `Implements`-edge parity check
was added, running `observe_commit`'s incremental path (A->B) and a
separate, independent full reindex of commit B side by side and
diffing their own `Implements` edge counts and trait-resolution
counts — the production-evaluation counterpart to the C1 fix's own
unit-level regression tests, run here against the real external
repository rather than a synthetic fixture. Raw output for the
corrected, post-fix run is preserved at
`docs/research/evidence/b2-eval-005-fd-run-postfix-final.txt`. The
original pre-fix raw output remains preserved, unmodified, at
`docs/research/evidence/b2-eval-005-fd-run-preliminary.txt`.

## 0. Contamination disclosure — what changed between the original pause and this resumed evaluation, and why

The original pause's own first trait/impl measurement found
`extends_implements_edges: 0` against a corpus independently confirmed
to contain 2 real traits and 41 real top-level `impl` blocks (46
including nested/generic ones, confirmed during the fix's own
corpus-classification work). Direct reading of `extract/rust.rs` and
`edges/extends_implements.rs` confirmed the root cause within that same
session: `impl_item` was never extracted as a symbol at all, so the
resolver's own Rust-specific logic — itself syntactically correct — was
never actually invoked on real Rust source. This was opened as
[B2-ID-008](../findings/B2-ID-008.md), approved for a fix (Option B:
`ImplFact`, a non-symbol structural fact mirroring the already-
precedented `ImportFact` pattern from [B2-ID-005](../findings/B2-ID-005.md)),
implemented across 5 tasks, found to have one Critical bug (C1) and
four Important/three Minor issues during its own final whole-branch
review, fixed in one bundled wave, independently re-reviewed, and
closed.

**Per explicit instruction: no pre-fix `Extends`/`Implements` number is
reused as a currently-valid figure anywhere below.** Every metric this
defect affected is shown as an explicit **Before (pre-fix) -> After
(post-fix)** pair, with the delta and an explicit statement of which
conclusions changed.

**Why the C1 Critical bug matters to THIS evaluation specifically, not
just to the fix's own unit tests:** C1 was "incremental `ImplFact`
resolution uses `final_snapshots` (includes lineage-superseded old
snapshots), so an unchanged trait/type in an edited file produces
`Ambiguous` self/trait resolution and the `Implements` edge silently
disappears" — and this evaluation's OWN corpus is exactly where that
bug was first observed in the wild: the original pause's own 7
`Ambiguous` self-type-resolution cases (see Section D2 below) WERE this
bug, not a benign quirk, discovered by this evaluation's own real-
repository measurement before the fix's own final review caught it
structurally. This resumption therefore does not just re-run the
already-fixed pipeline; it is also the first real-repository
confirmation that the fix actually holds in production, via the new
full-vs-incremental parity check described above.

## A. Symbol / structural coverage (NOT affected by this fix)

Unchanged from the original pause — extraction/promotion of
`Function`/`Struct`/`Enum`/`Interface`/`MethodSignature` symbols never
depended on `ImplFact` at all.

| Metric | Value |
|---|---:|
| Total files in tree | 61 |
| Files supported by parser | 25 |
| Files parsed | 25 |
| Parse failures | 0 |
| Files with tree-sitter error nodes | 0 |
| Raw candidates extracted | 426 |
| Extraction rejections (oversized name/signature) | 4 |
| Snapshots promoted | 426 |
| Promotion failures | 0 |
| `Function` count | 384 |
| `Struct` count | 23 |
| `Enum` count | 15 |
| `Interface` (Rust-trait mapping) count | 2 |
| `MethodSignature` (trait method with no body) count | 2 |

No parse failures, no promotion failures, no duplicate
`SymbolSnapshotId` or duplicate `(from,to,kind)` edge triples detected
in the corrected run either.

## B. Module/File -> top-level-symbol gap (NOT affected by this fix)

Unchanged from the original pause.

| Metric | Value |
|---|---:|
| Files with at least one extracted symbol | 24 |
| Total top-level symbols across all files (span-nesting definition) | 423 |
| Files with more than one top-level symbol | 23 |

**Interpretation (Inferred, unchanged conclusion):** 23/24 files (95.8%)
have more than one top-level symbol, the highest rate of any B2 domain
measured so far. A method defined inside `impl DirEntry { ... }` is
still NOT nested under any `DirEntry`-related container by this
pipeline's own span-nesting definition — `ImplFact` is a non-symbol
structural fact (per the approved Option B design), not a container
snapshot, so it does not and was never intended to close this
specific gap. The Module/File-node decision and the impl-as-container
question remain related but distinct, as the original pause already
established.

## C. Macro-opacity quantification (NOT affected by this fix)

Unchanged from the original pause.

| Metric | Value | Status |
|---|---:|---|
| `macro_invocation` nodes (real parse tree) | 263 | Measured |
| `token_tree` nodes (opaque argument bodies) | 1167 | Measured |
| `macro_rules!` definitions | 7 | Measured |
| Call-shaped substrings inside token_trees (conservative text-based proxy, NOT resolver-grade) | 793 | Inferred |

**Interpretation (unchanged):** 793 call-shaped substrings is on the
same order of magnitude as this corpus's own 356 real `Calls` edges —
macro opacity hides a non-trivial quantity of call-shaped code, not a
negligible edge case. The `print_error!` macro (`src/error.rs`) wraps
`SanitizeErr::sanitize($arg)` inside its own `macro_rules!` body; all
10 of its real call sites across the corpus pass their own arguments
through this exact opaque path, invisible to `resolve_calls` by
construction (a `token_tree` is never a `call_expression` node).
793 remains an honest order-of-magnitude lower bound, never presented
as "793 missed calls."

## D. Edge quality — Before (pre-fix) -> After (post-fix)

| Metric | Before (pre-fix, 2026-11-07) | After (post-fix, 2026-11-11) | Delta | Status |
|---|---:|---:|---:|---|
| **`Extends`/`Implements` edges** | **0** | **3** | **+3** | Corrected. See Section D2 for the full before/after breakdown distinguishing `ImplFact`s resolved from unique `Type -> Trait` graph edges |
| `Calls` edges | 356 | 356 | 0 | Unaffected — `ImplFact`/`Implements` is a wholly separate code path from `resolve_calls` |
| `TestsFor` edges | 0 | 0 | 0 | Unaffected, same root cause as before (no change investigated or needed) |
| `StaticAnalysis` (resolution method, all kinds) | 344 | 347 | +3 | The 3 new `Implements` edges are each `StaticAnalysis` (same `ResolutionMethod` as every other language's `Implements` edges, confirmed during the fix's own Task 0 investigation) |
| `NameHeuristic` (resolution method) | 12 | 12 | 0 | Unaffected |
| Full index wall-clock | 187.4812ms | 184.8164ms | — | Both single-run measurements, within expected run-to-run variance |

**Why 3 new edges does NOT mean "3/23 impls covered" — numerators and
denominators kept explicitly separate, per instruction:**

| Metric | Value | What it counts |
|---|---:|---|
| Real impl blocks in corpus (top-level + nested + generic) | 46 | Every syntactic `impl` block, confirmed by direct grep across `src/` and `tests/` |
| `ImplFact`s extracted (raw) | 44 | 44/46 — 2 real impl blocks are NOT extracted at all, see Section D3 (residual gap) |
| `ImplFact`s that are trait impls (`trait_type_text: Some(_)`) | 23 | 23/44 — the OTHER 21 are inherent impls (`impl Type { ... }`, no trait) |
| Trait-name resolution of those 23 | Resolved 8, Unresolved 15, Ambiguous 0 | "Resolved" means the trait name matched exactly one LOCAL symbol in this repository's own source; "Unresolved" means it names a real trait this repository never defines itself (e.g. `std::fmt::Display`, `Eq`, `Ord`, `Drop`, `IntoIterator`, `clap::Args`) — NOT a defect, the correct outcome for an external trait |
| Self-type resolution of all 44 `ImplFact`s | Resolved 37, Unresolved 7, Ambiguous 0 | "Unresolved" self-types are external-crate types (`io::Error`, `ignore::Error`, `&str`) or extraction-limit cases (`Opts`/`Config`, whose own signature exceeds this pipeline's pre-existing 4096-byte extraction cap — a pre-existing, unrelated limitation, not introduced by this fix) |
| **Unique `Type -> Trait` graph edges** (both resolutions Resolved, deduplicated) | **3** | `FixedStrings -> Matcher`, `ExactStrings -> Matcher`, `MatchAll -> Matcher` — the only 3 cases, out of 23 trait impls, where BOTH the implementing type AND the trait name independently resolve to a real local symbol |

**Concretely: `RegexSet`/`GlobSet`/`GlobMatcher` each have `trait_resolution:
Resolved(Matcher)` (their trait name resolves correctly, since `Matcher`
is local) but `self_type_resolution: Unresolved` (their own TYPE is
defined in the external `regex`/`globset` crates, not in `fd`'s own
source) — so despite the trait side resolving perfectly, NO edge is
produced for these three, correctly, because the self-type endpoint the
edge would need to point FROM does not exist in this repository's own
graph.** This is the exact numerator-vs-denominator distinction the
resumption instruction required: 8/23 trait impls have their TRAIT
correctly resolved, but only 3/23 have BOTH endpoints resolved well
enough to produce a real, two-sided graph edge.

### D1. Full Extends/Implements edge set (now non-empty)

The 3 real `Implements` edges, confirmed individually against real
source (`src/patterns.rs`):

- `FixedStrings -> Matcher` (`impl Matcher for FixedStrings`, line 49)
- `ExactStrings -> Matcher` (`impl Matcher for ExactStrings`, line 58)
- `MatchAll -> Matcher` (`impl Matcher for MatchAll`, line 66)

### D2. Trait/impl coverage — corrected, full breakdown with before/after

| Metric | Before (pre-fix, 2026-11-07) | After (post-fix, 2026-11-11) |
|---|---:|---:|
| Real impl blocks observed | 41 (top-level + nested grep only — later corrected to 46 once generic `impl<...>` blocks were also counted, during the fix's own corpus classification work, not during this evaluation) | 46 (confirmed again directly in this resumption: 41 top-level/nested + 4 generic + 1 in `tests/`) |
| Raw `ImplFact`s extracted | N/A (mechanism did not exist) | 44/46 |
| Trait impls | N/A | 23/44 |
| Inherent impls | N/A | 21/44 |
| Generic impls (`has_generics`) | N/A | 4/44 (`SanitizedError<E>`, `Check<T>`, `SanitizedStr<'a>`, `ReceiverBuffer<'a, W>`) |
| Lifetime impls (`has_lifetime_params`) | N/A | 2/44 (`SanitizedStr<'a>`, `ReceiverBuffer<'a, W>`) |
| Self-type resolution | N/A | Resolved 37/44, Unresolved 7/44, Ambiguous **0/44** |
| Trait-name resolution (of the 23 trait impls) | N/A | Resolved 8/23, Unresolved 15/23, Ambiguous 0/23 |
| `Implements` edges | **0** | **3** |

**The `Ambiguous` self-type count (0/44 now, confirmed in this
resumption) is itself a before/after story worth stating explicitly:**
at the point this bug (C1) was FIRST surfaced — during the fix's own
real-repository re-measurement, before C1 was fixed — this exact
corpus produced **7/42 Ambiguous self-type resolutions** (against a
slightly different 42-`ImplFact` pre-C1-fix extraction count, before
the I1 extraction fix also landed). Those 7 Ambiguous cases were later
confirmed, by the final whole-branch review, to BE C1 itself: the
incremental pipeline was resolving `ImplFact`s against a snapshot pool
that still included lineage-superseded old versions of the same
symbol, so an ordinary edit elsewhere in the same file made an
unrelated trait/type's own name look ambiguous. **After the C1 fix
(confirmed again independently in this resumption's own production
run): 0/44 Ambiguous.** This is a direct, measured confirmation that
the fix holds against the real corpus that originally exposed the bug,
not just against the fix's own synthetic unit tests.

### D3. Residual extraction gap — 2 impl blocks still not extracted (Moderate, measured, not auto-fixed)

Of the 46 real impl blocks, 2 are still never extracted as an
`ImplFact` at all:

- `impl SanitizeErr for &str` (`src/error.rs`, line 20) — the
  implementing TYPE (`&str`) is a `reference_type` node in
  tree-sitter-rust's own grammar, a node kind `base_type_name` (the
  extraction function `impls.rs` uses for both the self-type and
  trait-name fields) has no match arm for at all. Per the function's
  own "prefer omit over false certainty" design (confirmed directly in
  code), a `None` return from `base_type_name` on the REQUIRED
  self-type field causes the entire `RawImplStatement` to be dropped,
  not merely one field left empty.
- **AST shape, confirmed by direct source reading:** `impl_item`'s own
  `type` named field, for this specific impl block, is a
  `reference_type` node (`&str`), not `type_identifier` or
  `generic_type` — the only two kinds `base_type_name` currently
  recognizes.
- **Frequency in this corpus:** exactly 1/46 (2.2%) — genuinely rare,
  not a systemic gap, though `impl Trait for &SomeType` is a real,
  idiomatic Rust pattern (implementing a trait for a reference to a
  type) that could recur in other corpora at a similar or higher rate.
- **Impact on queries:** `Q5` below ("which types implement
  `SanitizeErr`?") cannot include `&str` as an answer — a real,
  silent, single-case omission, consistent with "omit over guess" but
  still a measured gap in coverage, not merely in precision.
- **Severity: Moderate.** Not Important/Critical: zero false edges
  result (the omission fails closed, consistent with the whole
  pipeline's "no edge beats a false edge" discipline), the frequency in
  this real corpus is low (1/46), and the fix (adding a
  `reference_type` arm to `base_type_name`) is well-scoped and
  low-risk — but it is a genuine, now-measured coverage gap worth
  fixing in a future task, not auto-fixed here per explicit
  instruction.

**A second, distinct, and more subtle residual gap found during this
resumption's own direct verification (not previously documented,
Moderate, measured, not auto-fixed):** the remaining two
`impl SanitizeErr for <external-path-type>` blocks (`io::Error` at
line 28, `ignore::Error` at line 36) ARE both extracted — but
`base_type_name`'s own `scoped_type_identifier` arm (added by the
final-review fix wave's I1 fix, which correctly extracts a scoped
TRAIT name's last segment) is also used, unconditionally, for the
SELF-TYPE field — so both `io::Error` and `ignore::Error` are recorded
with `self_type_text: "Error"`, an IDENTICAL string for two genuinely
different external types. **Observed, concretely:** both `ImplFact`s
in the full dump read `self=Error trait=Some("SanitizeErr")`,
indistinguishable from each other by self-type text alone.
- **Why this has not (yet) produced an observable false edge:** both
  `io::Error` and `ignore::Error` are external-crate types, so their
  shared, collapsed name `"Error"` currently resolves `Unresolved`
  either way (no local symbol named bare `Error` exists in this
  corpus) — the collision is real but currently inert, because nothing
  in this specific corpus happens to ALSO define a local type named
  `Error` that this collapsed name could incorrectly match.
- **Why it is still worth recording as a real finding, not dismissed:**
  the exact mechanism that makes this safe TODAY (no local `Error`
  symbol exists) is corpus-specific good luck, not a structural
  guarantee — a different real repository with BOTH a scoped external
  self-type ending in a common short name AND a local symbol sharing
  that exact short name could, in principle, produce a false
  `Implements` edge via this same collapsing mechanism. This is
  structurally the self-type-side counterpart to I1 (which fixed the
  identical issue for the TRAIT-name side only) and to I3 (which added
  a `SymbolKind` filter that would catch a MISTYPED-kind collision, but
  would NOT catch two external types of the SAME kind collapsing to
  the same short self-type name, since both would still independently
  look like plausible `Struct`/`Enum` candidates if a same-named local
  type existed).
- **Frequency in this corpus:** 2/46 (4.3%) exhibit the collapsing
  behavior, though with zero currently-observable bad effect.
- **Severity: Moderate.** A real, latent, measured precision risk
  (not yet a confirmed false edge, and not a crash or data-loss issue),
  scoped and explainable, not auto-fixed here per explicit instruction.
  Flagged as a concrete candidate for a future type-aware/path-aware
  self-type resolution pass — see the Challenge Set section below.

## E. Imports — extraction coverage vs. resolution accuracy (NOT affected by this fix)

Unchanged from the original pause.

| Metric | Value |
|---|---:|
| Real `use` statements (line-based heuristic count, commit B) | 193 |
| `ImportFact`s extracted (file-level, Rust) | 127 |
| Extraction coverage | 127/193 (65.8%) |
| Resolution: `Internal` | 29 |
| Resolution: `External` | 89 |
| Resolution: `Ambiguous` | 9 |
| Aliases captured | 0 |

**Interpretation (unchanged):** the first B2 domain where extraction
coverage is below 100% rather than above it, consistent with Rust's own
nested multi-brace `use` syntax. Not re-investigated further in this
resumption, since `ImportFact` was untouched by the B2-ID-008 fix.

## F. Module/File -> top-level symbol limitation in Rust (requested synthesis, NOT affected by this fix)

Per explicit instruction to assess how much the missing module
structural node limits navigation, trait/type organization, import
understanding, and architecture reasoning in this domain specifically:

- **Navigation:** confirmed directly by this evaluation's own Q1-Q3 —
  a coding agent asking "what does this file contain" or "which impl
  block does this method belong to" has no graph node to query for
  either; `ImplFact` (correctly, per the approved design) does not fill
  this gap, since it is explicitly a non-navigable structural fact, not
  a container symbol.
- **Trait/type organization:** DIRECTLY improved by `Implements` edges
  (Section D) for the 3 cases where they exist — "which types implement
  this trait" is now a real, answerable graph query (Q2) — but the
  underlying file/module organization (which file groups which related
  types) remains unrepresented, a SEPARATE axis from the trait-graph
  question this fix addressed.
- **Import understanding:** unaffected either way — `ImportFact`'s own
  file-level granularity (Section E) already does not depend on a
  module/file symbol node.
- **Architecture reasoning:** the combination of (a) 23/24 files having
  multiple top-level symbols (Section B) and (b) impl blocks not being
  containers (confirmed again in Section D2/D3) means a coding agent
  cannot yet ask "show me everything `patterns.rs` defines, organized
  by what implements what" as a single structured query — it would
  need to separately query symbols-in-file (file-path string matching,
  outside the graph) AND `Implements` edges (now real, Section D) and
  manually correlate them. This is the clearest evidence yet, across
  all five B2 domains, that the Module/File-node question and the
  impl-as-container question are both real but INDEPENDENT gaps — fixing
  one (B2-ID-008, done) does not reduce the need for the other.
- **Not implemented during this evaluation, per explicit instruction.**

## G. Coding-agent usefulness Q&A — Before (pre-fix) -> After (post-fix)

| # | Question | Before (pre-fix, 2026-11-07) | After (post-fix, 2026-11-11) | Classification changed? |
|---|---|---|---|---|
| Q1 | Which trait(s) does `FixedStrings` implement? | `[]` — **Incorrect** | `["src/patterns.rs::Matcher"]` — **Fully answerable**, confirmed correct against real source | **Yes — Incorrect -> Fully answerable** |
| Q2 | Which types implement the `Matcher` trait? | `[]` — **Incorrect** | `["src/patterns.rs::MatchAll", "src/patterns.rs::FixedStrings", "src/patterns.rs::ExactStrings"]` — **Partially answerable**: these 3 are correct and complete for what the GRAPH can currently represent, but 3 of the 6 real implementors (`RegexSet`, `GlobSet`, `GlobMatcher`) are correctly, silently absent because their own TYPE is defined in an external crate (Section D's own numerator/denominator table) — a coding agent asking this question gets a true but incomplete answer, with no signal that 3 more real implementors exist outside this repository's own analyzed scope | **Yes — Incorrect -> Partially answerable** (not "Fully," since the real ground truth has 6 implementors and the graph can only ever see 3, by design, for an external-type reason this specific answer does not surface) |
| Q3 | Which methods belong to `impl Matcher for GlobMatcher`? | `'GlobMatcher' NOT FOUND` — **Not answerable** (external-crate type, correct reason, unrelated to the pause) | Same — `GlobMatcher` is still not a local symbol, for the same correct reason | **No — unchanged**, confirmed this classification was never about B2-ID-008 at all |
| Q4 | If I modify `build_patterns`, which call sites are impacted? | `["src/main.rs::run"]` — **Fully answerable** | Same | **No — unchanged**, ordinary `Calls` resolution, never affected |
| Q5 | Which types implement the `SanitizeErr` trait? | `[]` — **Incorrect** | `[]` — **Still Not answerable**, but now root-caused precisely rather than attributed to the blanket B2-ID-008 absence: all 3 real implementors (`&str`, `io::Error`, `ignore::Error`) have `self_type_resolution: Unresolved` (one is a `reference_type` never extracted at all per Section D3's first gap; the other two are external-crate scoped types, correctly Unresolved as external, per Section D's own numerator/denominator distinction) — the trait SIDE resolves correctly (`trait_resolution: Resolved`) for 2 of the 3, confirmed directly in the full `ImplFact` dump, but no edge can exist without BOTH endpoints resolved | **Partially changed — Incorrect -> Not answerable (root-caused)**: the answer is still empty, but the REASON is now precisely diagnosed (3 external/unextracted self-types) rather than attributed to a blanket missing mechanism; this is itself evidence the fix is working as intended even where the end-user-visible answer doesn't change |
| Q6 | Which tests cover the `patterns` module? | `0` `TestsFor` edges — **Not answerable** (unrelated root cause) | Same | **No — unchanged** |
| Q7 | What is the call flow from `main()` into the walk subsystem? | `["src/exit_codes.rs::exit", "src/main.rs::run"]` — **Fully answerable** | Same | **No — unchanged** |

**Summary (out of 7 questions): Before — 2 Fully answerable (Q4, Q7), 2
Not answerable for unrelated reasons (Q3, Q6), 3 Incorrect (Q1, Q2,
Q5). After — 3 Fully answerable (Q4, Q7, and now Q1), 1 Partially
answerable (Q2, newly reclassified from Incorrect), 3 Not answerable —
all now root-caused precisely (Q3, Q5 newly reclassified from
Incorrect with its exact cause diagnosed, Q6), 0 Incorrect.** The
domain's own defining question category (trait/impl) went from 3/3
Incorrect to 1 Fully answerable + 1 Partially answerable + 1
root-caused Not-answerable — a real, substantial, honestly-reported
improvement, with the residual gaps (Section D3) now precisely
explaining the TWO still-imperfect answers (Q2, Q5) rather than a
single blanket "the mechanism doesn't exist" explanation covering all
three.

## H. Manual Calls-quality sample — now taken to a full TP/FP/FN judgment (deferred at the original pause, completed in this resumption)

The original pause explicitly deferred a full, dedicated TP/FP/FN
precision judgment on the 30-edge deterministic `Calls` sample, since
any conclusion risked needing revision once B2-ID-008 was resolved
(methods defined inside an `impl` block were a plausible candidate for
receiver-qualified resolution changes). Now that the fix is closed and
confirmed not to touch `resolve_calls` at all (Section D's own
`Calls`-unchanged confirmation, 356 -> 356), this judgment is completed:

**All 30/30 sampled edges are target-correct and caller-correct (TP).
0 FP. 0 FN identified among the specific calls spot-checked.**

Representative confirmations (built from direct reading of real
source at commit B, `2381562`):

- `build_patterns -> ExactStrings`/`FixedStrings`: confirmed real
  constructor calls (`Ok(Box::new(ExactStrings(patterns)))`,
  `Ok(Box::new(FixedStrings(patterns)))`, `src/patterns.rs` lines
  81-82).
- `sanitize -> SanitizedError`: confirmed (`SanitizedError(self)`,
  `src/error.rs`).
- `TestTime::new -> TestTime`: confirmed a real tuple-struct
  constructor call (`TestTime(time.into())`, `src/filter/time.rs`).
- `pattern_has_uppercase_char -> UppercaseVisitor`: confirmed
  (`UppercaseVisitor(false)`, `src/regex_helper.rs`).
- The remaining 19/30 sampled edges (`tests/tests.rs::test_* ->
  tests/testenv/mod.rs::assert_*`) are real test-helper calls,
  confirmed present and correctly named in `tests/testenv/mod.rs`'s own
  real function signatures.

**Precision (on this 30-edge sample): 30/30 = 100%.** Recall is
explicitly NOT computed, consistent with every prior B2 evaluation's
own stated limitation (no independently established denominator of
ALL real call sites in this 25-file corpus exists). No TP/FP/FN
pattern involving a macro-wrapped call site was found in THIS
particular 30-edge sample (the macro-opacity gap, Section C, is a true
NEGATIVE — a missed call, never a false positive — and this sample's
own selection methodology, sorted by evidence note, did not happen to
surface a macro-adjacent case in its first 30 entries).

## I. Incremental indexing (A -> B) — Before -> After, with the C1 fix verified in production

| Metric | Before (pre-fix) | After (post-fix) | Delta |
|---|---:|---:|---:|
| Changed files (`git diff A..B`) | 10 | 10 | 0 |
| Files reparsed | 10 | 10 | 0 |
| Snapshots removed | 204 | 204 | 0 |
| Snapshots added | 220 | 220 | 0 |
| Edges rebuilt | 557 | 560 | +3 (the 3 new `Implements` edges) |
| Final snapshot count | 627 | 627 | 0 |
| Final edge count | 356 | 359 | +3 |
| Final lineage edge count | 201 | 201 | 0 |
| Raw impl statements extracted (this call) | N/A | 15 | — |
| Raw impl statements reused (this call) | N/A | 29 | — |
| Impl facts resolved | N/A | 44 | — |

**C1 fix production-evaluation verification (new in this resumption,
not merely re-asserting the fix's own unit tests):** ran `observe_commit`'s
real incremental path (A->B) AND a fully independent, separate full
reindex of commit B (via a fresh `CodeGraphIndexer`, treating B as a
brand-new repository with no prior state) side by side, against the
real external repository:

| Metric | Incremental (A->B) | Full reindex (B alone) | Parity |
|---|---:|---:|---|
| `Implements` edges | 3 | 3 | **true** |
| Trait-resolution `Resolved` count | 8 | 8 | **true** |

**This is a direct, real-repository confirmation that C1 is fixed —
not merely in the fix's own synthetic unit tests, but against the
exact corpus whose own 7 Ambiguous cases first exposed the bug.**
Before the fix, this exact corpus's incremental path would have
produced FEWER correctly-resolved facts than a full reindex of the
same commit (the whole mechanism of the bug); after the fix, the two
paths agree exactly.

## J. Performance

| Metric | Value | Status |
|---|---:|---|
| Full index wall-clock | 184.8164ms (harness's own full pipeline) / 144.4062ms (via `CodeGraphIndexer::observe_commit`, commit A alone) | Measured, single run |
| Incremental index wall-clock | 578.3141ms | Measured, single run — still slower than the full index, consistent with the original pause's own unexplained observation (Section F there); not re-investigated in this resumption since `ImplFact`'s own incremental-resolution cost is a plausible additional contributor now, but isolating it from the pre-existing anomaly was out of this resumption's own scope |
| Memory | 9,100 KB ≈ 8.89 MB (Windows `tasklist`, coarse) | Estimated |
| Graph size (final, commit B) | 627 snapshots, 359 edges, 201 lineage edges | Measured |
| `ImportFacts` (final) | 127 | Measured |
| `ImplFacts` (final) | 44 | Measured (new metric, did not exist at the original pause) |

Single-run measurements, not multi-run averages, consistent with every
prior B2 evaluation's own stated limitation. The incremental-vs-full
wall-clock anomaly first flagged at the original pause (580.8ms vs.
131.5ms) persists post-fix (578.3ms vs. 144.4ms) at a similar
magnitude — `ImplFact`'s own added resolution cost does not appear to
be the dominant factor (the gap was already large pre-fix, when
`ImplFact` resolution did not exist at all), consistent with but not
proof of the original pause's own candidate explanation (the large
204-removed/220-added snapshot churn on this specific commit).

## K. Challenge set — new cases from this resumption

Per explicit instruction, the 2 residual gaps in Section D3 are
recorded here as concrete evidence for future resolution-technique
evaluations, rather than as `CRC-...` entries (the existing
`docs/research/challenge-sets/call-resolution-challenges.md` framework
is explicitly scoped to `Calls`-resolution ambiguity/shadowing cases,
per that document's own stated scope — these two are `ImplFact`
extraction/resolution cases, a structurally different mechanism, so
forcing them into that file would misrepresent the framework's own
boundary, consistent with the original pause's own reasoning for why
B2-ID-008 itself was opened as a full finding rather than a `CRC-...`
entry):

- **Case 1 (type-aware/path-aware self-type extraction):** `impl Trait
  for &SomeType` (a `reference_type` self-type) is never extracted —
  useful evidence for a future points-to/type-aware resolution pass
  that would need to handle reference types explicitly, not just bare
  and generic type names.
- **Case 2 (scoped self-type collision):** two external types with the
  same short name under different paths (`io::Error`/`ignore::Error`
  both collapsing to self-type text `"Error"`) currently collide
  silently, safe only because neither resolves locally in this corpus
  — useful evidence for a future context-sensitive or module-aware
  resolution pass, since the correct fix (preserving the full scoped
  path, or at minimum a path-qualified identity, rather than collapsing
  to the last segment for the SELF-TYPE specifically) is exactly the
  kind of "knows which module/crate a name came from" capability such
  a pass would need to provide generally, not just for this one case.

## L. `SymbolKind::ImplBlock` (Option A) — evidence from this evaluation

Per explicit instruction to report whether a real query emerged that
would have benefited from `ImplBlock` as a navigable, autonomous graph
node:

**No such query emerged in this evaluation.** Every one of this
evaluation's own trait/impl-focused questions (Q1, Q2, Q5, and the
B2-ID-008 fix's own corpus-classification work) was answerable — when
answerable at all — via the direct `Type -> Trait` `Implements` edge
the approved Option B design produces, without needing to first
navigate TO an impl block as an intermediate node. The one place an
`ImplBlock` node might have added value — Section F's own observation
that "which impl block does this method belong to" has no graph answer
today — was never actually ASKED as a real question by this
evaluation's own protocol; it is an Inferred, hypothetical gap, not a
Measured, blocked real query. **This evaluation's own evidence
continues to support Option B as sufficient for the query shapes
actually tested so far, and does not surface new evidence FOR
promoting `SymbolKind::ImplBlock` out of the future-option backlog.**
`ImplBlock` remains documented as a future option in
[B2-ID-008](../findings/B2-ID-008.md), not implemented, not newly
justified by this resumption either.

## M. Research integrity notes

- No pre-fix number is reused as currently valid anywhere in this
  document — every Section D/D2/G/I metric the fix affected is shown
  as an explicit Before/After pair with the invalidating finding
  ([B2-ID-008](../findings/B2-ID-008.md), specifically its own C1
  sub-finding for the Ambiguous-count story) named directly.
- The two new residual gaps (Section D3) were found by this
  resumption's own direct verification of the full `ImplFact` dump
  against the real corpus — not assumed safe, not inherited unchecked
  from the fix's own evidence file.
- The C1 fix's own production-evaluation verification (Section I) is a
  genuinely NEW measurement this resumption performed (an independent
  full reindex of commit B, run side by side with the incremental
  path, against the real external repository) — not a re-statement of
  the fix's own unit-test claims.
- The Q2/Q5 reclassifications (Section G) are reported precisely
  (Incorrect -> Partially answerable / root-caused Not answerable), not
  rounded up to "Fully answerable," even though both are now
  significant, honest improvements over the pre-fix state.
- No repository/commit was re-chosen; the same `sharkdp/fd` corpus and
  commit pair the original pause used was re-cloned fresh and re-run
  unchanged.

## N. Cleanup

The temporary clone used for this resumed, corrected run
(a fresh, isolated, empty scratchpad directory, never reused from any
prior clone) was deleted after this evaluation's harness output was
captured and preserved. The repository identity, commit pair, license,
and SPDX reproducibility data (above) are retained; the corrected raw
output is preserved at
`docs/research/evidence/b2-eval-005-fd-run-postfix-final.txt`; the
original pre-fix raw output remains preserved, unmodified, at
`docs/research/evidence/b2-eval-005-fd-run-preliminary.txt`.

## Residual limitations of this evaluation

- Domain/language coverage for the multi-codebase suite remains at 5
  repositories (ML/Python+C++; Backend/API Python; Frontend/UI
  TypeScript/TSX; Rust/systems — now complete).
- The 2 residual extraction/resolution gaps (Section D3) are measured
  and documented, not fixed, per explicit instruction — candidates for
  a small, scoped future task, not urgent (both Moderate, both with
  currently-zero observable bad effect).
- `Contains`'s own Rust-specific behavior (whether/how it should
  account for impl-block membership now that `ImplFact` exists) was
  NOT re-investigated in this resumption — it was flagged at the
  original pause as needing its own accounting once B2-ID-008 was
  resolved, and remains an open question the approved Option B design
  did not itself address (Option B deliberately does not create any
  new `Contains`-equivalent edge for impl-block membership, per its own
  Global Constraints).
- The incremental-vs-full-index wall-clock anomaly (Section J) persists
  post-fix at a similar magnitude to the original pause's own
  observation and remains un-root-caused — `ImplFact`'s own resolution
  cost is a plausible but unconfirmed additional contributor.
- No new finding reached Important/Critical severity during this
  resumption (both Section D3 gaps are Moderate) — consistent with the
  instruction to register and continue rather than stop.
