import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import isolate


def row(profile, error=None, warmup=False):
    return {"id": f"{profile}-{error}-{warmup}", "profile": profile, "case_id": "c", "repeat": 0, "category": "ask", "action": "ask", "shell": "bash", "input": "q", "passed": error is None, "checks": [],
            "warmup": warmup, **({"error": error} if error else {})}


def make(root, name="r1"):
    d = Path(root) / name
    (d / "cassettes").mkdir(parents=True)
    runs = [row("generator:haiku"), row("generator:xai", "402"), row("generator:xai"), row("generator:xai", warmup=True),
            row("generator:haiku+judge"), row("decider:judge"), row("decider:other", "boom")]
    (d / "results.json").write_text(json.dumps({"schema": 1, "suite_hash": "s", "date": "2026-11-21", "mode": "live",
                                                "evaluation_id": "e", "partial": False, "configuration": {}, "runs": runs}))
    for p in ["generator-haiku", "generator-xai", "generator-haiku+judge"]:
        (d / "cassettes" / p).mkdir()
        (d / "cassettes" / p / "x.json").write_text("{}")
    return d


class Isolate(unittest.TestCase):
    def setUp(self):
        quiet = contextlib.redirect_stdout(io.StringIO())
        quiet.__enter__()
        self.addCleanup(quiet.__exit__, None, None, None)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.run_dir = make(self.tmp.name)
        self.result = isolate.load(self.run_dir)

    def test_a_model_name_selects_its_composed_profiles_too(self):
        self.assertEqual(isolate.select(self.result, ["haiku"]), ["generator:haiku", "generator:haiku+judge"])
        self.assertEqual(isolate.select(self.result, ["generator:haiku"]), ["generator:haiku"])
        self.assertEqual(isolate.select(self.result, ["judge"]), ["generator:haiku+judge", "decider:judge"])

    def test_kinds_and_errors_combine(self):
        self.assertEqual(isolate.select(self.result, kind="composed"), ["generator:haiku+judge"])
        self.assertEqual(isolate.select(self.result, kind="decider"), ["decider:judge", "decider:other"])
        self.assertEqual(isolate.select(self.result, errors=True), ["generator:xai", "decider:other"])
        self.assertEqual(isolate.select(self.result, kind="decider", errors=True), ["decider:other"])

    def test_a_warmup_error_does_not_count(self):
        result = {"runs": [row("generator:a", "x", warmup=True), row("generator:a")]}
        self.assertEqual(isolate.select(result, errors=True), [])

    def test_no_selection_is_refused(self):
        with self.assertRaises(ValueError):
            isolate.select(self.result)

    def test_drop_removes_runs_and_cassettes_and_rewrites_the_summary(self):
        self.assertEqual(isolate.main(["drop", str(self.run_dir), "--errors"]), 0)
        left = {r["profile"] for r in isolate.load(self.run_dir)["runs"]}
        self.assertEqual(left, {"generator:haiku", "generator:haiku+judge", "decider:judge"})
        self.assertFalse((self.run_dir / "cassettes" / "generator-xai").exists())
        self.assertTrue((self.run_dir / "cassettes" / "generator-haiku").exists())
        self.assertNotIn("generator:xai", (self.run_dir / "summary.md").read_text())
        self.assertIn("review.html", {p.name for p in self.run_dir.iterdir()})

    def test_move_keeps_the_profiles_in_a_run_of_their_own(self):
        into = Path(self.tmp.name) / "bad"
        self.assertEqual(isolate.main(["move", str(self.run_dir), "xai", "--into", str(into)]), 0)
        self.assertEqual({r["profile"] for r in isolate.load(into)["runs"]}, {"generator:xai"})
        self.assertEqual(len(isolate.load(into)["runs"]), 3)
        self.assertTrue((into / "cassettes" / "generator-xai" / "x.json").exists())
        self.assertNotIn("generator:xai", {r["profile"] for r in isolate.load(self.run_dir)["runs"]})
        self.assertFalse((self.run_dir / "cassettes" / "generator-xai").exists())

    def test_copy_leaves_the_source_alone(self):
        before = (self.run_dir / "results.json").read_text()
        into = Path(self.tmp.name) / "copy"
        self.assertEqual(isolate.main(["copy", str(self.run_dir), "--kind", "decider", "--into", str(into)]), 0)
        self.assertEqual((self.run_dir / "results.json").read_text(), before)
        self.assertEqual({r["profile"] for r in isolate.load(into)["runs"]}, {"decider:judge", "decider:other"})

    def test_dry_run_changes_nothing_and_unsafe_calls_are_refused(self):
        before = (self.run_dir / "results.json").read_text()
        self.assertEqual(isolate.main(["drop", str(self.run_dir), "haiku", "--dry-run"]), 0)
        self.assertEqual((self.run_dir / "results.json").read_text(), before)
        self.assertEqual(isolate.main(["drop", str(self.run_dir)]), 1)
        self.assertEqual(isolate.main(["drop", str(self.run_dir), "nobody"]), 1)
        self.assertEqual(isolate.main(["move", str(self.run_dir), "haiku"]), 1)
        self.assertEqual(isolate.main(["drop", str(self.run_dir), "haiku", "--into", "x"]), 1)
        into = Path(self.tmp.name) / "taken"
        into.mkdir()
        (into / "results.json").write_text("{}")
        self.assertEqual(isolate.main(["copy", str(self.run_dir), "haiku", "--into", str(into)]), 1)
        self.assertEqual((self.run_dir / "results.json").read_text(), before)

    def test_list_marks_errors(self):
        text = "\n".join(isolate.listing(self.tmp.name))
        self.assertIn("generator:xai", text)
        self.assertIn("composed", text)
        self.assertIn("1 errors", text)


if __name__ == "__main__":
    unittest.main()
