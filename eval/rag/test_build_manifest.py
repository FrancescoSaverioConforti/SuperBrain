from pathlib import Path
import tempfile
import unittest

from build_manifest import cross_check_code_note_body_provenance


def _write_code_note(notes_dir, note_id, repository, commit, path, symbol):
    (notes_dir / f"{note_id}.md").write_text(
        f"---\nid: {note_id}\nschema_version: 1\ntype: code\nstatus: verified\n"
        f"trust: internal\nclassification: internal\nproject: eval-b3\n"
        f"source: test\ncreated: 2026-01-01\nupdated: 2026-01-01\n"
        f"event_period: 2026-01-01\nrelations: {{}}\n---\n"
        f"repository: {repository}\ncommit: {commit}\npath: {path}\nsymbol: {symbol}\n\n"
        f"Some body text.\n",
        encoding="utf-8",
    )


class CrossCheckCodeNoteBodyProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="superbrain-build-manifest-")
        self.notes_dir = Path(self.tmp.name) / "notes"
        self.notes_dir.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def test_matching_body_and_provenance_produce_no_problems(self):
        _write_code_note(self.notes_dir, "code-a", "controllore", "abc123", "app/x.py", "foo")
        provenance = {
            "code-a": {
                "category": "identifier-code", "stratum": "code",
                "repository": "controllore", "commit": "abc123",
                "path": "app/x.py", "symbol": "foo",
            }
        }
        problems = cross_check_code_note_body_provenance(provenance, self.notes_dir)
        self.assertEqual(problems, [])

    def test_flags_commit_mismatch_between_body_and_provenance(self):
        _write_code_note(self.notes_dir, "code-a", "controllore", "abc123", "app/x.py", "foo")
        provenance = {
            "code-a": {
                "category": "identifier-code", "stratum": "code",
                "repository": "controllore", "commit": "DIFFERENT",
                "path": "app/x.py", "symbol": "foo",
            }
        }
        problems = cross_check_code_note_body_provenance(provenance, self.notes_dir)
        self.assertTrue(any("commit" in p and "code-a" in p for p in problems))

    def test_flags_path_mismatch_between_body_and_provenance(self):
        _write_code_note(self.notes_dir, "code-a", "controllore", "abc123", "app/x.py", "foo")
        provenance = {
            "code-a": {
                "category": "identifier-code", "stratum": "code",
                "repository": "controllore", "commit": "abc123",
                "path": "app/DIFFERENT.py", "symbol": "foo",
            }
        }
        problems = cross_check_code_note_body_provenance(provenance, self.notes_dir)
        self.assertTrue(any("path" in p and "code-a" in p for p in problems))

    def test_ignores_project_history_entries(self):
        provenance = {
            "decision-a": {
                "category": "notes-decisions", "stratum": "project-history",
                "source": "docs/x.md",
            }
        }
        problems = cross_check_code_note_body_provenance(provenance, self.notes_dir)
        self.assertEqual(problems, [])


if __name__ == "__main__":
    unittest.main()
