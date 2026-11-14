"""Task 6 Step 1: hand-authored eval/rag/corpus-provenance.json.

Category assignment and continuity decision text copied verbatim here
match what was already verified during Task 2 (search_claims/why calls)
and recorded in the ledger. Never inferred from note id prefixes.
"""
import json
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "corpus-provenance.json"

CONTROLLORE_COMMIT = "ec274f4680c19f5b335a01a7a563c8f64546e7e2"

# category assignment, exactly as tallied manually after Task 2.
CATEGORY = {
    # bugs-fixes
    "bug-2026-10-27-sec001-toctou": "bugs-fixes",
    "bug-2026-10-27-sec002-fix-incomplete": "bugs-fixes",
    "bug-2026-10-27-sec002-symlink-aliasing": "bugs-fixes",
    "bug-2026-10-27-sec003-hardlink-aliasing": "bugs-fixes",
    "bug-2026-11-08-b2-id018-opened": "bugs-fixes",
    "bug-2026-11-09-b2-id019-020-opened": "bugs-fixes",
    "bug-2026-11-10-b2-id019-020-closed": "bugs-fixes",
    "bug-2026-11-b2-eval012-ts-interfaces-missing": "bugs-fixes",
    "bug-2026-11-b2-id013-014-015-fixed": "bugs-fixes",
    "bug-2026-11-b2-id017-parser-dispatch": "bugs-fixes",
    "bug-2026-11-java-scope-correction": "bugs-fixes",
    # notes-decisions (decision + session notes)
    "decision-2026-10-26-a1-complete": "notes-decisions",
    "decision-2026-10-26-c2-approval-channel": "notes-decisions",
    "decision-2026-10-26-faseb-deterministic-baseline": "notes-decisions",
    "decision-2026-10-26-spec-frozen": "notes-decisions",
    "decision-2026-10-a3-pii-design-rejected": "notes-decisions",
    "decision-2026-11-03-b2-eval010-js-commonjs-complete": "notes-decisions",
    "decision-2026-11-04-b2-eval011-html-css-complete": "notes-decisions",
    "decision-2026-11-10-b2-closure-authorized": "notes-decisions",
    "decision-2026-11-12-b2-closure-summary-complete": "notes-decisions",
    "decision-2026-11-14-b2-closed": "notes-decisions",
    "decision-2026-11-14-b2-confirmed-closed": "notes-decisions",
    "decision-2026-11-b2-eval010-scope-rejected": "notes-decisions",
    "session-2026-11-02-b2-eval009-cpp-dispatched": "notes-decisions",
    "session-2026-11-14-b2-closure-residuals": "notes-decisions",
    "session-2026-11-call-res-a4-perf": "notes-decisions",
    # patterns-knowledge
    "pattern-2026-09-11-automation-engine-boundary": "patterns-knowledge",
    "pattern-2026-09-11-timezone-fallback-reuse": "patterns-knowledge",
    "pattern-2026-09-sun-times-never-raises": "patterns-knowledge",
    "pattern-2026-10-26-c6-independent-gate-properties": "patterns-knowledge",
    "pattern-2026-10-26-deletion-log-three-step": "patterns-knowledge",
    "pattern-2026-10-26-write-visibility-linearization": "patterns-knowledge",
    "pattern-2026-10-27-handle-not-path": "patterns-knowledge",
    # identifier-code
    "code-api-lib-b1-callable-core": "identifier-code",
    "code-automation-rbac-actions-touch-device": "identifier-code",
    "code-document-sensitive-vs-internal-fields": "identifier-code",
    "code-roles-config-path-constant": "identifier-code",
    "code-roles-list-roles": "identifier-code",
    "code-roles-load-roles-reload-roles": "identifier-code",
    "code-roles-remove-role-raises-keyerror": "identifier-code",
    "code-roles-role-has-permission": "identifier-code",
    "code-search-provenance-withheld-for-sensitive": "identifier-code",
    "code-sun-times-get-sun-times-today": "identifier-code",
    "code-time-range-resolve-time-expression": "identifier-code",
    "code-timezone-resolver-resolve-configured-timezone": "identifier-code",
    # cross-file-concept
    "code-automation-rbac-collect-device-targets": "cross-file-concept",
    "code-document-text-searchable-classification": "cross-file-concept",
    "code-parse-note-id-shared-validator": "cross-file-concept",
    "code-propose-propose-and-commit": "identifier-code",
    "code-roles-add-remove-role-shared-state": "cross-file-concept",
    "code-schedule-lock-guardrail-evaluate": "cross-file-concept",
    "code-schedule-lock-guardrail-extract-device-ids": "cross-file-concept",
    "code-sun-times-reset-cache-for-tests": "cross-file-concept",
}

# Verbatim decision text (copied from what `why`/`search_claims` actually
# returned during Task 1/2 of this session), keyed by continuity id.
CONTINUITY_TEXT = {
    "d184pp": "B2-ID-018 opened (Important): TS interface_declaration not extracted — strict fix authorized and dispatched to background fork (agent a6c0f3e287d82e87b)",
    "d186zr": "B2-ID-018 approved CLOSED; B2-ID-019 (generic heritage names) and B2-ID-020 (abstract_class_declaration invisible) opened as Important, strict fixes authorized and dispatched in sequence to background fork (agent a229007a81582138f)",
    "d187ew": "B2-ID-019 and B2-ID-020 CLOSED (commits 6944094, d60d0c2) — both APPROVE, 0 genuine missed mutants; B2-EVAL-012 technically ready to resume (user's own call)",
    "d183dc": "B2-EVAL-012 (TS backend, domain-driven-hexagon@5c2d15a) PAUSED — new finding: TS interface_declaration never extracted (0/18 interfaces modeled, 0 Implements edges to local interfaces)",
    "d176ze": "CommonJS Extraction Checkpoint CLOSED (GO): B2-ID-013/014/015 all FIXED; new B2-ID-016 (Moderate) opened; A5 still not justified",
    "d180lt": "B2-ID-017 CLOSED (confirmed): parser_for() HTML/CSS dispatch gap fixed and verified",
    "d170ij": "Correction: Java scope gap is NOT B2-ID-012 — that ID stays assigned to the pre-existing resolve_lineage O(n^2) finding; Java logged as documented future-language-support gap, not a defect",
    "m1j5": "A1 (Schema e storage) COMPLETED and merged to main — tag a1-complete",
    "x1ks": "A3 draft 1 design: gate unconditionally rejects Sensitive forever, PiiStore+Markdown pointer as two separately-recovered transactions, unauthenticated deletion-log reference file, Task 0 test requiring SEC-00x to stay vulnerable, and a new propose_delete public entry point [REJECTED]",
    "d174xx": "B2-EVAL-010 (JS/CommonJS domain, request/request@3c0cddc) COMPLETE — GO to next domain, A5 still not justified",
    "d181cf": "B2-EVAL-011 (HTML/CSS domain, tbreuss/html-boilerplate@01d53fa) COMPLETE — GO to next domain, A5 still not justified",
    "d189ks": "B2 Closure Summary authorized as final B2 task — no new code, B2-EVAL-013/A5/Task 10 still not started",
    "d190t6": "B2 Closure Summary complete — GO to close B2, docs/research/B2-CLOSURE-SUMMARY.md",
    "m2g9": "B2 (Code Graph) phase CLOSED — next macro-step is the non-blocking follow-up set below, not A5 or Task 10",
    "d191ij": "B2 — Code Graph: CLOSED / confirmed by user",
    "x2gi": "Treat the stray off-scope fork's untracked-file audit (EVAL-006/007/009 cleanup, including discarding b2-eval-008-polly-run.txt) as part of B2-EVAL-010 [REJECTED]",
    "d172ir": "B2-EVAL-009 (C++ domain) dispatched to background fork ad0d465264f7dc553",
    "d163pa": "CALL-RES-A4 performance cost measured: Moderate overhead (commits 59510ae/f50c634)",
}

CODE_SOURCE_RE_CONTROLLORE = "controllore@" + CONTROLLORE_COMMIT + ":"

def parse_source(note_id, source_line):
    if source_line.startswith(CODE_SOURCE_RE_CONTROLLORE):
        rest = source_line[len(CODE_SOURCE_RE_CONTROLLORE):]
        path, _, symbol = rest.partition(":")
        return {
            "stratum": "code",
            "repository": "controllore",
            "commit": CONTROLLORE_COMMIT,
            "path": path,
            "symbol": symbol or None,
        }
    if source_line.startswith("crates/"):
        path, _, symbol = source_line.partition(":")
        return {
            "stratum": "code",
            "repository": "superbrain",
            "commit": None,  # filled from build_commit at manifest-build time
            "path": path,
            "symbol": symbol or None,
        }
    if source_line.startswith("continuity:"):
        cid = source_line.split(":", 1)[1]
        text = CONTINUITY_TEXT.get(cid)
        if text is None:
            raise SystemExit(f"no verbatim text recorded for continuity id {cid!r} ({note_id})")
        return {
            "stratum": "project-history",
            "source": f"continuity:{cid}",
            "continuity_id": cid,
            "continuity_text": text,
        }
    # docs/... path, possibly with a #anchor
    return {
        "stratum": "project-history",
        "source": source_line,
    }


def main():
    notes_dir = OUT.parent / "corpus" / "notes"
    provenance = {}
    for md in sorted(notes_dir.glob("*.md")):
        note_id = md.stem
        text = md.read_text(encoding="utf-8")
        source_line = None
        for line in text.splitlines():
            if line.startswith("source:"):
                source_line = line.split(":", 1)[1].strip()
                break
        if source_line is None:
            raise SystemExit(f"note {note_id} has no source: field")
        category = CATEGORY.get(note_id)
        if category is None:
            raise SystemExit(f"no category assigned for note {note_id}")
        entry = {"category": category}
        entry.update(parse_source(note_id, source_line))
        provenance[note_id] = entry

    OUT.write_text(json.dumps(provenance, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {len(provenance)} entries to {OUT}")


if __name__ == "__main__":
    main()
