# B1 Task 8 - evaluation split and isolation scaffolding

2026-10-30, baseline 8ca272b. Added eval/rag dev/calibration/test directories,
runner-only Python JSONL loader (standard library, no dependencies), five loader
tests and one production-path API contamination guard. Loader tests were RED on
the unimplemented loader, then 5/5 GREEN. The Rust guard passed 1/1 and proves
two cold rebuilds cannot retrieve distinct question/answer canaries from nested
label files (including a deliberately misleading .record suffix), while the
legitimate note fixture is retrieved. Runtime code was already isolated by
Vault::list_note_ids; no artificial runtime change was needed to make this guard
pass. Rust fixtures include labels only under integration tests, never runtime.

The loader whitelists the three splits, resolves canonical paths, rejects vault/
evaluation tree overlap and escaping labels, validates rows and reports malformed
input as errors. It is runner-side path isolation, not a runtime sandbox or a
proof against indirect copied contamination. README records B3's freeze-before-
heldout, commit/date/dataset/query hashes, protocol, cache invalidation and
post-benchmark contamination obligations. No benchmark thresholds or quality
results fabricated. The test labels are mechanical canaries only.

Independent review GO, no Critical/Important. Reviewer personally reran all five
Python tests, held_out_labels (two rebuilds), and Clippy workspace clean. They
confirmed runtime crates have no label-loader/path/canary references. No external
dependencies added, no spec or storage changes, no merge/tag/push. Final full
workspace/mutation/scalability checks are Task 9, which follows this commit.
