# Persistent call-resolution challenge set

Introduced by [B2-EVAL-002](../evaluations/B2-EVAL-002.md), per the
Evidence-before-claims Experimental Policy
(`docs/research/EXPERIMENTAL_POLICY.md`). This is a growing set of
REAL, concretely-reproducible call-resolution cases where at least one
evaluated configuration produced a false positive, a false negative, or
an unresolved/ambiguous result — collected during real evaluations, not
invented. Every future call-resolution configuration (CALL-RES-A0
through A8, and anything added later) is expected to be re-run against
every case here, so a new technique's effect on a PREVIOUSLY KNOWN
problem is directly comparable across configurations, not just against
whatever corpus a given evaluation happened to sample.

Each case records: repository, commit, exact caller, call site/span (as
precisely as the source text identifies it), the expected (ground-
truth, manually confirmed) target, and each configuration's actual
predicted target(s), with the failure type.

## Case CRC-001: ambiguous unqualified constructor call, two same-named classes, no scope signal

- **Repository:** `pytorch/examples` (https://github.com/pytorch/examples, BSD-3-Clause)
- **Commit:** `001d493285083b54cacbc1334cb26338bdff2455`
- **Caller:** `distributed/rpc/rnn/main.py::_run_trainer` (a free function, no enclosing class)
- **Call site:** `model = rnn.RNNModel('ps', ntoken, ninp, nhid, nlayers)` (line 30 of that file at this commit)
- **Expected target (ground truth, Manually validated by direct source reading):** `distributed/rpc/rnn/rnn.py::RNNModel`
- **Why it's ambiguous by name alone:** a second, unrelated class also named `RNNModel` exists in this same corpus, at `word_language_model/model.py::RNNModel`.
- **Discovered by:** [B2-EVAL-002](../evaluations/B2-EVAL-002.md), while re-deriving the true (not `.first()`-truncated) answer to B2-EVAL-001's own Q4.

| Configuration | Predicted target(s) | Failure type |
|---|---|---|
| CALL-RES-A0 (name-only, corpus-wide — the original Task 7 design) | BOTH `distributed/rpc/rnn/rnn.py::RNNModel` (correct) AND `word_language_model/model.py::RNNModel` (incorrect), as two separate `NameHeuristic` edges | True positive present, but inseparably mixed with one false positive — a consumer following this edge cannot tell which of the two is real without reading source |
| CALL-RES-A1+A2+A3 (Task 9.5's shipped configuration) | None — no edge produced for either candidate | False negative (the true target is lost); zero false positives |

**Not yet evaluated against this case:** CALL-RES-A4 (import/visibility-aware — would need to resolve that `main.py` imports the `rnn` module specifically from `distributed/rpc/rnn/rnn.py`, which the `import rnn` statement at the top of that file plausibly supports once import-to-file resolution exists), A5 (type-aware), A6 (points-to-assisted), A7 (context-sensitive), A8 (hybrid). Per the Experimental Policy, none of these is implemented speculatively here — this entry exists so that whichever of them is built next can be scored against this exact, real case alongside whatever new corpus it's evaluated on.

**What a future technique must achieve to "pass" this case:**
1. Recover the true edge (`_run_trainer -> distributed/rpc/rnn/rnn.py::RNNModel`).
2. Do so WITHOUT reintroducing the unrelated `word_language_model/model.py::RNNModel` false positive CALL-RES-A0 produced.

A technique that achieves (1) but not (2) is not an improvement over CALL-RES-A0's own result for this case; a technique that cannot achieve both should be measured honestly on this case rather than only on cases where ambiguity is easier.

## Case CRC-002: destructured prop with the same name as an unrelated same-named function in another file

- **Repository:** `yoonge/conduit-react` (https://github.com/yoonge/conduit-react, MIT)
- **Commit:** `56e1aa36b97418cbcfc19cf2c40136093a496af7`
- **Caller:** `src/pages/TopicDetail/CommentForm/index.tsx::handleSubmit`
- **Call site:** `handleToastShow('Error', 'Comment is required.', 'danger')` (and separately `await handleCommentSubmit(newComment as Comment)`) — both `handleToastShow` and `handleCommentSubmit` are PROPS destructured from `CommentForm`'s own function-component parameter (`const CommentForm: React.FC<CommentFormProps> = ({ handleCommentSubmit, handleToastShow, loading, topicId }) => {...}`, confirmed by direct source reading), never resolved at the call site to anything but the caller's own received function value at runtime.
- **Expected target (ground truth, Manually validated by direct source reading):** no static target exists at all — a prop is a runtime value supplied by whichever parent renders the component (here, `TopicDetail`'s own JSX, which this pipeline does not currently represent as a typed edge at all — see B2-EVAL-004's own Section D/gap notes on JSX-as-call). The CORRECT behavior is either no edge, or an edge explicitly marked as "resolved to a same-named prop, not a definition" — never a confident edge to an unrelated same-named function.
- **Why it's ambiguous by name alone:** `TopicDetail/index.tsx` happens to define its own real functions also named `handleCommentSubmit` and `handleToastShow` (the functions it passes INTO `CommentForm` as those very props) — same name, same file-tree neighborhood (parent/child component files), but a completely different symbol serving a completely different role (prop declaration site vs. prop consumption site).
- **Discovered by:** [B2-EVAL-004](../evaluations/B2-EVAL-004.md), 30-edge (25-edge, full-corpus) manual call-quality sample.

| Configuration | Predicted target(s) | Failure type |
|---|---:|---|
| CALL-RES-A1+A2+A3 (Task 9.5's shipped configuration, unchanged since B2-ID-006/007) | `src/pages/TopicDetail/index.tsx::handleCommentSubmit` / `::handleToastShow`, both `StaticAnalysis`, both reported as "resolves to exactly one candidate in the given snapshot pool" | **False positive** (confident, StaticAnalysis-labeled edge to a plausible-looking but factually wrong target — the resolver has no concept of "this identifier is a function parameter/prop binding, not a free reference," so a same-named top-level function anywhere in the corpus wins by elimination the moment it is the only other same-named candidate) |

**Why this is a distinct case from CRC-001, not a duplicate:** CRC-001 is an ambiguity failure (two equally plausible candidates, resolver either picks both or neither). CRC-002 is a SCOPE-BLINDNESS failure (the resolver is not ambiguous at all — it is confident — because it cannot see that the calling identifier is locally bound as a parameter, which should have removed it from consideration as "a free reference to some other symbol" entirely, before any cross-file candidate search ever started). A fix for CRC-001's ambiguity (e.g. import/visibility-aware resolution) would NOT fix CRC-002, because CRC-002's defect occurs before ambiguity resolution is even reached — it is a missing local-binding/shadowing check, not a candidate-ranking problem.

**What a future technique must achieve to "pass" this case:**
1. Recognize that `handleCommentSubmit`/`handleToastShow`, as used inside `CommentForm`'s own body, are bound by that component's own destructured parameter list — a local binding that shadows any same-named symbol anywhere else in the corpus, by ordinary lexical-scoping rules any JS/TS engine would apply.
2. Therefore produce NO edge to `TopicDetail`'s own same-named functions for these two call sites (the correct relationship — "which component supplies this prop" — is a prop-flow/JSX-composition question this pipeline does not yet model at all, not a `Calls` question).
