# B2 — Code Graph: design and operational plan

Status: design/planning only. No B2 code, branch or worktree created by this
document. `docs/Spec_mvp.md` not modified. Builds on the approved P0 closure
(`3ca2203`, merged at `d217f93b`, tag `b1-complete`) and the P1 checkpoint
(`docs/security/p1-checkpoint-pre-b2.md`): **GO for B2 planning, NO-GO for
implementing the surfaces gated by S1/S2/S3 below** until those close.

Scope per `docs/Spec_mvp.md` §Fase B, punto 3 e riga 429: tree-sitter
indexer (Python, C#, C++, JS/TS, HTML/CSS, Rust), code graph
(`symbol_snapshot_id`, `symbol_lineage_id`, typed edges with
method/evidence/confidence/commit), incremental reindex by hash with
line-reference invalidation after a commit change, `graph_query` +
`get_code_context` on the `api` crate, read-only MCP server for VS Code.

## 1. Architecture proposed

B2 adds two new workspace members, following the existing crate boundary
discipline (`storage` owns identity/durability, `index` owns derived
in-memory structures fed only by callbacks, `api` owns the policy-gated
callable surface — see `crates/storage/src/vault.rs:508` `Indexer` trait and
`crates/index/src/indexer_impl.rs`'s `RealIndexer`):

```
crates/
  codegraph-domain/   # NEW: validated identity types, typed edges, lineage
                       # semantics — mirrors crates/domain's role (schema,
                       # NoteId-style parsing), no I/O, no tree-sitter dep.
  codegraph-index/     # NEW: tree-sitter parsing, symbol extraction, the
                       # in-memory graph, incremental reindex, graph_query
                       # execution — mirrors crates/index's role (RealIndexer
                       # pattern: single publication lock, no callback into
                       # the authoritative source).
  api/                 # EXTENDED: graph_query, get_code_context added
                       # alongside existing search/retrieve, same
                       # RequestContext/ApiResponse/ReadPolicy shape.
  storage/             # EXTENDED ONLY if D2 requires a new Indexer-like
                       # callback seam for repository ingestion (see Task 2).
```

Why two new crates rather than folding into `domain`/`index`: `domain` and
`index` are already scoped to the Markdown-note model (`NoteMetadata`,
`NoteId`, trigram/BM25 `Document`). Code-graph identity (`symbol_snapshot_id`,
commit hashes, repository paths) is a different domain with different
invariants (cross-commit lineage, multi-language parsing) — mixing them
would force `domain`/`index` to depend on tree-sitter or vice versa, which
the existing `storage` module doc explicitly avoids doing across crate
boundaries for the temporal grammar (P1 checkpoint, D1: "il parser canonico
non deve dipendere da `index` in `domain`" — same discipline applies here).

Ingestion model: B2 does **not** make the source-of-truth repository a new
kind of `Vault` note. A repository's files live on disk under their own
path (not under the Markdown vault root), and `codegraph-index` reads them
directly via a `RepositorySource` abstraction (Task 3), the same way
`RealIndexer` reads Vault record bytes only through callbacks, never by
walking the filesystem itself. This keeps `collect_vault_files`-style raw
walks (Gate S2) out of the hot ingestion path by construction: B2 never
calls that function, and Gate S2 is about what *backup* does with whatever
ends up near the vault root, not about how B2 reads a repository.

## 2. Security gates (hold across the whole plan)

### Gate S1 — SEC-001/002/003 (write-path containment)

Blocks: Task 9 (incremental indexing — any step that WRITES graph state to
a Vault-adjacent persistent store) and Task 10 (persistence/integration).
Does not block Tasks 1–8 (pure in-memory parsing/graph/query logic has no
write path yet). The fix must land as a single coherent change to
`write_record_unchecked`'s pre-read (handle-relative containment on the
write side, not just canonicalization), with hard-link coverage, before
Task 9 starts. Planning for Task 9 may proceed; implementation may not.

### Gate S2 — Backup/restore provenance

Blocks: any task that places B2-derived artifacts (graph snapshots, parse
caches, per-repository indexes) in a location backup's `collect_vault_files`
would walk. Task 10 (persistence/integration) must choose a storage location
and format explicitly designed to avoid this (see Task 10's design note:
either keep B2 state entirely outside anything backup walks, derived
on-demand from the repository + a small authoritative journal, or wait for
S2's fix before co-locating with the vault). Does not block Tasks 1–9 if
Task 9's incremental index lives in memory/rebuildable cache, not in a
backed-up location.

### Gate S3 — Resource budgeting

Blocks: Task 11 (`get_code_context`), Task 12 (`graph_query`), Task 14 (MCP
adapter). Does not block Tasks 1–10 (internal parsing, graph construction
and incremental indexing are not yet exposed as a callable). Budgets (max
traversal depth, max nodes/edges, max returned context, query/work budget,
cancellation, incremental reindex budget) must be designed as part of Task
13 and implemented before Task 11/12 are wired into `api`, even if Task
11/12's internal logic is written and unit-tested earlier with budgets as
explicit parameters from the start (not retrofitted).

## 3. Architectural decisions (D1–D5)

### D1 — Raw → Validated

New validated types, each with a single `parse`/constructor following
`NoteId::parse`'s discipline (`crates/storage/src/note_id.rs:63`: validates,
never normalizes; a canonical form; specific typed errors per rejection
reason):

| Type | Canonical form | Rejects |
| --- | --- | --- |
| `RepositoryId` | opaque id derived from a validated root path, not the path string itself | path traversal, non-existent root, root outside configured allowlist |
| `CommitHash` | exactly 40 or 64 lowercase hex chars (git SHA-1/SHA-256) | wrong length, non-hex, uppercase |
| `FileIdentity` | `(RepositoryId, relative path validated like `NoteId` — no `..`, no absolute, no reserved device names)` + `CommitHash` it was read at | path traversal, path outside repository root |
| `SymbolSnapshotId` | hash over `(RepositoryId, FileIdentity, CommitHash, qualified_name, signature)` — opaque, not the raw qualified name | construction only via the extractor, never parsed from free text |
| `SymbolLineageId` | opaque id stable across commits for symbols the lineage resolver has linked | never user-supplied; only the resolver (Task 6) constructs one |
| `SymbolKind` | closed enum (Function, Method, Class, Struct, Enum, Trait/Interface, Module, Const, Macro — extensible per language, but closed per release) | any string not in the enum; no "other" catch-all that silently accepts anything |
| `EdgeKind` | closed enum (Calls, Imports, Implements, Extends, References, TestsFor, RenamedTo — see D3) | same as `SymbolKind` |
| `Evidence` | struct: `method: ResolutionMethod` (closed enum: StaticAnalysis, NameHeuristic, Inferred), `source_span: Option<(FileIdentity, LineRange)>`, free-text `note` capped and sanitized like `ApiError`'s Display (no raw user content echoed) | unbounded free text, missing method |
| `Confidence` | closed enum or bounded `0.0..=1.0` newtype with explicit `Certain`/`Likely`/`Speculative` bucket, never a raw float exposed without a bucket | NaN, out-of-range, silent default |

Rule carried over from the P1 checkpoint (CF-02/CF-03/ST-07 root cause:
"allowlist di nomi senza tipi validati"): **no field in the public B2 API or
persisted format is a bare `String` standing in for one of the above.** The
parser/extractor (Task 4) produces raw candidates; a validated constructor
is the only path into the typed model, mirroring `domain`'s
`NoteMetadata`-vs-validated-temporal split recommended in the P1 checkpoint.

### D2 — Single writer vs multi writer

**Decision: single authoritative writer**, per the stated preference.
`codegraph-index`'s incremental reindexer runs in the same process and
(where it touches anything Vault-adjacent) the same `Vault` handle as the
rest of SuperBrain — there is no separate watcher/hook process writing
concurrently. This is enforced architecturally, not just by convention:

- The reindexer is invoked synchronously from the same process that holds
  the `Vault`/`PiiStore`/`DeletionLog` handles (e.g. triggered by an
  explicit "reindex repository" call, or a future same-process file-watcher
  thread inside that one process), never by a separate binary or external
  cron process writing to the same files.
- If a future requirement needs a genuinely separate process (e.g. a
  VS Code extension triggering reindex out-of-process), that is a NEW
  decision requiring the cross-process lock + concurrent-barrier tests the
  prompt requires before any such multi-writer implementation — not an
  incremental extension of this plan. This plan does not build that lock.
- Task 9's design must still use the same per-note-id `Mutex` discipline
  `Vault::note_lock` already uses for anything it touches that is shared
  with the note-write path, so a future move to multi-writer only requires
  adding the cross-process layer, not re-deriving the locking model.

### D3 — Lineage semantics

Structurally distinct from the start, not a boolean flag:

```rust
pub enum LineageStatus {
    Confirmed { method: ResolutionMethod, evidence: Evidence },
    Hypothesized { method: ResolutionMethod, evidence: Evidence, confidence: Confidence },
}
```

A `SymbolLineageId` is only ever constructed attached to a `LineageStatus`;
there is no API that strips the status and leaves a bare id. Per the P1
checkpoint's verdict on CF-06/metadata classification: **a `Hypothesized`
lineage edge never silently becomes `Confirmed` through storage, retrieval,
or round-tripping through `graph_query`.** Promotion from Hypothesized to
Confirmed requires a specific resolver re-run with new evidence (e.g. a
later commit makes a rename unambiguous), never a copy/serialize/deserialize
cycle. `graph_query` and `get_code_context` responses must surface
`LineageStatus` on every lineage-derived edge they return — never flatten it
to "related" the way a consumer might expect simplicity.

### D4 — DL-04 threat model for B2

B2 does **not** implement an anti-rollback anchor. The plan explicitly
states its threat model instead of pretending DL-04 is resolved:

> B2's code graph is derived, rebuildable data: a rollback of the
> underlying DeletionLog (DL-04's open risk) that resurrects a deleted note
> could, if that note is ever linked into the code graph (e.g. a design doc
> referenced from a comment), cause B2 to re-surface a reference to
> content that should stay deleted. B2 inherits this risk unchanged; it adds
> no new amplification pathway because B2 reads through the same gated
> `Vault`/`PiiStore` reader any such link would use, not a second channel.
> No B2 task attempts to detect or repair a DL-04 replay. If DL-04 is fixed
> independently of B2, no B2 code should need to change — the fix is
> entirely inside the DeletionLog/Vault boundary B2 reads through.

### D5 — MCP boundary

The `api` crate's internal Rust surface (`graph_query`, `get_code_context`)
is designed and implemented now (Tasks 11–12), following the exact
`check()`/`ApiResponse`/`ReadPolicy` shape already in `crates/api/src/lib.rs`
(new `ReadOperation::GraphQuery`, `ReadOperation::GetCodeContext` variants).
The MCP server itself (Task 14) is a separate, later task that **does not
start** until, in addition to Gate S3:

- read-only capability model is defined (what MCP tool calls map to which
  `ReadOperation`, 1:1, no implicit escalation);
- host/client identity for the MCP transport is defined (who is the
  "trusted host" supplying `ReadPolicy` over this transport — this is new
  surface the current `api` crate doesn't have, since B1's callers are
  in-process);
- an explicit allowlist of operations exists (no generic "run arbitrary
  graph_query" without the same per-operation gating `ReadPolicy` already
  does for search/retrieve);
- separation between retrieved content and capability/authority is
  documented: a `get_code_context` response containing source code text is
  data, never an instruction the MCP client or any future agent executes
  (same principle as AP-04 in the audit).

Task 14 is included in the breakdown below for completeness and dependency
tracking, but its implementation is blocked until the above are written
down and reviewed — this plan does not consider that review itself a B2
task, since it is a security-checkpoint activity, not a code task.

## 4. Task breakdown

Corrected from the prompt's suggested decomposition: merged "schema/types"
and "repository/file identity" into one task (D1's types are one coherent
unit, splitting them would create a half-typed intermediate state); split
"persistence/integration" into its own gated task separate from
"incremental indexing model" (S1/S2 gate persistence specifically, not the
in-memory incremental algorithm); added an explicit Task 0 for the
cross-crate seam decision, since B1's Task 1 ("pin the Indexer integration
seam") shows this project treats the seam as a dedicated first step, not an
assumption.

---

**Task 0 — Pin the ingestion seam**
- Objective: decide and document the exact trait/interface boundary between
  `codegraph-index` and (a) the filesystem/tree-sitter, (b) `storage`/`Vault`
  for any B2 state that does end up Vault-adjacent. Mirrors B1 Task 1.
- Files/crates: new `crates/codegraph-domain/src/lib.rs` (seam trait only),
  workspace `Cargo.toml` member list.
- API/types introduced: `trait RepositorySource { fn read_file(&self, id: &FileIdentity) -> Result<Vec<u8>, SourceError>; fn list_files_at(&self, repo: &RepositoryId, commit: &CommitHash) -> Result<Vec<FileIdentity>, SourceError>; }` — no implementation yet, just the seam.
- Test RED: a test double implementing the trait proves the seam is
  reachable from a (future) `codegraph-index` crate without that crate
  depending on `tree-sitter` or `git2` directly yet.
- Implementation minimum: trait + error enum only.
- Test GREEN: seam compiles and is callable from a throwaway test crate.
- Security invariants: none yet (no I/O).
- Mutation/review gate: none (scaffold only, same as B1 Task 1's review scope).
- Dependencies: none. Not gated by S1/S2/S3.

**Task 1 — Identity and schema types (D1)**
- Objective: implement `RepositoryId`, `CommitHash`, `FileIdentity`,
  `SymbolSnapshotId`, `SymbolLineageId`, `SymbolKind`, `EdgeKind`,
  `Evidence`, `Confidence`, `LineageStatus` per D1/D3 above.
- Files/crates: `crates/codegraph-domain/src/identity.rs`,
  `crates/codegraph-domain/src/lineage.rs`.
- API/types introduced: all of D1's table, plus `LineageStatus` (D3).
- Test RED: for each type, the `NoteIdError`-style exhaustive rejection
  tests (empty, traversal, wrong length/encoding, case where applicable) —
  written before the constructor exists.
- Implementation minimum: `parse`/constructor per type, no normalization,
  specific typed errors (mirrors `note_id.rs` exactly).
- Test GREEN: all rejection + acceptance cases pass; no type has a public
  constructor that accepts an un-validated `String`.
- Security invariants: no constructor silently normalizes or truncates
  (echoes ST-07's `schema_version as u32` truncation mistake if violated —
  explicit test for this per type).
- Mutation/review gate: mutation testing on every `parse` boundary
  condition (same bar as A1/A2's domain mutation gate).
- Dependencies: Task 0. Not gated by S1/S2/S3 (pure types, no I/O, no
  persistence).

**Task 2 — Repository/file identity resolution**
- Objective: implement `RepositorySource` for a real local filesystem +
  `git2` (already a workspace dependency per `storage/src/git.rs`) to
  resolve `FileIdentity`/`CommitHash` from an actual repository path.
- Files/crates: `crates/codegraph-index/src/repository_source.rs` (new
  crate created here).
- API/types introduced: `LocalGitRepositorySource` implementing Task 0's
  trait; `RepositoryAllowlist` (explicit, caller-supplied list of roots B2
  is permitted to read — this is the containment boundary for repository
  paths, analogous to the vault root boundary, decided here rather than
  left implicit).
- Test RED: path traversal via a crafted repo path outside the allowlist;
  symlink inside a repo pointing outside the repo root (containment for
  *repository reads*, distinct from Gate S2's *backup* containment — this
  one is required now, S2 is about backup specifically); non-git directory;
  detached/corrupt `.git`.
- Implementation minimum: canonicalize + containment check before every
  read, same discipline as `ensure_resolves_inside_root`, applied to
  repository roots instead of the vault root.
- Test GREEN: all traversal/symlink-outside-repo cases rejected; legitimate
  nested paths accepted.
- Security invariants: this task's containment check is independent of and
  does not substitute for Gate S1 (which is about the Vault's own
  write-path, not repository reads) or Gate S2 (backup).
- Mutation/review gate: targeted mutation on the containment check only.
- Dependencies: Task 0, Task 1. Not gated by S1/S2/S3 (read-only, not yet
  wired to any write path or exposed API).

**Task 3 — Tree-sitter abstraction**
- Objective: a language-agnostic parse-tree abstraction over tree-sitter
  grammars for the six required languages, isolated behind a trait so
  language addition doesn't change callers.
- Files/crates: `crates/codegraph-index/src/parse/mod.rs`,
  `crates/codegraph-index/src/parse/{python,csharp,cpp,js_ts,html_css,rust}.rs`.
- API/types introduced: `trait LanguageParser { fn parse(&self, bytes: &[u8]) -> Result<ParseTree, ParseError>; }`, one implementor per language.
- Test RED: malformed/truncated source per language does not panic; a
  valid minimal file per language produces a non-empty tree; a
  pathologically large/deeply-nested file (tree-sitter's own known
  worst-case input shapes) does not hang — this anticipates Gate S3's
  budget concern at the parser level even though S3 formally gates the
  query APIs, not parsing; cheap to test now.
- Implementation minimum: wrap each tree-sitter grammar crate; no symbol
  extraction yet, just a parse tree.
- Test GREEN: no panics/hangs on the adversarial corpus above; correct
  tree shape on valid minimal fixtures per language.
- Security invariants: parser failures are typed errors, never a panic
  (tree-sitter C bindings are `unsafe` FFI — same caution as DPAPI FFI in
  `vault-security/src/keys.rs`: isolate and never let FFI panics cross the
  boundary uncaught).
- Mutation/review gate: fuzz-style corpus review (malformed inputs), not
  full mutation testing (parser wrapping, not business logic).
- Dependencies: Task 0. Independent of Tasks 1–2 (can be built in
  parallel). Not gated by S1/S2/S3.

**Task 4 — Symbol extraction**
- Objective: walk a `ParseTree` per language and extract raw candidate
  symbols (name, kind, span, signature text) — the "Raw" side of D1's
  Raw→Validated split.
- Files/crates: `crates/codegraph-index/src/extract/mod.rs` + per-language
  extractors.
- API/types introduced: `RawSymbolCandidate { name: String, kind_hint: String, span: LineRange, signature_text: String }` — deliberately NOT yet a validated `SymbolSnapshotId`/`SymbolKind`; this type never crosses into the public API or persisted format (same pattern as the audit's recommended raw-DTO-vs-validated-metadata split).
- Test RED: known fixture file per language with N expected symbols; a file
  with zero symbols (e.g. a config file misidentified as code) returns
  empty, not an error; a file with a syntax error tree-sitter can still
  partially parse extracts what it can rather than failing closed on the
  whole file (document which behavior is chosen — partial extraction is
  the default here since Fase B's acceptance criteria value recall).
- Implementation minimum: per-language tree-sitter query patterns mapping
  tree nodes to `RawSymbolCandidate`.
- Test GREEN: fixture-based extraction counts match expected per language.
- Security invariants: `RawSymbolCandidate.signature_text` is bounded
  (same IX-05 concern: unbounded body text already flagged in the audit;
  cap signature/snippet length here at the source, don't defer the cap to
  the API layer).
- Mutation/review gate: per-language extraction logic reviewed for
  false-negative symbol kinds (undercounting is a correctness bug feeding
  Fase B's Recall@10/nDCG@10 acceptance criteria, not just a style issue).
- Dependencies: Task 3. Not gated by S1/S2/S3.

**Task 5 — Snapshot model (validated promotion)**
- Objective: the single choke point that turns a `RawSymbolCandidate` +
  `FileIdentity` + `CommitHash` into a validated `SymbolSnapshotId` +
  typed `SymbolKind`, per D1.
- Files/crates: `crates/codegraph-domain/src/snapshot.rs`.
- API/types introduced: `SymbolSnapshot { id: SymbolSnapshotId, kind: SymbolKind, qualified_name: ValidatedName, signature: BoundedString, file: FileIdentity, commit: CommitHash, span: LineRange }`; `fn promote(candidate: RawSymbolCandidate, file: FileIdentity, commit: CommitHash) -> Result<SymbolSnapshot, PromotionError>`.
- Test RED: a `kind_hint` string not in `SymbolKind`'s enum is rejected,
  not silently mapped to a catch-all variant (direct echo of CF-04's
  "no Secret variant exists, parser rejects" discipline — unknown kind
  must be a typed `PromotionError`, never a default); oversized name/
  signature rejected, not truncated (ST-07's truncating-cast mistake,
  avoided here by construction).
- Implementation minimum: `promote` function, exhaustive kind mapping.
- Test GREEN: all per-language `kind_hint` strings map to a known
  `SymbolKind` or are explicitly rejected and logged as "unmapped kind for
  review" (a growth path for new languages/constructs, not a silent drop).
- Security invariants: this is the Raw→Validated boundary D1 requires; no
  caller outside this function constructs a `SymbolSnapshot` directly
  (enforce via module privacy, not just convention).
- Mutation/review gate: mutation testing on `promote`'s rejection branches.
- Dependencies: Task 1, Task 4. Not gated by S1/S2/S3 (still in-memory).

**Task 6 — Lineage model and resolver**
- Objective: implement the cross-commit lineage resolver producing
  `LineageStatus` (D3) — confirmed (e.g. identical qualified name + same
  file + consecutive commit parentage) vs. hypothesized (e.g. same
  signature, different file/name — a suspected rename/move) lineage edges
  between `SymbolSnapshot`s across commits.
- Files/crates: `crates/codegraph-domain/src/lineage_resolver.rs`.
- API/types introduced: `fn resolve_lineage(prev: &[SymbolSnapshot], next: &[SymbolSnapshot]) -> Vec<LineageEdge>`; `LineageEdge { from: SymbolSnapshotId, to: SymbolSnapshotId, status: LineageStatus }`.
- Test RED: an unambiguous same-name-same-file case resolves `Confirmed`;
  a renamed function (name changed, signature/body similar) resolves
  `Hypothesized` with `Confidence` reflecting similarity, never
  `Confirmed`; a deleted-then-unrelated-new-function-with-same-name is NOT
  linked (false lineage) when evidence is insufficient — explicit negative
  test, since Spec_mvp's acceptance criteria call out "a renamed function
  and a new commit: old references invalidated, new references present"
  as a required test.
- Implementation minimum: structural/name/signature similarity heuristic
  for the Hypothesized case; exact match for Confirmed.
- Test GREEN: rename test from Spec_mvp's own acceptance criteria passes;
  negative (no-link) test passes.
- Security invariants: D3's rule — no code path constructs a `Confirmed`
  `LineageEdge` from heuristic evidence alone.
- Mutation/review gate: mutation testing on the Confirmed/Hypothesized
  branch condition specifically (this is the highest-value mutation
  target in the whole plan — a flipped condition silently overclaims
  certainty).
- Dependencies: Task 5. Not gated by S1/S2/S3.

**Task 7 — Typed edges (non-lineage)**
- Objective: `Calls`, `Imports`, `Implements`, `Extends`, `References`,
  `TestsFor` edges (D1's `EdgeKind`), each carrying `Evidence` (resolution
  method: static analysis vs. name heuristic) per Spec_mvp's "chiamate
  risolte separate da quelle ipotizzate."
- Files/crates: `crates/codegraph-domain/src/edges.rs`.
- API/types introduced: `Edge { from: SymbolSnapshotId, to: SymbolSnapshotId, kind: EdgeKind, evidence: Evidence }`.
- Test RED: a resolved call (same-crate, unambiguous target) produces
  `Evidence::method = StaticAnalysis`; an ambiguous call (overloaded name,
  dynamic dispatch target) produces `Evidence::method = NameHeuristic`
  with lower `Confidence`, never silently merged into the resolved
  category — mirrors D3's Confirmed/Hypothesized split but for edges
  rather than lineage, since Spec_mvp explicitly requires this distinction
  for calls too ("chiamate risolte separate da quelle ipotizzate").
- Implementation minimum: edge construction from Task 4's parse data +
  Task 5's validated snapshots.
- Test GREEN: resolved-vs-heuristic call test passes per language (at
  least Rust and Python as the two structurally different cases — static
  typed vs. dynamic).
- Security invariants: same promotion discipline as Task 5 — no
  `EdgeKind` catch-all variant.
- Mutation/review gate: mutation on the resolved/heuristic classification
  branch, same priority as Task 6.
- Dependencies: Task 5. Not gated by S1/S2/S3.

**Task 8 — In-memory graph/query core**
- Objective: the queryable graph structure holding snapshots + lineage
  edges + typed edges for one or more repositories, with the traversal
  primitives `graph_query`/`get_code_context` will call — but not yet the
  `api` crate wrapper (Tasks 11–12) or any budget enforcement (Gate S3).
- Files/crates: `crates/codegraph-index/src/graph.rs`.
- API/types introduced: `CodeGraph` (in-memory adjacency structure);
  internal `fn neighbors(&self, id: SymbolSnapshotId, kind_filter: Option<EdgeKind>) -> impl Iterator<Item = &Edge>`; internal `fn definition_of(&self, id: SymbolSnapshotId) -> Option<&SymbolSnapshot>`.
- Test RED: traversal correctness fixtures (known small graph, known
  expected neighbor sets per edge-kind filter); a cyclic call graph
  (`A calls B calls A`) does not infinite-loop an unbounded internal
  traversal helper — written now even though the budget enforcement
  itself is Task 13's job, because an internal helper with no cycle
  awareness at all would make Task 13's budget a band-aid rather than a
  real limit.
- Implementation minimum: adjacency maps keyed by `SymbolSnapshotId`;
  visited-set-based traversal helpers (the same cycle-safety principle
  Gate S2 requires for filesystem walks, applied here to graph walks).
- Test GREEN: fixture traversals correct; cyclic graph traversal
  terminates using a visited set, with no depth/count limit enforced yet
  (that's Task 13 — this task proves termination-safety, not policy).
- Security invariants: none exposed externally yet (internal structure
  only).
- Mutation/review gate: mutation on filter/traversal logic.
- Dependencies: Task 5, Task 7 (needs both snapshots and edges to build a
  graph). Not gated by S1/S2/S3 (in-memory, unexposed).

**Task 9 — Incremental indexing model** [Gate S1 blocks the write-path
portion only — see note below]
- Objective: given a new commit, compute the minimal set of
  files/symbols/edges to re-parse and re-resolve, invalidate stale line
  references, and publish the update, following `RealIndexer`'s
  single-publication-lock/high-water-version/no-callback-into-source
  pattern (`crates/index/src/indexer_impl.rs:108-126`'s doc comment is the
  exact discipline to replicate).
- Files/crates: `crates/codegraph-index/src/incremental.rs`.
- API/types introduced: `trait CodeGraphIndexer { fn observe_commit(&self, repo: &RepositoryId, commit: &CommitHash, changed_files: &[FileIdentity]); fn graph_snapshot(&self) -> Arc<CodeGraph>; }` — mirrors `storage::Indexer`'s shape, but for commits/files instead of note writes.
- Test RED: changing one file invalidates only that file's symbols' line
  references and any lineage/edges pointing at them, leaving unrelated
  symbols' snapshots untouched (Spec_mvp's own required test: "dopo la
  rinomina di una funzione e un nuovo commit, i vecchi riferimenti
  risultano invalidati e quelli nuovi presenti"); a rebuild racing a live
  `observe_commit` call cannot publish a stale snapshot over a newer one
  (same barrier-test family as `concurrency_tests.rs`).
- Implementation minimum: diff changed files against the last-indexed
  commit per repository; re-run Tasks 4–7 only for changed files; re-run
  Task 6's lineage resolver only for symbols whose file changed or whose
  lineage neighbor changed.
- Test GREEN: invalidation test + concurrency barrier tests pass.
- Security invariants (**Gate S1 applies here**): this task's in-memory
  publication logic (the index-side Mutex/version-publish pattern) is NOT
  gated by S1 — it has no filesystem write path of its own. **If and only
  if** this task also persists anything to disk (a parse cache, a
  snapshot file) beyond rebuildable in-memory state, that persistence must
  wait for S1's fix, because any disk write path for B2 reuses the same
  underlying containment discipline SEC-001/002/003 are about. Recommended
  sequencing: implement Task 9 with in-memory-only state first (unblocked,
  proceeds now); defer the "persist incremental state to survive a
  restart" refinement to Task 10, which is explicitly gated.
- Mutation/review gate: mutation on the invalidation-scope computation
  (under-invalidation is a correctness bug matching Spec_mvp's own
  required test; over-invalidation is a performance bug feeding Gate S3).
- Dependencies: Task 8. In-memory portion: not gated. Any disk-persisted
  portion: **gated by S1**.

**Task 10 — Persistence/integration** [Gated by S1 and S2]
- Objective: decide and implement where B2's durable state lives (parse
  cache, graph snapshot for fast restart) such that it survives a process
  restart without re-parsing every repository from scratch, without
  becoming a backup-provenance problem (Gate S2) or a write-path
  containment problem (Gate S1).
- Files/crates: `crates/codegraph-index/src/persistence.rs`.
- API/types introduced: `CodeGraphStore` (format TBD at implementation
  time, pending the S1/S2 fixes' final shape — this plan does not
  prescribe the on-disk format now, since it depends on what S1/S2's fixes
  actually land as).
- Test RED: a persisted graph reloaded after restart matches the
  in-memory graph bit-for-bit for an unchanged repository; a restart
  followed by a new commit re-indexes only that commit's delta, not the
  whole repository.
- Implementation minimum: TBD pending S1/S2.
- Test GREEN: TBD pending S1/S2.
- Security invariants: location chosen must NOT be inside anything
  `collect_vault_files` walks unless S2's fix changes what that function
  considers authoritative; written via containment-safe write-path per
  S1's fix, not the pre-fix `write_record_unchecked` pattern.
- Mutation/review gate: deferred to implementation time.
- Dependencies: Task 9. **Blocked by Gate S1 and Gate S2 — do not start
  implementation until both close.** Planning (this entry) is not blocked.

**Task 11 — `get_code_context`** [Gate S3 blocks exposure, not internal logic]
- Objective: given a `SymbolSnapshotId` (or a `FileIdentity` + line),
  return definition + known callers + linked tests, per Spec_mvp's
  required test.
- Files/crates: `crates/codegraph-index/src/context.rs` (internal logic),
  `crates/api/src/graph_context.rs` (API wrapper).
- API/types introduced: `CodeContext { definition: SymbolSnapshot, callers: Vec<(SymbolSnapshot, Evidence)>, tests: Vec<SymbolSnapshot> }`; `fn get_code_context(ctx: RequestContext, policy: &ReadPolicy, graph: &CodeGraph, target: CodeContextTarget) -> ApiResponse<CodeContext>`.
- Test RED: known fixture graph, known expected context (Spec_mvp's
  required test); a request exceeding Gate S3's not-yet-defined budget
  returns a typed "truncated, here's why" response rather than hanging or
  OOMing — written as a budget-parameterized test from the start (budget
  value itself can be a large placeholder constant until Task 13 defines
  the real policy, but the *code path* for "budget exceeded" exists from
  Task 11, not bolted on later).
- Implementation minimum: internal logic can be implemented and unit
  tested now (Tasks 1–8 are sufficient dependencies); wiring into `api`'s
  `check()`/`ApiResponse` pattern with a real `ReadOperation::GetCodeContext`
  variant is held until Task 13's budgets are real, not placeholder.
- Test GREEN: fixture test passes; placeholder-budget truncation path
  exercised.
- Security invariants: same `ReadPolicy` gating as search/retrieve;
  response never includes raw repository paths outside what the caller's
  policy permits (mirrors `ApiError`'s sanitized-Display discipline).
- Mutation/review gate: mutation on the context-assembly logic (missing
  callers/tests is a Fase B acceptance-criteria failure, not just a style
  issue).
- Dependencies: Task 8. Internal logic: not gated. **Wiring into the
  exposed `api` callable: gated by Gate S3.**

**Task 12 — `graph_query`** [Gate S3 blocks exposure, not internal logic]
- Objective: general graph traversal query (by symbol, by edge kind, by
  repository/commit scope) — the more general sibling of `get_code_context`.
- Files/crates: `crates/codegraph-index/src/query.rs`,
  `crates/api/src/graph_query.rs`.
- API/types introduced: `GraphQuery { start: SymbolSnapshotId, edge_kinds: Vec<EdgeKind>, max_depth: u32, max_results: u32 }` (depth/result caps are MANDATORY fields, not optional-with-no-default, so a caller cannot construct an unbounded query even before Task 13 picks the server-side ceiling); `fn graph_query(ctx: RequestContext, policy: &ReadPolicy, graph: &CodeGraph, query: GraphQuery) -> ApiResponse<GraphQueryResult>`.
- Test RED: a cyclic-graph query with `max_depth` set terminates and
  returns within that depth, never more; a `max_depth`/`max_results` above
  a server-side ceiling (Task 13) is rejected with a typed error, not
  silently clamped (ST-07's silent-truncation mistake, avoided by
  construction: reject, don't clamp).
- Implementation minimum: traversal using Task 8's `neighbors`, respecting
  the mandatory caps.
- Test GREEN: cyclic-graph termination test, ceiling-rejection test pass.
- Security invariants: `GraphQuery`'s caps are mandatory in the type
  itself — this is the structural fix IX-04 is missing today (BM25 search
  has no equivalent mandatory cap in its request type); B2 does not repeat
  that gap.
- Mutation/review gate: mutation on depth/result-limit enforcement
  (highest-value mutation target for Gate S3's actual guarantee).
- Dependencies: Task 8. Internal logic: not gated. **Wiring into the
  exposed `api` callable: gated by Gate S3.**

**Task 13 — Resource budgets** [Implements Gate S3's requirements]
- Objective: define and implement the server-side ceiling values and
  enforcement Task 11/12's mandatory-but-caller-chosen caps are checked
  against, plus cancellation and incremental-reindex budget.
- Files/crates: `crates/codegraph-index/src/budget.rs`.
- API/types introduced: `QueryBudget { max_depth_ceiling: u32, max_results_ceiling: u32, max_context_bytes: u32, time_budget: Duration }`; `CancellationToken` threaded through Task 11/12's traversal and Task 9's incremental reindex.
- Test RED: a query within budget completes; a query at the ceiling is
  rejected before traversal starts (not after doing the work and then
  discarding it — cheap rejection, matching the audit's IX-04 concern
  about work already performed being the cost, not just the response
  size); a long-running reindex can be cancelled mid-way and leaves no
  partially-published (torn) graph state (same atomicity bar as
  `RealIndexer`'s publication rollback).
- Implementation minimum: ceiling checks before traversal; cooperative
  cancellation checks inside Task 9's per-file loop and Task 12's
  traversal loop.
- Test GREEN: pre-traversal rejection test, cancellation-leaves-no-torn-
  state test pass.
- Security invariants: this task is Gate S3's actual closure condition for
  the internal logic side; Gate S3 as a whole additionally requires Task
  11/12 to be wired to these budgets (not just budgets existing in
  isolation) before `api` exposure.
- Mutation/review gate: mutation on ceiling-comparison operators (an
  off-by-one or flipped comparison here reopens IX-04's exact failure
  mode).
- Dependencies: Task 9, Task 11, Task 12 (budgets need something to
  constrain). This task's own logic is not gated by S1/S2/S3; it is PART
  OF what satisfies Gate S3 for Tasks 11/12/14.

**Task 14 — MCP adapter** [Blocked by Gate S3 AND the D5 preconditions]
- Objective: read-only MCP server exposing `search`/`retrieve` (existing)
  and `graph_query`/`get_code_context` (B2) to VS Code.
- Files/crates: new `crates/mcp-server/` (or equivalent — exact crate
  boundary is a decision for whoever closes D5's preconditions, not fixed
  here).
- API/types introduced: TBD — depends entirely on the D5 preconditions'
  resolution (capability model, host identity, allowlist, retrieved-
  content/authority separation).
- Test RED/implementation/GREEN: cannot be specified yet; D5 explicitly
  defers this.
- Security invariants: everything in D5 above.
- Mutation/review gate: deferred.
- Dependencies: Task 11, Task 12, Task 13 (needs the budgeted, gated
  callables to exist first). **Blocked by Gate S3 and by D5's four
  unresolved preconditions.** This task should not be started — not even
  planning its internal structure beyond what D5 already says — until a
  separate, dedicated MCP-boundary design review happens.

**Task 15 — Evaluation/mutation/whole-branch review**
- Objective: Fase B's acceptance criteria (Recall@10/nDCG@10, citation
  correctness, abstention, latency, contamination isolation per
  `eval/rag/README.md`'s existing pattern) extended to cover code-graph
  queries specifically; whole-branch mutation and security review before
  any merge, following B1's own closing discipline (Task 9's final GO
  verdict, whole-branch review wave).
- Files/crates: `eval/rag/` extensions, no new crate.
- Dependencies: all prior tasks for the portion being evaluated. This task
  runs incrementally (evaluate what's implementable now) and finally
  (whole-branch, before any B2 merge) — same two-pass structure B1 used.

## 5. Dependency graph (summary)

```
Task 0 ─┬─> Task 1 ─┬─> Task 5 ──> Task 6 ──┬─> Task 8 ─┬─> Task 9 (in-mem: free; disk: GATE S1)
        │           │                       │           │        └──> Task 10 (GATE S1 + S2)
        │           └─> Task 2              │           ├─> Task 11 (internal: free; api: GATE S3)
        └─> Task 3 ──> Task 4 ──────────────┴─> Task 7 ─┤
                                                         └─> Task 12 (internal: free; api: GATE S3)
                                                                 │
                                              Task 13 <──────────┘ (implements S3)
                                                 │
                                              Task 14 (GATE S3 + D5 preconditions)
                                                 │
                                              Task 15 (continuous + final)
```

## 6. What can be implemented now (no blocker)

Tasks 0, 1, 2, 3, 4, 5, 6, 7, 8, and the in-memory portion of Task 9 — all
pure parsing/graph/identity logic with no write path and no exposed API.
This is the bulk of B2's novel logic and can proceed immediately.

## 7. What remains blocked

- Task 9's disk-persisted refinement and all of Task 10: **Gate S1** (write-
  path containment) and, for Task 10 specifically, also **Gate S2** (backup
  provenance).
- Task 11/12's wiring into the exposed `api` callable (internal logic is
  unblocked): **Gate S3** (resource budgeting), implemented by Task 13.
- Task 14 in full: **Gate S3** plus **D5's four preconditions** (capability
  model, host identity, allowlist, retrieved-content/authority separation),
  none of which this plan resolves — they require their own design review.

## 8. New decisions requiring authorization

None of D1–D5 required a new decision beyond what the prompt specified —
D2 follows the stated preference directly. Two items surfaced during
planning that were NOT in the original P1 checkpoint and need an explicit
call before Task 2 and Task 10 respectively, flagged here rather than
decided unilaterally:

1. **Repository allowlist authority (Task 2):** B2 needs an explicit,
   caller-supplied list of filesystem roots it is permitted to index (a new
   boundary — not the vault root, not an existing concept in A1–B1). Who
   supplies this list (the same trusted host that supplies `ReadPolicy`, or
   a separate configuration surface) is not specified in Spec_mvp and needs
   a decision before Task 2's implementation, though Task 2 can be planned
   assuming "the trusted host supplies it, same as ReadPolicy" without
   blocking.
2. **Task 10's on-disk format timing:** this plan deliberately leaves
   Task 10's format TBD pending Gate S1/S2's fixes, rather than guessing a
   format now and risking rework. This is a sequencing choice, not a
   request for a new decision — flagged here only so it isn't mistaken for
   an oversight.

## 9. GO/NO-GO on starting Task 0

**GO.** Task 0 (pin the ingestion seam) and the unblocked chain through
Task 8 involve no write path, no exposed API, and no backup-adjacent
storage — none of Gate S1/S2/S3 apply, and D1–D5 are resolved (or, for D4,
explicitly documented as an accepted open risk rather than silently
ignored) to the degree this planning stage requires. Implementation of
Task 0 may begin without further authorization beyond this plan's approval.

Tasks 9 (disk portion)/10/11(api wiring)/12(api wiring)/14 remain NO-GO for
implementation until their respective gates close, as summarized in
Section 7.
