# Handoff: B1 — Deterministic Search → OpenAI Codex (VS Code)

**Reason for handoff:** Claude Code session reached 94% of its usage limit. Work is paused at a clean, verified checkpoint — Tasks 0–4 of the B1 plan are complete and reviewed; Task 5 has not started. No destructive or in-progress operations are pending.

**Handoff prepared:** 2026-10-30, by Claude (Sonnet 5), via `superpowers:subagent-driven-development`.

---

## 1. Goal and exact status of Phase B1

**Goal (from the plan):** give the SuperBrain vault a deterministic retrieval path — exact-match (trigram) and ranked (BM25) search over committed note content, with correct citations, project/type/status/trust/date filters, and the C4 time-range contract. No embeddings, no tree-sitter, no code graph (those are B2/B3), no generative model (Fase D). This closes Fase B of `docs/Spec_mvp.md`'s consegna 2 (trigram+BM25 baseline) and partially consegna 5/6.

**Exact status:** 5 of 10 tasks complete (Tasks 0, 1, 2, 3, 4). Each passed an independent task review; Task 2 required one fix round (addressed, re-reviewed clean). Task 4 closes Continuity item `[n2io]` only **partially** — see §8. Task 5 has not been started: no brief extracted, no code written, no commits.

**This is mid-plan, not mid-task.** The working tree is clean (`git status` → nothing to commit) and the last commit (`94c59b4`) is a complete, reviewed task. A fresh agent can safely resume at Task 5 without untangling partial work.

---

## 2. Approved plan and reference files

- **The plan being executed:** `docs/superpowers/plans/2026-10-30-b1-deterministic-search.md` (committed on `main` at `55d1721`, carried into this branch). Read this file in full before resuming — it is the authority for every task's requirements, the Architecture section's Indexer/Vault seam design, the Sensitive-note allowlist, and the two consolidation rounds' corrections.
- **The canonical spec:** `docs/Spec_mvp.md` (brainCompany MVP v6) — Fase B section, "Ricerca per periodo," contracts C4 and C6.
- **Known technical debt:** `docs/technical-debt.md` — SEC-001/002/003 (open, unaffected by B1 — see §8).
- **Prior phase evidence:** `eval/results/A1/evidence.md`, `eval/results/A2/evidence.md`, `eval/results/A3/evidence.md` — precedent for this plan's own closing evidence file (`eval/results/B1/evidence.md`, Task 9, not yet created).
- **This plan's SDD workspace (⚠️ see warning below):** `.superpowers/sdd/2026-10-30-b1-deterministic-search/` — ledger, every task's brief/report, every review-package diff.

> **⚠️ Critical: the SDD workspace directory is gitignored and exists only in this worktree's local filesystem — it is NOT part of the git history and will NOT be visible if Codex opens a fresh clone or a different checkout of this repository.** If Codex is operating on this exact worktree (`.worktrees/b1-deterministic-search` under the main checkout), the files are present on disk and readable directly. If Codex is instead given a fresh clone of the `b1-deterministic-search` branch, this directory will be **absent**, and the per-task reports/review diffs referenced throughout this document will not exist there. In that case, this handoff document (committed in git, see §9) is the only surviving record of what each task did and why — treat its summaries in §4 as authoritative if the SDD workspace is unavailable.

---

## 3. Git state (verified at handoff time, not reported secondhand)

```
Branch:        b1-deterministic-search
Worktree path: C:\Users\fsave\Desktop\Python_code\SuperBrain\.worktrees\b1-deterministic-search
HEAD:          94c59b4a5370bc08d09863361fcbdf6deffe611e
HEAD subject:  feat(B1 Task 4): confidentiality boundary audit for the index surface
Branched from: main @ 55d1721 (merge-base confirmed)
git status:    clean — nothing to commit, nothing uncommitted
```

Full commit list on this branch (`git log --oneline main..HEAD`, oldest first):

```
edaf0fe feat(index): scaffold index crate with SEC/n2io status anchor
06221e3 chore(index): record index crate in Cargo.lock
f51120b docs(index): correct [n2io]'s surface enumeration to all five
274eadd feat(B1 Task 1): pin the Indexer integration seam
60e2a22 feat(B1 Task 2): add Document shape and TrigramIndex
3d82b31 fix(B1 Task 2): expose storage::split_record for raw-bytes-to-Document
00a3cc7 fix(B1 Task 2): relocate split_record to vault.rs to satisfy single-write-path guardrail
fb6ee61 fix(B1 Task 2): query_exact now verifies the literal substring, not just trigram co-occurrence
4521821 feat(B1 Task 3): add Bm25Index and wire the real Indexer into Vault's write pipeline
94c59b4 feat(B1 Task 4): confidentiality boundary audit for the index surface
```

**Other worktrees on this repo** (unaffected by this handoff, listed for orientation only — do not touch):
```
C:/Users/fsave/Desktop/Python_code/SuperBrain                       4c3e6e7 [main]
C:/Users/fsave/Desktop/Python_code/SuperBrain/.worktrees/a2-recovery f807c34 [a2-recovery]  (stale/historical, A2 already merged to main)
```

**No merge, tag, or push has been performed.** `main` is untouched by this branch's work beyond the already-merged A1/A2/A3 history it was branched from.

---

## 4. Tasks completed, and the task to resume

### Completed (all reviewed, all commits above)

| Task | What it built | Review verdict |
|---|---|---|
| 0 | Scaffolded `crates/index` crate; added workspace member; doc-only SEC-00x/[n2io] status anchor in `lib.rs` | Clean + 1 controller doc fix (surface enumeration correction) |
| 1 | **Highest-risk task.** Designed and implemented the `Indexer`/`Vault` integration seam: `Indexer` trait made `pub`; `mark_current` signature changed to carry `record_bytes: &[u8]` directly (no callback into `Vault::read_note`, eliminating a self-deadlock risk); two new additive `Vault` constructors (`open_with_indexer`, `open_with_pii_store_and_indexer`); new `pub fn list_note_ids` for cold-start enumeration; the Task 9 scalability-benchmark protocol written as a doc comment (4 synthetic sizes, 3 metrics, concrete query-set composition — no code, execution deferred to Task 9) | **GO** — reviewer independently re-ran clippy/tests, reproduced two pre-existing unrelated issues (see §7), confirmed zero callbacks at all 5 real call sites |
| 2 | `Document` (two-category shape: 10 structured-metadata fields vs. text-searchable content, Sensitive-note allowlist enforced) + `TrigramIndex` | Fix round 1 required (see §5) — now clean |
| 3 | **Second-highest-risk task.** `Bm25Index` (zero-floor scoring, independently verified) + `RealIndexer` wired into `storage::Vault`'s actual write path via `mark_current`; cold-start `rebuild_index` via `Vault::list_note_ids` | **GO** — reviewer independently retraced `propose_and_commit`'s validation logic to confirm a deliberate test-mechanism deviation from the brief (see §5) was genuinely justified, not just asserted |
| 4 | Confidentiality boundary audit for the index surface (new `crates/index/tests/b1_confidentiality_boundary.rs`) — proves `source`/`relations` never leak into `TrigramIndex`/`Bm25Index` postings, through both direct construction and the real `propose_and_commit` write path; reviewer independently mutation-tested the property (deliberately broke the allowlist, confirmed 4 tests failed, reverted) | Clean, with one Important non-blocking structural finding (see §6) |

### Next task: Task 5 — C4 relative date expression resolution

**Not started.** Brief has not been extracted. To resume:

1. From the worktree root, extract the brief: the project uses `superpowers:subagent-driven-development`'s `scripts/task-brief` tool against the plan file and task number `5`, writing to `.superpowers/sdd/2026-10-30-b1-deterministic-search/task-5-brief.md`. If that tooling isn't available to Codex, read Task 5's section directly from `docs/superpowers/plans/2026-10-30-b1-deterministic-search.md` (search for `## Task 5`).
2. Task 5 builds `crates/index/src/time_range.rs`: a pure function `resolve_time_expression` implementing contract C4's relative-date-to-interval resolution (e.g. "il mese scorso", "due mesi fa", DST transition on 2026-10-25), with the reference date always an explicit parameter (never `SystemTime::now()`), per the spec's own rule.
3. Task 5 has **no dependency on Tasks 1–4** — it is a standalone pure-function module, testable in isolation. It is safe to implement without re-reading the Indexer/Vault seam details, though understanding the overall plan and Global Constraints (license allowlist, no spec edits, etc.) still matters.
4. The plan's own Review Focus section calls out: verify the DST test exercises REAL `Europe/Rome` timezone arithmetic (via whatever date/time crate — `chrono` or `time` — gets added as the one new dependency), not UTC math that happens to look right.

---

## 5. Recent changes and fix rounds (what actually happened, not just "tests pass")

**Task 2 required one fix round** — two issues the independent reviewer found with concrete reproductions, both now fixed and re-reviewed clean:

1. `TrigramIndex::query_exact` promised exact-substring confirmation in its doc comment but only intersected trigram postings sets, producing real false positives (e.g., querying `"catdog"` against text containing `"cat"`, `"atd"`, `"tdo"`, `"dog"` as separate, non-contiguous substrings would wrongly match). Fixed by adding a `texts: HashMap<NoteId, String>` map and a real `contains_term` substring check after the trigram-intersection pre-filter.
2. `Document::new` took a pre-split `(metadata, body)` pair because `storage`'s existing frontmatter/body splitter helpers are `pub(crate)`, unreachable from `index` — which would have pushed frontmatter-split duplication risk into Task 3. **Controller ruling:** `storage` grows one small additive `pub fn split_record` (ended up in `vault.rs`, not `propose.rs`, after tripping the `storage_exposes_exactly_one_public_write_function` guardrail test — confirming that guardrail works as intended) that wraps the EXISTING private helpers rather than duplicating their delimiter-finding logic.

**Task 3's one notable deviation from its brief**, found acceptable only after independent scrutiny: the brief asked for a fault-injection mechanism "mirroring A2/A3's FaultPoint idiom" to simulate an indexing failure. The implementer argued this is impossible to trigger through a real `propose_and_commit` call (content is already successfully parsed before `mark_current` is ever reached) and built a `#[cfg(test)]`-only `poison_next_mark_current` hook instead. The reviewer independently retraced `propose_and_commit`'s entire validation sequence line-by-line and confirmed there is genuinely no asymmetry between what it validates and what the indexing path re-validates — the claim held up, the hook is confirmed test-only (not production-reachable), and both required assertions (write still succeeds; `state_for` stays `Stale`) are proven through a real `Vault`/`propose_and_commit` call, not a synthetic shortcut.

---

## 6. Open, non-blocking finding carried into the final review

**Task 4's confidentiality audit is structurally split across two files**, not consolidated into the one file the brief asked for:
- `crates/index/tests/b1_confidentiality_boundary.rs` — the `source`/`relations`-leak tests and the malformed-record error-output test.
- `crates/index/src/indexer_impl.rs`'s own `#[cfg(test)] mod tests` — the test exercising the `poison_next_mark_current` fault path specifically.

This split is **not a mistake** — it is a genuine Rust compilation constraint: an external integration-test crate (which `tests/*.rs` files are) cannot see a library's own `#[cfg(test)]`-gated items, so the poison hook is simply invisible from outside `indexer_impl.rs`. Both files carry explicit cross-reference comments pointing at each other. The independent reviewer confirmed this is sound but flagged it as an **Important, non-blocking** structural note for the final whole-branch review (Task 9) to be aware of — it does not need a dedicated fix round, but a future task brief asking for "one consolidated audit file" involving `#[cfg(test)]`-gated internals should account for this constraint up front.

---

## 7. Test results, commands, and exact numbers (re-verified at handoff time)

All of the following were run **just now**, directly in this worktree, by the handoff author — not copied from a prior subagent's report.

```
$ cargo build --workspace
   Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.17s

$ cargo clippy --workspace -- -D warnings
   Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.28s
   (zero warnings)

$ cargo test --workspace
domain          32 passed, 0 failed
index           38 passed, 0 failed  (unit tests)
index            6 passed, 0 failed  (tests/b1_confidentiality_boundary.rs)
policy          14 passed, 0 failed
storage        201 passed, 0 failed  (unit tests)
storage          4 passed, 0 failed  (tests/a3_backup_restore.rs)
storage          6 passed, 0 failed  (tests/a3_confidentiality_boundary.rs)
storage         14 passed, 0 failed  (tests/a3_pii_integration.rs)
storage         24 passed, 0 failed  (tests/integration.rs)
vault_security  86 passed, 0 failed  (unit tests)
doctests         5 passed, 0 failed  (storage only; domain/index/policy/vault_security have 0 doctests)
──────────────────────────────────────
TOTAL          430 passed, 0 failed, 0 ignored

$ cargo deny check
advisories ok, bans ok, licenses ok, sources ok
```

**Known, pre-existing, intermittent flake (NOT caused by this branch, NOT observed on this run):** `storage::restore`'s concurrency tests under full-workspace parallel scheduling have shown occasional, non-deterministic failures across Tasks 1–3's reviews (different specific test each time, e.g. `concurrent_restore_and_deletion_through_independent_handles_never_expose_the_deleted_note`, `f1_concurrent_restore_and_deletion_never_expose_a_deleted_non_sensitive_note`). Every occurrence was independently confirmed, across multiple tasks' reviews, to be:
- Absent when `-p storage --lib` or `-p index --lib` are run in isolation (passes reliably every time).
- Traced to `crates/storage/src/restore.rs`/`recovery.rs`, files this branch's entire diff never touches (confirmed via `git diff --stat` at every task).
- Predating this branch (git-blamed to a pre-B1 commit for the related `assert!(true)` clippy lint flagged in Task 1's review).

**Not yet run (correctly, per the plan):** `cargo mutants -p index`/`-p api` (Task 9's job, `api` crate doesn't exist yet), the Task 9 Step 3 scalability benchmark (protocol defined in Task 1, execution deliberately deferred to Task 9 per the user's own explicit instruction that benchmarks must run against real indexing/search code, not stubs).

---

## 8. Architectural decisions and open security constraints

**Decisions made during implementation (all already reviewed and accepted — do not re-litigate without new evidence):**

1. `Indexer` trait is `pub` (not a new parallel trait). `NullIndexer`/`index_state_for` stay `pub(crate)`.
2. `Indexer::mark_current`'s signature carries `record_bytes: &[u8]` directly — **no callback into `Vault::read_note` or any `Vault`-held lock, ever**, from inside `mark_current`. This is the single most load-bearing constraint in the whole plan; any future change to the real indexer must preserve it.
3. B1's index is **in-memory only** — no disk persistence, per explicit user instruction. A scalability benchmark protocol (4 sizes: 100/1,000/10,000/100,000 notes; 3 metrics: rebuild time, memory, query P50/P95) is defined (Task 1) but not yet executed (deferred to Task 9, intentionally).
4. Sensitive-note allowlist: **all ten** structured-metadata fields (`id, type, status, trust, project, created, updated, event_period, schema_version, classification`) are text-searchable even for `Sensitive` notes (confirmed directly against `document.rs`'s code during Task 4's review, not just inferred from the plan prose — this point has been a recurring source of confusion across earlier plan drafts, worth double-checking again before Task 6). Only `source` and `relations.*` are excluded from text-search for `Sensitive` notes specifically (never from `Internal`/`Public` notes, and never excluded from direct `retrieve` — the allowlist governs the search/index surface only).
5. `storage` gained two new additive public items during B1 so far: `Vault::list_note_ids` (Task 1) and `split_record`/`SplitRecordError` (Task 2, lives in `vault.rs`). Neither is a write path; `storage::propose_and_commit` remains the workspace's only public write entry point, confirmed at every task's review.

**Security constraints that remain exactly as before B1 (verified untouched at every task review via `git diff --stat` on `crates/storage/src/vault.rs`):**

- **SEC-001/002/003** (symlink race/TOCTOU, intra-vault symlink aliasing, hard-link aliasing — `docs/technical-debt.md`) — all three **DEFERRED**, unaffected by B1. B1 never calls `record_path_checked`/`ensure_resolves_inside_root`/`atomic_write` directly; it only calls `Vault::read_note`/`list_note_ids`, which already wrap those checks internally.
- **[n2io]** (A3's deferred index/embedding/summary/attachment/cache confidentiality audit) — **PARTIALLY closed** by Task 4 for the index-postings and log-output surfaces only. The `SearchResponse`/search-API-response surface (named in the original item) remains **open**, explicitly deferred to Task 6, which must add its own `source`/`relations`-leak test once that type exists (this requirement was ruled on directly with the user — see ledger entry — and must be carried into Task 6's own dispatch brief, not silently dropped). Embedding and cache surfaces remain open too, unrelated to B1 — no such subsystems exist anywhere in this codebase yet. **This Continuity update has NOT yet been recorded** (see §10) — whoever resumes should record it, or flag to the user that it's still pending.
- **C3** (data classification boundary: public/internal/sensitive/secret) — respected throughout: `Secret` never reaches storage at all (pre-existing, A1/A3); `Sensitive` notes' bodies are proven, via dedicated adversarial tests in Tasks 2/3/4, to never reach either postings structure, through both direct construction and the real write path. Task 4's reviewer additionally mutation-tested this property directly (deliberately broke the code, confirmed the tests catch it, reverted).
- **C4** (relative date resolution) — **not yet implemented**; this is exactly Task 5, the next task to resume.
- **C5** (deletion-log/backup consistency) — unaffected by B1; B1's cold-start rebuild correctly excludes deleted notes via the existing `DeletionLog`/`Vault::list_note_ids`, confirmed in Task 3's review, but B1 does not modify C5's own machinery at all.

---

## 9. What is still in progress or unresolved at this exact moment

- **Task 5 has not started.** No brief, no code, no commits.
- **[n2io]'s partial-closure update has NOT yet been recorded in Continuity** (the MCP tool for recording it was available to the controller but the controller paused — per the user's own "stop and check in" request after Task 4's review — before recording it). The exact, reviewer-verified text to record is quoted in Task 4's report (`.superpowers/sdd/2026-10-30-b1-deterministic-search/task-4-report.md`, if that file is reachable — see the §2 warning) or can be reconstructed from §8 above.
- **No fix rounds, reviews, or subagents are currently running.** Nothing was left mid-dispatch; this handoff point was deliberately chosen between tasks, not inside one.
- **This handoff document itself is about to be committed** (see immediately below) — once that commit lands, the working tree should be clean again and this section becomes purely historical.

---

## 10. First concrete action Codex should take

1. **Read `docs/superpowers/plans/2026-10-30-b1-deterministic-search.md` in full**, then this handoff document, then (if reachable — see §2's warning) `.superpowers/sdd/2026-10-30-b1-deterministic-search/progress.md` for the complete task-by-task ledger with reviewer findings in more detail than this document's summary.
2. **Verify the baseline independently before writing any code**: run `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings` in this worktree and confirm the same 430/0/0 result this document reports at §7. Do not trust this document's numbers without re-running them yourself — exactly the discipline this handoff was prepared under.
3. **Extract or read Task 5's brief** (`## Task 5` in the plan file) and begin implementing `crates/index/src/time_range.rs`'s `resolve_time_expression`, following the plan's TDD RED→GREEN discipline and its explicit requirement that every C4 test use an explicit reference-date parameter, never `SystemTime::now()`.
4. **After Task 5, dispatch an independent review before moving to Task 6** — this plan's established discipline (every task reviewed, Task 1/3 given extra scrutiny for their higher risk) should continue. Pay particular attention, when Task 6 is reached, to carrying forward the `SearchResponse` source/relations-leak test obligation flagged in §8 — this is the one concrete loose thread from Task 4 that a future task must not silently drop.

---

**Verified state at handoff** (restated for the record, matching what was shown directly in the conversation before this document was finalized):
- Document path: `docs/handoffs/B1-to-codex.md` (this file)
- HEAD: `94c59b4a5370bc08d09863361fcbdf6deffe611e`
- `git status`: clean
- Next task: **Task 5 — C4 relative date expression resolution**, not started
