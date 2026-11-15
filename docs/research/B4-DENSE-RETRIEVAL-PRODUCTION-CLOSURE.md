# B4 — Dense Retrieval Production Integration: Closure

**Branch:** `b4-dense-retrieval-production`, created from `main` at
`099eb28`. **HEAD at closure:** `d38e06e`. **Date:** 2026-11-15.
**Status: COMPLETE. Not merged into `main`.**

---

## 1. Architecture implemented

`v2 MiniLM dense-only` (the B3-TUNE-v2-accepted configuration, per
`eval/results/B3-TUNE-v2/frozen-config-v2.json`) is now available
through the real production API:

```rust
let scorer = ProductionDenseScorer::new(model, index_path);
let idx = index::Index::new(&vault, &indexer, "project-x", today)
    .with_retrieval_mode(index::search::RetrievalMode::DenseV2)
    .with_dense_scorer(&scorer);
idx.search(query, filters, k)
```

- **`RetrievalMode`** (B4.0): a two-variant enum (`Lexical` default,
  `DenseV2` opt-in) on `Index`, set via a new `with_retrieval_mode`
  builder — mirrors every prior lever's own additive, construction-
  time-only pattern.
- **`DenseScorer`** (B4.2, corrected in B4.6): a trait in `crates/index`
  (`candidate_ids`, `score`, `rebuild`, `on_note_indexed`,
  `on_note_deleted`), mirroring `storage::Indexer`'s own cross-crate
  trait-boundary pattern — `index` depends on no embedding runtime at
  all. `candidate_ids` was added in B4.6, correcting B4.2's original
  shape, once resolving the real candidate-set-scoping question
  required it.
- **`EmbeddingModel`** (B4.2, `crates/api/src/dense.rs`): real
  `all-MiniLM-L6-v2` ONNX load/infer/pool/normalize, hash-verified at
  load time against `frozen-config-v2.json`'s own recorded SHA-256
  values — a gap the Priority 4 evaluation harness never closed.
- **`DenseIndex` storage** (B4.3, `crates/api/src/dense_storage.rs`):
  a hand-rolled, versioned binary format (magic, format version,
  CRC32 checksum, combined model-identity hash, per-note entries),
  atomic write (temp file + fsync + rename, with a bounded retry for
  transient Windows sharing violations).
- **`ProductionDenseScorer`** (B4.4, `crates/api/src/dense_lifecycle.rs`):
  the real `DenseScorer` implementation — `Mutex`-guarded interior
  state (mirroring `RealIndexer`'s own pattern), cold-start `rebuild`,
  incremental `on_note_indexed`/`on_note_deleted` via a fast,
  non-cryptographic content hash.
- **`search_impl` wiring** (B4.6, `crates/index/src/search.rs`):
  `DenseV2` candidate generation comes from the dense scorer's OWN
  candidate set (every note it has embedded) — confirmed against the
  real evaluation harness that this, not the lexical candidate
  generator's output, is how every accepted Priority 4 quality number
  was actually measured. A new `MatchKind::SemanticMatch` lets
  dense-sourced candidates bypass the lexical-overlap gate. Structured
  filters, `Sensitive`-redaction, the authoritative per-candidate
  vault re-read, and the final tie-break all run unchanged regardless
  of mode.
- **Packaging** (B4.8, `crate::dense::setup`): the one-time, explicit,
  hash-verified model/tokenizer acquisition step; `EmbeddingModel::load`
  only ever reads already-present files — offline-after-setup is a
  structural property of the code, not just documented intent.

## 2. Deviations from the original design

| Design doc | What changed | Why |
|---|---|---|
| B3-TUNE-v2-PRIORITY4-PRODUCTION-INTEGRATION-DESIGN.md §3 | API surface change landed as a construction-time `Index` builder (`with_retrieval_mode`), not a `Index::search`/`SearchRequest` parameter | The design doc only analyzed `Index::search` directly; reading `api::search::search`'s real signature (it takes an already-built `&Index<'_>`) showed `RetrievalMode` only needs to be set once, at construction, exactly like every other B3-TUNE-v2 lever — simpler than the design doc's own preliminary (and, on reflection, incorrect) analysis of needing a `SearchRequest` field. |
| B4.2's own original `DenseScorer` trait | Added `candidate_ids(&self) -> Option<Vec<NoteId>>`, not present in the original trait | B4.2 only defined `score(query, candidate_ids)`, assuming the caller already knew the candidate set. Resolving B4.6's candidate-set-scoping question (dense index's own set, not the lexical generator's) required a way to ask the scorer what it has — discovered only once that question was actually being answered, not anticipated in advance. |
| frozen-config-v2.json cost-gate denominators | B1's "cost-control" numbers are reused from the Priority 4 evaluation cycle (`priority4-cost-B1-cost-control.json`), not re-measured in the production smoke test | The design intentionally reuses already-frozen numbers rather than re-opening B3-TUNE-v2's own scientific benchmark; the smoke test only measures the real production dense path's own cost, diffed against the existing frozen B1 baseline. |

## 3. Tasks B4.0–B4.8

| Task | Commit(s) | Summary |
|---|---|---|
| B4.0 | `acc0f23` | `RetrievalMode` frozen and wired |
| B4.1 | `55b061e` | `ort`/`tokenizers`/`ndarray` promoted to a feature-gated (`dense-retrieval`, default off) production dependency; caught and fixed a CI-breaking example-compilation regression and a license-allowlist gap (`CDLA-Permissive-2.0`) |
| B4.2 | `c7aa83f`, `25952ef`, `3dd36a5` | `DenseScorer` trait; hash-verified artifact loading; real ONNX embedding engine — found and fixed 2 real bugs (silent 128-token tokenizer truncation cap; a truncation-detection check that could never fire once the tokenizer pre-truncates) |
| B4.3 | `e0452b1` | Persistent, versioned, checksummed, atomically-written vector index storage |
| B4.4 | `c4f9ff8` | `ProductionDenseScorer`: real create/update/delete lifecycle |
| B4.5 | `6ddd4e6` | Privacy parity proven against the real production path (not just the evaluation harness) |
| B4.6 | `2b8e62d` | Wired `DenseV2` into the real `search_impl`; resolved the candidate-set-scoping question |
| B4.7 | `d40df43` | Restart, model-identity-mismatch, and concurrency integration tests |
| B4.8 | `2944507` | Explicit, hash-verified `setup` step; offline-after-setup confirmed structural |
| (closure) | `af3b091`, `c83f95f`, `d38e06e` | Performance smoke test (found+fixed a real Windows file-lock bug); mutation-testing fixes across all 3 flagged files |

**Total: 15 commits, 19 files changed, 4939 insertions, 79 deletions**
(`git diff --stat acc0f23~1..d38e06e`).

## 4. Tests

- **Full workspace, no feature:** `cargo build --workspace` and
  `cargo test --workspace` — every crate, 0 failures (unchanged from
  before B4; confirms zero behavior change for any caller that
  doesn't opt in).
- **`api` crate, `dense-retrieval` feature enabled:** 60 lib tests + 11
  integration tests, 0 failures. Includes: hash verification (match,
  mismatch, tamper rejection), real embedding dimension/normalization/
  determinism/discriminative-similarity/truncation-detection, storage
  round-trip + 3 adversarial corruption tests + a `Display`-message
  test, full lifecycle (rebuild/incremental-update/delete) against a
  real `storage::Vault`, privacy parity (structural + differential +
  reclassification), restart/model-mismatch/concurrency, and setup's
  no-network-on-already-present-artifacts guarantee.
- **`index` crate:** 94 unit + 6 + 53 integration + 0 doc tests, 0
  failures. Includes: `RetrievalMode` contract tests, `DenseScorer`
  trait-boundary tests, and 6 real `DenseV2` wiring tests (fallback
  parity with/without an available scorer, dense-score-not-BM25,
  **the candidate-set-scoping proof** — a note with zero lexical
  overlap is still found under `DenseV2`, structured filters still
  apply, deterministic tie-break).

## 5. Mutation testing (cargo-mutants)

All three files flagged as containing the user-specified critical
logic (stale detection, privacy gating, fallback, lifecycle, dimension/
hash validation, deterministic ordering) were run through
`cargo-mutants` and, after fixes, show **zero real surviving
mutants**:

| File | Result | Real gaps found + fixed |
|---|---|---|
| `crates/api/src/dense_storage.rs` | 45/45 caught (after fix; originally 42/45 + 2 expected timeouts + 1 miss) | `Display for LoadError`'s match arms were never asserted against their real formatted text — fixed with a test pinning the real output for all 5 variants. |
| `crates/api/src/dense_lifecycle.rs` | 25/25 caught (after fix; originally 20/25) | `model_identity_hash` had no test of its real value (only internal-consistency checks a constant would also pass); `candidate_ids()` was only tested indirectly through `score()`; `on_note_indexed`'s stale-detection `==` comparison could be inverted without any test noticing, since re-embedding identical text deterministically produces the same vector anyway (fixed with a test that plants a deliberately wrong embedding to prove a *skip* genuinely happened). |
| `crates/index/src/search.rs` | 62/72 caught, 10 unviable (after fix; originally 62/72 + 9 unviable + 1 miss) | `extract_structural_identifier`'s empty-`#`-fragment case had no coverage at all. **Process note:** a first fix attempt (an external integration test) was traced through the real call chain, found to be unable to actually kill the mutant (the check is genuinely unreachable from `search_impl`'s own real candidate-filtering path), and reverted before being relied on — the correct fix was `search.rs`'s own first internal unit-test module, since the function is module-private. |

**No mutation was left unresolved.** Every miss across all three
files was either fixed with a real, verified-effective test, or (the
`crc32` `+=`→`*=` timeouts in `dense_storage.rs`) confirmed to be an
expected degenerate-loop outcome, not a gap.

## 6. Dependency/package changes

- `ort` 2.0.0-rc.15, `tokenizers` 0.23, `ndarray` 0.17, `sha2` 0.10,
  `ureq` 3 — all promoted from `[dev-dependencies]` to optional,
  `dense-retrieval`-feature-gated `[dependencies]` of `api` (B4.1,
  B4.8). A build without the feature has the exact same dependency
  graph as before B4.
- `deny.toml`: added a documented, re-review-dated exception for
  `RUSTSEC-2024-0436` (`paste`, unmaintained, transitive via
  `tokenizers`), and added `CDLA-Permissive-2.0` to the license
  allowlist (`webpki-root-certs`/`webpki-roots`, transitive via
  `ort-sys`'s build-dependency on `ureq`).
- `.github/workflows/ci.yml`: **not changed.** CI's existing
  `cargo build/test --workspace` and `cargo clippy --workspace`
  commands run without `--all-features`, so they do not exercise
  `dense-retrieval` — a named limitation (Section 8), not silently
  resolved.

## 7. Performance smoke results (operational acceptance, not re-tuning)

Release-mode run (`cargo run --release --package api --example
b4_production_performance_smoke --features dense-retrieval`), 53-note
corpus (matching B3's own scale), against the already-frozen cost
gates in `frozen-config-v2.json`:

| Gate | Threshold | Measured | Result |
|---|---|---|---|
| Model-weight footprint | ≤ 150 MB | 86.2 MB | **PASS** |
| Peak RAM | ≤ 300 MB | 133.0 MB | **PASS** |
| Warm-query P95 | ≤ 3× B1 (373.6 ms) | 2.33 ms (0.02×) | **PASS** |
| Build/index time | ≤ 5× B1 (724.8 ms) | 366.6 ms (2.53×) | **PASS** |
| Vector-index footprint | ≈ ≤ 2 KB/note | 1.517 KB/note | **PASS** |

**All 5/5 operational acceptance gates pass.** No configuration was
changed to make any gate pass — these are the real numbers from the
real production code path, measured once and reported, per explicit
instruction.

## 8. Known limitations / technical debt (named, not resolved)

- **No logging facade for the B4.0 fallback warning.** `search_impl`'s
  `DenseV2`-unavailable fallback is silent (no log line) — this
  project has no existing logging convention to extend; adding one is
  deferred to whoever integrates this feature into a real calling
  context.
- **CI does not exercise `dense-retrieval`.** The feature defaults
  off and CI's existing commands don't pass `--features`/
  `--all-features`, so a regression in this code path would not be
  caught by CI until/unless a second job/matrix entry is added.
- **Vector-index storage is a flat array, no approximate index.**
  Deliberate (B4.1/the hybrid design doc both found brute-force
  adequate at this corpus scale); revisit only if corpus size growth
  makes it the real bottleneck — not estimated here.
- **ONNX Runtime is fetched via `ort`'s `download-binaries` feature,
  not vendored.** A real, one-time network dependency at setup time,
  not eliminated — B4.1's own deferred decision, unchanged.
- **No `.gitignore` entry yet for `.superbrain-model-cache/`** at a
  real vault root — not created since no real deployment has produced
  this directory outside of tests' own isolated temp directories.
- **Candidate-set-scoping architecture decision (B4.6) has a real
  cost:** `DenseV2` mode re-embeds the query and scores against every
  note the dense index holds, bypassing lexical pre-filtering entirely
  — this is correct per the evaluation's own methodology, but means
  `DenseV2`'s own candidate-generation cost does not shrink with a more
  selective lexical query the way `Lexical` mode's does. Not measured
  as a problem at this corpus scale (the smoke test's own numbers
  confirm this), but worth naming for a much larger future corpus.
- **No feature flag exists yet at a level above `Index`** (e.g. a
  config toggle a real calling application would use to decide
  whether to even attempt `DenseV2`) — `RetrievalMode` itself is the
  mechanism; wiring it into whatever calls `Index::search` today is
  outside B4's own scope (B4 is the library capability, not its
  integration into a specific caller).

## 9. Acceptance criteria — exact PASS/FAIL

1. DenseV2 uses really MiniLM ONNX production — **PASS** (B4.2, real
   model load/infer, hash-verified).
2. Works through the real `Index::search` — **PASS** (B4.6).
3. Lexical behavior stays backward compatible — **PASS** (B4.0's own
   byte-identical-to-pre-B4 tests; B4.6's fallback-parity tests).
4. Offline runtime complete — **PASS** (B4.8: `setup` is the only
   network-capable function; `load` only reads disk; structural, not
   just documented).
5. Vector index persistent and crash-safe — **PASS** (B4.3: atomic
   write, checksummed, 3 adversarial corruption tests + the B4
   closure's own real Windows file-lock fix).
6. Create/update/delete lifecycle correct — **PASS** (B4.4, B4.7
   restart test).
7. Model/config mismatch invalidates the index correctly — **PASS**
   (B4.7's dedicated model-identity-mismatch test).
8. Privacy parity verified on the production path — **PASS** (B4.5,
   two tests: structural+differential, and reclassification re-embed).
9. Fallback/error behavior deterministic — **PASS** (B4.0's frozen
   contract; B4.6's fallback-parity tests; transient vs. structural
   failures distinguished).
10. Real-vault integration tests pass — **PASS** (every B4.4–B4.7 test
    uses a real `storage::Vault`, never the evaluation harness's
    synthetic fixture).
11. Full workspace tests pass — **PASS** (Section 4).
12. Security/adversarial tests pass — **PASS** (storage corruption ×3,
    tamper rejection, model-mismatch, privacy structural+differential).
13. Performance operational gates pass — **PASS** (Section 7, 5/5).
14. No reading/rerun of the v2 held-out — **CONFIRMED.** No file under
    `eval/rag/v2-heldout/` was opened, read, or modified by any B4
    commit (verified: no commit in this branch touches that path).
15. No new retrieval tuning — **CONFIRMED.** No model, threshold,
    representation-semantics, or retrieval-config change was made;
    `frozen-config-v2.json` is read-only throughout B4, used only as a
    source of frozen constants (`HF_REPO`, pinned hashes, cost gates).

**All 15 acceptance criteria: PASS.**

## 10. Merge recommendation

**Recommend: merge `b4-dense-retrieval-production` into `main`**, once
the user explicitly authorizes it (not done as part of this closure).

Rationale: all 15 acceptance criteria pass with real, verified
evidence (not asserted); the feature is fully opt-in and additive
(zero behavior change for any existing caller); every real bug found
during implementation (tokenizer truncation, Windows file-lock,
mutation-testing gaps) was fixed and re-verified, not deferred; the
named limitations (Section 8) are genuine but none block correctness
or safety — they are follow-up integration/operational work for
whoever wires this capability into a real calling application, which
is explicitly outside B4's own scope (B4 delivers the library
capability; using it in `brainCompany` or elsewhere is a separate,
future task).

**This recommendation is not an authorization.** No merge, tag, or
push occurs as part of this document.

## 11. STOP

B4 is complete. This closure document, the branch's own commit
history, and the per-task design specs under `docs/superpowers/specs/
2026-11-15-b4-*.md` are the complete record. Next step, if authorized,
is the merge itself — not performed here.
