"""One-shot generator for eval/rag/corpus/notes/*.md (Task 2 scratch tool).

Not shipped/imported by anything; run once, inspect output, discard.
Encodes the curated content from source-inventory.md as structured note
specs, then writes valid frontmatter+body Markdown files.
"""
import json
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "corpus" / "notes"
OUT.mkdir(parents=True, exist_ok=True)

# Each note: id, type, status, created, updated, event_period, source, body, relations(optional)
NOTES = []

def note(id_, type_, status, date, source, body, relations=None, project="eval-b3"):
    NOTES.append({
        "id": id_, "type": type_, "status": status, "trust": "internal",
        "classification": "internal", "project": project, "source": source,
        "created": date, "updated": date, "event_period": date, "body": body,
        "relations": relations or {},
    })

# --- bugs-fixes ---
note("bug-2026-10-27-sec001-toctou", "bug", "reviewed", "2026-10-27",
     "docs/technical-debt.md#SEC-001",
     "A1 introduced anti-symlink checks in crates/storage/src/vault.rs "
     "(record_path_checked / ensure_resolves_inside_root) that canonicalize "
     "the candidate .record path before use. This is a check-then-act "
     "pattern: it blocks symlinks already present at check time, but the "
     "filesystem can change between the check and the actual I/O "
     "(read, write, rename), a TOCTOU race. Deferred to A2; the correct fix "
     "is to open the file without following links and verify the already-"
     "open handle itself (regular file, link count 1, path inside root), "
     "not the path before opening it.")

note("bug-2026-10-27-sec002-symlink-aliasing", "bug", "reviewed", "2026-10-27",
     "docs/technical-debt.md#SEC-002",
     "Reproduced empirically on 2026-10-27: a symlink at root/note-a.record "
     "pointing to root/note-b.record makes read_note(note-a) return "
     "note-b's content and version; propose_and_commit(note-a, expected = "
     "note-b's version) succeeds and replaces the symlink with a real "
     "file, leaving note-b intact. No escape from the vault and no cross-"
     "note write corruption, but a real integrity bug: content and "
     "version get misattributed to the wrong note id, and note-a inherits "
     "note-b's version number instead of starting from zero.")

note("bug-2026-10-27-sec003-hardlink-aliasing", "bug", "reviewed", "2026-10-27",
     "docs/technical-debt.md#SEC-003",
     "Reproduced empirically on Windows, 2026-10-27: a hard link at "
     "root/note-a.record pointing to a file outside the vault on the same "
     "volume makes read_note(note-a) return the external file's content, "
     "and propose_and_commit at the external file's version number "
     "succeeds. Canonicalization cannot detect this because a hard link "
     "is not a symlink pointing elsewhere — it is a second directory "
     "entry pointing directly at the same inode, a genuine regular file "
     "whose path genuinely resolves inside the vault root.")

note("bug-2026-10-27-sec002-fix-incomplete", "bug", "reviewed", "2026-10-27",
     "docs/technical-debt.md#SEC-002 (quarta revisione)",
     "An initial fix proposed for SEC-002 — reject any symlink at the "
     "candidate .record path, same mechanism as SEC-001, claimed to cost "
     "nothing extra — was checked and found incomplete: a hard link (not "
     "a symlink) to the same path produces the identical aliasing effect, "
     "both outside the vault (SEC-003) and between two notes inside it, "
     "and a check that only rejects symlinks does not catch it, because "
     "canonicalization does not see hard links. The correct fix, shared "
     "with SEC-001 and SEC-003, is verifying the open handle, not adding "
     "another path-level check.")

note("bug-2026-11-b2-id017-parser-dispatch", "bug", "verified", "2026-11-05",
     "continuity:d180lt",
     "B2-ID-017 was closed: a gap in parser_for()'s HTML/CSS dispatch "
     "during the code graph's tree-sitter indexing was found, fixed, and "
     "verified before B2 closure.")

note("bug-2026-11-java-scope-correction", "bug", "reviewed", "2026-11-05",
     "continuity:d170ij",
     "During B2, a correction was made: the Java language-support gap is "
     "NOT the same issue as B2-ID-012, which stays assigned to a pre-"
     "existing resolve_lineage O(n^2) performance finding. Java was "
     "re-logged as a documented future-language-support gap, not a "
     "defect in the shipped code graph.")

note("pattern-2026-09-11-automation-engine-boundary", "pattern", "verified", "2026-09-11",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/timezone_resolver.py",
     "During the \"Consolidamento timezone Scheduler/SystemContext\" fix "
     "on 2026-09-11, it was verified across the whole controllore repo "
     "that no module under Automazione/engine/ imports from app/ — this "
     "boundary had never been violated. resolve_configured_timezone() was "
     "extracted into app/core/timezone_resolver.py specifically to be the "
     "one point that builds the tz_provider injected into "
     "AutomationEngine/Scheduler, which stay unaware of "
     "app/routers/settings_models.")

# --- notes-decisions ---
note("decision-2026-10-26-a1-complete", "decision", "verified", "2026-10-26",
     "continuity:m1j5",
     "A1 (Schema e storage) was completed and merged to main, tagged "
     "a1-complete.")

note("decision-2026-11-14-b2-closed", "decision", "verified", "2026-11-14",
     "continuity:m2g9",
     "B2 (Code Graph) phase was formally closed. The next macro-step was "
     "declared to be the non-blocking follow-up set, explicitly not A5 "
     "and not Task 10.")

note("decision-2026-11-14-b2-confirmed-closed", "decision", "verified", "2026-11-14",
     "continuity:d191ij",
     "B2 — Code Graph was confirmed CLOSED directly by the user, after "
     "the B2 scope reconciliation report.")

note("decision-2026-10-a3-pii-design-rejected", "decision", "reviewed", "2026-10-20",
     "continuity:x1ks",
     "An early A3 draft design was rejected before A3 closed: it would "
     "have unconditionally rejected Sensitive notes forever, treated the "
     "PiiStore and the Markdown pointer as two separately-recovered "
     "transactions, added an unauthenticated deletion-log reference "
     "file, required a Task 0 test that SEC-00x stay vulnerable, and "
     "added a new propose_delete public entry point.")

note("decision-2026-11-b2-eval010-scope-rejected", "decision", "reviewed", "2026-11-10",
     "continuity:x2gi",
     "It was rejected to treat a stray off-scope fork's untracked-file "
     "audit (EVAL-006/007/009 cleanup, including discarding the "
     "b2-eval-008-polly-run.txt file) as part of B2-EVAL-010's scope.")

note("decision-2026-10-26-spec-frozen", "decision", "verified", "2026-10-26",
     "docs/Spec_mvp.md",
     "The MVP specification's scope was frozen on 2026-10-26 after four "
     "external reviews concluded: one human user, one agent, one "
     "generative model at a time, one active project at a time. From "
     "that point the specification changes only from implementation and "
     "test evidence, never from new general architectural revisions.")

note("decision-2026-10-26-faseb-deterministic-baseline", "decision", "verified", "2026-10-26",
     "docs/Spec_mvp.md#Fase-B",
     "Fase B must pass using only the deterministic retrieval baseline, "
     "trigrams plus BM25. CPU embeddings are introduced only if they "
     "measurably improve the benchmark without pushing latency or RAM "
     "past the stated limits.")

note("decision-2026-10-26-c2-approval-channel", "decision", "verified", "2026-10-26",
     "docs/Spec_mvp.md#C2",
     "In the MVP, the only trusted human-approval channel is a local CLI, "
     "`bc approve <proposal_id>`, which shows the diff before confirming. "
     "Native MCP client confirmation inside VS Code is allowed only after "
     "it is verified that the model cannot generate or forge that "
     "confirmation.")

note("session-2026-11-call-res-a4-perf", "session", "reviewed", "2026-11-08",
     "continuity:d163pa",
     "CALL-RES-A4's performance cost was measured during B2: moderate "
     "overhead, recorded against commits 59510ae and f50c634.")

note("session-2026-11-14-b2-closure-residuals", "session", "reviewed", "2026-11-14",
     "continuity:B2-CLOSURE",
     "At B2's closure, two non-blocking residual items were carried "
     "forward rather than blocking the close: a missing B2-EVAL-011.md "
     "evidence artifact, and a wiring gap between agent-policy/negative-"
     "knowledge and the incremental indexer.")

# --- patterns-knowledge ---
note("pattern-2026-10-27-handle-not-path", "pattern", "verified", "2026-10-27",
     "docs/technical-debt.md",
     "SEC-001, SEC-002 and SEC-003 (symlink TOCTOU, internal symlink "
     "aliasing, hard link aliasing) share one root cause: a check on a "
     "path cannot see how that path will resolve when it is actually "
     "opened. All three share one proposed closing mechanism: verify the "
     "already-open handle — regular file, link count 1, path inside "
     "root — rather than adding another check on the path before opening "
     "it.")

note("pattern-2026-10-26-write-visibility-linearization", "pattern", "verified", "2026-10-26",
     "docs/Spec_mvp.md#C1",
     "A write only becomes visible through the API once it reaches "
     "journal_committed, the logical linearization point. Every internal "
     "component — indexer, backup, MCP server — reads only committed "
     "versions through the core; none ever does a free scan of vault "
     "paths during a write.")

note("pattern-2026-10-26-deletion-log-three-step", "pattern", "verified", "2026-10-26",
     "docs/Spec_mvp.md#C5",
     "The deletion log and its trusted reference cannot update "
     "atomically, so a three-step protocol applies: write and sync the "
     "new log generation marked pending; update and sync the trusted "
     "reference; only then mark the generation completed. Deletion is "
     "not considered complete until step three.")

note("pattern-2026-10-26-c6-independent-gate-properties", "pattern", "verified", "2026-10-26",
     "docs/Spec_mvp.md#C6",
     "The gate tracks five independent properties of a note: schema "
     "validity, link validity, evidence presence, verification status, "
     "and retrieval eligibility. A structurally valid note does not "
     "automatically become verified, and a note with no relations at all "
     "is not rejected for that reason alone.")

note("pattern-2026-09-sun-times-never-raises", "pattern", "verified", "2026-09-10",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/sun_times.py",
     "get_sun_times_today() never propagates an exception to its caller: "
     "it returns None if the astral library is not installed, if "
     "latitude/longitude are not configured, or if the sun-time "
     "computation fails for any other reason — the same never-raise "
     "principle already used by both of its historical callers before "
     "this consolidation.")

note("pattern-2026-09-11-timezone-fallback-reuse", "pattern", "verified", "2026-09-11",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/timezone_resolver.py",
     "resolve_configured_timezone() falls back to timezone.utc if "
     "settings are unavailable or invalid, reusing the same try/except "
     "pattern _check_programs() in watchdogs.py already used. It "
     "deliberately leaves _check_sun_events()'s slightly different "
     "variant untouched — no \"UTC\" fallback there, no try/except, an "
     "early return instead when lat/lon aren't configured.")

# --- identifier-code ---
note("code-sun-times-get-sun-times-today", "code", "verified", "2026-09-10",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/sun_times.py:get_sun_times_today",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: app/core/sun_times.py\nsymbol: get_sun_times_today\n\n"
     "def get_sun_times_today(latitude: Optional[float], longitude: Optional[float], "
     "tz: tzinfo, today: Optional[_date] = None) -> Optional[dict]\n\n"
     "Returns {\"sunrise\": datetime, \"sunset\": datetime}, aware, in the "
     "given tz, for today at the given location — or None on any "
     "failure. The `today` parameter exists only so tests can inject a "
     "fixed date; production callers never pass it.")

note("code-timezone-resolver-resolve-configured-timezone", "code", "verified", "2026-09-11",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/timezone_resolver.py:resolve_configured_timezone",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: app/core/timezone_resolver.py\nsymbol: resolve_configured_timezone\n\n"
     "def resolve_configured_timezone() -> tzinfo\n\n"
     "Reads location.timezone via load_settings() and returns "
     "zoneinfo.ZoneInfo(loc.timezone or \"UTC\"); returns timezone.utc on "
     "any exception, logged at debug level, never raised to the caller.")

note("code-roles-role-has-permission", "code", "verified", "2026-09-01",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:role_has_permission",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: Access_control/src/roles.py\nsymbol: role_has_permission\n\n"
     "def role_has_permission(role_name: str, resource_type: str, action: str) -> bool\n\n"
     "Looks up the role via get_role(); returns False if the role does "
     "not exist (KeyError caught), otherwise checks whether `action` is "
     "in role.permissions.get(resource_type, []).")

note("code-roles-load-roles-reload-roles", "code", "verified", "2026-09-01",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:load_roles",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: Access_control/src/roles.py\nsymbol: load_roles, reload_roles\n\n"
     "def load_roles(path: Path = CONFIG_PATH) -> RoleRegistry parses "
     "roles.json via RoleRegistry.model_validate. def reload_roles(path) "
     "-> None rebuilds the module-level cache from a fresh parse, used to "
     "pick up config changes without restarting the process.")

note("code-automation-rbac-actions-touch-device", "code", "verified", "2026-09-05",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/automation_rbac.py:actions_touch_device",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: app/core/automation_rbac.py\nsymbol: actions_touch_device\n\n"
     "Function at line 61 of automation_rbac.py determines whether a set "
     "of automation actions touch a given device, forming the role-based "
     "access control boundary applied to automation actions.")

note("code-time-range-resolve-time-expression", "code", "verified", "2026-10-26",
     "crates/index/src/time_range.rs:resolve_time_expression",
     "repository: superbrain\ncommit: see corpus-manifest.json build_commit\n"
     "path: crates/index/src/time_range.rs\nsymbol: resolve_time_expression\n\n"
     "pub fn resolve_time_expression(expr: &TimeExpression, "
     "reference_date: NaiveDate) -> Result<(DateTime<Tz>, DateTime<Tz>), "
     "TimeResolutionError>\n\n"
     "Resolves local calendar boundaries to a half-open interval in "
     "Europe/Rome. MonthsAgo always means a whole calendar month, never "
     "a rolling count of days.")

note("code-propose-propose-and-commit", "code", "verified", "2026-10-26",
     "crates/storage/src/propose.rs:propose_and_commit",
     "repository: superbrain\ncommit: see corpus-manifest.json build_commit\n"
     "path: crates/storage/src/propose.rs\nsymbol: propose_and_commit\n\n"
     "pub fn propose_and_commit(vault: &Vault, note_text: &str, "
     "expected_version: Version, operation_id: Uuid, known_ids: "
     "&HashSet<NoteId>, pending_ids: &HashSet<String>, gate: "
     "&policy::Gate, pii_store: Option<&PiiStore>) -> "
     "Result<CommitResult, ProposeError>\n\n"
     "The one entry point that parses a note's frontmatter, validates "
     "its relations, and commits it through the gate.")

note("code-api-lib-b1-callable-core", "code", "verified", "2026-10-30",
     "crates/api/src/lib.rs",
     "repository: superbrain\ncommit: see corpus-manifest.json build_commit\n"
     "path: crates/api/src/lib.rs\nsymbol: module doc\n\n"
     "B1's callable core is synchronous, with no timeout, retry "
     "scheduler, size/rate cap, or remote identity yet. Chunk ids are "
     "explicitly rejected until B2. No backing data is ever fabricated "
     "when a field is unavailable.")

# --- cross-file-concept ---
note("code-roles-add-remove-role-shared-state", "code", "verified", "2026-09-01",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:add_role",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: Access_control/src/roles.py\nsymbol: add_role, remove_role\n\n"
     "add_role(role_name, description, permissions) raises ValueError if "
     "the role already exists in the module-level _registry dict; "
     "remove_role(role_name) raises KeyError if it does not exist. Both "
     "mutate the same dict returned by _get_registry(), the shared state "
     "also read by role_has_permission and list_roles.")

note("code-automation-rbac-collect-device-targets", "code", "verified", "2026-09-05",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/automation_rbac.py:collect_device_targets",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: app/core/automation_rbac.py\nsymbol: collect_device_targets\n\n"
     "Function at line 112 of automation_rbac.py collects the device "
     "targets implicated by an automation rule; its output feeds the "
     "RBAC check performed by actions_touch_device in the same module.")

note("code-schedule-lock-guardrail-evaluate", "code", "verified", "2026-09-20",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/schedule_lock_guardrail.py:evaluate_schedule_lock_guardrail",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: app/core/schedule_lock_guardrail.py\n"
     "symbol: evaluate_schedule_lock_guardrail\n\n"
     "Function at line 185 evaluates whether a schedule change is "
     "blocked by a lock guardrail, walking condition trees via the "
     "internal _walk helper (line 102) and the _LockEvent NamedTuple "
     "(line 96) defined earlier in the same module.")

note("code-schedule-lock-guardrail-extract-device-ids", "code", "verified", "2026-09-20",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/schedule_lock_guardrail.py:extract_trigger_condition_device_ids",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: app/core/schedule_lock_guardrail.py\n"
     "symbol: extract_trigger_condition_device_ids, "
     "extract_conditional_action_device_ids\n\n"
     "extract_trigger_condition_device_ids (line 59) extracts device ids "
     "referenced by a trigger's condition group, built on "
     "_device_ids_from_condition_group (line 45). "
     "extract_conditional_action_device_ids (line 78) does the "
     "equivalent for a conditional action. Both feed device ids into "
     "evaluate_schedule_lock_guardrail in the same module.")

note("code-document-text-searchable-classification", "code", "verified", "2026-10-30",
     "crates/index/src/document.rs",
     "repository: superbrain\ncommit: see corpus-manifest.json build_commit\n"
     "path: crates/index/src/document.rs\nsymbol: build_text_searchable\n\n"
     "text_searchable is built differently by classification: Sensitive "
     "notes include only the seven non-temporal allowlisted structured "
     "fields, never body, source, relations, or any temporal string; "
     "Internal and Public notes additionally include body, source, and "
     "every relations.* target.")

note("code-parse-note-id-shared-validator", "code", "verified", "2026-10-26",
     "crates/storage/src/propose.rs:parse_note_id",
     "repository: superbrain\ncommit: see corpus-manifest.json build_commit\n"
     "path: crates/storage/src/propose.rs\nsymbol: parse_note_id\n\n"
     "The internal function parse_note_id(field: &str, raw: &str) -> "
     "Result<NoteId, ProposeError> validates a candidate id string "
     "against NoteId's syntax, reused by propose_and_commit both for a "
     "note's own id and for every non-pending relations.* target.")

note("bug-2026-11-b2-id013-014-015-fixed", "bug", "verified", "2026-11-03",
     "continuity:d176ze",
     "The CommonJS Extraction Checkpoint closed with a GO verdict: B2-ID-013, "
     "B2-ID-014 and B2-ID-015 were all fixed and verified; a new, Moderate-"
     "severity finding B2-ID-016 was opened in the same checkpoint; A5 was "
     "re-confirmed as still not justified by the evidence.")

note("bug-2026-11-b2-eval012-ts-interfaces-missing", "bug", "reviewed", "2026-11-07",
     "continuity:d183dc",
     "B2-EVAL-012 (TypeScript backend, evaluated against "
     "domain-driven-hexagon@5c2d15a) was paused on a new finding: "
     "TypeScript interface_declaration nodes were never extracted by the "
     "parser — zero of 18 real interfaces in that codebase were modeled, "
     "and zero Implements edges were created to any local interface.")

note("decision-2026-11-10-b2-closure-authorized", "decision", "verified", "2026-11-10",
     "continuity:d189ks",
     "The B2 Closure Summary was authorized as the final B2 task, with no "
     "new code to be written for it. B2-EVAL-013, A5 and Task 10 were all "
     "explicitly confirmed as still not started at this point.")

note("decision-2026-11-12-b2-closure-summary-complete", "decision", "verified", "2026-11-12",
     "continuity:d190t6",
     "The B2 Closure Summary was completed with a GO verdict to formally "
     "close B2, recorded in docs/research/B2-CLOSURE-SUMMARY.md.",
     relations={"derived_from": ["decision-2026-11-10-b2-closure-authorized"]})

note("code-roles-list-roles", "code", "verified", "2026-09-01",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:list_roles",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: Access_control/src/roles.py\nsymbol: list_roles, get_role\n\n"
     "def list_roles() -> list[str] returns list(_get_registry().keys()). "
     "def get_role(role_name: str) -> RoleModel raises KeyError with a "
     "message in Italian (\"Ruolo '{role_name}' non trovato.\") if the "
     "role is missing from the registry.")

note("code-sun-times-reset-cache-for-tests", "code", "verified", "2026-09-10",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/sun_times.py:_reset_cache_for_tests",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: app/core/sun_times.py\nsymbol: _reset_cache_for_tests\n\n"
     "def _reset_cache_for_tests() -> None clears the module-level "
     "_cache_key/_cache_value globals used to memoize get_sun_times_today; "
     "any test that mocks the sun calculation must call this first, per "
     "the function's own docstring pointing at "
     "tests/unit/test_sun_times_consolidation.py.")

note("bug-2026-11-08-b2-id018-opened", "bug", "reviewed", "2026-11-08",
     "continuity:d184pp",
     "B2-ID-018 was opened as an Important finding: TypeScript "
     "interface_declaration nodes were not extracted by the code graph "
     "indexer. A strict fix was authorized and dispatched to a "
     "background fork (agent a6c0f3e287d82e87b).")

note("bug-2026-11-09-b2-id019-020-opened", "bug", "reviewed", "2026-11-09",
     "continuity:d186zr",
     "B2-ID-018 was approved closed. Two new Important findings were "
     "opened in the same pass: B2-ID-019 (generic heritage names not "
     "handled) and B2-ID-020 (abstract_class_declaration nodes invisible "
     "to the indexer). Strict fixes for both were authorized and "
     "dispatched in sequence to a background fork (agent a229007a81582138f).",
     relations={"derived_from": ["bug-2026-11-08-b2-id018-opened"]})

note("bug-2026-11-10-b2-id019-020-closed", "bug", "verified", "2026-11-10",
     "continuity:d187ew",
     "B2-ID-019 and B2-ID-020 were both closed with an APPROVE verdict "
     "at commits 6944094 and d60d0c2, with zero genuine missed mutants "
     "in the mutation-testing gate. B2-EVAL-012 (the paused TS backend "
     "evaluation) was declared technically ready to resume, left as the "
     "user's own call on timing.",
     relations={"supersedes": ["bug-2026-11-09-b2-id019-020-opened"]})

note("decision-2026-11-04-b2-eval011-html-css-complete", "decision", "verified", "2026-11-04",
     "continuity:d181cf",
     "B2-EVAL-011 (HTML/CSS domain, evaluated against "
     "tbreuss/html-boilerplate@01d53fa) completed with a GO verdict to "
     "move to the next domain; A5 was re-confirmed as still not "
     "justified by the evidence collected so far.")

note("session-2026-11-02-b2-eval009-cpp-dispatched", "session", "reviewed", "2026-11-02",
     "continuity:d172ir",
     "B2-EVAL-009, the C++ domain evaluation, was dispatched to a "
     "background fork (agent ad0d465264f7dc553) to run independently of "
     "the main session.")

note("decision-2026-11-03-b2-eval010-js-commonjs-complete", "decision", "verified", "2026-11-03",
     "continuity:d174xx",
     "B2-EVAL-010 (JS/CommonJS domain, evaluated against "
     "request/request@3c0cddc) completed with a GO verdict to move to "
     "the next domain; A5 was again confirmed as not yet justified.")

note("code-roles-remove-role-raises-keyerror", "code", "verified", "2026-09-01",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:remove_role",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: Access_control/src/roles.py\nsymbol: remove_role\n\n"
     "def remove_role(role_name: str) -> None looks up the module-level "
     "_registry via _get_registry(), raises KeyError if role_name is "
     "absent, otherwise deletes that entry with `del registry[role_name]`.")

note("code-roles-config-path-constant", "code", "verified", "2026-09-01",
     "controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py",
     "repository: controllore\ncommit: ec274f4680c19f5b335a01a7a563c8f64546e7e2\n"
     "path: Access_control/src/roles.py\nsymbol: CONFIG_PATH\n\n"
     "CONFIG_PATH = Path(__file__).parent / \"config\" / \"roles.json\" "
     "is the default path every role function reads from unless a "
     "caller passes an explicit path override, used as load_roles's and "
     "reload_roles's default argument.")

note("code-document-sensitive-vs-internal-fields", "code", "verified", "2026-10-30",
     "crates/index/src/document.rs",
     "repository: superbrain\ncommit: see corpus-manifest.json build_commit\n"
     "path: crates/index/src/document.rs\nsymbol: structured_fields_as_text\n\n"
     "The seven non-temporal allowlisted fields kept as searchable text "
     "even for Sensitive notes are id, schema_version, type, status, "
     "trust, classification, and project — joined with a space, in that "
     "fixed order, by structured_fields_as_text.")

note("code-search-provenance-withheld-for-sensitive", "code", "verified", "2026-10-30",
     "crates/index/src/search.rs",
     "repository: superbrain\ncommit: see corpus-manifest.json build_commit\n"
     "path: crates/index/src/search.rs\nsymbol: Provenance\n\n"
     "The Provenance struct's source field is withheld on the search "
     "surface for Sensitive notes, including in its Debug output — "
     "repository_id and commit stay None because B1 note records carry "
     "no repository/commit provenance and the code never guesses it.")

print(f"Prepared {len(NOTES)} notes.")

def write_note(n):
    relations_yaml = "relations: {}" if not n["relations"] else (
        "relations:\n" + "\n".join(f"  {k}:\n" + "\n".join(f"    - {v}" for v in vs) for k, vs in n["relations"].items())
    )
    fm = (
        f"---\n"
        f"id: {n['id']}\n"
        f"schema_version: 1\n"
        f"type: {n['type']}\n"
        f"status: {n['status']}\n"
        f"trust: {n['trust']}\n"
        f"classification: {n['classification']}\n"
        f"project: {n['project']}\n"
        f"source: {n['source']}\n"
        f"created: {n['created']}\n"
        f"updated: {n['updated']}\n"
        f"event_period: {n['event_period']}\n"
        f"{relations_yaml}\n"
        f"---\n"
        f"{n['body']}\n"
    )
    (OUT / f"{n['id']}.md").write_text(fm, encoding="utf-8", newline="\n")

for n in NOTES:
    write_note(n)

print(f"Wrote {len(NOTES)} files to {OUT}")

by_cat = {}
for n in NOTES:
    pass  # category isn't stored on NOTES; tallied separately in provenance task
