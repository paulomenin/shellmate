#!/usr/bin/env python3
"""Turn a performance result into Markdown that can be pasted into the docs.

  report.py perf  results/perf-a.json [--baseline results/perf-b.json]

`--docs-page` adds the front matter a Starlight page needs, so the output can be saved straight into
website/src/content/docs/. Everything here is a pure function of the result files (no clock, no network), so
the same inputs always give the same page.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lib  # noqa: E402


# -------------------------------------------------------------------------------------------- formatting
def cell(s) -> str:
    return str(s).replace("|", "\\|").replace("\n", " ")


def table(header: list[str], rows: list[list], align: list[str] | None = None) -> str:
    align = align or ["l"] + ["r"] * (len(header) - 1)
    sep = ["---:" if a == "r" else ":---" for a in align]
    lines = ["| " + " | ".join(header) + " |", "| " + " | ".join(sep) + " |"]
    lines += ["| " + " | ".join(cell(c) for c in r) + " |" for r in rows]
    return "\n".join(lines) + "\n"


def ms(x, digits=1) -> str:
    if x is None:
        return "–"
    if x >= 10000:
        return f"{x / 1000:.1f} s"
    if x >= 1000:
        return f"{x:,.0f} ms".replace(",", " ")
    return f"{x:.{digits}f} ms"


def pct(x, digits=0) -> str:
    return "–" if x is None else f"{x * 100:.{digits}f}%"


def front_matter(title: str, description: str) -> str:
    return f"---\ntitle: {title}\ndescription: {description}\n---\n\n"


def environment(r: dict) -> str:
    h, b = r.get("host", {}), r.get("build", {})
    rows = [
        ["Date", r.get("date", "–")],
        ["Version", f"{b.get('version') or '–'} (commit {b.get('commit') or '–'}{', with local changes' if b.get('dirty') else ''})"],
        ["CPU", f"{h.get('cpu', '–')} ({h.get('cores', '?')} cores)"],
        ["Memory", f"{h.get('mem_gib', '?')} GiB"],
        ["OS", h.get("os", "–")],
        ["Binary size", f"{(b.get('binary_bytes') or 0) / 1024 / 1024:.1f} MiB" if b.get("binary_bytes") else "–"],
    ]
    return table(["", ""], rows, ["l", "l"])


# ------------------------------------------------------------------------------------------------- perf
def compare_perf(cur: dict, base: dict, max_regression: float, min_abs_ms: float) -> dict[str, dict]:
    """Per scenario: change of the median wall time against `base`. A change counts only when it is both
    larger than `max_regression` percent and larger than `min_abs_ms`, and it is a warning (not a failure)
    when either run was noisy (median absolute deviation above 20% of the median)."""
    out = {}
    old = {s["name"]: s for s in base.get("scenarios", [])}
    for s in cur["scenarios"]:
        o = old.get(s["name"])
        c, b = s["wall_ms"], (o or {}).get("wall_ms", {})
        if not o or not c.get("n") or not b.get("n"):
            continue
        delta = c["p50"] - b["p50"]
        rel = delta / b["p50"] if b["p50"] else 0.0
        noisy = c["noise"] > 0.2 or b["noise"] > 0.2
        regressed = rel * 100 > max_regression and delta > min_abs_ms
        out[s["name"]] = {"delta_ms": delta, "rel": rel, "noisy": noisy, "regressed": regressed,
                          "improved": rel * 100 < -max_regression and -delta > min_abs_ms}
    return out


def perf_report(cur: dict, baseline: dict | None = None, budgets: dict | None = None,
                max_regression: float = 15.0, min_abs_ms: float = 1.0,
                docs_page: bool = False) -> tuple[str, dict]:
    budgets = budgets or {}
    p50_budget = budgets.get("p50_ms", {})
    rss_budget = budgets.get("rss_mib", {})
    cmp = compare_perf(cur, baseline, max_regression, min_abs_ms) if baseline else {}
    scen = {s["name"]: s for s in cur["scenarios"]}
    floor = scen.get("floor", {}).get("wall_ms", {}).get("p50")

    problems, warnings = [], []
    for s in cur["scenarios"]:
        n, w = s["name"], s["wall_ms"]
        if s["failures"]:
            problems.append(f"`{n}`: {s['failures']} failed run(s): {s['failure_samples'][:1]}")
        if w.get("n") and n in p50_budget and w["p50"] > p50_budget[n]:
            problems.append(f"`{n}`: p50 {w['p50']:.1f} ms is over the {p50_budget[n]} ms budget")
        if s["rss_mib"].get("n") and n in rss_budget and s["rss_mib"]["p50"] > rss_budget[n]:
            problems.append(f"`{n}`: {s['rss_mib']['p50']:.0f} MiB of memory is over the {rss_budget[n]} MiB budget")
        c = cmp.get(n)
        if c and c["regressed"]:
            (warnings if c["noisy"] else problems).append(
                f"`{n}`: p50 {c['rel'] * 100:+.0f}% ({c['delta_ms']:+.1f} ms) against the baseline"
                + (" (noisy measurement)" if c["noisy"] else ""))
        elif w.get("n") and w["noise"] > 0.2:
            warnings.append(f"`{n}`: noisy (spread {w['noise'] * 100:.0f}% of the median); treat small differences with care")
    verdict = {"ok": not problems, "problems": problems, "warnings": warnings}

    md = []
    if docs_page:
        md.append(front_matter("Performance", f"How long shellmate itself takes, measured on {cur['host'].get('cpu', 'one machine')}."))
    else:
        md.append(f"# shellmate performance: {cur['label']}\n\n")
    md.append(
        "How long the shellmate process takes on its own: start-up, building a request, checking the answer and "
        "drawing it. The model is replaced by a fake that answers instantly, so none of this includes waiting for "
        f"a real model. Each scenario ran {cur['runs']} times after {cur['warmup']} warm-up rounds, "
        "interleaved so that drift on the machine affects all of them alike.\n\n")
    if baseline or budgets:
        md.append("## Verdict\n\n")
        md.append(("**Pass.** " if verdict["ok"] else "**Fail.** ") +
                  ("Everything is within its budget" + (f" and within {max_regression:.0f}% of `{baseline['label']}`" if baseline else "") + ".\n\n"
                   if verdict["ok"] else "\n\n"))
        for p in problems:
            md.append(f"- {p}\n")
        for w_ in warnings:
            md.append(f"- (warning) {w_}\n")
        md.append("\n")
    elif warnings:
        md.append("## Notes\n\n" + "".join(f"- {w_}\n" for w_ in warnings) + "\n")

    md.append("## Whole process, wall time\n\n")
    header = ["Scenario", "What it measures", "p50", "p95", "min", "max", "Above a bare process"]
    if baseline:
        header.append(f"p50 vs `{baseline['label']}`")
    if p50_budget:
        header.append("Budget")
    rows = []
    for s in cur["scenarios"]:
        w = s["wall_ms"]
        if not w.get("n"):
            rows.append([f"`{s['name']}`", s["desc"]] + ["failed"] * 5)
            continue
        net = "–" if s["name"] == "floor" or floor is None else ms(max(0.0, w["p50"] - floor))
        r = [f"`{s['name']}`", s["desc"], ms(w["p50"]), ms(w["p95"]), ms(w["min"]), ms(w["max"]), net]
        if baseline:
            c = cmp.get(s["name"])
            r.append("–" if not c else f"{c['rel'] * 100:+.0f}% ({c['delta_ms']:+.1f} ms)"
                     + (" ⚠" if c["regressed"] else "") + (" ✓" if c["improved"] else ""))
        if p50_budget:
            r.append(f"{p50_budget[s['name']]} ms" if s["name"] in p50_budget else "–")
        rows.append(r)
    md.append(table(header, rows, ["l", "l"] + ["r"] * (len(header) - 2)))
    md.append("\n\"Above a bare process\" subtracts the cost of starting `true` on this machine, which is what the "
              "operating system charges for any program.\n\n")

    stage_rows = []
    for s in cur["scenarios"]:
        st = s.get("stages_ms") or {}
        if st:
            g = lambda k: ms(st[k]["p50"]) if k in st else "–"  # noqa: E731
            stage_rows.append([f"`{s['name']}`", g("startup_ms"), g("context_ms"), g("llm_ms"), g("validate_ms"),
                               g("overhead_ms"), g("process_ms")])
    if stage_rows:
        md.append("## Where the time goes\n\nMedian per stage, from `invoke --timings=json`. \"Model\" is the fake, "
                  "so it is the cost of the HTTP exchange and streaming, not of thinking.\n\n")
        md.append(table(["Scenario", "Start-up", "Context", "Model (fake)", "Checks", "Overhead", "Total"], stage_rows))
        md.append("\n")

    mem = [[f"`{s['name']}`", f"{s['rss_mib']['p50']:.1f} MiB", f"{s['rss_mib']['max']:.1f} MiB"]
           for s in cur["scenarios"] if s["rss_mib"].get("n") and s["name"] != "floor"]
    if mem:
        md.append("## Peak memory\n\nResident memory of the process at its largest, sampled on a few extra runs.\n\n"
                  + table(["Scenario", "Typical", "Worst"], mem))
    else:
        md.append("## Peak memory\n\nNot measured (needs Linux /proc).\n")
    md.append("\n## Environment\n\n" + environment(cur))
    return "".join(md), verdict


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("perf")
    p.add_argument("result")
    p.add_argument("--baseline")
    p.add_argument("--max-regression", type=float, default=15.0)
    p.add_argument("--min-abs-ms", type=float, default=1.0)
    p.add_argument("--docs-page", action="store_true")
    p.add_argument("-o", "--output", help="write here instead of standard output")
    a = ap.parse_args()
    cur = json.loads(Path(a.result).read_text())
    base = json.loads(Path(a.baseline).read_text()) if a.baseline else None
    md, verdict = perf_report(cur, base, max_regression=a.max_regression, min_abs_ms=a.min_abs_ms,
                              docs_page=a.docs_page)
    code = 0 if verdict["ok"] else 1
    if a.output:
        Path(a.output).write_text(md)
    else:
        print(md)
    return code


if __name__ == "__main__":
    sys.exit(main())
