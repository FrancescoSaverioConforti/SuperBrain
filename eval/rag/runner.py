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
        if not isinstance(row, dict) or not all(isinstance(row.get(k), str) and row[k] for k in ("id", "question")):
            raise ValueError("invalid evaluation row")
        if not isinstance(row.get("answer"), str):
            raise ValueError("invalid evaluation row")
        if not row["answer"] and row.get("abstain_expected") is not True:
            raise ValueError("invalid evaluation row")
        _validate_extended_schema(row)
        rows.append(row)
    return rows


def _validate_extended_schema(row):
    if not isinstance(row.get("category"), str) or not row["category"]:
        raise ValueError("row missing non-empty 'category'")
    if row.get("difficulty") not in ("easy", "medium", "hard"):
        raise ValueError("row 'difficulty' must be one of easy/medium/hard")
    if not isinstance(row.get("abstain_expected"), bool):
        raise ValueError("row missing boolean 'abstain_expected'")

    relevant_note_ids = row.get("relevant_note_ids")
    graded_relevance = row.get("graded_relevance")
    citation_spans = row.get("citation_spans")

    if row["abstain_expected"]:
        if relevant_note_ids != [] or graded_relevance != {} or citation_spans != []:
            raise ValueError(
                "abstain_expected row must have empty relevant_note_ids/graded_relevance/citation_spans"
            )
        return

    if not isinstance(relevant_note_ids, list) or not relevant_note_ids or not all(
        isinstance(i, str) and i for i in relevant_note_ids
    ):
        raise ValueError("row missing non-empty 'relevant_note_ids' list of str")
    if not isinstance(graded_relevance, dict) or set(graded_relevance) != set(relevant_note_ids) or not all(
        v in (0, 1, 2) for v in graded_relevance.values()
    ):
        raise ValueError("row 'graded_relevance' keys must match relevant_note_ids, values in {0,1,2}")
    if not isinstance(citation_spans, list) or not citation_spans or not all(
        isinstance(span, dict)
        and isinstance(span.get("note_id"), str) and span["note_id"]
        and isinstance(span.get("excerpt_substring"), str) and span["excerpt_substring"]
        for span in citation_spans
    ):
        raise ValueError("row 'citation_spans' must be a nonempty list of {note_id, excerpt_substring}")
