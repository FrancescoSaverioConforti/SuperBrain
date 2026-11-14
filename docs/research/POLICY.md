# Research-grade Findings Policy

Effective 2026-11-05, for the SuperBrain project generally and B2 (Code
Graph) specifically, and binding for every future session — Claude or
Codex — working on this project.

## Why this exists

The B2 real-repository smoke test checkpoint surfaced a genuine
`SymbolSnapshotId` collision in real code (see
[`findings/B2-ID-001.md`](findings/B2-ID-001.md)) that no synthetic test
had caught. The user's direction at that point: stop treating defects
discovered during development, testing, review, or validation as
"bug → patch → done." Treat them as research findings, with the same
rigor a research engineer would apply — reproduce, diagnose root cause,
weigh alternatives, validate the fix, and write it all down so the
reasoning survives the session that found it.

## When to open a finding

Any time, during development, testing, mutation testing, independent
review, security review, real-repository validation, benchmarking, or
debugging, one of the following emerges:

- an error, bug, or crash;
- a limit or unexpected behavior;
- a vulnerability;
- a false assumption;
- a significant architectural decision forced by new evidence.

Do not just fix it. Open a finding first (or alongside the fix for a
small one), per the template below.

## Finding ID scheme

Stable, never reused, prefixed by area:

- `B2-ID-xxx` — identity/data-model
- `B2-SEC-xxx` — security
- `B2-PERF-xxx` — performance
- `B2-COV-xxx` — coverage/semantic gaps
- `B2-GRAPH-xxx` — graph structure/traversal

Other prefixes may be added as needed, following the same `<AREA>-xxx`
shape, zero-padded to 3 digits, sequential within each prefix.

## Required sections per finding

Each finding is its own file at `docs/research/findings/<ID>.md`. The
required sections, in order:

1. **Finding ID** and **Title**.
2. **Discovery context** — how it was found (synthetic test, mutation
   testing, independent review, real repository, benchmark, security
   audit, user report). For a real-repository finding, record the exact
   repository and commit.
3. **Observable evidence** — concrete evidence (files, symbols, input,
   output, failing test, observed behavior, metrics). Separate
   **Observed** (what was directly seen) from **Inferred** (what was
   concluded from it) explicitly.
4. **Root cause** — not the symptom. Name the violated invariant, wrong
   assumption, abstraction leak, data-model deficiency, algorithmic
   limitation, race, security-boundary error, or coverage gap.
5. **Impact** — classify along correctness / security / performance /
   robustness / maintainability / semantic coverage / developer
   experience / agent usefulness, and assign severity (Critical /
   Important / Moderate / Minor) with a stated reason.
6. **Solution adopted** — what was chosen, why it addresses the root
   cause (not just the symptom), what invariants it introduces, and any
   schema/version change.
7. **Alternatives considered** — realistic alternatives and why each was
   rejected.
8. **Validation** — how the fix was verified: regression tests, mutation
   testing, real-repo reproduction, invariant/property tests,
   independent review, benchmark, security tests.
9. **Literature / authoritative evidence** (see below).
10. **Residual limitations** — always present. No finding is closed as
    fully solved if it is not.

## Literature / authoritative evidence

When a finding touches a principle with real, relevant technical
literature, search for authoritative sources supporting the diagnosis,
the architectural principle, the chosen solution, or the trade-off.
Priority order: peer-reviewed papers > conference/journal papers >
standards/RFCs > official documentation > recognized academic books or
technical texts > authoritative technical reports. Avoid random blogs as
a primary source. Never cite a source for more than it actually
supports. If nothing truly relevant exists, write exactly:

> No directly applicable authoritative source identified

rather than forcing an unrelated citation. For each source actually
used, record authors, title, venue/publisher, year, and a DOI/arXiv/
RFC/official URL, plus which specific claim it supports. Keep empirical
evidence from this project and literature support clearly distinguished
— never blend them into one claim.

A shared `docs/research/references.bib` may be used across findings when
convenient.

## Research workflow

For a significant finding, follow:

```
observation → reproduction → hypothesis → root cause → alternatives
  → intervention → validation → literature support → residual limitations
```

not `bug → patch → done`. Record negative results, rejected approaches,
and invalidated assumptions — they are part of the finding, not noise to
discard.

## Final project report

`docs/research/findings.md` is the index: one row per finding in a table
with columns `ID | Problem | Root cause | Impact | Solution | Validation
| References`, plus a narrative discussion for major findings structured
as:

> Problem → Evidence → Root Cause → Alternatives → Solution →
> Experimental Validation → Literature Support → Remaining Limitations

## Persistence and discoverability

Every finding must be:

- committed to the repository under `docs/research/findings/`;
- indexed in `docs/research/findings.md`;
- recorded in Continuity with enough detail that a future session (Claude
  or Codex) can recover it without re-reading the full finding file.

This policy document itself is recorded in Continuity so a future
session discovers it even before encountering the first finding.
