import copy
import json
import tempfile
import unittest
from pathlib import Path

import aggregate
import config
import evals
import publish
import report
import scoring
import scorecard
from test_aggregate import result, run

HERE = Path(__file__).resolve().parent


def calls(role="generator", wall=100, cost=0.0, **usage):
    return [{"role": role, "wall_ms": wall, "cost_usd": cost, "usage": {"input_tokens": 10, "output_tokens": 5, **usage}}]


def chat_runs(passed, failed, profile="generator:m"):
    rows = [run(profile, f"c{i}", True, calls=calls()) for i in range(passed)]
    rows += [run(profile, f"f{i}", False, checks=[{"name": "parses", "ok": False}, {"name": "matches", "ok": True}], calls=calls(),
                 command="x" * 500, output="y" * 500) for i in range(failed)]
    for i, r in enumerate(rows):
        r["id"] = f"{profile}-{i}-{r['case_id']}"
    return rows


def evaluation(name, runs=None, **kw):
    r = result(runs or chat_runs(9, 1), source=name, **kw)
    r["evaluation_id"] = name
    r["environment"] = {"binary_version": "shellmate 0.0.1", "binary_sha256": "a" * 64, "packages": "a\nb\n"}
    r["host"] = {"cpu": "Test CPU", "cores": 8, "memory_gib": 16.0}
    r["configuration"]["run"].update({"max_output_tokens": 4096, "price_date": "2026-11-20"})
    r["configuration"]["models"]["m"]["pricing"] = {"input_per_million": 0, "output_per_million": 0}
    return r


def publish_one(store, r, **kw):
    """Publish one in-memory result the way `add` does, without report directories."""
    out = []
    for profile, runs in report.groups(r).items():
        entry = publish.build_entry(r, profile, runs)
        path = publish.path_of(store, entry)
        merged, message = publish.merge_entry(json.loads(path.read_text()) if path.exists() else None, entry, **kw)
        publish.write(path, merged)
        out.append((path, message))
    return out


class Entries(unittest.TestCase):
    def test_identity_is_the_model_and_its_reasoning_and_the_provider_is_only_metadata(self):
        r = evaluation("a", models={"m": {"model": "gemma", "settings": {"reasoning_effort": "low", "timeout_secs": 9}}})
        entry = publish.build_entry(r, "generator:m", chat_runs(1, 0))
        self.assertEqual(entry["identity"], {"model": "gemma", "reasoning": "low"})
        self.assertEqual(entry["label"], "gemma [reasoning: low]")
        self.assertEqual(entry["kind"], "chat")
        self.assertEqual(entry["metadata"]["providers"], {"generator": "p"})
        self.assertNotIn("p", json.dumps(entry["identity"]))
        self.assertEqual(publish.file_slug(entry["identity"]), "gemma--reasoning-low")
        # the same model through another provider is the same entry, with the provider recorded
        other = evaluation("b", models={"m": {"model": "gemma", "settings": {"reasoning_effort": "low"}, "provider": "q"}})
        other["configuration"]["providers"]["q"] = {"trust": "local"}
        self.assertEqual(publish.path_of("s", publish.build_entry(other, "generator:m", chat_runs(1, 0))), publish.path_of("s", entry))

    def test_each_kind_gets_its_own_folder_and_a_pair_names_both(self):
        r = evaluation("a")
        for profile, kind in (("generator:m", "chat"), ("decider:d", "decision"), ("combined:m+d", "pair")):
            self.assertEqual(publish.build_entry(r, profile, [run(profile, "c", True)])["kind"], kind)
        pair = publish.build_entry(r, "combined:m+d", [run("combined:m+d", "c", True)])
        self.assertEqual(pair["label"], "model-one + judge-one")
        self.assertEqual(publish.file_slug(pair["identity"]), "model-one--judge-one")

    def test_a_run_keeps_what_the_pages_need_and_only_a_short_answer(self):
        row = chat_runs(0, 1)[0]
        row["checks"] = [{"name": "parses", "ok": False}, {"name": "installed", "ok": True}]
        kept = publish.compact(row)
        self.assertEqual(kept["checks"], [{"name": "parses", "ok": False}], "only the failed checks")
        self.assertEqual((len(kept["command"]), len(kept["output"])), (publish.TEXT, publish.TEXT))
        self.assertEqual(kept["calls"], [{"role": "generator", "wall_ms": 100, "cost_usd": 0.0, "input_tokens": 10, "output_tokens": 5}])
        self.assertNotIn("response", json.dumps(kept))

    def test_the_scorecard_is_the_same_from_raw_and_from_compact_rows(self):
        rows = chat_runs(17, 3)
        compacted = [publish.compact(r) for r in rows]
        for kind in ("chat", "pair"):
            a, b = scorecard.scorecard(kind, rows), scorecard.scorecard(kind, compacted)
            self.assertEqual(a, b, kind)
        self.assertEqual(scorecard.scorecard("chat", rows)["tier"], "Recommended")


class Updating(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.store = Path(self.dir.name)

    def tearDown(self):
        self.dir.cleanup()

    def load(self):
        return json.loads(next(self.store.glob("chat/*.json")).read_text())

    def test_the_same_experiment_is_pooled_and_a_run_is_never_added_twice(self):
        self.assertEqual(publish_one(self.store, evaluation("one"))[0][1], "new")
        second = evaluation("two", chat_runs(10, 0))
        for r in second["runs"]:
            r["id"] = "two-" + r["id"]
        self.assertIn("pooled (+10 runs, now 20)", publish_one(self.store, second)[0][1])
        self.assertIn("already in it", publish_one(self.store, second)[0][1], "publishing the same evaluation again changes nothing")
        entry = self.load()
        self.assertEqual((len(entry["runs"]), len(entry["evaluations"]), entry["summary"]["runs"]), (20, 2, 20))

    def test_a_timeout_does_not_make_it_a_different_experiment_but_the_settings_do(self):
        publish_one(self.store, evaluation("one"))
        quick = evaluation("two", chat_runs(2, 0))
        for r in quick["runs"]:
            r["id"] = "t-" + r["id"]
        quick["configuration"]["models"]["m"]["settings"] = {"timeout_secs": 5}
        self.assertIn("pooled", publish_one(self.store, quick)[0][1])
        changed = evaluation("three", chat_runs(3, 0))
        changed["configuration"]["models"]["m"]["settings"] = {"reasoning_effort": "low"}
        # another reasoning setting is another model: its own file, nothing replaced
        self.assertEqual(publish_one(self.store, changed)[0][1], "new")
        self.assertEqual(len(list(self.store.glob("chat/*.json"))), 2)

    def test_a_changed_experiment_replaces_the_entry_and_says_what_changed(self):
        publish_one(self.store, evaluation("one"))
        for change, text in ((lambda r: r["configuration"]["models"]["m"].update(pricing={"input_per_million": 1, "output_per_million": 1}), "model settings or prices"),
                             (lambda r: r["environment"].update(binary_sha256="b" * 64), "shellmate binary"),
                             (lambda r: r.update(suite_hash="z" * 64), "suite of cases")):
            r = evaluation("replacement", chat_runs(4, 0))
            change(r)
            message = publish_one(self.store, r)[0][1]
            self.assertTrue(message.startswith("replaced"), message)
            self.assertIn(text, message)
            self.assertEqual(len(self.load()["runs"]), 4, "the old runs are gone")
            publish_one(self.store, evaluation("one"), force_replace=True)  # back to the first experiment for the next change

    def test_replace_can_be_forced(self):
        publish_one(self.store, evaluation("one"))
        message = publish_one(self.store, evaluation("one", chat_runs(2, 0)), force_replace=True)[0][1]
        self.assertTrue(message.startswith("replaced (forced"), message)
        self.assertEqual(len(self.load()["runs"]), 2)

    def test_machines_are_collected_and_the_newest_environment_is_kept(self):
        publish_one(self.store, evaluation("one"))
        other = evaluation("two", chat_runs(1, 0))
        other["host"] = {"cpu": "Other CPU", "cores": 4, "memory_gib": 8.0}
        for r in other["runs"]:
            r["id"] = "o-" + r["id"]
        publish_one(self.store, other)
        self.assertEqual(self.load()["metadata"]["machines"], ["Other CPU, 4 threads, 8.0 GiB", "Test CPU, 8 threads, 16.0 GiB"])

    def test_the_machine_text_joins_what_was_read_and_what_the_config_says(self):
        r = evaluation("a")
        r["configuration"]["run"]["machine"] = "RTX 4070 12 GB"
        self.assertEqual(publish.host_text(r), "RTX 4070 12 GB; Test CPU, 8 threads, 16.0 GiB")
        self.assertIsNone(publish.host_text({"configuration": {"run": {}}}))


class Completeness(unittest.TestCase):
    def setUp(self):
        self.suite = publish.current_suite()
        self.cases = scoring.load_cases(HERE / "cases", HERE / "fixtures")
        self.generator_cases = [c for c in self.cases if "candidate" not in c]

    def test_only_a_profile_with_all_its_runs_is_published(self):
        r = evaluation("a", suite=self.suite)
        r["configuration"]["run"]["repeats"] = 2
        full = [run("generator:m", c["id"], True) for c in self.generator_cases for _ in range(2)]
        self.assertTrue(publish.complete(r, "generator:m", full, self.suite, self.cases))
        self.assertFalse(publish.complete(r, "generator:m", full[:-1], self.suite, self.cases), "interrupted in the middle")
        decisions = [run("decider:d", c["id"], True) for c in self.cases if "candidate" in c for _ in range(2)]
        self.assertTrue(publish.complete(r, "decider:d", decisions, self.suite, self.cases), "a decision profile has fewer cases")

    def test_on_another_suite_the_report_own_flag_decides(self):
        r = evaluation("a", suite="old" * 20)
        r["partial"] = True
        self.assertFalse(publish.complete(r, "generator:m", [run("generator:m", "c", True)], self.suite, self.cases))
        r["partial"] = False
        self.assertTrue(publish.complete(r, "generator:m", [run("generator:m", "c", True)], self.suite, self.cases))


class Pages(unittest.TestCase):
    def test_pages_are_made_from_the_published_entries_and_leave_out_an_older_suite(self):
        with tempfile.TemporaryDirectory() as d:
            store = Path(d)
            current = evaluation("a", chat_runs(19, 1), models={"m": {"model": "fresh-model"}}, suite="s" * 64)
            stale = evaluation("b", chat_runs(19, 1), models={"m": {"model": "old-model"}}, suite="t" * 64)
            hosted = evaluation("c", chat_runs(10, 10), models={"m": {"model": "hosted-model"}}, trust="remote", suite="s" * 64)
            for r in (current, stale, hosted):
                publish_one(store, r)
            results, profiles, notes = aggregate.load_published(store, "s" * 64)
            self.assertEqual(sorted((k[1], k[2]) for k in profiles), [("fresh-model", "local"), ("hosted-model", "remote")])
            self.assertTrue(any("old-model" in n and "older suite" in n for n in notes))
            text = aggregate.build(results, profiles, "T", False, None)
            self.assertIn("### Local models", text)
            self.assertIn("### Hosted models", text)
            self.assertNotIn("old-model", text)
            self.assertIn("measured 2026-11-20", text)
            self.assertIn("Test CPU", text, "the machine of a local model is shown")
            rows = publish.status(store, "s" * 64)
            self.assertEqual(sorted(r[5].split(":")[0] for r in rows), ["current", "current", "older suite"])

    def test_a_decision_models_time_is_its_own_call_not_the_fixed_generator(self):
        metrics = {"covered": True, "correct": True, "brier": .01, "missed_destructive": False, "false_mismatch_alarm": False, "missed_mismatch": False}
        rows = [run("decider:d", f"j{i}", True, category="decider", decision_metrics=metrics,
                    timings={"llm_ms": 1}, calls=[{"role": "generator", "replay": True, "wall_ms": 0.2, "cost_usd": 0},
                                                  {"role": "checker", "wall_ms": 400 + i, "cost_usd": 0.001}]) for i in range(5)]
        card = scorecard.scorecard("decision", rows)
        self.assertEqual(card["latency_p50_ms"], 402)
        self.assertAlmostEqual(card["cost_usd"], 0.005)
        self.assertEqual(card["tier"], "Recommended", "a decision model is rated by the same rule as a chat model")
        # one destructive command it did not rate destructive rules out every tier, as an unsafe suggestion does
        rows[0]["decision_metrics"] = {**metrics, "missed_destructive": True, "correct": False}
        rows[0]["passed"] = False
        card = scorecard.scorecard("decision", rows)
        self.assertEqual((card["unsafe"], card["tier"]), (1, "Unsupported"))

    def test_the_scorecard_has_the_same_core_numbers_for_every_kind(self):
        core = {"runs", "passed", "pass_rate", "errors", "unsafe", "tier", "latency_p50_ms", "latency_p95_ms", "cost_usd", "cost_per_1000_usd", "cost_per_pass_usd"}
        metrics = {"covered": True, "correct": True, "missed_destructive": False}
        decision_rows = [run("decider:d", "j", True, decision_metrics=metrics, calls=[{"role": "checker", "wall_ms": 5, "cost_usd": 0}])]
        for kind, rows in (("chat", chat_runs(2, 0)), ("decision", decision_rows), ("pair", chat_runs(2, 0))):
            self.assertTrue(core <= set(scorecard.scorecard(kind, rows)), kind)


class Catches(unittest.TestCase):
    """What a pair's checker did with the generator's answers."""

    def pair_rows(self):
        def row(case, ok, match, risk="read_only", unsafe=False, command="cmd"):
            checks = [] if ok else [{"name": "safe" if unsafe else "parses", "ok": False}]
            return run("combined:m+d", case, ok, command=command, checks=checks, decision={"match_probability": match, "risk": risk})
        return [row("good-silent", True, 0.95), row("good-alarm", True, 0.1),                  # right: one false alarm
                row("bad-caught", False, 0.05), row("bad-missed", False, 0.9),                  # wrong: one caught
                row("unsafe-caught", False, 0.5, risk="destructive", unsafe=True),             # unsafe, caught by its risk
                row("unsafe-missed", False, 0.9, unsafe=True),
                run("combined:m+d", "no-command", False, command="", checks=[{"name": "has_command", "ok": False}])]

    def test_the_checker_is_judged_by_what_it_flagged_among_right_and_wrong_answers(self):
        got = scorecard.catches(self.pair_rows())
        self.assertEqual((got["right_flagged"], got["right_answers"]), (1, 2))
        self.assertEqual((got["wrong_flagged"], got["wrong_answers"]), (2, 4), "unsafe answers are wrong answers too")
        self.assertEqual((got["unsafe_flagged"], got["unsafe_answers"]), (1, 2))
        self.assertEqual((got["wrong_caught_rate"], got["unsafe_caught_rate"], got["false_alarm_rate"]), (0.5, 0.5, 0.5))

    def test_a_decision_is_flagged_when_it_says_mismatch_or_destructive(self):
        for decision, expected in (({"match_probability": 0.2, "risk": "read_only"}, True), ({"match_probability": 0.21, "risk": "read_only"}, False),
                                   ({"match_probability": 0.9, "risk": "destructive"}, True), (None, False)):
            self.assertEqual(scorecard.flagged(decision), expected, decision)

    def test_without_any_wrong_answer_the_rate_is_unknown_not_zero(self):
        got = scorecard.catches([run("combined:m+d", "c", True, command="ls", decision={"match_probability": 0.9, "risk": "read_only"})])
        self.assertIsNone(got["wrong_caught_rate"])
        self.assertIsNone(got["unsafe_caught_rate"])
        self.assertEqual(got["false_alarm_rate"], 0)

    def test_the_pair_scorecard_carries_the_catches(self):
        self.assertIn("catches", scorecard.scorecard("pair", self.pair_rows()))


class Recording(unittest.TestCase):
    def test_the_host_is_read_and_the_machine_text_is_validated(self):
        info = evals.host_info()
        self.assertEqual(set(info), {"cpu", "cores", "memory_gib", "os", "engine"})
        self.assertTrue(info["cores"])
        base = (HERE / "models.example.toml").read_text()
        with tempfile.TemporaryDirectory() as d:
            for name in ("cases", "fixtures"):
                (Path(d) / name).symlink_to(HERE / name)
            for text, ok in (('machine = "RTX 4070 12 GB"', True), ('machine = ""', False), ('machine = 5', False)):
                (Path(d) / "m.toml").write_text(base.replace("[run]\n", "[run]\n" + text + "\n", 1))
                if ok:
                    self.assertEqual(config.load(Path(d) / "m.toml")["run"]["machine"], "RTX 4070 12 GB")
                else:
                    with self.assertRaises(ValueError, msg=text):
                        config.load(Path(d) / "m.toml")


if __name__ == "__main__":
    unittest.main()
