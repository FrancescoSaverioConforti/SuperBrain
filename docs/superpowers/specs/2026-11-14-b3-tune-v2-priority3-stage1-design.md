# B3-TUNE-v2 Priority 3 Stage 1 — P1/P2 Frozen Design

**Status:** approved for implementation (user authorization, 2026-11-14,
P1 GO, P2 GO WITH CONSTRAINT; P3/P4 explicitly deferred/no-go this
round). This document freezes every rule BEFORE any code or ablation
runs, per the user's own instruction and the project's standing
discipline (Priority 2's Lever B checkpoint exists specifically because
rules chosen after seeing results are not a valid evaluation).

**Depends on:** `docs/research/B3-TUNE-v2-PRIORITY3-RANKING-AUDIT.md`
(ground-truth pipeline audit) and
`docs/superpowers/specs/2026-11-14-b3-tune-v2-priority3-reranking-design.md`
(the four original proposals, P1-P4).

**Baseline:** official B1 (`eval/results/B3/frozen-config.json`, commit
`52d42ea`), unmodified.

---

## Scope

Exactly two isolated ablations, measured completely separately, never
composed:

- `B1 + P1` vs. official B1.
- `B1 + P2` vs. official B1.

**Explicitly excluded from this round:** `B1 + P1 + P2` composition,
P3 (recency tie-break), P4 (trust tie-break), any Priority 4/hybrid
work, any new held-out run. `eval/rag/v2-heldout/` stays unread.

---

## P1 — Exact identifier boost: frozen semantics

**Match target (user-selected):** the structural identifier extracted
from a note's `source:` frontmatter field — the `#<IDENTIFIER>`
fragment, read up to the first non-identifier character (whitespace,
`(`, etc.), identical extraction to
`crates/api/examples/b3_tune_v2_lever_c_alias_table.rs`'s
`extract_structural_candidates` (verified by reading that function:
`source: docs/technical-debt.md#SEC-002 (quarta revisione)` yields
`SEC-002`, not the parenthetical). **Not** the full note-id (that
target was offered and not selected).

**Matching rule (frozen, strict):**
1. Build a per-candidate identifier set the same way Lever C already
   does: for each result candidate, extract its `source`'s
   `#<IDENTIFIER>` fragment if one exists (none if `source` has no `#`
   fragment, or the note is `Sensitive` — `source` is withheld for
   Sensitive notes per `document.rs`/`search.rs:345-349`, so P1 cannot
   apply to a Sensitive candidate at all).
2. Tokenize the query by `split_whitespace()` (same as
   `alias_expansion_fired`'s own tokenization — no BM25/trigram
   tokenizer substitution).
3. The boost applies to a candidate **only if** one of the query's
   whitespace tokens, lowercased, is **exactly equal** (not
   `contains`, not `starts_with`) to that candidate's extracted
   identifier, also lowercased.
4. **Explicitly forbidden:** any substring, prefix, or fuzzy match
   (the user's own example: `SEC` must never match `SEC-001`). A query
   token `sec-001?` (punctuation-fused) does **not** match `sec-001`
   under this rule — this mirrors Lever C's own documented near-miss
   (q069's `SEC-001/2/3?`), which was correctly a non-match there for
   the same reason.

**Boost mechanism (frozen):** a **fixed additive bonus**,
`P1_IDENTIFIER_BOOST: f64`, added to the candidate's existing BM25
score (from `fresh_scores`, the real per-query scoring index) before
the final sort. Not a multiplier, not a new score replacing BM25 — BM25
score + bonus when the rule fires, BM25 score unchanged when it
doesn't. The exact bonus magnitude is an implementation task decision
within the plan (a value large enough to be decisive when it fires,
e.g. larger than the maximum observed BM25 score in the corpus, so the
boost reliably promotes a match to the top rather than merely nudging
it — the plan's Task 1 pins the exact constant and justifies it against
the actual B1 calibration score distribution, not guessed blind).

**Candidate set:** unchanged. P1 can only affect which already-present
candidate sorts higher — exactly as Section "What this design
deliberately does not propose" in the Priority 3 design doc already
established. A candidate with no identifier boost applied is scored
exactly as B1 scores it today.

**Instrumentation (required, Lever-C lesson):** the ablation report
must count `identifier_boost_fired: usize` (how many queries had at
least one candidate whose boost applied) and `identifier_boost_query_count`
— so a zero-activation result is reported as NOT EVALUATED / NO-OP,
never silently as REJECT, exactly as Priority 2 Lever C's post-hoc fix
established.

---

## P2 — Status tie-break: frozen semantics

**Status ordering (user-confirmed against the real domain model,**
`crates/domain/src/schema.rs:17-24`**, which has 6 variants and no
derived `Ord`):**

```
Tier 1 (highest): Verified, Reviewed
Tier 2:           TestPassed
Tier 3:           Draft
Tier 4 (lowest):  Superseded, Contradicted
```

Within a tier, no ordering is implied between its members (e.g.
Verified and Reviewed are equally ranked by this rule; a further
tie-break, if any is needed there, falls through to the existing
note-id lexicographic tie-break, unchanged).

**Equivalence/tie rule (user-confirmed, frozen):** P2 fires **only**
when two candidates' BM25 scores are **exactly equal** as `f64`
values — `score_a == score_b`, bit-for-bit, no epsilon. This is
deliberately the strictest possible definition: "must not exceed a
real BM25 difference" is satisfied trivially and unambiguously, at the
cost of being a strict floating-point equality (ties from independent
f64 summation order could theoretically fail to compare equal even for
"the same" mathematical value — this is accepted as a feature of the
strict rule, not a bug: a near-tie that fails bitwise equality simply
does not get P2's tie-break, and keeps the current (score, note-id)
ordering unchanged. This cannot cause a regression beyond what B1
already does for that pair).

**Application point (frozen):** inserted into `search_impl`'s existing
sort comparator, between the score comparison and the note-id
comparison:

```
sort key = (BM25 score desc, status tier asc [only if scores equal], note-id asc)
```

Never applied when scores differ by any nonzero amount, however small.

**Candidate set:** unchanged — ordering only, and only among exact-tied
candidates.

**Instrumentation (required, Lever-C lesson):** the ablation report
must count `ties_broken_by_status: usize` and `exact_ties_observed: usize`
(ties found before applying the status tie-break, so "no ties existed"
is distinguishable from "ties existed but status was already
equal/didn't change order").

---

## KEEP/REJECT criteria (frozen, same frozen rule as Stage 1, applied
separately to each of P1 and P2)

Unchanged from Priority 2 Stage 1's own frozen rule
(`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority2-lexical-stage1.md`):
- Recall@10/nDCG@10 regression ≤0.05 absolute vs. official B1, on
  calibration (the decision split).
- At least one of Recall@10/nDCG@10 must improve.
- Abstention confusion matrix pattern must stay unchanged (TP=0, FP=0,
  matching B1's own constant non-abstaining behavior on this corpus).
- Dev is diagnostic only, never decisive.
- Each of P1, P2 is judged **independently** against this rule. A
  result with zero activations (per the instrumentation above) is
  reported as **NOT EVALUATED / NO-OP**, not KEEP or REJECT, exactly
  as Lever C's corrected verdict label.

**Additional reporting requirements (user-specified this round, both
ablations):**
- By-category/by-difficulty breakdown (not just the aggregate
  Recall@10/nDCG@10 — whatever category/difficulty fields the existing
  `LabelRow` schema in `eval/rag/dev`/`eval/rag/calibration` already
  carries; the plan's Task 1 confirms the exact field names before
  writing the breakdown code, reading the real label schema rather
  than assuming its shape).
- Exact config/commit/hashes per the same `run_commit`/`labels_sha256`
  discipline Stage 1 used.
- Separate KEEP/REJECT/NOT-EVALUATED verdict per lever — never a
  combined verdict.

---

## Execution plan

Native (inline) execution, per the user's explicit instruction ("esegui
Native con review indipendente finale"). A `writing-plans`-style
implementation plan (`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority3-stage1-p1-p2.md`)
will be written next, breaking P1 and P2 into TDD tasks each ending in
an isolated ablation run against dev (diagnostic) then calibration
(decision), followed by one independent whole-branch review before
reporting results. STOP after P1/P2's isolated results — no
composition, no P3/P4, no Priority 4/hybrid, no MCP, no merge/tag/push.
