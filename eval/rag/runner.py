"""Runner-only label loader. Runtime crates never import or depend on this."""
import json
from pathlib import Path

def load_labels(eval_root, split, vault_root):
    if split not in {"dev", "calibration", "test"}:
        raise ValueError("unknown evaluation split")
    root, vault = Path(eval_root).resolve(), Path(vault_root).resolve()
    if root.is_relative_to(vault) or vault.is_relative_to(root):
        raise ValueError("evaluation and vault trees must be separate")
    labels = (root / split / "labels.jsonl").resolve(strict=True)
    if not labels.is_relative_to(root) or labels.is_relative_to(vault):
        raise ValueError("labels must stay inside the evaluation tree")
    rows = []
    for line in labels.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            raise ValueError("invalid evaluation row") from None
        if not isinstance(row, dict) or not all(isinstance(row.get(k), str) and row[k] for k in ("id", "question", "answer")):
            raise ValueError("invalid evaluation row")
        rows.append(row)
    return rows
