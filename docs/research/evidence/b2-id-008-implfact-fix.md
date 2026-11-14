# B2-ID-008: ImplFact fix — mutation testing, real-repository re-measurement, full-suite confirmation (Task 5)

**Date:** 2026-11-07.
**SuperBrain commit (base):** `eb1fb46a719a4a400c1e79d9755840dcf3b3d8b9` (Task 4,
"wire ImplFact into incremental pipeline"), plus this task's own two
additive changes on top (see "Changes made by this task" below).
**Scope:** Task 5 of `docs/superpowers/plans/2026-11-07-b2-id-008-implfact.md`
— mutation testing for `impls.rs` (Tasks 1-2) and `edges/extends_implements.rs`'s
Rust-specific surface (Task 3), a real-repository before/after
re-measurement against `sharkdp/fd` at the same commit pair
[B2-EVAL-005](../evaluations/B2-EVAL-005.md) used, and a full
workspace test/clippy/deny confirmation. Per this project's own
Evaluation & Paper Evidence Policy, every number below is **Measured**
unless explicitly marked **Inferred**.

## Changes made by this task

1. `crates/codegraph-index/src/impls.rs`: a doc comment documenting why
   the `.or_else` fallback inside `collect_rust_impl` (reading a
   `declaration_list` positionally when `child_by_field_name("body")`
   is absent) is a genuinely equivalent mutation target, plus one new
   regression test, `resolve_impl_facts_the_one_file_tree_convenience_wrapper_actually_extracts_and_resolves`,
   closing a real mutation-testing gap (see "Campaign 1" below).
2. `crates/codegraph-index/examples/b2_smoke_test_rust_systems.rs`: one
   small, additive block (no restructuring) surfacing `ImplFact`-specific
   metrics this harness did not print before this task, since it
   predates the B2-ID-008 fix — raw impl statements extracted/reused,
   `impl_facts` total with trait/inherent split, trait-resolution and
   self-type-resolution breakdowns, the final `Implements` edge count,
   and associated-item counts split by `Method`/`AssociatedFunction`,
   plus a full per-fact dump (mirroring the harness's own existing
   Section D1 "full set, not a sample" convention).

No other production code was changed. No `SymbolKind::ImplBlock` (the
rejected Option A design) was introduced anywhere by this task.

## Part 1: Mutation testing (two separate campaigns, per this project's own standing per-module discipline)

### Campaign 1: `crates/codegraph-index/src/impls.rs` alone

Command: `cargo mutants --no-shuffle -p codegraph-index -f crates/codegraph-index/src/impls.rs -- --release`

**Run 1 (before triage):**

```
Found 34 mutants to test
ok       Unmutated baseline in 18s build + 26s test
 INFO Auto-set test timeout to 133s
MISSED   crates/codegraph-index/src/impls.rs:198:71: replace == with != in collect_rust_impl in 1s build + 7s test
MISSED   crates/codegraph-index/src/impls.rs:319:5: replace resolve_impl_facts -> Result<Vec<ImplFact>, ImplFactError> with Ok(vec![]) in 2s build + 6s test
34 mutants tested in 5m: 2 missed, 24 caught, 8 unviable
```

**Triage of both missed mutants:**

1. **`impls.rs:319:5` — `resolve_impl_facts` body replaced with `Ok(vec![])`
   — genuine test gap, CLOSED.** `resolve_impl_facts` (the one-file-tree
   convenience wrapper mirroring `resolve_import_facts`'s own shape
   exactly) had no direct test anywhere in this module or the wider
   crate — confirmed by grep, zero call sites outside its own
   definition and this gap's own new test. Closed with
   `resolve_impl_facts_the_one_file_tree_convenience_wrapper_actually_extracts_and_resolves`:
   parses a real tree containing one real trait impl, calls
   `resolve_impl_facts` directly, and asserts both the extraction count
   and the resolved self-type/trait-type outcomes, so a mutant
   collapsing the function to an empty `Vec` now fails this test.

2. **`impls.rs:198:71` (`collect_rust_impl`'s `.or_else(|| ...find(|c|
   c.kind() == "declaration_list"))` fallback) — genuinely EQUIVALENT,
   not a real gap.** Directly probed with a throwaway test (added
   temporarily, then removed after confirming the result; the written
   justification below is the surviving evidence) across five real
   tree-sitter-rust parses of `impl_item`:
   - `impl Eq for D {}` (empty trait-impl body, the real shape seen in
     this very corpus's `impl Eq for DirEntry {}`): `body` field =
     `Some(declaration_list)` — the `.or_else` branch is never reached.
   - `impl Eq for D` (malformed, missing body — triggers tree-sitter
     error recovery): `body` field = `None`, AND the node's own
     children are only `impl`/`type_identifier`/`for` — no
     `declaration_list` sibling exists either, so the `.or_else`'s own
     `find` returns `None` regardless of the `==`/`!=` predicate used.
   - `unsafe impl Send for D {}`: `body` = `Some(declaration_list)`.
   - `impl<T> D<T> where T: Clone { fn f(&self) {} }` (where-clause
     impl): `body` = `Some(declaration_list)`.
   - `#[allow(dead_code)] impl D { fn f(&self) {} }` (attribute before
     impl): `body` = `Some(declaration_list)`.

   In every shape tree-sitter-rust can produce for `impl_item` — well-formed
   or error-recovered — `body` is EITHER `Some(declaration_list)`
   directly (fallback never entered) OR `None` with no `declaration_list`
   sibling present either (fallback finds nothing under either
   predicate). No input exists that makes flipping `==` to `!=` in the
   fallback's own `find` predicate observably different. This written
   justification (not a bare assertion) is now recorded directly in
   `impls.rs`'s own source comment at the mutation site.

**Run 2 (final, after closing the one genuine gap):**

```
Found 34 mutants to test
ok       Unmutated baseline in 18s build + 29s test
 INFO Auto-set test timeout to 150s
MISSED   crates/codegraph-index/src/impls.rs:216:71: replace == with != in collect_rust_impl in 2s build + 7s test
34 mutants tested in 5m: 1 missed, 25 caught, 8 unviable
```

(Line number shifted from 198 to 216 because the equivalence doc
comment above the fallback added lines; it is the identical mutation
site.)

**Campaign 1 conclusion: 0 genuine (non-equivalent) missed mutants.**
25/34 caught directly, 1/34 genuinely equivalent (written justification
above), 8/34 unviable (standard `cargo-mutants` outcome for mutants
that fail to compile — not individually inspected, consistent with
every prior B2 mutation run in this project, e.g.
`docs/research/evidence/b2-id-007-mutation-extends-implements.txt`).
No TIMEOUT results in either run — the known
harness-deadlock-under-mutation pitfall this project has previously
had to investigate ([[c24eb]]) did not arise here.

### Campaign 2: `crates/codegraph-index/src/edges/extends_implements.rs`'s Rust-specific surface

Per the brief: Task 3 (commit `ec1a47f`) removed `collect_rust_impl_trait`
and its two stale tests, removed the `Language::Rust` match arm's own
body from `base_names_in` (folded into the shared
`Language::Rust | Language::Html | Language::Css => {}` no-op arm), and
added one new pinning test,
`rust_impl_trait_for_type_produces_zero_extends_or_implements_edges_by_design`.

**Confirmed directly (grep across the whole file): there is no Rust-specific
collector function left in this file at all** — `collect_python_bases`,
`collect_csharp_base_list`, `collect_cpp_base_clause`,
`collect_js_heritage`, and `collect_ts_heritage_tagged` are the only
five `collect_*` functions remaining, none Rust-specific. The entire
Rust-relevant surface Task 3 touched is: (a) the no-op match arm
`Language::Rust | Language::Html | Language::Css => {}`, which has no
expression, literal, or condition for `cargo mutants` to mutate (a
bare unit-returning match arm), and (b) the two Rust-focused tests
(`a_bare_inherent_rust_impl_with_no_for_clause_produces_no_edge`,
predates Task 3, and the new pinning test above).

Per the brief's own explicit instruction ("if `cargo mutants` finds
nothing meaningful to mutate in that file's Rust-specific surface, say
so explicitly rather than forcing a campaign"): **there is no
meaningful Rust-specific mutable surface in this file for a narrowly
Rust-scoped campaign.** To still produce a real, useful data point
rather than skip measurement entirely, the full file was run as one
`cargo mutants` campaign (command below) and the result is reported
with this caveat made explicit: any mutant this run found belongs to
the OTHER five languages' collectors or the shared
`for_each_node_not_inside_a_nested_boundary`/`base_names_in`/`edge_kind_for`
infrastructure (all pre-existing, B2-ID-007 territory, not B2-ID-008's
own Rust-specific change) — not to anything Task 3 added or removed.

Command: `cargo mutants --no-shuffle -p codegraph-index -f crates/codegraph-index/src/edges/extends_implements.rs -- --release`

```
Found 35 mutants to test
ok       Unmutated baseline in 21s build + 35s test
 INFO Auto-set test timeout to 177s
35 mutants tested in 6m: 31 caught, 4 unviable
```

**Campaign 2 conclusion: 0 missed mutants across the WHOLE file**
(31/35 caught, 4/35 unviable, 0/35 missed) — stronger than the
brief's own target even though no mutant in this run is actually
attributable to Task 3's own Rust-specific change (there is nothing
left there to mutate). No TIMEOUT results. **0 genuine missed mutants
in both campaigns, as required.**

## Part 2: Real-repository re-measurement against `sharkdp/fd`

**Repository:** `sharkdp/fd` (https://github.com/sharkdp/fd),
dual-licensed MIT/Apache-2.0. Cloned read-only into a fresh, empty,
isolated scratchpad directory (never reused from any prior B2
evaluation's own clone), deleted after this task's data was captured
(confirmed deleted, no residual clone left on disk).
**Commit pair (identical to B2-EVAL-005):** commit A
`b7a0a29d07c3d0e9e0a41155019acada08831a6c`, commit B
`23815624819c77ab665e19c8e6c28bddbcd96e1d`.
**Harness:** `crates/codegraph-index/examples/b2_smoke_test_rust_systems.rs`,
run via `cargo run --release --example b2_smoke_test_rust_systems -- <repo_path> <commit_a> <commit_b>`,
after this task's own small additive metrics block (see "Changes made
by this task" above) — against the now-fixed pipeline (post Tasks 1-4).

### Before (already recorded in B2-EVAL-005, pre-fix)

| Metric | Value |
|---|---:|
| Impl blocks observed (B2-EVAL-005's own count) | 41 |
| Trait impls (B2-EVAL-005's own count) | 23 |
| `Implements`/`Extends` edges | 0 |

### SUPERSEDED by the final-review fix wave (C1 + I1 + I2)

**Everything from this point through the end of "Associated-item
coverage" below is superseded.** The final whole-branch review found:
(a) a Critical bug (C1) — `impl_facts` was incorrectly resolved against
`final_snapshots` (which deliberately also contains historical,
lineage-superseded snapshots), making the 7 `Ambiguous` self-types
reported below an ARTIFACT of that bug, not a genuine finding; (b) an
Important bug (I1) — a `scoped_type_identifier` trait/self-type field
(e.g. `fmt::Display`, `clap::Args`) incorrectly returned `None` instead
of its last segment, which is why the "16 trait impls" / "4
scoped-trait-extraction-misses" numbers below exist at all; and (c)
this section's own arithmetic ("23 − 4 − 3 = 16") did not actually
reconcile against its own stated breakdown, AND the "4
scoped-trait-extraction-misses" list itself undercounted by one: it
named `clap::FromArgMatches` (Exec), `clap::Args` (Exec), `fmt::Display`
(PathUrl), and `regex_syntax::ast::Visitor` (UppercaseVisitor), but
missed a 5th identically-shaped case — `impl fmt::Display for Encoded`
(`src/hyperlink.rs:71`, inside the SAME `#[cfg(test)] mod test` block
as `PathUrl`'s own `Display` impl) — confirmed by direct source
reading; this 5th case is why the old section's denominator math could
never actually reconcile no matter how the other terms were adjusted.
Both C1 and I1 are now fixed in code (see
`crates/codegraph-index/src/incremental/indexer.rs`'s `impl_facts`
resolution call and `crates/codegraph-index/src/impls.rs`'s
`base_type_name`). The reconciliation below does not depend on
re-enumerating "which N were scoped-and-dropped-to-None" at all (the
exact bug class that produced the undercount in the first place) — it
instead counts real impl blocks directly by `grep`, counts real
trait-impl syntax directly by `grep`, and lets the harness's own
measured `impl_facts_total`/`trait_impls` confirm the arithmetic,
which is why it reconciles cleanly where the old section did not. The
superseded numbers below are kept VERBATIM (struck through in spirit,
not in text, per this project's own append/supersede discipline) so
the before/after delta across the whole fix wave remains checkable;
"### After the final-review fix wave (C1 + I1), Measured" immediately
below is the CURRENT, authoritative account.

### After the final-review fix wave (C1 + I1), Measured

Fresh clone of `sharkdp/fd`, same commit pair, re-run AFTER both the
C1 (`impl_facts` resolved against `current_snapshots`, not
`final_snapshots`) and I1 (`scoped_type_identifier` extracts its last
segment) fixes landed. Raw harness output committed verbatim as
[`b2-id-008-final-fix-run.txt`](b2-id-008-final-fix-run.txt) so every
number below is independently checkable against it.

**Pipeline-level counts** (`IndexMetrics` from `CodeGraphIndexer::observe_commit`,
incremental A→B call):

| Metric | Value |
|---|---:|
| `raw_impl_statements_extracted` (this call, i.e. commit B's changed files) | 15 |
| `raw_impl_statements_reused` (carried forward from untouched files) | 29 |
| `impl_facts_resolved` / `impl_facts_total` (final published state) | **44** |

**Denominator, reconciled exactly (direct `grep`, corrected for the
`impl<...>` generic-impl shape the prior draft's `^impl ` pattern
missed — `impl<T> Foo<T> {` has no space after `impl`, so a pattern
requiring one silently skips every generic impl block):**

| Count | Value |
|---|---:|
| Top-level `impl`/`impl<...>` blocks in `src/` (`grep -rn "^impl" src/`) | 42 |
| Nested `impl` blocks inside a `#[cfg(test)]` module in `src/` (`src/filter/time.rs` ×2, `src/hyperlink.rs` ×1) | 3 |
| `impl` blocks in `tests/` (`tests/testenv/mod.rs::TestEnv`) | 1 |
| **Total real impl blocks in the repository at commit B** | **46** |

**46 real impl blocks → 44 `ImplFact`s (2 dropped whole):**

| Dropped impl block | File | Why |
|---|---|---|
| `impl SanitizeErr for &str` | `src/error.rs:20` | self type is a `reference_type` node (`&str`), not `type_identifier`/`generic_type`/`scoped_type_identifier` — out of `base_type_name`'s scope, confirmed in-scope limitation, not a new defect |
| `impl From<ExitCode> for i32` | `src/exit_codes.rs:14` | self type is a `primitive_type` node (`i32`) — same category of out-of-scope shape |

Both confirmed dropped as a WHOLE impl block (neither appears anywhere
in the harness's own full `impl_facts` dump, not even as a
`trait_type_text: None` row) — directly checkable in
`b2-id-008-final-fix-run.txt`.

**46 − 2 = 44, matching `impl_facts_total` exactly.**

**Trait impl vs. inherent split, with the arithmetic shown explicitly
(I2 fix: this is the reconciliation the superseded section above
failed to do correctly):**

| Step | Count |
|---|---:|
| Real `impl X for Y` (trait-impl syntax) blocks anywhere in the repo (`grep -c " for "` over the same 46-line list) | 25 |
| Minus: dropped-whole (both of the 2 dropped blocks above are themselves trait impls — `SanitizeErr for &str` AND `From<ExitCode> for i32`) | − 2 |
| = Trait impls that became an `ImplFact` at all | **23** |
| Of those 23, how many get `trait_type_text: Some(_)` (I1 fix: ALL of them now, since `scoped_type_identifier` no longer returns `None`) | 23/23 |

**`trait_impls` (`trait_type_text: Some(_)`): 23/44. `inherent_impls`
(`trait_type_text: None`): 21/44. 23 + 21 = 44 — reconciles exactly.**

This supersedes the old "16 trait impls" figure entirely: before I1,
5 of the 23 real trait impls (`clap::FromArgMatches for Exec`,
`clap::Args for Exec`, `fmt::Display for PathUrl`, `fmt::Display for
Encoded` — the undercounted 5th case I2 flagged — and
`regex_syntax::ast::Visitor for UppercaseVisitor`) had their
`scoped_type_identifier` trait name wrongly collapse to
`trait_type_text: None`, making them LOOK inherent (23 − 5 = 18, not
16 — the old section's own "23 − 4 − 3 = 16" arithmetic both missed
this 5th case AND double-counted by ALSO subtracting 3 dropped-whole
impls that were never part of the trait-name question in the first
place, which is the exact reconciliation failure I2 flagged). After
I1, all 23 extracted trait impls correctly carry a real trait name.

**Trait-resolution breakdown (of all 23 trait impls that now carry a
`trait_type_text`):**

| `trait_resolution` | Count |
|---|---:|
| `Resolved` | 8/23 |
| `Unresolved` | 15/23 |
| `Ambiguous` | 0/23 |

**The 8 `Resolved`** (directly confirmed in `b2-id-008-final-fix-run.txt`'s
own dump, `trait_res=Some(Resolved(...))`):
- `SanitizeErr` ×2 — the trait IS local (`src/error.rs`'s own
  `trait SanitizeErr`), resolved correctly for both `impl SanitizeErr
  for io::Error` and `impl SanitizeErr for ignore::Error` (both report
  `self=Error` since I1 extracts `io::Error`/`ignore::Error`'s own last
  segment; their SELF type resolution is separately `Unresolved`,
  covered in the self-type breakdown below — the trait name resolving
  correctly and the self type resolving correctly are independent
  questions, and `resolve_implements_from_impl_facts` requires BOTH
  `Resolved` before it emits an edge, so neither of these 2 produces an
  edge).
- `Matcher` ×6 — all 6 `impl Matcher for <Type>` impls in
  `src/patterns.rs` (`RegexSet`, `GlobSet`, `GlobMatcher`,
  `FixedStrings`, `ExactStrings`, `MatchAll`); `Matcher` is a real
  local `trait_item`.

I1's own net effect on trait-RESOLUTION counts for this specific
corpus is zero (every `scoped_type_identifier` trait name `fd` happens
to reference at commit B — `FromArgMatches`, `Args`, `Display` on
`PathUrl`, `Visitor` — names a genuinely EXTERNAL trait, so each still
correctly resolves `Unresolved` after I1 rescues it from wrongly being
`None`). I1's real, measured effect here is purely on EXTRACTION (4
real trait impls that used to look inherent now correctly carry a
trait name at all — see the reconciliation above), not on the
Resolved/Unresolved split.

**The 15 `Unresolved`**, every one independently confirmed a REAL
external trait (never a resolver defect):

| Self type | Trait name (as extracted) | Why `Unresolved` |
|---|---|---|
| `DirEntry` | `PartialEq` | `std::cmp::PartialEq`, external |
| `DirEntry` | `Eq` | `std::cmp::Eq`, external |
| `DirEntry` | `PartialOrd` | `std::cmp::PartialOrd`, external |
| `DirEntry` | `Ord` | `std::cmp::Ord`, external |
| `DirEntry` | `Colorable` | `lscolors` crate's own trait, external |
| `SanitizedError` | `Display` | `std::fmt::Display`, external |
| `TestTime` | `Drop` | `std::ops::Drop`, external |
| `Token` | `Display` | `std::fmt::Display`, external |
| `PathUrl` | `Display` (I1: was `None` pre-fix, now `Some("Display")`) | `std::fmt::Display`, external — NEWLY extracted by I1, still correctly `Unresolved` |
| `Encoded` | `Display` | `std::fmt::Display`, external |
| `UppercaseVisitor` | `Visitor` (I1: was `None` pre-fix) | `regex_syntax::ast::Visitor`, external — NEWLY extracted by I1, still correctly `Unresolved` |
| `SanitizedStr` | `Display` | `std::fmt::Display`, external |
| `Batch` | `IntoIterator` | `std::iter::IntoIterator`, external |
| `Exec` | `FromArgMatches` (I1: was `None` pre-fix) | `clap::FromArgMatches`, external — NEWLY extracted by I1, still correctly `Unresolved` |
| `Exec` | `Args` (I1: was `None` pre-fix) | `clap::Args`, external — NEWLY extracted by I1, still correctly `Unresolved` |

8 Resolved + 15 Unresolved + 0 Ambiguous = 23 — reconciles exactly.

**0/23 are a genuine resolver defect.** Every `Unresolved` here is the
correct answer. I1's own effect on this corpus specifically: it
rescues 4 real trait impls from being mis-extracted as `None` (looking
inherent) into correctly carrying their real external trait name,
which then correctly resolves `Unresolved` for the identical reason
every other external-trait case already does — I1 did not, for this
specific corpus, turn any `Unresolved` into a `Resolved` (every
`scoped_type_identifier` trait name `fd` happens to use at commit B
names a genuinely external trait), but it DID fix the underlying
extraction defect regardless, and a future codebase whose own local
trait happens to be referenced via a scoped path (e.g.
`crate::patterns::Matcher`, covered by `impls.rs`'s own dedicated unit
test) would see a real `Resolved` outcome I1 enables.

**Self-type-resolution breakdown (all 44 impl facts, inherent + trait)
— this is where C1's fix has its real, measured effect:**

| `self_type_resolution` | Count |
|---|---:|
| `Resolved` | 37/44 |
| `Unresolved` | 5/44 |
| `Ambiguous` | **0/44** (was 7/42 before the C1 fix) |

**The C1 fix eliminated all 7 `Ambiguous` self-type resolutions.**
Before the fix, `Batch` (×2 facts), `BatchSender`, `ReceiverBuffer`,
`WorkerState` (all in `src/walk.rs`, a file that changed between
commit A and B), plus `Exec` (×2, `src/cli.rs`, also changed) were
`Ambiguous` — purely because `impl_facts` was resolved against
`final_snapshots`, which deliberately retains the OLD
(commit-A-rooted) snapshot for any unrenamed symbol in a changed file
as a lineage edge's own `from` endpoint, so the by-name lookup found
TWO same-named candidates (the old and new snapshot) for each of these
7. After the fix (resolving against `current_snapshots` instead, the
same pool `typed_edges` already uses), exactly one candidate exists
for each, and all 7 now correctly resolve `Resolved` — directly
confirmed in `b2-id-008-final-fix-run.txt`'s own full `impl_facts`
dump: `self=Batch ... self_res=Resolved(...)`, `self=BatchSender ...
self_res=Resolved(...)`, `self=ReceiverBuffer ... self_res=Resolved(...)`,
`self=WorkerState ... self_res=Resolved(...)`, `self=Exec ...
self_res=Resolved(...)` (×2) — every one, not a sample.

The 5 `Unresolved` self types are unaffected by either fix, same real
reasons as before:
- `Opts` (`src/cli.rs`) and `Config` (`src/config.rs`): both structs'
  own signature text exceeds `MAX_SIGNATURE_BYTES` (4096 bytes), so
  neither struct's own snapshot was ever promoted — a pre-existing
  extraction-size policy, not a defect in `impls.rs`.
- `RegexSet`, `GlobSet`, `GlobMatcher` (`src/patterns.rs`): all three
  are imported from EXTERNAL crates (`regex::bytes`, `globset`) — no
  local snapshot exists under those names.

**This Ambiguous-count drop (7/42 → 0/44) is the real, measured C1
fix number the brief asked for** — confirmed as a genuine fix, not an
assumed drop to zero: the 5 real `Unresolved` cases above remain
`Unresolved` (correctly so), demonstrating the fix narrows exactly the
false-ambiguity class C1 described without masking genuine unresolved
cases.

### `Implements` edge count (unchanged by C1/I1 on this specific corpus)

| Metric | Value |
|---|---:|
| `Implements` edges in the final graph (`outcome_b.graph`, `EdgeKind::Implements` count) | **3** |
| `extends_implements_edges` (the OLD resolver, standalone, full-index-only pool) | 0 (by design) |

Still **3**: `FixedStrings -> Matcher`, `ExactStrings -> Matcher`,
`MatchAll -> Matcher` (all in `src/patterns.rs`). Neither C1 nor I1
changes this count for THIS specific corpus: C1's own 7 rescued
self-types (`Batch`/`BatchSender`/`ReceiverBuffer`/`WorkerState`/`Exec`×2)
are all either inherent impls (no trait to pair with) or pair with a
trait that is itself `Unresolved` (`Exec`'s own `FromArgMatches`/`Args`),
so rescuing their SELF-type ambiguity alone does not create a new
`Implements` edge without the TRAIT side also resolving; I1's own 4
rescued trait names are all external traits (`Unresolved`), so they
likewise cannot pair into a new edge. The count staying at 3 across
both fixes is itself a confirming data point, not a sign either fix
did nothing: it demonstrates precisely which combination of
(self resolved, trait resolved) is required, and that this corpus's
own real trait-impl landscape (dominated by external-crate self types
and external-std/clap traits) happens to already have all of its
TRUE positive cases (`FixedStrings`/`ExactStrings`/`MatchAll` ->
`Matcher`) captured even before this fix wave.

**Before (B2-EVAL-005, pre-ImplFact) → after this whole fix wave: 0 →
3 real `Implements` edges.** Still a genuine, measured, non-zero
improvement from the original defect B2-ID-008 found.

### Associated-item coverage (Measured, post C1+I1 fix)

| Metric | Value |
|---|---:|
| `associated_items_total` (summed across all 44 `impl_facts`) | 126 |
| `Method` (has a `self`/`&self`/`&mut self` receiver) | 98 |
| `AssociatedFunction` (no receiver, factory-shaped) | 28 |

(126 = 98 + 28, reconciles; up from 124 = 96 + 28 before this fix wave
— the 2 additional associated items come from the 2 extra `ImplFact`s
C1/I1 together did not drop that the pre-fix run's different candidate
pool and extraction logic affected incidentally; not independently
decomposed further here since associated-item extraction itself was
untouched by either fix.)

### Superseded pre-fix-wave sections (kept verbatim for the record)

### Coding-agent Q&A re-check (Section H, same 7 questions B2-EVAL-005 asked)

Q1/Q2/Q5 (trait/impl questions) still print `[]` in this harness's own
UNCHANGED Section H code, because Section H's own pre-existing query
logic filters `graph.outgoing_edges/incoming_edges(..., Some(EdgeKind::Extends))`
— it was written when Rust's `Implements` edges did not exist at all,
and this task's own brief scoped the harness change to ADDING new
`ImplFact` metrics, not rewriting Section H's pre-existing query logic
(which is Extends-specific by original design, predating this fix,
and out of this task's own additive-only scope). The REAL answer is
now directly available via the new metrics block above and via
`graph.outgoing_edges(..., Some(EdgeKind::Implements))`: `FixedStrings`
and `ExactStrings`/`MatchAll` DO now have a real, correct `Implements`
edge to `Matcher` in the graph; `GlobMatcher`/`RegexSet`/`GlobSet` do
not, for the honest external-self-type reason documented above. This
is flagged explicitly rather than silently left looking like Q1/Q2/Q5
are still "Incorrect" in the B2-EVAL-005 sense — they are not; the
printed `[]` is an artifact of Section H's own pre-existing
`EdgeKind::Extends`-only query, not a renewed absence of the
relationship itself. A full Section H rewrite (and any resumption of
B2-EVAL-005 itself) is explicitly NOT part of this task, per the plan's
own "After Final Review" instruction not to resume B2-EVAL-005/start
B2-EVAL-006 without the user's explicit GO.

## Part 3: Full workspace test / clippy / deny confirmation

**`cargo test --release --workspace`:** ALL GREEN. Every crate's test
binary reported `0 failed`, including `codegraph-index`'s own lib
suite: **402 passed, 0 failed.** Task 4's own commit already recorded
395 → 401; this task's own mutation-triage regression test
(`resolve_impl_facts_the_one_file_tree_convenience_wrapper_actually_extracts_and_resolves`)
accounts for the remaining 401 → 402. Doc-tests across every
crate: 0 failed.

**`cargo clippy --workspace -- -D warnings`** (the project's own
standing gate, exact command per this task's own brief): **CLEAN.**
No warnings, no errors.

**`cargo clippy --release --workspace --all-targets -- -D warnings`**
(the stricter variant): surfaces **12 errors across 4 files**, all
PRE-EXISTING and UNRELATED to this plan's own work, none a new
regression — this corrects an earlier draft of this evidence file,
which under-counted this at "exactly ONE error" by only reading the
first error block printed before the build gave up on one crate; the
full count below was obtained by re-running the exact same command and
counting every `^error:` line in its complete output, then confirming
each location's content was byte-for-byte present at commit `eb1fb46`
(the commit immediately preceding this task's own work) via `git show
eb1fb46:<path>`:

| File | Line | Lint | Confirmed present at `eb1fb46`? |
|---|---:|---|---|
| `crates/policy/src/lib.rs` | 10 | `assertions_on_constants` (`assert!(true)`) | Yes — identical `fn crate_builds() { assert!(true); }` |
| `crates/domain/src/lib.rs` | 16 | `assertions_on_constants` (`assert!(true)`) | Yes — identical `fn crate_builds() { assert!(true); }` |
| `crates/storage/src/lib.rs` | 110 | `assertions_on_constants` (`assert!(true)`) | Yes — identical `fn crate_builds() { assert!(true); }` |
| `crates/codegraph-index/src/graph/store.rs` | 568 | `bool_assert_comparison` (`assert_eq!(x, true)`) | Yes — identical line, immediately following an existing `assert!(retrieved.status.is_confirmed())` at line 567 |
| `crates/codegraph-index/src/impls.rs` | 644, 663, 681 | `cloned_ref_to_slice_refs` (×3) | Yes — all three are PRE-EXISTING test code this task did not touch (verified at `eb1fb46`'s own line numbers 626/645/663, before this task's own added doc comment shifted later line numbers down); this task's OWN new test (added later in the same file) also clones into a slice literal but with TWO elements per array (`&[trait_snapshot.clone(), self_snapshot.clone()]`), which this specific lint does not flag (it only fires on a single-element array literal) |
| `crates/codegraph-index/src/imports.rs` | 608, 647, 677, 678, 694 | `cloned_ref_to_slice_refs` (×5) | Yes — this task never touched `imports.rs` at all; all 5 are identical, byte-for-byte, at `eb1fb46`'s own same line numbers |

3 + 1 + 3 + 5 = **12**, none introduced by this task's own diff (the 3
`impls.rs` locations predate this task's own edits to that file; the
other 9 are in files this task never modified at all: `policy`,
`domain`, `storage`, `codegraph-index/src/graph/store.rs`,
`codegraph-index/src/imports.rs`). `crates/domain` and `crates/policy`
(note: neither is `codegraph-domain`, a different crate in this
workspace) were both last touched at commits (`8261543` for `domain`)
entirely outside this plan's own commit range
(`93b560d`/`b805c84`/`ec1a47f`/`eb1fb46`/this task's own `a8e5b30`).
This mirrors Task 4's own already-documented finding that
`--all-targets` surfaces pre-existing unrelated issues the standing
non-`--all-targets` gate does not — the exact COUNT was wrong in this
file's own earlier draft, but the underlying classification (clean
under the standing gate; pre-existing, unrelated noise under the
stricter one) was and remains correct.

**`cargo deny check`:** **CLEAN.** `advisories ok, bans ok, licenses
ok, sources ok`.

## Summary

| Check | Result |
|---|---|
| Campaign 1 (`impls.rs`) | 0 genuine missed mutants (25 caught, 1 equivalent w/ written justification, 8 unviable) |
| Campaign 2 (`extends_implements.rs`, full file — no Rust-specific surface remained to scope narrowly) | 0 missed mutants at all (31 caught, 4 unviable) |
| New regression tests added | 1 (`impls.rs`, closing the `resolve_impl_facts` gap) |
| Real impl blocks in `src/` (corrected denominator) | 45 (not 41 — see "Denominator correction" above: a literal top-level-only grep today gives 42, not 41; the real reconciliation is B2-ID-008's own original manual count being short by exactly one inherent impl (18 vs. 19) and not counting 3 impls nested in `#[cfg(test)]` modules at all) |
| `ImplFact`s extracted | 42/45 (3 dropped: external/reference self-type shapes, documented in-scope limitation) |
| Trait impls (`trait_type_text: Some`) | 16/42 |
| Trait-resolution: Resolved / Unresolved / Ambiguous | 6/16, 10/16, 0/16 (all 10 Unresolved are real external-trait cases, not defects) |
| Self-type-resolution: Resolved / Unresolved / Ambiguous | 30/42, 5/42, 7/42 (all explained: oversized-signature rejection, external self-type, or rename-across-lineage ambiguity shared with `ImportFact`) |
| `Implements` edges | 0 → 3 (real, verified-correct edges) |
| Associated items | 124 total (96 `Method`, 28 `AssociatedFunction`) |
| `cargo test --release --workspace` | GREEN (402/402 in codegraph-index, 0 failed workspace-wide) |
| `cargo clippy --workspace -- -D warnings` | CLEAN |
| `cargo clippy --release --workspace --all-targets -- -D warnings` | 12 pre-existing, unrelated errors across 4 files (`crates/policy`, `crates/domain`, `crates/storage`, `crates/codegraph-index/src/graph/store.rs` ×1, `impls.rs` ×3, `imports.rs` ×5) — none introduced by this task, confirmed individually against `eb1fb46` |
| `cargo deny check` | CLEAN |

Scratchpad clone of `sharkdp/fd` deleted after this task's data was
captured (confirmed, no residual clone left on disk).

## Summary addendum: final-review fix wave (C1 + I1 + I2), Measured

The table above reflects Task 5's own closing state, BEFORE the final
whole-branch review's fixes. This addendum reflects the state AFTER
C1 (incremental `impl_facts` resolved against `current_snapshots`, not
`final_snapshots`) and I1 (`scoped_type_identifier` trait/self-type
fields extract their last segment) — see "### SUPERSEDED by the
final-review fix wave" and "### After the final-review fix wave"
above for the full reconciliation.

| Check | Result |
|---|---|
| Real impl blocks in the repository at commit B (reconciled: 42 top-level in `src/` + 3 nested in `src/` `#[cfg(test)]` modules + 1 in `tests/`) | **46** |
| `ImplFact`s extracted | 44/46 (2 dropped whole: `impl SanitizeErr for &str` — `reference_type` self; `impl From<ExitCode> for i32` — `primitive_type` self; both documented in-scope limitations) |
| Trait impls (`trait_type_text: Some`) | 23/44 (up from 16/42 — I1 rescues 5 real trait impls, `clap::FromArgMatches`/`clap::Args` on `Exec`, `fmt::Display` on `PathUrl` AND `Encoded` (the undercounted 5th case I2 flagged), `regex_syntax::ast::Visitor` on `UppercaseVisitor`, from wrongly collapsing to `None`) |
| Trait-resolution: Resolved / Unresolved / Ambiguous | 8/23, 15/23, 0/23 (all 15 Unresolved are real external-trait cases; I1's 4 rescued names are each genuinely external for this specific corpus, so I1 fixes extraction without changing this split) |
| Self-type-resolution: Resolved / Unresolved / **Ambiguous** | 37/44, 5/44, **0/44 (down from 7/42 — the C1 fix's own real, measured number)** |
| `Implements` edges | 0 → 3 (unchanged by C1/I1 on this specific corpus; same 3 true-positive edges as Task 5's own run — see "`Implements` edge count" above for why) |
| Associated items | 126 total (98 `Method`, 28 `AssociatedFunction`) |
| Raw harness output for this re-run | committed verbatim: [`b2-id-008-final-fix-run.txt`](b2-id-008-final-fix-run.txt) |
| `cargo test --release --workspace` | GREEN (408/408 in codegraph-index lib tests, 0 failed workspace-wide) — see the fix wave's own final report for the exact before/after test counts across C1/I1/I3/I4/M1/M2 |
| `cargo clippy --workspace -- -D warnings` | CLEAN |

Scratchpad clone of `sharkdp/fd` used for this re-measurement deleted
after this task's data was captured (confirmed, no residual clone left
on disk).
