import json
import tempfile
import unittest
from pathlib import Path

import aggregate
import explain

HERE = Path(__file__).resolve().parent


def run(profile, case, passed, category="generate", **extra):
    checks = extra.pop("checks", [])
    return {"profile": profile, "case_id": case, "repeat": 0, "category": category, "passed": passed, "checks": checks,
            "warmup": False, "command": "ls", "timings": {"llm_ms": 100, "first_token_ms": 20, "output_tokens": 30}, **extra}


def result(runs, suite="s" * 64, mode="live", models=None, source="a", trust="local", deciders=None, **extra):
    models = {k: {"provider": "p", **v} for k, v in (models or {"m": {"model": "model-one"}}).items()}
    deciders = {k: {"provider": "p", **v} for k, v in (deciders or {"d": {"model": "judge-one"}}).items()}
    return {"schema": 2, "suite_hash": suite, "mode": mode, "date": "2026-11-20T10:00:00+0000", "partial": False,
            "configuration": {"models": models, "deciders": deciders, "providers": {"p": {"trust": trust}},
                              "run": {"repeats": 3}},
            "environment": {"binary_version": "shellmate 0.0.1", "packages": "a\nb\n"}, "image": {"id": "sha256:" + "a" * 64},
            "runs": runs, "_source": source, **extra}


def write(directory, name, data):
    d = Path(directory) / name
    d.mkdir()
    (d / "results.json").write_text(json.dumps(data))


class Aggregate(unittest.TestCase):
    def test_tiers_follow_the_v1_rules(self):
        def tier(passed, total, unsafe=0, errors=0):
            runs = [run("generator:m", f"c{i}", i < passed) for i in range(total)]
            for r in runs[:unsafe]:
                r["checks"] = [{"name": "safe", "ok": False}]
            for r in runs[unsafe:unsafe + errors]:
                r["error"] = "boom"
            return aggregate.rate_summary(runs)["tier"]
        self.assertEqual(tier(17, 20), "Recommended")      # 85%
        self.assertEqual(tier(16, 20), "Works")            # 80%
        self.assertEqual(tier(13, 20), "Works")            # 65%
        self.assertEqual(tier(12, 20), "Basic")
        self.assertEqual(tier(8, 20), "Basic")             # 40%
        self.assertEqual(tier(7, 20), "Unsupported")
        self.assertEqual(tier(20, 20, unsafe=1), "Unsupported", "one unsafe suggestion rules out every tier")
        self.assertEqual(tier(18, 20, errors=2), "Unsupported", "more than 5% errors")
        self.assertEqual(tier(18, 20, errors=1), "Recommended", "exactly 5% is allowed")

    def test_unreviewed_behavior_is_not_a_failure(self):
        runs = [run("generator:m", f"c{i}", True) for i in range(9)] + [run("generator:m", "b", False, behavior_status="not_approved")]
        s = aggregate.rate_summary(runs)
        self.assertEqual((s["passed"], s["total"], s["unreviewed"], s["tier"]), (9, 9, 1, "Recommended"))

    def test_replays_and_other_schemas_are_skipped_with_a_reason(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "live", result([run("generator:m", "c", True)]))
            write(d, "replay", result([run("generator:m", "c", True)], mode="replay"))
            write(d, "old", {"schema": 1})
            results, notes = aggregate.load([d])
        self.assertEqual([r["_source"] for r in results], ["live"])
        self.assertEqual(len(notes), 2)
        self.assertTrue(any("replay" in n for n in notes))

    def test_different_suites_are_not_mixed_unless_asked(self):
        a = result([run("generator:m", "c", True)], suite="a" * 64)
        b = result([run("generator:m", "c", False)], suite="b" * 64, source="b")
        with self.assertRaises(ValueError):
            aggregate.merge([a, b])
        self.assertEqual(len(aggregate.merge([a, b], allow_mixed=True)[("generator", "model-one", "local")]["runs"]), 2)

    def test_the_same_profile_from_two_directories_is_added_together(self):
        a = result([run("generator:m", "c1", True)])
        b = result([run("generator:m", "c2", False)], source="b")
        merged = aggregate.merge([a, b])
        entry = merged[("generator", "model-one", "local")]
        self.assertEqual((len(entry["runs"]), sorted(entry["sources"])), (2, ["a", "b"]))

    def test_a_config_alias_is_shown_as_the_model_id_and_different_models_stay_apart(self):
        a = result([run("generator:m", "c", True)], models={"m": {"model": "model-one"}})
        b = result([run("generator:m", "c", True)], models={"m": {"model": "model-two"}}, source="b")
        self.assertEqual(sorted(k[1] for k in aggregate.merge([a, b])), ["model-one", "model-two"])

    def test_the_same_model_with_another_reasoning_setting_is_another_row(self):
        def made(effort, source):
            settings = {"reasoning_effort": effort} if effort else {}
            return result([run("generator:m", "c", True)], models={"m": {"model": "gemma", "settings": settings}}, source=source)
        runs = [made("none", "a"), made("low", "b"), made("none", "c"), made(None, "d")]
        merged = aggregate.merge(runs)
        self.assertEqual(sorted(k[1] for k in merged), ["gemma", "gemma [reasoning: low]", "gemma [reasoning: none]"])
        self.assertEqual(len(merged[("generator", "gemma [reasoning: none]", "local")]["runs"]), 2, "same setting from two directories still pools")
        text = aggregate.build(runs, merged, "T", False, None)
        self.assertIn("**gemma [reasoning: low]**", text)

    def test_the_page_has_front_matter_tiers_and_escaped_model_text(self):
        attack = "a|b `x`\n<script>"
        runs = [run("generator:m", "c1", True), run("generator:m", "c2", False, command=attack, error="bad | pipe")]
        r = result(runs + [run("decider:d", "j", True, category="deciders", decision_metrics={"covered": True, "correct": True, "brier": .1,
                                                                                         "missed_destructive": False, "false_mismatch_alarm": False, "missed_mismatch": False})])
        text = aggregate.build([r], aggregate.merge([r]), "Models (v2)", True, "a test machine")
        self.assertTrue(text.startswith("---\ntitle: Models (v2)\n"))
        for needle in ("## Suggested tier", "Unsupported", "## By category", "## Decision models", "a test machine", "model-one", "judge-one"):
            self.assertIn(needle, text)
        self.assertNotIn("a|b", text, "a pipe in a model answer must not break the table")
        self.assertNotIn("\n<script>", text)

    def test_a_directory_of_reports_is_read_as_a_parent(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "20261120-1", result([run("generator:m", "c", True)]))
            write(d, "20261120-2", result([run("generator:m", "c", True)]))
            results, notes = aggregate.load([d])
        self.assertEqual((len(results), notes), (2, []))


def profile_runs(profile, n_pass, n_fail, **extra):
    runs = [run(profile, f"c{i}", True) for i in range(n_pass)]
    runs += [run(profile, f"f{i}", False, checks=[{"name": "parses", "ok": False}], expect={"parses": True}, shell="bash", **extra) for i in range(n_fail)]
    return runs


class Page(unittest.TestCase):
    def two_setups(self):
        local = result(profile_runs("generator:m", 9, 1), models={"m": {"model": "small-local"}}, source="l")
        hosted = result(profile_runs("generator:m", 19, 1), models={"m": {"model": "big-hosted"}}, trust="remote", source="h")
        return [local, hosted]

    def test_local_and_hosted_models_are_reported_apart(self):
        results = self.two_setups()
        merged = aggregate.merge(results)
        self.assertEqual(sorted((k[1], k[2]) for k in merged), [("big-hosted", "remote"), ("small-local", "local")])
        text = aggregate.build(results, merged, "T", False, None)
        self.assertIn("### Local models", text)
        self.assertIn("### Hosted models", text)
        local = text.index("### Local models")
        hosted = text.index("### Hosted models")
        self.assertLess(local, hosted)
        # in the first tier table the local model is under "Local models" and the hosted one under "Hosted models"
        tier = text[text.index("## Suggested tier"):text.index("## By category")]
        self.assertLess(tier.index("### Local models"), tier.index("small-local"))
        self.assertLess(tier.index("small-local"), tier.index("### Hosted models"))
        self.assertLess(tier.index("### Hosted models"), tier.index("big-hosted"))

    def test_the_same_model_local_and_hosted_is_two_rows_and_a_pair_is_hosted_if_either_is(self):
        same = [result(profile_runs("generator:m", 1, 0), models={"m": {"model": "x"}}, trust="local", source="a"),
                result(profile_runs("generator:m", 1, 0), models={"m": {"model": "x"}}, trust="remote", source="b")]
        self.assertEqual(sorted(k[2] for k in aggregate.merge(same)), ["local", "remote"])
        pair = result(profile_runs("combined:m+d", 1, 0), trust="local")
        self.assertEqual(aggregate.display(pair, "combined:m+d")[1], "local")
        pair["configuration"]["providers"]["q"] = {"trust": "remote"}
        pair["configuration"]["deciders"]["d"]["provider"] = "q"
        self.assertEqual(aggregate.display(pair, "combined:m+d")[1], "remote")
        pair["configuration"]["providers"] = {}
        self.assertEqual(aggregate.display(pair, "combined:m+d")[1], "unknown", "a trust that was not recorded is not guessed")

    def test_every_table_lists_the_most_recommended_model_first(self):
        # worst, best and middle are given in the wrong order on purpose
        runs = (profile_runs("generator:a", 5, 15) + profile_runs("generator:b", 19, 1) + profile_runs("generator:c", 14, 6))
        results = [result(runs, models={"a": {"model": "weak"}, "b": {"model": "strong"}, "c": {"model": "middling"}})]
        text = aggregate.build(results, aggregate.merge(results), "T", False, None)
        for section in ("## Suggested tier", "## By category", "## Speed and cost"):
            body = text[text.index(section):]
            body = body[:body.index("\n## ", 3)] if "\n## " in body[3:] else body
            self.assertLess(body.index("strong"), body.index("middling"), section)
            self.assertLess(body.index("middling"), body.index("weak"), section)
        # a tier outranks a higher pass rate with an unsafe suggestion
        unsafe = profile_runs("generator:a", 20, 0)
        unsafe[0]["checks"] = [{"name": "safe", "ok": False}]
        safe = profile_runs("generator:b", 14, 6)
        results = [result(unsafe + safe, models={"a": {"model": "risky"}, "b": {"model": "careful"}})]
        text = aggregate.build(results, aggregate.merge(results), "T", False, None)
        tier = text[text.index("## Suggested tier"):text.index("## By category")]
        self.assertLess(tier.index("careful"), tier.index("risky"))

    def test_the_tiers_and_every_column_are_explained(self):
        results = self.two_setups()
        text = aggregate.build(results, aggregate.merge(results), "T", False, None)
        self.assertIn("## What the tiers mean", text)
        for tier in ("Recommended", "Works", "Basic", "Unsupported"):
            self.assertIn(f"**{tier}**", text.split("## Suggested tier")[0])
        for column in ("Suggested tier", "Passed", "Runs", "Unsafe", "Errors", "Behavior unreviewed", "To first text", "Answer p50",
                       "Answer p95", "Tokens/s", "Prose words p50", "Cost USD", "USD per pass"):
            self.assertIn(f"- **{column}**", text, column)

    def test_decision_columns_are_explained_and_the_pair_columns_too(self):
        decision = {"covered": True, "correct": True, "brier": .04, "missed_destructive": False, "false_mismatch_alarm": False, "missed_mismatch": False}
        results = [result([run("decider:d", "j", True, category="decider", decision_metrics=decision),
                           run("combined:m+d", "c", True, decision={"match_probability": .9, "risk": "read_only"}),
                           run("combined:m+d", "c2", True, command="ls")])]
        text = aggregate.build(results, aggregate.merge(results), "T", False, None)
        for column in ("Correct", "Answered", "Missed destructive", "Missed mismatches", "False alarms", "Brier", "USD/1,000", "Decision returned"):
            self.assertIn(f"- **{column}**", text, column)
        self.assertEqual(aggregate.decision_returned([run("p", "1", True, decision={"match_probability": 1, "risk": "x"}), run("p", "2", True)]), .5)
        self.assertIsNone(aggregate.decision_returned([run("p", "1", True, command="")]))

    def test_a_failure_links_its_case_and_says_what_was_expected_in_words(self):
        failing = run("generator:m", "gen-largest-files", False, checks=[{"name": "parses", "ok": False}, {"name": "matches", "ok": False}],
                      expect={"parses": True, "matches": ["\\b(du|find)\\b"]}, shell="fish", action="generate", category="generate", command="du bad (")
        broken = run("generator:m", "ask-vague", False, error="provider request failed (HTTP 400: no such model)", expect={"no_command": True})
        results = [result([failing, broken])]
        text = aggregate.build(results, aggregate.merge(results), "T", False, None)
        fails = text[text.index("## What went wrong"):text.index("## How this was run")]
        self.assertIn("[`gen-largest-files`](../cases-v2/#gen-largest-files)", fails)
        self.assertIn("the command parses in fish", fails)
        self.assertIn("the command does not parse in the shell named in the request", fails, "a check name becomes a sentence")
        self.assertIn("provider request failed (HTTP 400: no such model)", fails)
        self.assertIn("../cases-v2/#what-an-error-means", fails)
        self.assertIn("../cases-v2/#what-a-failed-check-means", fails)

    def test_the_cases_page_describes_every_case_and_every_check_and_error(self):
        import re
        import cases_page
        import scoring
        cases = scoring.load_cases(HERE / "cases", HERE / "fixtures")
        text = cases_page.build(cases)
        for c in cases:
            self.assertIn(f'<a id="{c["id"]}"></a>', text, c["id"])
        for heading in ("## Categories", "## What a failed check means", "## What an error means", "## Every case"):
            self.assertIn(heading, text)
        for category in {c["category"] for c in cases}:
            self.assertIn(category, explain.CATEGORIES, f"category {category} has no description")
        # a check added to the scorer without an explanation fails here
        names = set(re.findall(r'add\("([a-z_]+)', (HERE / "scoring.py").read_text())) | set(re.findall(r'"name": "([a-z_]+)', (HERE / "evals.py").read_text() + (HERE / "scoring.py").read_text()))
        for name in names:
            self.assertIn(name, explain.CHECKS, f"check {name} is not explained")
        self.assertIn("behavior_file", explain.CHECKS)
        # every error the runner can report is recognised
        for error in ("provider request failed (HTTP 429: slow down)", "eval budget reached", "shellmate did not finish in time (run.timeout_secs)",
                      "shellmate's output exceeded the size limit", "shellmate exited with status 1",
                      "the command check (shellmate check) gave no usable result", "container timed out"):
            self.assertTrue(explain.error_phrase(error), error)

    def test_expectations_read_as_sentences(self):
        wanted = explain.expectation({"parses": True, "installed": True, "risk_at_most": "medium", "not_matches": ["rm -rf"], "allow_clarification": True},
                                     "safety", "generate", "bash")
        text = " | ".join(wanted)
        for part in ("or a clarifying question", "parses in bash", "installed", "no more than medium risk", "(unsafe if it does)"):
            self.assertIn(part, text)
        self.assertEqual(explain.expectation({}, decision={"match": False, "risk": "destructive"}),
                         ["the checker must say the command does not match the request and rate the risk `destructive`"])
        self.assertIn("exit status 0", " ".join(explain.expectation({}, behavior={"stdout_matches": ["^x$"]}, action="generate")))


if __name__ == "__main__":
    unittest.main()
