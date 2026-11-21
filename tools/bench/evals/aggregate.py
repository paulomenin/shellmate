#!/usr/bin/env python3
"""Read results (raw reports or published entries) and write them as documentation pages.

  aggregate.py reports/*                                   explore: one page with everything, printed
  aggregate.py --published published --pages-dir ../../../website/src/content/docs/docs/benchmarks
                                                           the docs: overview, chat, decision and composed pages

Only live runs count (replays say nothing about a model). Every input must have run the same suite, or the
numbers would not be comparable; runs of the same profile from several directories are added together.
Local and hosted models are reported apart, and every table lists the most recommended model first.
Standard library only.
"""
import argparse
import json
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pages
import report
import scorecard
from scorecard import TIER_RANK, TIERS, decision_returned, rate_summary, tokens_per_second  # noqa: F401  (kept importable here)



def load(paths):
    """Result files from files or directories, in a stable order. Returns (results, notes)."""
    files = []
    for p in map(Path, paths):
        files += sorted(p.glob("*/results.json")) if p.is_dir() and not (p / "results.json").exists() else [p / "results.json" if p.is_dir() else p]
    results, notes = [], []
    for f in files:
        try:
            r = json.loads(f.read_text())
        except (OSError, ValueError):
            notes.append(f"skipped {f}: not readable")
            continue
        if r.get("mode") == "replay":
            notes.append(f"skipped {f.parent.name}: a replay, not a model measurement")
        elif r.get("schema") != 1:
            notes.append(f"skipped {f.parent.name}: not a result")
        else:
            r["_source"] = f.parent.name
            results.append(r)
    return results, notes


def display(result, profile):
    """(label, where) of a profile. `generator:local` is a config alias; the model ID, and its reasoning setting
    if one is set, is what a reader needs. `where` is local, remote or unknown, from the trust of its provider;
    a pair is remote when either side is."""
    cfg = result["configuration"]
    kind, _, names = profile.partition(":")
    gen, _, chk = names.partition("+")

    def model(group, n):
        entry = cfg[group].get(n, {})
        effort = (entry.get("settings") or {}).get("reasoning_effort")
        trust = (cfg.get("providers", {}).get(entry.get("provider"), {})).get("trust", "unknown")
        # The same model with another reasoning setting, or with its tools offered another way, is another setup, so it
        # is a separate row.
        return (entry.get("model", n) + (f" [reasoning: {effort}]" if effort else "")
                + (" [native tools]" if entry.get("tool_calling") == "native" else "")), trust

    if kind == "generator":
        return model("models", gen)
    if kind == "decider":
        return model("deciders", gen)
    (a, ta), (b, tb) = model("models", gen), model("deciders", chk)
    where = "remote" if "remote" in (ta, tb) else "local" if ta == tb == "local" else "unknown"
    return f"{a} + {b}", where


def load_published(store, suite):
    """Entries of a published store as (results-like objects, profiles, notes). An entry measured on another suite
    than `suite` is left out: its numbers would not compare with the rest."""
    import publish
    results, profiles, notes = [], {}, []
    for path, e in publish.entries(store):
        if e["suite_hash"] != suite:
            notes.append(f"left out {e['label']} ({e['kind']}): measured on an older suite, run it again")
            continue
        kind = publish.PROFILE[e["kind"]]
        key = (kind, e["label"], e["where"])
        profiles[key] = {"runs": e["runs"], "sources": {x["id"] or x["date"] for x in e["evaluations"]},
                         "measured": e["measured"]["last"][:10], "machines": e["metadata"].get("machines", [])}
        env = e["environment"]
        results.append({"date": e["measured"]["last"], "suite_hash": e["suite_hash"], "partial": False, "_source": e["label"],
                        "_evaluations": len(e["evaluations"]), "_machines": e["metadata"].get("machines", []),
                        "configuration": {"run": {"repeats": e["metadata"].get("repeats"), "price_date": e["metadata"].get("price_date")}},
                        "environment": {"binary_version": env.get("binary_version"), "packages": "p\n" * (env.get("packages") or 0)},
                        "image": {"id": env.get("image_id") or "–"}})
    return results, profiles, notes


def merge(results, allow_mixed=False):
    suites = {r["suite_hash"] for r in results}
    if len(suites) > 1 and not allow_mixed:
        raise ValueError(f"results come from {len(suites)} different suites ({', '.join(s[:8] for s in sorted(suites))}); "
                         "numbers would not compare. Pass --allow-mixed-suites to merge anyway")
    profiles = {}
    for r in results:
        for profile, runs in report.groups(r).items():
            label, where = display(r, profile)
            kind = profile.split(":")[0]
            entry = profiles.setdefault(("combined" if kind == "composed" else kind, label, where), {"runs": [], "sources": set()})
            entry["runs"] += runs
            entry["sources"].add(r["_source"])
    return profiles


def build(results, profiles, title, docs_page, machine):
    """Everything on one page (see pages.single)."""
    return pages.single(results, profiles, title, docs_page, machine)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("paths", nargs="*", help="result directories (or a parent such as reports/)")
    ap.add_argument("--published", type=Path, help="build the page from a published store (see publish.py) instead of raw reports")
    ap.add_argument("--title", default="Models")
    ap.add_argument("--docs-page", action="store_true", help="add the Starlight front matter")
    ap.add_argument("--pages-dir", type=Path, help="write the overview, chat, decision and composed pages into this directory (the docs)")
    ap.add_argument("--machine", help="the hardware the models ran on, free text (results do not record it)")
    ap.add_argument("--allow-mixed-suites", action="store_true")
    ap.add_argument("-o", "--out", type=Path)
    args = ap.parse_args()
    try:
        if args.published:
            import publish
            results, profiles, notes = load_published(args.published, publish.current_suite())
        elif args.paths:
            results, notes = load(args.paths)
            profiles = merge(results, args.allow_mixed_suites) if results else {}
        else:
            raise ValueError("give result directories, or --published DIR")
        for n in notes:
            print(n, file=sys.stderr)
        if args.pages_dir:
            # the docs always have all four pages, even before anything is published
            written = pages.pages(results, profiles, args.machine, stubs=True)
            args.pages_dir.mkdir(parents=True, exist_ok=True)
            for slug, text in written.items():
                (args.pages_dir / f"{slug}.md").write_text(text)
                print(f"wrote {args.pages_dir / (slug + '.md')}")
            return 0
        if not results:
            raise ValueError("nothing is published for the current suite yet (make publish, then make published-status)"
                             if args.published else "no live results found")
        text = build(results, profiles, args.title, args.docs_page, args.machine)
    except ValueError as error:
        print(f"aggregate: {error}", file=sys.stderr)
        return 1
    if args.out:
        args.out.write_text(text)
        print(f"wrote {args.out}")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
