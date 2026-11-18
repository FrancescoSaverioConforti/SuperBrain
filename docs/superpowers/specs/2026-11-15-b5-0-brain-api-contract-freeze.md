# B5.0 — Brain API Contract Freeze

Status: FROZEN (user-approved 2026-11-16)
Supersedes: nothing (first formal freeze of this contract)
Depends on: `docs/superpowers/specs/2026-11-15-b5-brain-api-mcp-design.md` (B5 design), `docs/Spec_mvp.md` (MVP perimeter), continuity decisions `d238mm`/`d239a3`/`d240mr`/`d241qo`/`d242y7`/`d245xo`/`d246ec`/`d248e4`/`d249ef`/`d251bo`/`d252xh`
Implements: plan Task 7 (`docs/superpowers/plans/2026-11-15-b5-brain-api-mcp.md`)

## Scope

Freezes the public contract for all 12 Brain API capabilities: stable
name, input schema, output schema, typed error set, permission
requirement, determinism statement, empty/not-found semantics,
pagination/limits (exact numeric limits deferred to plan Task 35;
this document freezes which capabilities need one, not the number).

This document is the single source of truth both `crates/brain-api`
(the new service layer, Task 8+) and the MCP server (Task 21+) build
against. Neither may diverge from it without a new freeze.

## Architecture boundary (frozen, restated from the design doc)

- `crates/api` stays exactly what it is today: "B1 callable core: local
  single-user read-only functions, no transport" (`crates/api/src/lib.rs:1`).
  Nothing in this document adds transport logic, proposal semantics, or
  write capability to `crates/api`.
- `crates/brain-api` (new) is the only place write/proposal/approval
  logic lives. It delegates the 9 read-shaped capabilities to the real
  underlying crates (`api`, `index`, `codegraph-index`/`codegraph-domain`,
  to-be-built rules/regulations/canon source) and owns the 3
  proposal-creating capabilities plus the CLI-only approval pair.
- No caller of any kind (MCP server, future B6 UI) reaches `storage`
  directly. Every write-shaped effect goes through `crates/brain-api`.

## Trust model (frozen, restated from design doc + continuity)

- Read-shaped capabilities execute directly when the caller's
  `ReadPolicy`/`ReadOperation` check passes.
- `save_memory`, `update_memory`, `link_entities` from any caller
  (including MCP) never execute a write directly — they always produce
  a `PendingWriteProposal` (`Pending` status).
- Only a trusted human approval surface (CLI now; B6 UI later) may
  move a proposal to `Approved`/`Rejected`, via `WritePolicy::trusted_cli()`-gated
  `approve_proposal`/`reject_proposal`. An MCP-originated `RequestContext`
  is always constructed with `write_policy.approve_write_allowed == false`
  — never a caller-supplied flag.
- The agent side can never: self-approve, impersonate the approval
  surface, escalate a read permission to write, bypass approval via
  another Brain API call, or mutate proposal status directly in storage
  (`c29zz`).
- `originating_identity` on a proposal is derived from the MCP
  connection/session binding (B5.8's server-registration layer), never
  from a client-supplied request field (`d246ec`).

## Frozen typed error set (B5.14, restated here since every capability's table row references it)

```
NotFound
InvalidInput(String)
PermissionDenied
RuntimeUnavailable
DenseUnavailable
ExternalToolUnavailable
Timeout
Conflict
CorruptData
UnsupportedOperation
```

No capability may introduce a new error variant outside this set
without a new freeze of this document.

## `get_rules` / `get_regulations` / `get_canon`: NEW B5.0 definitions

**`get_rules`, `get_regulations`, and `get_canon` are first formally
introduced by B5.0. No repository-wide semantic contract for these
concepts existed before B5.**

Grounding performed before writing this section (user-directed, not
skipped): searched `docs/Spec_mvp.md`, `docs/technical-debt.md`,
`crates/domain/src/schema.rs`'s `NoteType` enum (7 variants: `Code`,
`Bug`, `Decision`, `Pattern`, `Research`, `Session`, `Summary` — none
map to rules/regulations/canon), and the whole `docs/` tree for
"canon", "regulation", "constitution", "project rules". Found: no
`CLAUDE.md`/`AGENTS.md` in this repository; four B2-scoped policy
documents (`docs/research/POLICY.md`, `EVALUATION_POLICY.md`,
`EXPERIMENTAL_POLICY.md`, `MUTATION_TESTING_POLICY.md`), each
explicitly "binding for every future session" but scoped to Code Graph
research methodology (findings, evaluation evidence, experimental
hypothesis discipline, mutation testing), not a project-wide
constitution; `docs/research/AGENT_RELIABILITY_CONTRACT.md`, scoped to
Code Graph edge-evidence trust, not general engineering rules. None of
these is generalized beyond what it actually declares.

### `get_rules`

Represents the project's **internal operating and engineering rules**:
engineering constraints, coding conventions, security rules, testing
requirements, mutation-testing discipline, research methodology, agent
operating rules, internal workflow/policy documents, and operating
invariants that must hold during development and execution.

- **Source of truth**: any document the project has explicitly
  designated as a binding internal policy. Today, concretely, this
  means the four `docs/research/*POLICY.md` files and
  `AGENT_RELIABILITY_CONTRACT.md` — each classified under its own real,
  declared scope, never generalized.
- **Scope metadata (required on every returned item)**: a `scope`
  field naming the rule's real declared domain, e.g. `code_graph`,
  `research`, `evaluation`, `mutation_testing`. A rule with no
  identifiable scope is not returned by `get_rules` as a general
  project rule — scope must be real, not inferred.
- **Filtering**: callers may filter by `scope`; omitting the filter
  returns rules across all known scopes.
- **Precedence**: no internal precedence between rules of different
  scopes (they govern different domains) — see the cross-capability
  precedence model below for `Canon > Rules > ordinary project
  knowledge`.
- **Empty behavior**: if no rule matches the filter, returns an empty
  result set (`Ok(vec![])`), never `NotFound` — an empty rule corpus
  for a given scope is a valid state, not an error.
- **Provenance**: each returned item carries its source document path
  and the "binding" declaration's own effective date, where stated.

### `get_regulations`

Represents **external regulatory, legal, and compliance constraints**
applicable to the project — e.g. GDPR, NIS2, the EU AI Act, security/
compliance standards, legal requirements. This is an axis distinct
from `get_rules`: regulations are imposed from outside the project,
not authored by it.

- **Source of truth**: none exists in the repository today. The corpus
  is legitimately empty.
- **Empty behavior**: an empty corpus returns `Ok(vec![])`, never an
  error — `get_regulations` returning nothing is the expected, correct
  answer until an external regulation is explicitly recorded against
  the project.
- **Scope**: when populated, each entry must name the regulatory
  regime it comes from (e.g. `gdpr`, `ai_act`) and must never be
  conflated with an internal rule's `scope` field (different field,
  different namespace — a regulation is never returned by `get_rules`
  and vice versa).
- **Precedence**: regulations are a separate external axis, not a rung
  on the `Canon > Rules` ladder (see below).

### `get_canon`

Represents the **authoritative corpus of consolidated project
truths**: explicitly canonized architecture, stable decisions,
invariants, principles, system contracts — facts that must prevail
over exploratory, session, or non-consolidated research notes.

- **Canonization is explicit, not automatic.** A `Decision`-typed note
  (`domain::NoteType::Decision`) is NOT automatically Canon. Something
  becomes Canon only through an explicit canonization step (mechanism
  deferred to implementation — e.g. a dedicated classification field or
  a canon-specific marker; NOT a new `NoteType` variant added
  automatically by this freeze, per the user's explicit instruction not
  to touch `NoteType` yet).
- **Source of truth**: today, nothing in the repository has been
  explicitly canonized under this new contract — the corpus starts
  empty. Frozen architecture decisions in this document and the B5
  design doc are themselves CANDIDATES for canonization once the
  mechanism exists, not automatically canon by virtue of being written
  down.
- **Empty behavior**: `Ok(vec![])`, never an error.
- **Provenance**: each canon entry must carry what canonized it (who/
  when/why), not just its content.

### Cross-capability precedence model (frozen)

```
Canon > Rules > ordinary project knowledge (search/retrieve results)
```

**Regulations are a separate external axis, not a level in that
ladder.** When an applicable regulation imposes a constraint
incompatible with Canon or Rules, the system must:

- **expose the conflict** (return it as visible, structured information
  to the caller — never silently pick a side);
- **never resolve it silently**;
- **never invent a new precedence rule** to resolve it automatically;
- **leave the decision to a trusted human surface / policy layer** —
  the same trusted-human-approval boundary already frozen for write
  proposals, not a new authority.

This conflict-exposure behavior is a requirement on `get_regulations`
callers composing results with `get_canon`/`get_rules`, not a new
automatic cross-capability call `get_rules` or `get_canon` must make
internally — B5.0 freezes the *contract* for this; the orchestration
(if any) is implementation detail for B5.1+.

## The 12-capability table

| # | Capability | Shape | Permission | Determinism | Empty/not-found |
|---|---|---|---|---|---|
| 1 | `search` | read, direct | `ReadOperation::Search` (existing, `crates/policy/src/read.rs`) | DenseV2 mode unchanged from B4; BM25 scores comparable within one response only | `Ok(vec![])` on no match |
| 2 | `retrieve` | read, direct | `ReadOperation::Retrieve` (existing) | Deterministic for a given committed version | `NotFound` for an unknown id; `Inconsistent`→`CorruptData` on hash mismatch (never a silent fallback, per `d9mm`) |
| 3 | `graph_query` | read, direct | `ReadOperation::GraphQuery` (new variant) | Deterministic for a fixed graph snapshot; bounded traversal (depth/result limits, exact numbers at plan Task 35) | `Ok(vec![])` for no matching paths/nodes |
| 4 | `get_entity` | read, direct | `ReadOperation::GetEntity` (new variant) | Deterministic for a fixed graph snapshot | `NotFound` for an unknown entity id |
| 5 | `get_code_context` | read, direct | `ReadOperation::GetCodeContext` (new variant) | Deterministic for a fixed index snapshot | `NotFound` for an unresolvable symbol/location |
| 6 | `get_previous_solution` | read, direct | `ReadOperation::GetPreviousSolution` (new variant) | Deterministic for a fixed committed vault state | `Ok(vec![])` when nothing matches — never fabricated |
| 7 | `get_rules` | read, direct | `ReadOperation::GetRules` (new variant) | Deterministic for a fixed set of binding policy documents | `Ok(vec![])` for an unmatched scope filter |
| 8 | `get_regulations` | read, direct | `ReadOperation::GetRegulations` (new variant) | Deterministic (trivially — corpus starts empty) | `Ok(vec![])` always today |
| 9 | `get_canon` | read, direct | `ReadOperation::GetCanon` (new variant) | Deterministic for a fixed canonized corpus | `Ok(vec![])` always today (nothing yet canonized) |
| 10 | `save_memory` | **proposal-creating** | `WriteOperation::ProposeWrite` (new, `crates/brain-api`) | Proposal creation is deterministic given its inputs; the write it describes is NOT yet applied | Returns a `ProposalId`, never a committed result |
| 11 | `update_memory` | **proposal-creating** | `WriteOperation::ProposeWrite` | Same as `save_memory` | Same |
| 12 | `link_entities` | **proposal-creating** | `WriteOperation::ProposeWrite` | Same | Same |
| — | `approve_proposal` / `reject_proposal` (execution, not one of the 12 — required by the trust model, CLI-only origin) | execution | `WriteOperation::ApproveProposal` / `RejectProposal` | Deterministic; never executes a second time for the same proposal | `NotFound` for an unknown proposal id; `InvalidInput` for an already-resolved/expired proposal |

No capability beyond these 12 (plus the required approval pair) is
authorized by this freeze. Adding one requires stopping and freezing a
new version of this document, per the user's explicit instruction.

## Per-capability schemas

### 1. `search` (existing, unchanged)

Delegates to `api::search` (`crates/api/src/search.rs`) as-is. Input/
output types are whatever `api::search`'s own real signature already
defines — this freeze does not redefine them, only confirms `brain-api`
calls through without modification.

### 2. `retrieve` (existing, unchanged)

Delegates to `api::retrieve` (`crates/api/src/retrieve.rs`) as-is, same
rule as `search`.

### 3. `graph_query`

- **Input**: a query scope bounded to what `codegraph-index`/
  `codegraph-domain` actually support today (confirm exact shape at
  plan Task 17 by reading `crates/codegraph-index/src/*.rs`'s real
  public functions before implementing — this freeze does not invent
  a query grammar beyond what the underlying crate exposes), plus a
  `limit` parameter (exact default/max at plan Task 35).
- **Output**: a bounded list of graph facts/paths, using the real
  types already confirmed to exist: `codegraph_domain::{EdgeKind,
  SymbolKind, SymbolSnapshotId}` (`crates/codegraph-domain/src/lib.rs:16-18`).
- **Errors**: `InvalidInput` for a query exceeding the traversal-depth
  bound; `RuntimeUnavailable` if the graph index isn't built.

### 4. `get_entity`

- **Input**: an entity id (`codegraph_domain::SymbolSnapshotId` or
  equivalent, confirmed at plan Task 17).
- **Output**: the entity's known facts (kind, evidence method, source
  location) using the real `codegraph_domain::evidence` types.
- **Errors**: `NotFound` for an unknown id; `InvalidInput` for a
  malformed id.

### 5. `get_code_context`

- **Input**: a symbol or location reference.
- **Output**: surrounding code context from `codegraph-index`, bounded
  in size (exact limit at plan Task 35).
- **Errors**: `NotFound`, `RuntimeUnavailable` if the code graph isn't
  built for the target repository.

### 6. `get_previous_solution`

- **Input**: a query (free text or structured filter) plus an optional
  `NoteType` filter restricted to the types this capability's own
  intent covers (bug/fix/test/decision provenance, per the original
  B5 brief) — concretely `NoteType::Bug`, `NoteType::Decision` are the
  two existing, real `NoteType` variants (`crates/domain/src/schema.rs:5-13`)
  this capability targets; `NoteType::Pattern`/`Research`/`Session`/
  `Summary`/`Code` are not excluded by construction but are not this
  capability's primary intent.
- **Output**: matching notes with their `Relations` (links to
  entities/other notes) per `domain::Relations`
  (`crates/domain/src/note.rs`), so a result can be traced to linked
  context when available.
- **Errors**: none beyond the frozen set; an empty match is `Ok(vec![])`.

### 7. `get_rules`

- **Input**: optional `scope` filter (string, matched against each
  rule document's real declared scope — see the New B5.0 Definitions
  section above).
- **Output**: list of `{ content_or_reference, source, scope,
  provenance, status, version_or_date }` — per the user's explicit
  response-shape requirement. `content_or_reference` may be the
  document's own path/section reference rather than inlined content,
  for large policy documents.
- **Errors**: none beyond the frozen set; unmatched scope is
  `Ok(vec![])`.

### 8. `get_regulations`

- **Input**: optional `regime` filter (string, e.g. `gdpr` — no-op
  today since the corpus is empty).
- **Output**: same shape as `get_rules`'s response fields (`content_or_reference`,
  `source`, `scope`→here meaning regulatory regime, `provenance`,
  `status`, `version_or_date`), kept structurally parallel so a caller
  composing results from both capabilities can reason about them
  uniformly, but namespaced separately per the "never conflated with
  get_rules" rule above.
- **Errors**: none; always `Ok(vec![])` until a regulation is recorded.

### 9. `get_canon`

- **Input**: optional topic/area filter.
- **Output**: same response shape as `get_rules`, plus an explicit
  `canonized_by`/`canonized_at` provenance pair distinguishing it from
  a rule's provenance (a rule is binding because a policy document
  declares it so; a canon entry is binding because someone explicitly
  canonized it — the provenance must show which).
- **Errors**: none; always `Ok(vec![])` until something is canonized.

### 10–12. `save_memory` / `update_memory` / `link_entities`

- **Input** (per operation; this list is EXACT, corrected post-Task-13
  against `crates/brain-api/src/memory.rs`'s real `ProposedOperation` —
  a Task 13 draft under-specified both `save_memory` and `update_memory`
  against this freeze's own original text; fixed as a Task 14
  prerequisite before any execution code could be written against the
  incomplete shape):
  - `save_memory`: `note_id` (target `NoteId` for the brand-new note —
    caller-chosen, since `update_memory`/`link_entities` need a stable
    id to reference it by), `NoteType`, `Classification`, `project`
    (`domain::NoteMetadata`'s field, `crates/domain/src/note.rs:27-40`),
    `source` (same struct's field — frontmatter METADATA, e.g. an
    agent/tool label; explicitly NOT a security identity and never a
    substitute for `originating_identity`), `event_period` (mandatory
    `String`, per `domain::parse_frontmatter`'s real, always-required
    field — no `Option`, no derived-from-timestamp convention; the
    caller states it explicitly as part of the note's intent), note
    content/body, `originating_identity` (server-assigned, never
    caller-supplied). `created`/`updated` are NOT inputs: both are
    system-generated at `approve_proposal` execution time from whatever
    timestamp it runs with (`updated = created` at creation, matching
    `storage::propose_and_commit`'s own note-authoring convention — no
    further update-timestamp semantics invented beyond what the storage
    model already provides).
  - `update_memory`: target `NoteId`, new content/body, `expected_version`
    (optimistic concurrency, mirroring `storage::propose_and_commit`'s
    existing `expected_version` parameter — confirmed real at
    `crates/storage/src/propose.rs`; MANDATORY, not optional — without it
    `approve_proposal` cannot detect that the target note changed between
    proposal creation and approval), `originating_identity`.
  - `link_entities`: source/target identifiers, relation kind
    (constrained to `domain::Relations`'s real fields: `supports`,
    `contradicts`, `derived_from`, `supersedes`, `applies_to` —
    `crates/domain/src/note.rs:13-24`, confirmed real), `originating_identity`.
    No `expected_version` — this operation's input list has never
    included one, and adding relations is append-only (never a
    replace-in-place that a stale read could silently clobber).
- **Output**: `ProposalId` (new type, `crates/brain-api`), plus the
  `PendingWriteProposal`'s initial `status: Pending`.
- **Errors**: `PermissionDenied` if the caller's `WritePolicy` doesn't
  allow `ProposeWrite`; `InvalidInput` for a malformed relation kind or
  missing required field.
- **Proposal semantics**: see the design doc's `PendingWriteProposal`
  shape (`ProposalId`, `originating_identity`, `operation`,
  `provenance`, `expires_at`, `status`) — unchanged by this freeze,
  restated here for completeness: `status` transitions only
  `Pending → Approved/Rejected/Expired`, enforced by
  `valid_proposal_transition`, never backward, never settable by the
  agent side.

### `approve_proposal` / `reject_proposal`

- **Input**: `ProposalId`, plus for `reject_proposal` a `reason: String`.
- **Output**: for `approve_proposal`, the `CommitOutcome` from the
  underlying `storage::propose_and_commit` call (real type, confirmed
  at `crates/storage/src/propose.rs`); for `reject_proposal`, `Ok(())`.
- **Errors**: `PermissionDenied` if `WritePolicy.approve_write_allowed`
  is false — checked BEFORE the proposal is loaded, so an unauthorized
  caller learns nothing about the proposal's existence or state
  (restated from the design doc); `NotFound` for an unknown proposal
  id; `InvalidInput` for an already-resolved or expired proposal
  (expiry re-checked at approval time regardless of persisted status,
  per the design doc's defense-in-depth requirement); `Conflict` if a
  concurrent approval already resolved the same proposal.

## Versioning / backward-compatibility rules

- This document's version is its filename date (`2026-11-15`, the plan
  task's date — the freeze itself happened 2026-11-16; the filename
  stays as the plan originally named it to match the task, with this
  status line recording the real freeze date).
- Any change to an existing capability's input/output shape, error
  set, or permission requirement requires a new dated freeze document,
  not an in-place edit of this one — this one is immutable once
  approved, matching the "FROZEN" status line.
- Adding a 13th capability requires the same: a new freeze, explicit
  user approval, never silently folded into this document or into
  `brain-api`'s implementation without one.
- `get_rules`/`get_regulations`/`get_canon`'s underlying storage
  mechanism (how canonization is recorded, whether `NoteType` grows new
  variants) is explicitly NOT frozen by this document — only the
  logical contract (inputs, outputs, precedence, empty semantics) is.
  A future, separate, minimal task may introduce the storage mechanism
  without requiring a new contract freeze, as long as the logical
  contract here is preserved.

## Non-goals (explicit)

- This document does not specify exact numeric limits (query length,
  result count, traversal depth, payload size, timeout, concurrency) —
  those are plan Task 35's job, against the capability list frozen here.
- This document does not specify the MCP wire schema (tool names,
  JSON-RPC shapes) — that is plan Task 22/23's job, mapping 1:1 onto
  the capabilities frozen here.
- This document does not implement or redesign `NoteType`, graph query
  grammar, or any underlying crate's real API — it freezes the
  contract `brain-api` exposes on top of what those crates already do,
  verified against their real signatures where already confirmed, and
  explicitly deferred to the relevant later plan task where not yet
  confirmed (graph_query/get_entity/get_code_context's exact
  parameter shapes, at plan Task 17).

## Self-review against design doc + real code

- Cross-checked every capability against `docs/superpowers/specs/2026-11-15-b5-brain-api-mcp-design.md`'s
  own capability table: no divergence found; this document is strictly
  more detailed, not contradictory.
- Cross-checked `domain::Relations`'s real field names
  (`crates/domain/src/note.rs:13-24`) before writing `link_entities`'s
  input schema — used the real five field names, not invented ones.
- Cross-checked `domain::NoteType`'s real 7 variants before writing
  `get_previous_solution`'s filter — did not invent a `NoteType::Fix`
  or `NoteType::Test` variant the brief's prose mentioned but the real
  enum does not have; named the two real variants that actually fit
  (`Bug`, `Decision`) and left the other 5 un-excluded by construction
  rather than fabricating new ones.
- Confirmed `codegraph_domain`'s real exports
  (`crates/codegraph-domain/src/lib.rs:8-19`) before referencing them
  in `graph_query`/`get_entity` — used real names
  (`SymbolSnapshotId`, `EdgeKind`, `SymbolKind`), explicitly deferred
  the exact query-grammar shape to plan Task 17 rather than inventing
  one now.
- No capability introduces an error variant outside the frozen B5.14
  set.
- No capability bypasses the trust model frozen in the design doc; the
  proposal-creating three and the approval pair are restated, not
  altered.
