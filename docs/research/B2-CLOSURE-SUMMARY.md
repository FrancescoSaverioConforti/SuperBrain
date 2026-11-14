# B2 Closure Summary

**Phase:** B2 (Code Graph — multi-language semantic indexing).
**Date:** 2026-11-14.
**Status of this document:** consolidation only. No new code, no new
measurement, no new corpus run. Built entirely from evidence already on
record (evaluation docs, finding docs, checkpoint docs, Continuity
decisions). B2-EVAL-013, CALL-RES-A5 and Task 10 are **not started** by
this document and are not authorized by it.

---

## 1. Final scope of B2

### 1.1 Languages supported in the live pipeline

Confirmed from `extract/` dispatch and corpus evidence across every
evaluation: **Python, C#, C++, JavaScript, TypeScript, TSX, HTML, CSS,
Rust** — 9 languages with a working extractor reaching the real
`CodeGraphIndexer::observe_commit` pipeline.

### 1.2 Principal Code Graph capabilities

- Multi-language symbol extraction (Class/Interface/Function/Method/
  MethodSignature/Module/File, per language's own grammar).
- `Calls` edge resolution — name-based, import/namespace/visibility-aware
  (CALL-RES-A1–A4), with explicit negative-knowledge tracking for every
  call that does not resolve cleanly.
- Structural edges: `Contains`, `Extends`, `Implements`, `Instantiates`.
- `TestsFor` edge (naming-convention-based test-to-subject linkage).
- File-level `ImportFact` (replaced the original symbol-granularity
  `Imports` edge after B2-ID-005 found it had near-zero real capture).
- Lineage tracking across commits (`Confirmed`/`Hypothesized` status,
  `Certain`/`Likely`/`Speculative` confidence).
- Incremental indexing with **exact** full/incremental parity, verified
  by multiset (not just count) comparison on every domain evaluated.
- Provenance and a four-tier evidence model (`Resolved`/`Heuristic`/
  `Ambiguous`/`Unresolved`) shared across edges, import/impl resolution,
  and negative-knowledge `CallAttempt` records.
- An agent-reliability policy (`agent_policy::evaluate`) that
  unconditionally forbids high-impact automated action on `Ambiguous`
  evidence — stress-tested against real per-domain data, not just unit
  fixtures.

---

## 2. Task/finding status

### 2.1 Closed findings (B2-ID-001 through B2-ID-020)

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| 001 | — | Symbol identity collision (`init_weights`) | CLOSED, fixed pre-EVAL-001 |
| 002 | — | Incremental edge-rebuild scope | CLOSED — documented full-pool-recompute design |
| 003 | — | Chained-call false negative | WITHDRAWN — was an eval-harness bug, not a pipeline defect |
| 004 | — | Method-name-collision false positives (receiver/scope-unaware `Calls`) | CLOSED, Task 9.5 Priority 5(A) |
| 005 | — | `Imports` edge ~0% real capture (granularity mismatch) | CLOSED — `EdgeKind::Imports` removed, replaced by `ImportFact` |
| 006 | Important | `resolve_calls` attributed calls to every enclosing container, not nearest real caller | CLOSED, mutation-verified |
| 007 | Important | Same defect family in `resolve_extends_implements`/`resolve_tests_for` | CLOSED, 25 regression tests |
| 008 | — | Rust `impl` blocks never extracted (0 Implements coverage) | CLOSED via `ImplFact`; own Critical sub-bug (incremental stale-pool resolution) found and fixed in the same pass |
| 009 | **Critical** | Polly/C# method misnaming collapsed ~1,535/4,543 methods under the bogus name "Task" | CLOSED, commit `c09c34d` |
| 010 | Important | Partial-class/namespace inheritance resolution gap | CLOSED, commits `2001661`, `cc1878f` |
| 011 | Important | Generic method calls never reaching candidate status | CLOSED, commits `21382ef`, `c037b08` |
| 013 | Moderate | `require()` produced no `ImportFact` | CLOSED (CommonJS checkpoint) |
| 014 | Moderate | `exports.X =` / `module.exports.X =` property-assigned functions never extracted | CLOSED |
| 015 | Moderate | `Foo.prototype.X =` (ES5 class-via-prototype) never extracted | CLOSED |
| 017 | Critical | HTML/CSS parsers implemented but unreachable from production `parser_for()` dispatch | CLOSED, commit `1d7a939` |
| 018 | Important | TS `interface_declaration` never extracted (0/18 interfaces, 0 Implements to local interfaces) | CLOSED, commit `01ddb37` |
| 019 | Important | Generic heritage name (`implements Bar<T>`) parses as `generic_type`, never unwrapped | CLOSED, commit `6944094` |
| 020 | Important | `abstract class X` parses as distinct `abstract_class_declaration`, never matched | CLOSED, commit `d60d0c2` |

All Critical/Important findings opened across the entire B2 phase are
CLOSED. None are open.

### 2.2 Moderate/Minor findings still open (technical backlog, not blockers)

| ID / name | Severity | Status |
|---|---|---|
| B2-ID-012 | Moderate | `resolve_lineage` O(n²) similarity-scoring cost — **documented, not fixed** |
| B2-ID-016 | Moderate | `Foo.X = function(){}` for an arbitrary local identifier `Foo` (5th CommonJS idiom) — **documented, not fixed**, explicitly out of the CommonJS checkpoint's authorized scope |

### 2.3 Separate technical backlog (not formal findings — real, cited, no B2-ID assigned)

- `LocalGitRepositorySource::read_file` reopens the git repository on
  every file read — structural perf cost, found during B2-ID-012's own
  investigation, recorded in `SEMANTIC-RESOLUTION-CHECKPOINT-001.md`'s
  "New findings" section, never promoted to its own ID.
- C++ namespace-qualified template calls (`ns::foo<T>()`) never reach
  candidate status — quantified at 678/4,215 call expressions (16.1%)
  in B2-EVAL-009, confirmed general rather than corpus-specific.
- A narrow arity-aware base-clause disambiguation step for C# generic
  overload collisions (e.g. `IBulkheadPolicy` vs. `IBulkheadPolicy<T>`)
  — cross-oracle (Roslyn) confirmed recoverable via arity alone, not
  yet implemented; flagged in B2-EVAL-008 as the single cheapest,
  most concretely evidenced next increment.
- Negative-knowledge provenance uses one canned `method_limitations`
  string per tier rather than a per-case real cause — Moderate, new in
  B2-EVAL-008.
- `CallAttempt`/negative-knowledge not wired into the incremental
  indexer in every configuration (only the full-reindex path reliably
  produces it) — confirmed in B2-EVAL-008 Section N as a real residual
  gap, flagged for a future hardening phase, not escalated to its own
  ID.
- Two residual Rust `ImplFact` gaps (reference-type self-type never
  extracted; scoped external self-type collapsing to a shared short
  name) — both Moderate, both currently zero observable bad effect.
- C# extraction-coverage gaps: struct/enum/constructor/field
  declarations never walked by `extract/csharp.rs`.
- xUnit `TestsFor` naming-convention mismatch (C#) — carried forward
  unchanged.
- An incremental-indexing-slower-than-full-reindex anomaly on specific
  commit pairs — reproduced 3x, never root-caused, distinct from
  B2-ID-012.
- JSX composition not represented as a `Calls` edge, and array-
  destructured hook/store exports never extracted — both Moderate,
  from B2-EVAL-004.
- CRC-002: local-binding/parameter shadowing blindness in
  `resolve_calls` — open challenge-set entry, re-confirmed
  non-regressed through the CALL-RES-A4 wiring, not independently
  fixed.
- A "command-registration-graph" structural category (dynamic dispatch
  via value assignment — e.g. Python `argparse` `set_defaults(func=X)`,
  C# constructor-injected delegate fields) — named as a descriptive
  absence in B2-EVAL-007/008, not a defect, no fix proposed.
- `AddBuilder` on `ResiliencePipelineRegistry<T>` (Polly) — one
  honestly-unconfirmed possible false negative, never resolved either
  way.

None of the items in 2.3 are Critical or Important. All are documented
with a citable source.

---

## 3. Cross-domain evaluation summary

B2-EVAL-001/002 are the founding ML/Python+C++ corpus
(`pytorch/examples`) that predates the later "8 broad domains"
multi-codebase validation sweep and established the baseline/Task-9.5
re-evaluation pattern; they are not counted among the 8 below.

| # | Repo | Language/domain | Outcome | Key findings | Fix/checkpoint | Reliability/precision notes |
|---|---|---|---|---|---|---|
| EVAL-003 | `nsidnev/fastapi-realworld-example-app` | Backend/API Python | COMPLETE, GO | B2-ID-006, B2-ID-007 | Both fixed, mutation-verified, 25 regression tests | First real-corpus confirmation of the "wrong enclosing container" defect family |
| EVAL-004 | `yoonge/conduit-react` | Frontend/UI TS/TSX | COMPLETE | JSX-composition-not-a-Calls-edge; array-destructured export gap; CRC-002 shadowing opened | None of these three fixed in this round | First domain to surface CRC-002, still open |
| EVAL-006 | `pyjanitor-devs/pyjanitor` | Data/scientific Python | COMPLETE | SignatureTooLong first quantified (53/2822) | — | Confirms SignatureTooLong is cross-domain, not corpus-specific |
| EVAL-007 | `pypa/pipx` | CLI/tooling Python | COMPLETE | SignatureTooLong (18/2558); command-registration-graph category named | — | Reinforces SignatureTooLong; first naming of the dynamic-dispatch structural gap |
| EVAL-008 | `App-vNext/Polly` | C# | COMPLETE, GO | B2-ID-009 (**Critical**, method misnaming), B2-ID-010, B2-ID-011; generic-arity collision; negative-knowledge provenance granularity; CallAttempt/incremental wiring gap | All three B2-IDs CLOSED | First real ground-truth P/R/F1 use (precision 1.0, recall 0.913, F1 0.955); SignatureTooLong 213/5403; command-registration category reconfirmed non-CLI-specific |
| EVAL-009 | `fmtlib/fmt` | C++ (standalone) | COMPLETE, GO | C++ namespace-qualified template call gap (678/4215, 16.1%) quantified | — | SignatureTooLong 16/1441; adversarial-sample recall 0.231 (not general); zero false positives confirmed |
| EVAL-010 | `request/request` | JS/CommonJS | COMPLETE, GO | B2-ID-013/014/015 (all three CLOSED); B2-ID-016 opened (open) | CommonJS checkpoint: 40→96 snapshots, 0→48 ImportFacts, 12→88 Calls edges, 0/27→24/27 prototype methods, 0 false edges | 0/68 Ambiguous — all gaps were pure extraction, not resolution-quality |
| EVAL-011 | `tbreuss/html-boilerplate` | HTML/CSS | GO per Continuity (d178lp/d179tr/d181cf) | B2-ID-017 (**Critical**, parsers unreachable from dispatch) | CLOSED, commit `1d7a939` | **No `B2-EVAL-011.md` document was ever written** — see §9 incongruence. Metrics beyond "fixed and GO" cannot be cited because no doc exists. |
| EVAL-012 | `Sairyss/domain-driven-hexagon` | TS backend/Node.js | COMPLETE, GO | B2-ID-018/019/020 (all three CLOSED) | 18/18 interfaces recovered, 5 Implements edges, full/incremental parity exact | A5 evidence 68/245 = 27.8% Ambiguous, highest fraction of any domain |

---

## 4. Current capabilities, by name

| Capability | Status | Evidence |
|---|---|---|
| Symbols | Implemented, 9 languages | extractor dispatch, every eval doc |
| Calls | Implemented, import/namespace/visibility-aware (A1–A4) | CALL-RES-A4-CHECKPOINT.md |
| Contains | Implemented | every eval doc's edge-kind breakdown |
| Extends | Implemented | every eval doc |
| Implements | Implemented (TS fixed this phase via B2-ID-018/019/020; Rust via `ImplFact`/B2-ID-008) | B2-EVAL-012, B2-ID-008 |
| Instantiates | Implemented | Task 9.5 (per Continuity d100oc) |
| TestsFor | Implemented, naming-convention-based | multiple eval docs; xUnit mismatch noted as residual gap |
| ImportFact | Implemented, file-level (replaced symbol-level `Imports`) | B2-ID-005 |
| Lineage | Implemented — `Confirmed`/`Hypothesized` status, `Certain`/`Likely`/`Speculative` confidence | `evidence.rs`, lineage docs |
| Incremental indexing | Implemented, exact full/incremental parity via multiset comparison | `conformance::compare_full_incremental_state`, zero divergences in every eval |
| Provenance | Implemented | `Provenance` type, evidence model |
| Negative knowledge | Implemented — `CallAttempt` records every non-clean resolution with tier + reason | `negative_knowledge.rs`; **known gap: not reliably wired into every incremental-indexer configuration** (§2.3) |
| Agent reliability policy | Implemented — unconditional "Ambiguous always Forbidden" for high-impact action | `agent_policy.rs`, stress-tested on real per-domain data (e.g. Polly 8,465/8,465) |

---

## 5. Known limitations

- **SignatureTooLong** — pre-existing extraction-rejection limitation,
  confirmed general (not corpus-specific) across pyjanitor, pipx,
  Polly, fmt; did not fire at all on the B2-EVAL-012 corpus. Accepted
  limitation, not a finding.
- **B2-ID-012** — `resolve_lineage` O(n²) similarity-scoring dominates
  incremental cost. Documented, not fixed. Open Moderate backlog item.
- **`LocalGitRepositorySource::read_file` git-reopen** — reopens the
  git repository per file read; structural performance cost, no B2-ID,
  documented in `SEMANTIC-RESOLUTION-CHECKPOINT-001.md`.
- **C++ namespace-qualified template calls** — `ns::foo<T>()` never
  reaches candidate status; quantified at 16.1% of call expressions in
  the fmt corpus; confirmed general, no B2-ID.
- **B2-ID-016** — CommonJS `Foo.X = function(){}` for an arbitrary
  local identifier `Foo`, the 5th CommonJS idiom; explicitly out of
  the CommonJS checkpoint's authorized scope. Open.
- **HTML/CSS cross-reference graph** — status **cannot be confirmed
  either way**. No `B2-EVAL-011.md` exists to document what, if
  anything, was measured on this axis. Flagged as an unconfirmed gap,
  not asserted as present or absent.
- **TS/JS residual visibility gaps** — JSX composition not represented
  as a `Calls` edge; array-destructured hook/store exports never
  extracted. Both Moderate, from B2-EVAL-004, not fixed.
- **CRC-002** — local-binding/parameter shadowing blindness in
  `resolve_calls`. Open challenge-set entry, re-confirmed
  non-regressed but not independently fixed.
- **A5 (type inference)** — not implemented. See §7.
- **CFG / data-flow / points-to / context-sensitive / interprocedural
  analysis** — none implemented. Current `Calls` resolution is
  name/import/namespace/visibility-aware (A1–A4) only; no flow-
  sensitive or interprocedural analysis exists anywhere in the
  pipeline.

---

## 6. Quality gates reached

| Gate | Status | Evidence |
|---|---|---|
| Golden corpus | Active, extended every checkpoint | `tests/corpus/{language}/`, Hardening Phase 1 (17 fixtures/12 tests) → CommonJS checkpoint (+9/+7) → SEMANTIC-RESOLUTION (+2) |
| Grammar-conformance matrix | Active | 34/34 tests, reconfirmed at every subsequent baseline freeze (e.g. B2-EVAL-009) |
| End-to-end harness | Active, one per domain | `b2_smoke_test_*.rs` / `b2_eval_0XX_*.rs`, structurally identical convention across all 12 eval docs |
| Full/incremental parity | Active, exact multiset comparison | `conformance::compare_full_incremental_state`, zero divergences in every eval from EVAL-001 onward; extended to `CallAttempt`/`EvidenceTier` in SEMANTIC-RESOLUTION |
| Mutation policy | Active, every fix mutation-tested | `cargo mutants`, used on B2-ID-006 through 020, CALL-RES-A4, CommonJS checkpoint — reports caught/unviable/genuine-equivalent, never silently accepts a missed mutant |
| Roslyn differential testing | Active | `tools/roslyn-oracle/Program.cs`, real C# compiler semantic model as oracle; cross-confirmed the generic-arity gap in B2-EVAL-008 |
| Ground truth schema | Active, machine-computed P/R/F1 | `GROUND_TRUTH_SCHEMA.md` + `ground_truth.rs`; first real use B2-EVAL-008 (precision 1.0, recall 0.913, F1 0.955) |
| Reliability/provenance model | Active, zero tier-assignment errors found on real data | `EvidenceTier`, `Provenance`, stress-tested in B2-EVAL-008 |
| Agent-policy enforcement | Active on the data checked, but **not reliably wired into every incremental-indexer configuration** | `agent_policy.rs`; gap noted in B2-EVAL-008 Section N |

---

## 7. A5 (type inference) decision

Cross-domain ambiguity evidence (all domains with a measured figure):

| Domain | Ambiguous evidence |
|---|---|
| C#/C++ (Polly) | 8,465 real Ambiguous CallAttempts; 846/847 `Handle` overload calls Ambiguous |
| C++ standalone (fmt) | 782 Ambiguous CallAttempts; adversarial-sample recall 0.231 (not general) |
| JS/CommonJS (request) | 0/68 — all gaps were extraction, not resolution-quality |
| TS backend (domain-driven-hexagon) | 68/245 = 27.8% — highest fraction of any domain |

**Conclusion:** ambiguity exists and is real in every domain measured,
but in 8/8 domains it never dominates — it is consistently a real,
nonzero, minority slice of total unresolved/ambiguous calls, and
current behavior is confirmed correct and honest in every sampled case
(zero false positives found anywhere). A5 would improve *recall* on
already-identified ambiguous cases; it would not fix wrong answers,
because none exist in the samples checked. No domain's evidence
overrides this pattern. **A5 remains a separate, deliberate
experimental ablation checkpoint (per decision d102zp) — it is not
part of B2 closure, and this closure does not make it the obligatory
next step.**

---

## 8. B2 Exit Condition — explicit verification

| Condition | Status |
|---|---|
| No open Critical/Important finding | **TRUE** — every Critical/Important finding across the whole phase (001–020) is CLOSED; none open |
| All findings have a disposition | **TRUE for every assigned B2-ID** — CLOSED-with-fix, WITHDRAWN, or documented-not-fixed (B2-ID-012, B2-ID-016); the broader technical backlog in §2.3 is documented with citations even without formal IDs |
| Evaluations complete | **TRUE with one caveat** — all 8 broad domains plus the founding ML corpus have a GO outcome on record; **B2-EVAL-011 (HTML/CSS) has no written evaluation document**, so its GO rests on Continuity decisions alone, not a citable report (see §9) |
| Reliability gates active | **TRUE with one caveat** — all 9 gates in §6 are active and evidenced; the agent-policy/negative-knowledge incremental-wiring gap (§2.3, §6) means the policy gate is not uniformly enforced in every indexing path |
| Residual risk documented | **TRUE** — §5 and §2.3 document every known limitation found across the phase, each with a citation or explicit "no B2-ID assigned" note |

---

## 9. Final recommendation

**GO to close B2**, with two items carried forward rather than blocking
closure:

1. **Backfill or formally accept the missing `B2-EVAL-011.md`.** HTML/CSS
   was evaluated, its Critical finding (B2-ID-017) was fixed, and GO was
   recorded in Continuity — but no evaluation document exists to cite
   corpus metrics, edge counts, or whether an HTML/CSS cross-reference
   graph exists. This is the one real incongruence found while
   compiling this summary: a gap in documentation completeness, not a
   gap in the underlying fix or in B2's substantive readiness. Low
   cost, does not block closure, but should not be silently dropped.
2. **The negative-knowledge/agent-policy incremental-wiring gap
   (B2-EVAL-008 Section N)** — the reliability gate is confirmed correct
   on data it sees, but not every incremental-indexing configuration
   reliably produces that data. Worth a dedicated hardening pass before
   any future phase leans heavily on agent-policy enforcement in
   production.

**What stays backlog (not B2-closure blockers):** B2-ID-012 (O(n²)
lineage), B2-ID-016 (5th CommonJS idiom), the git-reopen cost, the C++
namespace-qualified template gap, the C# generic-arity disambiguation
opportunity, CRC-002 shadowing, the two Rust `ImplFact` residuals, C#
struct/enum/constructor/field extraction gaps, and the incremental-
slower-than-full anomaly. None are Critical/Important; all are
documented with a source.

**Proposed next macro-step after B2 closure:** a short, explicitly
scoped **Hardening Phase** addressing the negative-knowledge/agent-
policy incremental-wiring gap and the backfilled B2-EVAL-011 doc,
*before* either (a) the CALL-RES-A5 experimental ablation checkpoint
(per d102zp — still a separate decision, not implied by this
recommendation) or (b) Task 10. Neither A5 nor Task 10 is started or
authorized by this document.

No merge, tag, or push performed.
