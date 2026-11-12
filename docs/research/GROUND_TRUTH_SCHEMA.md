# Machine-readable ground-truth schema (precision/recall harness)

Quality & Semantic Reliability Hardening, Phase 1, deliverable 9.
**Status: IMPLEMENTED (Phase 2).** This schema is now real, tested
code in `crates/codegraph-index/src/ground_truth.rs`: a dependency-free
JSONL parser (`parse_jsonl`), the `GroundTruthRow`/`Status`/
`EvidenceTierField` types, the status-derivation table as
`GroundTruthRow::compute_status()`, and `compute_metrics` (gated by
`MIN_ROWS_FOR_COMPUTED_METRICS` so a too-small sample refuses to report
a misleadingly precise percentage). A real fixture,
`crates/codegraph-index/tests/ground_truth_fixtures/b2-eval-008-extends-implements.jsonl`,
retrofits 4 rows directly from B2-EVAL-008's own already-confirmed
Section C findings, exercised by
`crates/codegraph-index/tests/ground_truth_schema.rs`. Every real
evaluation before this phase (B2-EVAL-001 through 008) still recorded
its Calls-quality samples as prose inside the evaluation markdown
document — this schema is what a FUTURE evaluation can now emit
instead (or alongside), making recomputation of precision/recall
mechanical rather than requiring a human to re-read a prose table every
time. Golden-corpus/ground-truth integration (asserting an expected
resolution tier per fixture, not just expected edges) remains separate,
pending Phase 2 deliverable 9.

## Format

JSON Lines (one observation per line) — chosen over a single large
JSON array so a future harness can APPEND observations incrementally
during a long corpus walk without holding the whole result set in
memory or risking a malformed partial file on interruption, and so
`grep`/`jq`-style line-oriented tools work directly on the raw file.

## Schema

```jsonc
{
  // Which real evaluation produced this observation. Must match an
  // existing docs/research/evaluations/B2-EVAL-NNN.md.
  "evaluation_id": "B2-EVAL-008",

  // Which resolver CONFIGURATION produced the prediction being
  // recorded -- this project's own existing naming convention
  // (CALL-RES-A0 through A8, or "shipped" for whatever the current
  // production default is). Lets the SAME real callsite be scored
  // against multiple configurations over time, exactly how
  // call-resolution-challenges.md's own table-per-case already does
  // in prose -- this schema generalizes that pattern to a flat,
  // queryable row format.
  "configuration_id": "CALL-RES-A1+A2+A3",

  // The real, cloneable repository and exact commit this observation
  // was measured against -- never a repo/commit this schema's own
  // consumer cannot independently re-fetch and re-verify.
  "repo": "App-vNext/Polly",
  "commit": "1f232caf9672e515751ba2df76d82bfbbb62e016",

  // The real source language of the callsite (one of this project's
  // own supported Language variants).
  "language": "CSharp",

  // The real symbol (by human-readable "file::name" form, matching
  // this project's own existing convention in every evaluation doc
  // and manual sample table) that CONTAINS the call site -- the
  // Edge's own `from` endpoint, in readable form.
  "caller": "src/Polly/Bulkhead/BulkheadPolicy.cs::Dispose",

  // The exact byte span of the call site itself (not the caller's own
  // whole span) -- using this project's own existing ByteSpan
  // convention (zero-based, end-exclusive), so a consumer can locate
  // the EXACT text under test, not just the file/caller it lives in.
  "callsite_span": { "start": 1842, "end": 1856 },

  // The real, ground-truth, MANUALLY-CONFIRMED correct target(s), by
  // the SAME "file::name" convention as `caller`. A Vec, not a single
  // string, because a genuinely ambiguous real-world case (CRC-001-
  // shaped) can have more than one EQUALLY VALID true target --
  // recording only one in that case would make the ground truth
  // itself dishonestly over-precise. Empty means "confirmed, by
  // direct source reading, that no real target exists" (e.g. a
  // dynamically-dispatched value, per CRC-002's own real shape --
  // this is DIFFERENT from "we didn't check," which must never be
  // recorded as an empty array; see `notes` below for that case).
  "expected_targets": ["src/Polly/Bulkhead/BulkheadPolicy.cs::Dispose"],

  // What THIS resolver configuration actually predicted, by the same
  // convention. Empty means "produced no edge" (this project's own
  // "no edge beats a false edge" outcome) -- distinct from
  // `expected_targets` being empty, which means the GROUND TRUTH
  // itself has no real target.
  "predicted_targets": ["src/Polly/Bulkhead/BulkheadPolicy.cs::Dispose"],

  // Which EvidenceTier (per SEMANTIC_UNCERTAINTY_MODEL.md's own
  // proposed model) the prediction carried, if any. One of:
  // "Resolved" | "Heuristic" | "Ambiguous" | "Unresolved" | null (no
  // prediction was attempted at all for this exact callsite, which
  // should be rare -- almost every real callsite reaches SOME
  // resolver outcome, even if that outcome is "Unresolved").
  "resolution_method": "Resolved",

  // The classification this ONE observation resolves to, given
  // expected_targets vs. predicted_targets vs. resolution_method.
  // One of: "TP" | "FP" | "FN" | "ambiguous_correct" | "ambiguous_incorrect" | "unresolved".
  // See "Status definitions" below for the exact rule computing this
  // -- a harness MUST compute status FROM the other fields, never
  // record it independently (which would let the two silently drift).
  "status": "TP",

  // Free-text, bounded (recommend capping at 1024 bytes, generous
  // compared to this project's own Evidence.note 512-byte cap, since
  // ground-truth notes often need to explain WHY a case was manually
  // confirmed one way) context a human reviewer needs to independently
  // re-verify this row without re-deriving everything from scratch --
  // e.g. "confirmed via direct source reading of commit 1f232caf,
  // Dispose() calls the semaphore cleanup inline, self-referential-
  // looking caller list artifact of a same-named Dispose elsewhere in
  // the pool, see B2-EVAL-008 Q7."
  "notes": "ground truth confirmed by direct source reading; this specific case's own caller-list result was flagged in B2-EVAL-008 as needing root-cause investigation for a possible same-name ambiguity artifact"
}
```

## Status definitions

Computed deterministically from `expected_targets`,
`predicted_targets`, and `resolution_method` — never recorded as a
free, independently-settable field:

| `expected_targets` | `predicted_targets` | `resolution_method` | `status` |
|---|---|---|---|
| non-empty, 1 real target | same target | `Resolved`/`Heuristic` | `TP` |
| non-empty, 1 real target | different target | `Resolved`/`Heuristic` | `FP` |
| non-empty, 1 real target | empty | `Unresolved` | `FN` |
| non-empty, 1 real target | empty | `Ambiguous` | `FN` (the real target existed among the candidates the resolver saw, but the resolver correctly declined to guess — still a missed TRUE edge from the consumer's own point of view, even though it was the CORRECT behavior given the ambiguity) |
| non-empty, 2+ equally valid targets (a CRC-001-shaped case) | the resolver produced NO edge | `Ambiguous` | `ambiguous_correct` (the resolver's own "no edge beats a false edge" behavior is the RIGHT answer when the ground truth itself is genuinely multi-valued — this is NOT scored as `FN`, since there was no single correct target to miss) |
| non-empty, 2+ equally valid targets | the resolver produced exactly one of them, confidently | `Resolved`/`Heuristic` | `ambiguous_incorrect` (the resolver picked ONE of several equally-valid real targets and reported it with unwarranted confidence — this is the CRC-001 pre-A1/A2/A3 configuration's own real historical shape, and must be scored distinctly from a clean `TP`, since presenting one of several equally-valid answers as certain is its own failure mode even though the picked target is technically "a" real one) |
| empty (confirmed no real target exists) | empty | `Unresolved`/`Ambiguous`/null | `unresolved` (trivially correct — nothing to find, nothing was found; recorded for completeness, never counted toward precision/recall's own numerator or denominator, see below) |
| empty (confirmed no real target exists) | non-empty | `Resolved`/`Heuristic` | `FP` (the CRC-002 shape exactly — a confident edge to something that is NOT the real target, because no real target exists at all, e.g. a runtime-bound value) |

## When metrics must NOT be computed

Per the explicit instruction this whole evaluation series has
followed ("Non calcolare metriche quando il ground truth non è
sufficiente"):

- **Never compute precision/recall/F1 from a sample where
  `expected_targets` was not independently, manually confirmed by
  direct source reading for every single row.** A sample built by
  trusting the resolver's OWN output as its own ground truth is
  circular and must never be used to score that same resolver.
- **Never compute aggregate metrics across rows from DIFFERENT
  `configuration_id`s as if they were comparable samples**, unless the
  underlying `evaluation_id`/`repo`/`commit`/callsite set is IDENTICAL
  across them (i.e. the same real callsites were re-scored under a
  different configuration, exactly how `call-resolution-challenges.md`'s
  own per-case tables already do this correctly in prose).
- **Never include `unresolved`-status rows (ground truth confirmed
  empty) in a precision/recall denominator at all** — they are neither
  a true positive opportunity nor a real miss; they exist in this
  schema purely so a reviewer can distinguish "we checked and there's
  nothing here" from "we never checked this callsite," which an
  ABSENT row cannot distinguish.
- **A sample smaller than roughly 20-30 confirmed rows per
  configuration should report raw TP/FP/FN/ambiguous counts, not a
  computed percentage** — this project's own prior evaluations have
  consistently used sample sizes in this range (e.g. B2-EVAL-004's own
  "30-edge (25-edge, full-corpus) manual call-quality sample") and
  reported raw counts, not a percentage presented with false
  precision; this schema does not change that discipline, it only
  makes the underlying rows machine-readable.

## Derived metrics (how to recompute, once ground truth is sufficient)

Given a set of rows sharing one `evaluation_id` + `configuration_id`
(or a deliberate, documented cross-configuration comparison per the
rule above):

- `precision = TP / (TP + FP)`
- `recall = TP / (TP + FN)`
- `F1 = 2 * precision * recall / (precision + recall)`
- `ambiguity_rate = (ambiguous_correct + ambiguous_incorrect) / total_scored_rows`
- `unresolved_rate = count(status == "unresolved" among rows where expected_targets was non-empty, i.e. real FN-shaped unresolved, NOT the trivial empty/empty case) / total_scored_rows`

`ambiguous_incorrect` rows should be reported SEPARATELY from both
`FP` and `TP` counts in any summary, per this schema's own status
table above — folding them into either would misrepresent either the
resolver's own real false-positive rate or its real true-positive
rate.

## Residual limitations

- This schema is designed but not yet implemented as an actual
  emitting harness — no evaluation in this series has produced a real
  `.jsonl` file in this format yet. A future evaluation adopting it
  would need to write the emission code (straightforward, but not yet
  done) and retrofit at least one PAST evaluation's own existing prose
  sample into this format to prove the schema is actually sufficient
  in practice, not just on paper.
- The `ambiguous_correct`/`ambiguous_incorrect` distinction is new —
  introduced by this document, not something any PAST evaluation's own
  prose reporting already used as a named category (though CRC-001's
  own two-configuration table already implicitly demonstrates exactly
  this distinction without naming it). A future harness adopting this
  schema should treat this distinction as the schema's own
  contribution, not an existing, already-validated convention.
