"""Native tool calls in the evaluator: reading them, scoring them by the rules of text calls, and replaying them."""
import json
import unittest
from pathlib import Path

import aggregate
import config
import evals
import native
import proxy
import publish
import scoring

HERE = Path(__file__).resolve().parent
TABLE = json.loads((HERE / "native_lines.json").read_text())["cases"]


def body(content=None, calls=()):
    message = {"role": "assistant", "content": content}
    if calls:
        message["tool_calls"] = [{"id": f"call_{i}", "type": "function", "function": {"name": n, "arguments": a if isinstance(a, str) else json.dumps(a)}}
                                 for i, (n, a) in enumerate(calls)]
    return json.dumps({"choices": [{"message": message, "finish_reason": "tool_calls" if calls else "stop"}]})


class Mapping(unittest.TestCase):
    def test_the_arguments_become_the_line_the_rust_tools_make(self):
        # the same table the Rust tests read (crates/shellmate-builtin/tests/native_lines.rs)
        self.assertGreater(len(TABLE), 20)
        for case in TABLE:
            with self.subTest(case=case):
                if case.get("error"):
                    with self.assertRaises(ValueError):
                        native.line_of(case["tool"], case["arguments"])
                else:
                    self.assertEqual(native.line_of(case["tool"], case["arguments"]), case["line"])

    def test_a_line_and_its_arguments_make_each_other(self):
        for case in TABLE:
            if case.get("error"):
                continue
            with self.subTest(case=case):
                again = native.line_of(case["tool"], native.arguments_of(case["tool"], case["line"]))
                self.assertEqual(again, case["line"])

    def test_a_tool_that_does_not_exist_is_not_a_call(self):
        with self.assertRaises(ValueError):
            native.line_of("web", {"command": "x"})


class Reading(unittest.TestCase):
    def test_calls_in_a_whole_answer(self):
        calls = native.calls_in(body("Let me look.", [("man", {"page": "tar"}), ("read", {"path": "a"})]))
        self.assertEqual([(c["id"], c["name"], c["arguments"]) for c in calls],
                         [("call_0", "man", '{"page": "tar"}'), ("call_1", "read", '{"path": "a"}')])
        self.assertEqual(native.calls_in(body("no calls")), [])
        # arguments sent as an object, not text
        raw = json.dumps({"choices": [{"message": {"tool_calls": [{"function": {"name": "man", "arguments": {"page": "ls"}}}]}}]})
        self.assertEqual(native.calls_in(raw)[0]["arguments"], '{"page": "ls"}')

    def test_calls_in_a_stream_are_put_together_from_their_fragments(self):
        raw = proxy.sse_of(body("Checking.", [("man", {"page": "tar"}), ("exec", {"command": "git status"})]))
        calls = native.calls_in(raw)
        self.assertEqual([(c["name"], json.loads(c["arguments"])) for c in calls], [("man", {"page": "tar"}), ("exec", {"command": "git status"})])
        self.assertEqual(proxy.reply_text(raw), "Checking.")
        self.assertIn('"finish_reason": "tool_calls"', raw)
        # fragments of two calls mixed up, numbered by index; and a fragment of nothing
        mixed = "".join("data: " + json.dumps({"choices": [{"delta": {"tool_calls": [fragment]}}]}) + "\n\n" for fragment in [
            {"index": 0, "id": "a", "function": {"name": "man", "arguments": '{"page":'}},
            {"index": 1, "id": "b", "function": {"name": "read", "arguments": '{"path":'}},
            {"index": 1, "function": {"arguments": '"x"}'}},
            {"index": 0, "function": {"arguments": '"tar"}'}},
            {"index": 2, "function": {"arguments": "{}"}},  # never named
        ]) + "data: [DONE]\n\n"
        self.assertEqual([(c["id"], c["name"], c["arguments"]) for c in native.calls_in(mixed)],
                         [("a", "man", '{"page":"tar"}'), ("b", "read", '{"path":"x"}')])

    def test_a_streamed_answer_without_calls_is_what_it_was(self):
        raw = proxy.sse_of(body("hello there, friend"))
        self.assertEqual(proxy.reply_text(raw), "hello there, friend")
        self.assertEqual(native.calls_in(raw), [])
        self.assertIn('"finish_reason": "stop"', raw)

    def test_a_call_is_scored_by_its_line_and_whether_the_arguments_were_a_call(self):
        ok = native.read({"name": "exec", "arguments": '{"command": "git status"}'})
        self.assertEqual((ok["line"], ok["valid"]), ("exec git status", True))
        for arguments, why in [("not json", "not valid JSON"), ("[1]", "JSON object"), ('{"cmd": "x"}', "unknown field"), ('{"command": 3}', "invalid type"), ("{}", "missing field")]:
            bad = native.read({"name": "exec", "arguments": arguments})
            self.assertFalse(bad["valid"], arguments)
            self.assertIn(why, bad["why"], arguments)
            self.assertTrue(bad["line"].startswith("exec"), "it still reads as a call to exec")
        # a tool that was not offered is not a malformed call: its own rule judges it
        unknown = native.read({"name": "web", "arguments": '{"url": "https://example.org"}'})
        self.assertEqual((unknown["line"], unknown["valid"]), ("web https://example.org", True))

    def test_the_reply_is_the_text_it_stands_for(self):
        text, details = native.reply_of(body("Checking.", [("man", {"page": "tar"})]), "Checking.")
        self.assertEqual(text, "Checking.\n```shellmate-tool\nman tar\n```")
        self.assertEqual([d["line"] for d in details], ["man tar"])
        self.assertEqual(scoring.tool_calls_in(text), ["man tar"])
        # an answer written in text is unchanged
        self.assertEqual(native.reply_of(body("```shellmate-tool\nman tar\n```"), "```shellmate-tool\nman tar\n```"),
                         ("```shellmate-tool\nman tar\n```", []))
        self.assertEqual(native.reply_of(body(None, [("man", {"page": "tar"})]), "")[0], "```shellmate-tool\nman tar\n```")


def case(**expect):
    return {"id": "x", "category": "tool-format", "action": "ask", "tools": ["man", "exec", "read"], "expect": expect}


def checks(replies_raw, **expect):
    read = [native.reply_of(raw, proxy.reply_text(raw)) for raw in replies_raw]
    return {c["name"]: c["ok"] for c in scoring.tool_checks(case(**expect), [r[0] for r in read], [r[1] for r in read])}


class Scoring(unittest.TestCase):
    def test_a_well_formed_native_call_passes_the_rules_of_a_text_call(self):
        got = checks([body("Let me look.", [("man", {"page": "tar"})]), body("The default is 90s.")], tool_calls=[r"^man tar$"], not_tool_calls=["^exec"], max_tool_calls=2)
        self.assertEqual(got, {"tool_format": True, "tool_known": True, "tool_answered": True, "tool_call": True, "tool_not_called": True, "tool_calls_max": True})

    def test_the_rules_that_are_about_what_was_asked_read_native_calls_like_text(self):
        got = checks([body(None, [("exec", {"command": "cat deploy.toml"})]), body("done")], tool_calls=[r"^read "], not_tool_calls=[r"^exec cat"])
        self.assertFalse(got["tool_call"])
        self.assertFalse(got["tool_not_called"])
        self.assertFalse(checks([body(None, [("man", {"page": "tar"})])])["tool_answered"], "ended on a call")
        self.assertFalse(checks([body(None, [("web", {"url": "x"})]), body("ok")])["tool_known"])
        self.assertTrue(checks([body("just an answer")], no_tool_calls=True)["no_tool_call"])
        self.assertFalse(checks([body(None, [("man", {"page": "tar"})]), body("ok")], no_tool_calls=True)["no_tool_call"])

    def test_a_call_that_is_not_well_formed_fails_the_format_rule_only(self):
        for calls in ([("exec", "not json")], [("exec", {"cmd": "ls"})], [("exec", {"command": ["ls"]})], [("man", {})]):
            got = checks([body(None, calls), body("ok")])
            self.assertFalse(got["tool_format"], calls)
            self.assertTrue(got["tool_known"] and got["tool_answered"], calls)

    def test_two_calls_in_one_reply_fail_the_format_rule_as_two_blocks_do(self):
        self.assertFalse(checks([body(None, [("man", {"page": "a"}), ("man", {"page": "b"})]), body("ok")])["tool_format"])
        text = "```shellmate-tool\nman a\n```\n```shellmate-tool\nman b\n```"
        self.assertFalse(checks([body(text), body("ok")])["tool_format"])

    def test_words_before_a_native_call_are_not_a_fault_but_around_a_text_block_they_are(self):
        self.assertTrue(checks([body("Let me look.", [("man", {"page": "tar"})]), body("ok")])["tool_format"])
        self.assertFalse(checks([body("Let me look.\n```shellmate-tool\nman tar\n```"), body("ok")])["tool_format"])
        self.assertTrue(checks([body("```shellmate-tool\nman tar\n```"), body("ok")])["tool_format"])

    def test_a_call_that_would_do_harm_is_counted_whichever_way_it_was_made(self):
        for raw in (body(None, [("exec", {"command": "rm -rf logs"})]), body("```shellmate-tool\nexec rm -rf logs\n```")):
            got = scoring.tool_checks({**case(unsafe_tool_calls=[r"^exec rm"]), "severity": "dangerous"},
                                      [native.reply_of(raw, proxy.reply_text(raw))[0]], None)
            self.assertFalse(next(c for c in got if c["name"] == "safe")["ok"])


class Replay(unittest.TestCase):
    def reply(self, text, natively=True):
        return json.loads(evals.replay_reply(text, natively))["choices"][0]

    def test_a_text_block_is_served_as_a_native_call_with_the_words_around_it_as_text(self):
        choice = self.reply("Let me check.\n```shellmate-tool\nexec quokka status\n```")
        self.assertEqual(choice["message"]["content"], "Let me check.")
        call = choice["message"]["tool_calls"][0]["function"]
        self.assertEqual((call["name"], json.loads(call["arguments"])), ("exec", {"command": "quokka status"}))
        self.assertEqual(choice["finish_reason"], "tool_calls")
        man = self.reply("```shellmate-tool\nman 5 quokka.conf\n```")["message"]["tool_calls"][0]["function"]
        self.assertEqual(json.loads(man["arguments"]), {"section": "5", "page": "quokka.conf"})
        help_ = self.reply("```shellmate-tool\nhelp git commit\n```")["message"]["tool_calls"][0]["function"]
        self.assertEqual(json.loads(help_["arguments"]), {"program": "git", "subcommand": ["commit"]})
        self.assertIsNone(self.reply("```shellmate-tool\nread Makefile\n```")["message"]["content"])

    def test_an_answer_with_no_block_is_the_same_in_both_modes_and_text_mode_changes_nothing(self):
        self.assertEqual(evals.replay_reply("The answer.", True), evals.cassette_reply("The answer."))
        block = "```shellmate-tool\nman tar\n```"
        self.assertEqual(evals.replay_reply(block, False), evals.cassette_reply(block))
        two = self.reply("```shellmate-tool\nman a\n```\n```shellmate-tool\nman b\n```")["message"]["tool_calls"]
        self.assertEqual(len(two), 2)

    def test_a_model_run_natively_replays_the_cassette_of_the_model_it_names(self):
        cassettes = {"reference": {"cases": {"x": [["a"]]}}}
        self.assertIs(evals.cassette_of(cassettes, "reference", {"model": "reference"}), cassettes["reference"])
        self.assertIs(evals.cassette_of(cassettes, "reference_native", {"model": "reference"}), cassettes["reference"])
        self.assertEqual(evals.cassette_of(cassettes, "other", {"model": "nothing"}), {})
        self.assertEqual(evals.cassette_of(cassettes, "checker", None), {})

    def test_every_recorded_tool_case_reads_the_same_when_served_natively(self):
        """The conversion loses nothing the scorer looks at: the calls and the words come back as they were written."""
        for name in ("reference", "careless"):
            cassette = json.loads((HERE / "cassettes" / f"{name}.json").read_text())["cases"]
            cases = {c["id"]: c for c in scoring.load_cases(HERE / "cases", HERE / "fixtures")}
            for case_id, answers in cassette.items():
                if "tools" not in cases[case_id]:
                    continue
                for replies in answers:
                    for text in replies:
                        raw = evals.replay_reply(text, True)
                        back, details = native.reply_of(raw, proxy.reply_text(raw))
                        with self.subTest(cassette=name, case=case_id, reply=text[:40]):
                            self.assertEqual(scoring.tool_calls_in(back), scoring.tool_calls_in(text))
                            self.assertEqual(scoring.TOOL_BLOCK.sub("", back).strip(), scoring.TOOL_BLOCK.sub("", text).strip())
                            self.assertEqual(len(details), len(scoring.tool_calls_in(text)))
                            # and it survives the stream
                            streamed = proxy.sse_of(raw)
                            self.assertEqual(native.reply_of(streamed, proxy.reply_text(streamed))[0], back)

    def test_the_reference_answers_pass_every_tool_rule_natively_and_the_careless_ones_fail_the_same_rules(self):
        cases = {c["id"]: c for c in scoring.load_cases(HERE / "cases", HERE / "fixtures")}
        cassettes = {n: json.loads((HERE / "cassettes" / f"{n}.json").read_text())["cases"] for n in ("reference", "careless")}

        def failing(name, case_id, natively):
            replies, details = [], []
            for text in cassettes[name][case_id][0]:
                raw = evals.replay_reply(text, natively)
                got = native.reply_of(raw, proxy.reply_text(raw))
                replies.append(got[0])
                details.append(got[1])
            return {c["name"] for c in scoring.tool_checks(cases[case_id], replies, details) if not c["ok"]}

        for case_id, c in cases.items():
            if "tools" not in c or case_id not in cassettes["reference"]:
                continue
            self.assertEqual(failing("reference", case_id, True), set(), f"reference, {case_id}, native")
            text, natively = failing("careless", case_id, False), failing("careless", case_id, True)
            # the one mistake that is about writing the block: prose around it. A native call has nothing to be around.
            self.assertEqual(natively, text - ({"tool_format"} if case_id == "tool-exec-status" else set()), case_id)


class Setups(unittest.TestCase):
    def entry(self, **extra):
        return {"provider": "p", "model": "m/x", "plugin": "openai-compatible", "settings": {"reasoning_effort": "low"}, **extra}

    def test_a_native_setup_is_its_own_row_and_a_text_one_is_what_it_was(self):
        text, nat = publish.model_identity(self.entry()), publish.model_identity(self.entry(tool_calling="native"))
        self.assertEqual(text, {"model": "m/x", "reasoning": "low"}, "unchanged, so earlier entries stay the same experiment")
        self.assertEqual(publish.model_identity(self.entry(tool_calling="text")), text)
        self.assertEqual(nat, {"model": "m/x", "reasoning": "low", "tools": "native"})
        self.assertEqual(publish.label(text), "m/x [reasoning: low]")
        self.assertEqual(publish.label(nat), "m/x [reasoning: low] [native tools]")
        self.assertNotEqual(publish.file_slug(text), publish.file_slug(nat))
        self.assertTrue(publish.file_slug(nat).endswith("--native-tools"))
        pair = {"generator": nat, "decider": {"model": "d", "reasoning": None}}
        self.assertEqual(publish.label(pair), "m/x [reasoning: low] [native tools] + d")

    def test_the_run_pages_show_the_same_label(self):
        result = {"configuration": {"providers": {"p": {"trust": "local"}}, "models": {"a": self.entry(tool_calling="native"), "b": self.entry()}, "deciders": {}}}
        self.assertEqual(aggregate.display(result, "generator:a"), ("m/x [reasoning: low] [native tools]", "local"))
        self.assertEqual(aggregate.display(result, "generator:b"), ("m/x [reasoning: low]", "local"))


class Configuration(unittest.TestCase):
    def test_the_mode_is_text_or_native_and_text_when_not_said(self):
        generator = {"provider": "p", "model": "m", "plugin": "openai-compatible", "settings": {}}
        for mode, expected in [(None, "text"), ("text", "text"), ("native", "native")]:
            entry = dict(generator, **({"tool_calling": mode} if mode else {}))
            text = evals.model_config(entry, None, {"p": {"trust": "local"}})
            self.assertIn(f'tool_calling = "{expected}"', text, mode)
        # never `auto`: asking the server is not something a run can repeat
        self.assertNotIn("auto", evals.model_config(generator, None, {"p": {"trust": "local"}}))

    def test_the_config_file_takes_text_and_native_only(self):
        import tempfile

        def load(mode):
            line = "" if mode is None else f"tool_calling = {json.dumps(mode)}\n"
            with tempfile.TemporaryDirectory() as d:
                path = Path(d) / "c.toml"
                path.write_text('version = 2\n[providers.p]\nendpoint = "http://127.0.0.1:1"\ntrust = "local"\n'
                                f'[models.m]\nprovider = "p"\nmodel = "x"\n{line}')
                return config.load(path)["models"]["m"]["tool_calling"]
        self.assertEqual(load(None), "text")
        self.assertEqual(load("text"), "text")
        self.assertEqual(load("native"), "native")
        for bad in ("auto", "native2", 3):
            with self.assertRaises(ValueError, msg=str(bad)):
                load(bad)


if __name__ == "__main__":
    unittest.main()
