"""Task 3 Step 3 check: no query shares a 4+ consecutive-word run with its
target note's body, case-insensitive. Scratch tool, not shipped."""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NOTES_DIR = ROOT / "corpus" / "notes"
DRAFT = Path(__file__).resolve().parent / "queries-draft.jsonl"

def words(text):
    return re.findall(r"[a-z0-9_]+", text.lower())

def ngrams(tokens, n=4):
    return {tuple(tokens[i:i+n]) for i in range(len(tokens) - n + 1)}

note_bodies = {}
for f in NOTES_DIR.glob("*.md"):
    text = f.read_text(encoding="utf-8")
    parts = text.split("---\n", 2)
    body = parts[2] if len(parts) == 3 else text
    note_bodies[f.stem] = body

violations = []
with open(DRAFT, encoding="utf-8") as fh:
    for line in fh:
        line = line.strip()
        if not line:
            continue
        row = json.loads(line)
        target = row.get("target")
        if not target:
            continue
        if target not in note_bodies:
            violations.append(f"{row['id']}: target note {target} not found in corpus")
            continue
        q_grams = ngrams(words(row["question"]))
        b_grams = ngrams(words(note_bodies[target]))
        shared = q_grams & b_grams
        if shared:
            violations.append(f"{row['id']} -> {target}: shared 4-gram(s) {shared}")

if violations:
    print(f"{len(violations)} VIOLATIONS:")
    for v in violations:
        print(" ", v)
else:
    print("OK: no 4+ word verbatim overlap between any query and its target note's body.")
