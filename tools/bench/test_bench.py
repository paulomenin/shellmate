"""Tests for the benchmark tooling itself: `python3 -m unittest discover -s tools/bench`.

The pure parts (statistics, reports) always run. The tests that start the real binary use
$SHELLMATE_BIN, or target/release|debug/shellmate, and are skipped when none is built."""
import json
import os
import subprocess
import sys
import tempfile
import unittest
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import lib  # noqa: E402
import report  # noqa: E402


def binary():
    for c in (os.environ.get("SHELLMATE_BIN"), lib.ROOT / "target/release/shellmate", lib.ROOT / "target/debug/shellmate"):
        if c and Path(c).is_file():
            return Path(c)
    return None


BIN = binary()


class Stats(unittest.TestCase):
    def test_percentiles_interpolate(self):
        s = lib.stats([1, 2, 3, 4, 100])
        self.assertEqual((s["n"], s["min"], s["p50"], s["max"]), (5, 1, 3, 100))
        self.assertAlmostEqual(s["p95"], 80.8)
        self.assertEqual(lib.stats([]), {"n": 0})
        self.assertEqual(lib.stats([7])["p95"], 7)

    def test_noise_is_robust_to_one_outlier(self):
        quiet = lib.stats([10, 10.1, 9.9, 10, 10.2, 500])
        self.assertLess(quiet["noise"], 0.05)  # one outlier does not make the run look noisy
        self.assertGreater(lib.stats([5, 15, 8, 20, 6, 18])["noise"], 0.2)


def perf_result(p50, noise=0.02, name="dry-run"):
    st = {"n": 50, "min": p50, "p50": p50, "p90": p50, "p95": p50, "max": p50, "mean": p50, "stdev": 0, "mad": 0, "noise": noise}
    return {"label": "x", "runs": 50, "warmup": 5, "host": {}, "build": {},
            "scenarios": [{"name": name, "desc": "d", "wall_ms": st, "rss_mib": {"n": 0}, "stages_ms": {},
                           "failures": 0, "failure_samples": []}]}


class PerfCompare(unittest.TestCase):
    def test_only_big_and_meaningful_changes_count(self):
        base = perf_result(10.0)
        self.assertTrue(report.compare_perf(perf_result(13.0), base, 15, 1)["dry-run"]["regressed"])
        self.assertFalse(report.compare_perf(perf_result(11.0), base, 15, 1)["dry-run"]["regressed"])
        # +50% but only 0.5 ms: below the absolute floor
        self.assertFalse(report.compare_perf(perf_result(1.5), perf_result(1.0), 15, 1)["dry-run"]["regressed"])
        self.assertTrue(report.compare_perf(perf_result(7.0), base, 15, 1)["dry-run"]["improved"])

    def test_verdict_fails_on_regression_and_budget_but_only_warns_when_noisy(self):
        base = perf_result(10.0)
        _, v = report.perf_report(perf_result(13.0), base)
        self.assertFalse(v["ok"])
        _, v = report.perf_report(perf_result(13.0, noise=0.4), base)
        self.assertTrue(v["ok"] and v["warnings"])
        _, v = report.perf_report(perf_result(50.0), None, budgets={"p50_ms": {"dry-run": 30}})
        self.assertFalse(v["ok"])
        _, v = report.perf_report(perf_result(10.0), base, budgets={"p50_ms": {"dry-run": 30}})
        self.assertTrue(v["ok"])


class Servers(unittest.TestCase):
    def post(self, url, stream):
        req = urllib.request.Request(url + "/chat/completions", method="POST",
                                     data=json.dumps({"stream": stream, "messages": []}).encode())
        with urllib.request.urlopen(req, timeout=5) as r:
            return r.read().decode()

    def test_fake_llm_serves_both_modes(self):
        with lib.FakeLLM(lambda b: "hello world", piece_chars=3) as f:
            self.assertIn("hello world", json.loads(self.post(f.url, False))["choices"][0]["message"]["content"])
            streamed = self.post(f.url, True)
            self.assertIn("data: [DONE]", streamed)
            self.assertEqual(len(f.requests), 2)


@unittest.skipUnless(BIN, "build shellmate first (cargo build --release)")
class WithTheBinary(unittest.TestCase):
    def test_perf_quick_run_writes_results_and_a_report(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "p.json"
            r = subprocess.run([sys.executable, "-I", str(HERE / "perf.py"), "--bin", str(BIN), "--no-build", "--quick",
                                "--only", "version,dry-run,invoke-ask", "--no-budgets", "--out", str(out)],
                               capture_output=True, text=True, timeout=300)
            self.assertEqual(r.returncode, 0, r.stderr[-2000:])
            res = json.loads(out.read_text())
            names = {s["name"] for s in res["scenarios"]}
            self.assertEqual(names, {"floor", "version", "dry-run", "invoke-ask"})
            ask = next(s for s in res["scenarios"] if s["name"] == "invoke-ask")
            self.assertEqual(ask["failures"], 0)
            self.assertEqual(ask["wall_ms"]["n"], 20)
            self.assertIn("llm_ms", ask["stages_ms"])
            self.assertIn("## Whole process, wall time", r.stdout)
            # comparing a run with itself is never a regression
            r2 = subprocess.run([sys.executable, "-I", str(HERE / "report.py"), "perf", str(out), "--baseline", str(out)],
                                capture_output=True, text=True, timeout=60)
            self.assertEqual(r2.returncode, 0, r2.stderr)


if __name__ == "__main__":
    unittest.main()
