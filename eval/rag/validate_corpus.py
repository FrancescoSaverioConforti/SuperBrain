"""Cross-corpus label consistency validator (no PyYAML, no Rust crate
dependency). Minimal hand-written frontmatter reader: sufficient for
this script's one need (a note's `id` and body), not a replacement for
`domain::parse_frontmatter`.
"""
import re
from pathlib import Path

from runner import load_labels

SPLITS = ("dev", "calibration", "test")


def _parse_note(path):
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    delimiters = [i for i, line in enumerate(lines) if line.strip() == "---"]
    if len(delimiters) < 2:
        return None, ""
    front = lines[delimiters[0] + 1 : delimiters[1]]
    body = "\n".join(lines[delimiters[1] + 1 :])
    note_id = None
    for line in front:
        if line.startswith("id:"):
            note_id = line.split(":", 1)[1].strip()
            break
    return note_id, body


def _words(text):
    return re.findall(r"[a-z0-9_]+", text.lower())


def _four_grams(tokens):
    return {tuple(tokens[i : i + 4]) for i in range(len(tokens) - 3)}


def validate_corpus(corpus_dir, eval_root):
    corpus_dir = Path(corpus_dir)
    eval_root = Path(eval_root)
    notes_dir = corpus_dir / "notes"
    # load_labels requires a vault_root genuinely outside eval_root; this
    # script never reads or writes it, so any sibling-level placeholder works.
    no_vault = eval_root.resolve().parent / "_validate_corpus_no_vault_placeholder"

    note_bodies = {}
    for md_file in sorted(notes_dir.glob("*.md")):
        note_id, body = _parse_note(md_file)
        if note_id:
            note_bodies[note_id] = body

    problems = []

    for split in SPLITS:
        try:
            rows = load_labels(eval_root, split, no_vault)
        except ValueError as exc:
            problems.append(f"{split}: labels failed to load ({exc})")
            continue

        for row in rows:
            referenced_ids = set(row.get("relevant_note_ids", []))
            referenced_ids |= set(row.get("graded_relevance", {}).keys())
            referenced_ids |= {
                span.get("note_id") for span in row.get("citation_spans", [])
            }
            for note_id in referenced_ids:
                if note_id not in note_bodies:
                    problems.append(
                        f"{split}/{row['id']}: unknown note id {note_id!r} "
                        "not present in corpus"
                    )

            if row.get("abstain_expected"):
                continue

            question_grams = _four_grams(_words(row["question"]))
            for note_id in row.get("relevant_note_ids", []):
                body = note_bodies.get(note_id)
                if body is None:
                    continue
                shared = question_grams & _four_grams(_words(body))
                if shared:
                    problems.append(
                        f"{split}/{row['id']}: question shares a verbatim substring "
                        f"with target note {note_id!r}: {sorted(shared)}"
                    )

    return problems
