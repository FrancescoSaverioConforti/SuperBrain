# RAG evaluation scaffolding (B1, not a benchmark)

dev, calibration and test are separate splits. Only dev may inform retrieval
changes; calibration freezes configuration; the held-out test is read only after
that freeze. The one test note and canary label row are mechanical isolation
fixtures, not a question set or quality measurement. Other label files are empty.

`runner.py` alone loads labels. It has no runtime API role and requires disjoint
canonical evaluation/vault trees. A caller imports only note fixtures into the
vault through `storage::propose_and_commit`. Never import labels, questions,
answers or runner output into the vault, index, sessions, summaries, logs or
caches. Runtime crates have no reference to this loader or label paths. The
Rust API isolation test puts adversarial labels under a nested vault path and
proves two cold rebuilds cannot retrieve them, while real fixture content works.
This is a narrow path-isolation check, not a universal indirect-copy detector.

Checks:

```text
python -B -m unittest discover -s eval/rag -p test_runner.py -v
cargo test -p api held_out_labels -- --exact
```

B3 must build a stratified representative corpus/query set and labels, freeze
thresholds and configuration before viewing test results, record commit/date,
dataset and query hashes, protocol and rationale. Save raw measurements and
uncertainty intervals, evaluate retrieval/citation/abstention separately, invalidate
eval caches, keep sessions/output out of normal memory, and repeat contamination
checks after a prior benchmark run. A threshold changed after seeing held-out
results invalidates that run as independent evidence. B1 creates no thresholds
or final benchmark claim; provenance/state and indirect-copy controls remain
obligations of the systems that later introduce those surfaces.
