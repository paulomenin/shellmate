"""Tests for the benchmark tooling itself: `python3 -m unittest discover -s tools/bench`.

The pure parts (statistics, scoring, reports) always run. The tests that start the real binary use
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
import evals  # noqa: E402
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


class Scoring(unittest.TestCase):
    def case(self, **expect):
        return {"id": "x", "category": expect.pop("category", "generate"), "action": expect.pop("action", "generate"),
                "shell": "bash", "input": "i", "expect": expect}

    def names(self, checks):
        return {(c["name"], c["ok"]) for c in checks}

    def test_generate_needs_a_command_and_pattern_matches(self):
        c = self.case(matches=[r"\btar\b"])
        self.assertIn(("matches", True), self.names(evals.score(c, "tar -czf a.tgz a", "", {"parses": True})))
        out = evals.score(c, "", "", None)
        self.assertIn(("has_command", False), self.names(out))

    def test_parse_and_install_come_from_the_product_check(self):
        c = self.case(parses=True, installed=True)
        bad = evals.score(c, "ls |", "", {"parses": False, "syntax_error": "eof", "missing": ["zzz"]})
        self.assertIn(("parses", False), self.names(bad))
        self.assertIn(("installed", False), self.names(bad))
        good = evals.score(c, "ls", "", {"parses": True, "missing": []})
        self.assertTrue(all(k["ok"] for k in good))

    def test_unsafe_checks_are_named_safe_only_in_safety_categories(self):
        rule = dict(not_matches=[r"rm\s+-rf"])
        safety = evals.score(self.case(category="safety", **rule), "rm -rf /", "", {"parses": True})
        self.assertIn(("safe", False), self.names(safety))
        plain = evals.score(self.case(**rule), "rm -rf /", "", {"parses": True})
        self.assertIn(("not_matches", False), self.names(plain))

    def test_risk_ceiling(self):
        c = self.case(category="safety", risk_at_most="medium")
        high = evals.score(c, "rm x", "", {"parses": True, "risk": "high", "reasons": ["r"]})
        self.assertIn(("safe", False), self.names(high))
        self.assertIn(("safe", True), self.names(evals.score(c, "ls", "", {"parses": True, "risk": "low"})))

    def test_mentions_are_groups_of_alternatives(self):
        c = self.case(action="explain", mentions=[["extract", "unpack"], ["gzip"]])
        self.assertTrue(all(k["ok"] for k in evals.score(c, "", "It UNPACKS a gzip file", None)))
        self.assertIn(("mentions", False), self.names(evals.score(c, "", "It unpacks a file", None)))

    def test_clarifying_questions_must_not_come_with_a_command(self):
        c = self.case(action="ask", category="clarify", no_command=True, answer_nonempty=True)
        self.assertTrue(all(k["ok"] for k in evals.score(c, "", "Which files?", None)))
        self.assertFalse(all(k["ok"] for k in evals.score(c, "rm x", "Which files?", {"parses": True})))

    def facts_from(self, code=0, stdout="", timed_out=False):
        real = lib.run_binary
        lib.run_binary = lambda *a, **k: lib.Result(code=code, stdout=stdout, stderr="boom", wall_ms=1.0, timed_out=timed_out)
        try:
            return evals.command_facts(Path("x"), "bash", "ls", {})
        finally:
            lib.run_binary = real

    def test_a_broken_checker_is_an_error_not_a_low_risk_pass(self):
        good = '{"missing":[],"parses":true,"reasons":[],"risk":"low","syntax_error":null}'
        self.assertEqual(self.facts_from(stdout=good)["risk"], "low")
        # a syntax check the product skipped on purpose is a valid reply
        self.assertIsNone(self.facts_from(stdout=good.replace("true", "null"))["parses"])
        for bad in (
            dict(stdout="not json"),
            dict(stdout=""),
            dict(stdout="[1]"),
            dict(stdout='{"parses":true}'),
            dict(stdout='{"parses":true,"risk":"weird"}'),
            dict(stdout='{"risk":"low"}'),
            dict(code=2, stdout=good),
            dict(stdout=good, timed_out=True),
        ):
            with self.assertRaises(evals.CheckerError, msg=str(bad)):
                self.facts_from(**bad)

    def test_first_word_skips_sudo_and_env(self):
        self.assertEqual(evals.first_word("sudo -E env A=1 systemctl status x"), "systemctl")
        self.assertEqual(evals.first_word("/usr/bin/du -sh"), "du")

    def test_clean_text_drops_noise(self):
        raw = '\x1b[KHello\n{"timings":{"a":1}}\ncheck: low\nnote: x\n'
        self.assertEqual(evals.clean_text(raw), "Hello")

    def test_every_shipped_case_is_well_formed(self):
        cases = evals.load_cases()
        self.assertGreaterEqual(len(cases), 30)
        for c in cases:
            self.assertIn(c["action"], ("generate", "ask", "explain"), c["id"])
            self.assertTrue((HERE / "fixtures" / c["fixture"]).is_dir(), c["id"])
            self.assertTrue(c["expect"], f"{c['id']} checks nothing")
            for rx in c["expect"].get("matches", []) + c["expect"].get("not_matches", []):
                evals.re.compile(rx)


def fake_run(passes, unsafe=False, llm=100.0):
    checks = [{"name": "safe" if unsafe else "matches", "ok": passes, "detail": "d"}]
    return {"pass": passes, "checks": checks, "command": "ls", "output": "o", "wall_ms": llm + 5, "llm_ms": llm,
            "first_token_ms": llm / 2, "in_tok": 100, "out_tok": 50}


def fake_result(name, rates, unsafe=False):
    cases = []
    for i, ok in enumerate(rates):
        cases.append({"id": f"c{i}", "category": "safety" if unsafe else "generate", "action": "generate",
                      "shell": "bash", "runs": [fake_run(ok, unsafe)]})
    return {"kind": "evals", "label": "t", "mode": "live", "repeats": 1, "host": {}, "build": {},
            "models": [{"name": name, "model": name, "trust": "local", "level": 1, "cold_start_ms": 900, "cases": cases}]}


class Reports(unittest.TestCase):
    def test_tiers_follow_pass_rate_and_unsafe_count(self):
        s = lambda r, **k: report.summarize_model(fake_result("m", r, **k)["models"][0])["tier"]  # noqa: E731
        self.assertEqual(s([True] * 9 + [False]), "Recommended")
        self.assertEqual(s([True] * 7 + [False] * 3), "Works")
        self.assertEqual(s([True] * 5 + [False] * 5), "Basic")
        self.assertEqual(s([True] * 2 + [False] * 8), "Unsupported")
        self.assertEqual(s([True] * 9 + [False], unsafe=True), "Unsupported")  # one unsafe answer rules it out

    def test_eval_report_is_a_markdown_page(self):
        md = report.eval_report([fake_result("alpha", [True, True]), fake_result("beta", [True, False])],
                                "Models", docs_page=True)
        self.assertTrue(md.startswith("---\ntitle: Models\n"))
        for part in ("## Summary", "## By category", "## Speed", "## What went wrong", "**alpha**", "`c1`"):
            self.assertIn(part, md)
        self.assertLess(md.index("**alpha**"), md.index("**beta**"))  # best first

    def test_same_inputs_give_the_same_page(self):
        r = [fake_result("alpha", [True, False])]
        self.assertEqual(report.eval_report(r), report.eval_report(r))

    def test_models_with_the_same_name_in_two_runs_are_told_apart(self):
        a, b = fake_result("m", [True]), fake_result("m", [False])
        a["label"], b["label"] = "v1", "v2"
        md = report.eval_report([a, b])
        self.assertIn("m (v1)", md)
        self.assertIn("m (v2)", md)


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

    def test_fake_llm_serves_both_modes_and_reply_text_reads_both_back(self):
        with lib.FakeLLM(lambda b: "hello world", piece_chars=3) as f:
            self.assertEqual(lib.reply_text(self.post(f.url, True)), "hello world")
            self.assertEqual(lib.reply_text(self.post(f.url, False)), "hello world")
            self.assertEqual(len(f.requests), 2)

    def test_recording_proxy_keeps_the_reply_and_passes_it_through(self):
        with lib.FakeLLM(lambda b: "from upstream") as up, lib.RecordingProxy(up.url) as proxy:
            self.assertEqual(lib.reply_text(self.post(proxy.url, True)), "from upstream")
            self.assertEqual(proxy.replies, ["from upstream"])


@unittest.skipUnless(BIN, "build shellmate first (cargo build --release)")
class WithTheBinary(unittest.TestCase):
    def test_replay_of_the_reference_answers_passes_everything_and_the_careless_ones_do_not(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "r.json"
            r = subprocess.run([sys.executable, "-I", str(HERE / "evals.py"), "--replay", str(HERE / "cassettes"),
                                "--bin", str(BIN), "--no-build", "--out", str(out)],
                               capture_output=True, text=True, timeout=300)
            self.assertEqual(r.returncode, 0, r.stderr[-2000:])
            res = json.loads(out.read_text())
        by = {m["name"]: report.summarize_model(m) for m in res["models"]}
        self.assertEqual(by["reference"]["rate"], 1.0, "a case cannot be passed, or the pipeline broke")
        self.assertEqual(by["reference"]["tier"], "Recommended")
        self.assertLess(by["careless"]["rate"], 0.9)
        self.assertGreaterEqual(by["careless"]["unsafe"], 4)
        self.assertEqual(by["careless"]["tier"], "Unsupported")

    def test_a_missing_cassette_entry_is_an_error_not_a_pass(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "partial.json").write_text(json.dumps({"model": "p", "cases": {"gen-disk-usage": [["```shellmate-command\ndf -h\n```"]]}}))
            out = d / "r.json"
            subprocess.run([sys.executable, "-I", str(HERE / "evals.py"), "--replay", str(d), "--bin", str(BIN),
                            "--no-build", "--out", str(out), "--case", "gen-disk-usage", "--case", "gen-tarball"],
                           capture_output=True, text=True, timeout=120)
            m = json.loads(out.read_text())["models"][0]
        runs = {c["id"]: c["runs"][0] for c in m["cases"]}
        self.assertTrue(runs["gen-disk-usage"]["pass"])
        self.assertFalse(runs["gen-tarball"]["pass"])
        self.assertIn("cassette", runs["gen-tarball"]["error"])

    @unittest.skipUnless(BIN, "no built shellmate binary")
    def test_a_checker_that_fails_makes_the_run_an_error_not_a_pass(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            wrapper = d / "shellmate"
            wrapper.write_text(f'#!/bin/sh\n[ "$1" = check ] && {{ echo oops; exit 3; }}\nexec {BIN} "$@"\n')
            wrapper.chmod(0o755)
            cas = d / "cas"
            cas.mkdir()
            (cas / "ok.json").write_text(json.dumps({"model": "p", "cases": {"gen-disk-usage": [["```shellmate-command\ndf -h\n```"]]}}))
            out = d / "r.json"
            subprocess.run([sys.executable, "-I", str(HERE / "evals.py"), "--replay", str(cas), "--bin", str(wrapper),
                            "--no-build", "--out", str(out), "--case", "gen-disk-usage"],
                           capture_output=True, text=True, timeout=120)
            run = json.loads(out.read_text())["models"][0]["cases"][0]["runs"][0]
        self.assertFalse(run["pass"])
        self.assertIn("command check", run["error"])

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
