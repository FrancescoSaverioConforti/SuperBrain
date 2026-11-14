import json
from pathlib import Path
import tempfile
import unittest

from validate_corpus import validate_corpus


def _write_note(notes_dir, note_id, body):
    (notes_dir / f"{note_id}.md").write_text(
        f"---\nid: {note_id}\nschema_version: 1\ntype: pattern\nstatus: verified\n"
        f"trust: internal\nclassification: internal\nproject: eval-b3\n"
        f"source: test\ncreated: 2026-01-01\nupdated: 2026-01-01\n"
        f"event_period: 2026-01-01\nrelations: {{}}\n---\n{body}\n",
        encoding="utf-8",
    )


def _write_labels(eval_root, split, rows):
    (eval_root / split).mkdir(parents=True, exist_ok=True)
    with open(eval_root / split / "labels.jsonl", "w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row) + "\n")


def _answerable_row(row_id, target, question, answer="an answer", span="an answer"):
    return {
        "id": row_id, "question": question, "answer": answer,
        "category": "patterns-knowledge", "difficulty": "easy",
        "abstain_expected": False,
        "relevant_note_ids": [target],
        "graded_relevance": {target: 2},
        "citation_spans": [{"note_id": target, "excerpt_substring": span}],
    }


class ValidateCorpusTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="superbrain-validate-corpus-")
        self.base = Path(self.tmp.name)
        self.corpus_dir = self.base / "corpus"
        self.notes_dir = self.corpus_dir / "notes"
        self.notes_dir.mkdir(parents=True)
        self.eval_root = self.base / "eval"
        self.eval_root.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def test_flags_note_id_not_present_in_corpus(self):
        _write_note(self.notes_dir, "real-note", "The real note body text.")
        row = _answerable_row("q1", "ghost-note", "A question about something.")
        for split in ("dev", "calibration", "test"):
            _write_labels(self.eval_root, split, [row] if split == "dev" else [])

        problems = validate_corpus(self.corpus_dir, self.eval_root)

        self.assertTrue(any("unknown note id" in p for p in problems))

    def test_flags_query_that_is_verbatim_substring_of_its_target_note(self):
        _write_note(self.notes_dir, "real-note", "the quick brown fox jumps over the lazy dog")
        row = _answerable_row(
            "q1", "real-note", "Does the quick brown fox jumps somewhere in this story?"
        )
        for split in ("dev", "calibration", "test"):
            _write_labels(self.eval_root, split, [row] if split == "dev" else [])

        problems = validate_corpus(self.corpus_dir, self.eval_root)

        self.assertTrue(any("verbatim substring" in p for p in problems))

    def test_clean_corpus_and_labels_produce_no_problems(self):
        _write_note(self.notes_dir, "real-note", "This note explains the retry policy in detail.")
        row = _answerable_row(
            "q1", "real-note", "What governs repeated attempts after failure?",
            answer="retry policy", span="retry policy",
        )
        for split in ("dev", "calibration", "test"):
            _write_labels(self.eval_root, split, [row] if split == "dev" else [])

        problems = validate_corpus(self.corpus_dir, self.eval_root)

        self.assertEqual(problems, [])

    def test_abstain_rows_are_never_flagged_for_substring_overlap(self):
        _write_note(self.notes_dir, "real-note", "the quick brown fox jumps over the lazy dog")
        abstain_row = {
            "id": "a1", "question": "the quick brown fox jumps somewhere else entirely",
            "answer": "", "category": "temporal", "difficulty": "easy",
            "abstain_expected": True, "relevant_note_ids": [],
            "graded_relevance": {}, "citation_spans": [],
        }
        for split in ("dev", "calibration", "test"):
            _write_labels(self.eval_root, split, [abstain_row] if split == "dev" else [])

        problems = validate_corpus(self.corpus_dir, self.eval_root)

        self.assertEqual(problems, [])


if __name__ == "__main__":
    unittest.main()
