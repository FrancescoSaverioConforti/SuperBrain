# B3-TUNE-v2 Priority 4 Stage 1, Task 1 — Embedding Runtime Selection

**Date:** 2026-11-14. **Status:** Task 1 of
`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority4-stage1-dev-only.md`,
complete. No benchmark has run yet (Task 2 onward).

## Runtime chosen: ONNX Runtime via the Rust `ort` crate

**Exact dependency versions added to `crates/api/Cargo.toml`
(dev-dependencies, never a production dependency — this is an
example-only harness):**

- `ort = "2.0.0-rc.15"` (features: `download-binaries`, `ndarray`)
- `tokenizers = "0.23.2"` (fast-tokenizer loading, `tokenizer.json`
  format, no Python)
- `ndarray = "0.17.2"` (tensor shaping for model input/output)

All three resolved and built successfully (`cargo check -p api
--tests`, confirmed in this session).

## License verification (per plan Task 1 requirement)

Checked directly against crates.io metadata, not assumed from
popularity:

| Crate | License |
|---|---|
| `ort` | MIT OR Apache-2.0 |
| `tokenizers` | Apache-2.0 |
| `ndarray` | MIT OR Apache-2.0 |

All three are compatible with this project's own Apache-2.0 license.

**`ort`'s `download-binaries` feature** fetches a prebuilt ONNX
Runtime shared library once, from `cdn.pyke.io` (the `ort` crate
maintainer's own CDN, redistributing Microsoft's official ONNX Runtime
release — observed version `1.31.0`, Windows x86_64 build with
DirectML). This download happened automatically during this session's
`cargo check` and is now cached locally at
`%LOCALAPPDATA%\ort.pyke.io\dfbin\x86_64-pc-windows-msvc\<hash>\`
(confirmed present in this session) — it is a one-time fetch at
build-script time, not a runtime network call; subsequent builds and
all inference calls use the cached binary with no further network
access. ONNX Runtime itself is Microsoft's official project, MIT-
licensed — compatible with this project's Apache-2.0 licensing and
local-first/offline distribution model.

## Offline completeness

After this one-time binary fetch, no further network access is
required for ONNX Runtime itself to function. Each of the 4
shortlisted models' own ONNX weight files (`onnx/model.onnx` from
each model's HuggingFace repository) and tokenizer files
(`tokenizer.json`) must still be fetched once per model — this
happens in Task 2, scoped to exactly the 4 authorized shortlist
models at their pinned revisions (Section 1 of the frozen spec,
`docs/superpowers/specs/2026-11-14-b3-tune-v2-priority4-stage1-dev-freeze.md`),
nothing else.

## `trust_remote_code` / custom-code re-confirmation (per plan Task 1 requirement)

Re-checked directly against each model's live HuggingFace repository
metadata (`config.json`, file listings), not carried forward from the
shortlist audit's documentation-level estimate:

- **MiniLM, mpnet, BGE:** standard `bert`/`mpnet`-type `config.json`,
  no `auto_map` entries, native `transformers` support confirmed. Not
  applicable to the ONNX loading path used here regardless.
- **Nomic:** `config.json` **does** carry `auto_map` entries pointing
  to `nomic-ai/nomic-bert-2048`'s custom
  `modeling_hf_nomic_bert.py` — this is exactly what triggers
  `trust_remote_code=True` in the PyTorch/`transformers` loading path,
  confirming the shortlist audit's prediction for that specific
  loading path.
- **Correction, material to this benchmark:** this Stage 1 benchmark
  does not use the PyTorch/`transformers` loading path at all. All 4
  models have a published, static ONNX export
  (`onnx/model.onnx`, confirmed present in each repository's file
  listing) that this benchmark loads directly through `ort`. An ONNX
  graph is a fixed computational graph with no embedded Python code;
  loading and running it executes no custom modeling code and does
  not invoke or require `trust_remote_code` in any form — the concept
  does not apply to this loading path. **Nomic's custom-code
  supply-chain cost identified in the shortlist audit is therefore
  resolved as a side effect of this runtime choice**, not merely
  accepted as a cost to weigh. The model-weight supply chain (pinning
  the fetched ONNX file's own hash, Section headed "Pinned model
  revisions" in the frozen spec) still applies identically to all 4
  models, Nomic included.

This correction is also recorded in the frozen spec document (Section
4) as the authoritative, re-verified state for Task 4's selection
step — the model-selection tie-break's fixed precedence order is left
unmodified, per that document's own reasoning.

## What this task does not do

- No model weights downloaded yet (ONNX graphs, tokenizers) — that is
  Task 2.
- No embeddings generated.
- No benchmark run.
- No calibration, no held-out read.
