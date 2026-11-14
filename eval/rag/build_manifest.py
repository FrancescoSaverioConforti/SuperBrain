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


def main():
    provenance = json.loads((ROOT / "corpus-provenance.json").read_text(encoding="utf-8"))
    notes_dir = ROOT / "corpus" / "notes"
    cross_check_provenance_vs_corpus(provenance, notes_dir)

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
