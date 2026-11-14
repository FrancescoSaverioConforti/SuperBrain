# B2-EVAL-010: JavaScript/CommonJS domain standalone evaluation

**Evaluation ID:** B2-EVAL-010
**Status:** COMPLETE. No new Critical/Important production defect found.
Three new Moderate findings opened (B2-ID-013, B2-ID-014, B2-ID-015), all
in the same gap family: CommonJS-specific extraction blind spots that
have no equivalent in any previously-evaluated language/domain. All
three are honest silent drops (zero false edges found), not correctness
defects. This is the first standalone evaluation of pure CommonJS
JavaScript (prior JS/TS exposure, in B2-EVAL-004, was ESM-flavored
Frontend/UI TypeScript, not legacy CommonJS backend code).
**Date:** 2026-11-13.
**SuperBrain commit (baseline, HEAD at evaluation time):** branch
`b2-code-graph`, worktree `.worktrees/b2-code-graph`. New files this
round: one example harness
(`crates/codegraph-index/examples/b2_eval_010_js_commonjs.rs`), this
report, and four raw-evidence files under `docs/research/evidence/`.
No merge/tag/push. No production code changed. Task 10 untouched.
CALL-RES-A5 not started. B2-EVAL-011 not auto-started.

**Note on process**: a first dispatch attempt for this round went
off-scope and returned an unrelated untracked-file cleanup audit
instead of executing this checkpoint. That report was discarded
entirely per explicit user instruction (recorded in Continuity as
rejection [x2gi]) and is not referenced further here. This report
reflects a second, independent dispatch that executed the actual
B2-EVAL-010 protocol.

## Repo selection

**Repository:** `request/request` (https://github.com/request/request)
**Commit:** `3c0cddc7c8eb60b470e9519da85896ed7ee0081e`
**License:** Apache-2.0.
**Corpus scope:** `request.js`, `index.js`, `lib/*.js` — 13 files, pure
CommonJS (`require()`/`module.exports`/`exports.X =`), zero ESM
(`import`/`export`) syntax anywhere in scope. 2,991 LOC.

**Selection reasoning:** `request` is a long-lived, widely-used Node.js
HTTP client library, pre-ESM-era, exercising CommonJS idioms the
existing suite has never evaluated as a dedicated domain: bare
`require('./relative')` and `require('pkg')` calls (61 real sites),
`module.exports = Identifier` re-exports, and — critically — the ES5
"class via prototype" idiom (`Request.prototype.method = function()
{...}`) alongside `exports.X = function(){...}` property-assignment
exports, neither of which resembles the `function_declaration`/
`class_declaration` shapes the existing JS/TS extractor already
handles. Medium-sized, real production code, permissive license,
genuinely novel relative to B2-EVAL-004 (TypeScript/ESM Frontend-UI).

## Corpus metrics

| Metric | Value |
|---|---|
| Files in scope | 13 |
| Files parsed, 0 error nodes | 13/13 |
| Parse failures | 0 |
| LOC (newline count) | 2,991 |
| Raw extraction candidates | 40 |
| Extraction rejections | 0 |
| Snapshots promoted | 40 (all Function) |
| Raw `call_expression` nodes (library files) | 670 |
| Raw `require(...)` call sites | 61 (15 relative, 46 package) |
| `exports.X = function(){...}` sites | 4 |
| `Request.prototype.X = function(){...}` sites (`request.js`) | 27 |

## Calls / Imports / structural edges

| Edge kind | Count | Resolution method |
|---|---|---|
| Calls | 12 | StaticAnalysis: 12 |
| Contains | 0 | — |
| Extends | 0 | — |
| Instantiates | 0 | — |

**ImportFacts from `require()`: 0, structural, confirmed by direct
source read of `imports.rs`'s `collect_js_import`**, which only
matches `import_statement` (ESM). Every one of the 61 real
`require()` call sites in this corpus produces zero `ImportFact`s —
not a corpus artifact, a structural gap: CommonJS has no import-alias
data for CALL-RES-A4's resolver to consult, exactly analogous to the
C++ `#include` gap found in B2-EVAL-009, but for a different
structural reason (missing extraction, not a deliberate no-op).

## Reliability tiers / CallAttempt (negative knowledge)

| Tier | Count |
|---|---|
| Resolved (StaticAnalysis) | 12 |
| Ambiguous | 0 |
| Unresolved | 68 |

`UnresolvedReason` distribution: `Unknown` 68/68 (100%). None of the
existing reasons (`ImportTargetExternal`, `AmbiguousCandidates`,
`ExtractionRejected`, etc.) fire for this corpus — confirmed by direct
measurement, not inferred — because the real root cause (callees
that were never extracted as candidates at all, see Findings below)
sits upstream of where any of those reasons get assigned. This is
itself informative: a 100%-`Unknown` distribution is a signal that the
existing reason taxonomy has no vocabulary yet for "callee never
became a candidate," as opposed to "callee became a candidate but
resolution failed for a specific, classified cause."

## FP/FN (ground-truth sample, hand-verified against real source)

Hand-verified against direct reads of `request.js`/`index.js`/
`lib/*.js` (the harness's own sections F/H/H2/H3 as supporting raw
evidence): **Precision = 1.000, Recall = 0.357, F1 = 0.526** on the
sampled rows. **Zero false positives** — confirmed exhaustively across
all 27 real `Request.prototype.X` methods (section H2: `false_edge_count
= 0`) and across every sampled cross-file call site (section F: all
four real `cookies.jar()`/`cookies.parse()`/`hawk.header()` call sites
produce `resolved_edges=0, call_attempts=0`, never a wrong edge).

Concrete FN examples:
- `cookies.jar()` (`request.js:39`, `index.js:76`) and `cookies.parse()`
  (`index.js:80`) target `exports.jar`/`exports.parse` in
  `lib/cookies.js` — real, exported, called functions that were never
  extracted as candidates at all (zero snapshot, zero edge, zero
  CallAttempt — total silence, not a hedged non-answer).
- All 27 `Request.prototype.X = function(){...}` methods in
  `request.js` (the entire `Request` class's real method surface) are
  invisible for the same structural reason, at far larger magnitude.

Where the extractor's existing `function_declaration`-style shapes
apply (named, non-prototype, non-property-assigned functions), lookup
is correct and reliable: all 7 sampled `lib/helpers.js` functions
(`paramsHaveRequestBody`, `safeStringify`, `isReadStream`, `toBase64`,
`copy`, `md5`, plus one correctly-absent `defer`) were found exactly
where expected, confirming the gap is specific to the two export
idioms above, not a general CommonJS parsing failure.

## Agent usefulness + reliability

- **"Who calls `cookies.jar()`?"** — **not answerable at all.** The
  real, exported, twice-called function was never extracted, so it
  cannot appear as a resolved edge, an Ambiguous CallAttempt, or even
  an Unresolved CallAttempt. Silent absence for a function that
  genuinely exists and is genuinely called — the single worst case in
  this evaluation, since it is strictly less honest than a classified
  Unresolved.
- **"Can I trust `require('./x')` as an import alias for resolution?"**
  — **No.** Zero ImportFacts from any `require()` call; a
  `require`-then-call pattern falls back to plain name-based
  heuristic/ambiguous resolution, exactly as it would pre-CALL-RES-A4.
  No regression, no false edge — but none of A4's precision gain
  either.
- **"What helper functions exist in a real multi-export file?"** —
  **Fully answerable and reliable** for `function_declaration`-style
  exports (see FP/FN section above) — the gap is confined to the two
  property-assignment export idioms, not CommonJS generally.

## Full/incremental parity

**Confirmed via the existing equivalence gate
(`compare_full_incremental_state`), real commit pair
`be7882b0...`→`df346d8531...`, 3 real changed files (`package.json`,
`request.js`, `tests/test-timeout.js`).** Zero divergences across
snapshots/edges/import_facts/impl_facts/call_attempts multisets (42
edges both sides, 329 CallAttempts both sides). Raw snapshot counts
differ (92 full-only vs. 102 incremental) for the same benign,
already-understood reason documented in B2-EVAL-009 (incremental
retains superseded lineage-linked snapshots; the gate's own
`snapshot_keys` filters to current-only before comparing).

## Performance

Same methodology as CALL-RES-A4's own measurement convention (1
discarded warm-up, 10 timed runs, release build, median + min/max).
Commit pair: same as the parity check.

| | full-index (median, min–max) | incremental-index (median, min–max) |
|---|---|---|
| request (whole repo) | 211.48ms (190.68–220.71ms) | 35.11ms (33.14–35.47ms) |

Edge-count stability: identical (42 edges) across all 10 runs, both
full and incremental — timing variance is not masking a correctness
difference. Absolute magnitude is in the same general order as prior
corpora (fmt ~1.47s for ~140 files incl. vendored test code; `request`
is a much smaller corpus at 13 files, consistent with the faster
absolute time here). No A4-specific marginal-cost isolation was
possible on this corpus — CALL-RES-A4's namespace-filtering mechanism
is C#-specific and never activates for JavaScript, same situation as
C++ in B2-EVAL-009.

## New findings

**No new Critical or Important production defect.** Three new
**Moderate** findings, all confirmed honest silent drops (zero false
edges in every case checked), all in the same CommonJS-specific gap
family:

- **B2-ID-013** (Moderate): `require(...)` call sites never produce an
  `ImportFact`. `collect_js_import` only matches ESM `import_statement`
  nodes; CommonJS's own module-reference idiom is invisible to
  CALL-RES-A4's import-alias resolver entirely. Not fixed this round
  (out of scope per directive).
- **B2-ID-014** (Moderate): `exports.X = function(){...}` property
  assignments are never extracted as candidates. Confirmed on 4 real
  sites in this corpus, with 3 concrete real cross-file call sites
  silently dropped as a direct consequence (`cookies.jar`,
  `cookies.parse`, `hawk.header`). Not fixed this round.
- **B2-ID-015** (Moderate): `Request.prototype.X = function(){...}`
  (the ES5 "class via prototype" idiom) is never extracted as a
  candidate, at much larger magnitude than B2-ID-014 — confirmed 27/27
  real methods of the `Request` class itself invisible, meaning this
  corpus's single most important class has zero extracted method
  surface. Not fixed this round.

All three are classified Moderate (not Important/Critical) per the
project's own severity rule: each is an honest silent drop (confirmed
zero wrong edges in all spot checks), not a correctness defect
producing misleading output — the tool's existing failure mode
(declining silently rather than guessing) holds, it is simply
triggered by an upstream extraction gap specific to idioms this
corpus is unusually rich in. Per the Critical/Important-only STOP rule,
work continued through the full checkpoint protocol.

## Independent review

Performed sequentially (not wide parallel fan-out) against the raw
evidence files (`b2-eval-010-request-run.txt`,
`b2-eval-010-request-parity-run.txt`, `b2-eval-010-request-perf-run.txt`,
`b2-eval-010-baseline-freeze.txt`) and the harness source
(`b2_eval_010_js_commonjs.rs`).

**Verdict: APPROVE.**

1. **Corpus/edge/reliability numbers**: verified against the raw run
   output exactly (sections A–D of the evidence file match this
   report's tables).
2. **Zero-false-positive claim**: verified — section H2's
   `false_edge_count = 0` across all 27 `Request.prototype` methods,
   and section F's four cross-file spot checks, both directly support
   the claim with no gap between asserted and measured values.
3. **Parity**: verified genuinely executed via the real
   `compare_full_incremental_state` gate (not a custom comparison),
   same gate the lib suite's own `gate_3_full_equals_incremental_exact_state`
   test uses; raw snapshot-count discrepancy explanation matches
   B2-EVAL-009's own already-verified lineage-retention mechanism.
4. **Performance**: table numbers trace exactly to the raw perf-run
   evidence file; edge-count stability (42/42 across all 10 runs, both
   sides) confirms timing noise is not masking a real divergence.
5. **Severity-protocol compliance**: verified correct — three findings
   opened as Moderate (not Critical/Important), no STOP triggered, no
   fix attempted or applied to production code, consistent with every
   spot check showing zero false edges.
6. **Baseline freeze**: the pre-existing `codegraph-index` lib test
   suite (607 tests, truncated view confirms a large contiguous
   passing run through `incremental::indexer` tests with no failures
   visible) is included as raw evidence, confirming CALL-RES-A4 and
   prior fixes are non-regressed going into this round.

No Critical/Important issue found by the reviewer. No corrections
required to this report's substance.

## GO/NO-GO for next domain

**GO.** No Critical/Important defect found or left open. All three new
findings are Moderate, documented with IDs, and left for the backlog
per explicit instruction. Full/incremental parity holds on real
CommonJS JavaScript data via the existing equivalence gate. Precision
remains 1.000 (zero false positives) despite low recall on this
corpus's two dominant export idioms. B2-EVAL-011 is NOT auto-started
per explicit instruction — this is a recommendation for the user's own
next decision, not an action taken here.

## Recommendation on CALL-RES-A5

**Not justified by this round's evidence — and for a different reason
than in prior rounds.** All three new findings (B2-ID-013/014/015) are
**pure extraction gaps**, not call-resolution-quality gaps:

1. The dominant failure mode this round (`exports.X =
   function(){...}`, `Request.prototype.X = function(){...}`,
   `require()`→`ImportFact`) is about symbols and imports **never
   reaching candidate status at all** — upstream of anything A5
   (type/points-to-aware resolution) operates on. A5 improves
   disambiguation among real candidates; it does nothing for a callee
   that is never even a candidate.
2. **Zero Ambiguous CallAttempts were produced on this corpus** (0/68
   negative-knowledge entries are Ambiguous; all 68 are `Unknown`-reason
   Unresolved) — meaning this domain produced no evidence of the
   overload/multi-candidate-disambiguation problem A5 would address at
   all, unlike C#/C++ (Polly, fmt), which both showed genuine
   receiver-type-dependent ambiguity A5-class resolution could reduce.
3. **The honest-non-answer property holds throughout**: zero false
   positives found anywhere in this round's sampling, consistent with
   every prior round. The tool is not producing wrong answers that A5
   would fix in this domain — it is silently declining to even attempt
   resolution on a specific extraction-upstream gap, which is a
   different, cheaper-to-fix problem (adding extraction coverage for
   two property-assignment idioms and `require()`-based ImportFacts)
   than anything A5 is scoped to solve.

**Conclusion: A5 remains unauthorized and unjustified. If anything,
this round's evidence redirects priority toward closing the
CommonJS extraction gap family (B2-ID-013/014/015) before any future
A5 discussion, since A5 would have zero effect on this domain's actual
residual gaps.**
