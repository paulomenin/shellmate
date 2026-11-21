#!/usr/bin/env python3
"""Write the documentation page that describes every evaluation case, check and error in plain words.

  cases_page.py -o ../../../website/src/content/docs/docs/benchmarks/cases.md

The case list is read from cases/*.toml, so the page cannot drift from the suite. The results page links to
a case here by its id (`cases/#<id>`). Standard library only.
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import explain
import scoring

HERE = Path(__file__).resolve().parent
ORDER = ["generate", "ask", "explain", "dialect", "clarify", "safety", "injection", "tool-format", "tool-choice", "tool-use", "tool-safety", "behavior", "decider"]


def cell(text):
    return str(text).replace("\n", " ").replace("|", "\\|")


def table(header, rows):
    out = ["| " + " | ".join(header) + " |", "| " + " | ".join(":---" for _ in header) + " |"]
    return "\n".join(out + ["| " + " | ".join(cell(c) for c in row) + " |" for row in rows]) + "\n"


def anchor(case_id):
    return f'<a id="{case_id}"></a>`{case_id}`'


def build(cases):
    by = {}
    for c in cases:
        by.setdefault(c["category"], []).append(c)
    md = ['---\ntitle: "Evaluation cases"\ndescription: "What the evaluator asks, how an answer is judged, and what each failure in the results means."\n---\n\n']
    md.append(
        "The [results](../results/) come from a fixed set of requests. Each one is a **case**: a request, the shell it is for, "
        "and the rules a correct answer must satisfy. A model answers every case several times (the **repeats**); one answer is a **run**. "
        "A run passes only if every rule of its case holds. No model judges another model: the rules are fixed and written below, "
        "so a score is repeatable. They also measure only what they look for, not how good a model is in general.\n\n")
    md.append("This page is generated from the case files in `tools/bench/evals/cases`, so it lists exactly what was run.\n\n")

    md.append("## Categories\n\n")
    rows = [[f"**{c}**", explain.CATEGORIES.get(c, ""), len(by[c])] for c in ORDER if c in by]
    rows += [[f"**{c}**", explain.CATEGORIES.get(c, ""), len(v)] for c, v in by.items() if c not in ORDER]
    md.append(table(["Category", "What it tests", "Cases"], rows))

    md.append("\n## What a failed check means\n\nIn the results, a failed run lists the checks that did not hold. These are all of them.\n\n")
    rows = [[f"`{name}`", phrase, meaning] for name, (phrase, meaning) in explain.CHECKS.items()]
    md.append(table(["Check", "What it says", "Details"], rows))

    md.append("\n## What an error means\n\nA run ends in an **error**, not a failed check, when no answer could be scored. "
              "Errors are counted separately, and a model whose runs end in an error more than 5% of the time cannot be rated above Unsupported. "
              "A failed run keeps the reason in the \"What went wrong\" section of the results.\n\n")
    rows = [[f"`{prefix}…`", meaning] for prefix, meaning in explain.ERRORS]
    md.append(table(["Error starts with", "What it means"], rows))

    md.append("\n## Every case\n\n"
              "The first column is the id used in the results. Patterns are regular expressions, shown as written in the case.\n\n")
    for category in ORDER + [c for c in by if c not in ORDER]:
        if category not in by:
            continue
        md.append(f"### {category}\n\n{explain.CATEGORIES.get(category, '')}\n\n")
        rows = []
        for c in by[category]:
            wanted = explain.expectation(c["expect"], category, c["action"], c["shell"], c.get("behavior"), c.get("decision"), c.get("candidate"))
            rows.append([anchor(c["id"]), c["shell"], explain.code(c["input"]), "<br>".join(wanted) or "any answer"])
        md.append(table(["Case", "Shell", "Request", "A correct answer"], rows))
        md.append("\n")
    return "".join(md)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cases", default=str(HERE / "cases"))
    ap.add_argument("--fixtures", default=str(HERE / "fixtures"))
    ap.add_argument("-o", "--out", type=Path)
    args = ap.parse_args()
    try:
        text = build(scoring.load_cases(args.cases, args.fixtures))
    except ValueError as error:
        print(f"cases_page: {error}", file=sys.stderr)
        return 1
    if args.out:
        args.out.write_text(text)
        print(f"wrote {args.out}")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
