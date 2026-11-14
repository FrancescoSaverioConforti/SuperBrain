# B2-EVAL-007: multi-codebase validation, CLI / tooling domain

**Evaluation ID:** B2-EVAL-007
**Status:** **COMPLETE.**
**Date:** 2026-11-11.
**SuperBrain commit:** branch `b2-code-graph` (post B2-ID-008 ImplFact
fix, post B2-EVAL-006). No production code was modified for this
evaluation — only a new, domain-specific throwaway harness was added
(`crates/codegraph-index/examples/b2_smoke_test_cli_tooling.rs`), plus
one throwaway real-extractor probe used to directly confirm two
specific byte-size claims (written, run, and deleted during this
evaluation — never committed, consistent with how B2-EVAL-006's own
independent review verified its central claims).
**External repository:** `pypa/pipx`
(https://github.com/pypa/pipx), MIT license ("Copyright (c) 2018 Chad
Smith and contributors"), confirmed by direct read of the repository's
own `LICENSE` file at the repository root. A real, actively-maintained
Python CLI tool (installs and runs Python applications in isolated
environments), chosen per the user's explicit CLI/tooling domain
requirements: a real multi-file package (`src/pipx/`, 55 `.py` files),
a genuine `src/pipx/commands/` directory with one file per real
subcommand (`install.py`, `uninstall.py`, `upgrade.py`, `inject.py`,
`run.py`, `list_packages.py`, and 18 more), a real `argparse`-based CLI
with multiple real subcommands wired through `subparsers.add_parser(...)`
and `parser.set_defaults(func=...)`, and 1454 real commits of history.
Not a toy script and not a thin wrapper — `pipx` is a widely-used,
PyPA-maintained real-world tool. Cloned read-only into a fresh,
isolated scratchpad directory; deleted after this evaluation's data
was captured (confirmed deleted, no residual clone left on disk).
**External commits:** commit A `2299c0124283a6f8a8c6b38f326a236ea4221014`
("fix(transaction): keep the running copy in trash"), commit B
`c923d239fef3ccb1c00c49aae87f8855f85f62c1` ("fix(inject): load own code
on self-inject and runpip") — a real, multi-file commit introducing a
genuine new cross-file relationship (a new function, `runs_from_venv`,
added to `src/pipx/commands/transaction.py` and imported into BOTH
`src/pipx/commands/inject.py` and `src/pipx/commands/run_pip.py`),
exercising incremental indexing against a real cross-file dependency
addition, not a trivial one-line diff.
**Configuration:** the frozen baseline configuration, unchanged from
every prior B2 domain. No algorithm was changed to produce this
evaluation's numbers.
**Methodology:** drove the real production pipeline via a new
domain-specific harness,
`crates/codegraph-index/examples/b2_smoke_test_cli_tooling.rs`,
structurally mirroring the prior six domain harnesses for every generic
measurement section, with a new Section C2 (call-graph vs. command-
registration-graph distinction — this domain's own central, concrete
demonstration of a structural limitation this project's own protocol
asked to quantify, not merely assert). Two specific byte-size claims
in Section H (Q3/Q4) were verified directly via a throwaway real-
extractor probe (calling `extract_symbols_reporting_rejections`
directly against the real cloned source, then deleting the probe) —
a lesson applied directly from B2-EVAL-006's own independent review,
which found manually-estimated byte sizes in that evaluation to be
unreliable; this evaluation does not repeat that mistake. Raw output
for the FINAL harness run (confirmed to be the one actually cited
below, not a stale intermediate run — a second lesson applied directly
from B2-EVAL-006's own independent review, which found a timing-figure
mismatch between the document and its own cited evidence file) is
preserved at `docs/research/evidence/b2-eval-007-pipx-run.txt`.

## A. Symbol / structural coverage

| Metric | Value |
|---|---:|
| Total files in tree | 251 |
| Files supported by parser | 140 |
| Files parsed | 140 |
| Parse failures | 0 |
| Files with tree-sitter error nodes | 0 |
| Raw candidates extracted | 2540 |
| Extraction rejections (oversized name/signature) | 18 |
| Snapshots promoted | 2540 |
| Promotion failures | 0 |
| `Function` count | 2311 |
| `Class` count | 129 |
| `Field` count | 100 |
| Python files with ≥1 import statement | 130 |

No parse failures, no promotion failures, no duplicate
`SymbolSnapshotId` or duplicate `(from,to,kind)` edge triples detected.

### A1. SignatureTooLong — is this gap pyjanitor-specific, or general? ANSWERED: general

Per explicit instruction to determine whether B2-EVAL-006's own
SignatureTooLong finding was specific to `pyjanitor` or a general
pattern: **of the 18 extraction rejections in this entirely different
corpus, ALL 18 are `SignatureTooLong`** (confirmed by the same explicit
per-rejection-type counter used in B2-EVAL-006's own corrected harness).
**This confirms the gap is GENERAL, not pyjanitor-specific** — it
recurs, independently, in a second, structurally unrelated real
repository (a CLI tool vs. a data-cleaning library), hitting a
proportionally SMALLER but still non-trivial fraction of candidates
(18/2558 = 0.70% here, vs. 53/2875 ≈ 1.84% in `pyjanitor` — both real,
measured denominators, not comparable to each other as a precision
figure but both confirming the mechanism recurs).

**Which `SymbolKind` does it hit:** confirmed, via the same mechanism
as `pyjanitor`'s own case, that `Function` candidates are the kind
affected in this corpus too (both this evaluation's own Section H Q3/Q4
targets — `install`, `inject_dep` — and the raw file list below are
all free functions, not classes/fields). **Which files it hits (full
list, not a sample, since there are only 18):** `scripts/release.py`,
`src/pipx/backends/pip.py`, `src/pipx/backends/uv.py`,
`src/pipx/commands/inject.py`, `src/pipx/commands/install.py` (×2),
`src/pipx/commands/manifest.py`, `src/pipx/commands/reinstall.py`,
`src/pipx/commands/run.py` (and more — full raw list preserved in the
evidence file). **Which queries become impossible:** directly
demonstrated in Section H (Q3, Q4) — two of this evaluation's own
real, representative coding-agent questions about this exact commit
pair's own introduced relationship are Not-answerable specifically
because of this gap, confirmed via a dedicated real-extractor probe
(not estimated):

```
src/pipx/commands/install.py::install -> NOT EXTRACTED (candidates=27, rejections=2)
src/pipx/commands/inject.py::inject_dep -> NOT EXTRACTED (candidates=9, rejections=1)
src/pipx/main.py::_cmd_install -> EXTRACTED, signature_text.len()=1198
src/pipx/main.py::_cmd_inject -> EXTRACTED, signature_text.len()=919
```

**Interpretation (Inferred):** in THIS corpus, unlike `pyjanitor`'s own
NumPy-docstring-adjacent explanation (itself walked back during
B2-EVAL-006's own independent review to a more modest "implementation
body is simply long" claim), the pattern is structurally similar: the
functions affected (`install`, `inject_dep`) are each genuinely large,
multi-step orchestration functions — the HANDLER functions that call
them (`_cmd_install`, `_cmd_inject`) are deliberately kept SHORT (under
1200 bytes each), while the real WORK happens in longer, separately-
defined functions these handlers delegate to. **This is itself a
real architectural insight the extraction limit happens to surface
only for the "worker" half of a handler/worker split** — a pattern
this domain's own CLI architecture (thin dispatch, thick command
implementation) may make MORE likely to recur than in less
layered code, though this evaluation's own two-corpus evidence is not
sufficient to claim that generally; it is recorded as a plausible,
Inferred hypothesis for a future, dedicated cross-domain frequency
study, not asserted as proven.

## B. Module/File -> top-level-symbol gap

| Metric | Value |
|---|---:|
| Files with at least one extracted symbol | 131 |
| Total top-level symbols across all files (span-nesting definition) | 2251 |
| Files with more than one top-level symbol | 117 |
| **Percentage of files with multiple top-level symbols** | **89.3%** |
| Mean top-level symbols per file | 17.18 |
| Median top-level symbols per file | 11 |

Top files: `tests/test_install.py` (142), `src/pipx/main.py` (112),
`tests/test_run.py` (105), `src/pipx/commands/common.py` (60).

**Interpretation (Inferred) — the SECOND-highest "multiple top-level
symbols" rate of any domain measured so far** (89.3%, behind only
Rust's own 95.8%; ahead of data-science Python's 71.8%, backend-Python's
58.2%, frontend-TS's 24%). **This is the first domain where the single
most architecturally important file (`src/pipx/main.py`, the CLI's own
entry point AND the file containing every subcommand's own argument-
parser construction AND every `_cmd_*` handler) is ALSO one of the
highest-top-level-symbol-count files (112 symbols)** — directly
demonstrated as a real architectural-navigation failure in Section H
(Q5): a coding agent asking "what does `main.py` contain" gets a flat
list of 122 symbols (handlers, parser-construction helpers, dispatch
internals, all genuinely top-level by this pipeline's own span-nesting
definition) with NO structural grouping distinguishing "this is a
command handler" from "this is a parser-setup helper" from "this is
internal dispatch machinery" — all three categories are real,
architecturally distinct roles in this file's own design, invisible to
this pipeline's current flat symbol list.

### B1. Further evidence for/against B2-EVAL-006's Option 2 recommendation

B2-EVAL-006's own recommendation (a synthetic `File`/`Module` node with
`Contains` edges, Option 2, explicitly tentative) is evaluated against
this domain's own evidence:

**Supporting evidence:** this domain's own Q5 result (122 symbols, one
flat list, no substructure) is exactly the query shape Option 2 was
designed to answer directly via a `Contains` edge from a `Module`/`File`
node — this domain's own evidence continues to support Option 2's own
basic mechanism (file -> contained-symbols) as directionally correct.

**New, qualifying evidence this domain adds (not seen in prior
domains):** this domain's own `main.py` case shows that a bare
`Contains`-edge list, EVEN IF a Module/File node existed, would NOT by
itself distinguish "this is a real command handler" (`_cmd_install`)
from "this is an internal helper" (`_venv_dir`) from "this is argument-
parser setup" (`_add_install`) — all three are structurally identical
(top-level functions in the same file) but SEMANTICALLY distinct roles
a coding agent would want to filter by. **This is new evidence that
Option 2's own basic mechanism, while still directionally correct, may
be INSUFFICIENT on its own for a CLI-shaped codebase specifically** —
a future, separate enhancement (e.g. tagging a `Contains` edge, or the
contained symbol itself, with a role inferred from a decorator/
registration pattern) might be needed ON TOP of Option 2, not instead
of it. This evaluation does NOT recommend implementing this refinement
now — it is recorded as a qualification to B2-EVAL-006's own
recommendation, still tentative, still not implemented, for a future
evaluation (or the eventual Module/File implementation task itself) to
weigh.

## C. Call-graph vs. command-registration-graph distinction — this domain's own central research question

| Metric | Value |
|---|---:|
| `parser.set_defaults(func=IDENTIFIER)`-shaped assignments (raw parse) | 36 |
| `X.func(...)`-shaped dynamic-dispatch call sites (raw parse) | 1 |
| Function-local/lazy `import`/`from...import` statements (raw parse) | 65 |

**Directly demonstrated, not merely counted (Section H, Q2):**
`_cmd_install` — confirmed the REAL handler for the `install`
subcommand (`parser.set_defaults(func=_cmd_install)`, `src/pipx/main.py`
line 456, corrected during independent review from an initially
mis-cited line 493) — has **ZERO** callers in the real `Calls` edge
set. This is the EXPECTED, CORRECT result given this pipeline's own
`resolve_calls` design (name-based matching against literal callee
identifiers): the one real dynamic-dispatch call site (`result: Final
= args.func(args, ctx)`, `src/pipx/main.py` line 2244, corrected during
independent review from an initially mis-cited line 2276 — confirmed
the SOLE occurrence in this entire 140-file corpus —
`dynamic_func_dispatch_call_sites: 1`) has callee NAME `"func"`, a
runtime-bound attribute, never
literally `"_cmd_install"` — so `resolve_calls` correctly finds no
candidate named `"func"` anywhere relevant and produces no edge, rather
than guessing. **The real relationship — "`parser.set_defaults(func=X)`
registers X as this subparser's own dispatch target" — is a
COMMAND-REGISTRATION relationship this pipeline has no edge kind for
at all, confirmed structurally** (no `EdgeKind` variant represents
"this value assignment registers a callable for later dynamic
dispatch"), the exact same category of structural absence Rust's own
B2-EVAL-005 found for data-flow (B2-EVAL-005 Section D1) — but a
DIFFERENT specific relationship (registration, not value-flow).

**36 real `func=` registrations exist in this corpus — this is NOT a
rare, one-off pattern; it is THE mechanism by which every one of this
CLI's own ~20 real subcommands reaches its own handler.** This means
the command-registration-graph gap is not a minor edge case for this
domain — it is the primary architectural relationship a coding agent
would need to trace "which code runs when a user types `pipx install
X`," and this pipeline cannot represent it at all today.

## D. CRC-003 (import-alias blindness) — further evidence from this domain

Per explicit instruction to look for further CRC-003-shaped cases in a
second domain: **a real, concrete instance was found**,
`src/pipx/commands/upgrade.py` line 41:
`from pipx.commands.install import install as install_package`,
with a real call site at line 107: `installed: Final = install_package(...)`.

**This specific instance is a COMPOUND case, not a clean isolated
CRC-003 repeat:** the real target (`install.py::install`) is ALSO a
SignatureTooLong rejection (Section A1) — so this specific call site's
own failure to produce an edge cannot be cleanly attributed to the
import-alias gap alone; EVEN IF `resolve_calls` were made alias-aware
tomorrow, this specific edge would still not exist, because the
target itself has no snapshot to point to. **This is itself useful,
disclosed evidence, not a weakness in this evaluation's own
methodology:** it demonstrates that real codebases can exhibit
MULTIPLE compounding gaps on the exact same relationship, and that
CRC-003's own fix (CALL-RES-A4, import/visibility-aware resolution)
alone would not have been sufficient to recover this specific edge —
a concrete, disclosed caveat for whoever eventually scopes that
backlog item's own expected real-world recovery rate.

**No clean, isolated (single-gap) CRC-003 repeat was found in this
corpus** — the other real import aliases in this codebase
(`import datetime as dt`, `from configparser import Error as
ConfigParserError`) were checked and confirmed to NOT be used in a
way that would exercise `resolve_calls`'s own alias-blindness (the
`datetime as dt` aliases are used for attribute access on a module,
never a direct function call matching this pattern; `ConfigParserError`
is used only in an `except` clause, never instantiated via a call
expression). **This evaluation's own contribution to CRC-003 is
therefore: confirmed the underlying mechanism recurs in a second,
independent domain, but found it entangled with the SignatureTooLong
gap in its own clearest real instance — not a second clean repeat.**
CRC-003's own entry is NOT duplicated or modified; this finding is
recorded here, in this evaluation's own document, as corroborating
evidence.

## E. Same-name collision — handled correctly, zero false positive (positive evidence, not a new finding)

Investigating Q3's own "found target: false" result surfaced a real,
interesting case worth recording explicitly: `install_fn_exists_in_pool=true`
— meaning a DIFFERENT symbol named `install` DOES exist in this
corpus's own pool (three, in fact: `backends/pip.py::install`,
`backends/uv.py::install`, `backends/_base.py::install`, each a real
METHOD on a different `Backend` implementation class) — yet **zero
edges to ANY of these three same-named methods appear anywhere in this
corpus's own 3427-edge `Calls` result, confirmed by direct grep of the
full raw evidence file.** This is the CORRECT behavior: this
pipeline's own current-generation call resolver (CALL-RES-A1+A2+A3,
per [B2-ID-004](../findings/B2-ID-004.md)'s own scope-narrowing design)
correctly declines to guess among genuinely ambiguous same-named
candidates rather than picking one arbitrarily or emitting all three —
a direct, positive confirmation of this pipeline's own "no edge beats
a false edge" discipline holding up on a REAL same-name collision in a
REAL corpus, not merely in a synthetic test fixture. Not filed as a new
`CRC-...` case, since no defect (FP or incorrect FN beyond the already-
understood ambiguity-produces-no-edge design) was found — recorded here
as positive evidence this project's own prior fix (B2-ID-004) continues
to hold on new real data.

## F. CLI-specific semantic patterns

| Pattern | Measured? | Finding |
|---|---|---|
| `argparse`-based CLI with subcommands | Yes | Confirmed real, `src/pipx/main.py`'s own `cli()`/`_dispatch()`/`_cmd_*` structure |
| Decorators/registration | Yes (via `set_defaults`, not a decorator in this specific corpus) | `parser.set_defaults(func=X)` is this corpus's own registration idiom — NOT a decorator pattern (unlike `click`/`typer`-based CLIs, which this evaluation did not target); confirmed this pipeline has no representation for EITHER shape, since both reduce to the same underlying gap (Section C) |
| Command tables | Yes (36 real cases) | See Section C |
| Callbacks | Yes (`args.func` is itself a callback-shaped dynamic dispatch) | See Section C |
| Subcommands | Yes (~20 real subcommands, one file each in `src/pipx/commands/`) | Confirmed real, multi-file, one-command-per-file architecture |
| Config loading | Not specifically isolated in this evaluation's own Q&A (no dedicated question targeted it) | Flagged as a residual limitation (below), not investigated |
| Environment/config dependencies | Same as above | Same |

**This evaluation deliberately did NOT implement any framework-specific
special case** (e.g. an `argparse`-aware or `click`-aware resolver
extension) — per explicit instruction, every gap above is recorded as
a semantic absence, not patched around.

## G. Edge quality

| Metric | Value | Status |
|---|---:|---|
| Lineage edges (commit B alone, no prev) | 0 | Measured |
| `Calls` edges | 3427 | Measured |
| `Contains` edges | 94 | Measured |
| `Extends`/`Implements` edges | 20 | Measured — this corpus's own real class hierarchy (Backend implementations, confirmed real inheritance via the `backends/_base.py` base class) exercises `Extends`/`Implements` meaningfully, unlike `pyjanitor`'s own near-zero class usage |
| `Instantiates` edges | 21 | Measured |
| `TestsFor` edges | 18 | Measured |
| `StaticAnalysis` (resolution method) | 3509 | Measured |
| `NameHeuristic` (resolution method) | 71 | Measured |
| Full index wall-clock | 1.0698404s | Measured, single run |

## H. Manual Calls-quality sample — TP/FP/FN judgment

The deterministic 30-edge sample (sorted by evidence note) was
individually checked against real source at commit B:

**30/30 sampled edges are target-correct and caller-correct (TP). 0
FP. 0 FN identified among the specific calls spot-checked.**

Representative confirmations: `BackendOptions` (a real class,
`options.py` line 16, called as a constructor from 9 different files
in this one 30-edge sample alone — cross-file resolution confirmed at
real scale), `CacheData`/`InjectionData` (real `OperationData`
subclasses, each confirmed instantiated via a real constructor call at
their own cited line), `IncompatiblePythonError` (a real exception
class, correctly resolved from two different callers in `venv.py`).

**Precision (on this 30-edge sample): 30/30 = 100%.** Recall explicitly
NOT computed, consistent with every prior B2 evaluation's own stated
limitation.

## I. Coding-agent usefulness Q&A

| # | Question | Answer (observed) | Classification | Missing capability if Not answerable/Incorrect |
|---|---|---|---|---|
| Q1 | What is the CLI entry point, and what does it call? | `cli()` calls: `stopping_children_on_signals`, `hide_cursor`, `show_cursor`, `_dispatch` — all confirmed real against source | **Fully answerable** | — |
| Q2 | Which handler executes the install subcommand? | `[]` — ground truth: `_cmd_install`, registered via `set_defaults(func=...)`, never a direct call | **Not answerable, precisely root-caused** | Command-registration-graph (new structural category, Section C) |
| Q3 | Does `_cmd_install` call `install` (cross-file, locally-imported)? | `[]` (only `InstallOptions`/`ExposureOptions` found) — ground truth YES, but `install` itself is a SignatureTooLong rejection, confirmed via real-extractor probe | **Not answerable, precisely root-caused** | SignatureTooLong extraction gap |
| Q4 | Does `inject_dep` call `runs_from_venv` (this exact commit's own introduced relationship)? | NOT ANSWERABLE: `inject_dep` itself not a symbol, confirmed SignatureTooLong via the same probe | **Not answerable, precisely root-caused** | SignatureTooLong extraction gap (second independent instance) |
| Q5 | What does `src/pipx/main.py` contain? | 122 symbols via file-path string matching, no structural grouping | **Not answerable** (plus the qualifying evidence in Section B1) | Module/File structure |
| Q6 | Which tests cover the install command? | `18` `TestsFor` edges exist corpus-wide; not isolated per-command in this pass | **Partially answerable** | TestsFor granularity not drilled further in this evaluation |
| Q7 | If I modify `preserve_venv`, what is impacted? | 12 real callers found (3 real production call sites, 9 real test functions), confirmed against source | **Fully answerable** | — |

**Summary (out of 7 questions): 2 Fully answerable (Q1, Q7), 1
Partially answerable (Q6), 4 Not answerable — every one precisely
root-caused** (2 to SignatureTooLong, 1 to Module/File structure, 1 to
the NEW command-registration-graph category this domain's own
protocol specifically asked to surface). **Zero Incorrect answers** —
every Not-answerable result is an honest, precisely-diagnosed absence,
never a confident wrong guess.

### I1. Data/control-flow distinction, quantified per question

| Information need | Questions | Count |
|---|---|---:|
| Calls only | Q1, Q7 | 2 |
| Calls, but blocked by SignatureTooLong | Q3, Q4 | 2 |
| Module/File structure | Q5 | 1 |
| Command-registration-graph (NOT Calls, NOT control/data-flow) | Q2 | 1 |
| TestsFor (granularity-limited) | Q6 | 1 |

**Of the 4 Not-answerable questions, the command-registration-graph
gap (Q2) is a DIFFERENT category from every gap measured in prior B2
domains** — not a call-graph limitation (Rust/B2-EVAL-005), not a
data-flow limitation (data-science/B2-EVAL-006), not purely a Module/File
limitation, but a genuinely new third category: a relationship
established by VALUE ASSIGNMENT (a function passed as a value, stored,
and invoked indirectly later) that is neither a direct call NOR a
data-flow relationship between two computed values — it is closer to a
"deferred/indirect call" relationship, which this evaluation names
explicitly as its own category rather than forcing it into either of
the two previously-established ones.

## J. Imports

| Metric | Value |
|---|---:|
| Real import statements (line-based heuristic count, commit B) | 1998 |
| `ImportFact`s extracted (file-level, Python) | 2685 |
| Extraction coverage | 2685/1998 (134.4%) |
| Resolution: `Internal` | 796 |
| Resolution: `External` | 1829 |
| Resolution: `Ambiguous` | 60 |
| Aliases captured | 20 |

**Interpretation (Inferred):** 134.4% extraction coverage is the
HIGHEST of any B2 domain measured so far, consistent with this
corpus's own heavy use of multi-name `from X import (A, B, C)`
statements across its 20 subcommand modules each importing several
shared helpers. 60 ambiguous resolutions (2.2% of all `ImportFact`s) —
moderate, not the highest or lowest of any domain. 20 aliases captured
— directly connects to Section D's own CRC-003 corroborating evidence
(`install as install_package` is one of these 20).

## K. Incremental indexing (A -> B, real multi-file commit)

| Metric | Value |
|---|---:|
| Changed files (`git diff A..B`) | 4 (`changelog.d/...`, `commands/inject.py`, `commands/run_pip.py`, `tests/test_transaction.py`) |
| Files reparsed | 4 |
| Snapshots removed | 35 |
| Snapshots added | 35 |
| Edges rebuilt | 3615 |
| Raw import statements extracted (this call) | 92 |
| Raw import statements reused (this call) | 2593 |
| Import facts resolved | 2685 |
| Full index (commit A) wall-clock | 1.1099652s |
| Incremental index (A->B) wall-clock | 674.9548ms |
| Final snapshot count | 2575 |
| Final edge count | 3580 |
| Final lineage edge count | 35 |

**Interpretation:** 2593/2685 (96.6%) raw import statements reused
despite the new `runs_from_venv` cross-file relationship this commit
introduces — a high reuse rate, consistent with a small (4-file),
surgical real-world bugfix commit. No `DuplicateSnapshotId`/`DuplicateEdge`
panics, no stale-state warnings. Incremental (675ms) is faster than
full (1.11s), consistent with data-science Python's own B2-EVAL-006
result (small/surgical commit), unlike Rust's own B2-EVAL-005 anomaly.

## L. Performance

| Metric | Value | Status |
|---|---:|---|
| Full index wall-clock | 1.0698404s | Measured, single run |
| Incremental index wall-clock | 674.9548ms | Measured, single run |
| Memory | 12,820 KB ≈ 12.52 MB (Windows `tasklist`, coarse) | Estimated |
| Graph size (final, commit B) | 2575 snapshots, 3580 edges, 35 lineage edges | Measured |
| `ImportFacts` (final) | 2685 | Measured |

Single-run measurements, not multi-run averages, consistent with every
prior B2 evaluation's own stated limitation.

## M. Challenge set

**No new `CRC-00x` case was opened in this evaluation.** The one
candidate investigated (Section D, the `install as install_package`
alias) was found to be a COMPOUND case entangled with SignatureTooLong,
not a clean, isolated repeat of CRC-003's own failure shape — recording
it as a new, separately-numbered CRC entry would misrepresent it as an
independently-reproducible alias-blindness case when it is not (fixing
alias-awareness alone would not recover this specific edge). It is
instead recorded here, in this document, as corroborating/qualifying
evidence for the EXISTING CRC-003 entry, consistent with the challenge-
set framework's own standard of only filing genuinely new, cleanly-
isolated failure shapes.

## N. Research integrity notes

- Both specific byte-size claims in Section A1/H (Q3/Q4) were verified
  via a dedicated, real-production-extractor probe — not estimated
  manually — a direct, applied lesson from B2-EVAL-006's own
  independent review, which found 2 of 3 manually-estimated byte
  figures in that evaluation to be wrong by up to 35%.
- The timing figures in Section G/K/L were confirmed to come from the
  SAME run whose raw output is cited as this evaluation's own evidence
  file (the stale intermediate run was deleted immediately after the
  final run completed, specifically to prevent the exact
  evidence-integrity gap B2-EVAL-006's own independent review found).
- Section D (CRC-003 corroboration) discloses its own limitation
  honestly (a compound, not a clean, case) rather than presenting it as
  a second clean repeat to inflate the apparent strength of the
  CRC-003 finding.
- Section E (same-name collision) is recorded as POSITIVE evidence
  (a prior fix holding up correctly), not downgraded or hidden because
  it doesn't itself constitute a new defect — consistent with the
  Evaluation Policy's own even-handed evidence requirement.
- No repository/commit was excluded or re-chosen to produce a more
  favorable result; `pipx` was the first and only CLI/tooling candidate
  evaluated in depth.

## O. Cleanup

The temporary clone (a fresh, isolated scratchpad directory, never
reused from any prior clone) was deleted after this evaluation's
harness output was captured and preserved. The throwaway real-extractor
probe used to confirm Section A1's byte-size claims was also deleted
after use, never committed. The repository identity, commit pair,
license, and SPDX reproducibility data (above) are retained; the raw
run output is preserved at
`docs/research/evidence/b2-eval-007-pipx-run.txt`.

## Residual limitations of this evaluation

- Domain/language coverage for the multi-codebase suite is now 7
  repositories (ML/Python+C++; Backend/API Python; Frontend/UI
  TypeScript/TSX; Rust/systems; Data analysis/scientific Python; CLI/
  tooling — this evaluation).
- Config loading and environment/config dependencies (Section F) were
  NOT specifically isolated with a dedicated Q&A question in this
  evaluation — flagged as a gap in THIS evaluation's own coverage, a
  candidate for a future CLI-domain evaluation or a targeted follow-up,
  not claimed as measured here.
- The command-registration-graph category (Section C/I1) is named and
  demonstrated but not further sub-classified (e.g. whether it differs
  meaningfully across `argparse`/`click`/`typer`-style registration
  idioms) — this evaluation used only an `argparse`-based corpus; a
  future `click`/`typer`-based CLI evaluation would be needed to
  confirm whether the SAME category applies identically or needs its
  own refinement for decorator-based registration specifically
  (`@cli.command()`-shaped patterns, not investigated here).
- This evaluation's own TestsFor granularity (Q6) was reported at the
  corpus-wide level only, not drilled down to confirm which SPECIFIC
  tests cover the `install` command specifically — a smaller,
  deferred gap in this evaluation's own depth, not a claim that
  `TestsFor`'s own mechanism was found broken.
- The `main.py`-role-distinction qualification to B2-EVAL-006's Option
  2 recommendation (Section B1) is itself Inferred from one file in one
  corpus — a real, concrete observation, but not yet corroborated by a
  second CLI-domain corpus's own equally central entry-point file.
