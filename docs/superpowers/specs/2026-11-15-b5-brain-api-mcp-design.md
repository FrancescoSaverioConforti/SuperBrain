# B5 — Brain API + MCP Integration: Design

Status: approved (trust model + architecture decisions confirmed by user 2026-11-15)
Branch: `b5-brain-api-mcp` (worktree already exists at `.worktrees/b5-brain-api-mcp/`, clean at `main` HEAD `b58820b`)
Continuity decisions this design implements: `d238mm`, `d239a3`, `d240mr`, `d241qo`, `d242y7`; constraints `c29zz`, `c30so`.

## Intent and why

Make SuperBrain usable by external coding agents over MCP, and by a future SuperBrain UI (B6), without weakening the security/privacy guarantees built in A1–A3 or the DenseV2 retrieval guarantees closed in B4. The user runs SuperBrain as a personal, local-first memory system; B5 is the first phase that exposes it to a caller that is not the trusted local process — so every new surface must assume the caller is untrusted until proven otherwise, and no new surface may create a path to modify the vault except through the one audited write path that already exists (`storage::propose_and_commit`) plus the new deferred-approval path this design adds.

## Goals

- A frozen, documented Brain API contract covering 12 capabilities (4 already in `docs/Spec_mvp.md`, 8 newly authorized: `get_entity`, `get_previous_solution`, `get_rules`, `get_regulations`, `save_memory`, `update_memory`, `link_entities`, `get_canon`), with one service layer used by both internal callers and the MCP transport.
- A real MCP server exposing the read-shaped capabilities directly and the three write-shaped capabilities (`save_memory`, `update_memory`, `link_entities`) only as proposal-creation.
- A `PendingWriteProposal` mechanism: agents (local or remote, over MCP) can propose a write; only a trusted human approval surface can approve it; approval triggers execution through the existing storage/policy path.
- A minimal MCP client foundation (server registration, permission-gated external tool invocation) — separate component, cannot bypass Brain API/policy.
- SEC-001/002/003 closed first, as prerequisite phase `B5.SEC`, because the new proposal storage and the MCP read surface must not be built on the currently-vulnerable path-based check.

## Non-goals

- No SuperBrain UI (B6's job). B5 prepares the API surface B6 will call; it ships a CLI approval path only, sufficient for E2E testing.
- No multi-agent/multi-user identity framework. Today's binary trust split (trusted-local vs. remote-untrusted) gets one new capability (`ApproveWriteProposal`), not a general actor/identity system.
- No generalization of `Note`'s persistence into a shared typed-record framework. Proposals get their own small, dedicated storage module.
- No reopening of B3/B4/DenseV2 behavior, no change to the frozen perimeter in `docs/Spec_mvp.md` beyond the explicitly authorized 8-capability extension and the explicitly authorized write-proposal trust-model addition.

## Anti-goals (would be failure even if "it works")

- An MCP tool that can execute a write without a human approval step, however indirect (e.g. a tool that calls another tool that writes).
- A proposal whose `status` field can be set to `Approved` by anything other than the trusted human approval surface's dedicated code path.
- New proposal file I/O that inherits the existing TOCTOU gap — i.e., shipping B5 read/write surfaces before `B5.SEC` closes.
- Silent capability creep: any MCP tool whose effect isn't 1:1 (or documented N:1) traceable to a named Brain API operation.

## Constraints (from continuity, frozen)

- `c1p2`: `storage::propose_and_commit` remains the only function that writes committed Note bytes to the vault. The new proposal-approval execution path calls into it (or an equivalent policy-gated commit path) — it does not bypass it.
- `c7ec`: MVP perimeter frozen; the 8-capability extension and the write-proposal trust model are the only authorized exceptions, recorded in `d239a3`/`d238mm`.
- `d9mm`: read paths never fall back silently on hash mismatch.
- `c29zz`/`c30so`: agent side can never self-approve, impersonate the approval surface, escalate read→write, bypass approval via another API call, or mutate proposal status directly in storage; none of B5's implementation choices are further trust-model revisions.
- Security audit (`docs/security/security-audit.md`, 2026-11-01): AP-02/03/04/05 flagged Not Implemented, explicitly "P1 before MCP." B5.7 (permission model) and B5.9 (external tool authorization) must leave these addressed for the surfaces B5 actually ships (local MCP server/client transport, no sandboxed tool execution yet — that remains out of scope until an `agent` crate exists).

## Phase ordering (gate)

```
B5.SEC (Filesystem Security Closure) — BLOCKS everything below
   │
   ▼
B5.0  Freeze Brain API contract (12 capabilities, schemas, errors)
   │
   ▼
B5.1  Brain service layer (one service, two callers: internal + MCP)
   │
   ├─▶ B5.3  Search/retrieval tools (search, retrieve, get_code_context)
   ├─▶ B5.4  Knowledge/graph tools (graph_query, get_entity, link_entities-as-proposal)
   ├─▶ B5.5  Memory/proposal tools (get_previous_solution, save_memory/update_memory-as-proposal)
   ├─▶ B5.6  Rules/regulations/canon (get_rules, get_regulations, get_canon)
   │
   ▼
B5.2  MCP server (wires B5.3–B5.6 as tools/resources)
   │
   ▼
B5.7  Permission model hardening  ──▶ B5.8/B5.9 MCP client foundation + external tool auth
   │
   ▼
B5.10 server/client separation verification
   │
   ▼
B5.11 E2E flows  ──▶ B5.12 audit/observability  ──▶ B5.13 adversarial  ──▶ B5.14 error semantics
   │
   ▼
B5.15 packaging/docs ──▶ B5.16 limits ──▶ B5.17 mutation testing ──▶ B5.18 final verification
   │
   ▼
Closure document
```

## Approaches considered

### A. SEC-001/002/003 fix: handle-based verification (chosen)

Replace the check-then-use pattern in `record_path_checked`/`before_image_path_checked` (`crates/storage/src/vault.rs:2044-2151, 2177`) with a primitive that opens without following a reparse point/symlink at the final component, then verifies the **already-open handle**: regular file, link count 1, resolved real path inside the vault root. This directly extends the existing `identity_of_open_file`/`FileIdentity` pattern already proven in `crates/vault-security/src/deletion_log.rs:574-597` (confirmed real, read in full) — that function today checks device+inode equality but does not check link count or reject reparse points, so it is necessary-but-not-sufficient and must be extended, not replaced wholesale.

- **Trade-off:** touches a stable, previously-reviewed crate (`storage`) and a security-critical path; requires careful adversarial testing (listed below) and Windows-specific API (`GetFileInformationByHandle` for link count, `GetFinalPathNameByHandleW` for resolved path — both already available via the `windows` crate dependency pattern `vault-security` already uses).
- **Why chosen:** it's the only approach that actually closes a TOCTOU race (a second check-then-use, even a tighter one, is still check-then-use). The user explicitly approved this as prerequisite (`d242y7`).

### B. Mitigate without closing (rejected)

Narrow the race window (e.g. re-check immediately before I/O) or add monitoring/alerting on suspicious symlink activity without removing the fundamental check-then-use race.

- **Rejected:** does not close the race, just shrinks it; the user explicitly rejected proceeding with B5 on the vulnerable pattern (`d242y7` — "NON costruire il nuovo proposal storage sopra il pattern vulnerabile attuale").

### Proposal persistence: dedicated `.proposals` store (chosen) vs. generalized typed-record framework (rejected)

Chosen: `root/.proposals/{proposal_id}.proposal`, its own small module in `storage`, reusing `atomic_write` (`crates/storage/src/atomic.rs:28`, once that primitive's rename-destination handling is itself verified-safe per B5.SEC) and the (fixed) root-confinement check, but with proposal-specific status-transition validation that `Note`'s code has no equivalent of. Rejected generalizing `Note`'s hand-written persistence into a shared abstraction both types ride on — continuity `d241qo` explicitly rejects this; `Note`'s persistence code is stable and reviewed, and forcing a second type through it now is exactly the kind of scope creep the "boring tech" / "don't design for hypothetical future requirements" discipline warns against.

### Approval authorization: new Capability variant (chosen) vs. full actor/identity type (deferred)

**Correction (2026-11-15, post-builder-check):** an earlier recon pass reported `policy::Capability`, `policy::PolicyContext`, `ctx.actor_is_trusted_local`, and a `Gate::evaluate(capability, ctx)` signature as real, citing specific file:line locations. A builder-check agent, and this author's own direct re-read of every cited file, confirmed **none of that exists**. The real `policy` crate (`crates/policy/src/lib.rs`, 11 lines, pure re-exports) has:
- `Gate::evaluate(&self, note: &domain::NoteMetadata, content: &str, link_state_result: Result<LinkState, LinkError>, store: EncryptedStoreAvailability) -> GateDecision` (`crates/policy/src/gate.rs:69-104`) — a **content-admission** gate (schema/link/classification/secret-scan), unrelated to per-action write permission or actor trust.
- `ReadPolicy { search_allowed: bool, retrieve_allowed: bool }` / `ReadOperation { Search, Retrieve }` (`crates/policy/src/read.rs`) — the only actual permission-check mechanism today, read-only, with no actor/trust concept at all.
- No `Capability` enum, no `PolicyContext`, no `actor_is_trusted_local`, no `WriteNote`/`DeleteNote`/`ReconcileVault` variant, anywhere in the repo (grep across all crates and the `b5-brain-api-mcp` worktree: zero hits).
- `api::RequestContext` is `{ schema_version: u32, correlation_id: Uuid }` (`crates/api/src/lib.rs:36-40`) — it carries no policy reference at all; `ReadPolicy`/`ReadOperation` are passed as separate parameters to `check()` (`api/src/lib.rs:76-88`).
- `storage::propose_and_commit` (`crates/storage/src/propose.rs:179`, real, confirmed) is gated only by `policy::Gate` (content admission), not by any actor/trust check — there is no concept today of "is this caller trusted-local" anywhere in the write path.

**Consequence:** the write-permission/trust-actor mechanism this design needs is not an extension of an existing pattern — it is new design work, built on real but much thinner existing pieces: `ReadOperation` (extend with 9 new read-shaped variants: `GraphQuery`, `GetEntity`, `GetCodeContext`, `GetPreviousSolution`, `GetRules`, `GetRegulations`, `GetCanon`, plus the 2 existing), and a **new, separate mechanism** for the write/approval side, since `ReadPolicy`'s two bools have no room for "may write" or "may approve" and conflating them with read-allow flags would blur exactly the read→write escalation boundary the anti-goals forbid.

**Chosen shape**, grounded in what's real:
- A new `policy::WritePolicy` (sibling to `ReadPolicy`, not a merge into it) carrying the only two write-shaped permissions B5 needs: `propose_write_allowed: bool` and `approve_write_allowed: bool`. Constructed exclusively by the API-layer boundary code from the transport the call arrived on — an MCP-originated call is always built with `approve_write_allowed: false` (hard-coded at the construction site, never a caller-supplied flag) and a CLI-originated call is the only path that can construct one with `approve_write_allowed: true`. This mirrors the one real precedent that does exist for "boundary code asserts trust, transport never self-asserts it": `ReadPolicy`'s own module doc ("Local read policy supplied by the trusted single-user host... not an authorization inferred from retrieved text", `policy/src/read.rs:1-2`).
- `WriteOperation { ProposeWrite, ApproveProposal, RejectProposal }`, checked the same mechanical way `ReadOperation` is checked today (`policy.allows(op)`), not routed through `Gate::evaluate` (which stays scoped to content admission, unchanged).
- `api::RequestContext` grows a field to carry which policy applies (today it carries none at all — "today it carries only `ReadPolicy`" in an earlier draft of this document was also wrong, corrected here): e.g. `pub write_policy: Option<policy::WritePolicy>`, `None` for pure-read calls.

Deferred: a full `Identity`/`Actor` type naming individual MCP clients/servers distinctly. Continuity `d240mr` explicitly defers this to B6 when the UI needs to show "Agent X wants to...", which requires naming agents distinctly; B5 only needs a yes/no "may this call approve," which the two-bool `WritePolicy` above provides without inventing an actor system.

## What's new vs. what's reused (grounded in real code, verified by direct file reads)

**Reused as-is** (each independently re-verified against the real file after the builder-check correction above):
- `storage::propose_and_commit` (`crates/storage/src/propose.rs:179`) remains the only function that commits Note bytes; proposal-approval execution calls into it, never around it.
- `atomic_write` (`crates/storage/src/atomic.rs:28`) — proposal file I/O reuses this, after B5.SEC fixes its rename-destination safety.
- `domain::WriteState`/`valid_transition` (`crates/domain/src/write_state.rs`) is the real stylistic precedent for a validated state machine in this codebase — not a `VerificationState` (that type does not exist). `ProposalStatus`'s transitions should be validated the same explicit-pairwise-match way `valid_transition` does it (`write_state.rs:12-32`), rather than trusting callers to only ever construct valid next-states.
- `storage::note_id::NoteId(String)` (`crates/storage/src/note_id.rs:33`, a private-field newtype with validating constructors, not the public-tuple-field newtype an earlier draft assumed) is the real precedent `ProposalId` should follow: validated construction, no bare `pub struct ProposalId(pub String)`.
- `ReadPolicy`/`ReadOperation`'s module-doc precedent (`policy/src/read.rs:1-2`: "supplied by the trusted single-user host... never inferred from retrieved text") — the load-bearing precedent for why an MCP-originated call can never self-assert `approve_write_allowed: true`; policy objects are always constructed by boundary code, never by the transport.
- `fn(RequestContext, ...) -> ApiResponse<T>` / `fn check(context, policy, op) -> Result<(), ApiError>` shape (`crates/api/src/lib.rs:76-95`) — new operations follow this, extended with the new `WritePolicy`/`WriteOperation` parameters alongside the existing `ReadPolicy`/`ReadOperation` ones.
- `identity_of_open_file`/`FileIdentity` device+inode check (`crates/vault-security/src/deletion_log.rs:564-597`, confirmed real on direct read) — B5.SEC extends this exact function (add link-count + reparse-point + resolved-path checks) rather than writing a parallel mechanism.

**Genuinely new (no existing precedent, confirmed by grep across the whole workspace and the `b5-brain-api-mcp` worktree):**
- The entire write-permission/approval mechanism: `policy::WritePolicy`, `policy::WriteOperation`. There is no existing `Capability` enum, `PolicyContext`, or actor-trust boolean anywhere in the codebase — B5.1/B5.7 must design and build this from nothing, not extend something that exists. (This corrects the earlier draft of this document, which cited a `Capability`/`PolicyContext` mechanism that a builder-check and direct file verification showed does not exist on `main` or in the `b5-brain-api-mcp` worktree.)
- `PendingWriteProposal` type, its `ProposalId`/`ProposalStatus` types, and its dedicated storage module.
- Any notion of "originating MCP client/server identity" as a named, carried value (the proposal record needs to *record* an opaque originating-identity string for audit/provenance even though nothing yet treats distinct identities differently — approval authority stays boolean, not actor-keyed, per the deferred-to-B6 decision above). **Provenance of this string (user decision, 2026-11-15):** it is derived from the MCP connection/session binding established at B5.8's server-registration/connection-lifecycle layer, never from a field the MCP client sets in its request payload — the same anti-self-assertion principle already applied to `approve_write_allowed` applies here. A spoofed or absent binding (e.g. during early B5.2 development before B5.8 exists) must produce a clearly-marked placeholder identity, never a value indistinguishable from a real one.
- The MCP server and client themselves (zero existing code — confirmed, no `mcp` references anywhere except docs/spec mentions and the security audit's threat-model rows).
- The `RequestContext` field needed to carry `Option<WritePolicy>` — today the struct carries no policy reference of any kind (not "only `ReadPolicy`," as an earlier draft said); `ReadPolicy`/`ReadOperation` are passed as separate `check()` parameters, not struct fields.
- `domain::Provenance` does not exist as a reusable type (an earlier draft of this document was wrong about this too) — the two `Provenance`-named types in the workspace live in `codegraph-domain::evidence` and `index::search` and are unrelated to notes or proposals. The proposal's provenance field needs its own small struct (e.g. `{ source: String, created_at: String }`), modeled on `NoteMetadata`'s existing flat `source`/`created` string fields (`crates/domain/src/note.rs:27-40`), not on a nonexistent shared type.

## Brain API contract (B5.0 scope — to be fully specified as its own frozen doc, outline here)

12 capabilities, each getting: stable name, input schema, output schema, typed error schema (from the frozen set: NotFound/InvalidInput/PermissionDenied/RuntimeUnavailable/DenseUnavailable/ExternalToolUnavailable/Timeout/Conflict/CorruptData/UnsupportedOperation), permission requirement, determinism statement, empty/not-found semantics, pagination/limits.

| Capability | Shape | Permission | Notes |
|---|---|---|---|
| `search` | read, direct | `ReadOperation::Search` | Already real (`api::search`); DenseV2 mode unchanged from B4 |
| `retrieve` | read, direct | `ReadOperation::Retrieve` | Already real (`api::retrieve`) |
| `graph_query` | read, direct | `ReadOperation::GraphQuery` (new variant) | New; bounded traversal (depth/result limits — B5.16) |
| `get_entity` | read, direct | `ReadOperation::GetEntity` (new variant) | New |
| `get_code_context` | read, direct | `ReadOperation::GetCodeContext` (new variant) | New; built on existing `codegraph-index` |
| `get_previous_solution` | read, direct | `ReadOperation::GetPreviousSolution` (new variant) | New; provenance-linked lookup |
| `get_rules` | read, direct | `ReadOperation::GetRules` (new variant) | New; scope = engineering constitution, versioned |
| `get_regulations` | read, direct | `ReadOperation::GetRegulations` (new variant) | New; scope = external regulations, distinct source from get_rules |
| `get_canon` | read, direct | `ReadOperation::GetCanon` (new variant) | New; distinct from both rules and regulations |
| `save_memory` | **proposal-creating** | `WriteOperation::ProposeWrite` (new type, see Approaches below — not an extension of anything that exists) | Returns `ProposalId`, not a committed Note |
| `update_memory` | **proposal-creating** | `WriteOperation::ProposeWrite` | Same |
| `link_entities` | **proposal-creating** | `WriteOperation::ProposeWrite` | Same |
| *(new, not in user's list but required by the trust model)* `approve_proposal` / `reject_proposal` | **execution** | `WriteOperation::ApproveProposal` / `RejectProposal` | CLI-only surface in B5; callable by B6 UI later |

`get_rules` vs `get_regulations` vs `get_canon` precedence/scope must be nailed down in B5.0 against whatever `docs/Spec_mvp.md` or other docs already define "engineering constitution" / "project rules" / "canon" as — this needs one more grounding pass in B5.0 itself (flagged as builder-check-worthy, not resolved here) since the recon so far hasn't located a formal definition of these three nouns' boundaries.

## PendingWriteProposal (concrete shape)

```rust
// Validated newtype, following storage::note_id::NoteId's real shape
// (private field + constructor), not a bare pub tuple struct.
pub struct ProposalId(String);

// No reusable domain::Provenance exists (verified: that name doesn't appear
// in domain::lib.rs's exports); proposals get their own small struct,
// modeled on NoteMetadata's existing flat source/created string fields.
pub struct ProposalProvenance {
    pub source: String,
    pub created_at: String,
}

pub struct PendingWriteProposal {
    pub id: ProposalId,
    pub originating_identity: String,   // opaque, audit-only in B5; not trust-bearing
    pub operation: ProposedOperation,   // SaveMemory { .. } | UpdateMemory { .. } | LinkEntities { .. }
    pub provenance: ProposalProvenance,
    pub expires_at: Option<String>,
    pub status: ProposalStatus,
}

pub enum ProposalStatus {
    Pending,
    Approved { approved_at: String },
    Rejected { reason: String, rejected_at: String },
    Expired,
}

// Validated the same explicit-pairwise-match way domain::write_state::valid_transition
// does it, not left to callers to only ever construct valid next-states.
pub fn valid_proposal_transition(from: &ProposalStatus, to: &ProposalStatus) -> bool { /* ... */ }
```

Storage: `root/.proposals/{proposal_id}.proposal`, atomic write, status transitions validated server-side via `valid_proposal_transition` (only `approve_proposal`/`reject_proposal` — gated by `WriteOperation::ApproveProposal`/`RejectProposal` — may move Pending → Approved/Rejected; nothing moves a proposal backward). Expiry is evaluated on every read, not a background job (deterministic/offline-safe), and specifically: **both** the listing surface the CLI/UI approval tool uses (so a stale proposal never shows as approvable) **and** `approve_proposal`/`reject_proposal` themselves re-check `expires_at` against current time before acting, regardless of the persisted `status` value — a proposal whose persisted status is still `Pending` past its `expires_at` must be rejected as expired at the point of approval, not just at the point of listing, so a race between "listed as pending" and "approved" can never approve something that should already read as expired. The permission check (`approve_write_allowed`) happens before the proposal is loaded or its transition validated, so an unauthorized caller is denied without learning anything about whether the proposal exists or what state it's in.

## MCP server/client separation

Server exposes SuperBrain's own 12 capabilities (9 direct reads, 3 proposal-creating) plus the CLI-only approval pair, never reachable from the MCP transport itself (approval requires `WritePolicy.approve_write_allowed == true`, which only CLI-origin boundary code ever constructs — an MCP-originated `RequestContext` is always built with `approve_write_allowed: false`, enforced at the API layer construction site, not by trusting a caller-supplied flag). Client is a separate component for SuperBrain to call *external* MCP tools under its own permission gate (B5.8/B5.9); no capability of one automatically propagates to the other, verified by an explicit architecture test in B5.10.

**Crate boundary (user decision, 2026-11-15):** `crates/api/src/lib.rs`'s own module doc states it is "B1 callable core: local single-user read-only functions, no transport" (api/src/lib.rs:1) — this contract is preserved unchanged. A new sibling crate, `crates/brain-api`, owns everything write/proposal-shaped:

```
MCP server
   │
   ▼
crates/brain-api   (orchestration/service layer; the Brain API's public contract)
   ├─ delegates reads to crates/api as-is
   ├─ proposal creation (save_memory/update_memory/link_entities)
   ├─ proposal inspection
   ├─ approval/rejection (CLI-only origin in B5; B6 UI later)
   ├─ approved-write execution (calls storage::propose_and_commit)
   ├─ WritePolicy/WriteOperation enforcement
   ├─ typed errors (the frozen set from B5.14)
   └─ provenance/audit hooks
   │
   ▼
crates/api / crates/storage / crates/index / crates/policy / crates/domain
```

`crates/api` is not modified beyond what's already compatible with its existing read-only contract (small read-only extensions are fine if consistent with that contract; write/proposal semantics never go into `api`). The future B6 UI also calls `brain-api`, never storage directly — same rule as the MCP server: no caller of any kind reaches `storage::propose_and_commit` except through `brain-api`'s approval-gated execution path. `brain-api` depends only on what it actually needs (`api`, `storage`, `policy`, plus `domain`/graph/index crates as needed) — no circular dependencies.

Required tests for this layering (B5.1): read delegation to `api` behaves identically to calling `api` directly; proposal creation persists correctly; approval is denied for a remote/MCP-originated context; approval succeeds only with the explicit `approve_write_allowed` capability; an approved proposal executes exactly once (no double-execution); a rejected or expired proposal cannot execute; no code path reaches `storage` bypassing `brain-api`.

## What's left to the builder (B5.0 and later phases)

- Exact input/output JSON schemas for all 12 capabilities (B5.0).
- `get_rules`/`get_regulations`/`get_canon` precedence and scope definitions, grounded against real existing docs, not invented (B5.0 — flagged above as needing one more grounding pass).
- Exact Windows reparse-point/junction rejection mechanism and the Unix equivalent of `GetFinalPathNameByHandleW` (B5.SEC — the prior recon flagged this as a genuinely open implementation question, not a design question, since it's a "which syscall" choice bounded by the design's required semantics, not a trade-off the user needs to pick between).
- Graph traversal limits, MCP payload size limits, concurrent request limits (B5.16).
- Audit event schema (B5.12) — minimized, no sensitive content, structured.
