"""Task 4 Step 7: transcribe queries-draft.jsonl into the three real
labels.jsonl files, in the full extended schema.

For each answerable query: relevant_note_ids = [target], graded_relevance
= {target: 2} (single clearly-relevant note per query, as authored),
citation_spans = one real verbatim substring of the target note's body
(a short, specific sentence fragment chosen to be identifiable but not
the query's own wording), answer = a short honest answer derived from
that same body text.

For abstain queries: answer = "", relevant_note_ids/graded_relevance/
citation_spans all empty, per the corrected schema.
"""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NOTES_DIR = ROOT / "corpus" / "notes"
DRAFT = Path(__file__).resolve().parent / "queries-draft.jsonl"

note_bodies = {}
for f in NOTES_DIR.glob("*.md"):
    text = f.read_text(encoding="utf-8")
    parts = text.split("---\n", 2)
    body = parts[2].strip() if len(parts) == 3 else text.strip()
    note_bodies[f.stem] = body

# Manually chosen citation span + short answer per target note id.
# span must be a real, exact substring of that note's body (verified below).
# answer is a short, honest, human-written summary (not required to be
# verbatim, but grounded in the body).
CITATIONS = {
    "bug-2026-10-27-sec001-toctou": (
        "the correct fix is to open the file without following links and verify the already-open handle itself",
        "Because the original check verifies the path, not the open handle, so the filesystem can change between the check and the actual I/O — the fix is to verify the already-open handle instead.",
    ),
    "bug-2026-10-27-sec002-symlink-aliasing": (
        "makes read_note(note-a) return note-b's content and version",
        "read_note(note-a) returned note-b's content and version, and note-a ended up inheriting note-b's version number instead of starting from zero.",
    ),
    "bug-2026-10-27-sec002-fix-incomplete": (
        "a hard link (not a symlink) to the same path produces the identical aliasing effect",
        "Because a hard link to the same path produces the identical aliasing effect, and canonicalization cannot see hard links, so rejecting only symlinks doesn't close the gap.",
    ),
    "bug-2026-10-27-sec003-hardlink-aliasing": (
        "Reproduced empirically on Windows, 2026-10-27",
        "It was reproduced on Windows.",
    ),
    "bug-2026-11-08-b2-id018-opened": (
        "dispatched to a background fork (agent a6c0f3e287d82e87b)",
        "Agent a6c0f3e287d82e87b, running as a background fork.",
    ),
    "bug-2026-11-09-b2-id019-020-opened": (
        "B2-ID-019 (generic heritage names not handled) and B2-ID-020 (abstract_class_declaration nodes invisible to the indexer)",
        "B2-ID-019 (generic heritage names not handled) and B2-ID-020 (abstract_class_declaration nodes invisible to the indexer).",
    ),
    "bug-2026-11-10-b2-id019-020-closed": (
        "closed with an APPROVE verdict at commits 6944094 and d60d0c2, with zero genuine missed mutants",
        "Yes — both closed with an APPROVE verdict and zero genuine missed mutants in the mutation-testing gate.",
    ),
    "bug-2026-11-b2-eval012-ts-interfaces-missing": (
        "zero of 18 real interfaces in that codebase were modeled",
        "Zero of the 18 real interfaces were modeled before the fix.",
    ),
    "bug-2026-11-b2-id013-014-015-fixed": (
        "a new, Moderate-severity finding B2-ID-016 was opened in the same checkpoint",
        "A new Moderate-severity finding, B2-ID-016, was opened in the same checkpoint.",
    ),
    "bug-2026-11-b2-id017-parser-dispatch": (
        "a gap in parser_for()'s HTML/CSS dispatch during the code graph's tree-sitter indexing",
        "A gap in parser_for()'s dispatch logic for HTML/CSS during tree-sitter indexing.",
    ),
    "bug-2026-11-java-scope-correction": (
        "which stays assigned to a pre-existing resolve_lineage O(n^2) performance finding",
        "No — Java's gap is a separate, documented future-language-support gap, not the same issue as B2-ID-012's resolve_lineage O(n^2) finding.",
    ),
    "code-api-lib-b1-callable-core": (
        "no timeout, retry scheduler, size/rate cap, or remote identity yet",
        "No — B1's callable core is synchronous with no retry scheduler at all yet.",
    ),
    "code-automation-rbac-actions-touch-device": (
        "determines whether a set of automation actions touch a given device",
        "actions_touch_device, at line 61 of automation_rbac.py.",
    ),
    "code-automation-rbac-collect-device-targets": (
        "collects the device targets implicated by an automation rule",
        "collect_device_targets (line 112), whose output feeds actions_touch_device's RBAC check.",
    ),
    "code-document-sensitive-vs-internal-fields": (
        "id, schema_version, type, status, trust, classification, and project",
        "id, schema_version, type, status, trust, classification, and project.",
    ),
    "code-document-text-searchable-classification": (
        "Internal and Public notes additionally include body, source, and every relations.* target",
        "Body text, the source field, and every relations.* target — none of which a Sensitive note exposes.",
    ),
    "code-parse-note-id-shared-validator": (
        "reused by propose_and_commit both for a note's own id and for every non-pending relations.* target",
        "It's also reused to validate every non-pending relations.* target a note declares.",
    ),
    "code-propose-propose-and-commit": (
        "The one entry point that parses a note's frontmatter, validates its relations, and commits it through the gate",
        "propose_and_commit — it parses the frontmatter, validates relations, and commits through the gate.",
    ),
    "code-roles-add-remove-role-shared-state": (
        "raises ValueError if the role already exists in the module-level _registry dict",
        "ValueError.",
    ),
    "code-roles-config-path-constant": (
        "Path(__file__).parent / \"config\" / \"roles.json\"",
        "Under a config/roles.json path relative to the roles module's own file, unless a caller overrides it.",
    ),
    "code-roles-list-roles": (
        "a message in Italian (\"Ruolo '{role_name}' non trovato.\")",
        "Italian.",
    ),
    "code-roles-load-roles-reload-roles": (
        "rebuilds the module-level cache from a fresh parse, used to pick up config changes without restarting",
        "Yes — reload_roles(path) rebuilds the cache from a fresh parse without restarting the process.",
    ),
    "code-roles-remove-role-raises-keyerror": (
        "raises KeyError if role_name is absent",
        "It raises KeyError.",
    ),
    "code-roles-role-has-permission": (
        "returns False if the role does not exist (KeyError caught)",
        "No — it catches the KeyError internally and returns False instead.",
    ),
    "code-schedule-lock-guardrail-evaluate": (
        "the internal _walk helper (line 102) and the _LockEvent NamedTuple (line 96)",
        "The internal _walk helper (line 102) and the _LockEvent NamedTuple (line 96).",
    ),
    "code-schedule-lock-guardrail-extract-device-ids": (
        "Both feed device ids into evaluate_schedule_lock_guardrail in the same module",
        "They each extract device ids from a different source (trigger conditions vs. conditional actions) and both feed evaluate_schedule_lock_guardrail.",
    ),
    "code-search-provenance-withheld-for-sensitive": (
        "withheld on the search surface for Sensitive notes, including in its Debug output",
        "No — the source field is withheld on the search surface for Sensitive notes, including in Debug output.",
    ),
    "code-sun-times-get-sun-times-today": (
        "or None on any failure",
        "It returns None.",
    ),
    "code-sun-times-reset-cache-for-tests": (
        "any test that mocks the sun calculation must call this first",
        "It must call _reset_cache_for_tests() first, per the function's own docstring.",
    ),
    "code-time-range-resolve-time-expression": (
        "MonthsAgo always means a whole calendar month, never a rolling count of days",
        "A whole calendar month, never a rolling day count.",
    ),
    "code-timezone-resolver-resolve-configured-timezone": (
        "returns timezone.utc on any exception, logged at debug level, never raised to the caller",
        "timezone.utc, logged at debug level, never raised to the caller.",
    ),
    "decision-2026-10-26-a1-complete": (
        "A1 (Schema e storage) was completed and merged to main, tagged a1-complete",
        "A1 (Schema e storage) was completed and merged to main, tagged a1-complete.",
    ),
    "decision-2026-10-26-c2-approval-channel": (
        "it is verified that the model cannot generate or forge that confirmation",
        "It has to be verified that the model cannot generate or forge that confirmation.",
    ),
    "decision-2026-10-26-faseb-deterministic-baseline": (
        "CPU embeddings are introduced only if they measurably improve the benchmark",
        "Only if they measurably improve the benchmark without pushing latency or RAM past the stated limits.",
    ),
    "decision-2026-10-26-spec-frozen": (
        "after four external reviews concluded",
        "Four, concluded on 2026-10-26.",
    ),
    "decision-2026-10-a3-pii-design-rejected": (
        "added a new propose_delete public entry point",
        "Unconditionally rejecting Sensitive notes forever, treating PiiStore and the Markdown pointer as two separate transactions, adding an unauthenticated deletion-log reference file, requiring a test that SEC-00x stay vulnerable, and adding a new propose_delete entry point.",
    ),
    "decision-2026-11-03-b2-eval010-js-commonjs-complete": (
        "request/request@3c0cddc",
        "request/request, pinned at commit 3c0cddc.",
    ),
    "decision-2026-11-04-b2-eval011-html-css-complete": (
        "A5 was re-confirmed as still not justified by the evidence collected so far",
        "No — A5 was re-confirmed as still not justified by the evidence.",
    ),
    "decision-2026-11-10-b2-closure-authorized": (
        "B2-EVAL-013, A5 and Task 10 were all explicitly confirmed as still not started",
        "B2-EVAL-013, A5, and Task 10.",
    ),
    "decision-2026-11-12-b2-closure-summary-complete": (
        "recorded in docs/research/B2-CLOSURE-SUMMARY.md",
        "docs/research/B2-CLOSURE-SUMMARY.md.",
    ),
    "decision-2026-11-14-b2-closed": (
        "explicitly not A5 and not Task 10",
        "A5 and Task 10 were both explicitly ruled out as the next step.",
    ),
    "decision-2026-11-14-b2-confirmed-closed": (
        "was confirmed CLOSED directly by the user",
        "The user, directly.",
    ),
    "decision-2026-11-b2-eval010-scope-rejected": (
        "treat a stray off-scope fork's untracked-file audit",
        "Kept separate — folding it into B2-EVAL-010's scope was rejected.",
    ),
    "pattern-2026-09-11-automation-engine-boundary": (
        "it was verified across the whole controllore repo that no module under Automazione/engine/ imports from app/",
        "No — it was verified across the whole repo that this boundary had never been violated.",
    ),
    "pattern-2026-09-11-timezone-fallback-reuse": (
        "resolve_configured_timezone() falls back to timezone.utc if settings are unavailable or invalid, reusing the same try/except pattern _check_programs() in watchdogs.py already used",
        "resolve_configured_timezone() reuses the same try/except pattern _check_programs() in watchdogs.py already used.",
    ),
    "pattern-2026-09-sun-times-never-raises": (
        "it returns None if the astral library is not installed, if latitude/longitude are not configured, or if the sun-time computation fails for any other reason",
        "Never — it returns None on any failure instead of raising.",
    ),
    "pattern-2026-10-26-c6-independent-gate-properties": (
        "a note with no relations at all is not rejected for that reason alone",
        "No, it is not rejected for that reason alone.",
    ),
    "pattern-2026-10-26-deletion-log-three-step": (
        "write and sync the new log generation marked pending; update and sync the trusted reference; only then mark the generation completed",
        "Because the deletion log and its trusted reference cannot update atomically, so it needs: write+sync the new generation marked pending, update+sync the trusted reference, then mark the generation completed.",
    ),
    "pattern-2026-10-26-write-visibility-linearization": (
        "none ever does a free scan of vault paths during a write",
        "No — every internal component reads only committed versions through the core, never by scanning paths directly.",
    ),
    "pattern-2026-10-27-handle-not-path": (
        "a check on a path cannot see how that path will resolve when it is actually opened",
        "A check on a path cannot see how that path will resolve when it is actually opened.",
    ),
    "session-2026-11-02-b2-eval009-cpp-dispatched": (
        "dispatched to a background fork (agent ad0d465264f7dc553)",
        "Agent ad0d465264f7dc553, running as a background fork.",
    ),
    "session-2026-11-14-b2-closure-residuals": (
        "a missing B2-EVAL-011.md evidence artifact, and a wiring gap between agent-policy/negative-knowledge and the incremental indexer",
        "A missing B2-EVAL-011.md evidence artifact, and a wiring gap between agent-policy/negative-knowledge and the incremental indexer.",
    ),
    "session-2026-11-call-res-a4-perf": (
        "recorded against commits 59510ae and f50c634",
        "59510ae and f50c634.",
    ),
}

# q064/q065 both target the same note but need different spans for variety;
# q068/q069 both target pattern-2026-10-27-handle-not-path. Reuse is fine —
# citation correctness only requires the span be a real substring, not unique
# per query.

violations = []
out_rows = {"dev": [], "calibration": [], "test": []}

with open(DRAFT, encoding="utf-8") as fh:
    for line in fh:
        line = line.strip()
        if not line:
            continue
        r = json.loads(line)
        target = r["target"]
        if target is None:
            row = {
                "id": r["id"], "question": r["question"], "answer": "",
                "category": r["category"], "difficulty": r["difficulty"],
                "abstain_expected": True,
                "relevant_note_ids": [], "graded_relevance": {}, "citation_spans": [],
            }
        else:
            if target not in CITATIONS:
                violations.append(f"{r['id']}: no citation entry for target {target}")
                continue
            span, answer = CITATIONS[target]
            body = note_bodies.get(target, "")
            if span not in body:
                violations.append(f"{r['id']} -> {target}: citation span not found verbatim in body")
                continue
            row = {
                "id": r["id"], "question": r["question"], "answer": answer,
                "category": r["category"], "difficulty": r["difficulty"],
                "abstain_expected": False,
                "relevant_note_ids": [target],
                "graded_relevance": {target: 2},
                "citation_spans": [{"note_id": target, "excerpt_substring": span}],
            }
        out_rows[r["split"]].append(row)

if violations:
    print(f"{len(violations)} VIOLATIONS - not writing output:")
    for v in violations:
        print(" ", v)
else:
    for split, rows in out_rows.items():
        path = ROOT / split / "labels.jsonl"
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            for row in rows:
                f.write(json.dumps(row) + "\n")
        print(f"{split}: wrote {len(rows)} rows to {path}")
