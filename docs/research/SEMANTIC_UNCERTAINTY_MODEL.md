# Semantic uncertainty / provenance model — design

Quality & Semantic Reliability Hardening, Phase 1, deliverable 6.
**Status: design only, grounded in the real current code below — not
yet implemented as a new, unified type.**

## Audit of the CURRENT state (as of this document, verified by
## direct code reading, not described from memory)

### `codegraph_domain::ResolutionMethod` (`crates/codegraph-domain/src/evidence.rs`)

```rust
pub enum ResolutionMethod {
    StaticAnalysis,
    NameHeuristic,
    Inferred,
}
```

Three values, flat, no hierarchy. `StaticAnalysis` means "a candidate
pool search found exactly one match" — NOT "a compiler/type-checker
verified this." This is a real naming trap: `StaticAnalysis` in THIS
codebase is still ultimately name-based matching, just with zero
ambiguity left among the candidates it was given. It does not mean
"semantically certain" in the sense a type-checker's own
`StaticAnalysis` would. **This document's own proposed model below
deliberately does not reuse this name for its own top tier**, to avoid
compounding the confusion.

### `codegraph_domain::Confidence`

```rust
pub enum Confidence {
    Certain,
    Likely,
    Speculative,
}
```

Used only by `LineageStatus::Hypothesized` (cross-commit rename/move
detection) today — confirmed by grep across the whole `edges`/`impls`/
`imports`/`csharp_declarations` modules, every `Edge`'s own evidence
construction always passes `Confidence::Speculative` when it passes
one at all (`resolve_calls`'s `NameHeuristic` edges), never a real
graded score. `Confidence::from_score` exists and buckets a
continuous `[0.0, 1.0]` score, but **no typed-edge resolver in this
codebase actually computes a continuous score to bucket** — only
`lineage::similarity` does (text-similarity scoring for rename
detection). This is a real, confirmed gap: the infrastructure for a
graded confidence exists, but `Calls`/`Extends`/`Implements` edges
never use it, because there is no scoring function for them at all
today, by design (per this whole evaluation series' own
"evidence-before-claims" policy — no invented percentage has ever been
introduced for these).

### `codegraph_domain::Evidence`

```rust
pub struct Evidence {
    method: ResolutionMethod,
    source_span: Option<(FileIdentity, LineRange)>,
    note: String,  // max 512 bytes
}
```

Every `Edge` carries one, unconditionally (`edges/resolution.rs`'s own
doc comment: "there is no 'confirmed, no explanation needed'
shortcut"). `note` is free text — genuinely useful provenance detail
(e.g. "call to 'install' resolves to exactly one candidate in the
given pool") lives here today, but it is UNSTRUCTURED: a consumer
(including a future coding agent) can read it as a human can, but
cannot programmatically ask "which candidates existed?" or "why was
THIS one chosen over the others?" without parsing free text.

### The central finding: the "no edge beats a false edge" discipline means `Ambiguous` and `Unresolved` are NEVER represented for `Edge`s themselves

Confirmed by direct reading of every `Edge`-producing resolver:

- `resolve_calls`: an ambiguous callee name (even after
  `narrow_by_scope`'s own scope-narrowing) produces **NO edge at all**
  — not a "this is ambiguous, here are the candidates" edge, just
  silence. Only the SINGLE narrowed-down winner ever becomes an edge,
  tagged `NameHeuristic`/`Speculative`.
- `resolve_extends_implements`/`resolve_extends_implements_with_csharp_contexts`:
  `candidates.len() != 1` (and, post-B2-ID-010, every reconciliation
  condition failing) produces **NO edge**.
- `resolve_instantiates`: same `candidates.len() != 1` -> no edge
  pattern (`edges/instantiates.rs:100`).
- `resolve_tests_for`: the same two-part gate (naming convention AND
  an actual call) either fully succeeds or produces nothing.

**This means the final `CodeGraph` cannot distinguish, for any pair of
symbols that might plausibly be related:**
1. "this pipeline looked and found no relationship" (genuinely
   unresolved/zero candidates);
2. "this pipeline looked and found the relationship genuinely
   ambiguous between 2+ real candidates" (ambiguous);
3. "this pipeline never attempted to look at all" (e.g. the symbol
   pair was never in the same resolution pool, an unrelated reason).

All three collapse to the same observable state: **absence of an
edge.** A consumer graph-querying for "what does X call?" gets the
same empty/partial answer regardless of which of the three is
actually true. This is the single most consequential provenance gap
this audit found, and it is DELIBERATE (the "no edge beats a false
edge" policy is correct and should not be reversed) — but it means any
future provenance model must be layered ALONGSIDE the edge set, not
purely encoded within it, if "ambiguous" and "unresolved" are ever to
become distinguishable to a consumer.

### Structural facts (`ImportFact`, `ImplFact`) ALREADY have a real 3-way status — the inconsistency to resolve

Unlike `Edge`s, `ImportFact`/`ImplFact` (and, by the same pattern, any
future structural fact) carry an EXPLICIT, QUERYABLE per-fact
resolution status:

```rust
// imports.rs
pub enum ImportResolution {
    Internal(SymbolSnapshotId),  // exactly one match
    External,                     // zero matches (not in this pool)
    Ambiguous,                    // 2+ matches
}

// impls.rs
pub enum ImplTargetResolution {
    Resolved(SymbolSnapshotId),
    Unresolved,
    Ambiguous,
}
```

These ARE queryable and DO distinguish "zero candidates" from
"ambiguous" from "resolved" — but only for file-level import
statements and Rust `impl` blocks, not for `Calls`/`Extends`/
`Implements`/`Instantiates`/`TestsFor` edges, which are the far more
heavily-used relationship kinds. **This is the core inconsistency this
model must resolve**: the richer status already exists in the
codebase, in two places, and the uniform model below is substantially
an exercise in generalizing an ALREADY-PROVEN pattern to the `Edge`
type, not inventing something new from nothing.

### `LineageStatus`

```rust
pub enum LineageStatus {
    Confirmed { evidence: Evidence },      // must be StaticAnalysis
    Hypothesized { evidence: Evidence, confidence: Confidence },
}
```

A two-tier model, enforced at construction (`confirmed()` rejects
non-`StaticAnalysis` evidence). No `Ambiguous`/`Unresolved` variant —
lineage either proposes exactly one link or proposes nothing (handled
by `resolve_lineage`'s own tie-break rule: "two equally plausible rename
targets produce no edge, not two").

## Proposed uniform model

A single, crate-wide enum expressing evidence TIER, orthogonal to
*which* fact/edge kind it attaches to — generalizing the
`ImportResolution`/`ImplTargetResolution` pattern that already exists,
rather than introducing a fourth, differently-shaped status type:

```rust
/// A uniform evidence tier for ANY resolved-or-not relationship this
/// pipeline produces -- an Edge, a structural fact's own resolution
/// status, or a lineage link. Discrete and verifiable; NEVER a
/// continuous/invented confidence percentage (see "What this model
/// explicitly rejects" below).
pub enum EvidenceTier {
    /// Exactly one real candidate existed, and the relationship
    /// follows from syntax alone with no naming ambiguity at any
    /// point in the resolution process. Mirrors today's
    /// `ResolutionMethod::StaticAnalysis` + `ImportResolution::Internal`/
    /// `ImplTargetResolution::Resolved`, generalized to every kind.
    Resolved,
    /// A naming-based match was found, but the matching process
    /// itself involved a non-syntactic judgment call (e.g. scope
    /// narrowing picked the nearest enclosing class/file among
    /// several same-named candidates). Mirrors today's
    /// `ResolutionMethod::NameHeuristic`.
    Heuristic,
    /// Two or more real candidates existed and this pipeline
    /// DECLINED to pick one -- never silently collapsed to the first
    /// or an arbitrary one. Mirrors today's `ImportResolution::Ambiguous`/
    /// `ImplTargetResolution::Ambiguous`, generalized to `Edge`s,
    /// which currently have NO way to represent this at all (the
    /// central finding above).
    Ambiguous,
    /// Zero real candidates existed in the given pool -- the target
    /// may be external (stdlib/third-party/not-yet-indexed) or may
    /// simply not exist. Mirrors today's `ImportResolution::External`/
    /// `ImplTargetResolution::Unresolved`.
    Unresolved,
}
```

Paired with explicit, STRUCTURED provenance (not free text) wherever
`EvidenceTier` is `Heuristic`/`Ambiguous`/`Unresolved` — generalizing
`Evidence.note`'s own existing free-text content into queryable
fields:

```rust
pub struct Provenance {
    /// Which resolver/extractor produced this determination (e.g.
    /// "resolve_calls", "resolve_extends_implements_with_csharp_contexts").
    /// A plain string naming the real function, not an enum -- new
    /// resolvers must not require a central enum edit just to report
    /// their own provenance.
    resolver: String,
    /// Every real candidate this resolver actually considered (by
    /// SymbolSnapshotId), REGARDLESS of which one, if any, was chosen.
    /// For `Ambiguous`, this is what lets a consumer inspect the real
    /// options instead of just knowing "it was ambiguous." For
    /// `Resolved`, this is always exactly one id (confirming there
    /// never was a second candidate, not just that one was picked).
    candidates_considered: Vec<SymbolSnapshotId>,
    /// Why THIS candidate (if any) was chosen over the others, in the
    /// SAME structured terms used throughout this project's own real
    /// evidence notes today (e.g. "narrowed to caller's own enclosing
    /// class", "lowest byte-span-start fragment among N partial-class
    /// fragments") -- kept as a short, bounded string for now (this
    /// model does not propose fully structuring every resolver's own
    /// reasoning into typed fields in this Phase 1 pass; see residual
    /// limitations below).
    selection_rationale: Option<String>,
    /// Known limitations of the METHOD used, not the specific
    /// instance -- e.g. "name-based matching cannot distinguish
    /// overloads by parameter list or generic arity" would be
    /// attached to every Heuristic-tier Calls edge in a domain with
    /// real overloads, not computed per-edge.
    method_limitations: Vec<String>,
}
```

### What this model explicitly rejects

- **An invented confidence percentage.** `Confidence::from_score`
  already exists and is explicitly reserved for cases where a REAL
  continuous score exists to bucket (today: only lineage-rename
  text-similarity). This model does not propose extending percentage-
  based confidence to `Calls`/`Extends`/etc., which have no real
  scoring function today — inventing one now, just to produce a
  number, would be exactly the kind of "confidence theater" the
  user's own instruction explicitly rejected ("Non usare una
  percentuale di confidence inventata. Preferisci evidence/status
  discreti e verificabili"). If a real scoring function is ever built
  for a specific resolver (e.g. a future overload-ranking heuristic),
  it should bucket into `EvidenceTier` the same way lineage similarity
  already buckets into `Confidence` — discretely, with a named,
  checkable threshold, not as a raw float exposed to a consumer.
- **A fifth, "Confirmed/strongly-grounded" tier distinct from
  `Resolved`.** The design brief asked for "Confirmed / strongly
  grounded, StaticAnalysis, Heuristic, Ambiguous, Unresolved" — five
  named tiers. On inspection, this project's own `StaticAnalysis`
  EVIDENCE always already means "exactly one candidate, no ambiguity"
  (see the audit above) — there is no SEPARATE, STRONGER tier above it
  in any resolver this project has today (e.g. no type-checker-backed
  confirmation exists yet). Introducing an unused "Confirmed" tier
  above `Resolved` now, with nothing that could ever produce it, would
  be speculative infrastructure this Phase 1 pass's own instruction
  explicitly warns against ("non implementare ancora... A4, A5..."). A
  future differential-testing oracle (Roslyn, rustc, etc. — see the
  differential-testing design doc) IS a plausible future source of a
  genuinely stronger tier (a relationship both this pipeline's own
  resolver AND an external compiler/language-service agree on) — this
  model names that as the natural future home for a `Confirmed` tier,
  rather than fabricating one now with no real producer.

## How this maps onto what exists today (migration sketch, not implemented)

| Today | Proposed `EvidenceTier` |
|---|---|
| `ResolutionMethod::StaticAnalysis` | `Resolved` |
| `ResolutionMethod::NameHeuristic` | `Heuristic` |
| `ResolutionMethod::Inferred` (lineage-only) | `Heuristic` (lineage's own `Hypothesized` already carries a separate `Confidence`, which this model leaves untouched — lineage is a different enough domain, cross-commit identity, not cross-symbol relationship, that forcing it through the exact same tier set is not proposed here) |
| `ImportResolution::Internal` | `Resolved` |
| `ImportResolution::External` | `Unresolved` |
| `ImportResolution::Ambiguous` | `Ambiguous` |
| `ImplTargetResolution::Resolved`/`Unresolved`/`Ambiguous` | same 1:1 mapping as `ImportResolution` |
| An `Edge` that was never produced because the resolver found 2+ candidates | **Currently: nothing. Proposed: a NEW, separate `AmbiguousAttempt` record** (not an `Edge` — `Edge` still requires a real `(from, to)` pair; an ambiguous attempt has one `from` and MULTIPLE possible `to`s) kept alongside the edge set, generalizing `ImportFact`/`ImplFact`'s own already-proven "keep the structural fact even when unresolved" pattern to `Calls`/`Extends`/etc. **This is the one genuinely new piece of data model this design proposes** — everything else above is a relabeling of what already exists. |

## Residual limitations of this design (honest, not hidden)

- This is a DESIGN, not an implementation. No code changes were made
  to `ResolutionMethod`/`Evidence`/the edge resolvers as part of this
  Phase 1 pass — per the user's own explicit scope ruling.
- `selection_rationale` is left as a short string, not fully
  structured, in this pass — fully structuring "why this candidate
  won" (e.g. a typed enum of narrowing strategies: `SameEnclosingClass`,
  `SameFile`, `LowestSpanStart`, ...) is real future work this design
  does not attempt to finish now.
- The proposed `AmbiguousAttempt` record is the one piece of this
  design that would require real engineering work to introduce (new
  type, new storage on `CodeGraph`, new resolver-side construction
  logic in every typed-edge resolver) — this document does not
  estimate that work's own size or claim it is small.
- This model does not address `TestsFor`'s own, separate, DIFFERENT
  reliability question (a naming CONVENTION being treated as evidence
  at all) — that is a methodology question about what counts as
  evidence in the first place, not a provenance-representation
  question this model's own scope covers.
