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

### Denominator correction (Measured, this task, independent of the fix)

Direct `grep` of `src/` at commit B, counting BOTH top-level `^impl`
lines AND impl blocks indented inside a nested scope (e.g.
`#[cfg(test)] mod tests { impl TestTime { .. } impl Drop for TestTime
{ .. } }` in `src/filter/time.rs`, and `#[cfg(test)] mod test { impl
fmt::Display for Encoded { .. } }` in `src/hyperlink.rs` — both real,
syntactically valid impl blocks tree-sitter parses regardless of the
`#[cfg(test)]` attribute, which is not evaluated at parse time),
confirms **45 real impl blocks exist in `src/` at commit B**, not 41.
This 45 figure (and the `ImplFact` extraction/resolution numbers built
on it below) is unaffected by the correction in this section — only
the EXPLANATION of the original 41 count is corrected here.

**Corrected explanation (this file's own earlier draft got this wrong
— a top-level-only `grep -n "^impl" src/` was re-run during this
correction and gives 42, not 41, so "a top-level-only grep that missed
3 nested impls" does not arithmetically reconcile: 42 − 3 ≠ 41, and a
top-level-only grep by definition would not have found the 3 nested
ones to miss in a way that produces 41 either).** The real
reconciliation: counting the SAME 42 top-level-only impl lines by
trait-impl vs. inherent-impl gives **23 trait + 19 inherent = 42**,
confirmed by direct `grep -c " for "` / `grep -vc " for "` against the
identical 42-line list. [B2-ID-008](../findings/B2-ID-008.md)'s own
classification table reported **23 trait + 18 inherent = 41** — the
trait-impl count (23) matches exactly; the discrepancy is entirely in
the INHERENT count, short by exactly one (18 vs. the real 19). This is
consistent with B2-ID-008's own original count having been a manual,
file-by-file enumeration (its own table cites specific line numbers
per file, e.g. "`DirEntry`: lines 26/118/125/127/134/141"), not a
grep-based count at all, that missed exactly one real inherent `impl`
block somewhere among the 19 in `src/` — this task did not re-identify
WHICH specific one was missed (the original investigation's own raw
notes, if any existed beyond the published table, are not available to
re-derive it from), so this is reported as an honest "off-by-one in a
prior manual count, not a systematic methodology difference" finding,
not a fully pinpointed root cause. It also did not count the 3 nested
test-module impls at all (41/42 vs. 45 is a 1-off difference, not a
3-off one), consistent with a manual top-level walk of each file that
never looked inside `#[cfg(test)]` modules in the first place. This
correction is reported plainly because the brief explicitly asked not
to assume the after
numbers would match the before numbers exactly.

### After (Measured, post-fix, same commit pair, this task's own run)

**Pipeline-level counts** (`IndexMetrics` from `CodeGraphIndexer::observe_commit`,
incremental A→B call):

| Metric | Value |
|---|---:|
| `raw_impl_statements_extracted` (this call, i.e. commit B's changed files) | 15 |
| `raw_impl_statements_reused` (carried forward from untouched files) | 27 |
| `impl_facts_resolved` / `impl_facts_total` (final published state) | **42** |

**42/45 real impl blocks in `src/` produced an `ImplFact`.** The 3
missing are a confirmed, documented, in-scope limitation of
`base_type_name` (which only recognizes a `type_identifier` or
`generic_type` self-type-field shape, per `impls.rs`'s own module doc
comment: "out of this plan's own scope... prefer omitting the fact
over guessing at an unconfirmed shape"), not a new defect:

- `impl SanitizeErr for &str` (`src/error.rs:20`) — self type is a
  `reference_type` node (`&str`), not `type_identifier`/`generic_type`.
- `impl SanitizeErr for io::Error` (`src/error.rs:28`) — self type is a
  `scoped_type_identifier` (`io::Error`).
- `impl SanitizeErr for ignore::Error` (`src/error.rs:36`) — self type
  is a `scoped_type_identifier` (`ignore::Error`).

All three are dropped as a WHOLE impl block (the `let Some(self_type_text)
= base_type_name(...) else { return; }` guard in `collect_rust_impl`),
never partially extracted — confirmed directly: none of the three
appears anywhere in the harness's own full `impl_facts` dump, not even
as a `trait_type_text: None` row.

Separately, the TRAIT name field has the identical, symmetric
limitation: `impl clap::FromArgMatches for Exec`, `impl clap::Args for
Exec`, `impl fmt::Display for PathUrl`, and `impl regex_syntax::ast::Visitor
for UppercaseVisitor` all have a `scoped_type_identifier` TRAIT name,
so each is extracted as an `ImplFact` with `trait_type_text: None` (the
self type still resolves normally) rather than being dropped entirely
— confirmed directly in the dump (`Exec`/`PathUrl`/`UppercaseVisitor`
each appear with `trait=None`, alongside their OWN separate inherent
impl facts where applicable).

**Trait impl vs. inherent split, with full numerator/denominator detail:**

| Split | Count |
|---|---:|
| `impl_facts_total` | 42 |
| `trait_impls` (`trait_type_text: Some(_)`) | 16/42 |
| `inherent_impls` (`trait_type_text: None`) | 26/42 |

16, not 23 (B2-EVAL-005's own pre-fix count of real `impl X for Y`
syntax occurrences), because 4 of the 23 real `impl Trait for Type`
blocks have a scoped (`a::b`) trait name and are extracted with
`trait_type_text: None` per the limitation above (`clap::FromArgMatches`,
`clap::Args`, `fmt::Display` on `PathUrl` only — `Display` on `Token`/
`SanitizedError`/`SanitizedStr` is a BARE `type_identifier`, via `use
std::fmt::Display`, and DOES get `trait_type_text: Some("Display")` —
and `regex_syntax::ast::Visitor`), and 3 more of the 23 are the
`SanitizeErr`-for-external-self-type trio dropped entirely above
(23 − 4 − 3 = 16, confirmed by direct reconciliation against the real
source: 23 real `impl X for Y` lines in `src/`, minus the 4 scoped-trait-name
ones now `None`, minus the 3 dropped-whole ones, equals exactly 16 —
the harness's own measured number, independently re-derived from first
principles, not merely trusted).

**Trait-resolution breakdown (of the 16 trait impls that DID get a
`trait_type_text`):**

| `trait_resolution` | Count |
|---|---:|
| `Resolved` | 6/16 |
| `Unresolved` | 10/16 |
| `Ambiguous` | 0/16 |

The 6 `Resolved`: all 6 are `impl Matcher for <Type>` (`RegexSet`,
`GlobSet`, `GlobMatcher`, `FixedStrings`, `ExactStrings`, `MatchAll`),
one trait name (`Matcher`) resolved `Resolved` 6 separate times
because `Matcher` is a real, local `trait_item` in `src/patterns.rs` —
confirmed directly. (Whether the SELF type of each also resolves is a
separate question, covered in the self-type-resolution breakdown
below — 3 of these 6 have an external-crate self type and so do NOT
produce an `Implements` edge despite the trait itself resolving.)

The 10 `Unresolved`, with the REAL reason for each, reported honestly
per the brief's explicit instruction not to assume the split should
match 23 minus-something exactly — this corpus has several trait impls
targeting EXTERNAL `std` traits this repository never defines itself,
which correctly resolve `Unresolved`, NOT a defect:

| Self type | Trait name (as extracted) | Why `Unresolved` |
|---|---|---|
| `DirEntry` | `PartialEq` | `std::cmp::PartialEq`, external, never locally defined |
| `DirEntry` | `Eq` | `std::cmp::Eq`, external |
| `DirEntry` | `PartialOrd` | `std::cmp::PartialOrd`, external |
| `DirEntry` | `Ord` | `std::cmp::Ord`, external |
| `DirEntry` | `Colorable` | external (`lscolors` crate's own trait, not locally defined in this repo) |
| `SanitizedError` | `Display` | `std::fmt::Display`, external |
| `TestTime` | `Drop` | `std::ops::Drop`, external |
| `Token` | `Display` | `std::fmt::Display`, external |
| `SanitizedStr` | `Display` | `std::fmt::Display`, external |
| `Batch` | `IntoIterator` | `std::iter::IntoIterator`, external |

Every one of these 10 is a real, externally-defined trait this
repository imports and implements against but never declares itself —
exactly the shape the brief anticipated (`Display`/`Drop`/`Eq`/
`PartialEq`/`PartialOrd`/`Ord`/`IntoIterator` all appear, plus
`Colorable` from the `lscolors` crate as one not explicitly
anticipated but the identical shape). **0/16 are a genuine resolver
defect** — every `Unresolved` here is the CORRECT answer, confirmed by
direct source reading of each self type's real defining file, not
assumed from the trait name alone.

**Self-type-resolution breakdown (all 42 impl facts, inherent + trait):**

| `self_type_resolution` | Count |
|---|---:|
| `Resolved` | 30/42 |
| `Unresolved` | 5/42 |
| `Ambiguous` | 7/42 |

The 5 `Unresolved` self types, with real reasons:
- `Opts` (`src/cli.rs`) and `Config` (`src/config.rs`): both structs'
  own `struct_item` signature text exceeds this pipeline's
  pre-existing `MAX_SIGNATURE_BYTES` (4096 bytes) extraction limit
  (`Opts`'s own struct body is 24,620 bytes — a `clap`-derive struct
  with dozens of heavily-doc-commented fields; `Config`'s is 4,398
  bytes), so NEITHER struct's own `Struct` snapshot was ever promoted
  in the first place — confirmed directly measuring both structs'
  real byte spans in the real source, and cross-checked against the
  harness's own independently-reported `extraction_rejections: 4` in
  Section B. This is a PRE-EXISTING extraction-size policy interacting
  with the new resolver, not a defect in `impls.rs` itself: there
  genuinely is no snapshot to resolve `Opts`/`Config` against.
- `RegexSet`, `GlobSet`, `GlobMatcher` (`src/patterns.rs`'s own
  `Matcher` trait impls): all three are imported from EXTERNAL crates
  (`regex::bytes::{RegexSet}`, `globset::{GlobSet, GlobMatcher}`,
  confirmed via `src/patterns.rs`'s own `use` statements) — correctly
  `Unresolved` because no local snapshot named `RegexSet`/`GlobSet`/
  `GlobMatcher` exists in this repository at all. The TRAIT side of
  these same three impls (`Matcher`) correctly resolves `Resolved`,
  since `Matcher` genuinely IS local.

The 7 `Ambiguous` self types — a real, honest, and informative
real-repository finding about how `ImplFact` resolution interacts with
this project's PRE-EXISTING incremental-pipeline design, not a new
B2-ID-008-specific defect:
`Batch`, `Batch` (again, the `IntoIterator` trait impl of the same
type), `BatchSender`, `ReceiverBuffer`, `WorkerState` (all in
`src/walk.rs`, a file that genuinely changed between commit A and B —
confirmed via `git diff`), plus `Exec` (×2, `src/cli.rs`, also a
changed file). Root cause, confirmed directly by reading
`indexer.rs`'s own incremental resolution code
(`crates/codegraph-index/src/incremental/indexer.rs` lines ~601-657):
`resolve_raw_impl_statements` (mirroring `resolve_raw_import_statements`
exactly, by design) is resolved against `final_snapshots`, which
DELIBERATELY includes historical, lineage-superseded snapshots from
the PRIOR commit (added back in specifically so a `LineageEdge`'s
`from` endpoint remains present in the stored graph — see that
function's own doc comment). For any struct in a CHANGED file whose
name is unchanged across the rename/edit (e.g. `WorkerState` in
`src/walk.rs`, whose own field type changed from `Vec<Regex>` to
`Patterns` but whose struct NAME did not), BOTH the old
(commit-A-rooted) and new (commit-B-rooted) snapshots end up in
`final_snapshots` sharing the same name, making `resolve_target_by_name`'s
by-name lookup find 2 candidates → `Ambiguous`, per this project's own
"no edge beats a false edge" discipline (never arbitrarily picked).

**This is NOT a new defect introduced by this fix.** `import_facts`
(line 638 of `indexer.rs`) is resolved against the exact SAME
`final_snapshots` pool, so `ImportFact`'s own `Internal` resolution
faces the identical by-name-ambiguity risk under the identical real
corpus conditions — confirmed by direct code reading, not assumed.
Notably, `typed_edges` (`Calls`/`Extends`/`Contains`/`Instantiates`,
line 575) are resolved against `current_snapshots` ONLY (deliberately
excluding historical snapshots, per that line's own doc comment citing
a real bug this exclusion was added to fix). `ImplFact` therefore
faithfully mirrors `ImportFact`'s own existing, already-reviewed
resolution discipline exactly — including this shared, pre-existing
quirk — rather than being a subtly different reimplementation. This is
exactly the property the plan's own Final Review is asked to verify,
and this real-repository run is direct, positive evidence that it
holds. Fixing this ambiguity-under-rename quirk, if ever desired, is a
pre-existing `ImportFact`-and-`ImplFact`-wide concern, out of scope for
B2-ID-008.

### `Implements` edge count

| Metric | Value |
|---|---:|
| `Implements` edges in the final graph (`outcome_b.graph`, `EdgeKind::Implements` count) | **3** |
| `extends_implements_edges` (the OLD resolver, `resolve_extends_implements`, run standalone over the full-index-only snapshot pool, no incremental lineage pool) | 0 (by design — see below) |

**3 `Implements` edges**, corresponding to the 3/16 trait impls whose
BOTH self type AND trait resolved `Resolved` AND whose self type is
not one of the `Ambiguous`/`Unresolved` cases above:
`FixedStrings -> Matcher`, `ExactStrings -> Matcher`, `MatchAll ->
Matcher` (all in `src/patterns.rs`) — `RegexSet`/`GlobSet`/`GlobMatcher`
also implement `Matcher` but their own self type is `Unresolved`
(external crate type), so per `resolve_implements_from_impl_facts`'s
own documented behavior (never arbitrarily resolve a partial match),
they correctly produce NO edge.

`resolve_extends_implements`'s own standalone `extends_implements_edges: 0`
(printed separately in the harness's pre-existing Section D, run over
`snapshots` — the full-index-only pool, NOT `outcome_b`'s incremental
`CodeGraphIndexer` pool) is CORRECT BY DESIGN, not a residual gap: per
Task 3's own fix, `resolve_extends_implements` deliberately produces
zero `Extends`/`Implements` edges for Rust now, since `impls.rs`'s
`ImplFact` mechanism is the real one. The 3 real `Implements` edges
live in `outcome_b.graph` (the `CodeGraphIndexer`-built graph, which
DOES fold `resolve_implements_from_impl_facts`'s own edges into
`typed_edges` per Task 4's wiring) — confirmed directly by filtering
`outcome_b.graph.all_edges()` for `EdgeKind::Implements` and getting 3,
not 0. These are two different pools/call paths in the SAME run, both
correctly reported, and this evidence file makes the distinction
explicit so neither number is misread as contradicting the other.

**Before → after: 0 → 3 real `Implements` edges**, on this specific
commit-B snapshot. This is a genuine, measured, non-zero improvement —
small in absolute count because this corpus's own real trait-impl
landscape is dominated by either external-crate self types or
external-std traits (both correctly `Unresolved`, not a defect), not
because the fix itself is incomplete for the shapes it IS scoped to
handle (every one of the 3 is a correct, verified-against-real-source
edge).

### Associated-item coverage

| Metric | Value |
|---|---:|
| `associated_items_total` (summed across all 42 `impl_facts`) | 124 |
| `Method` (has a `self`/`&self`/`&mut self` receiver) | 96 |
| `AssociatedFunction` (no receiver, factory-shaped) | 28 |

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
