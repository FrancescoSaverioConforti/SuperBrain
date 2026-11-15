# B3-TUNE-v2 Priority 4 — v2 Held-Out Verification: BLOCKED

**Date:** 2026-11-14. **Status: HELD-OUT RUN DID NOT PROCEED.**

Per the authorized protocol's Step 2 ("verifica che
`eval/rag/v2-heldout/labels.jsonl` corrisponda esattamente al
manifest/hash creato prima del tuning... nessuna modifica alla
configurazione dopo questa verifica"), this session verified the
held-out labels file's integrity against its own recorded manifest
hash **before** touching its content in any way, and before running
any held-out evaluation. **That verification failed.**

## What was checked (hash comparison only — file content never displayed)

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

**The held-out set is NOT consumed by this session.** It remains
available for a future run, once this hash discrepancy is resolved
and re-verified by the user or a session they direct.

## What this document does NOT contain

- No held-out Recall@10/nDCG@10 numbers — none were computed.
- No PASS/FAIL/ACCEPTED/FAIL verdict for v2 — the held-out gate was
  never reached.
- No category/difficulty breakdown, candidate_recall_rate, or
  abstention matrix for held-out — none of these were computed.

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

## Explicit confirmations

- The held-out labels file's actual query/answer content was **never**
  displayed, printed, or read by this session — only its hash was
  computed via `sha256sum`.
- No held-out evaluation binary was built or run.
- Hybrid-RRF was not run (moot — nothing was run against held-out).
- No other model was run against held-out.
- No merge, tag, or push occurred.
- No MCP tool was used.
- Working tree is clean apart from this session's own committed
  `frozen-config-v2.json` and this document.

## STOP

This session stops here, as instructed, having reached a verification
failure rather than a held-out result. The v2 held-out set remains
**unconsumed**. No next step is recommended beyond flagging this
mismatch for the user's decision.
