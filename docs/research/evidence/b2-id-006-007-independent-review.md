# B2-ID-006 / B2-ID-007 combined independent review

**Reviewer:** fresh `agent-protocols:code-reviewer` agent, no prior
involvement in this finding's own implementation work (dispatched from
this session, operating independently against the uncommitted working
tree).
**Date:** 2026-11-06
**Scope:** all uncommitted changes implementing B2-ID-006's fix
(`SymbolKind::can_originate_calls`, the nested-scope span boundary in
`codegraph-index::edges::calls`) and B2-ID-007's two fixes
(`resolve_extends_implements`'s parse-tree-local node-kind boundary,
`resolve_tests_for`'s cross-snapshot span-boundary check), plus the
non-destructive errata added to B2-EVAL-001/002.
**Verdict:** **APPROVE** — no Critical or Important issues found, no
changes required before merge.

## Full review report (verbatim)

## Review Summary

**Verdict:** APPROVE

**Overview:** This is an independent review of the uncommitted B2-ID-006 (`resolve_calls` over-attribution) and B2-ID-007 (same defect family in `resolve_extends_implements`/`resolve_tests_for`) fixes in the SuperBrain codegraph pipeline, worktree `C:\Users\fsave\Desktop\Python_code\SuperBrain\.worktrees\b2-code-graph`. Both fixes are well-designed, genuinely cross-language, backed by real-parse (not placeholder) regression tests, mutation-tested to closure, and accompanied by honest, internally-consistent evaluation errata. I verified every claim empirically rather than trusting narrative — ran the full workspace test suite, clippy, and cargo-deny myself, and traced the incremental-indexing invocation path directly in source.

---

### Checklist-by-checklist findings

**1. Fix is NOT Python-specific — PASS.**
Cross-referenced each of the 6 per-language boundary-kind lists in `extends_implements.rs` against `kind_mapping.rs`'s own exhaustive `map_kind_hint` table (`crates/codegraph-index/src/snapshot/kind_mapping.rs`):
- Python: `["class_definition"]` — matches mapping table exactly.
- C#: `["class_declaration", "interface_declaration"]` — matches.
- C++: `["class_specifier", "struct_specifier"]` — matches.
- JS: `["class_declaration"]` — matches.
- TS/Tsx: `["class_declaration"]` — matches.
- Rust: `["impl_item"]` — NOT in `kind_mapping.rs`'s table at all, but this is correctly documented and tested: `impl` blocks are never extracted as their own `RawSymbolCandidate` (no `impl_item` mapping exists), and a Rust `impl` block cannot syntactically contain another `impl_item` as a direct child, so there is no real nested-impl scenario to defend against (`rust_impl_trait_is_unaffected_by_the_boundary_fix` test, `extends_implements.rs:824-852`, confirms no regression). Not a gap — correctly scoped.
All 6 collectors have real-parse regression tests (`b2_id_007_nested_declaration_ownership` module, `extends_implements.rs:683-866`) confirming the boundary holds per-language, not merely asserted.

**2. Nested functions handled correctly — PASS.**
Traced `call_site_belongs_to_a_more_nested_caller` (`calls.rs:280-288`) against the doubly-nested `outer`→`middle`→`inner` case: the function checks `strictly_contains(caller, candidate)` for ANY other caller-eligible snapshot nested inside `caller`'s span that also contains the call site. For `caller = outer`, `middle` is found nested and contains the call site position → `outer` is excluded. On the next outer-loop iteration with `caller = middle`, `inner` is found nested and contains the position → `middle` is also excluded. Only when `caller = inner` does no nested candidate exist → `inner` is attributed. The test `python_doubly_nested_function_call_is_owned_by_the_innermost_function_only` (`calls.rs:1719-1730`) exercises exactly this shape via real extraction (not a placeholder body) and asserts `callers_of(&edges, target, &snapshots) == vec!["inner"]`, explicitly checking neither `outer` nor `middle` appear. Genuinely proves the claim, not a weaker check.

**3. Container symbols never originate spurious relations — PASS.**
`SymbolKind::can_originate_calls()` (`kind.rs:176-192`) is an exhaustive match with no wildcard/default arm — the Rust compiler itself enforces exhaustiveness over every current variant, and `exactly_function_and_method_can_originate_calls` (`kind.rs:297-327`) independently restates this as an explicit allow/deny list covering all 12 variants. The `Field` classification (`false`) is justified by a real, checkable argument, not merely asserted: confirmed via `extract/python.rs`'s own extraction logic (lines 68-106) that a `Field` is only ever produced from a `self.X = <rhs>` assignment, and `self` is only bound as a parameter inside a method body — so a `Field`'s span is structurally always nested inside its enclosing method. The doc comment also cites a concrete synthetic reproduction (`self.x = f(g())`) showing what would go wrong if `Field` were eligible. The `Const`/`Macro` classification (`false`, "conservative default, not currently producible") is honestly qualified as undecided-pending-real-data rather than claimed as a settled semantic answer — confirmed no extractor produces either kind via `kind_mapping.rs`'s own exhaustive table. This is appropriately hedged, not overclaimed.

**4. Real executable callers continue to work — no silent loss of legitimate calls — PASS, and well-tested.**
`strictly_contains` (`calls.rs:298-302`) correctly uses strict containment: `outer_span.start <= inner_span.start && inner_span.end <= outer_span.end && (outer_span.start, outer_span.end) != (inner_span.start, inner_span.end)` — the explicit tuple inequality excludes an identical-span candidate from "containing" itself, so a caller is never excluded from eligibility for its own call. I read this directly, not inferred from a comment.
`python_outer_own_call_plus_nested_call_are_each_owned_by_their_real_caller` (`calls.rs:1634-1654`) constructs `outer` calling `first()` directly in its own body (outside `inner`'s span) while `inner` (nested inside `outer`) calls `second()`, and asserts `outer→first` and `inner→second` both resolve with no cross-contamination — this is a real, meaningful proof that a genuine direct call by the outer function survives the fix.
`rust_outer_call_immediately_adjacent_to_a_nested_functions_closing_brace_still_resolves` (`calls.rs:1657-1684`) is a genuine mutation-killing test: it pins the exact byte position where `<` and `<=` diverge (`fn outer(){fn inner(){}other();}`, where `other()`'s call site starts at exactly `inner.span().end`), and the finding doc (`B2-ID-006.md:225-241`) documents that this test was specifically added after a real `cargo-mutants` run caught the `<`/`<=` boundary as a missed mutant on the first pass, then closed on the second. This is strong, verifiable evidence the test is load-bearing, not decorative.

**5. No hardcoded repo-specific behavior — PASS.**
Grepped all resolver/logic files (`calls.rs`, `extends_implements.rs`, `tests_for.rs`, `kind.rs`) for repo-specific names (`RNNModel`, `TransformerModel`, `CommentsRepository`, `validate`) outside of comments/doc-comments: zero matches in actual logic. All such references found elsewhere are confined to test fixtures, doc comments citing real-world evidence, or the standalone diagnostic `examples/` harnesses (`b2_id_006_audit.rs`, `b2_smoke_test_backend_api.rs`), which are explicitly documented as throwaway verification tools, not production code.

**6. Determinism — PASS.**
`calls.rs` has `result_is_deterministic_regardless_of_input_snapshot_order` (`calls.rs:1794-1809`) and `extends_implements.rs` has `result_is_deterministic_regardless_of_snapshot_order` (`extends_implements.rs:855-865`) — both genuinely reverse the snapshot pool (`reversed.reverse()`) and compare results against the forward-order run, not merely re-running the same input twice. Names differ slightly from the checklist's exact wording but the tests do what was asked.

**7. Compatibility with Task 9.5/incremental indexing — PASS, verified empirically in source, not guessed.**
Traced both code paths in `crates/codegraph-index/src/incremental/indexer.rs`:
- `full_reindex` (line 409) calls `index_files` with the complete file list; internally (`pipeline.rs:131`) this calls `resolve_all_typed_edges(&snapshots)` against the full snapshot set.
- `incremental_reindex` (line 475) calls `index_files` with ONLY `files_to_reparse` (changed files) — but critically, the `typed_edges` computed by that narrower call is **discarded**. Line 497 explicitly recomputes `typed_edges = crate::edges::resolve_all_typed_edges(&current_snapshots)` where `current_snapshots` is `untouched_snapshots` (from `prior_graph`) + freshly rebuilt snapshots — i.e., the FULL current pool, same shape as a full reindex would produce. `build_graph` at line 573 uses this recomputed `typed_edges`, not the narrower one.
This means `resolve_calls`/`resolve_extends_implements`/`resolve_tests_for` always see the complete current snapshot pool in both full and incremental runs — the new nested-scope-boundary checks behave identically regardless of indexing mode. This is a deliberate, well-documented design choice (see the comment at `indexer.rs:427-443` and `477-493`), not an accident.

**8. Prior evaluation errata integrity — PASS.**
- B2-EVAL-001's erratum is a clean append (verified via `git diff`), clearly marked `> **ERRATUM pointer...**`, does not delete or rewrite the original `11,934`-edge claim, and explicitly states the CALL-RES-A0 baseline was NOT independently re-measured for this specific defect's magnitude — an honest limitation, not an overclaim.
- B2-EVAL-002's erratum is a clean, large append (`> ## ERRATUM...`), also non-destructive. I verified its arithmetic: 878 total, removed-edge breakdown (256 container + 81 Field + 6 nested-Function = 343) exactly equals 878−535=343; the `StaticAnalysis`/`NameHeuristic` deltas (1,168→942 = −226, 340→223 = −117) sum to 343, matching the total edge delta exactly (consistent, since the footnoted totals include unaffected `Contains`/`Instantiates` edges too).
- Cross-checked the erratum's numbers against the actual evidence file `docs/research/evidence/b2-id-006-pytorch-before-after-edge-diff.txt`: the real diff output shows a "before" breakdown of `Class: 252, Field: 81, Function: 541, Struct: 4` — Class+Struct = 256, matching the claimed "256 container-originated" figure exactly, and this is genuine tool output (real file paths, real diff syntax), not fabricated prose.
- The erratum explicitly distinguishes Measured ("full-corpus contamination count run... AFTER this evaluation was published") from "not separately measured at the time" for the per-category breakdown that existed before the more precise re-run — it does not present an Estimated/Inferred number as Measured.

**9. Full test suite / clippy / cargo-deny, run myself — PASS, all verified directly.**
- `cargo test --release --workspace`: ran it myself (not trusting commit-message claims). Result: 100% green across every crate. `codegraph-domain`: 70 passed, 0 failed. `codegraph-index`: 378 passed, 0 failed (matches B2-ID-007.md's own claimed "378 passing" exactly, superseding B2-ID-006.md's earlier "366" figure from before B2-ID-007's own tests were added — these two documents are consistent with each other once sequencing is accounted for, not contradictory).
- `cargo clippy --release --workspace --all-targets`: ran it myself. Exit 0. All existing warnings (in `graph/store.rs`, `imports.rs`, and three unrelated `crate_builds`/`assert!(true)` tests) are pre-existing and in files NOT touched by this diff — grepped the clippy output for `calls.rs|extends_implements.rs|contains.rs|instantiates.rs|tests_for.rs|kind.rs` and got zero matches. Zero new warnings on touched files, confirmed directly.
- `cargo deny check`: ran it myself. Result: `advisories ok, bans ok, licenses ok, sources ok`. Only pre-existing duplicate-dependency warnings (`syn`, `hashbrown` — unrelated transitive deps). No `Cargo.toml`/`Cargo.lock` changes exist in this diff at all (confirmed via `git diff --stat`), so no new dependencies were introduced.
- Mutation testing evidence files (`b2-id-006-mutation-run-2-final.txt`, `b2-id-007-mutation-extends-implements.txt`, `b2-id-007-mutation-tests-for.txt`) contain genuine `cargo-mutants` tool output, including an untranslated Italian OS file-lock error message in the TestsFor log — a realistic artifact of a real concurrent run that would be very unusual to fabricate. This corroborates the claimed mutation-closure numbers (B2-ID-006: 107/114 caught, 1 pre-existing equivalent mutant; B2-ID-007 Extends/Implements: 35/39 caught, 0 missed; TestsFor: 19/20 caught, 0 missed).

**10. Finding documents match the code — PASS, one minor sequencing note, not a discrepancy.**
Read both `docs/research/findings/B2-ID-006.md` and `B2-ID-007.md` in full and compared their narrative against the actual code:
- B2-ID-006.md's "Status update" and "Validation" sections accurately describe `can_originate_calls()`, the nested-scope boundary mechanism, and the test names/counts I independently verified in `calls.rs`.
- B2-ID-006.md states "366/366 `codegraph-index` tests green" while my own run shows 378. This is NOT a real discrepancy: B2-ID-007.md explicitly documents 12 additional tests (10 for extends_implements + 2 for tests_for) added after B2-ID-006.md's own count was written, and B2-ID-007.md's own stated count ("Full `codegraph-index` suite: 378 passing") matches my measurement exactly. The two documents are sequential snapshots, not conflicting claims — 366+12=378 checks out.
- B2-ID-007.md's "Residual limitations" section honestly states the AUDIT (pre-fix) only empirically reproduced the defect in Python/C# for `Extends`/`Implements`, with Rust/C++/JS/TS "Inferred-affected... not yet empirically reproduced per-language" — this refers specifically to the pre-fix audit stage, and is a different claim from the FIX's own regression-test coverage, which I confirmed does cover all 6 languages (`extends_implements.rs:683-866`). Not a contradiction once the distinction between "audit scope" and "fix validation scope" is tracked, which the document itself tracks carefully.
- `resolve_instantiates`'s and `resolve_contains`'s "immune, confirmed by direct reproduction" claims (both finding docs and both module doc comments) are backed by real synthetic reproduction tests I read directly (`contains.rs:221-241`, `instantiates.rs:200-210`) — the prose matches the code exactly.

---

### What's Done Well

- The decision to implement TWO DIFFERENT, relation-specific mechanisms for B2-ID-007 (parse-tree-local node-kind boundary for inheritance vs. cross-snapshot span check for TestsFor) rather than forcing a one-size-fits-all reuse of B2-ID-006's mechanism is exactly right, and the finding doc explicitly documents why a naive "the same fix surely applies everywhere" assumption would have been wrong (Section "Alternatives considered → A").
- The `for_each_node_not_inside_a_nested_boundary` implementation history (`extends_implements.rs:206-229`) candidly documents a real bug in an earlier attempt (re-deriving "inside a boundary" state from each node's immediate parent, which let a nested class through via an intermediate non-boundary wrapper node like Python's `block`) and how it was caught by test failure before landing. This kind of documented false start is exactly the transparency a research record should have.
- The `strictly_contains` tie-breaking test in `contains.rs` (`resolve_contains_picks_the_first_innermost_class_found_when_a_later_candidate_ties_in_span_length`) and the byte-adjacency boundary test in `calls.rs` are genuine mutation-killing tests built from real, deliberately adversarial byte positions rather than convenient fixtures — this is a notably higher testing standard than typical.
- Every resolver's "no edge beats a false edge" discipline is applied consistently across all five edge kinds, and the errata documents never retroactively inflate confidence — they are candid about exactly what was/was not independently re-measured.

### Verification
- Tests reviewed: yes. All new regression tests for both findings use real tree-sitter extraction (`extract_symbols_reporting_rejections` + `promote`), not hand-written placeholder signatures, specifically because placeholder bodies structurally cannot exercise this defect family — confirmed this design choice is followed consistently across `calls.rs`, `extends_implements.rs`, and `tests_for.rs`.
- Build verified: yes, ran `cargo test --release --workspace` myself — 100% green (70 codegraph-domain, 378 codegraph-index, all other crates green).
- Security checked: no security-relevant surface in this change (static-analysis graph resolver logic, no user input/auth/secrets/network boundary touched); not a focus area for this diff and nothing concerning found incidentally.

No Critical or Important issues found. No changes required before merge.
