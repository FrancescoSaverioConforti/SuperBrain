"""Generate eval/rag/corpus-manifest.json from the explicit, hand-authored
corpus-provenance.json (never from note id prefixes) plus the three real
label files. Run once; the manifest is a frozen snapshot, not a service.
"""
import datetime
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

from runner import load_labels

ROOT = Path(__file__).resolve().parent
CONTROLLORE_COMMIT = "ec274f4680c19f5b335a01a7a563c8f64546e7e2"
CONTROLLORE_PATH = r"C:\Users\fsave\Desktop\Python_code\controllore"
SPLITS = ("dev", "calibration", "test")


def superbrain_head():
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=True
    )
    return result.stdout.strip()


def cross_check_provenance_vs_corpus(provenance, notes_dir):
    note_ids = {p.stem for p in notes_dir.glob("*.md")}
    provenance_ids = set(provenance.keys())
    missing_from_provenance = note_ids - provenance_ids
    missing_from_corpus = provenance_ids - note_ids
    if missing_from_provenance or missing_from_corpus:
        problems = []
        for nid in sorted(missing_from_provenance):
            problems.append(f"note {nid!r} has no corpus-provenance.json entry")
        for nid in sorted(missing_from_corpus):
            problems.append(f"corpus-provenance.json entry {nid!r} has no corpus note file")
        raise SystemExit("provenance/corpus mismatch:\n  " + "\n  ".join(problems))


def _body_provenance_lines(note_path):
    """Parses the plain-text repository:/commit:/path:/symbol: lines a
    `type: code` note's body carries (Task 2's authoring convention),
    distinct from the YAML frontmatter block."""
    text = note_path.read_text(encoding="utf-8")
    lines = text.splitlines()
    delimiters = [i for i, line in enumerate(lines) if line.strip() == "---"]
    if len(delimiters) < 2:
        return {}
    body_lines = lines[delimiters[1] + 1 :]
    fields = {}
    for line in body_lines:
        for key in ("repository", "commit", "path", "symbol"):
            prefix = f"{key}:"
            if line.startswith(prefix):
                fields[key] = line[len(prefix):].strip()
    return fields


def cross_check_code_note_body_provenance(provenance, notes_dir):
    """Fails loudly (returns problem strings) if a `stratum: code` note's
    body provenance lines drift from corpus-provenance.json's structured
    entry — the exact case Review Focus #4 required enforcement for."""
    problems = []
    for note_id, entry in provenance.items():
        if entry.get("stratum") != "code":
            continue
        note_path = notes_dir / f"{note_id}.md"
        if not note_path.exists():
            continue  # already reported by cross_check_provenance_vs_corpus
        body_fields = _body_provenance_lines(note_path)
        for key in ("repository", "commit", "path"):
            expected = entry.get(key)
            actual = body_fields.get(key)
            if expected != actual:
                problems.append(
                    f"{note_id}: body {key!r} ({actual!r}) does not match "
                    f"corpus-provenance.json {key!r} ({expected!r})"
                )
    return problems


def main():
    provenance = json.loads((ROOT / "corpus-provenance.json").read_text(encoding="utf-8"))
    notes_dir = ROOT / "corpus" / "notes"
    cross_check_provenance_vs_corpus(provenance, notes_dir)
    body_problems = cross_check_code_note_body_provenance(provenance, notes_dir)
    if body_problems:
        raise SystemExit("code note body/provenance mismatch:\n  " + "\n  ".join(body_problems))

    note_count_by_category = defaultdict(int)
    for entry in provenance.values():
        note_count_by_category[entry["category"]] += 1

    no_vault = ROOT.resolve().parent / "_build_manifest_no_vault_placeholder"
    query_counts = defaultdict(lambda: defaultdict(lambda: defaultdict(int)))
    for split in SPLITS:
        rows = load_labels(ROOT, split, no_vault)
        for row in rows:
            query_counts[split][row["category"]][row["difficulty"]] += 1

    manifest = {
        "build_date": datetime.date.today().isoformat(),
        "build_commit": superbrain_head(),
        "sources": {
            "superbrain": {"commit": superbrain_head()},
            "controllore": {"commit": CONTROLLORE_COMMIT, "path": CONTROLLORE_PATH},
        },
        "note_count_by_category": dict(sorted(note_count_by_category.items())),
        "note_count_total": len(provenance),
        "query_count_by_split_category_difficulty": {
            split: {cat: dict(sorted(diffs.items())) for cat, diffs in sorted(cats.items())}
            for split, cats in query_counts.items()
        },
    }

    print(json.dumps(manifest, indent=2, sort_keys=False))


if __name__ == "__main__":
    main()
