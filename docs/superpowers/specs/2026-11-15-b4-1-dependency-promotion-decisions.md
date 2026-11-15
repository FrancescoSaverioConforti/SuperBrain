# B4.1 — Production Dependency Promotion: Decisions

**Status: decisions frozen before any `Cargo.toml` change.** Resolves
the open Section 6a questions the production-integration design and
its review deferred to this task.

**Real baseline verified before any decision below** (not assumed):
`cargo tree -p tokenizers -i` and `cargo tree -p ort -i` confirm `ort`,
`tokenizers`, `ndarray` are `[dev-dependencies]` of `api` only today —
zero production dependency anywhere in the workspace. `.github/workflows/ci.yml`
runs on `ubuntu-latest` (not Windows), `cargo build/test --workspace`,
`cargo clippy --workspace -- -D warnings` (**warnings are errors**),
and `cargo deny check` (licenses + advisories + bans + sources). `deny.toml`
allows `["Apache-2.0", "MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "Unicode-3.0"]`
and has one existing advisory exception
(`RUSTSEC-2024-0449`, `unsafe-libyaml`, documented and re-review-dated).

## 1. Real finding: `cargo deny check` already fails today, on `main`, unrelated to B4

Running `cargo deny check` against the current tree surfaces two
`advisories` failures:

- `RUSTSEC-2026-0459` (`libsqlite3-sys`/`rusqlite`, a real CVE) — this
  dependency chain runs through `storage`/`vault-security`'s own
  `rusqlite` use, has **nothing to do with Priority 4/B4**, and is
  present on `main` today regardless of anything this task does
  (confirmed: `cargo tree -i -p libsqlite3-sys` shows no path through
  `tokenizers`/`ort`/`ndarray`). **Out of scope for B4** — flagged here
  for visibility only, not fixed by this task, since fixing it would
  mean upgrading `rusqlite` across the whole workspace, an unrelated
  change this task has no mandate to make.
- `RUSTSEC-2024-0436` (`paste`, unmaintained) — this chain runs through
  `tokenizers v0.23.2`'s own `(dev)` dependency tree today, **already**,
  before this task promotes anything. **This IS directly relevant to
  B4.1**, because `tokenizers` is one of the three crates this task
  promotes to production — promoting it does not introduce this
  advisory (it already exists today, as a dev-dependency), but it does
  change its classification from "affects only `cargo test`/examples"
  to "affects the shipped binary," which is worth stating honestly
  rather than treated as new.

**Decision, executed:** added a `deny.toml` exception for
`RUSTSEC-2024-0436`, following the project's own existing convention
(`RUSTSEC-2024-0449`'s own entry, with a reason and a re-review date).
Verified with real commands, both before and after the `Cargo.toml`
change: `cargo deny check` (feature off, default) shows only the
pre-existing `RUSTSEC-2026-0459` sqlite advisory, confirmed via
`cargo tree -i -p libsqlite3-sys` to have no path through `tokenizers`/
`ort`/`ndarray` at all — present on `main` today, unrelated to B4,
out of scope for this task to fix. `cargo deny --all-features check`
(the configuration that actually resolves `tokenizers`→`paste`) shows
the same single pre-existing sqlite advisory plus nothing else,
confirming the new exception entry resolves `RUSTSEC-2024-0436`
exactly as intended, with `licenses ok`/`bans ok`/`sources ok` across
both invocations once Section 10's own license-allowlist fix (below)
was also applied.

## 2. Feature flag

**Decision: `dense-retrieval` Cargo feature on the `api` crate, default
OFF.** Matches the design doc's own candidate name exactly.
`crates/index` does **not** get this feature — `RetrievalMode::DenseV2`
as an enum variant already exists unconditionally in `index` (B4.0,
cheap, carries no dependency weight by itself), but the actual ONNX/
embedding runtime implementation lives in `api` (Section 3 below
explains why), so that is where the feature-gated dependencies belong.
A build with the feature disabled has the exact same dependency graph
`api` has today (zero ONNX/embedding exposure); a build with it
enabled links `ort`/`tokenizers`/`ndarray`.

## 3. Crate ownership

**Decision: the dense embedding runtime lives in `crates/api`, not
`crates/index`.** Rationale, from reading the real dependency
structure: `index`'s own `Cargo.toml` has exactly two production
dependencies (`chrono`, `chrono-tz`) plus `domain`/`storage` — it is
deliberately the thinnest possible crate, with every even-ablation-only
addition (BM25 param overrides, identifier boost, status tie-break)
implemented as pure Rust with zero new external dependencies. Adding a
native ONNX Runtime dependency to `index` would be a much larger
architectural change than anything that crate has taken on before,
for every consumer of `index` (including `codegraph-index`, which
depends on `domain` but not `index` directly — confirmed via the
workspace member list — so this specific concern is narrower than
first appears, but the principle holds: `index` has never carried a
native/FFI dependency, and introducing one is not something to do
implicitly as a side effect of a feature flag on a different crate).
`api` already carries the `[dev-dependencies]` versions of `ort`/
`tokenizers`/`ndarray` today (Priority 4's own evaluation harness) —
promoting them in place, in the crate that already has them, is the
smaller, more consistent change. The actual dense-scoring
implementation will be a new `api::dense` module (B4.2) that `index`
calls into through a trait object or function pointer `Index` accepts
at construction time — the exact mechanism is B4.2's job, not decided
here, but the ownership boundary (runtime lives in `api`, `index`
only knows about an abstract capability) is decided now, consistent
with how `index` already has zero concrete dependency on any specific
embedding implementation.

**Correction found while writing this decision:** this means B4.0's
own `RetrievalMode::DenseV2` variant, which lives in `index`, cannot
itself call into `ort` directly — `index` has no path to the embedding
runtime without taking on the dependency B4.1/this section just
decided against. This is resolved by `Index` accepting the dense
scorer as an injected capability (e.g. a trait implemented in `api`,
passed in at `Index` construction or via a new builder), not by
`index` depending on `api` (which would be a cyclic/backwards
dependency — `api` already depends on `index`, so `index` cannot
depend back on `api`). **This specific mechanism is B4.2's/B4.6's task
to design and implement; this section only fixes the ownership
boundary that makes that design necessary, flagged honestly as a
consequence discovered while making this decision, not pre-solved
here.**

## 4. Target/platform support

**Decision: keep `ort`'s `download-binaries` feature for now, same as
the evaluation harness.** Vendoring a per-platform ONNX Runtime binary
is strictly more packaging work (per-platform binary management,
license/attribution bookkeeping for a vendored binary) with no
offsetting benefit at this project's current scale (local-first,
single-developer-operated, no release pipeline distributing prebuilt
binaries to end users yet — confirmed: `deny.toml`/`ci.yml` show a
test-and-check pipeline, not a release/publish one). **Revisit this
decision if/when the project gains an actual release/distribution
pipeline** — not before, since vendoring now would be solving a
packaging problem this project does not have yet.

**Consequence, stated honestly:** this means "offline after first
fetch" (Section 6 of the design doc) depends on `ort`'s
`download-binaries` fetch happening during the SAME explicit,
user-visible setup step that fetches the model/tokenizer weights
(B4.8's job), not silently during `cargo build`. This is a real
constraint this decision creates, not a detail to discover later.

## 5. Windows build

**Not yet verified in production form** — the evaluation harness
confirmed `ort`'s `download-binaries` feature builds and runs on this
exact Windows machine (already proven throughout B3-TUNE-v2's Priority
4 work), but CI itself runs on `ubuntu-latest` only. **Decision:**
verify the production build (once B4.2 produces one) on this Windows
machine directly, the same way Priority 4's evaluation harness already
was — do not infer Windows support from CI passing on Linux alone,
since CI never exercises the Windows path at all. This is restated as
an explicit B4.2 acceptance item, not assumed satisfied by this
decision record.

## 6. Dynamic/static linking

**Decision: dynamic (the `ort` default under `download-binaries`),
unchanged from the evaluation harness.** No evidence or requirement
has surfaced for static linking; revisit only if a real packaging
problem (e.g. a missing shared library at a user's runtime) is
observed, not preemptively.

## 7. Artifact/model discovery path

**Decision: `.superbrain-model-cache/` at the vault root** (not
`.priority4-model-cache/`, which was explicitly an evaluation-only
convention per the design doc's own Section 6a) — mirrors the existing
`.pii-store/` convention already used at the vault root
(`vault_security::PiiStore::open`, confirmed in
`crates/index/tests/b1_search.rs`'s own `Fixture::new()`), so model
artifacts live alongside the other vault-root-scoped, non-note state
this project already has a convention for, rather than inventing a
new top-level location. Exact subpath layout (per-model subdirectory,
naming) is B4.8's job; this decision only fixes the root location.

## 8. Release pipeline impact / CI implications

**Decision: do not change `.github/workflows/ci.yml` as part of B4.1.**
Feature-gating the dependency behind `dense-retrieval`, default off,
means `cargo build/test --workspace` and `cargo clippy --workspace`
(both run without `--all-features` in the existing CI config) do not
exercise the new dependency at all — CI's build/test/clippy times are
unaffected by this promotion unless/until CI is separately updated to
also build with `--features dense-retrieval`. **This is itself a
limitation worth stating plainly: CI will not catch a regression in
the dense-retrieval path until CI is updated to build/test it.**
Flagged as known technical debt for B4's own closure document, not
silently resolved here — adding a second CI job/matrix entry for the
feature-enabled build is a reasonable follow-up but is not required
to ship B4 itself, since the feature defaults off and CI's default
build already covers the unchanged, default behavior.

## 9. Binary size

Not measured yet — will be measured once B4.2 produces a buildable
`dense-retrieval`-enabled binary, as a real number, not estimated here.

## 10. Licensing/attribution

**Correction, found while actually running `cargo deny --all-features
check` (not inferred from the three top-level crates' own licenses
alone, which was this section's original, incomplete check):** `ort`
itself is MIT, `tokenizers` is Apache-2.0, `ndarray` is MIT/Apache-2.0
dual — but `ort-sys` (a **build-dependency** of `ort`, so it does not
show up in an ordinary `cargo tree` of runtime dependencies) pulls in
`ureq`'s own transitive dependencies `webpki-root-certs` and
`webpki-roots`, both licensed `CDLA-Permissive-2.0` — **not** on the
existing `deny.toml` allowlist. This only surfaces when the feature is
actually enabled (`cargo deny check` alone, feature off, does not
resolve this part of the graph at all — confirmed: it shows
`licenses ok` with the feature off, `licenses FAILED` with
`--all-features`). **Decision: add `CDLA-Permissive-2.0` to
`deny.toml`'s license allowlist.** This is a real, permissive,
OSI-adjacent data/content license (used here for Mozilla's bundled
root certificate list, not for any code logic) — adding it is the
correct fix, not an exception needing a re-review date the way an
advisory does, since this is a license-policy addition, not a
time-bounded risk acceptance.

Attribution text itself (crediting these crates and the MiniLM model
in whatever distribution documentation accompanies this feature)
remains B4.8's job, per the design doc's own Section 6.

## 11. Failure mode when runtime/model artifact is missing

Unchanged from B4.0's frozen fallback contract (Section 5 of that
freeze document) — restated here only to confirm this decision record
does not weaken it: a `dense-retrieval`-enabled build missing its
acquired artifacts falls back to `Lexical` with a loud warning, never
a hard crash. Implementation is B4.2/B4.6's job.

## 12. Task ordering for the actual `Cargo.toml` change

1. Add the `dense-retrieval` feature to `crates/api/Cargo.toml`,
   moving `ort`/`tokenizers`/`ndarray` from `[dev-dependencies]` to
   `[dependencies]` with `optional = true`, wired to the feature.
2. Add the `deny.toml` exception for `RUSTSEC-2024-0436` at the same
   commit, with the same reason/re-review-date discipline the existing
   `RUSTSEC-2024-0449` entry uses, so `cargo deny check` keeps passing
   once the dependency's production relevance is real, not just a
   dev/example concern.
3. Run `cargo build --package api` (feature disabled, default) and
   `cargo build --package api --features dense-retrieval` (feature
   enabled) to confirm both configurations compile, as this task's own
   acceptance criterion.
4. Run `cargo deny check` again to confirm it passes with the new
   exception entry (modulo the pre-existing, out-of-scope
   `RUSTSEC-2026-0459` sqlite advisory, which this task does not fix).

## 13. STOP — decisions frozen

Section 3's "crate ownership" decision surfaces a real design
consequence for B4.2/B4.6 (how `Index` accepts an injected dense
scorer without `index` depending on `api`) — flagged honestly, not
pre-solved here, consistent with the instruction to document findings
rather than silently work around them. Everything else above is a
final decision for B4.1's own implementation to execute next.
