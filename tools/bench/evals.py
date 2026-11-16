#!/usr/bin/env python3
"""Measure models through the real shellmate binary: how often the answer is right, and how long it takes.

For every (model, case) the binary is run exactly as the shell glue runs it (`invoke --timings=json`), and the
answer is scored by fixed rules from cases/*.toml (does the command parse in the shell, does it use the right
tool, does it avoid what it must not do). No model judges another. Timings come from shellmate itself.

  evals.py --ollama                       every model your local Ollama has
  evals.py --models models.toml           the models you list (local or remote)
  evals.py --models m.toml --model qwen   only models whose name contains "qwen"
  evals.py --replay cassettes             no model: replay recorded answers (checks the pipeline and the scoring)
  evals.py ... --record cassettes         also save each model's answers for later replay

Results go to results/evals-<label>.json and a Markdown report is printed; `report.py evals` rebuilds the
report from result files, and merges several of them into one comparison.
"""
from __future__ import annotations

import argparse
import collections
import json
import re
import sys
import tempfile
import time
import tomllib
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lib  # noqa: E402
import report  # noqa: E402

HERE = Path(__file__).resolve().parent
PRESETS = {"ollama": "http://localhost:11434", "lmstudio": "http://localhost:1234",
           "openai": "https://api.openai.com", "openrouter": "https://openrouter.ai/api"}
SAFETY = {"safety", "injection"}
RISK = {"low": 0, "medium": 1, "high": 2}
WARMUP = ("generate", "list the files in this directory")


# ----------------------------------------------------------------------------------------------- cases
def load_cases(path: Path = HERE / "cases") -> list[dict]:
    cases = []
    for f in sorted(path.glob("*.toml")):
        for c in tomllib.loads(f.read_text()).get("case", []):
            missing = {"id", "category", "action", "shell", "input"} - set(c)
            if missing:
                raise SystemExit(f"{f.name}: a case lacks {sorted(missing)}: {c.get('id', c)}")
            c.setdefault("fixture", "rust-project")
            c.setdefault("expect", {})
            cases.append(c)
    ids = [c["id"] for c in cases]
    dup = {i for i in ids if ids.count(i) > 1}
    if dup:
        raise SystemExit(f"duplicate case ids: {sorted(dup)}")
    return cases


# --------------------------------------------------------------------------------------------- scoring
ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


def clean_text(stderr: str) -> str:
    """The model's words as the user would read them: no colour codes, spinner, timings or `check:` notes."""
    keep = []
    for line in ANSI.sub("", stderr).replace("\r", "\n").splitlines():
        s = line.strip()
        if s.startswith('{"timings"') or s.startswith(("check:", "note:")) or s.endswith("thinking…"):
            continue
        keep.append(line.rstrip())
    return "\n".join(keep).strip()


def command_facts(binary: Path, shell: str, command: str, env: dict) -> dict:
    """What the product itself says about a command line (`shellmate check --json`): never run."""
    r = lib.run_binary([str(binary), "check", "--shell", shell, "--json", command], env=env, timeout=20)
    try:
        return json.loads(r.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return {"parses": None, "missing": [], "risk": "low", "reasons": []}


def first_word(command: str) -> str:
    words = command.split()
    while words and words[0] in ("sudo", "env", "time", "command", "exec", "nice"):
        words = words[1:]
        while words and (words[0].startswith("-") or "=" in words[0]):
            words = words[1:]
    return Path(words[0]).name if words else ""


def score(case: dict, command: str, text: str, facts: dict | None) -> list[dict]:
    """Every rule in [case.expect] becomes one check {name, ok, detail}."""
    e, action = case["expect"], case["action"]
    safety = case["category"] in SAFETY
    # what a pattern is searched in: the command for `generate`, the whole answer otherwise
    target = command if action == "generate" else (text + "\n" + command).strip()
    out: list[dict] = []

    def add(name, ok, detail=""):
        out.append({"name": name, "ok": bool(ok), "detail": detail})

    if action == "generate" and e.get("command", True) is not False and not e.get("no_command"):
        add("has_command", command.strip(), "no command was suggested")
    if e.get("no_command"):
        add("no_command", not command.strip(), f"suggested a command instead of asking: {command.strip()[:60]}")
    if e.get("answer_nonempty"):
        add("answer", text.strip(), "no answer text")
    if command.strip() and facts is not None:
        if e.get("parses"):
            add("parses", facts.get("parses") is not False,
                f"does not parse in {case['shell']}: {facts.get('syntax_error') or '?'}")
        if e.get("installed"):
            add("installed", not facts.get("missing"), f"not installed: {', '.join(facts.get('missing', []))}")
        if "risk_at_most" in e:
            add("safe", RISK.get(facts.get("risk", "low"), 0) <= RISK[e["risk_at_most"]],
                f"{facts.get('risk')} risk: {'; '.join(facts.get('reasons', []))[:100]}")
    elif command.strip() == "" and (e.get("parses") or e.get("installed")):
        add("parses" if e.get("parses") else "installed", False, "there is no command to check")
    if "first_word_in" in e:
        w = first_word(command)
        add("first_word", w in e["first_word_in"], f"starts with `{w}`")
    for rx in e.get("matches", []):
        add("matches", re.search(rx, target, re.I | re.M), f"does not match /{rx}/")
    for rx in e.get("not_matches", []):
        hit = re.search(rx, target, re.I | re.M)
        add("safe" if safety else "not_matches", not hit, f"matches /{rx}/: `{hit.group(0)[:40] if hit else ''}`")
    low = target.lower()
    for group in e.get("mentions", []):
        add("mentions", any(t.lower() in low for t in group), f"mentions none of {group}")
    return out


# ----------------------------------------------------------------------------------------------- models
def load_models(path: Path) -> list[dict]:
    models = tomllib.loads(path.read_text()).get("model", [])
    for m in models:
        if not {"name", "model"} <= set(m) or not ({"preset", "endpoint"} & set(m)):
            raise SystemExit(f"{path.name}: a model needs name, model and preset or endpoint: {m}")
    return models


def ollama_models(host: str = "http://localhost:11434") -> list[dict]:
    try:
        with urllib.request.urlopen(host + "/api/tags", timeout=5) as r:
            tags = json.load(r)["models"]
    except (OSError, ValueError, KeyError) as e:
        raise SystemExit(f"cannot list Ollama models at {host}: {e}")
    return [{"name": t["name"].replace(":", "-"), "model": t["name"], "preset": "ollama", "trust": "local",
             "size_gb": round(t.get("size", 0) / 1e9, 1)} for t in tags
            if "embed" not in t["name"] and "systemone" not in t["name"]]


def toml_str(s: str) -> str:
    return json.dumps(s)  # a JSON string is a valid TOML basic string


def model_config(m: dict, endpoint: str, tools: str) -> str:
    conn = [f"endpoint = {toml_str(endpoint)}"]
    if m.get("api_key"):
        conn.append(f"api_key = {toml_str(m['api_key'])}")
    s = [f"model = {toml_str(m['model'])}"]
    for k, v in m.get("settings", {}).items():
        s.append(f"{k} = {json.dumps(v)}")
    level = m.get("level", 1)
    return (
        f'version = 1\n[context]\ndefault_level = {level}\n'
        f'[providers.m]\ntrust = {toml_str(m.get("trust", "local"))}\n'
        f'max_context_level = {max(level, 1)}\n'
        + "\n".join(conn)
        + '\n[models.m]\nprovider = "m"\nplugin = "builtin:openai-compatible"\n'
        + "\n".join(s)
        + '\n[llm]\nchain = ["m"]\n'
        '[audit]\nenabled = false\n[conversation]\nstore = "none"\n[validator]\nenabled = false\n'
        + ('[tools.man]\nenabled = false\n' if tools == "none" else "")
        + m.get("extra_config", "")
    )


# ------------------------------------------------------------------------------------------- the runner
class Cassette:
    """Recorded answers of one model: {case id: [one list of replies per repeat]}. A case may take several
    replies in one run (a tool call, then the answer)."""

    def __init__(self, path: Path):
        self.path = path
        self.data = json.loads(path.read_text()) if path.exists() else {"cases": {}}

    def replies(self, case_id: str, repeat: int) -> list[str] | None:
        runs = self.data["cases"].get(case_id)
        return runs[repeat % len(runs)] if runs else None

    def put(self, case_id: str, repeat: int, replies: list[str]):
        self.data["cases"].setdefault(case_id, [])
        runs = self.data["cases"][case_id]
        while len(runs) <= repeat:
            runs.append([])
        runs[repeat] = replies

    def save(self, meta: dict):
        self.data.update(meta)
        self.data["recorded"] = time.strftime("%Y-%m-%d")
        lib.write_json(self.path, self.data)


def one_run(binary: Path, cfg: Path, env: dict, case: dict, timeout: float, tools_cwd: Path) -> dict:
    cwd = str(tools_cwd / case["fixture"])
    r = lib.run_binary([str(binary), "--config", str(cfg), "invoke", "--timings=json"],
                       lib.envelope(case["action"], case["input"], case["shell"], cwd), env=env, timeout=timeout)
    t = lib.parse_timings(r.stderr) or {}
    run = {"wall_ms": round(r.wall_ms, 1), "llm_ms": t.get("llm_ms"), "first_token_ms": t.get("first_token_ms"),
           "in_tok": t.get("input_tokens"), "out_tok": t.get("output_tokens")}
    command, text = r.stdout.strip(), clean_text(r.stderr)
    run["command"], run["output"] = command, text[:1500]
    if r.timed_out:
        run["error"] = f"no answer within {timeout:.0f} s"
    elif r.code not in (0, 10):
        # shellmate's own error message (empty answer, unreachable server...), not something the model said
        run["error"] = f"exit {r.code}: {' '.join(text.split())[:200]}"
        run["output"] = ""
    return run


def evaluate(models: list[dict], cases: list[dict], binary: Path, repeats: int, timeout: float, tools: str,
             record: Path | None, replay: Path | None, warmup: bool, out: list[dict]) -> list[dict]:
    """Fills `out` as it goes, so an interrupted run still has everything finished so far."""
    with tempfile.TemporaryDirectory(prefix="sm-evals-") as tmp:
        tmp = Path(tmp)
        env = lib.isolated_env(tmp / "home")
        fixtures = tmp / "fixtures"
        import shutil
        shutil.copytree(HERE / "fixtures", fixtures)
        for mi, m in enumerate(models):
            print(f"== {m['name']} ({mi + 1}/{len(models)})", file=sys.stderr)
            cassette = Cassette((replay or record) / f"{m['name']}.json") if (replay or record) else None
            current: collections.deque = collections.deque()
            miss = {"n": 0}

            def reply(body, current=current, miss=miss):
                if current:
                    return current.popleft()
                miss["n"] += 1
                return ""

            if replay:
                server = lib.FakeLLM(reply)
            elif record:
                server = lib.RecordingProxy(m.get("endpoint") or PRESETS[m["preset"]])
            else:
                server = None
            with server or _Null() as srv:
                endpoint = srv.url if server else (m.get("endpoint") or PRESETS[m["preset"]])
                cfg = tmp / f"{m['name']}.toml"
                cfg.write_text(model_config(m, endpoint, tools))
                cold = None
                if warmup and not replay:
                    w = one_run(binary, cfg, env, {"action": WARMUP[0], "input": WARMUP[1], "shell": "bash",
                                                   "fixture": "empty", "id": "warmup"}, timeout, fixtures)
                    cold = None if w.get("error") else w["wall_ms"]
                    if w.get("error"):
                        print(f"   warm-up failed: {w['error']}", file=sys.stderr)
                entry = {"name": m["name"], "model": m["model"], "trust": m.get("trust", "local"),
                         "level": m.get("level", 1), "size_gb": m.get("size_gb"), "cold_start_ms": cold, "cases": []}
                out.append(entry)
                for c in cases:
                    runs = []
                    for rep in range(repeats):
                        if replay:
                            recorded = cassette.replies(c["id"], rep)
                            if recorded is None:
                                runs.append({"pass": False, "checks": [], "error": "not in the cassette (record it again)"})
                                continue
                            current.clear()
                            current.extend(recorded)
                        if record:
                            srv.replies.clear()
                        run = one_run(binary, cfg, env, c, timeout, fixtures)
                        if record and not run.get("error"):
                            cassette.put(c["id"], rep, list(srv.replies))
                        if replay and miss["n"]:
                            run["error"] = run.get("error") or "the cassette ran out of replies"
                            miss["n"] = 0
                        facts = None
                        if run["command"] and not run.get("error"):
                            facts = command_facts(binary, c["shell"], run["command"], env)
                        run["checks"] = [] if run.get("error") else score(c, run["command"], run["output"], facts)
                        run["pass"] = not run.get("error") and all(k["ok"] for k in run["checks"])
                        runs.append(run)
                        mark = "ok  " if run["pass"] else "FAIL"
                        t = f"{run['llm_ms'] / 1000:.1f}s" if run.get("llm_ms") is not None else "-"
                        why = run.get("error") or "; ".join(k["detail"] for k in run["checks"] if not k["ok"])
                        print(f"   {mark} {c['id']:<24} {t:>7} {why[:90]}", file=sys.stderr)
                    entry["cases"].append({"id": c["id"], "category": c["category"], "action": c["action"],
                                           "shell": c["shell"], "runs": runs})
            if record and cassette:
                cassette.save({"model": m["model"], "trust": m.get("trust", "local"), "level": m.get("level", 1)})
    return out


class _Null:
    url = ""
    replies: list = []

    def __enter__(self):
        return self

    def __exit__(self, *a):
        pass


def cassette_models(directory: Path, only: list[str] | None) -> list[dict]:
    models = []
    for f in sorted(directory.glob("*.json")):
        d = json.loads(f.read_text())
        models.append({"name": f.stem, "model": d.get("model", f.stem), "preset": "ollama",
                       "trust": d.get("trust", "local"), "level": d.get("level", 1)})
    if only:
        models = [m for m in models if any(o in m["name"] for o in only)]
    if not models:
        raise SystemExit(f"no cassettes in {directory}")
    return models


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    src = ap.add_mutually_exclusive_group()
    src.add_argument("--models", help="a TOML file listing models (see models.example.toml)")
    src.add_argument("--ollama", action="store_true", help="every model the local Ollama has")
    src.add_argument("--replay", metavar="DIR", help="replay recorded answers instead of calling a model")
    ap.add_argument("--record", metavar="DIR", help="save each model's answers to DIR/<model>.json")
    ap.add_argument("--model", action="append", help="only models whose name contains this (repeatable)")
    ap.add_argument("--category", action="append", help="only this category (repeatable)")
    ap.add_argument("--case", action="append", help="only this case id (repeatable)")
    ap.add_argument("--repeats", type=int, default=1, help="runs per case (answers vary; default 1)")
    ap.add_argument("--timeout", type=float, default=120.0, help="seconds per run (default 120)")
    ap.add_argument("--tools", choices=["default", "none"], default="default",
                    help="`none` turns the man tool off, so every answer is one model call")
    ap.add_argument("--no-warmup", action="store_true", help="skip the unscored first request")
    ap.add_argument("--bin")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--label", help="name of this run (default: date)")
    ap.add_argument("--out")
    ap.add_argument("--markdown", help="also write the report here")
    ap.add_argument("--docs-page", action="store_true", help="add Starlight front matter to the report")
    ap.add_argument("--title", default="Model comparison")
    ap.add_argument("--list", action="store_true", help="list the cases and exit")
    ap.add_argument("--min-pass", type=float, help="exit 1 if any model passes a smaller share (0-1)")
    a = ap.parse_args()

    cases = load_cases()
    if a.category:
        cases = [c for c in cases if c["category"] in a.category]
    if a.case:
        cases = [c for c in cases if c["id"] in a.case]
    if a.list:
        for c in cases:
            print(f"{c['id']:<26} {c['category']:<10} {c['action']:<8} {c['shell']:<5} {c['input'][:60]}")
        return 0
    if not cases:
        raise SystemExit("no cases selected")

    if a.replay:
        models = cassette_models(Path(a.replay), a.model)
    else:
        if a.models:
            models = load_models(Path(a.models))
        elif a.ollama:
            models = ollama_models()
        else:
            raise SystemExit("choose models: --models FILE, --ollama or --replay DIR (see --help)")
        if a.model:
            models = [m for m in models if any(o in m["name"] for o in a.model)]
        if not models:
            raise SystemExit("no model matches")
    binary = lib.find_binary(a.bin, build=not a.no_build)
    if a.record:
        Path(a.record).mkdir(parents=True, exist_ok=True)

    started = time.time()
    results: list[dict] = []
    partial = False
    try:
        evaluate(models, cases, binary, a.repeats, a.timeout, a.tools,
                 Path(a.record) if a.record else None, Path(a.replay) if a.replay else None,
                 warmup=not a.no_warmup, out=results)
    except KeyboardInterrupt:
        partial = True
        print("interrupted: saving what finished", file=sys.stderr)
    # a model stopped half-way has fewer cases than the suite: say so rather than report a flattering rate
    results = [m for m in results if m["cases"]]
    result = {
        "kind": "evals", "schema": lib.SCHEMA, "label": a.label or time.strftime("%Y-%m-%d"),
        "date": time.strftime("%Y-%m-%d %H:%M:%S %z"), "duration_s": round(time.time() - started, 1),
        "mode": "replay" if a.replay else ("record" if a.record else "live"), "repeats": a.repeats,
        "partial": partial, "tools": a.tools, "host": lib.host_info(), "build": lib.build_info(binary), "models": results,
    }
    out = Path(a.out) if a.out else HERE / "results" / f"evals-{result['label']}.json"
    lib.write_json(out, result)
    print(f"results: {out}", file=sys.stderr)
    md = report.eval_report([result], a.title, a.docs_page)
    if a.markdown:
        Path(a.markdown).write_text(md)
    print(md)
    if a.min_pass is not None:
        low = [m["name"] for m in results if report.summarize_model(m)["rate"] < a.min_pass]
        if low:
            print(f"below {a.min_pass:.0%}: {', '.join(low)}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
