# B3-TUNE-v2 Priority 2 Stage 1 — Lexical Tuning Isolated Ablations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task (Native execution, consistent with B3's and Priority 1's own execution — implement in the current session, one independent review of the whole result at the end). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure three lexical-tuning levers — BM25 `k1`/`b`, a
conservative punctuation-normalization rule, and a small hand-curated
alias table — each as a fully isolated ablation against the **official**
B1 baseline, and report a KEEP/REJECT decision per lever. Stop after
that report. Do not compose accepted levers (Stage 2) without separate
authorization.

**Architecture:** Three independent diagnostic/ablation binaries, one
per lever, each running the real `api::search` path against dev then
calibration. Lever A adds an additive, default-preserving constructor
path through `Bm25Index`/`Index` so custom `k1`/`b` values reach the
real scoring code with zero change to any existing call site. Lever B
adds one small, narrowly-scoped tokenizer function, tested in isolation
before being wired into an ablation binary (never modifying
`bm25::tokenize` itself, which stays the frozen B3/Priority-1-era
production tokenizer — see Task 4's own note on this). Lever C is a
static `HashMap`-backed query-time expansion applied only inside its own
ablation binary, built from a diagnostic pass over the corpus + dev
only, frozen before calibration ever sees it.

**Tech Stack:** Rust (existing `index`/`api` crates), the existing B3
corpus/dev/calibration label files. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-14-b3-tune-v2-priority2-lexical-design.md`
(read in full before starting, including the 3 corrections). Builds on
the accepted B1 baseline recorded in `eval/results/B3/frozen-config.json`
(commit `52d42ea`) and `docs/research/B3-CLOSURE-SUMMARY.md`. Priority 1
is CLOSED as REJECT (`docs/research/B3-TUNE-v2-PRIORITY1-CLOSURE.md`) —
this plan does not build on it, only on the official B1 baseline.

## Global Constraints

- **Stage 1 only.** `B1+A`, `B1+B`, `B1+C` are each measured
  independently against the official B1 baseline. No lever's ablation
  configuration includes another lever's change. Composition (Stage 2)
  is explicitly out of this plan's scope — the design doc requires
  separate user authorization before any composite configuration is
  even designed, let alone measured.
- **Calibration never selects or justifies an alias-table entry**
  (design doc correction #1). Lever C's candidate identification uses
  only the corpus and the dev split. Calibration is read only once the
  alias table is already frozen, purely to evaluate it.
- **Lever B's normalization rule is conservative, not generic.** No
  "strip all leading/trailing non-alphanumeric characters" — only the
  specific trailing sentence-boundary character set the design doc
  names (design doc correction #3), confirmed safe against technical
  tokens by an explicit diagnostic pass before being finalized.
- `index::search`'s production behavior changes **only** via the
  additive constructor paths each task below specifies — no existing
  call site of `Bm25Index::new()`, `Index::new()`, or `bm25::tokenize`
  changes behavior. Every ablation binary opts in explicitly; nothing
  B3's own frozen runner does changes.
- The new v2 held-out set (`eval/rag/v2-heldout/`) stays completely
  unread. Nothing in this plan opens `eval/rag/v2-heldout/labels.jsonl`.
- No new held-out final-test run of any kind.
- `eval/rag/`'s existing `dev`/`calibration`/`test` splits and
  `eval/results/B3/` reports are read-only inputs. Nothing in `eval/rag/`
  or `eval/results/B3/` from the B3 phase is modified.
- Every ablation is logged per the design doc's experimental logging
  rule: full config, commit, corpus/labels hashes, Recall@10 (+CI),
  nDCG@10 (+CI), the abstention confusion matrix (as a correctness
  tripwire, not a tuning target — flag any change in `NotPresent`
  behavior explicitly), delta vs. the **official** B1 baseline, and an
  explicit KEEP/REJECT decision.
- No per-category thresholds, no abstention-threshold tuning of any
  kind — Priority 1 stays closed.
- No MCP, no merge/tag/push, no Priority 3/4 work.

## Review Focus

- Lever A's k1/b override accidentally also changing the candidate
  pre-filter's behavior (`RealIndexer::candidate_ids`'s own, separate
  `Bm25Index`), making the ablation's Recall@10 change attributable to
  candidate-set drift rather than re-ranking — Task 1's own test must
  confirm the override touches only `Index::search`'s final-scoring
  `Bm25Index`, never `RealIndexer`'s internal one.
- Lever B's punctuation rule silently corrupting a real corpus token
  this plan didn't anticipate (beyond the illustrative list in the
  design doc) — Task 4's diagnostic pass over every distinct token in
  corpus+dev is the actual safety net, not the illustrative examples
  alone; a cutoff that still collides on a real token must narrow the
  rule, not proceed.
- Lever C's alias expansion firing on a query substring inside a
  different, unrelated word (e.g. expanding "SEC" inside "SECTION") —
  Task 7's alias-application function must match whole tokens only, not
  substrings, and its tests must cover this directly.
- An ablation's Recall@10/nDCG@10 computation silently drifting from
  the frozen B3 formulas (e.g. by reimplementing recall/nDCG inline
  instead of reusing `api::metrics::recall_at_10`/`ndcg_at_10`) — every
  ablation binary in this plan reuses those exact functions, never a
  reimplementation; Tasks 2/5/8's own steps call this out explicitly.
- A lever accidentally changing abstention behavior (B1 going from
  "never abstains" to sometimes returning `NotPresent`, or vice versa)
  without that being noticed because the ablation report only looks at
  Recall@10/nDCG@10 — every ablation report in this plan includes the
  abstention confusion matrix specifically as a tripwire, per the
  Global Constraints.

---

## File Structure

- Modify: `crates/index/src/bm25.rs` — add `k1`/`b` instance fields,
  `Bm25Index::new_with_params(k1, b)`, replace `#[derive(Default)]`
  with a manual `impl Default` that calls `new()`. `new()`'s own
  observable behavior is unchanged (same constants, same formula).
- Modify: `crates/index/src/search.rs` — add two fields to `Index<'a>`
  carrying the BM25 params (defaulted in `Index::new`), a builder method
  `with_bm25_params(k1, b)`, and switch the one `fresh_scores =
  Bm25Index::new()` call (line 276) to use the stored params. No
  existing call site of `Index::new` changes.
- Create: `crates/api/examples/b3_tune_v2_lever_a_bm25_params.rs` —
  Lever A's ablation binary: runs dev then calibration through
  `Index::new(...).with_bm25_params(k1, b)` for each grid point, reusing
  `api::metrics::recall_at_10`/`ndcg_at_10`/confusion-matrix logic.
- Create: `crates/index/src/lexical_normalize.rs` — Lever B's narrow,
  tested-in-isolation punctuation-stripping function. New module, not a
  change to `bm25::tokenize` itself (see Task 4).
- Create: `crates/api/examples/b3_tune_v2_lever_b_normalization.rs` —
  Lever B's diagnostic (corpus+dev token scan) and ablation binary.
- Create: `crates/api/examples/b3_tune_v2_lever_c_alias_table.rs` —
  Lever C's diagnostic (corpus+dev candidate identification), alias
  application function, and ablation binary, all in one file (small
  enough not to warrant a split; see Task 6-8).
- Create: `eval/results/B3-TUNE-v2/priority2-lever-a-grid.json`,
  `priority2-lever-a-dev-ablation.json`,
  `priority2-lever-a-calibration-ablation.json` — Lever A's grid
  results and dev/calibration ablation reports.
- Create: `eval/results/B3-TUNE-v2/priority2-lever-b-token-diagnostic.json`,
  `priority2-lever-b-dev-ablation.json`,
  `priority2-lever-b-calibration-ablation.json` — Lever B's diagnostic
  and ablation reports.
- Create: `eval/results/B3-TUNE-v2/priority2-lever-c-alias-candidates.json`,
  `priority2-lever-c-alias-table.json`,
  `priority2-lever-c-dev-ablation.json`,
  `priority2-lever-c-calibration-ablation.json` — Lever C's candidate
  diagnostic, frozen table, and dev/calibration ablation reports.
- Create: `eval/results/B3-TUNE-v2/priority2-stage1-summary.md` — the
  final KEEP/REJECT report across all three levers (Task 9).

---

### Task 1: Additive BM25 `k1`/`b` override path (`bm25.rs` + `search.rs`)

**Files:**
- Modify: `crates/index/src/bm25.rs`
- Modify: `crates/index/src/search.rs`

**Interfaces:**
- Consumes: nothing new — this task only adds constructor/builder
  surface to existing types.
- Produces: `Bm25Index::new_with_params(k1: f64, b: f64) -> Self` (new,
  alongside unchanged `new()`), `Index::with_bm25_params(self, k1: f64,
  b: f64) -> Self` (new builder method on `Index<'a>`). Consumed by
  Task 2's ablation binary.

- [ ] **Step 1: Write the failing test proving `new()`'s behavior is unchanged after the refactor**

```rust
#[test]
fn new_and_new_with_params_default_values_score_identically() {
    let mut a = Bm25Index::new();
    let mut b = Bm25Index::new_with_params(1.2, 0.75);
    a.insert(id("note-a"), "rust rust systems");
    b.insert(id("note-a"), "rust rust systems");
    a.insert(id("note-b"), "rust coding is fun with tests");
    b.insert(id("note-b"), "rust coding is fun with tests");
    assert_eq!(a.query_ranked("rust systems", 10), b.query_ranked("rust systems", 10));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p index bm25::new_and_new_with_params`
Expected: FAIL (`new_with_params` not yet defined).

- [ ] **Step 3: Implement the `k1`/`b` fields and `new_with_params`**

Add `k1: f64, b: f64` fields to `Bm25Index` (alongside the existing
`term_freqs`/`doc_lengths`/`doc_freqs`). Change `new()`'s body to call
`Self::new_with_params(1.2, 0.75)` (single source of truth for the
defaults — the module-level `K1`/`B` consts are removed, replaced by
these two instance fields). `new_with_params(k1, b)` constructs with
empty maps plus the given `k1`/`b`. In `query_ranked`'s scoring loop,
change the two bare `K1`/`B` references to `self.k1`/`self.b`. Replace
`#[derive(Debug, Default)]` on `Bm25Index` with `#[derive(Debug)]` plus:

```rust
impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Run the test to verify it passes, then run the full existing `index` suite to verify zero regressions**

Run: `cargo test -p index bm25::new_and_new_with_params` — Expected: PASS.
Run: `cargo test -p index` — Expected: PASS, same count as before this
task plus the one new test (the existing `bm25.rs` tests hardcode scores
computed from k1=1.2/b=0.75; they must still pass unchanged, proving the
refactor preserved `new()`'s exact behavior).

- [ ] **Step 5: Add the builder method to `Index<'a>`**

Add two fields to `Index<'a>` (`crates/index/src/search.rs:175-180`),
e.g. `bm25_k1: f64, bm25_b: f64`. In `Index::new` (line 183-195), set
them to `1.2, 0.75` (the current defaults) — no change to `new`'s
signature or any of its 5 existing call sites. Add:

```rust
pub fn with_bm25_params(mut self, k1: f64, b: f64) -> Self {
    self.bm25_k1 = k1;
    self.bm25_b = b;
    self
}
```

Change line 276's `let mut fresh_scores = Bm25Index::new();` to `let mut
fresh_scores = Bm25Index::new_with_params(self.bm25_k1, self.bm25_b);`.

- [ ] **Step 6: Write the failing test proving the override reaches real search scoring**

```rust
// In crates/index/tests/b1_search.rs, reusing that file's existing Fixture.
#[test]
fn with_bm25_params_changes_real_search_scores() {
    let f = Fixture::new();
    // commit two notes where term-frequency/length differences make b's
    // effect visible: a short note and a long note both matching the query.
    f.commit_note("short-note", "rust");
    f.commit_note("long-note", "rust ".repeat(50).trim());
    rebuild_index(&f.vault, &f.indexer).unwrap();
    let default_idx = Index::new(&f.vault, &f.indexer, "project-x", date("2026-10-26"));
    let b_zero_idx = Index::new(&f.vault, &f.indexer, "project-x", date("2026-10-26"))
        .with_bm25_params(1.2, 0.0); // b=0 disables length normalization entirely
    let default_scores = default_idx.search("rust", SearchFilters::default(), 10).unwrap();
    let b_zero_scores = b_zero_idx.search("rust", SearchFilters::default(), 10).unwrap();
    assert_ne!(default_scores, b_zero_scores, "b=0.0 must change real search scores vs the default b=0.75");
}
```

(Adapt the exact fixture/commit helper names to whatever
`crates/index/tests/b1_search.rs`'s existing `Fixture` actually exposes
— read that file's existing tests for the precise helper signatures
before writing this one; the assertion and the `b=0.0` contrast are
what's fixed, not the exact commit-helper call shape.)

- [ ] **Step 7: Run the test to verify it fails, then implement if any gap remains, then verify it passes**

Run: `cargo test -p index with_bm25_params_changes_real_search_scores`
Expected: FAIL initially only if Step 5's wiring has a gap (it shouldn't,
since Step 5 already wires the field through) — if it fails for any
reason other than "not yet compiling because the field doesn't exist
yet" (which Step 5 already resolved), treat it as `systematic-debugging`
territory, not a sign to adjust the test's expectation.
Re-run after any fix: Expected: PASS.

- [ ] **Step 8: Run the full `index` and `api` test suites for regressions**

Run: `cargo test -p index` and `cargo test -p api --lib` (library tests
only, not the examples) — Expected: PASS, same counts as before this
task plus the two new tests.

- [ ] **Step 9: Commit**

```bash
git add crates/index/src/bm25.rs crates/index/src/search.rs crates/index/tests/b1_search.rs
git commit -m "feat(index): additive Bm25Index/Index constructor for custom k1/b, defaults unchanged"
```

---

### Task 2: Lever A ablation binary — BM25 `k1`/`b` grid

**Files:**
- Create: `crates/api/examples/b3_tune_v2_lever_a_bm25_params.rs`
- Create: `eval/results/B3-TUNE-v2/priority2-lever-a-grid.json`

**Interfaces:**
- Consumes: Task 1's `Index::with_bm25_params`, `api::metrics::{recall_at_10,
  ndcg_at_10, confusion_matrix_metrics, ConfusionMatrix}` (existing,
  unchanged), the same `TestFixture`/`LabelRow`/`parse_label_rows`/
  `exclude_canary`/`run_one_query` pattern `b3_metric_runner.rs` and
  `b3_tune_v2_signal_analysis.rs` already use (duplicate the minimal
  fixture code into this new file, per the established pattern — this
  plan does not refactor either existing example file to share code).
- Produces: `eval/results/B3-TUNE-v2/priority2-lever-a-grid.json` (dev
  results for every grid point) and, after the grid is reviewed,
  `priority2-lever-a-dev-ablation.json` /
  `priority2-lever-a-calibration-ablation.json` (Task 3's deliverable,
  using the grid's chosen candidate).

- [ ] **Step 1: Write the failing test for one grid point's end-to-end run**

```rust
#[test]
fn grid_point_runs_against_the_real_corpus_and_returns_metrics() {
    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index_with_bm25_params(1.2, 0.75); // same as B1 default, sanity check
    let report = run_split_with_bm25_params("dev", 1.2, 0.75);
    assert_eq!(report.k1, 1.2);
    assert_eq!(report.b, 0.75);
    assert!(report.aggregate.recall_at_10 > 0.0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p api --example b3_tune_v2_lever_a_bm25_params`
Expected: FAIL (`build_index_with_bm25_params`/`run_split_with_bm25_params` not yet defined).

- [ ] **Step 3: Implement the grid-point runner**

`fn build_index_with_bm25_params(&self, k1: f64, b: f64) -> index::Index<'_>`
on the duplicated `TestFixture` (same as the existing `build_index`, but
calling `.with_bm25_params(k1, b)` before returning). `struct
GridPointReport { k1: f64, b: f64, aggregate: MetricBreakdown, ... }`
(reuse `b3_metric_runner.rs`'s `MetricBreakdown`/`AbstentionReport`
shapes by duplicating their definitions, same pattern Task 1's sibling
files already use) and `fn run_split_with_bm25_params(split: &str, k1:
f64, b: f64) -> GridPointReport` — loads the split's labels, builds the
index with the given params, runs every non-canary query through
`api::search`, scores with `api::metrics::recall_at_10`/`ndcg_at_10`,
computes the abstention confusion matrix the same way
`b3_metric_runner.rs`'s `run_split` does.

- [ ] **Step 4: Run the test to verify it passes**

Run the same command as Step 2. Expected: PASS.

- [ ] **Step 5: Implement `main` to run the grid over dev and write the report**

Candidate grid (fixed by this plan, per the design doc's "small, fixed
candidate set" choice): `k1 ∈ {0.9, 1.2, 1.5, 1.8}`, `b ∈ {0.4, 0.6,
0.75, 0.9}` — 16 combinations. `main()` runs `run_split_with_bm25_params("dev",
k1, b)` for all 16, writes the full list of `GridPointReport`s to
`eval/results/B3-TUNE-v2/priority2-lever-a-grid.json`, and prints a
short table (k1, b, Recall@10, nDCG@10) to stdout for immediate
inspection.

- [ ] **Step 6: Run it against the real dev split**

Run: `cargo run --release -p api --example b3_tune_v2_lever_a_bm25_params`
Expected: exits 0, writes the report, prints the 16-row table.

- [ ] **Step 7: Commit**

```bash
git add crates/api/examples/b3_tune_v2_lever_a_bm25_params.rs eval/results/B3-TUNE-v2/priority2-lever-a-grid.json
git commit -m "feat(eval): B3-TUNE-v2 Priority 2 Lever A BM25 k1/b grid over dev"
```

---

### Task 3: Lever A dev/calibration ablation and KEEP/REJECT

**Files:**
- Create: `eval/results/B3-TUNE-v2/priority2-lever-a-dev-ablation.json`,
  `priority2-lever-a-calibration-ablation.json`

**Interfaces:**
- Consumes: Task 2's grid report, the official B1 baseline
  (`eval/results/B3/frozen-config.json`,
  `docs/research/B3-CLOSURE-SUMMARY.md`).
- Produces: the dev/calibration ablation reports plus a KEEP/REJECT
  decision, folded into Task 9's combined summary (this task does not
  write its own separate summary doc — Task 9 covers all three levers
  in one place, per "Proportion" self-review discipline).

- [ ] **Step 1: Read Task 2's grid and pick ONE candidate for calibration**

This is an analysis step: read `priority2-lever-a-grid.json`'s 16 rows,
identify the single `(k1, b)` pair with the best dev Recall@10+nDCG@10
combination (ties broken toward values closest to the current defaults
`1.2`/`0.75`, to avoid picking an extreme grid edge on noisy dev data) —
record the chosen pair and a one-paragraph rationale as a new field in
a hand-edited copy of the grid JSON, or a short inline note in Task 9's
summary (either is fine; this is data/rationale, not executable logic).
**If the best dev candidate is identical to the current defaults**
(`1.2`/`0.75` ties or wins), that is itself a valid, reportable outcome —
do not force a different pick to have something to show.

- [ ] **Step 2: Run the chosen candidate against dev and calibration, writing full ablation reports**

Run `run_split_with_bm25_params` (Task 2's function) for both `dev` and
`calibration` with the chosen `(k1, b)`, writing
`priority2-lever-a-dev-ablation.json` /
`priority2-lever-a-calibration-ablation.json` in the full schema (per
Global Constraints: config, commit, hashes, Recall@10+CI, nDCG@10+CI,
confusion matrix, delta vs. official B1 baseline, "not measured" for
latency).

- [ ] **Step 3: Commit**

```bash
git add eval/results/B3-TUNE-v2/priority2-lever-a-dev-ablation.json eval/results/B3-TUNE-v2/priority2-lever-a-calibration-ablation.json
git commit -m "docs(eval): B3-TUNE-v2 Priority 2 Lever A dev/calibration ablation"
```

---

### Task 4: Lever B diagnostic — token-level audit of the proposed normalization rule

**Files:**
- Create: `crates/index/src/lexical_normalize.rs`
- Create: `eval/results/B3-TUNE-v2/priority2-lever-b-token-diagnostic.json`

**Interfaces:**
- Consumes: `eval/rag/corpus/notes/*.md` (via `Document::from_record_bytes`
  on each note's raw bytes, same accessor Priority 1's binaries didn't
  need but is already public — read the note file, call
  `storage::split_record`/`Document::new` directly, or simplest: read
  each note's full text and pass it through the existing
  `build_text_searchable`-equivalent path by reusing `Document::from_record_bytes`),
  `eval/rag/dev/labels.jsonl` (question text only — this is a token
  diagnostic, not a search run).
- Produces: `strip_trailing_sentence_punctuation(token: &str) -> &str`
  (pure function, new module, **not** wired into `bm25::tokenize`
  itself by this task — it is a standalone function the diagnostic and
  later Task 5's ablation binary both call explicitly, keeping B3's
  frozen `bm25::tokenize` completely untouched, per Global Constraints'
  "no existing call site changes behavior"). Consumed by Task 5's
  ablation binary and its own diagnostic main.

- [ ] **Step 1: Write the failing tests for the conservative stripping rule**

```rust
#[test]
fn strips_trailing_sentence_punctuation() {
    assert_eq!(strip_trailing_sentence_punctuation("bm25,"), "bm25");
    assert_eq!(strip_trailing_sentence_punctuation("index."), "index");
    assert_eq!(strip_trailing_sentence_punctuation("really?"), "really");
}

#[test]
fn preserves_technical_identifiers_unchanged() {
    assert_eq!(strip_trailing_sentence_punctuation("c++"), "c++");
    assert_eq!(strip_trailing_sentence_punctuation("c#"), "c#");
    assert_eq!(strip_trailing_sentence_punctuation("sec-001"), "sec-001");
    assert_eq!(strip_trailing_sentence_punctuation("foo::bar"), "foo::bar");
    assert_eq!(strip_trailing_sentence_punctuation("foo.bar"), "foo.bar");
    assert_eq!(strip_trailing_sentence_punctuation("snake_case"), "snake_case");
    assert_eq!(strip_trailing_sentence_punctuation("--split"), "--split");
    assert_eq!(strip_trailing_sentence_punctuation("crates/api/src/metrics.rs"), "crates/api/src/metrics.rs");
}

#[test]
fn never_strips_leading_or_interior_punctuation() {
    assert_eq!(strip_trailing_sentence_punctuation("(parenthetical"), "(parenthetical");
    assert_eq!(strip_trailing_sentence_punctuation("mid.dle"), "mid.dle");
}

#[test]
fn does_not_strip_a_token_that_is_entirely_punctuation() {
    assert_eq!(strip_trailing_sentence_punctuation("..."), "...");
    assert_eq!(strip_trailing_sentence_punctuation(")"), ")");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p index lexical_normalize`
Expected: FAIL (`strip_trailing_sentence_punctuation` not yet defined).

- [ ] **Step 3: Implement `strip_trailing_sentence_punctuation`**

```rust
const TRAILING_SENTENCE_PUNCTUATION: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\''];
```

Strip characters from `TRAILING_SENTENCE_PUNCTUATION` from the token's
end, repeatedly, **only** while the remainder after stripping is
nonempty (the "does not strip a token that is entirely punctuation"
test case: `"..."` and `")"` must pass through unchanged because
stripping all of it would leave nothing). Leading and interior
characters are never inspected or modified.

- [ ] **Step 4: Run the tests to verify they pass**

Run the same command as Step 2. Expected: PASS.

- [ ] **Step 5: Write the corpus+dev token diagnostic `main`**

A `main()` (in `crates/api/examples/b3_tune_v2_lever_b_normalization.rs`,
created now even though its ablation logic is Task 5's job — this
file's diagnostic mode is Task 4's deliverable, its ablation mode is
Task 5's addition to the same file) that: reads every corpus note's
`text_searchable` (via `Document::from_record_bytes` on each note file's
raw bytes) plus every dev-split question's text, tokenizes each with
`bm25::tokenize` (the EXISTING, unchanged tokenizer — this diagnostic
observes what `strip_trailing_sentence_punctuation` would do to real
tokens, it doesn't change tokenization yet), applies
`strip_trailing_sentence_punctuation` to every distinct token, and
reports every token where the stripped form differs from the original —
for manual review, not automated pass/fail. Write the full list (token,
stripped form) to
`eval/results/B3-TUNE-v2/priority2-lever-b-token-diagnostic.json`.

- [ ] **Step 6: Run the diagnostic and review its output**

Run: `cargo run --release -p api --example b3_tune_v2_lever_b_normalization -- --diagnostic`
Expected: exits 0, writes the report. This is an analysis step: read the
output and confirm every affected token is a genuine sentence-boundary
case (e.g. `"corpus,"` → `"corpus"`), not a technical identifier the
Review Focus/design doc's conservative-policy requirement would flag as
a false positive. **If any affected token is a real technical identifier
being incorrectly stripped, narrow `TRAILING_SENTENCE_PUNCTUATION`**
(e.g. drop a character) and re-run this step before proceeding — do not
proceed with a known collision.

- [ ] **Step 7: Commit**

```bash
git add crates/index/src/lexical_normalize.rs crates/api/examples/b3_tune_v2_lever_b_normalization.rs eval/results/B3-TUNE-v2/priority2-lever-b-token-diagnostic.json
git commit -m "feat(index): conservative trailing-punctuation stripping, tested in isolation + corpus/dev diagnostic"
```

---

### Task 5: Lever B ablation — normalization in the real search path

**Files:**
- Modify: `crates/api/examples/b3_tune_v2_lever_b_normalization.rs`
  (adds ablation mode alongside Task 4's diagnostic mode)
- Create: `eval/results/B3-TUNE-v2/priority2-lever-b-dev-ablation.json`,
  `priority2-lever-b-calibration-ablation.json`

**Interfaces:**
- Consumes: Task 4's `strip_trailing_sentence_punctuation`, the same
  duplicated `TestFixture`/metrics pattern Task 2 uses.
- Produces: the dev/calibration ablation reports, same schema as Task 3's.

- [ ] **Step 1: Write the failing test for a normalized-tokenizer search**

```rust
#[test]
fn a_query_with_trailing_punctuation_matches_the_same_as_without() {
    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index(); // plain B1 index, unmodified
    let with_comma = run_one_query_normalized(&idx, "resolve_configured_timezone,", 10);
    let without_comma = run_one_query_normalized(&idx, "resolve_configured_timezone", 10);
    assert_eq!(with_comma, without_comma);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p api --example b3_tune_v2_lever_b_normalization normalized`
Expected: FAIL (`run_one_query_normalized` not yet defined).

- [ ] **Step 3: Implement `run_one_query_normalized`**

`fn run_one_query_normalized(idx: &index::Index<'_>, question: &str, k: usize) -> QueryOutcome` —
applies `strip_trailing_sentence_punctuation` to `question` **split on
whitespace, stripped per-token, rejoined with single spaces** before
calling `api::search` with the rejoined string (this ablation tests the
effect of normalizing the QUERY text; the design doc's lever is about
query-time matching quality, and the indexed corpus text is already
fixed/frozen — normalizing only the query side is the correct, minimal
test of this lever without touching indexed content). Otherwise
identical to the existing `run_one_query` pattern (same `QueryOutcome`
enum, same `SearchOutcome` match).

- [ ] **Step 4: Run the test to verify it passes**

Run the same command as Step 2. Expected: PASS.

- [ ] **Step 5: Implement the ablation `main` mode and run it against dev + calibration**

Extend `main()` with a `--ablate <split>` mode (alongside Task 4's
`--diagnostic`) that runs every non-canary row of the given split
through `run_one_query_normalized`, scores with
`api::metrics::recall_at_10`/`ndcg_at_10`, and writes the full ablation
report (same schema as Task 3). Run:
`cargo run --release -p api --example b3_tune_v2_lever_b_normalization -- --ablate dev`
then `-- --ablate calibration`.
Expected: both exit 0, write their reports.

- [ ] **Step 6: Commit**

```bash
git add crates/api/examples/b3_tune_v2_lever_b_normalization.rs eval/results/B3-TUNE-v2/priority2-lever-b-dev-ablation.json eval/results/B3-TUNE-v2/priority2-lever-b-calibration-ablation.json
git commit -m "feat(eval): B3-TUNE-v2 Priority 2 Lever B normalization dev/calibration ablation"
```

---

### Task 6: Lever C diagnostic — identify alias candidates from corpus + dev only

**Files:**
- Create: `crates/api/examples/b3_tune_v2_lever_c_alias_table.rs`
- Create: `eval/results/B3-TUNE-v2/priority2-lever-c-alias-candidates.json`

**Interfaces:**
- Consumes: `eval/rag/corpus/notes/*.md` (frontmatter `source` fields
  and body text), `eval/rag/dev/labels.jsonl` (question text and target
  `relevant_note_ids` only — **never `eval/rag/calibration/labels.jsonl`
  at any point in this task**, per design doc correction #1).
- Produces: `eval/results/B3-TUNE-v2/priority2-lever-c-alias-candidates.json`,
  a list of candidate `(short_form, expansion)` pairs with the
  corpus/dev evidence justifying each. Human-reviewed (Step 4) before
  any candidate becomes a frozen table entry (Task 7).

- [ ] **Step 1: Write the failing test for structural candidate extraction**

```rust
#[test]
fn extracts_a_source_identifier_to_body_phrase_candidate() {
    let note_text = std::fs::read_to_string(
        format!("{CORPUS_NOTES_DIR}/bug-2026-10-27-sec001-toctou.md")
    ).unwrap();
    let candidates = extract_structural_candidates(&note_text);
    assert!(candidates.iter().any(|c| c.short_form == "SEC-001"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p api --example b3_tune_v2_lever_c_alias_table extracts_a_source`
Expected: FAIL (`extract_structural_candidates` not yet defined).

- [ ] **Step 3: Implement `extract_structural_candidates`**

`struct AliasCandidate { short_form: String, note_id: String, evidence: String }`.
`fn extract_structural_candidates(note_text: &str) -> Vec<AliasCandidate>` —
parses the note's frontmatter `source:` field (e.g.
`docs/technical-debt.md#SEC-001`) for a trailing `#<IDENTIFIER>`
fragment; if present, the identifier is a candidate `short_form`, with
`evidence` set to the note's own `id:` frontmatter value (this plan
does not require NLP-level "find the plain-English description" —
Step 5's human-review step is where the actual `expansion` text gets
chosen, by reading the note body, not synthesized automatically).

- [ ] **Step 4: Run the test to verify it passes**

Run the same command as Step 2. Expected: PASS.

- [ ] **Step 5: Implement `main` to scan the full corpus + dev, cross-reference, and write candidates**

`main()` runs `extract_structural_candidates` over every corpus note,
then cross-references each candidate's `note_id` against
`eval/rag/dev/labels.jsonl`'s `relevant_note_ids` (does any real dev
query target a note with a structural short-form alias candidate? —
this is the evidence that the alias would actually help a real query,
not just that the corpus happens to contain an identifier). Writes
every candidate (whether or not a dev query references it, but flagging
which ones do) to
`eval/results/B3-TUNE-v2/priority2-lever-c-alias-candidates.json`.

- [ ] **Step 6: Run it and manually review the output**

Run: `cargo run --release -p api --example b3_tune_v2_lever_c_alias_table -- --diagnostic`
Expected: exits 0, writes the candidates file. This is an analysis step:
read the output, and for each candidate flagged as dev-referenced, read
that note's actual body to write a real `expansion` phrase (e.g.
`"SEC-001"` → `"symlink TOCTOU bug"`, following the design doc's own
illustrative examples) — hand-written, not auto-generated, since the
expansion must be a phrase a real paraphrased query would plausibly use,
which requires reading the note like a person would. **If zero
candidates are dev-referenced, that is Option C3 from the design doc
(no alias table this round) — a legitimate, explicitly reportable
outcome; do not force a table into existence.**

- [ ] **Step 7: Commit**

```bash
git add crates/api/examples/b3_tune_v2_lever_c_alias_table.rs eval/results/B3-TUNE-v2/priority2-lever-c-alias-candidates.json
git commit -m "feat(eval): B3-TUNE-v2 Priority 2 Lever C alias candidate diagnostic (corpus+dev only)"
```

---

### Task 7: Lever C — freeze the alias table and implement query-time expansion

**Files:**
- Modify: `crates/api/examples/b3_tune_v2_lever_c_alias_table.rs`
- Create: `eval/results/B3-TUNE-v2/priority2-lever-c-alias-table.json`

**Interfaces:**
- Consumes: Task 6's human-reviewed candidates (the ones with a written
  `expansion`, from Step 6's manual review — this task does not
  re-derive expansions, it freezes what Task 6 produced).
- Produces: `fn apply_alias_expansion(query: &str, table: &HashMap<String, String>) -> String`
  (whole-token matching only, case-insensitive on the short form), the
  frozen `priority2-lever-c-alias-table.json`. Consumed by Task 8's
  ablation.

- [ ] **Step 1: Write the failing tests for whole-token alias expansion**

```rust
#[test]
fn expands_a_whole_token_match() {
    let table = HashMap::from([("sec-001".to_string(), "symlink toctou bug".to_string())]);
    assert_eq!(apply_alias_expansion("what is sec-001 about", &table), "what is sec-001 about symlink toctou bug");
}

#[test]
fn does_not_expand_a_substring_match_inside_a_different_token() {
    let table = HashMap::from([("sec".to_string(), "security".to_string())]);
    assert_eq!(apply_alias_expansion("see the section below", &table), "see the section below");
}

#[test]
fn is_case_insensitive_on_the_short_form() {
    let table = HashMap::from([("sec-001".to_string(), "symlink toctou bug".to_string())]);
    assert_eq!(apply_alias_expansion("SEC-001 details", &table), "SEC-001 details symlink toctou bug");
}
```

(Chosen behavior: expansion APPENDS the expansion phrase to the query
rather than replacing the short form, so the original token remains
matchable too — this is a plan decision, not left to the implementer:
appending is strictly additive to query-term coverage and cannot reduce
recall the way a replacement could if the expansion phrase were
imperfect.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p api --example b3_tune_v2_lever_c_alias_table apply_alias_expansion`
Expected: FAIL (`apply_alias_expansion` not yet defined).

- [ ] **Step 3: Implement `apply_alias_expansion`**

Tokenize `query` on whitespace (reuse `bm25::tokenize`'s own
whitespace-splitting logic conceptually, but do not lowercase-destroy
the original casing of the returned string — only use lowercasing for
the table lookup itself). For each whitespace-delimited token, lowercase
it and look it up in `table`; if found, note its expansion. After
scanning all tokens, return the original `query` with every found
expansion phrase appended once (deduplicated if the same short form
appears twice), separated by spaces.

- [ ] **Step 4: Run the tests to verify they pass**

Run the same command as Step 2. Expected: PASS.

- [ ] **Step 5: Write and freeze the alias table from Task 6's reviewed candidates**

Write `eval/results/B3-TUNE-v2/priority2-lever-c-alias-table.json`
containing exactly the candidates Task 6 Step 6 hand-wrote an expansion
for (a `{short_form: expansion}` map plus a `rationale` field per entry
citing the corpus note id and the dev query it was justified against).
**This file, once committed, is the frozen table — Task 8's ablation
reads it, calibration never contributes to it.**

- [ ] **Step 6: Commit**

```bash
git add crates/api/examples/b3_tune_v2_lever_c_alias_table.rs eval/results/B3-TUNE-v2/priority2-lever-c-alias-table.json
git commit -m "feat(eval): B3-TUNE-v2 Priority 2 Lever C frozen alias table + query-time expansion"
```

---

### Task 8: Lever C ablation — frozen alias table against dev then calibration

**Files:**
- Modify: `crates/api/examples/b3_tune_v2_lever_c_alias_table.rs`
- Create: `eval/results/B3-TUNE-v2/priority2-lever-c-dev-ablation.json`,
  `priority2-lever-c-calibration-ablation.json`

**Interfaces:**
- Consumes: Task 7's frozen `priority2-lever-c-alias-table.json`,
  Task 7's `apply_alias_expansion`.
- Produces: the dev/calibration ablation reports, same schema as
  Tasks 3/5's.

- [ ] **Step 1: Write the failing test for an alias-expanded search**

```rust
#[test]
fn an_alias_expanded_query_matches_what_the_expansion_alone_would() {
    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index();
    let table = load_frozen_alias_table(); // reads priority2-lever-c-alias-table.json
    let expanded = run_one_query_with_aliases(&idx, "tell me about sec-001", 10, &table);
    let direct = run_one_query_with_aliases(&idx, "tell me about symlink toctou bug", 10, &table);
    // The expanded query's results should include the bug note; this
    // doesn't require exact equality with `direct`, only that expansion
    // succeeded in surfacing the target note the short form alone might miss.
    assert!(matches!(expanded, QueryOutcome::Matches { .. }));
}
```

(Adapt the exact query text to whichever real alias Task 7 actually
froze — this illustrates the shape, not a literal fixed string if
`SEC-001` ends up not being one of the human-reviewed, dev-justified
entries.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p api --example b3_tune_v2_lever_c_alias_table run_one_query_with_aliases`
Expected: FAIL (`load_frozen_alias_table`/`run_one_query_with_aliases` not yet defined).

- [ ] **Step 3: Implement `load_frozen_alias_table` and `run_one_query_with_aliases`**

`load_frozen_alias_table() -> HashMap<String, String>` reads and parses
`eval/results/B3-TUNE-v2/priority2-lever-c-alias-table.json`'s
`{short_form: expansion}` map (ignoring the `rationale` field).
`run_one_query_with_aliases(idx, question, k, table) -> QueryOutcome`
calls `apply_alias_expansion(question, table)` first, then runs the
expanded string through `api::search` exactly like the existing
`run_one_query` pattern.

- [ ] **Step 4: Run the test to verify it passes**

Run the same command as Step 2. Expected: PASS.

- [ ] **Step 5: Implement the ablation `main` mode and run it against dev + calibration**

Extend `main()` with an `--ablate <split>` mode: loads the frozen table,
runs every non-canary row of the given split through
`run_one_query_with_aliases`, scores with
`api::metrics::recall_at_10`/`ndcg_at_10`, writes the full ablation
report. Run:
`cargo run --release -p api --example b3_tune_v2_lever_c_alias_table -- --ablate dev`
then `-- --ablate calibration`.
Expected: both exit 0, write their reports.

- [ ] **Step 6: Commit**

```bash
git add crates/api/examples/b3_tune_v2_lever_c_alias_table.rs eval/results/B3-TUNE-v2/priority2-lever-c-dev-ablation.json eval/results/B3-TUNE-v2/priority2-lever-c-calibration-ablation.json
git commit -m "feat(eval): B3-TUNE-v2 Priority 2 Lever C frozen alias table dev/calibration ablation"
```

---

### Task 9: Stage 1 summary — KEEP/REJECT per lever, STOP for user review

**Files:**
- Create: `eval/results/B3-TUNE-v2/priority2-stage1-summary.md`

**Interfaces:**
- Consumes: all three levers' dev/calibration ablation reports (Tasks
  3, 5, 8), the official B1 baseline numbers
  (`docs/research/B3-CLOSURE-SUMMARY.md`).
- Produces: the written Stage 1 summary — the plan's final deliverable.

- [ ] **Step 1: Write the per-lever KEEP/REJECT sections**

For each of Lever A, B, C: report the full config (exact `k1`/`b` or
the normalization rule or the frozen alias table), commit, corpus/
labels hashes (already in each ablation report), Recall@10 (+CI),
nDCG@10 (+CI), the abstention confusion matrix and whether it changed
from B1's baseline pattern (constant non-abstaining — flag explicitly
if any lever caused even one `NotPresent` where B1 didn't, or vice
versa), latency ("not measured"), delta vs. the **official** B1
baseline (never vs. another lever), and an explicit KEEP/REJECT
decision with rationale, checked against: does calibration Recall@10/
nDCG@10 move in the right direction without a false-abstention-rate-style
hidden cost (there is no abstention tuning here, but a lever that
degrades Recall@10 while not improving anything else is a REJECT
regardless of dev's own numbers, same "calibration decides" discipline
Priority 1 used).

- [ ] **Step 2: Note whether any lever moved Priority 1's signal separation (secondary, non-gating)**

If time/scope allows within this task (optional, not blocking): for any
lever marked KEEP, note in one paragraph whether re-running
`b3_tune_v2_signal_analysis`'s existing `ratio`-signal computation under
that lever's configuration shows the calibration `separation_gap`
moving toward less overlap (closer to 0) or more. Explicitly label this
**informational only** — it does not change any KEEP/REJECT decision in
this task, and does not reopen Priority 1's closed threshold search.

- [ ] **Step 3: Commit**

```bash
git add eval/results/B3-TUNE-v2/priority2-stage1-summary.md
git commit -m "docs(eval): B3-TUNE-v2 Priority 2 Stage 1 KEEP/REJECT summary (Levers A/B/C, isolated)"
```

- [ ] **Step 4: STOP and report to the user**

Report all three levers' KEEP/REJECT decisions and rationale. Explicitly
state: no composite configuration has been built or measured (Stage 2
requires separate authorization); the held-out set remains unread; no
Priority 3/4 work has started.

---

## What this plan stops short of

This plan ends at Task 9's Stage 1 KEEP/REJECT report. It does **not**
include:
- Stage 2 (composing any accepted levers together) — requires the
  user's separate, explicit authorization after reviewing this plan's
  Stage 1 results (design doc correction #2).
- Running the new v2 held-out final-test, or using it to decide
  anything — out of scope, unread throughout.
- Priority 3 (reranking) or Priority 4 (hybrid retrieval) — design-only
  until separately planned and authorized.
- Declaring any Stage 1 lever, alone or composed, the new official
  baseline — that requires Stage 2 (if authorized) and, eventually, the
  full priority-chain stop/proceed decision the parent B3-TUNE-v2
  design describes.
- Making BM25 `k1`/`b` end-user-configurable as a product feature
  (explicit non-goal) — Task 1's constructor additions exist only for
  this plan's ablation binaries to call; nothing in production code
  (the real indexing/search pipeline's own default construction)
  changes behavior.

## Self-Review Notes

- **Spec coverage**: the design doc's three levers (A/B/C) each map to
  a dedicated pair of tasks (build + ablate: Tasks 1-2-3 for A, 4-5 for
  B, 6-7-8 for C), its two-stage correction maps to this plan's own
  scope boundary (Stage 1 tasks only, Stage 2 explicitly excluded in
  "What this plan stops short of"), its calibration-never-selects-aliases
  correction maps to Task 6's "corpus + dev only" consumption and Task
  7's freeze-then-evaluate ordering, and its conservative-normalization
  correction maps to Task 4's narrow character set + diagnostic-gated
  finalization. The secondary signal-separation diagnostic (design
  doc's Goal 2) maps to Task 9 Step 2, explicitly non-gating.
- **Step scan**: Task 3 Step 1, Task 6 Step 6, and Task 2's grid-pick
  rule each flag a real judgment call (which grid point to carry
  forward, whether any alias candidate is dev-justified, tie-breaking
  toward current defaults) and give the implementer an explicit
  tie-break rule or a legitimate "do nothing" outcome (C3) rather than
  forcing a decision where the evidence doesn't support one.
- **Type consistency**: `QueryOutcome` (Task 5, Task 8) matches the
  existing enum shape from `b3_metric_runner.rs`/`b3_tune_v2_signal_analysis.rs`
  exactly (`NotPresent` / `Matches { retrieved_note_ids }`); `GridPointReport`
  (Task 2) is defined once and referenced by Task 3 only for reading its
  JSON output, never re-declared; `AliasCandidate` (Task 6) and the
  frozen table's `{short_form: expansion}` shape (Task 7) are used
  consistently from Task 6 through Task 8.
- **Review Focus**: each of the 4 named risks has an owning task —
  the candidate-prefilter-vs-final-scoring distinction (Task 1 Step 6's
  `b=0.0` contrast test, chosen specifically because it's a large,
  visible effect isolated to final scoring), the token-collision risk
  (Task 4's diagnostic-before-finalize gate), the substring-vs-whole-token
  alias risk (Task 7 Step 1's dedicated test), and the silent-abstention-drift
  risk (every ablation's confusion-matrix field, explicit in Task 9
  Step 1's reporting requirement).
- **Proportion**: 9 tasks, each small (one pure function or one
  additive constructor plus one diagnostic/ablation binary), for a
  design doc whose own scope is three parallel, independent levers —
  proportionate given the plan's real complexity is in correctly
  isolating each lever's measurement from the others and from
  calibration, not in code volume.
