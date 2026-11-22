#!/usr/bin/env python3
"""Take profiles out of a report directory, so a bad measurement never reaches `publish`.

  isolate.py list   [REPORTS]                      what each run holds, per profile (errors are marked)
  isolate.py drop   RUN SELECTION                  delete the selected profiles from RUN (its answers and cassettes too)
  isolate.py move   RUN SELECTION --into DIR       move them to a new run directory, so nothing is lost
  isolate.py copy   RUN SELECTION --into DIR       copy them, RUN stays as it was

A profile is `generator:NAME`, `decider:NAME`, or a composed pair `generator:NAME+CHECKER`. SELECTION is any of:

  NAME...          a profile (`generator:haiku`) or a model name (`haiku`, `ms_decider`); `*` and `?` work.
                   A model name also selects the composed profiles that use it.
  --kind K         generator, decider or composed (a generator whose answers a checker judges)
  --errors         the profiles that have at least one run that ended in an error (no credit, a refused key, ...)

Several selections must all hold. At least one is required, so nothing is dropped by accident. `drop` and `move`
rewrite RUN's results, summary and review page, and delete the cassettes of what they took. Standard library only.
"""
import argparse
import fnmatch
import json
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import report


def kind_of(profile):
    kind, _, names = profile.partition(":")
    return "composed" if kind == "generator" and "+" in names else kind


def matches(profile, patterns):
    _, _, names = profile.partition(":")
    candidates = [profile, names, *names.split("+")]
    return any(fnmatch.fnmatchcase(c, p) for p in patterns for c in candidates)


def select(result, patterns=(), kind=None, errors=False):
    """The profiles of a result that satisfy every given selection. ValueError when none was given."""
    if not (patterns or kind or errors):
        raise ValueError("say what to select: a profile or model name, --kind, or --errors")
    profiles = list(dict.fromkeys(r["profile"] for r in result["runs"]))
    failing = {r["profile"] for r in result["runs"] if r.get("error") and not r.get("warmup")}
    return [p for p in profiles
            if (not patterns or matches(p, patterns)) and (not kind or kind_of(p) == kind) and (not errors or p in failing)]


def cassette_dir(directory, profile):
    return Path(directory) / "cassettes" / profile.replace(":", "-")


def load(run):
    path = Path(run) / "results.json"
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise ValueError(f"{run}: cannot read results.json ({error})")


def subset(result, profiles, keep):
    """The result with only the runs of `profiles` (keep=True) or without them (keep=False)."""
    chosen = set(profiles)
    return {**result, "runs": [r for r in result["runs"] if (r["profile"] in chosen) == keep]}


def split(run, profiles, into=None, remove=False):
    """Write the profiles to `into` (when given) and take them out of `run` (when `remove`). Returns the profiles done."""
    run = Path(run)
    result = load(run)
    if into:
        into = Path(into)
        if (into / "results.json").exists():
            raise ValueError(f"{into} already holds a run; pick a new directory")
        report.write(subset(result, profiles, True), into)
        for p in profiles:
            if cassette_dir(run, p).is_dir():
                shutil.copytree(cassette_dir(run, p), cassette_dir(into, p))
    if remove:
        report.write(subset(result, profiles, False), run)
        for p in profiles:
            shutil.rmtree(cassette_dir(run, p), ignore_errors=True)
    return profiles


def listing(reports):
    root = Path(reports)
    runs = [root] if (root / "results.json").exists() else sorted(p.parent for p in root.glob("*/results.json"))
    lines = []
    for run in runs:
        try:
            result = load(run)
        except ValueError as error:
            lines.append(str(error))
            continue
        lines.append(f"{run}  ({result.get('mode')}{', partial' if result.get('partial') else ''}, {result.get('date', '')[:10]})")
        for profile, rows in report.groups(result).items():
            errors = sum(1 for r in rows if r.get("error"))
            passed = sum(1 for r in rows if r.get("passed") and not r.get("error"))
            lines.append(f"  {profile:<40} {kind_of(profile):<9} {passed:>3}/{len(rows):<3} passed" + (f"  {errors} errors" if errors else ""))
    return lines


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)
    sub.add_parser("list").add_argument("reports", nargs="?", default="reports")
    for name in ("drop", "move", "copy"):
        p = sub.add_parser(name)
        p.add_argument("run", help="a report directory, e.g. reports/20261121-193525")
        p.add_argument("names", nargs="*", help="profiles or model names (globs allowed)")
        p.add_argument("--kind", choices=["generator", "decider", "composed"])
        p.add_argument("--errors", action="store_true", help="only profiles with at least one errored run")
        p.add_argument("--into", help="the new run directory (required for move and copy)")
        p.add_argument("--dry-run", action="store_true", help="say what would be selected, change nothing")
    args = ap.parse_args(argv)
    try:
        if args.command == "list":
            print("\n".join(listing(args.reports)))
            return 0
        if args.command in ("move", "copy") and not args.into:
            raise ValueError(f"{args.command} needs --into DIR")
        if args.command == "drop" and args.into:
            raise ValueError("drop deletes; use move --into DIR to keep them")
        chosen = select(load(args.run), args.names, args.kind, args.errors)
        if not chosen:
            raise ValueError("nothing matches that selection (see `isolate.py list`)")
        print(("would " if args.dry_run else "") + f"{args.command}: " + ", ".join(chosen))
        if not args.dry_run:
            split(args.run, chosen, args.into, remove=args.command in ("drop", "move"))
    except ValueError as error:
        print(f"isolate: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
