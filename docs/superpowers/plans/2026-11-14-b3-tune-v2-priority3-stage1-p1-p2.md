# B3-TUNE-v2 Priority 3 Stage 1 (P1/P2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure two isolated, deterministic reranking ablations — P1
(exact structural-identifier boost) and P2 (status tie-break) — each as
`B1 + <lever>` vs. official B1, on dev (diagnostic) then calibration
(decision), through the real `Index::search` pipeline, never composed
with each other.

**Architecture:** Both levers are additive, opt-in parameters on
`index::search::Index`, mirroring the existing `with_bm25_params`
pattern (Priority 2 Lever A) exactly: a new builder method per lever,
defaulted off, every existing call site (including each other's
ablation) unaffected. P1 adds a fixed bonus to a candidate's BM25
score inside `search_impl`, before the final sort, when its frozen
whole-token identifier-match rule fires. P2 inserts a status-tier
comparison into the existing sort comparator, applied only when two
candidates' BM25 scores are exactly equal as `f64`. Two new example
binaries (`b3_tune_v2_priority3_p1_identifier_boost.rs`,
`b3_tune_v2_priority3_p2_status_tiebreak.rs`) reuse the established
ablation-harness pattern from
`crates/api/examples/b3_tune_v2_lever_c_alias_table.rs` (TestFixture,
`AblationReport`, `MetricBreakdown`, bootstrap CI, abstention
confusion matrix) and add the by-category/by-difficulty breakdown this
round's design requires.

**Tech Stack:** Rust workspace (`crates/index`, `crates/api`), existing
`api::metrics` bootstrap/CI helpers, existing `eval/rag/dev` and
`eval/rag/calibration` label fixtures.

**Spec:**
`docs/superpowers/specs/2026-11-14-b3-tune-v2-priority3-stage1-design.md`
(frozen P1/P2 semantics, KEEP/REJECT criteria, reporting requirements)
and `docs/research/B3-TUNE-v2-PRIORITY3-RANKING-AUDIT.md` (ground-truth
pipeline audit this design builds on).

## Global Constraints

- Candidate set must stay unchanged for both levers — verified by a
  test per lever asserting the candidate id set is identical with and
  without the lever enabled, for at least one query.
- P1's boost fires **only** on a whole-token (`split_whitespace`),
  case-insensitive, exact-equality match between a query token and a
  candidate's `source:`-extracted `#<IDENTIFIER>` fragment — never
  `contains`/`starts_with`/substring (frozen design, "Matching rule").
- P1 never applies to a `Sensitive` candidate (its `source` is
  withheld by the existing pipeline before P1's logic ever sees it).
- P2's tie-break fires **only** when two candidates' BM25 scores are
  bit-for-bit equal `f64` values — never within an epsilon (frozen
  design, "Equivalence/tie rule").
- P2's status tiers (frozen, from the real `domain::NoteStatus`, 6
  variants, no derived `Ord`): Tier 1 `{Verified, Reviewed}` > Tier 2
  `{TestPassed}` > Tier 3 `{Draft}` > Tier 4 `{Superseded,
  Contradicted}`. Within a tier, no additional ordering — falls
  through to the existing note-id lexicographic tie-break.
- Every existing call site of `Index::new`/`Index::search` (including
  B1's own production path and Lever A's ablation) must be provably
  unaffected — both new builder methods default to off/no-op.
  (`with_bm25_params`, added in Priority 2 Lever A, is the precedent:
  Task 1's test asserts `new()` alone reproduces B1's exact
  Recall@10/nDCG@10 on dev, same as Lever A/B/C's own fixtures already
  prove.)
- Dev is diagnostic only; calibration is the decision split (same as
  Stage 1).
- Both ablations use the SAME frozen KEEP/REJECT rule as Priority 2
  Stage 1: ≤0.05 absolute Recall@10/nDCG@10 regression vs. B1, at
  least one metric must improve, abstention confusion matrix pattern
  (TP=0, FP=0) unchanged. A zero-activation result is reported NOT
  EVALUATED / NO-OP, never REJECT.
- `eval/rag/v2-heldout/` is never read by any task in this plan.
- No composition of P1+P2. No P3/P4. No MCP, merge, tag, or push.

## Review Focus

- **Zero-activation runs** (P1 fires on no query, or P2 finds zero
  exact ties): both ablation binaries must report
  `activations_fired`/`ties_broken` explicitly, distinguishing
  NOT-EVALUATED from a genuine tested-and-failed REJECT, per Lever C's
  own corrected lesson. Task 2 and Task 4 each pin this.
- **A `Sensitive` candidate with a `#<IDENTIFIER>`-shaped `source`
  value that P1 must NOT boost** (its `source` is withheld before P1's
  logic runs, so this should already be structurally impossible — Task
  1 adds a test proving it, not just assuming the existing redaction
  covers it).
- **Near-miss identifier strings that must NOT trigger P1's boost**
  (punctuation-fused tokens like `sec-001?`, and partial strings like
  `sec` that must not match `sec-001`) — mirrors Lever C's own q069
  near-miss; Task 1 pins both as explicit non-match tests.
- **Floating-point near-ties that must NOT trigger P2** (two scores
  differing by a tiny nonzero amount, e.g. `1e-12`, must sort exactly
  as B1 does today, with no status influence) — Task 3 pins this
  alongside the genuine-exact-tie positive case.
- **`NoteStatus` variants absent from the frozen tiers** — there are
  none (all 6 variants are assigned a tier in the Global Constraints
  list above), but Task 3's test suite enumerates all 6 variants
  explicitly against the comparator, so a future variant addition to
  `NoteStatus` would make this test block incomplete rather than
  silently wrong.

---

## Task 1: P1 — exact structural-identifier boost in `Index::search`

**Files:**
- Modify: `crates/index/src/search.rs` (add `identifier_boost: bool`
  field + builder, apply inside `search_impl`'s scoring step)
- Test: `crates/index/tests/b1_search.rs`

**Interfaces:**
- Produces: `Index::with_identifier_boost(mut self) -> Self` (builder,
  parallel to `with_bm25_params`; no params needed since the rule and
  bonus constant are both frozen, not tunable).
- Produces (new, `pub(crate)` or private helper in `search.rs`):
  `fn extract_structural_identifier(source: &str) -> Option<String>` —
  same extraction as `extract_structural_candidates` in
  `b3_tune_v2_lever_c_alias_table.rs:70-90-ish` (read that file's exact
  implementation before writing this one — reuse its logic, not its
  code, since `search.rs` cannot depend on an example binary): reads
  the trailing `#<IDENTIFIER>` fragment from `source` up to the first
  non-identifier character (whitespace, `(`), returns `None` if no `#`
  fragment exists.
- Produces: a frozen constant `const IDENTIFIER_BOOST: f64 = <value>;`
  in `search.rs`, pinned in Step 3 below (not left to Priority 3
  Stage 1's ablation binary to decide).
- Consumes: `Provenance.source: Option<String>` (already on
  `SearchResult`, already `None` for `Sensitive` — see `search.rs:345-
  349`), `document.r#type`/`metadata.id` are NOT needed — only
  `source`.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/index/tests/b1_search.rs

#[test]
fn identifier_boost_fires_on_exact_whole_token_match_against_source_fragment() {
    let f = Fixture::new();
    // note-a's source contains "...#SEC-001" per the frozen extraction rule.
    // `Note` is a plain struct with `..Note::new(id, body)` defaults
    // (b1_search.rs:111-141) — override `source` via struct-update syntax,
    // the same idiom the file's other tests already use for non-default fields.
    f.write(&Note { source: "docs/technical-debt.md#SEC-001", ..Note::new("note-a", "a note with no query terms in its body") }, 0);
    f.write(&Note { source: "docs/technical-debt.md#SEC-002", ..Note::new("note-b", "a note with no query terms in its body either") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let idx = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_identifier_boost();

    let response = idx.search("tell me about sec-001", Default::default(), 10).unwrap();

    let index::search::SearchOutcome::Matches { first, .. } = response.outcome else {
        panic!("expected a match");
    };
    assert_eq!(first.note_id.as_str(), "note-a", "the exact identifier match must rank first");
}

#[test]
fn identifier_boost_never_fires_on_a_substring_or_prefix_match() {
    let f = Fixture::new();
    f.write(&Note { source: "docs/technical-debt.md#SEC-001", ..Note::new("note-a", "distinctive_filler_term_alpha") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let idx = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_identifier_boost();

    // "sec" is a substring/prefix of "SEC-001" but must never match it.
    let response = idx.search("what does sec mean", Default::default(), 10);

    // "sec" has no BM25/trigram overlap with note-a's body or structured
    // fields either, so with the boost correctly NOT firing this must be
    // NotPresent, not a boosted hit on note-a.
    assert!(matches!(response.unwrap().outcome, index::search::SearchOutcome::NotPresent));
}

#[test]
fn identifier_boost_never_applies_to_a_sensitive_candidates_withheld_source() {
    let f = Fixture::new();
    // Sensitive: source is withheld by the existing pipeline (search.rs:345-349)
    // before identifier_boost's logic can see it at all.
    f.write(&Note { source: "docs/technical-debt.md#SEC-001", classification: "sensitive", ..Note::new("note-a", "sensitive project-x bug draft internal") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let idx = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_identifier_boost();

    let response = idx.search("sec-001", Default::default(), 10).unwrap();

    // No term/trigram overlap and no boost possible (source withheld) ->
    // the Sensitive note must not be boosted into a match it otherwise
    // would not earn on text alone.
    assert!(matches!(response.outcome, index::search::SearchOutcome::NotPresent));
}

#[test]
fn identifier_boost_does_not_change_the_candidate_set() {
    let f = Fixture::new();
    f.write(&Note { source: "docs/technical-debt.md#SEC-001", ..Note::new("note-a", "a note about sec-001 topic") }, 0);
    f.write(&Note::new("note-b", "a note about sec-001 topic too"), 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let plain = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26"));
    let boosted = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_identifier_boost();

    let ids = |outcome: index::search::SearchOutcome| -> std::collections::HashSet<String> {
        match outcome {
            index::search::SearchOutcome::NotPresent => Default::default(),
            index::search::SearchOutcome::Matches { first, rest } => {
                let mut s: std::collections::HashSet<String> = rest.iter().map(|r| r.note_id.as_str().to_string()).collect();
                s.insert(first.note_id.as_str().to_string());
                s
            }
        }
    };
    let plain_ids = ids(plain.search("sec-001 topic", Default::default(), 10).unwrap().outcome);
    let boosted_ids = ids(boosted.search("sec-001 topic", Default::default(), 10).unwrap().outcome);
    assert_eq!(plain_ids, boosted_ids, "identifier_boost must only reorder, never change membership");
}
```

(No new `Fixture` helper methods are needed: `Note` is already a plain
struct with `fn new(id, body) -> Self` defaults — `project: "project-x"`,
`classification: "internal"`, `source: "agent:fixture"`, etc.
(`b1_search.rs:111-141`) — and `Fixture::write(&Note, expected_version)`
already exists. Override only the fields each test needs via struct-
update syntax, `Note { source: "...", ..Note::new(id, body) }`, the
same idiom this file's other tests already use.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p index --test b1_search identifier_boost`
Expected: FAIL — `with_identifier_boost` not found (compile error),
confirming the feature doesn't exist yet.

- [ ] **Step 3: Implement in `crates/index/src/search.rs`**

Add `identifier_boost: bool` field to `Index<'a>` (default `false` in
`new()`), and:

```rust
pub fn with_identifier_boost(mut self) -> Self {
    self.identifier_boost = true;
    self
}
```

Pin `IDENTIFIER_BOOST` by reading the actual B1 calibration score
distribution from `eval/results/B3/calibration-20261114T123333Z-e613edd.json`
(or by instrumenting a quick run) and choosing a constant strictly
larger than the maximum observed BM25 score in that run — document the
exact value and the max observed score it was compared against in a
code comment. Implement `extract_structural_identifier` as a private
function (port the logic read from
`b3_tune_v2_lever_c_alias_table.rs`'s `extract_structural_candidates`,
adapted to operate on a `source: &str` directly rather than raw note
text, since `search.rs` already has `Provenance.source` separately).

Inside `search_impl`, after `fresh_scores.insert(...)` (around line
362) but before the final score-lookup loop (around line 391-397): for
each `fresh_results` entry, if `self.identifier_boost` is true and
`result.provenance.source` is `Some(source)`, extract its identifier;
if `Some(identifier)`, check whether any `query.split_whitespace()`
token, lowercased, equals `identifier.to_lowercase()` exactly; if so,
add `IDENTIFIER_BOOST` to that result's score after the existing
`scores.get(...)` lookup at line 396 (i.e. `result.score =
scores.get(...).copied().unwrap_or(0.0) +
if boosted { IDENTIFIER_BOOST } else { 0.0 }`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p index --test b1_search identifier_boost`
Expected: PASS, all 4 new tests.

- [ ] **Step 5: Run the full `index` crate test suite**

Run: `cargo test -p index`
Expected: PASS, no regressions (same count as before this task plus
the 4 new tests).

- [ ] **Step 6: Commit**

```bash
git add crates/index/src/search.rs crates/index/tests/b1_search.rs
git commit -m "feat(index): additive P1 exact-identifier boost for Priority 3 ablation"
```

---

## Task 2: P1 — ablation binary and run

**Files:**
- Create: `crates/api/examples/b3_tune_v2_priority3_p1_identifier_boost.rs`

**Interfaces:**
- Consumes: `Index::with_identifier_boost()` (Task 1).
- Consumes (pattern reuse, read exactly from
  `crates/api/examples/b3_tune_v2_lever_c_alias_table.rs`):
  `TestFixture` (lines 138-231, including `Drop` and `build_index`),
  `LabelRow`/`parse_label_rows`/`exclude_canary` (lines 26-55),
  `QueryOutcome`/`run_one_query` (lines 245-271), `ScoredRow`/
  `score_answerable_row` (lines 333-348), `MetricBreakdown`/
  `metrics_breakdown` (lines 350-374), `ConfusionMatrixReport`/
  `AbstentionReport`/`optional_metric_report` (lines 376-412),
  `git_rev_parse_head`/`sha256_hex_of_file` (lines 432-447).
- Produces: `AblationReportWithBreakdown` (new struct, serde-serializable
  like Lever C's `AblationReport` but not shared with it — each lever
  binary keeps its own independent copy, matching how Lever A/B/C
  already each define their own report struct rather than sharing one):
  fields `split: String`, `run_commit: String`, `labels_sha256: String`,
  `identifier_boost_fired: usize`, `query_count: usize`,
  `aggregate: MetricBreakdown`, `by_category: HashMap<String,
  MetricBreakdown>` (keyed by `LabelRow.category`), `by_difficulty:
  HashMap<String, MetricBreakdown>` (keyed by `LabelRow.difficulty`),
  `abstention: AbstentionReport`. (Do not modify Lever C's own
  `AblationReport` in `b3_tune_v2_lever_c_alias_table.rs` — this is a
  wholly separate, new struct in the new file.)

- [ ] **Step 1: Write the failing test**

```rust
// in this new file's own #[cfg(test)] mod tests, mirroring
// b3_tune_v2_lever_c_alias_table.rs's own test module pattern

#[test]
fn identifier_boost_fired_is_true_only_when_a_query_token_matches_a_candidates_identifier() {
    assert!(identifier_boost_would_fire("tell me about sec-001", "docs/x.md#SEC-001"));
    assert!(!identifier_boost_would_fire("tell me about section one", "docs/x.md#SEC-001"));
    assert!(!identifier_boost_would_fire("sec", "docs/x.md#SEC-001"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p api --example b3_tune_v2_priority3_p1_identifier_boost identifier_boost_fired`
Expected: FAIL — `identifier_boost_would_fire` not defined.

- [ ] **Step 3: Implement the binary**

Write `fn identifier_boost_would_fire(query: &str, source: &str) -> bool`
as a local re-implementation of Task 1's matching predicate (same
whole-token exact-match rule; duplicated across the crate boundary the
same way `tokenize_like_bm25` was duplicated for Lever B — add the
same kind of pinned parity test/doc-comment cross-reference Lever
B/bm25.rs used, pointing at `crates/index/src/search.rs`'s
`extract_structural_identifier`/matching logic).

Implement `run_ablation_for_split(split: &str) -> AblationReportWithBreakdown`
following `run_ablation_for_split`'s exact structure from Lever C
(lines 449-513), with these differences:
- Build the index via `fixture.build_index().with_identifier_boost()`
  instead of plain `fixture.build_index()`.
- Track `identifier_boost_fired: usize` by calling
  `identifier_boost_would_fire` per candidate per row (count a row once
  if it fires for at least one candidate in that row's search) instead
  of `expansions_fired`.
- Group `recall_values`/`ndcg_values` additionally by `row.category`
  and `row.difficulty` (remove the `#[allow(dead_code)]` on those two
  `LabelRow` fields — they are now read) and compute a
  `MetricBreakdown` per distinct category and per distinct difficulty
  value observed in the split, via `metrics_breakdown`.

Add a `--ablate <split>` CLI mode identical in structure to Lever C's
`main()` (lines 515-545), writing to
`eval/results/B3-TUNE-v2/priority3-p1-{split}-ablation.json`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p api --example b3_tune_v2_priority3_p1_identifier_boost identifier_boost_fired`
Expected: PASS.

- [ ] **Step 5: Run dev, then calibration**

Run: `cargo run --release -p api --example b3_tune_v2_priority3_p1_identifier_boost -- --ablate dev`
Run: `cargo run --release -p api --example b3_tune_v2_priority3_p1_identifier_boost -- --ablate calibration`
Expected: both complete, writing their JSON reports; note the printed
`identifier_boost_fired` count for each split — if either is 0, this
lever's result for that split must be reported NOT EVALUATED / NO-OP
per the frozen design, not REJECT.

- [ ] **Step 6: Commit**

```bash
git add crates/api/examples/b3_tune_v2_priority3_p1_identifier_boost.rs eval/results/B3-TUNE-v2/priority3-p1-dev-ablation.json eval/results/B3-TUNE-v2/priority3-p1-calibration-ablation.json
git commit -m "feat(eval): B3-TUNE-v2 Priority 3 P1 identifier-boost ablation (dev+calibration)"
```

---

## Task 3: P2 — status tie-break in `Index::search`

**Files:**
- Modify: `crates/index/src/search.rs`
- Test: `crates/index/tests/b1_search.rs`

**Interfaces:**
- Produces: `Index::with_status_tiebreak(mut self) -> Self` (builder,
  same additive pattern as Task 1's `with_identifier_boost`).
- Produces: a private `fn status_tier(status: domain::NoteStatus) -> u8`
  mapping all 6 `NoteStatus` variants to `{1, 1, 2, 3, 4, 4}` per the
  frozen tiers (`Verified`=1, `Reviewed`=1, `TestPassed`=2, `Draft`=3,
  `Superseded`=4, `Contradicted`=4) — lower number sorts first.
- Consumes: `SearchResult`'s existing `score: f64`, and
  `document.status: NoteStatus` (already read inside `search_impl`'s
  per-candidate loop, just not currently threaded onto `SearchResult`
  for sort purposes — Task 3 must add a `status: NoteStatus` field to
  the struct used internally for sorting; it does not need to be added
  to the public `SearchResult` struct itself unless convenient, since
  `document.status` is still in scope at the point `fresh_results` is
  built).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn status_tiebreak_fires_only_on_exactly_equal_bm25_scores() {
    let f = Fixture::new();
    // Both notes share identical text so their BM25 scores are
    // bitwise-identical f64 values under the real pipeline.
    // `Note.status` is a kebab-case `&str` field written straight into
    // frontmatter (`Note::new`'s default is `"draft"`) — override it via
    // struct-update syntax, same idiom as the `source` override above.
    f.write(&Note { status: "draft", ..Note::new("note-draft", "shared_term appears here") }, 0);
    f.write(&Note { status: "verified", ..Note::new("note-verified", "shared_term appears here") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let idx = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();

    let response = idx.search("shared_term", Default::default(), 10).unwrap();

    let index::search::SearchOutcome::Matches { first, .. } = response.outcome else {
        panic!("expected a match");
    };
    assert_eq!(first.note_id.as_str(), "note-verified", "Verified must outrank Draft at an exact score tie");
}

#[test]
fn status_tiebreak_never_fires_on_a_nonzero_score_difference() {
    let f = Fixture::new();
    // note-a has extra matching term frequency -> a strictly higher,
    // non-tied BM25 score than note-b, despite note-b's higher status tier.
    f.write(&Note { status: "draft", ..Note::new("note-a", "shared_term shared_term shared_term extra filler text to differ length") }, 0);
    f.write(&Note { status: "verified", ..Note::new("note-b", "shared_term") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let plain = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26"));
    let tiebreak = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();

    let plain_outcome = plain.search("shared_term", Default::default(), 10).unwrap().outcome;
    let tiebreak_outcome = tiebreak.search("shared_term", Default::default(), 10).unwrap().outcome;

    // Ordering must be identical whether or not the tiebreak flag is set,
    // because the scores are NOT exactly equal.
    assert_eq!(format!("{plain_outcome:?}"), format!("{tiebreak_outcome:?}"));
}

#[test]
fn all_six_note_status_variants_map_to_the_frozen_tier_via_observed_ordering() {
    // `status_tier` itself is private to `search.rs` and `#[cfg(test)]`
    // does NOT cross the crate boundary into this integration-test binary
    // (confirmed: Priority 2 Lever B had to duplicate logic locally for
    // exactly this reason, and `indexer_impl.rs`'s own `#[cfg(test)]`
    // helpers like `trigram_hits` are never called from this file today).
    // So this test observes the tier ordering indirectly, through
    // `with_status_tiebreak`'s real effect on search output, for every
    // pair of tiers, rather than calling a private function directly.
    let pairs = [
        ("verified", "test-passed"),
        ("reviewed", "test-passed"),
        ("test-passed", "draft"),
        ("draft", "superseded"),
        ("draft", "contradicted"),
    ];
    for (higher_status, lower_status) in pairs {
        let f = Fixture::new();
        f.write(&Note { id: "note-higher", status: higher_status, ..Note::new("note-higher", "shared_tier_probe_term") }, 0);
        f.write(&Note { id: "note-lower", status: lower_status, ..Note::new("note-lower", "shared_tier_probe_term") }, 0);
        let indexer = RealIndexer::new();
        index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
        let idx = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();
        let response = idx.search("shared_tier_probe_term", Default::default(), 10).unwrap();
        let index::search::SearchOutcome::Matches { first, .. } = response.outcome else {
            panic!("expected a match for {higher_status} vs {lower_status}");
        };
        assert_eq!(first.note_id.as_str(), "note-higher", "{higher_status} must outrank {lower_status} at an exact score tie");
    }
    // Same-tier pair: Verified and Reviewed must NOT have a defined
    // relative order from this rule (falls through to note-id tie-break).
    let f_same_tier = Fixture::new();
    f_same_tier.write(&Note { id: "note-verified", status: "verified", ..Note::new("note-verified", "shared_tier_probe_term") }, 0);
    f_same_tier.write(&Note { id: "note-reviewed", status: "reviewed", ..Note::new("note-reviewed", "shared_tier_probe_term") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f_same_tier.vault, &indexer).unwrap();
    let idx = index::Index::new(&f_same_tier.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();
    let response = idx.search("shared_tier_probe_term", Default::default(), 10).unwrap();
    let index::search::SearchOutcome::Matches { first, .. } = response.outcome else {
        panic!("expected a match");
    };
    // note-reviewed < note-verified lexicographically -> the existing
    // note-id tie-break decides, unchanged by the same-tier status pair.
    assert_eq!(first.note_id.as_str(), "note-reviewed");
}

#[test]
fn status_tiebreak_does_not_change_the_candidate_set() {
    let f = Fixture::new();
    f.write(&Note { status: "draft", ..Note::new("note-a", "shared_term") }, 0);
    f.write(&Note { status: "verified", ..Note::new("note-b", "shared_term") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let plain = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26"));
    let tiebreak = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();

    let ids = |outcome: index::search::SearchOutcome| -> std::collections::HashSet<String> {
        match outcome {
            index::search::SearchOutcome::NotPresent => Default::default(),
            index::search::SearchOutcome::Matches { first, rest } => {
                let mut s: std::collections::HashSet<String> = rest.iter().map(|r| r.note_id.as_str().to_string()).collect();
                s.insert(first.note_id.as_str().to_string());
                s
            }
        }
    };
    assert_eq!(
        ids(plain.search("shared_term", Default::default(), 10).unwrap().outcome),
        ids(tiebreak.search("shared_term", Default::default(), 10).unwrap().outcome),
    );
}
```

(No new `Fixture` helper is needed — `Note.status` is overridden via
struct-update syntax exactly as `source` was in Task 1. No test-only
export of `status_tier` is needed either: `#[cfg(test)]` in a library
crate does not make an item visible to a separate integration-test
binary like `tests/b1_search.rs` — confirmed by `indexer_impl.rs`'s
own existing `#[cfg(test)]` helpers (e.g. `trigram_hits`,
`poison_next_mark_current`), which are never called from
`b1_search.rs` today, only from `indexer_impl.rs`'s own in-crate test
module. All tests above therefore observe `status_tier`'s effect only
indirectly, through `with_status_tiebreak`'s real impact on search
ordering — never by calling the private tier function directly.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p index --test b1_search status_tiebreak`
Expected: FAIL — `with_status_tiebreak` not found.

- [ ] **Step 3: Implement in `crates/index/src/search.rs`**

Add `status_tiebreak: bool` field to `Index<'a>` (default `false`).
Add `with_status_tiebreak(mut self) -> Self` builder. Add private
`fn status_tier(status: domain::NoteStatus) -> u8` per the frozen
mapping. Thread `document.status` onto each `fresh_results` entry
(either as a new private tuple alongside `SearchResult`, or as a
temporary parallel `Vec<NoteStatus>` built in the same loop that builds
`fresh_results` — implementer's choice, whichever is simpler given the
existing loop shape). Modify the sort comparator at lines 398-403:

```rust
fresh_results.sort_by(|a, b| {
    b.score
        .total_cmp(&a.score)
        .then_with(|| {
            if self.status_tiebreak && a.score == b.score {
                status_tier(a_status).cmp(&status_tier(b_status))
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .then_with(|| a.note_id.as_str().cmp(b.note_id.as_str()))
});
```

(exact variable names for `a_status`/`b_status` depend on how status
was threaded above — implementer's choice of the cleanest shape that
satisfies this ordering).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p index --test b1_search -- status_tiebreak tier`
Expected: PASS, all 4 new tests from Step 1 (two match `status_tiebreak`
by name, the tier-ordering test matches `tier`, and
`status_tiebreak_does_not_change_the_candidate_set` matches
`status_tiebreak` — `cargo test` with multiple filter args runs the
union of tests matching any filter).

- [ ] **Step 5: Run the full `index` crate test suite**

Run: `cargo test -p index`
Expected: PASS, no regressions.

- [ ] **Step 6: Commit**

```bash
git add crates/index/src/search.rs crates/index/tests/b1_search.rs
git commit -m "feat(index): additive P2 status tie-break for Priority 3 ablation"
```

---

## Task 4: P2 — ablation binary and run

**Files:**
- Create: `crates/api/examples/b3_tune_v2_priority3_p2_status_tiebreak.rs`

**Interfaces:**
- Consumes: `Index::with_status_tiebreak()` (Task 3).
- Consumes: the same reusable pattern pieces listed in Task 2's
  Interfaces block (`TestFixture`, `LabelRow`, `QueryOutcome`,
  `run_one_query`, `MetricBreakdown`/`metrics_breakdown`,
  `AbstentionReport`, `git_rev_parse_head`/`sha256_hex_of_file`).
- Produces: `AblationReportWithBreakdown` — same shape as Task 2's
  (duplicate the struct locally in this binary rather than sharing
  across example binaries, matching how Lever A/B/C each keep their
  own independent copies of this report shape today).
- Produces: `exact_ties_observed: usize` and `ties_broken_by_status:
  usize` fields on this binary's ablation report (the design doc's
  required instrumentation for P2) — computed by running the SAME
  query through both a plain `Index` and a `with_status_tiebreak()`
  `Index` per row, comparing their top-10 orderings, and counting rows
  where the two orderings differ (a tie was broken) vs. counting exact
  `f64` score ties directly if that's simpler to instrument from
  inside the ablation loop (implementer's choice of the more direct
  measurement, as long as both counts are reported).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn ablation_report_serializes_the_required_instrumentation_fields() {
    let report = AblationReportWithBreakdown {
        split: "dev".to_string(),
        run_commit: "deadbeef".to_string(),
        labels_sha256: "abc123".to_string(),
        exact_ties_observed: 0,
        ties_broken_by_status: 0,
        query_count: 0,
        aggregate: MetricBreakdown { recall_at_10: 0.0, ndcg_at_10: 0.0, recall_ci: [0.0, 0.0], ndcg_ci: [0.0, 0.0], n: 0, low_confidence: true },
        by_category: Default::default(),
        by_difficulty: Default::default(),
        abstention: AbstentionReport { /* ... */ },
    };
    let json = serde_json::to_value(&report).unwrap();
    assert!(json.get("exact_ties_observed").is_some());
    assert!(json.get("ties_broken_by_status").is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p api --example b3_tune_v2_priority3_p2_status_tiebreak ablation_report_serializes`
Expected: FAIL — `AblationReportWithBreakdown` not defined.

- [ ] **Step 3: Implement the binary**

Same structure as Task 2's binary, built against `with_status_tiebreak()`
instead of `with_identifier_boost()`. Add the `by_category`/
`by_difficulty` breakdown the same way Task 2 does. Add a `--ablate
<split>` CLI mode writing to
`eval/results/B3-TUNE-v2/priority3-p2-{split}-ablation.json`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p api --example b3_tune_v2_priority3_p2_status_tiebreak ablation_report_serializes`
Expected: PASS.

- [ ] **Step 5: Run dev, then calibration**

Run: `cargo run --release -p api --example b3_tune_v2_priority3_p2_status_tiebreak -- --ablate dev`
Run: `cargo run --release -p api --example b3_tune_v2_priority3_p2_status_tiebreak -- --ablate calibration`
Expected: both complete; note the printed `exact_ties_observed`/
`ties_broken_by_status` counts — if zero ties were observed on a
split, that split's result is NOT EVALUATED / NO-OP for P2, not
REJECT.

- [ ] **Step 6: Commit**

```bash
git add crates/api/examples/b3_tune_v2_priority3_p2_status_tiebreak.rs eval/results/B3-TUNE-v2/priority3-p2-dev-ablation.json eval/results/B3-TUNE-v2/priority3-p2-calibration-ablation.json
git commit -m "feat(eval): B3-TUNE-v2 Priority 3 P2 status-tiebreak ablation (dev+calibration)"
```

---

## Task 5: Verdicts and summary document

**Files:**
- Create: `eval/results/B3-TUNE-v2/priority3-stage1-summary.md`

**Interfaces:**
- Consumes: the four JSON reports from Tasks 2 and 4
  (`priority3-p1-{dev,calibration}-ablation.json`,
  `priority3-p2-{dev,calibration}-ablation.json`).

- [ ] **Step 1: No test — this is a documentation task.** (Right-sized
  into its own task because it depends on both ablations' real output,
  not because it needs its own test cycle.)

- [ ] **Step 2: Write the summary document**

For each of P1 and P2, independently: apply the frozen KEEP/REJECT
rule from the design doc against calibration's numbers (dev reported
as diagnostic only), state the activation/tie counts from Task 2/4,
include the by-category/by-difficulty breakdown table, state exact
`run_commit`/`labels_sha256` from the JSON reports, and give a
standalone verdict: KEEP, REJECT, or NOT EVALUATED / NO-OP. State
explicitly: no composition (`B1+P1+P2`) was run or measured; P3/P4
remain deferred; `eval/rag/v2-heldout/` was not read.

- [ ] **Step 3: Commit**

```bash
git add eval/results/B3-TUNE-v2/priority3-stage1-summary.md
git commit -m "docs(eval): B3-TUNE-v2 Priority 3 Stage 1 P1/P2 isolated results"
```

---

## Task 6: Final whole-branch review

Per `superpowers:executing-plans`: dispatch (or self-review, if no
subagent tool) a fresh-context review of every commit made by this
plan, using the plan's Review Focus section and this design's frozen
rules as the checklist. Fix Critical/Important findings in one pass,
each with its own RED→GREEN test; ledger Minor findings as deferred.
Report results to the user and STOP — no composition, no P3/P4, no
Priority 4/hybrid, no MCP, no merge/tag/push.
