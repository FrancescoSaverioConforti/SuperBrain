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
