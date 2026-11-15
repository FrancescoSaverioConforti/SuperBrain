# B3-TUNE-v2 Priority 3 — Deterministic Reranking (Design)

**Status:** design only. **Not authorized for implementation.** No code
in this document has been written or run.

**Date:** 2026-11-14.

**Depends on:** `docs/research/B3-TUNE-v2-PRIORITY3-RANKING-AUDIT.md`
(ground-truth audit of the current pipeline, read directly from the
code in `crates/index/src/{search,bm25,trigram,indexer_impl,document}.rs`
at commit `d7f182f`). Every signal proposed below is checked against
that audit's Section 6 ("signals confirmed present but not used for
ranking") — nothing here proposes a signal the audit didn't already
confirm exists in `Document`/`SearchResult` today.

**Baseline:** official B1 (`eval/results/B3/frozen-config.json`,
commit `52d42ea`), unmodified. Neither Priority 1 nor Priority 2
produced a KEEP, so B1 remains the only configuration in active use and
the only baseline any Priority 3 ablation is measured against.

## Explicitly NOT authorized by this document

- Learned reranker (any model trained on relevance judgments).
- Embeddings / dense retrieval of any kind.
- Hybrid retrieval (combining lexical with any non-lexical signal
  source).
- Priority 4 work of any kind.
- A new held-out run — `eval/rag/v2-heldout/` stays unread.
- Any code change. This document proposes; nothing here is built until
  separately authorized, task by task, the same way Priority 2's plan
  was.

## What "deterministic and auditable" means here

Every proposal below is a **fixed, hand-specified function** of fields
already on `Document`/`SearchResult` — no learned weights, no
training data, no corpus statistics beyond what BM25 already computes.
"Auditable" means: given a query and a candidate's metadata, a human
can compute the reranking contribution by hand from the rule alone,
without consulting a model or a fitted parameter. Each proposal is
scoped to see if it should move the needle at all (ablation first);
none is proposed as already decided to ship.

## Proposals

Each proposal states: the signal, why it might help, the regression
risk, how Stage-1-style isolated ablation would measure it, and
whether it can only reorder the existing candidate set or could also
change candidate membership.

### P1 — Exact note-id / identifier match boost

**Signal:** if the query (trimmed, case-insensitive) equals the
candidate's `metadata.id` exactly, or equals a `#<IDENTIFIER>`-style
fragment extracted from `source` (the same extraction Priority 2
Lever C's alias table already does by hand, per
`priority2-stage1-summary.md`'s Lever C config section) — add a fixed
bonus to the BM25 score before sorting.

**Why it might help:** the audit (Section 6) confirms there is
currently no identifier-aware signal at all; `MatchKind::ExactSubstring`
is substring-based, not identifier-based, and doesn't affect score or
order today. A query that IS a note's own id or short identifier is
about as strong a relevance signal as exists, and it's currently
worth nothing extra over an ordinary token-overlap hit.

**Risk of regression:** Lever C's own finding (Priority 2, closed
NOT EVALUATED/NO-OP) was that dev/calibration queries essentially never
contain an identifier's literal short form — if that generalizes, this
boost may simply never fire on these splits either, repeating Lever
C's no-op outcome. Needs an `activations_fired`-style counter from the
first run, same discipline Lever C's post-hoc fix added, to avoid
re-making that mistake blind.

**Ablation isolation:** single boolean toggle on `Index::search_impl`'s
scoring step (a new, additive, opt-in parameter paralleling
`with_bm25_params`'s pattern — default off, every existing call site
unaffected). Measure Recall@10/nDCG@10 delta vs. B1 on dev, then
calibration, same frozen-rule discipline as Stage 1.

**Candidate set vs. ordering only:** ordering only. The candidate
already has to be in `candidate_ids`'s union (trigram or BM25 overlap)
before this bonus can apply to it — this cannot pull in a note that
wasn't already a candidate.

### P2 — Status-aware tie-break (not a filter change)

**Signal:** among candidates with equal (or near-equal, within a fixed
epsilon) BM25 score, prefer `NoteStatus::Verified` or `Reviewed` over
`Draft`/`Superseded`/`Contradicted`, as a secondary sort key before the
existing note-id lexicographic tie-break.

**Why it might help:** the audit confirms `status` is already read and
already an optional filter, but never a tie-break signal — two notes
scoring identically today are ordered by id alone, which is arbitrary
with respect to which one a user actually wants. A verified/reviewed
note outranking a draft at equal lexical score is a defensible
default ordering preference.

**Risk of regression:** changes ordering only among already-tied
scores — by construction this cannot change which notes appear in the
top-k, only their relative order among exact or near-ties, so the
blast radius is small. The risk is that dev/calibration may have very
few genuine ties, making this another possible no-op; needs a
tie-count instrumentation field in the ablation report from the start
(same lesson as Lever C).

**Ablation isolation:** new tie-break key inserted between the score
comparison and the id comparison in `search_impl`'s sort (line
398-403 per the audit) — additive, opt-in via a flag, default off.

**Candidate set vs. ordering only:** ordering only, and only among
ties.

### P3 — Recency tie-break (updated-at, not a decay/boost term)

**Signal:** among score-tied candidates (same scope as P2 — these two
could compose, or be tested independently; the plan would specify
which), prefer the candidate with the more recent `updated` timestamp.

**Why it might help:** `updated` is confirmed present on `Document`
and already used as an optional filter (`date_filter`), but never
affects ordering. A fresher note among equally-scored candidates is a
reasonable deterministic default — no decay function, no score
blending, just a secondary sort key, which keeps it auditable (no
tunable half-life or weight to justify).

**Risk of regression:** identical shape of risk to P2 (ties may be
rare on these splits); additionally, `updated` can be `None` for a
candidate with invalid/malformed temporal metadata (audit Section 1,
`document.rs`'s `parse_instant`) — the rule must specify a defined
order for `None` vs. `Some` (e.g. `None` sorts last) so the tie-break
itself stays deterministic and doesn't introduce nondeterminism via
unordered `None`s.

**Ablation isolation:** same mechanism as P2, independently toggleable.

**Candidate set vs. ordering only:** ordering only, and only among
ties.

### P4 — Trust-level tie-break

**Signal:** among score-tied candidates, prefer `TrustLevel::Official`
over `Internal` over `Web`.

**Why it might help:** same shape of argument as P2/P3 — `trust` is
confirmed present and filter-only today (audit Section 6); preferring
higher-trust sources among equally-scored candidates is a defensible,
fully auditable default (a fixed 3-level ordering, no learned weight).

**Risk of regression:** same tie-rarity risk as P2/P3. Also: if P2,
P3, and P4 are all proposed as tie-breaks, composing more than one
requires a fixed precedence order declared in the plan (which tie-break
applies first) — that precedence itself needs to be pinned before any
ablation runs, not chosen after seeing which order produces better
numbers (that would be the same post-hoc-selection problem Priority 2's
Lever B checkpoint was built specifically to avoid).

**Ablation isolation:** same mechanism as P2/P3, independently
toggleable, with an explicit composition-order rule if more than one
tie-break lever is tested together.

**Candidate set vs. ordering only:** ordering only, and only among
ties.

## What this design deliberately does not propose

- **Any change to candidate generation** (Section 1 of the audit).
  Every proposal above assumes the current trigram-union-BM25
  candidate set unchanged — none of P1-P4 can add or remove a
  candidate, only reorder within the set that already exists. A
  proposal that changed candidate membership would need its own,
  separate risk analysis (it could silently change Recall@10 itself,
  not just nDCG@10/ordering), which none of these do.
- **Any change to final BM25 scoring itself** (`k1`/`b`, formula,
  smoothing) — that's Priority 2 Lever A's territory, closed REJECT,
  and the user's standing instruction is no further BM25 k1/b tuning
  in any future round.
- **Any change to `MatchKind`'s role.** The audit (Section 4) found
  `MatchKind` is currently informational only, not used for scoring or
  ordering. Making it a signal (e.g., `ExactSubstring` outranks
  `RankedTerms` at equal score) is a plausible fifth proposal but is
  deliberately left out of this first batch pending the user's choice
  of which proposals to authorize for ablation — it would compose with
  P1 in a way that needs its own precedence discussion if both are
  wanted.

## Recommended evaluation discipline (carried over from Priority 2 Stage 1)

If any of P1-P4 is authorized:
- Same frozen decision rule: ≤0.05 absolute Recall@10/nDCG@10
  regression vs. B1, AND at least one metric improves, AND the
  abstention confusion matrix pattern stays unchanged (TP=0, FP=0).
- Same isolated-ablation discipline: each proposal measured
  independently against B1 first; composition (if any proposal KEEPs)
  is a separate, explicitly authorized Stage 2, not assumed.
- Same instrumentation lesson from Lever C: every proposal's ablation
  report must count how many queries actually exercised the new logic
  (`activations_fired` / `ties_broken`, as applicable) so a genuine
  no-op is never reported as a tested-and-failed REJECT.
- Same real-pipeline discipline from Lever B's verification checkpoint:
  any proposal must be measured through the real `Index::search`
  pipeline directly (an additive, opt-in builder parameter, default
  off, mirroring `with_bm25_params`'s existing pattern) — never through
  a standalone/substitute scorer.
- Held-out (`eval/rag/v2-heldout/`) stays unread throughout.

## Next step

This document proposes four deterministic, auditable reranking
candidates (P1-P4) and explicitly excludes learned/embedding/hybrid
approaches and any candidate-set change. **No implementation starts
without the user selecting which proposal(s), if any, to authorize for
Stage-1-style isolated ablation**, following the same plan-writing →
TDD-execution discipline used for Priority 2.
