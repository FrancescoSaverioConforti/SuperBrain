# B3.1 source material inventory

Scratch working doc for corpus curation (Task 1). Not shipped to the
runner, not read by any code. Format per entry:

`source ref -> one-sentence fact -> proposed NoteType -> proposed category [-> date]`

Target: ~60 corpus notes total (±10), split ~roughly 40-45
decision/bug/pattern/session + ~15-20 code/identifier/cross-file, with
every one of the 6 query categories (notes-decisions, bugs-fixes,
patterns-knowledge, temporal, identifier-code, cross-file-concept) having
at least 5 corpus notes, and at least 3 distinct real months represented
in event_period values.

## Strata 1: SuperBrain project history (decision/bug/pattern/session)

### Bug/fix facts (category: bugs-fixes)

1. SEC-001 (docs/technical-debt.md) -> A1 introduced anti-symlink checks in `crates/storage/src/vault.rs` (`record_path_checked`/`ensure_resolves_inside_root`) that canonicalize the candidate `.record` path, but this is a check-then-act pattern vulnerable to TOCTOU races; deferred to A2, fix is to verify the open handle itself, not the path. -> bug -> bugs-fixes -> 2026-10-27
2. SEC-002 (docs/technical-debt.md) -> Reproduced empirically 2026-10-27: a symlink at root/note-a.record pointing to root/note-b.record makes read_note(note-a) return note-b's content and version, and propose_and_commit succeeds, replacing the symlink with a real file — cross-note integrity/attribution bug, not an escape. -> bug -> bugs-fixes -> 2026-10-27
3. SEC-003 (docs/technical-debt.md) -> Reproduced empirically on Windows 2026-10-27: a hard link at root/note-a.record pointing outside the vault on the same volume makes read_note(note-a) return the external file's content, because canonicalization cannot detect hard links (same inode, genuine regular file). -> bug -> bugs-fixes -> 2026-10-27
4. d180lt (continuity) -> B2-ID-017 closed: parser_for() HTML/CSS dispatch gap was found and fixed and verified during B2 code graph work. -> bug -> bugs-fixes -> 2026-11 (continuity decision date, see decision log context)
5. d170ij (continuity) -> Correction during B2: the Java scope gap was found NOT to be the same issue as B2-ID-012 (which stays assigned to a pre-existing resolve_lineage O(n^2) finding); Java was logged as a documented future-language-support gap, not a defect. -> bug -> bugs-fixes -> 2026-11
6. SEC-002 correction (docs/technical-debt.md, "quarta revisione", 2026-10-27) -> An initial fix proposal for SEC-002 ("reject all symlinks to the .record path") was verified and found incomplete: a hard link produces the identical aliasing effect and canonicalization cannot see hard links, so the fix must check the open handle (regular file, link count == 1, path inside root), not add another path-level check. -> bug -> bugs-fixes -> 2026-10-27
7. controllore timezone_resolver.py header comment -> During the "Consolidamento timezone Scheduler/SystemContext" fix (2026-09-11), it was verified across the whole repo that no module under Automazione/engine/ imports from app/ — this boundary had never been violated, confirmed before extracting resolve_configured_timezone() into app/core/timezone_resolver.py as the sole construction point for the tz_provider injected into AutomationEngine/Scheduler. -> pattern -> cross-file-concept -> 2026-09-11

### Decision facts (category: notes-decisions)

8. m1j5 (continuity milestone) -> A1 (Schema e storage) was completed and merged to main, tagged a1-complete. -> decision -> notes-decisions -> 2026-10-26 (Spec_mvp.md revision date, A1 phase)
9. m2g9 (continuity milestone) -> B2 (Code Graph) phase was formally closed; the next macro-step was declared to be the non-blocking follow-up set, explicitly not A5 and not Task 10. -> decision -> notes-decisions -> 2026-11-14
10. d191ij (continuity) -> B2 — Code Graph: CLOSED, confirmed directly by the user. -> decision -> notes-decisions -> 2026-11-14
11. x1ks (continuity, rejected_alternative) -> An A3 draft design was rejected: unconditionally rejecting Sensitive notes forever, treating PiiStore+Markdown pointer as two separately-recovered transactions, an unauthenticated deletion-log reference file, a Task 0 test requiring SEC-00x to stay vulnerable, and a new propose_delete public entry point. -> decision -> notes-decisions -> 2026-10 (A3 phase, before A3 close)
12. x2gi (continuity, rejected_alternative) -> Rejected: treating a stray off-scope fork's untracked-file audit (EVAL-006/007/009 cleanup, including discarding b2-eval-008-polly-run.txt) as part of B2-EVAL-010. -> decision -> notes-decisions -> 2026-11
13. Spec_mvp.md intro -> The MVP specification's scope was frozen after four external reviews concluded on 2026-10-26: one human user, one agent, one generative model at a time, one active project at a time; from that point changes come only from implementation/test evidence, not new general architectural revisions. -> decision -> notes-decisions -> 2026-10-26
14. Spec_mvp.md Fase B description -> Fase B must pass with only the deterministic baseline (trigrams + BM25); CPU embeddings are added only if they improve the benchmark without worsening latency or RAM beyond the stated limits. -> decision -> notes-decisions -> 2026-10-26
15. Spec_mvp.md C2 contract -> In the MVP, the only trusted approval channel is a local CLI (`bc approve <proposal_id>`) that shows the diff before confirming; native MCP client confirmation in VS Code is allowed only after verifying the model cannot generate or forge it. -> decision -> notes-decisions -> 2026-10-26

### Pattern facts (category: patterns-knowledge)

16. docs/technical-debt.md SEC-001/002/003 shared pattern -> All three filesystem security debts (SEC-001 symlink TOCTOU, SEC-002 internal symlink aliasing, SEC-003 hard link aliasing) share one root cause — a check on a path cannot see how that path will resolve when actually opened — and one proposed closing mechanism: verify the already-open handle (regular file, link count 1, path inside root), not the path before opening it. -> pattern -> patterns-knowledge -> 2026-10-27
17. Spec_mvp.md C1 contract -> A write is only visible through the API once it reaches journal_committed (the logical linearization point); all internal components (indexer, backup, MCP server) read only committed versions through the core, never via a free scan of paths during a write. -> pattern -> patterns-knowledge -> 2026-10-26
18. Spec_mvp.md C5 contract -> The deletion log and its trusted reference do not update atomically, so a three-step protocol applies: write+sync the new log generation marked pending, update+sync the trusted reference, then mark the generation completed — only then is deletion considered complete. -> pattern -> patterns-knowledge -> 2026-10-26
19. Spec_mvp.md C6 contract -> The gate tracks five independent properties of a note (schema validity, link validity, evidence presence, verification status, retrieval eligibility) — a structurally valid note does not automatically become verified, and is not rejected merely for having no relations. -> pattern -> patterns-knowledge -> 2026-10-26
20. controllore sun_times.py docstring -> get_sun_times_today() never propagates an exception to its caller: it returns None if astral isn't installed, if lat/lon aren't configured, or if the computation fails for any reason — the same "never raise" principle already used by both of its historical callers. -> pattern -> patterns-knowledge -> 2026-09 (consolidation period, inferred from module's stated extraction context)
21. controllore timezone_resolver.py docstring -> resolve_configured_timezone() returns timezone.utc if settings are unavailable or invalid, via the same try/except pattern _check_programs() in watchdogs.py already used — deliberately not replacing or touching _check_sun_events()'s slightly different variant (no "UTC" fallback, no try/except, early return instead). -> pattern -> patterns-knowledge -> 2026-09-11

### Session/handoff facts (category: notes-decisions, filling gap)

22. d163pa (continuity) -> CALL-RES-A4 performance cost was measured during B2: moderate overhead, recorded against commits 59510ae/f50c634. -> session -> notes-decisions -> 2026-11
23. B2-CLOSURE (continuity, from earlier conversation fork summary) -> At B2 closure, two non-blocking residual items were carried forward: a missing B2-EVAL-011.md artifact and a wiring gap between agent-policy/negative-knowledge and the incremental indexer. -> session -> notes-decisions -> 2026-11-14

## Strata 2: code material (code/identifier/cross-file-concept)

### controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2

24. controllore:app/core/sun_times.py:get_sun_times_today -> `def get_sun_times_today(latitude: Optional[float], longitude: Optional[float], tz: tzinfo, today: Optional[_date] = None) -> Optional[dict]` returns `{"sunrise": datetime, "sunset": datetime}` aware in the given tz, or None on any failure; `today` is injectable only for tests. -> code -> identifier-code -> n/a (code, no event date beyond repo pin)
25. controllore:app/core/timezone_resolver.py:resolve_configured_timezone -> `def resolve_configured_timezone() -> tzinfo` reads `location.timezone` via `load_settings()`, returns `zoneinfo.ZoneInfo(loc.timezone or "UTC")`, or `timezone.utc` on any exception — logged at debug level, never raised. -> code -> identifier-code
26. controllore:Access_control/src/roles.py:role_has_permission -> `def role_has_permission(role_name: str, resource_type: str, action: str) -> bool` looks up the role via `get_role()`, returns False on KeyError, else checks `action in role.permissions.get(resource_type, [])`. -> code -> identifier-code
27. controllore:Access_control/src/roles.py:add_role/remove_role -> `add_role(role_name, description, permissions)` raises `ValueError` if the role already exists; `remove_role(role_name)` raises `KeyError` if it does not exist — both mutate the same module-level `_registry` dict returned by `_get_registry()`. -> code -> cross-file-concept
28. controllore:Access_control/src/roles.py:load_roles/reload_roles -> `load_roles(path: Path = CONFIG_PATH) -> RoleRegistry` parses `roles.json` via `RoleRegistry.model_validate`; `reload_roles(path)` rebuilds the module-level cache from a fresh parse, used to pick up config changes without restarting. -> code -> identifier-code
29. controllore:app/core/automation_rbac.py:actions_touch_device -> function at line 61 in automation_rbac.py determines whether a set of automation actions touch a given device (role-based access control boundary for automation actions). -> code -> identifier-code
30. controllore:app/core/automation_rbac.py:collect_device_targets -> function at line 112 collects the device targets implicated by an automation rule, feeding the RBAC check in actions_touch_device. -> code -> cross-file-concept
31. controllore:app/core/schedule_lock_guardrail.py:evaluate_schedule_lock_guardrail -> function at line 185 evaluates whether a schedule change is blocked by a "lock" guardrail, walking condition trees via the internal `_walk` helper (line 102) and `_LockEvent` NamedTuple (line 96). -> code -> cross-file-concept
32. controllore:app/core/schedule_lock_guardrail.py:extract_trigger_condition_device_ids -> function at line 59 extracts device ids referenced by a trigger's condition group, built on `_device_ids_from_condition_group` (line 45) — part of the same guardrail module's device-id extraction chain feeding `evaluate_schedule_lock_guardrail`. -> code -> cross-file-concept

### SuperBrain crates (own code history)

33. crates/index/src/time_range.rs:resolve_time_expression -> `pub fn resolve_time_expression(expr: &TimeExpression, reference_date: NaiveDate) -> Result<(DateTime<Tz>, DateTime<Tz>), TimeResolutionError>` resolves local calendar boundaries to a half-open interval in Europe/Rome; `MonthsAgo` is always a whole calendar month, never a rolling day count. -> code -> identifier-code
34. crates/storage/src/propose.rs:parse_note_id (line 418) -> internal function `parse_note_id(field: &str, raw: &str) -> Result<NoteId, ProposeError>` validates a candidate id string against `storage::NoteId`'s syntax, used by `propose_and_commit` both for a note's own `id` and for every non-pending `relations.*` target. -> code -> cross-file-concept
35. crates/storage/src/propose.rs:propose_and_commit (line 179) -> `pub fn propose_and_commit(vault: &Vault, note_text: &str, expected_version: Version, operation_id: Uuid, known_ids: &HashSet<NoteId>, pending_ids: &HashSet<String>, gate: &policy::Gate, pii_store: Option<&PiiStore>) -> Result<CommitResult, ProposeError>` is the one entry point that parses frontmatter, validates relations, and commits a note through the gate. -> code -> identifier-code
36. crates/index/src/document.rs build_text_searchable -> `text_searchable` is built differently by classification: Sensitive notes include only the seven non-temporal allowlisted structured fields (never body, source, relations, or any temporal string); Internal/Public notes additionally include body, source, and every relations.* target. -> code -> cross-file-concept
37. crates/api/src/lib.rs module doc -> B1's callable core is synchronous with no timeout, retry scheduler, size/rate cap, or remote identity; chunk ids are explicitly rejected until B2; no backing data is ever fabricated when a field is unavailable. -> code -> identifier-code

## Temporal coverage check

Distinct real months represented: 2026-09 (controllore timezone consolidation), 2026-10 (A1/A3 phase, SEC-001/2/3 discovery, Spec_mvp.md freeze), 2026-11 (B2 closure, continuity decisions). Three distinct months confirmed — temporal stratum has genuine targets across Q3-Q4 2026.

## Category tally (target: >=5 each)

- notes-decisions: entries 8-15, 22-23 = 10
- bugs-fixes: entries 1-7 = 7
- patterns-knowledge: entries 16-21 = 6
- temporal: drawn from the above (any entry with a real date) — covered by entries 1-23 collectively, no separate content needed
- identifier-code: entries 24-26, 28-29, 33, 35, 37 = 8
- cross-file-concept: entries 27, 30-32, 34, 36 = 6

Total distinct facts: 37. This inventory intentionally lists more candidate
facts (~37) than the ~30-45 project-history notes needed, to allow Task 2
to drop weak entries and still land at 60 notes total ±10 after combining
with the ~15-25 code entries above (13 code entries listed; Task 2 may
split some into 2 notes — e.g. separate notes for extract_trigger_condition_device_ids
and extract_conditional_action_device_ids — to reach the code-stratum target).
