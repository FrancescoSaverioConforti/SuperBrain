# Mutation testing policy

Quality & Semantic Reliability Hardening, Phase 1, deliverable 5.

## Why this exists

This project has already run `cargo mutants` as part of closing
B2-ID-009, B2-ID-010, and B2-ID-011, applying the same classification
discipline informally each time. This document formalizes that
discipline as an explicit, named policy, so future work does not have
to re-derive the rules from reading three prior finding docs.

## The five outcome categories

Every mutant `cargo mutants` generates for a scoped file set resolves
to exactly one of these. A mutation run's own report is not "done"
until every mutant has been placed into one of these five — an
unclassified mutant is treated as `genuine missed` until proven
otherwise.

1. **Caught** — the existing test suite fails when this mutant is
   applied. No action needed; this is the expected, healthy outcome
   for the overwhelming majority of mutants in correctness-critical
   code.

2. **Unviable** — the mutated code does not compile at all (e.g.
   `cargo mutants` substituting `Default::default()` for a return type
   with no `Default` impl). Not a test-coverage gap; the mutant itself
   is not a meaningful program. No action needed.

3. **Equivalent** — the mutated code compiles and the test suite still
   passes, but ONLY because no real input can ever reach the branch
   where the mutation and the original would actually disagree. This
   is the category that requires the most discipline to classify
   honestly:
   - An equivalent-mutant claim must be backed by a STRUCTURAL
     argument about the real grammar/domain, never by "I couldn't
     think of a test." Every equivalent-mutant classification this
     project has made so far cites one of two argument shapes:
     - a grammar-level impossibility (e.g. B2-ID-010's
       `kind == "partial" && text == "partial"` check: the `partial`
       keyword is a fixed lexical terminal, so no valid parse can ever
       produce a node where the kind and text checks would disagree —
       `&&` and `||` are provably behaviorally identical on every
       reachable AST);
     - an invariant of the data structure itself (e.g. B2-ID-010's
       `<`/`<=` and `>`/`>=` comparisons on AST node `start_byte`
       values: two distinct sibling nodes in one parse tree can never
       share the exact same byte offset, so the equality case that
       would distinguish strict from non-strict comparison is
       structurally unreachable).
   - **Never** write an artificial test for impossible/synthetic input
     just to flip an equivalent mutant from "equivalent" to "caught."
     That produces a test that asserts nothing about real program
     behavior and inflates the coverage number dishonestly. The
     correct action for a genuinely equivalent mutant is to document
     the reasoning INLINE in the source code next to the logic in
     question (not just in a report), so a future reader does not
     rediscover the same "why doesn't this have a test" question.
   - An "equivalent" classification applied without this structural
     argument is not actually equivalent — treat it as `genuine
     missed` until the argument is made explicit and checked.

4. **Genuine missed** — the mutated code compiles, the test suite
   still passes, AND a real input exists that would make the mutant
   and the original disagree. This is a real test-coverage gap and
   must be closed by adding a test — never by weakening the gate.

5. **Timeout** — the mutant caused the test suite to hang or exceed
   the configured time budget rather than failing cleanly (e.g. an
   introduced infinite loop). Investigate: a timeout can mask a real
   `genuine missed` mutant (the program never reaches the point where
   it would otherwise fail) and should not be silently treated as
   equivalent to a pass.

## The gate

For every correctness-critical path (currently: every typed-edge
resolver in `edges/`, every structural-fact extractor/resolver in
`imports.rs`/`impls.rs`/`csharp_declarations.rs`, and the per-language
symbol extractors in `extract/`):

> **`genuine missed` (correctness-relevant) = 0 is a hard gate,
> not a target to approach.**

"Correctness-relevant" explicitly excludes mutations to: `#[derive(Debug)]`-only
code paths, logging/diagnostic strings that carry no decision logic,
and test-module code itself (mutating a test's own assertions is not
meaningful — `cargo mutants` is scoped to exclude `#[cfg(test)]`
modules by default in this project's own usage so far; confirm this
default holds before trusting a mutation run's own `0 missed` claim).

A finding/fix is not considered CLOSED (per this evaluation series'
own closure discipline) until its own mutation run reports `0 genuine
missed` for the files it touched, with every `equivalent` claim
carrying the structural argument above, documented inline.

## Known blind spot: code under `tests/` is never mutated

Confirmed directly (this Phase 1 pass's own experience, not a
theoretical concern): `cargo mutants --file <path-under-tests/>`
reports **`Found 0 mutants to test`** for any file under a crate's
`tests/` directory (a Rust integration-test binary), regardless of how
much real, non-trivial logic that file contains. `cargo-mutants`'
own default scope is `src/` — it does not mutate integration-test
crates at all, by design (mutating test-only code is not meaningful
in the tool's own model, matching the "test-module code itself" line
already in this document's "correctness-relevant" exclusion above —
but that line was written with `#[cfg(test)]` unit-test modules in
mind, not realizing it ALSO exempts entire integration-test binaries
that contain real, load-bearing comparison logic).

This is a genuine, currently-unaddressed gap for code like
`full_incremental_equivalence.rs`'s own `snapshot_keys`/`edge_keys`/
`import_fact_keys`/`impl_fact_keys`/`assert_full_equals_incremental`
functions — real correctness-critical logic (a bug in `edge_keys`
could make the equivalence gate silently blind to a true divergence)
that currently has ZERO mutation-coverage verification, and cannot
get any under this policy's own standard command as written.

**Mitigation in place today, not a full substitute**: this exact risk
is why `full_incremental_equivalence.rs` includes
`the_equivalence_helper_itself_detects_an_injected_divergence` — a
manually-constructed, single adversarial case proving the helper
catches AT LEAST ONE real kind of divergence. This is strictly weaker
than mutation testing (one hand-picked case vs. systematic exploration
of every mutation of the comparison logic) and should not be
represented as an equivalent substitute.

**Not yet solved, named here as real future work**: the two
structural options are (1) move correctness-critical comparison
logic like `edge_keys`/`snapshot_keys` OUT of `tests/` and into a
`#[cfg(test)]`-gated module inside the crate's own `src/` tree (where
`cargo mutants` WOULD reach it), re-exporting it for the integration
tests to use, or (2) extend this project's own `cargo-mutants`
configuration (if the tool supports it) to include specific `tests/`
files. Neither is implemented in this Phase 1 pass — this is recorded
as a residual risk, not resolved.

## Scope discipline

Mutation testing is run SCOPED to the specific file(s) a fix actually
changed (`cargo mutants --file <path> [--file <path> ...]`), never
the whole workspace by default — a whole-workspace run is
prohibitively slow and dilutes signal with mutants in code the current
fix did not touch and is not responsible for. A whole-crate or
whole-workspace baseline run is a separate, periodic health-check
activity (not yet scheduled as part of this Phase 1 pass — see the
gap inventory's own residual-risk section).

## Known run-to-run variance

A real observation from this project's own history (B2-ID-010's
independent review): re-running `cargo mutants` on the IDENTICAL file
scope with no code changes between runs can produce a 1-mutant
difference in the caught/unviable split (e.g. 74/10 vs. 73/11), while
the `missed`/`timeout`/total counts stay identical. This is believed
to be build-flakiness or ordering-sensitivity in how `cargo mutants`
classifies a mutant that sits exactly on that boundary, not a real
discrepancy in the code under test. When comparing two mutation runs:
- Treat a difference confined to the caught/unviable split, with
  identical missed/timeout/total, as this known variance — not a
  regression.
- Treat ANY difference in the `missed` count, the `timeout` count, or
  the total mutant count as a real discrepancy requiring
  investigation, never dismissed as "probably the same variance."

## Standard command

```
cargo mutants -p codegraph-index --file <path/to/file.rs> [--file <path/to/other.rs>] --no-shuffle
```

`--no-shuffle` is used throughout this project's own history for
reproducible mutant ordering across runs, which makes a genuine
discrepancy easier to spot against a prior run's own output.
`mutants.out/` (the tool's own scratch output directory) is always
deleted after a run — it is never committed, per this project's
existing `.gitignore`-equivalent discipline (confirmed by every prior
fix's own commit history showing no `mutants.out/` artifacts).
