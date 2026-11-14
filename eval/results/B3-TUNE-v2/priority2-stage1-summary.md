# B3-TUNE-v2 Priority 2 Stage 1 — KEEP/REJECT Summary

**STATUS: PRIORITY 2 CLOSED (2026-11-14).** Final verdicts: Lever A
REJECT (confirmed, closed), Lever B REJECT (after real-pipeline
verification), Lever C NOT EVALUATED / NO-OP. **No lever is promoted.
Priority 2 Stage 2 (composition) does NOT run** — composition requires
at least one Stage 1 KEEP to compose, and none exists. The active
baseline for any future priority remains the official B1 configuration
(`eval/results/B3/frozen-config.json`), unchanged, since neither
Priority 1 nor Priority 2 produced a KEEP configuration.

**Phase:** B3-TUNE-v2, Priority 2 (lexical tuning), Stage 1 (isolated
ablations).
**Date:** 2026-11-14.
**Scope:** Three independent levers (BM25 `k1`/`b`, punctuation
normalization, deterministic alias table), each measured against the
**official** B1 baseline (`eval/results/B3/frozen-config.json`, commit
`52d42ea`) — never against Priority 1 (closed REJECT,
`docs/research/B3-TUNE-v2-PRIORITY1-CLOSURE.md`) and never against each
other. No Stage 2 (composition) in this document.

All KEEP/REJECT decisions below apply the Frozen decision rules fixed
in the plan (`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority2-lexical-stage1.md`)
**before** any grid or ablation run: Recall@10/nDCG@10 regression ≤0.05
absolute vs. the official B1 baseline, at least one of the two metrics
must improve, and the abstention confusion matrix pattern must stay
unchanged (TP=0, FP=0, matching B1's own constant non-abstaining
behavior) — any lever causing even one `NotPresent` where B1 didn't, or
vice versa, is an automatic REJECT regardless of the other metrics.

---

## Lever A — BM25 `k1`/`b` — **REJECT (CONFIRMED, CLOSED)**

**User review (2026-11-14): REJECT CONFIRMED. Closed. No further BM25
k1/b sweep or tuning in any future round of this lever.**

**Config:** `k1=1.8, b=0.75` (selected from a 16-point grid over dev via
the frozen rule: max Recall@10 → tie-break max nDCG@10 → tie-break min
distance to defaults `(1.2, 0.75)`; `(1.8, 0.75)` won the nDCG@10
tie-break uniquely, no further tie-break needed — see
`priority2-lever-a-selection.md`).

**Variants measured:** `A-score` (custom params reach only final
scoring) and `A-full` (custom params also reach the candidate
prefilter). **Correction (post-review):** these two variants are
identical by construction on this corpus, not merely "no drift observed
at this grid point." The candidate prefilter's own `Bm25Index` is only
used for candidate *membership* via `query_ranked(query, usize::MAX)`
filtered to score > 0; since IDF is `ln(... + 1) > 0` and the BM25 term
score is strictly positive for any `k1 ≥ 0`, `b ∈ [0, 1]` whenever a
query term overlaps a document at all, the candidate SET can never
change across this grid — only each candidate's downstream re-ranking
score can, and that re-ranking always happens through the `Index`-level
`fresh_scores` (`A-score`'s own mechanism) regardless of which variant
is run. The grid's invariant Recall@10 (0.836735 at all 16 points,
`priority2-lever-a-grid.json`) is the direct evidence of this: a
changing candidate set would show up as a changing Recall@10, and it
never does. The `A-score`/`A-full` plumbing itself (Task 1b) is correct
and real — it is exercised and does take effect — but the two variants
cannot diverge in their RESULT on this corpus under additive,
non-negative BM25 scoring; a construction that could make them diverge
(e.g. a smoothing floor letting a zero-overlap document enter the
candidate set) does not exist in this codebase's BM25 implementation.

**Commit provenance (correction post-review):** each ablation JSON's
`run_commit` field reflects `git rev-parse HEAD` at the moment the
binary RAN, which in several cases was a commit prior to the one that
added the code producing that exact report (the binary was built and
run, then its sources were committed afterward in the same task) — the
`run_commit` field in these reports should be read as "the code's last
commit was no earlier than this," not as the exact commit. The grid
ran under code committed at `5d4196a`; the dev/calibration ablation at
the selected point ran under code committed at `efd4ccf`. Corpus
manifest hash is not recorded in these reports (only `labels_sha256`
is) — a gap versus the design doc's logging rule, noted but not
re-run to fix retroactively, since it does not change any verdict.

| | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 |
|---|---|---|---|---|
| Dev (`A-full`) | 0.8367 | — (diagnostic only) | 0.7170 | — |
| Calibration (`A-full`) | 0.9167 | **+0.0000** | 0.6966 | **−0.0125** |

**Abstention (calibration, `A-full`):** TP=0, TN=24, FP=0, FN=6 —
unchanged from B1's own pattern.

**Latency/resource:** not measured.

**KEEP/REJECT rationale:** Recall@10 regression (0.0000) and nDCG@10
regression (0.0125) both pass the ≤0.05 tolerance. Abstention pattern
unchanged. **But neither metric improves** — Recall@10 is exactly flat,
nDCG@10 regresses (does not improve). Per the frozen rule, "improves
nothing but doesn't regress much either is not a reason to change a
frozen baseline."

**Verdict: REJECT (CONFIRMED, CLOSED).**

---

## Lever B — Conservative Punctuation Normalization — **REJECT (real-pipeline verification result)**

**User review (2026-11-14): verdict downgraded to PROVISIONAL KEEP**
pending a real-pipeline verification, since the original measurement
below used a standalone scorer, not the real `Index::search` pipeline.
**That verification has now run (see "Real-pipeline verification"
section below) and resolves the lever to REJECT** — the real pipeline
does not reproduce the standalone scorer's promising signal; it shows
a calibration nDCG@10 regression instead.

**Config:** `strip_trailing_sentence_punctuation` — strips only trailing
`. , ; : ! ? " '` (narrowed from the design doc's original candidate
set, which also included `) ] }`, after Task 4's corpus+dev diagnostic
found 50+ real collisions where those characters closed a function
call, type annotation, or complexity notation rather than a sentence —
see `priority2-lever-b-token-diagnostic.json` and commit `ee5c6a4`).

**Variants measured:** `B-query` (normalization applied to the query
only, against the real `Index::search` pipeline unmodified) and
`B-symmetric` (normalization applied to both indexed text and the
query, via a **standalone** `Bm25Index`/`TrigramIndex` pair that
bypasses `RealIndexer`/`Index::search` entirely, since the frozen
production tokenizer is never modified).

**Correction (post-review): `B-symmetric`'s numbers are NOT directly
"B1 + normalization" — they come from a substitute scorer that differs
from the real pipeline in three ways, confirmed in review:**
1. IDF and `avgdl` are computed over the WHOLE 53-note corpus, not over
   the per-query eligible-candidate subset the real `Index::search`
   uses for its own `fresh_scores` (`search.rs`'s per-query re-scoring
   index is scoped to candidates only, after project/type/status/trust/
   time filtering — the standalone scorer has none of that scoping).
2. The standalone scorer's `TrigramIndex` candidates are computed but
   never actually unioned into the returned ranking — only BM25's own
   `query_ranked` output is returned, so the trigram side is a no-op
   past an emptiness check.
3. `NotPresent` semantics differ slightly as a result, so the
   abstention tripwire under `B-symmetric` is a weaker check than under
   the real-pipeline-based Levers A and C.

A post-review control run (same normalization rule, but through the
standalone scorer WITHOUT normalization, as a matched baseline) found
the standalone scorer alone already differs from real B1 by about
−0.015 nDCG@10 on calibration (0.6937 vs. B1's real 0.7091) — the
substitution is a bias AGAINST this lever, not in its favor. Measured
against its own matched control rather than against real B1, Lever B's
actual normalization effect is **+0.019 nDCG@10** on calibration, larger
than the +0.0036 reported below against real B1. The KEEP verdict is
robust to this — if anything the true effect is understated — but the
exact calibration numbers in the table below describe "standalone
scorer + normalization" vs. "real B1", not "real B1 + normalization vs.
real B1". **Confirming this lever's effect inside the real `Index::search`
pipeline (which requires actually modifying tokenization symmetrically,
not a standalone scorer) remains open and should happen before this
lever is composed with anything else in a future Stage 2.**

Also note: the SAME control run found `B-query` (query-side-only
normalization, run through the REAL pipeline, no substitute scorer)
*regresses* calibration nDCG@10 to 0.6877 (−0.0214 vs. real B1) — one-
sided normalization hurts in the real pipeline. This is additional
evidence that the symmetric effect genuinely depends on the indexed
side changing too, not an artifact of the substitute scorer.

**Commit:** `ee5c6a4` (rule + diagnostic), `c191b21` (ablation).

| | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 |
|---|---|---|---|---|
| Dev (`B-query`, real pipeline) | 0.8776 | — (diagnostic only) | 0.7503 | — |
| Dev (`B-symmetric`, standalone scorer) | 0.9592 | — (diagnostic only) | 0.7969 | — |
| Calibration (`B-query`, real pipeline) | 0.9167 | +0.0000 | 0.6877 | **−0.0214** |
| Calibration (`B-symmetric`, standalone scorer) | 0.9167 | **+0.0000** | 0.7126 | **+0.0036** |

**Abstention (calibration, `B-symmetric`):** TP=0, TN=24, FP=0, FN=6 —
unchanged from B1's own pattern. (Per the correction above, this check
is weaker under the standalone scorer than under the real pipeline.)

**Latency/resource:** not measured.

**KEEP/REJECT rationale:** Recall@10 regression (0.0000, a flat tie —
not a regression) passes. nDCG@10 **improves** by +0.0036 — a small but
genuine improvement over real B1, not a regression. At least one metric
improves (nDCG@10). Abstention pattern unchanged (weak-check caveat
above). **This margin is noise-level at n=24** — the bootstrap CI on
calibration nDCG@10 is wide relative to the +0.0036 delta; the frozen
rule's "at least one metric improves, no regression beyond tolerance"
criterion is satisfied, but this is a KEEP for a real, if small and
imprecisely measured, effect, not a decisive win.

**Verdict (standalone scorer only): PROVISIONAL KEEP** — **superseded by
the real-pipeline verification below, which resolves this lever to
REJECT.** This standalone-scorer measurement is kept in this document
for the record (it is what first surfaced the lever as promising and
justified running the verification at all), but it is not what decides
Lever B's final status.

(Note: dev's own `B-symmetric` improvement — 0.9592 vs. B1's 0.8367 —
is much larger than calibration's, and is itself measured through the
same standalone-scorer substitute, so it overstates the real-pipeline
effect for the same reason above. Dev is diagnostic only per the frozen
discipline; the calibration number is what decided this lever.)

---

## Lever C — Deterministic Alias Table — **NOT EVALUATED / NO-OP**

**User review (2026-11-14): verdict corrected from `REJECT (no-op)` to
`NOT EVALUATED / NO-OP`.** Rationale: `expansions_fired = 0` on both
splits means the alias table's actual effectiveness was never measured
at all — "REJECT" implies a tested hypothesis that failed, which
overstates what happened. **This lever is closed here as inconclusive.
No new aliases will be constructed to force activations on dev or
calibration** — that would mean selecting the table's content by
looking at the evaluation data it's supposed to be judged against,
which is exactly the calibration-leakage risk the design doc's
correction #1 exists to prevent, now generalized to dev as well once
the table is being iterated specifically to produce hits.

**Config:** 7-entry frozen table (`SEC-001`, `SEC-002`, `SEC-003`, `C1`,
`C5`, `C6`, `Fase-B`), each entry a hand-reviewed expansion phrase read
from its note's real body, built from corpus structure
(`source:` frontmatter `#<IDENTIFIER>` fragments) cross-referenced
against dev's `relevant_note_ids` only — calibration was never read to
identify, choose, or justify any entry (design doc correction #1). Query-time
expansion, whole-token matching only, appends rather than replaces.

**Commit:** `ced0540` (diagnostic), `ec96891` (frozen table),
`676954c` (ablation).

| | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 |
|---|---|---|---|---|
| Dev | 0.8367 | — (diagnostic only) | 0.7139 | — |
| Calibration | 0.9167 | **+0.0000** | 0.7091 | **+0.0000** |

**Abstention (calibration):** TP=0, TN=24, FP=0, FN=6 — unchanged.

**Latency/resource:** not measured.

**Correction (post-review): this REJECT is a no-op, not a tested-and-
tied result.** Independent verification (whole-token match of all 7
frozen short forms, case-insensitive, against every dev and calibration
question): **zero of 59 dev queries and zero of 30 calibration queries
contain any of the 7 short forms as a literal whitespace token.** The
alias table's `apply_alias_expansion` never fired on a single query in
either split — the identical-to-B1 numbers above are not evidence the
table "didn't help because the target notes were already found," they
are evidence the table was **never exercised at all**. Root cause: Task
6's diagnostic cross-referenced whether a dev query TARGETS a
candidate's note (via `relevant_note_ids`), never whether a dev query
actually USES the short form's exact literal text — a query can
legitimately target `bug-2026-10-27-sec001-toctou` while phrasing the
question entirely in plain English, with no occurrence of the string
"SEC-001" at all (confirmed: that is exactly what happened for all 7
entries). Two near-misses were found during verification, both
correctly non-matches: q069 contains `SEC-001/2/3?` as one token
(punctuation-fused, doesn't equal any single short form), and q079
contains a commit-hash substring `...f50c634` (substring `c6` inside
it, correctly rejected by whole-token matching — confirming
`apply_alias_expansion`'s substring-safety is sound, Review Focus #4,
even though it had no opportunity to fire here).

**KEEP/REJECT rationale:** Both regression tolerances pass trivially
(exact zero delta, both metrics, both splits, because the harness ran
and scored every query with the expansion function present and it
never activated). Abstention pattern unchanged. Neither metric improves
— but not because the alias table was tried and found useless; because
it was never triggered. **Verdict stands as REJECT on the frozen rule's
own terms** (no improvement observed), but the correct characterization
is "REJECT (no-op: zero activations on dev and calibration)," not "the
table didn't help." A future round of this lever, if ever reattempted,
would need its candidate-identification step to check literal short-form
occurrence in dev's actual question text (not just target-note
reference), and to count `expansions_fired` explicitly in the ablation
report so a future "REJECT" can be told apart from a future "no-op"
without a manual audit.

**Verdict: NOT EVALUATED / NO-OP (zero activations on dev and
calibration; closed as inconclusive, not reattempted).**

---

## Secondary diagnostic: Priority 1 signal separation (informational only, non-gating)

Priority 1 (closed REJECT) measured the `ratio` (top1/top2 score)
signal's separation between abstain-expected and answerable queries on
calibration, finding `separation_gap = -0.445` under the plain B1
baseline — heavy overlap, no cutoff of this signal cleared the required
abstention guardrails.

Lever B (KEEP) changes the score distribution itself (both Recall@10
and nDCG@10 shift), which could plausibly sharpen or blur `ratio`'s
separation as a side effect. This was not re-measured in this task
(optional per the plan, not re-run to keep Stage 1's scope to its own
three ablations) — flagging it here as a candidate follow-up measurement
for a future, separately-authorized round, not as a reopening of
Priority 1's closed threshold search. No KEEP/REJECT decision in this
document depends on this diagnostic.

---

## Real-pipeline verification of Lever B (user-authorized checkpoint, 2026-11-14) — **REJECT**

**Mechanism:** `RealIndexer::new_with_text_normalizer(fn(&str) -> String)`
(new, additive constructor, commit `daa622e`) applies a transform to
`Document::text_searchable` right before it reaches
`trigram.insert`/`bm25.insert` inside `index_document` — before
`bm25::tokenize` ever sees the text, never inside it. `None` for every
existing constructor (`new()`, `new_with_bm25_params`), so default/
production indexing is completely unaffected. The normalizer passed is
the literal `normalize_text` function already used by the query side —
**the identical function both sides, not a reimplementation** — which
in turn calls `strip_trailing_sentence_punctuation`, Task 4's
already-frozen rule. **No new punctuation set, no new rule, no new
tuning, no variant selection after seeing results.**

**Config confirmation:** `normalization_policy_identical_to_frozen: true`
in both reports below (self-documenting field) — the exact same
`TRAILING_SENTENCE_PUNCTUATION = ['.', ',', ';', ':', '!', '?', '"', '\'']`
set frozen in `crates/index/src/lexical_normalize.rs` at commit `ee5c6a4`,
unchanged.

**Commit:** `daa622e` (hook), ablation run at `daa622e`.

| | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 | Queries w/ changed ranking |
|---|---|---|---|---|---|
| Dev (B1 this run) | 0.8367 | — | 0.7139 | — | — |
| Dev (`B-symmetric`, real pipeline) | 0.8776 | — (diagnostic only) | 0.7503 | — | 33/60 |
| Calibration (B1 this run) | 0.9167 | — | 0.7091 | — | — |
| Calibration (`B-symmetric`, real pipeline) | 0.9167 | **+0.0000** | 0.6877 | **−0.0213** | 14/30 |

**Abstention (calibration, real pipeline):** TP=0, TN=24, FP=0, FN=6 —
unchanged from B1's own pattern. (This check is now the strong,
real-pipeline version, not the standalone scorer's weaker one.)

**Candidate-set/ranking change:** 14 of calibration's 30 non-canary
queries (and 33 of dev's 60) retrieve a DIFFERENT top-10 ranking under
real-pipeline `B-symmetric` versus plain B1 on the same run — the
normalization genuinely changes retrieval behavior through the real
pipeline, it is not a no-op. The change is simply not a beneficial one
on calibration.

**KEEP/REJECT rationale (same Frozen decision rules criterion, applied
mechanically, not re-chosen after this result):** Recall@10 regression
(0.0000) passes the ≤0.05 tolerance trivially. nDCG@10 regression
(0.0213) also passes the ≤0.05 tolerance — but **it is a regression,
not an improvement**. Abstention pattern unchanged. **Neither metric
improves.** Per the same rule that rejected Levers A and (now) B's own
original standalone-scorer-vs-control comparison was promising: "the
real-pipeline verification does not confirm the standalone scorer's
signal — the real effect on calibration is a small regression, not an
improvement."

**This result also explains dev's real-pipeline number matching
`B-query`'s (not the standalone `B-symmetric`'s) almost exactly**
(0.8776/0.7503 here vs. 0.8776/0.7503 in the original `B-query` row
above): on this corpus, through the real pipeline, symmetric
normalization's effect is dominated by the query-side change alone —
indexing-side normalization changes individual rankings (33/60 on dev,
14/30 on calibration) without moving the aggregate beyond what
query-only normalization already produced. The standalone scorer's
much larger dev improvement (0.9592/0.7969) was an artifact of its own
different IDF/avgdl scope, not a real-pipeline effect that survived
verification.

**Final verdict for Lever B: REJECT.**

## Overall Stage 1 result (FINAL)

| Lever | Decision variant | Verdict |
|---|---|---|
| A (BM25 k1/b) | `A-full` (identical to `A-score` by construction) | **REJECT (CONFIRMED, CLOSED)** |
| B (punctuation normalization) | `B-symmetric`, real-pipeline verified | **REJECT** (calibration nDCG@10 regression −0.0213; real pipeline does not reproduce the standalone scorer's promising signal) |
| C (alias table) | — | **NOT EVALUATED / NO-OP (closed, inconclusive)** |

**No lever is accepted from this Stage 1 round.** All three close
without a KEEP. Lever B came closest — its standalone-scorer measurement
and matched control both suggested a real improvement — but the
real-pipeline verification (the one measurement method that actually
represents what shipping this change would do) found a small
calibration regression instead. This is exactly why the verification
checkpoint was required before treating the standalone-scorer number as
decisive.

**History of this document's revisions, for the record:** an
independent whole-branch review (fresh context, 2026-11-14) found the
original draft's verdicts were defensible on the frozen rule's own
terms but mischaracterized what was actually measured in three places.
The user's own review (2026-11-14, second pass) adjusted the verdict
labels: Lever A's two variants are mathematically identical on this
corpus (not merely "no drift observed") — **REJECT confirmed by the
user, closed, no further BM25 k1/b tuning**. Lever C never actually
fired a single alias on either split — relabeled **NOT EVALUATED /
NO-OP** rather than REJECT, since REJECT implies a tested-and-failed
hypothesis; **closed as inconclusive, not reattempted** (constructing
new aliases now, with dev/calibration results in hand, would leak the
evaluation data into the table's own construction). Lever B's
standalone-scorer numbers were downgraded to **PROVISIONAL KEEP
pending a real-pipeline verification**, since a matched control
suggested the real effect might be larger than reported (+0.019 vs.
+0.0036). **That verification has now run** (see the dedicated section
above) **and resolves Lever B to REJECT**: the real pipeline shows a
small calibration nDCG@10 regression (−0.0213), not the improvement the
standalone scorer suggested.

## Explicit confirmations

- **The new v2 held-out final-test set (`eval/rag/v2-heldout/`) has not
  been read by this session or any prior Priority 2 session.** Its
  content remains completely unread; nothing in this plan opens
  `eval/rag/v2-heldout/labels.jsonl`.
- **No composite configuration (Stage 2) has been built or measured.**
  Composing Lever B's accepted change with anything else requires the
  user's separate, explicit authorization after reviewing this summary.
- **No Priority 3 (reranking) or Priority 4 (hybrid retrieval) work has
  started.**
- No modification to the B3 frozen config, runner, corpus, or labels.
- No MCP, no merge, no tag, no push.

## Priority 2 closure (2026-11-14)

**Priority 2 is CLOSED.** All three Stage 1 levers have final,
user-confirmed verdicts (table above) and none is a KEEP. Per the
project's own frozen rule, Stage 2 composition requires composing two
or more Stage 1 KEEPs — with zero KEEPs, **there is nothing to compose,
so Stage 2 does not run.** This is a closure, not a deferral: no further
BM25 k1/b tuning (Lever A, user-confirmed closed), no new alias
construction against dev/calibration (Lever C, would leak evaluation
data into the table), and Lever B's real-pipeline regression stands as
the decisive measurement unless a future, separately-authorized round
chooses a different normalization policy from scratch.

**Baseline going forward:** official B1 (`eval/results/B3/frozen-config.json`,
commit `52d42ea`), unmodified. Priority 1 (closed REJECT) and Priority 2
(closed, this document) leave B1 as the only configuration in active
use.

**Before any Priority 3 implementation:** a full audit of the current
ranking pipeline is required (candidate generation, BM25 final scoring,
trigram/substring role, `MatchKind`, tie-breaking, and use of any other
signal — trust level, note type, status, recency, project/source
metadata, exact identifier match) — see
`docs/research/B3-TUNE-v2-PRIORITY3-RANKING-AUDIT.md` once produced.
Priority 3 design proposes deterministic, auditable reranking only.
Learned rerankers, embeddings, hybrid retrieval, and Priority 4 remain
unauthorized.
