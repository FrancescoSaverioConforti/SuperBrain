# B4.8 — Packaging, Offline Setup, and Operational Readiness

**Status: implemented.** `crate::dense::setup` (feature-gated,
`dense-retrieval`) is the one-time, explicit acquisition step; this
document is the operational reference for it.

## 1. Where artifacts live

`.superbrain-model-cache/all-MiniLM-L6-v2/{model.onnx,tokenizer.json}`
at the vault root (B4.1 decision record, Section 7 — mirrors the
existing `.pii-store/` convention, not the evaluation harness's own
`.priority4-model-cache/`). `crate::dense::model_cache_dir(vault_root)`
and `crate::dense::artifact_paths(vault_root)` are the canonical
accessors — nothing else in this codebase should hardcode this path.

## 2. Running setup

`crate::dense::setup(vault_root)` — fetches both artifacts if either
is missing, verifying each one's SHA-256 against the frozen
`ONNX_ARTIFACT_SHA256`/`TOKENIZER_ARTIFACT_SHA256` constants **before**
writing it to its final path. A hash mismatch at fetch time is a hard
`SetupError`, never a silently-accepted unverified file. Idempotent:
if both artifacts already exist, returns immediately with **zero
network access** — confirmed by this module's own test
(`setup_is_a_no_op_requiring_no_network_access_when_both_artifacts_already_exist`),
which would fail loudly (a connection error) rather than silently
pass if it actually attempted a fetch in a sandboxed test environment.

This is the **only** function in `crate::dense` that makes a network
call. `EmbeddingModel::load` only ever reads files already on disk —
there is no network-capable code path reachable from `load` at all
(`ureq` is referenced nowhere else in the module), which is what makes
"offline after setup" a structural property of this module's code
layout, not merely a documented intention.

## 3. No model weights in the Git repository

Confirmed: `.superbrain-model-cache/` is not tracked (mirrors
`.priority4-model-cache/`'s own existing `.gitignore` entry — a new
entry for the production path should be added to `.gitignore` at the
point this feature is actually deployed; not added here since no
build has yet created the directory on this machine outside of
tests, which use their own temp directories, never the real vault
root's cache path).

## 4. Diagnosing setup/load failures

| Symptom | Cause | What to do |
|---|---|---|
| `SetupError::FetchFailed` | Network unavailable, HF unreachable, or the pinned revision was removed/moved upstream | Retry once network is confirmed available; if the pinned revision truly no longer exists at that URL, this requires re-pinning the model revision — a decision outside this task's scope (it would mean re-opening Priority 4's own frozen configuration) |
| `SetupError::FetchedArtifactHashMismatch` | The download completed but didn't match the frozen hash — could be a corrupted transfer, a MITM, or an upstream content change at a supposedly-immutable pinned revision | Re-run `setup` once (transient corruption is the common case); if it mismatches consistently, treat as a security-relevant anomaly, not a retry-until-it-works problem |
| `LoadError::ArtifactReadFailed` | `setup` was never run, or the cache directory was deleted/moved after setup | Run `setup(vault_root)` |
| `LoadError::HashMismatch` (from `EmbeddingModel::load`, distinct from `SetupError`'s own hash check) | An artifact already on disk was modified/corrupted after a successful `setup` | Delete the artifact and re-run `setup` |
| `LoadError::OnnxSessionFailed` | ONNX Runtime native library missing/incompatible, or `ort`'s `download-binaries` fetch (B4.1 decision: still network-dependent, not vendored) never completed | Confirm network access for `ort`'s own one-time runtime-binary fetch; see B4.1's own "Target/platform support" open decision for the vendoring alternative, not adopted yet |
| `DenseScorer::score` returns `None` in production | Any of the above, OR the dense index was never built (`rebuild` never called), OR the persisted index's `model_identity_hash` doesn't match the loaded model | This is B4.0's frozen fallback contract working as intended — search falls back to `Lexical`, not an error; check logs for which specific condition triggered it (B4.6's wiring point is the place to add that logging, not yet done — see Section 6) |

## 5. Windows verification

This entire B4 implementation (B4.0 through B4.7) was built and its
full test suite run on this Windows machine, including the real
model-load/inference/storage/lifecycle tests against the real cached
artifacts — not inferred from the evaluation harness's own earlier
Windows success (per B4.1's own explicit requirement not to
extrapolate). `ort`'s `download-binaries` feature's Windows behavior
is therefore directly confirmed for this specific production code
path, not assumed.

## 6. What remains explicitly open (named, not resolved here)

- **Logging for the B4.0 fallback warning**: B4.0's frozen contract
  requires a loud, explicit warning when `DenseV2` falls back to
  `Lexical` due to unavailability — `search_impl`'s own B4.6 wiring
  does not yet emit one (it silently uses the lexical path). This
  project has no existing logging facade (confirmed in B4.1); adding
  one (or using `eprintln!` minimally) is deferred to whoever
  integrates this feature into a real calling context, since the
  right mechanism depends on how that context already handles
  diagnostics — not invented speculatively here.
- **`.gitignore` entry for `.superbrain-model-cache/`**: not added,
  since no real deployment has created this directory yet outside of
  tests (which use isolated temp directories). Add it at the point of
  actual deployment.
- **Vendored ONNX Runtime binary**: B4.1's own deferred decision,
  unchanged by this task.
- **License attribution text**: B4.1 confirmed no `deny.toml` license-
  allowlist change is needed beyond `CDLA-Permissive-2.0`; the actual
  attribution *text* accompanying a real distribution is a
  documentation/packaging task for whoever ships this feature, not
  authored speculatively here.
