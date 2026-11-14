"""Task 4 Step 7 (v2, post-review fix): transcribe queries-draft.jsonl into
the three real labels.jsonl files, in the full extended schema.

v2 fix: the first version keyed CITATIONS by target NOTE id, so every
query aimed at the same note got the identical answer/span regardless
of what it actually asked (final review, Critical #1). This version
keys by QUERY id instead — each answer/span was authored by rereading
that specific question against its target note's real body text.

Also fixes Critical #2 (mistargeted relevance): q003 and q076 were
rewritten to match what their target note actually says (their
original wording didn't match any corpus note); q081 gained a second
relevant note (sec001-toctou) because the real answer to "is the check
before or after open" lives there, not only in sec003.
"""
import json
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

# Question-text corrections for the two mistargeted queries found by the
# final review (Critical #2). Applied before building the output rows.
QUESTION_OVERRIDES = {
    "q003": "Why does the original A1 symlink check in vault.rs count as checking the path first and acting on it later, instead of something more immediate?",
    "q076": "In mid-September, what cross-module import rule did a full repo sweep confirm had held up everywhere in controllore, before the timezone consolidation fix?",
}

# q081's draft target (sec003-hardlink-aliasing) doesn't actually state the
# before/after answer; sec001-toctou does. Swap: sec001 becomes primary,
# sec003 (the question's nominal subject) becomes secondary.
TARGET_OVERRIDES = {
    "q081": "bug-2026-10-27-sec001-toctou",
}

# Per-query (not per-note) citation span + answer. Each entry was authored
# by rereading QUESTION_OVERRIDES.get(id, original question) against the
# real body of its target note(s).
PER_QUERY = {
    "q001": ("the correct fix is to open the file without following links and verify the already-open handle itself",
              "Because the original check canonicalizes the path before use — a check-then-act pattern — so the filesystem can change between the check and the actual I/O; the fix is to verify the already-open handle instead."),
    "q002": ("a TOCTOU race", "A TOCTOU (time-of-check to time-of-use) race."),
    "q003": ("This is a check-then-act pattern: it blocks symlinks already present at check time, but the filesystem can change between the check and the actual I/O",
              "Because it blocks symlinks present at check time, but the filesystem can change between the check and the actual I/O — a check-then-act (TOCTOU) pattern."),
    "q004": ("makes read_note(note-a) return note-b's content and version",
              "read_note(note-a) returned note-b's content and version, and note-a inherited note-b's version number instead of starting from zero."),
    "q005": ("No escape from the vault and no cross-note write corruption, but a real integrity bug",
              "No — no escape from the vault and no cross-note write corruption, but it is a real content/version misattribution bug."),
    "q006": ("a hard link (not a symlink) to the same path produces the identical aliasing effect",
              "Because a hard link to the same path produces the identical aliasing effect, and canonicalization cannot see hard links, so rejecting only symlinks doesn't close the gap."),
    "q007": ("Canonicalization cannot detect this because a hard link is not a symlink pointing elsewhere",
              "No — canonicalization cannot detect it, because a hard link is a second directory entry to the same inode, not a symlink pointing elsewhere."),
    "q008": ("Reproduced empirically on Windows, 2026-10-27",
              "Windows."),
    "q009": ("dispatched to a background fork (agent a6c0f3e287d82e87b)",
              "Agent a6c0f3e287d82e87b, running as a background fork."),
    "q010": ("B2-ID-019 (generic heritage names not handled) and B2-ID-020 (abstract_class_declaration nodes invisible to the indexer)",
              "B2-ID-019 (generic heritage names not handled) and B2-ID-020 (abstract_class_declaration nodes invisible to the indexer)."),
    "q011": ("closed with an APPROVE verdict at commits 6944094 and d60d0c2, with zero genuine missed mutants",
              "Yes — both closed with an APPROVE verdict and zero genuine missed mutants in the mutation-testing gate."),
    "q012": ("zero of 18 real interfaces in that codebase were modeled",
              "Zero of the 18 real interfaces were modeled before the fix."),
    "q013": ("a new, Moderate-severity finding B2-ID-016 was opened in the same checkpoint",
              "A new Moderate-severity finding, B2-ID-016, was opened in the same checkpoint."),
    "q014": ("a gap in parser_for()'s HTML/CSS dispatch during the code graph's tree-sitter indexing",
              "A gap in parser_for()'s dispatch logic for HTML/CSS during tree-sitter indexing."),
    "q015": ("the Java language-support gap is NOT the same issue as B2-ID-012",
              "No — Java's gap is a separate, documented future-language-support gap, re-logged as not the same issue as B2-ID-012's resolve_lineage finding."),
    "q016": ("no timeout, retry scheduler, size/rate cap, or remote identity yet",
              "No — B1's callable core is synchronous with no retry scheduler at all yet."),
    "q017": ("Chunk ids are explicitly rejected until B2",
              "No — chunk ids are explicitly rejected until B2; only note ids work today."),
    "q018": ("determines whether a set of automation actions touch a given device",
              "actions_touch_device, at line 61 of automation_rbac.py."),
    "q019": ("collects the device targets implicated by an automation rule",
              "collect_device_targets (line 112), whose output feeds actions_touch_device's RBAC check."),
    "q020": ("id, schema_version, type, status, trust, classification, and project",
              "id, schema_version, type, status, trust, classification, and project."),
    "q021": ("joined with a space, in that fixed order, by structured_fields_as_text",
              "In that fixed order — id, schema_version, type, status, trust, classification, project — joined with a space."),
    "q022": ("Internal and Public notes additionally include body, source, and every relations.* target",
              "Body text, the source field, and every relations.* target — none of which a Sensitive note exposes."),
    "q023": ("reused by propose_and_commit both for a note's own id and for every non-pending relations.* target",
              "It's also reused to validate every non-pending relations.* target a note declares."),
    "q024": ("The one entry point that parses a note's frontmatter, validates its relations, and commits it through the gate",
              "propose_and_commit — it parses the frontmatter, validates relations, and commits through the gate."),
    "q025": ("raises ValueError if the role already exists in the module-level _registry dict",
              "ValueError."),
    "q026": ("Both mutate the same dict returned by _get_registry(), the shared state also read by role_has_permission and list_roles",
              "Yes — add_role and remove_role mutate the same _registry dict that role_has_permission (and list_roles) read from."),
    "q027": ("Path(__file__).parent / \"config\" / \"roles.json\"",
              "Under a config/roles.json path relative to the roles module's own file, unless a caller overrides it."),
    "q028": ("a message in Italian (\"Ruolo '{role_name}' non trovato.\")",
              "Italian."),
    "q029": ("rebuilds the module-level cache from a fresh parse, used to pick up config changes without restarting",
              "Yes — reload_roles(path) rebuilds the cache from a fresh parse without restarting the process."),
    "q030": ("raises KeyError if role_name is absent",
              "It raises KeyError."),
    "q031": ("returns False if the role does not exist (KeyError caught)",
              "No — it catches the KeyError internally and returns False instead."),
    "q032": ("the internal _walk helper (line 102) and the _LockEvent NamedTuple (line 96)",
              "The internal _walk helper (line 102) and the _LockEvent NamedTuple (line 96)."),
    "q033": ("Both feed device ids into evaluate_schedule_lock_guardrail in the same module",
              "They each extract device ids from a different source (trigger conditions vs. conditional actions) and both feed evaluate_schedule_lock_guardrail."),
    "q034": ("withheld on the search surface for Sensitive notes, including in its Debug output",
              "No — the source field is withheld on the search surface for Sensitive notes, including in Debug output."),
    "q035": ("or None on any failure",
              "None."),
    "q036": ("The `today` parameter exists only so tests can inject a fixed date; production callers never pass it",
              "No — the today parameter exists only for tests; production callers never pass it."),
    "q037": ("any test that mocks the sun calculation must call this first",
              "It must call _reset_cache_for_tests() first, per the function's own docstring."),
    "q038": ("MonthsAgo always means a whole calendar month, never a rolling count of days",
              "No — it's always a whole calendar month, never a rolling 60-day count."),
    "q039": ("Resolves local calendar boundaries to a half-open interval in Europe/Rome",
              "Europe/Rome."),
    "q040": ("returns timezone.utc on any exception, logged at debug level, never raised to the caller",
              "timezone.utc, logged at debug level, never raised to the caller."),
    "q041": ("A1 (Schema e storage) was completed and merged to main, tagged a1-complete",
              "A1 (Schema e storage) — merged to main and tagged a1-complete."),
    "q042": ("it is verified that the model cannot generate or forge that confirmation",
              "It has to be verified that the model cannot generate or forge that confirmation."),
    "q043": ("the only trusted human-approval channel is a local CLI, `bc approve <proposal_id>`, which shows the diff before confirming",
              "A local CLI, bc approve <proposal_id>, which shows the diff before confirming."),
    "q044": ("CPU embeddings are introduced only if they measurably improve the benchmark",
              "Only if they measurably improve the benchmark without pushing latency or RAM past the stated limits."),
    "q045": ("frozen on 2026-10-26 after four external reviews concluded",
              "Four, concluded on 2026-10-26."),
    "q046": ("From that point the specification changes only from implementation and test evidence, never from new general architectural revisions",
              "Only implementation and test evidence — never new general architectural revisions."),
    "q047": ("unconditionally rejected Sensitive notes forever, treated the PiiStore and the Markdown pointer as two separately-recovered transactions, added an unauthenticated deletion-log reference file, required a Task 0 test that SEC-00x stay vulnerable, and added a new propose_delete public entry point",
              "Unconditionally rejecting Sensitive notes forever, treating PiiStore and the Markdown pointer as two separate transactions, adding an unauthenticated deletion-log reference file, requiring a test that SEC-00x stay vulnerable, and adding a new propose_delete entry point."),
    "q048": ("request/request@3c0cddc",
              "request/request, pinned at commit 3c0cddc."),
    "q049": ("A5 was re-confirmed as still not justified by the evidence collected so far",
              "No — A5 was re-confirmed as still not justified by the evidence."),
    "q050": ("B2-EVAL-013, A5 and Task 10 were all explicitly confirmed as still not started",
              "B2-EVAL-013, A5, and Task 10."),
    "q051": ("with no new code to be written for it",
              "No — the task was authorized with no new code to be written for it."),
    "q052": ("recorded in docs/research/B2-CLOSURE-SUMMARY.md",
              "docs/research/B2-CLOSURE-SUMMARY.md."),
    "q053": ("explicitly not A5 and not Task 10",
              "A5 and Task 10 were both explicitly ruled out as the next step."),
    "q054": ("was confirmed CLOSED directly by the user",
              "The user, directly."),
    "q055": ("treat a stray off-scope fork's untracked-file audit",
              "Kept separate — folding it into B2-EVAL-010's scope was rejected."),
    "q056": ("discarding the b2-eval-008-polly-run.txt file",
              "b2-eval-008-polly-run.txt."),
    "q057": ("this boundary had never been violated",
              "No — it was verified across the whole repo that this boundary had never been violated."),
    "q058": ("resolve_configured_timezone() was extracted into app/core/timezone_resolver.py specifically to be the one point that builds the tz_provider injected into AutomationEngine/Scheduler, which stay unaware of app/routers/settings_models",
              "Because app/core/ is where the sole point that builds the tz_provider for AutomationEngine/Scheduler lives, keeping the automation engine unaware of app/routers/settings_models."),
    "q059": ("It deliberately leaves _check_sun_events()'s slightly different variant untouched",
              "No — _check_sun_events() uses a deliberately different variant (no UTC fallback, no try/except, an early return instead)."),
    "q060": ("no try/except, an early return instead when lat/lon aren't configured",
              "It does an early return with no try/except, instead of falling back to UTC like the newer helper does."),
    "q061": ("it returns None if the astral library is not installed, if latitude/longitude are not configured, or if the sun-time computation fails for any other reason",
              "Never — it returns None on any failure instead of raising."),
    "q062": ("a note with no relations at all is not rejected for that reason alone",
              "No, it is not rejected for that reason alone."),
    "q063": ("schema validity, link validity, evidence presence, verification status, and retrieval eligibility",
              "Schema validity, link validity, evidence presence, verification status, and retrieval eligibility."),
    "q064": ("The deletion log and its trusted reference cannot update atomically, so a three-step protocol applies",
              "Because the deletion log and its trusted reference cannot update atomically, so it needs: write+sync the pending generation, update+sync the trusted reference, then mark it completed."),
    "q065": ("Deletion is not considered complete until step three",
              "Only at step three — marking the generation as completed."),
    "q066": ("none ever does a free scan of vault paths during a write",
              "No — every internal component reads only committed versions through the core, never by scanning paths directly."),
    "q067": ("A write only becomes visible through the API once it reaches journal_committed, the logical linearization point",
              "Once it reaches journal_committed — the logical linearization point."),
    "q068": ("a check on a path cannot see how that path will resolve when it is actually opened",
              "A check on a path cannot see how that path will resolve when it is actually opened."),
    "q069": ("verify the already-open handle — regular file, link count 1, path inside root",
              "Verify the already-open handle: regular file, link count 1, path inside root."),
    "q070": ("dispatched to a background fork (agent ad0d465264f7dc553)",
              "Agent ad0d465264f7dc553, running as a background fork."),
    "q071": ("a missing B2-EVAL-011.md evidence artifact and a wiring gap between CallAttempt/negative-knowledge and the incremental indexer",
              "A missing B2-EVAL-011.md evidence artifact, and a wiring gap between CallAttempt/negative-knowledge and the incremental indexer."),
    "q072": ("recorded against commits 59510ae and f50c634",
              "59510ae and f50c634."),
    "q073": ("moderate overhead",
              "Moderate, not severe."),
    "q074": ("A1 introduced anti-symlink checks in crates/storage/src/vault.rs",
              "A check-then-act TOCTOU gap in the anti-symlink checks added in A1, in crates/storage/src/vault.rs."),
    "q075": ("A1 (Schema e storage) was completed and merged to main, tagged a1-complete",
              "A1 (Schema e storage), merged to main and tagged a1-complete."),
    "q076": ("it was verified across the whole controllore repo that no module under Automazione/engine/ imports from app/",
              "It was confirmed that no module under Automazione/engine/ imports from app/ — a boundary never violated, checked before the timezone consolidation fix."),
    "q077": ("B2 (Code Graph) phase was formally closed",
              "B2 (Code Graph) phase was formally closed."),
    "q078": ("closed with an APPROVE verdict at commits 6944094 and d60d0c2",
              "B2-ID-019 and B2-ID-020 (generic heritage names and abstract_class_declaration), closed at commits 6944094 and d60d0c2."),
    "q079": ("CALL-RES-A4's performance cost was measured during B2: moderate overhead",
              "CALL-RES-A4's performance cost — moderate overhead."),
    "q080": ("Reproduced empirically on Windows, 2026-10-27",
              "A hard link outside the vault aliasing read_note's content, reproduced on Windows."),
    "q081": ("verify the already-open handle itself (regular file, link count 1, path inside root), not the path before opening it",
              "After — the correct fix (shared with the hard-link aliasing case) is to verify the already-open handle, not add another check before opening the file."),
    "q082": ("note-a inherits note-b's version number instead of starting from zero",
              "It becomes note-b's version number, not zero."),
    "q083": ("closed with an APPROVE verdict at commits 6944094 and d60d0c2",
              "Commits 6944094 and d60d0c2."),
    "q084": ("A structurally valid note does not automatically become verified",
              "No — a structurally valid note does not automatically become verified just by having links."),
    "q085": ("a message in Italian (\"Ruolo '{role_name}' non trovato.\")",
              "\"Ruolo '{role_name}' non trovato.\" — Italian."),
    "q086": ("returns False if the role does not exist (KeyError caught)",
              "No — it catches KeyError internally and returns False."),
    "q087": ("parses roles.json via RoleRegistry.model_validate",
              "A RoleRegistry, built via RoleRegistry.model_validate."),
    "q088": ("the _LockEvent NamedTuple (line 96) defined earlier in the same module",
              "_LockEvent."),
    "q089": ("zero of 18 real interfaces in that codebase were modeled",
              "Zero of 18."),
    "q090": ("MonthsAgo always means a whole calendar month, never a rolling count of days",
              "No — never a sliding day count, always a whole calendar month."),
    "q091": ("CPU embeddings are introduced only if they measurably improve the benchmark without pushing latency or RAM past the stated limits",
              "The statement that CPU embeddings are introduced only if they measurably improve the benchmark without pushing latency or RAM past the stated limits."),
    "q092": ("tagged a1-complete",
              "Yes — tagged a1-complete."),
    "q093": ("tbreuss/html-boilerplate@01d53fa",
              "tbreuss/html-boilerplate, pinned at commit 01d53fa."),
    "q094": ("a check on a path cannot see how that path will resolve when it is actually opened",
              "A check on a path cannot see how that path will resolve when it is actually opened — the same root cause for both bugs."),
    "q095": ("reusing the same try/except pattern _check_programs() in watchdogs.py already used",
              "_check_programs()."),
    "q096": ("Deletion is not considered complete until step three",
              "No — per the three-step protocol, deletion isn't complete until step three, so a restore can't expose data from an incomplete deletion."),
    "q097": ("A write only becomes visible through the API once it reaches journal_committed",
              "A tool reading the vault directly can see an uncommitted write; the API only ever shows writes once they reach journal_committed."),
    "q098": ("B2-ID-019 (generic heritage names not handled) and B2-ID-020 (abstract_class_declaration nodes invisible to the indexer)",
              "B2-ID-019 and B2-ID-020 — generic heritage names and abstract_class_declaration nodes."),
    "q099": ("the Java language-support gap is NOT the same issue as B2-ID-012, which stays assigned to a pre-existing resolve_lineage O(n^2) performance finding",
              "No — they're separate: B2-ID-012 is the resolve_lineage O(n^2) finding, Java's gap is a documented future-language-support gap."),
    "q100": ("a new, Moderate-severity finding B2-ID-016 was opened in the same checkpoint",
              "Moderate."),
}

# q081 gets a SECOND relevant note (sec001-toctou), since the real answer
# ("after" / verify-the-open-handle) is stated there, not only in sec003.
# Handled specially below rather than via PER_QUERY's 2-tuple shape.
SECONDARY_RELEVANCE = {
    "q081": [("bug-2026-10-27-sec003-hardlink-aliasing",
              "Canonicalization cannot detect this because a hard link is not a symlink pointing elsewhere",
              1)],
    "q094": [("bug-2026-10-27-sec002-fix-incomplete",
              "canonicalization does not see hard links",
              1)],
    "q007": [("bug-2026-10-27-sec002-fix-incomplete",
              "canonicalization does not see hard links",
              1)],
}

violations = []
out_rows = {"dev": [], "calibration": [], "test": []}

with open(DRAFT, encoding="utf-8") as fh:
    for line in fh:
        line = line.strip()
        if not line:
            continue
        r = json.loads(line)
        target = TARGET_OVERRIDES.get(r["id"], r["target"])
        question = QUESTION_OVERRIDES.get(r["id"], r["question"])
        if target is None:
            row = {
                "id": r["id"], "question": question, "answer": "",
                "category": r["category"], "difficulty": r["difficulty"],
                "abstain_expected": True,
                "relevant_note_ids": [], "graded_relevance": {}, "citation_spans": [],
            }
        else:
            entry = PER_QUERY.get(r["id"])
            if entry is None:
                violations.append(f"{r['id']}: no PER_QUERY entry for target {target}")
                continue
            span, answer = entry
            body = note_bodies.get(target, "")
            if span not in body:
                violations.append(f"{r['id']} -> {target}: primary citation span not found verbatim in body")
                continue

            relevant_note_ids = [target]
            graded_relevance = {target: 2}
            citation_spans = [{"note_id": target, "excerpt_substring": span}]

            for extra_note, extra_span, extra_grade in SECONDARY_RELEVANCE.get(r["id"], []):
                extra_body = note_bodies.get(extra_note, "")
                if extra_span not in extra_body:
                    violations.append(f"{r['id']} -> {extra_note}: secondary citation span not found verbatim in body")
                    continue
                relevant_note_ids.append(extra_note)
                graded_relevance[extra_note] = extra_grade
                citation_spans.append({"note_id": extra_note, "excerpt_substring": extra_span})

            row = {
                "id": r["id"], "question": question, "answer": answer,
                "category": r["category"], "difficulty": r["difficulty"],
                "abstain_expected": False,
                "relevant_note_ids": relevant_note_ids,
                "graded_relevance": graded_relevance,
                "citation_spans": citation_spans,
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
