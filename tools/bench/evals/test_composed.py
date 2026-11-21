"""Composed mode: each checker judges the answers a generator gave in an earlier run.

The generator's replies come from a recorded cassette, so a new checker costs only its own requests, and a new
generator brings its answers with it. Only the checker is ever called live.
"""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

import aggregate
import config
import evals
import proxy
import publish
import report
import scoring
from test_aggregate import result, run

HERE = Path(__file__).resolve().parent


def composed_config(directory, answers="reference.json", modes='["composed"]', extra=""):
    """A config in `directory`: the reference model with recorded answers, and the careless checker."""
    d = Path(directory)
    for name in ("cases", "fixtures"):
        if not (d / name).exists():
            (d / name).symlink_to(HERE / name)
    if not (d / "reference.json").exists():
        (d / "reference.json").write_text((HERE / "cassettes" / "reference.json").read_text())
    text = (HERE / "replay.toml").read_text()
    text = text.replace('models = ["reference", "careless"]', 'models = ["reference"]').replace('deciders = ["reference", "careless"]', 'deciders = ["careless"]')
    text = text.replace('modes = ["generator", "decider", "combined"]', f"modes = {modes}")
    text = text.replace("[models.reference]\n", f'[models.reference]\nanswers = "{answers}"\n') if answers else text
    (d / "c.toml").write_text(text + extra)
    return d / "c.toml"


class Configuration(unittest.TestCase):
    def test_composed_needs_recorded_answers_and_a_checker(self):
        with tempfile.TemporaryDirectory() as d:
            c = config.load(composed_config(d))
            self.assertEqual(c["models"]["reference"]["answers"], str((Path(d) / "reference.json").resolve()), "relative to the config file")
            with self.assertRaises(ValueError) as e:
                config.load(composed_config(d, answers=None))
            self.assertIn("answers", str(e.exception))
            with self.assertRaises(ValueError) as e:
                config.load(composed_config(d, answers="missing.json"))
            self.assertIn("no such file", str(e.exception))

    def test_answers_belong_to_generators_not_to_checkers(self):
        with tempfile.TemporaryDirectory() as d:
            path = composed_config(d)
            path.write_text(path.read_text().replace("[deciders.careless]\n", '[deciders.careless]\nanswers = "reference.json"\n'))
            with self.assertRaises(ValueError):
                config.load(path)

    def test_a_composed_run_asks_only_the_checker_provider_for_a_key(self):
        with tempfile.TemporaryDirectory() as d:
            path = composed_config(d)
            path.write_text(path.read_text() + '\n[providers.other]\nendpoint = "https://api.example.com"\ntrust = "remote"\napi_key = "env:NOT_SET_ANYWHERE"\n'
                            '[models.hosted]\nprovider = "other"\nmodel = "m"\nanswers = "reference.json"\n')
            c = config.load(path)
            c["run"]["models"] = ["reference", "hosted"]
            self.assertEqual(config.credentials(c, {}), {"replay": None}, "the generator is never called, so its key is not needed")

    def test_the_matrix_has_one_profile_per_recorded_model_and_checker(self):
        with tempfile.TemporaryDirectory() as d:
            c = config.load(composed_config(d))
            cases = scoring.load_cases(c["run"]["cases"], c["run"]["fixtures"])
            profiles = list(evals.matrix(c, cases))
            self.assertEqual([p[0] for p in profiles], ["composed:reference+careless"])
            self.assertEqual(len(profiles[0][3]), 45, "the generator cases")
            # a model without answers is not part of it
            c["models"]["live"] = {"provider": "replay", "model": "x", "plugin": "openai-compatible", "settings": {}, "pricing": {}}
            c["run"]["models"] = ["reference", "live"]
            self.assertEqual([p[0] for p in evals.matrix(c, cases)], ["composed:reference+careless"])


class Running(unittest.TestCase):
    def evaluate(self, directory, warmup=False, repeats=1, answers=None):
        c = config.load(composed_config(directory))
        c["run"].update({"warmup": warmup, "repeats": repeats, "retries": 0, "reuse_container": False})
        if answers is not None:
            Path(c["models"]["reference"]["answers"]).write_text(json.dumps(answers))
        cases = [x for x in scoring.load_cases(c["run"]["cases"], c["run"]["fixtures"]) if x["id"] in ("gen-disk-usage", "gen-tarball")]
        res = {"schema": 1, "mode": "live", "suite_hash": scoring.fingerprint(cases, c["run"]["fixtures"]), "runs": []}
        seen = []

        def broker(*args, **kwargs):
            seen.append({"routes": sorted(args[1]), "replay": dict(args[4]),
                         "replay_only": kwargs.get("replay_only")})
            b = MagicMock()
            b.calls = [{"role": "generator", "replay": True, "response": evals.cassette_reply("x"), "cost_usd": 0},
                       {"role": "checker", "response": evals.cassette_reply('{"match":0.9,"risk":"read_only"}'), "cost_usd": 0, "wall_ms": 42}]
            m = MagicMock()
            m.__enter__.return_value = b
            return m
        with patch("proxy.Broker", side_effect=broker), patch("sandbox.run") as invoke, patch("report.write"), patch("evals.write_recordings"):
            invoke.return_value = {"code": 0, "stdout": "df -h", "stderr": "", "facts": {"parses": True, "risk": "low"}}
            evals.evaluate(c, cases, "sha256:image", {}, None, False, res, Path(directory) / "out", False)
        return res, seen

    def test_the_generator_is_replayed_from_the_cassette_and_the_checker_is_live(self):
        with tempfile.TemporaryDirectory() as d:
            res, seen = self.evaluate(d)
            ref = json.loads((HERE / "cassettes/reference.json").read_text())["cases"]
            self.assertEqual(len(seen), 2)
            for call, case in zip(seen, ("gen-disk-usage", "gen-tarball")):
                self.assertEqual(set(call["replay"]), {"generator"}, "only the generator is replayed")
                self.assertEqual(call["replay"]["generator"], evals.cassette_reply(ref[case][0][0]))
                self.assertFalse(call["replay_only"], "the checker may be called")
            self.assertEqual([r["profile"] for r in res["runs"]], ["composed:reference+careless"] * 2)

    def test_the_warm_up_uses_the_first_case_and_each_repeat_its_own_recorded_answer(self):
        with tempfile.TemporaryDirectory() as d:
            two = {"cases": {"gen-disk-usage": [["```shellmate-command\ndf -h\n```"], ["```shellmate-command\ndu -sh .\n```"]],
                             "gen-tarball": [["```shellmate-command\ntar -czf a.tgz .\n```"]]}}
            res, seen = self.evaluate(d, warmup=True, repeats=2, answers=two)
            replies = [json.loads(s["replay"]["generator"])["choices"][0]["message"]["content"] for s in seen]
            self.assertIn("df -h", replies[0], "the warm-up is the first case's first answer")
            self.assertIn("df -h", replies[1])
            self.assertIn("du -sh", replies[2], "repeat 2 is the second recorded answer")
            self.assertEqual(len(replies), 5, "warm-up, then two cases twice")

    def test_a_case_without_a_recorded_answer_is_an_error_and_asks_nobody(self):
        with tempfile.TemporaryDirectory() as d:
            res, seen = self.evaluate(d, answers={"cases": {"gen-disk-usage": [["df -h"]]}})
            rows = {r["case_id"]: r for r in res["runs"]}
            self.assertIn("missing from the recorded answers", rows["gen-tarball"]["error"])
            self.assertEqual(len(seen), 1, "nothing was sent for it")

    def test_the_timings_of_a_composed_row_are_not_kept_and_its_recording_skips_the_replayed_answers(self):
        with tempfile.TemporaryDirectory() as d:
            res, _ = self.evaluate(d)
            self.assertTrue(all(not r["timings"] for r in res["runs"]), "the generator was instant, so its time says nothing")
        rows = [{"profile": "composed:g+d", "case_id": "c", "repeat": 0, "generator_name": "g", "checker_name": "d",
                 "calls": [{"role": "generator", "replay": True, "response": evals.cassette_reply("ls")},
                           {"role": "checker", "response": evals.cassette_reply('{"match":1,"risk":"read_only"}')}]}]
        with tempfile.TemporaryDirectory() as out:
            evals.write_recordings({"runs": rows}, out)
            written = {p.parent.name for p in Path(out).rglob("*.json")}
            self.assertEqual(written, {"composed-g+d"})
            self.assertEqual(set(json.loads(next(Path(out).rglob("*.json")).read_text())["cases"]), set(), "no generator answers recorded again")


class Publishing(unittest.TestCase):
    def evaluation(self, profile, answers="a" * 64, name="one"):
        r = result([run(profile, "c", True)], source=name)
        r["evaluation_id"] = name
        r["answers"] = {"m": answers}
        r["environment"] = {"binary_version": "v", "binary_sha256": "b" * 64, "packages": "x\n"}
        return r

    def test_a_pair_judged_on_recorded_answers_is_the_same_pair_by_another_method(self):
        recorded = publish.build_entry(self.evaluation("composed:m+d"), "composed:m+d", [run("composed:m+d", "c", True)])
        live = publish.build_entry(self.evaluation("combined:m+d"), "combined:m+d", [run("combined:m+d", "c", True)])
        self.assertEqual((recorded["kind"], live["kind"]), ("pair", "pair"))
        self.assertEqual(publish.path_of("s", recorded), publish.path_of("s", live), "one pair, one file")
        self.assertNotEqual(recorded["fingerprint"], live["fingerprint"], "so switching method replaces and never pools")
        self.assertEqual(recorded["metadata"]["answers_from"], {"model": "m", "sha256": "a" * 64})
        self.assertNotIn("answers_from", live["metadata"])

    def test_other_recorded_answers_are_another_experiment(self):
        one = publish.build_entry(self.evaluation("composed:m+d", "a" * 64), "composed:m+d", [run("composed:m+d", "c", True)])
        same = publish.build_entry(self.evaluation("composed:m+d", "a" * 64, "two"), "composed:m+d", [run("composed:m+d", "c", True)])
        other = publish.build_entry(self.evaluation("composed:m+d", "c" * 64, "three"), "composed:m+d", [run("composed:m+d", "c", True)])
        self.assertEqual(one["fingerprint"], same["fingerprint"])
        self.assertNotEqual(one["fingerprint"], other["fingerprint"])

    def test_a_composed_pair_costs_only_its_checker_and_times_only_its_checker_call(self):
        import scorecard
        raw = run("composed:m+d", "c", True, command="ls", decision={"match_probability": 0.9, "risk": "read_only"},
                  calls=[{"role": "generator", "replay": True, "wall_ms": 0.2, "cost_usd": None},
                         {"role": "checker", "wall_ms": 300, "cost_usd": 0.002, "usage": {"input_tokens": 80, "output_tokens": 5}}])
        for rows in ([raw], [publish.compact(raw)]):  # the same from a raw run and from what an entry keeps
            card = scorecard.scorecard("pair", rows)
            self.assertEqual((card["cost_usd"], card["checker_p50_ms"]), (0.002, 300), rows[0]["calls"])

    def test_composed_pairs_are_listed_with_the_other_pairs(self):
        r = self.evaluation("composed:m+d")
        profiles = aggregate.merge([r])
        self.assertEqual([k[0] for k in profiles], ["combined"])
        self.assertEqual(publish.PROFILE["pair"], "combined")
        self.assertEqual(report.groups(r).keys(), {"composed:m+d"})


if __name__ == "__main__":
    unittest.main()
