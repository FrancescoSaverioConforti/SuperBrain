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

## Case CRC-003: call site uses an import alias, resolver has no alias-to-real-name mapping, so a real cross-module call is simply never recovered

- **Repository:** `pyjanitor-devs/pyjanitor` (https://github.com/pyjanitor-devs/pyjanitor, MIT)
- **Commit:** `1247923ff01dd9a74f40f55d55f108e503e4b218`
- **Caller:** `janitor/__init__.py`'s own deprecated wrapper function `get_features_targets(*args, **kwargs)` (itself a different symbol from the real target, sharing its name only because this file re-exports a deprecated top-level alias of the same name).
- **Call site:** `return _get_features_targets(*args, **kwargs)` (`janitor/__init__.py`, inside the wrapper body), where `_get_features_targets` is a LOCAL IMPORT ALIAS introduced by `from .ml import get_features_targets as _get_features_targets` at the top of the same file.
- **Expected target (ground truth, Manually validated by direct source reading):** `janitor/ml.py::get_features_targets` — the real function this alias refers to, confirmed by the `import ... as` statement naming it explicitly.
- **Why it's missed, not just ambiguous:** this pipeline's `resolve_calls` matches a call site's own callee NAME directly against the snapshot pool by string equality (`"_get_features_targets"`). No snapshot in this corpus is named `_get_features_targets` — the real function is named `get_features_targets` — so the lookup returns ZERO candidates, not multiple. This is a different failure shape from both CRC-001 (multiple real candidates) and CRC-002 (a confident but wrong single match due to scope-blindness): here, the resolver is honestly uncertain (zero candidates), and — per this pipeline's own "no edge beats a false edge" discipline — correctly produces NO edge at all rather than guessing. The call is real and the target is real, but the relationship between them is never recovered, because `ImportFact`'s own alias information (`alias: Some("_get_features_targets")`, confirmed present and correctly captured in this corpus's own `ImportFact` data, 459 aliases captured in total) is never CONSULTED by `resolve_calls` at all — the two pieces of information (the alias mapping, and the call site's own callee name) exist independently in this pipeline today but are never joined.
- **Discovered by:** [B2-EVAL-006](../evaluations/B2-EVAL-006.md), while root-causing an unexpectedly empty caller list for `get_features_targets` in its own coding-agent usefulness Q&A section.

| Configuration | Predicted target(s) | Failure type |
|---|---:|---|
| CALL-RES-A1+A2+A3 (Task 9.5's shipped configuration, unchanged since B2-ID-006/007) | None — no edge produced | **False negative** (a genuinely missed real call, not a false positive — zero candidates matched the alias name, and the resolver correctly declined to guess rather than, say, picking a same-named-but-wrong symbol; the correctness discipline held, but the real relationship is still unrecovered) |

**Why this is a distinct case from CRC-001 and CRC-002:** CRC-001 is an ambiguity failure (too many candidates). CRC-002 is a scope-blindness failure (a confident match to the WRONG single candidate, because a local binding was not recognized as shadowing). CRC-003 is an **alias-blindness** failure: zero candidates match, because the information needed to connect the call site's own locally-aliased name back to the real target's own canonical name exists in this pipeline (`ImportFact.alias`) but is architecturally separate from `resolve_calls`'s own name-matching logic, which never consults it. A technique that fixes CRC-001 (candidate ranking) or CRC-002 (local-binding/shadowing detection) would NOT fix this case — it requires joining import-alias data INTO the call-resolution name-lookup step specifically, a different kind of fix (import/visibility-aware resolution, per this project's own CALL-RES-A4 backlog entry) than either of the other two cases needs.

**What a future technique must achieve to "pass" this case:**
1. When resolving a callee name that does not match any snapshot directly, check whether the CALLING FILE's own `ImportFact`s include an alias whose `alias` field matches the callee name — if so, re-resolve using the alias's own real imported-symbol name instead.
2. Produce the real edge (`janitor/__init__.py::get_features_targets -> janitor/ml.py::get_features_targets`) once that re-resolution succeeds, with evidence that names the alias explicitly (so a reader can tell "resolved via import alias" apart from "resolved by direct name match").
