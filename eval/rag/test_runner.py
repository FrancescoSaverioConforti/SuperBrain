import json
from pathlib import Path
import tempfile
import unittest

from runner import load_labels


class LabelIsolationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="superbrain-rag-runner-")
        self.base = Path(self.tmp.name)
        self.eval_root = self.base / "eval"
        self.vault = self.base / "vault"
        self.vault.mkdir()
        (self.eval_root / "test").mkdir(parents=True)
        self.row = {
            "id": "smoke", "question": "QUESTION-CANARY", "answer": "ANSWER-CANARY",
            "category": "notes-decisions", "difficulty": "easy",
            "abstain_expected": False, "relevant_note_ids": ["smoke-note"],
            "graded_relevance": {"smoke-note": 2},
            "citation_spans": [{"note_id": "smoke-note", "excerpt_substring": "SMOKE-EXCERPT"}],
        }
        (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(self.row) + "\n", encoding="utf-8")

    def tearDown(self):
        self.tmp.cleanup()

    def test_labels_loaded_only_by_runner_from_separate_tree(self):
        self.assertEqual(load_labels(self.eval_root, "test", self.vault), [self.row])

    def test_traversal_and_unknown_split_rejected(self):
        for split in ["../test", "test/../test", "other", ""]:
            with self.subTest(split=split), self.assertRaises(ValueError):
                load_labels(self.eval_root, split, self.vault)

    def test_eval_tree_nested_in_vault_rejected(self):
        with self.assertRaises(ValueError):
            load_labels(self.eval_root, "test", self.base)

    def test_vault_tree_nested_in_eval_rejected(self):
        with self.assertRaises(ValueError):
            load_labels(self.eval_root, "test", self.eval_root / "vault")

    def test_bad_rows_are_errors_and_not_silent_empty_sets(self):
        (self.eval_root / "test" / "labels.jsonl").write_text('{"question": "missing id and answer"}\n', encoding="utf-8")
        with self.assertRaises(ValueError):
            load_labels(self.eval_root, "test", self.vault)

    def test_rejects_row_missing_new_required_fields(self):
        row = {"id": "q1", "question": "...", "answer": "...",
               "abstain_expected": False}
        (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            load_labels(self.eval_root, "test", self.vault)

    def test_rejects_abstain_row_with_nonempty_relevant_note_ids(self):
        row = {"id": "q1", "question": "...", "answer": "...", "category": "temporal",
               "difficulty": "easy", "abstain_expected": True,
               "relevant_note_ids": ["some-note"], "graded_relevance": {"some-note": 2},
               "citation_spans": []}
        (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            load_labels(self.eval_root, "test", self.vault)

    def test_rejects_abstain_row_missing_category_or_difficulty(self):
        row = {"id": "q1", "question": "...", "answer": "...", "abstain_expected": True,
               "relevant_note_ids": [], "graded_relevance": {}, "citation_spans": []}
        (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            load_labels(self.eval_root, "test", self.vault)

    def test_rejects_graded_relevance_keys_not_matching_relevant_note_ids(self):
        row = {"id": "q1", "question": "...", "answer": "...", "category": "temporal",
               "difficulty": "easy", "abstain_expected": False,
               "relevant_note_ids": ["note-a"], "graded_relevance": {"note-b": 2},
               "citation_spans": []}
        (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            load_labels(self.eval_root, "test", self.vault)

    def test_accepts_well_formed_abstain_row(self):
        row = {"id": "q1", "question": "...", "answer": "", "category": "temporal",
               "difficulty": "easy", "abstain_expected": True,
               "relevant_note_ids": [], "graded_relevance": {}, "citation_spans": []}
        (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
        self.assertEqual(load_labels(self.eval_root, "test", self.vault), [row])

    def test_accepts_well_formed_answerable_row(self):
        row = {"id": "q1", "question": "...", "answer": "a short answer",
               "category": "identifier-code", "difficulty": "medium",
               "abstain_expected": False, "relevant_note_ids": ["note-a"],
               "graded_relevance": {"note-a": 2},
               "citation_spans": [{"note_id": "note-a", "excerpt_substring": "the real excerpt"}]}
        (self.eval_root / "test" / "labels.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
        self.assertEqual(load_labels(self.eval_root, "test", self.vault), [row])


if __name__ == "__main__":
    unittest.main()
