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
        self.row = {"id": "smoke", "question": "QUESTION-CANARY", "answer": "ANSWER-CANARY"}
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


if __name__ == "__main__":
    unittest.main()
