#!/usr/bin/env python3
"""Performance of the shellmate process itself: no model, no network beyond localhost.

Each scenario starts the real binary many times, interleaved round by round so that drift (thermal, other
load) hits all of them alike. `invoke-*` scenarios talk to an instant fake LLM and use `--timings=json`, so
the time is split into startup, context, the (fake) model, checks and the client's own overhead.

  perf.py                        run everything, write results/perf-<label>.json, print a Markdown report
  perf.py --quick                20 runs, for a smoke test
  perf.py --baseline old.json    also compare with an earlier run; exit 1 on a regression
"""
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lib  # noqa: E402
import report  # noqa: E402

HERE = Path(__file__).resolve().parent
REPLY = "Largest entries first.\n```shellmate-command\ndu -sh * | sort -rh | head -5\n```"
LONG_REPLY = ("word " * 400).strip() + "\n```shellmate-command\nls -la\n```"
STAGES = ["startup_ms", "context_ms", "llm_ms", "validate_ms", "overhead_ms", "process_ms"]


def write_config(path: Path, url: str, level: int = 2) -> None:
    path.write_text(
        f'version = 1\n[context]\ndefault_level = {level}\n'
        '[providers.fake]\ntrust = "local"\n'
        f'endpoint = "{url}"\n'
        '[models.fake]\nprovider = "fake"\nplugin = "builtin:openai-compatible"\ntool_calling = "text"\nmodel = "bench"\n'
        '[llm]\nchain = ["fake"]\n[audit]\nenabled = false\n[conversation]\nstore = "none"\n'
    )


def make_repo(root: Path) -> Path:
    """A small git repository with a project marker: what the collectors see on a real checkout."""
    repo = root / "repo"
    repo.mkdir()
    (repo / "Cargo.toml").write_text('[package]\nname = "x"\nversion = "0.1.0"\n')
    (repo / "src").mkdir()
    (repo / "src" / "main.rs").write_text("fn main() {}\n")
    if shutil.which("git"):
        env = {"GIT_AUTHOR_NAME": "b", "GIT_AUTHOR_EMAIL": "b@x", "GIT_COMMITTER_NAME": "b",
               "GIT_COMMITTER_EMAIL": "b@x", "PATH": "/usr/bin:/bin:/usr/local/bin", "HOME": str(root)}
        for cmd in (["init", "-q", "-b", "main"], ["add", "."], ["commit", "-q", "-m", "init"]):
            subprocess.run(["git", *cmd], cwd=repo, env=env, capture_output=True)
        (repo / "src" / "new.rs").write_text("// untracked\n")
    return repo


class Scenario:
    def __init__(self, name: str, desc: str, args: list[str], stdin: bytes = b"", cwd: str | None = None,
                 timings: bool = False, env_extra: dict | None = None):
        self.name, self.desc, self.args, self.stdin, self.cwd = name, desc, args, stdin, cwd
        self.timings, self.env_extra = timings, env_extra or {}
        self.walls: list[float] = []
        self.rss: list[float] = []
        self.stages: dict[str, list[float]] = {s: [] for s in STAGES}
        self.failures: list[str] = []

    def once(self, env: dict, record: bool) -> None:
        r = lib.run_binary(self.args, self.stdin, env={**env, **self.env_extra}, cwd=self.cwd, timeout=60)
        if not record:
            return
        if r.timed_out or r.code not in (0, 10):
            self.failures.append(f"exit {r.code}{' (timed out)' if r.timed_out else ''}: {r.stderr.strip()[:200]}")
            return
        self.walls.append(r.wall_ms)
        if self.timings:
            t = lib.parse_timings(r.stderr)
            if t is None:
                self.failures.append("no --timings line in the output")
                return
            for s in STAGES:
                if t.get(s) is not None:
                    self.stages[s].append(t[s])

    def sample_memory(self, env: dict) -> None:
        m = lib.peak_rss_mib(self.args, self.stdin, env={**env, **self.env_extra}, cwd=self.cwd)
        if m is not None:
            self.rss.append(m)

    def result(self) -> dict:
        return {
            "name": self.name, "desc": self.desc,
            "wall_ms": lib.stats(self.walls), "rss_mib": lib.stats(self.rss),
            "stages_ms": {s: lib.stats(v) for s, v in self.stages.items() if v},
            "failures": len(self.failures), "failure_samples": self.failures[:3],
        }


def build_scenarios(binary: Path, cfg: Path, repo: Path, fake_url: str, session: str) -> list[Scenario]:
    b, c = str(binary), str(cfg)
    ask = lib.envelope("ask", "find the biggest files", shell="bash", cwd=str(repo),
                       extra={"session": session, "status": "0"})
    gen = lib.envelope("generate", "find the biggest files", shell="bash", cwd=str(repo),
                       extra={"session": session})
    cap = lib.envelope("capture", "ls -la | sort", shell="bash", cwd=str(repo))
    true = shutil.which("true") or "/usr/bin/true"
    return [
        Scenario("floor", "the system's cost of starting a trivial process (`true`)", [true]),
        Scenario("version", "`shellmate --version`: binary load and argument parsing", [b, "--version"]),
        Scenario("config-check", "`config check`: load and validate the config, build providers", [b, "--config", c, "config", "check"]),
        Scenario("dry-run", "`invoke --dry-run`: config, context, redaction and the request payload", [b, "--config", c, "invoke", "--dry-run"], ask),
        Scenario("context", "`context`: the collectors (project files, git) in a small repository", [b, "--config", c, "context", "hello"], cwd=str(repo)),
        Scenario("check", "`check`: syntax check in bash, program lookup, risk", [b, "--config", c, "check", "--shell", "bash", "du -sh * | sort -rh | head -5"]),
        Scenario("capture", "`invoke` with the capture action: wrap the line, no model", [b, "--config", c, "invoke"], cap),
        Scenario("invoke-ask", "a whole `ask` against an instant fake model, level 2 context", [b, "--config", c, "invoke", "--timings=json"], ask, timings=True),
        Scenario("invoke-generate", "a whole `generate` against an instant fake model", [b, "--config", c, "invoke", "--timings=json"], gen, timings=True),
        Scenario("invoke-stream", "a long (2 KB) streamed answer in small pieces: the renderer under load", [b, "--config", str(cfg.with_name("long.toml")), "invoke", "--timings=json"], ask, timings=True),
        Scenario("run-wrap", "`run -- true`: the cost of capturing a command's output", [b, "run", "--", true], env_extra={"SHELLMATE_SESSION": session}),
    ]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", help="the shellmate binary (default: target/release/shellmate, built first)")
    ap.add_argument("--profile", default="release", choices=["release", "debug"])
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--runs", type=int, default=100, help="measured runs per scenario (default 100)")
    ap.add_argument("--warmup", type=int, default=5, help="unmeasured rounds first (default 5)")
    ap.add_argument("--quick", action="store_true", help="20 runs, 2 warmup rounds")
    ap.add_argument("--only", help="comma-separated scenario names")
    ap.add_argument("--label", help="name of this run (default: <version>-<commit>)")
    ap.add_argument("--out", help="result file (default: results/perf-<label>.json)")
    ap.add_argument("--markdown", help="also write the report here")
    ap.add_argument("--baseline", help="an earlier result file to compare with")
    ap.add_argument("--max-regression", type=float, default=15.0, help="percent on p50 (default 15)")
    ap.add_argument("--min-abs-ms", type=float, default=1.0, help="ignore changes smaller than this (default 1 ms)")
    ap.add_argument("--no-budgets", action="store_true", help="do not enforce budgets.toml")
    a = ap.parse_args()
    if a.quick:
        a.runs, a.warmup = 20, 2

    binary = lib.find_binary(a.bin, build=not a.no_build, profile=a.profile)
    host = lib.host_info()
    if host["load_1m"] and host["cores"] and host["load_1m"] > host["cores"] * 0.5:
        print(f"warning: load average is {host['load_1m']} on {host['cores']} cores; results will be noisy",
              file=sys.stderr)

    with tempfile.TemporaryDirectory(prefix="sm-perf-") as tmp:
        tmp = Path(tmp)
        home = tmp / "home"
        home.mkdir()
        env = lib.isolated_env(home)
        repo = make_repo(tmp)
        replies = {"normal": REPLY}
        with lib.FakeLLM(lambda body: replies["normal"]) as fake, \
                lib.FakeLLM(lambda body: LONG_REPLY, piece_chars=2) as long_fake:
            cfg = tmp / "config.toml"
            write_config(cfg, fake.url)
            write_config(tmp / "long.toml", long_fake.url)
            scenarios = build_scenarios(binary, cfg, repo, fake.url, "bench-session")
            if a.only:
                want = set(a.only.split(","))
                unknown = want - {s.name for s in scenarios}
                if unknown:
                    raise SystemExit(f"unknown scenario(s): {', '.join(sorted(unknown))}")
                scenarios = [s for s in scenarios if s.name in want or s.name == "floor"]
            total = a.warmup + a.runs
            t_start = time.time()
            for i in range(total):
                for s in scenarios:
                    s.once(env, record=i >= a.warmup)
                    # memory is sampled on a few extra runs through GNU time, never in the timed ones
                    if i >= a.warmup and (i - a.warmup) % max(1, a.runs // 5) == 0:
                        s.sample_memory(env)
                if (i + 1) % 10 == 0 or i + 1 == total:
                    print(f"  round {i + 1}/{total}", file=sys.stderr)

    build = lib.build_info(binary)
    label = a.label or f"{(build['version'] or 'shellmate').split()[-1]}-{build['commit'] or 'nogit'}"
    result = {
        "kind": "perf", "schema": lib.SCHEMA, "label": label,
        "date": time.strftime("%Y-%m-%d %H:%M:%S %z"), "duration_s": round(time.time() - t_start, 1),
        "runs": a.runs, "warmup": a.warmup, "profile": a.profile,
        "host": host, "build": build,
        "scenarios": [s.result() for s in scenarios],
    }
    out = Path(a.out) if a.out else HERE / "results" / f"perf-{label}.json"
    lib.write_json(out, result)
    print(f"results: {out}", file=sys.stderr)

    budgets = {} if a.no_budgets else tomllib.loads((HERE / "budgets.toml").read_text())
    baseline = None
    if a.baseline:
        import json
        baseline = json.loads(Path(a.baseline).read_text())
    md, verdict = report.perf_report(result, baseline=baseline, budgets=budgets,
                                     max_regression=a.max_regression, min_abs_ms=a.min_abs_ms)
    if a.markdown:
        Path(a.markdown).write_text(md)
    print(md)
    failed = any(s["failures"] for s in result["scenarios"])
    return 1 if (failed or not verdict["ok"]) else 0


if __name__ == "__main__":
    sys.exit(main())
