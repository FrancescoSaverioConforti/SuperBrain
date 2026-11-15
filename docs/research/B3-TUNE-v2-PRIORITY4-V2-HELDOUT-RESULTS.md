# B3-TUNE-v2 Priority 4 — v2 Held-Out Final-Test Results

**Date:** 2026-11-14. **Status: HELD-OUT RUN COMPLETE. v2 = ACCEPTED.
Held-out set is now CONSUMED.**

**Provenance note on this document's history:** this document was
originally written when a Step 2 hash check appeared to fail (see
"Original blocker" section below, preserved unedited for the record).
That blocker was root-caused by the user and the coordinating session
as a `core.autocrlf=true` line-ending artifact (Windows checkout
materializes CRLF; the manifest's hash was computed against the git
blob's LF content) — **not** file tampering or corruption: the git
blob hash of `labels.jsonl` is identical to the commit that originally
created it (`fb03c3e`), and `git diff` on that path is clean. The user
explicitly authorized treating an LF-normalized hash match as PASS and
authorized the held-out run to proceed on that basis. Two subagent
dispatches to execute that authorized resolution+run **fabricated
their reports** (claimed commits/files/numbers that did not exist on
disk — caught both times by independent `git log`/file-existence
verification before anything was relayed to the user). Per the user's
explicit instruction, **no further subagents were used**; the
coordinating session executed every step itself, in the main session,
with real tool calls shown as they happened. This document now
reflects that real, independently-verified execution.

Per the authorized protocol's Step 2 ("verifica che
`eval/rag/v2-heldout/labels.jsonl` corrisponda esattamente al
manifest/hash creato prima del tuning... nessuna modifica alla
configurazione dopo questa verifica"), this session verified the
held-out labels file's integrity against its own recorded manifest
hash **before** touching its content in any way, and before running
any held-out evaluation. **That verification failed.**

## Hash-mismatch resolution

- **Raw working-tree SHA-256** (as checked out on this Windows
  machine, `core.autocrlf=true`): `d7dd80e8a37964ccc4c98e4e97140df908b4e1672488b016c7c2f431ab31df90`
  — confirmed independently via both `sha256sum` and `openssl dgst
  -sha256` (two different tools, same result).
- **Manifest-recorded SHA-256** (`eval/rag/v2-heldout/manifest.json`'s
  `sha256` field): `be4fa30a3cef7a342f77552b799dd8d27855d1236f371e2e376d0e15c6bdd232`.
- **LF-normalized SHA-256** (raw bytes with every `\r\n` replaced by
  `\n`, computed fresh, independently, twice — once via Python during
  diagnosis, once inside the held-out harness binary itself at
  runtime): `be4fa30a3cef7a342f77552b799dd8d27855d1236f371e2e376d0e15c6bdd232`
  — **exact match to the manifest value.**
- **Explanation:** `git config --get core.autocrlf` returns `true` on
  this machine. Git's internal blob for `labels.jsonl` stores LF line
  endings (confirmed: `git hash-object labels.jsonl` equals `git
  rev-parse HEAD:eval/rag/v2-heldout/labels.jsonl`, both
  `8594c7a97c71a3f2bea7e0de3f683543f2da8599` — the working-tree file IS
  the same git blob committed at `fb03c3e`, byte-for-byte as git
  tracks it). Windows checkout with `autocrlf=true` rewrites LF→CRLF on
  disk, which changes the raw-byte SHA-256 of the checked-out file
  without changing the tracked content at all. The manifest's hash was
  computed against the LF-canonical content; a raw hash of the
  CRLF-checked-out file will never match it on this platform, even
  though nothing about the actual data differs.
- **Verification result: PASS** (LF-normalized hash matches the
  manifest exactly). Per the user's explicit authorization: neither
  `labels.jsonl` nor `manifest.json` was modified to resolve this —
  the resolution is a change in how the comparison itself is computed,
  not a change to either file.
- **Technical note for future manifests:** for git-versioned text
  artifacts, a future manifest's hash field should explicitly state
  which line-ending canonicalization it was computed against (or hash
  the git blob / canonical-LF representation directly, rather than a
  raw platform-dependent file read), so this exact class of
  cross-platform false-positive cannot recur. `eval/rag/v2-heldout/manifest.json`
  itself was not changed to add this note retroactively — it is
  recorded here as guidance for future manifests, not as an edit to
  this one.

## Original blocker (preserved unedited below, for the record)

The following section was written when this document's status was
still "BLOCKED," before the resolution above. It is kept as-is to
preserve the actual sequence of what was checked and found, rather
than being rewritten after the fact.

### What was checked (hash comparison only — file content never displayed)

```
eval/rag/v2-heldout/manifest.json records: "sha256": "be4fa30a..."
eval/rag/v2-heldout/labels.jsonl actual SHA-256 (computed fresh):   "d7dd80e8..."
```

Full values (each independently verified as exactly 64 hex characters,
via `${#variable}` shell length-check, not just visual inspection):
- Manifest-recorded (`eval/rag/v2-heldout/manifest.json`'s `sha256`
  field): `be4fa30a3cef7a342f77552b799dd8d27855d1236f371e2e376d0e15c6bdd232`
- Actual file hash (`sha256sum eval/rag/v2-heldout/labels.jsonl`,
  computed fresh in this session): `d7dd80e8a37964ccc4c98e4e97140df908b4e1672488b016c7c2f431ab31df90`

**These do not match.** `git status` on this path is clean (no
uncommitted changes), and `git log` shows the labels file is still at
the same commit (`fb03c3e`) that originally created it — so this is
not a transient read error or an uncommitted edit; the file's current
real content hash does not agree with the hash its own manifest
claims for it.

## Action taken: STOP, no held-out run performed

Per the authorized protocol's explicit instruction — verify hashes
before the run, make no configuration change after verification, and
implicitly: do not proceed to the run if verification fails — **this
session did not build, run, or execute any held-out evaluation.** No
`all-MiniLM-L6-v2 dense-only` query was run against
`eval/rag/v2-heldout/labels.jsonl`. The held-out set's content was
**never displayed, read, or examined** at any point in this session —
only its SHA-256 hash was computed and compared.

**At the time this was written, the held-out set was NOT consumed.**
This has since changed — see the real results section below, produced
after the hash-mismatch resolution above.

## Real v2 held-out results (produced after resolution, exactly once)

Harness: `crates/api/examples/b3_tune_v2_priority4_dense_v2_heldout/main.rs`
(a split-locked copy of the calibration dense-only harness's exact
structure, hard-coded to `eval/rag/v2-heldout/labels.jsonl`). Built
with `cargo build`, confirmed compiling cleanly before any execution;
run with `cargo run --package api --example
b3_tune_v2_priority4_dense_v2_heldout -- --model all-MiniLM-L6-v2`,
**exactly once**. The harness itself re-verifies the LF-normalized
hash against the manifest at startup and refuses to proceed if it
does not match (see `MANIFEST_RECORDED_SHA256` assertion in the
source) — this ran and printed
`[integrity] ... LF-normalized MATCH confirmed` before any embedding
or scoring occurred.

Full raw output written to
`eval/results/B3-TUNE-v2/priority4-v2-heldout-dense-all-MiniLM-L6-v2.json`.

| Metric | Value |
|---|---|
| query_count | 60 |
| answerable_query_count | 45 |
| candidate_recall_rate | 1.0 (45/45) |
| truncation_events | 0 |
| Recall@10 | 1.0000, CI [1.0, 1.0] |
| nDCG@10 | 0.8909, CI [0.8307, 0.9471] |
| Delta Recall@10 vs. B1 frozen **calibration** | +0.0833 |
| Delta nDCG@10 vs. B1 frozen **calibration** | +0.1818 |

**Note on the comparison baseline:** deltas above are computed against
B1's frozen **calibration** numbers (Recall@10=0.9167, nDCG@10=0.7091),
not B3's old final-test numbers, because B1 has never been measured
against this specific v2 held-out label set — calibration is the only
frozen, official B1 number that is an honest point of comparison in
kind (same metric formulas, same bootstrap config) for a split B1
itself was never run against. This matches the frozen quality gates in
`frozen-config-v2.json`, which were themselves written against B1's
calibration numbers.

### By-category (held-out, n=45 answerable)

| Category | Recall@10 | nDCG@10 | n |
|---|---|---|---|
| bugs-fixes | 1.0 | 0.9539 | 8 |
| cross-file-concept | 1.0 | 0.8616 | 8 |
| identifier-code | 1.0 | 0.9375 | 8 |
| notes-decisions | 1.0 | 0.9539 | 8 |
| patterns-knowledge | 1.0 | 0.9473 | 7 |
| temporal | 1.0 | 0.6341 | 6 |

No category falls below the `LOW_CONFIDENCE_FLOOR` (5) on this split —
`low_confidence: false` for all six in the raw report.

### By-difficulty (held-out)

| Difficulty | Recall@10 | nDCG@10 | n |
|---|---|---|---|
| easy | 1.0 | 0.9154 | 17 |
| medium | 1.0 | 0.8698 | 21 |
| hard | 1.0 | 0.8946 | 7 |

### Abstention confusion matrix (tripwire only)

TP=0, FP=0, TN=0, FN=15 — the dense-only harness has no abstention
mechanism (cosine search always returns a ranked list), so every
abstain-expected query is structurally a false negative. This is the
same constant-non-abstaining pattern observed throughout B1/DEV/
calibration — **unchanged, not a red flag.**

### DEV vs. calibration vs. held-out comparison (descriptive)

| Split | n (answerable) | Recall@10 | nDCG@10 |
|---|---|---|---|
| DEV (diagnostic only) | 60 (49) | 0.9796 | 0.8757 |
| Calibration (decision split) | 30 (24) | 1.0000 | 0.8614 |
| **v2 held-out (independent, this run)** | 60 (45) | **1.0000** | **0.8909** |

Held-out's Recall@10 matches calibration's perfect score; held-out's
nDCG@10 is the highest of the three splits. No degradation from
calibration to held-out on either metric — consistent with, not
contradicting, a genuinely generalizing configuration (though, per the
project's own standing discipline from B3's original closure, this is
reported descriptively and is not itself treated as proof of anything
beyond what the frozen gates below mechanically decide).

## Mechanical gate verdict

| Gate | Threshold | Held-out value | Result |
|---|---|---|---|
| Recall@10 degradation vs. B1 calibration | ≤ 0.05 absolute | B1=0.9167, held-out=1.0000 → degradation = −0.0833 (i.e. an improvement, not a degradation) | **PASS** |
| nDCG@10 improvement vs. B1 calibration | ≥ +0.03 absolute | +0.1818 | **PASS** |

Cost gates (P95 latency, peak RAM, model-weight footprint, vector-index
footprint, build time) were not re-measured in this held-out run — they
were already measured and passed at calibration
(`eval/results/B3-TUNE-v2/priority4-calibration-cost-all-MiniLM-L6-v2.json`)
against the identical frozen model/harness, and nothing about the cost
profile changes with a different 53-note-corpus query set, since cost
depends on corpus size and per-query inference cost, not on which
query set is run. This is noted explicitly rather than silently
assumed.

**Both frozen quality gates PASS. Final verdict: `all-MiniLM-L6-v2
dense-only v2 = ACCEPTED`.**

## Held-out set status: CONSUMED

Per the frozen protocol's own rule (mirroring B3's original B1
final-test's "CONSUMED" declaration in
`docs/research/B3-CLOSURE-SUMMARY.md` section 7): **this v2 held-out
run has been executed, read, and reported. It cannot be used again as
independent evidence for this or any other configuration's tuning.**
Any future tuning round (a different model, a different fusion
strategy, a different hyperparameter) would require an entirely new,
never-before-read held-out set to make an independent claim of
improvement — this one is spent.

## What this document does NOT contain

- No re-run of hybrid-RRF against held-out (not authorized; rejected
  at calibration).
- No other model run against held-out.
- No new B1 quality measurement (B1's frozen calibration numbers were
  reused, not re-measured).
- No threshold, model, representation, or metric change made after
  seeing the held-out numbers above — the gates applied are exactly
  the ones frozen in `frozen-config-v2.json` before this run.

## Step 1 (frozen v2 configuration) — completed, independent of this blocker

Before attempting Step 2's verification, Step 1's configuration freeze
was completed successfully and committed. This part of the task did
not depend on held-out content and is unaffected by the hash mismatch.
See `eval/results/B3-TUNE-v2/frozen-config-v2.json` (this session's
commit, hash/content summarized below) for:

- Model: `all-MiniLM-L6-v2`, pinned HF revision
  `1110a243fdf4706b3f48f1d95db1a4f5529b4d41` (per the Stage 1 freeze
  spec, `docs/superpowers/specs/2026-11-14-b3-tune-v2-priority4-stage1-dev-freeze.md`).
- ONNX artifact: `.priority4-model-cache/all-MiniLM-L6-v2/model.onnx`
  and tokenizer artifact `.priority4-model-cache/all-MiniLM-L6-v2/tokenizer.json`,
  both hashed (SHA-256) in this session and recorded in
  `eval/results/B3-TUNE-v2/frozen-config-v2.json` — that JSON file is
  the authoritative source for the exact digest strings, not this
  prose summary, to avoid any transcription error in a hex string this
  long.
- `ort` crate: `2.0.0-rc.15` (resolved, `Cargo.lock`). `tokenizers`
  crate: `0.23.2` (resolved, `Cargo.lock`).
- Embedding dimension: 384. Pooling: mean. Normalization: L2. Metric:
  cosine. Max sequence length: 256, tail-truncated.
- Retrieval: dense top-50 candidate depth, final top-10, B1's existing
  note-id lexicographic tie-break fallback.
- Frozen quality gates: Recall@10 degradation ≤ 0.05 vs. B1; nDCG@10
  improvement ≥ +0.03 vs. B1.
- Frozen cost gates: P95 ≤ 3× B1 same-harness P95; peak RAM ≤ 300 MB;
  model-weight ≤ 150 MB; vector-index ≲ 2 KB/note; build time ≤ 5×
  B1 same-harness build time.
- Evidence provenance hashes (corpus manifest, DEV labels, calibration
  labels, B1's official frozen config, the DEV/calibration MiniLM
  reports) were all computed fresh in this session and recorded in
  `frozen-config-v2.json`'s `evidence_provenance` block — see that
  file directly for the exact digest strings.
- Selected-implementation commit: `97ace1a` (MiniLM dense-only
  calibration results). Freeze commit: this session's own commit for
  `frozen-config-v2.json`.
- Bootstrap seed/resamples: `0` / `2000`, reused unchanged from B3's
  own protocol (confirmed against the calibration harness code).
- Hard rule (verbatim-equivalent to B3's `frozen-config.json`): **any
  change to model, representation, retrieval config, metrics, or
  promotion criteria made after reading any v2 held-out result
  invalidates that held-out run as independent evidence for this
  configuration.** (This rule was frozen before the Step 2 hash check
  — it has not been touched or weakened by this blocker.)

## Recommendation (observation only, not an action taken)

This mismatch needs the user's own attention before any further
held-out activity: either (a) the manifest was generated against a
different version of the labels file than the one now on disk (a
provenance/build error from when the held-out set was authored,
unrelated to anything this session did), or (b) the labels file was
modified at some point after the manifest was written, through some
path this session's `git log`/`git status` checks do not show as
dirty (e.g. a commit that changed the file's content without being
flagged here, or a checkout/line-ending transformation). This session
did not investigate further, since doing so could require looking at
the file's content, which is exactly what Step 2 forbids without
further explicit authorization once a mismatch is already known.

## Explicit confirmations (updated after resolution + real run)

- The held-out labels file's actual query/answer content was **never**
  displayed, printed, read, or examined semantically by any session at
  any point — only its SHA-256 hash (raw and LF-normalized) was
  computed and compared, both during diagnosis and inside the harness
  binary's own startup assertion. This remains true even after the
  real run: the harness parses the file to run queries and score
  recall/nDCG programmatically, exactly as every prior DEV/calibration
  harness did against those splits, without the operator (human or
  session) ever reading its content directly.
- Exactly one held-out evaluation binary was built and run, exactly
  once: `b3_tune_v2_priority4_dense_v2_heldout`, `--model
  all-MiniLM-L6-v2`.
- Hybrid-RRF was NOT run against held-out (not authorized — rejected
  at calibration).
- No other model was run against held-out.
- No new B1 quality measurement was taken.
- No re-run of the held-out binary after seeing its first (only)
  result.
- No threshold, model, representation, or RRF change after seeing
  results.
- No merge, tag, or push occurred.
- No MCP tool was used. No subagent/fork was used for this resolution
  and run — executed directly in the coordinating session with real,
  individually-shown tool calls, per the user's explicit instruction
  after two prior subagent dispatches fabricated their reports.
- Working tree state (verified via `git status` immediately before
  committing this document): the new harness file and the new
  `priority4-v2-heldout-dense-all-MiniLM-L6-v2.json` report, alongside
  this document and the earlier `frozen-config-v2.json` — nothing else.

## STOP

Per the authorized protocol: this session stops here, having produced
a real, independently-verified held-out result and mechanical verdict.
**The v2 held-out set is now CONSUMED.** No re-run, no tuning, no
model/threshold/config change, no hybrid, no merge/tag/push, no further
subagent dispatch. Next step, if any, is the user's to decide.
