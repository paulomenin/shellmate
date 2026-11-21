#!/usr/bin/env python3
"""Keep the published results: one small JSON file per model setup, updated as new runs come in.

  publish.py add reports/20261120-141834 reports/20261121-090000      add runs from report directories
  publish.py status                                                    what is published, and what is out of date

The raw reports (answers, calls, review pages) stay in `reports/` and are not committed. An entry in `published/`
is what the documentation pages are made from, and it is committed.

An entry is identified by its kind (chat, decision or pair), the model and its reasoning setting. The provider is
kept as information only. Publishing runs for an entry that already exists:
- **pools** them when the experiment is the same: same suite, same shellmate binary, same model settings and prices.
  The sample grows, and a run that is already in the entry is never added twice;
- **replaces** the entry when anything else differs, because runs from two different experiments cannot be added up.
  The message says what differed.

Entries measured on an older suite are left out of the pages until they are run again. Standard library only.
"""
import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import report
import scorecard
import scoring

HERE = Path(__file__).resolve().parent
STORE = HERE / "published"
KIND = {"generator": "chat", "decider": "decision", "combined": "pair", "composed": "pair"}
PROFILE = {"chat": "generator", "decision": "decider", "pair": "combined"}  # the profile kind the pages group them under
SCHEMA = 1
TEXT = 200  # characters of an answer kept per run: enough to read a failure, not a transcript


def slug(text):
    return re.sub(r"[^a-z0-9.]+", "-", text.lower()).strip("-") or "model"


def model_identity(entry):
    return {"model": entry["model"], "reasoning": (entry.get("settings") or {}).get("reasoning_effort")}


def label(identity):
    """The text a page shows for an identity: the model ID, and its reasoning setting when one is set."""
    if "generator" in identity:
        return f"{label(identity['generator'])} + {label(identity['decider'])}"
    return identity["model"] + (f" [reasoning: {identity['reasoning']}]" if identity.get("reasoning") else "")


def file_slug(identity):
    if "generator" in identity:
        return f"{file_slug(identity['generator'])}--{file_slug(identity['decider'])}"
    return slug(identity["model"]) + (f"--reasoning-{slug(identity['reasoning'])}" if identity.get("reasoning") else "")


def trust_of(config, entry):
    return (config.get("providers", {}).get(entry.get("provider"), {})).get("trust", "unknown")


def describe(result, profile):
    """What a profile is: (kind, identity, where, metadata, fingerprint parts)."""
    config = result["configuration"]
    prefix, _, names = profile.partition(":")
    generator_name, _, checker_name = names.partition("+")
    parts = {}
    meta = {"providers": {}, "plugins": {}, "settings": {}, "pricing": {}}
    sides = []
    pair = prefix in ("combined", "composed")
    if prefix == "generator" or pair:
        sides.append(("generator", config["models"][generator_name]))
    if prefix == "decider":
        sides.append(("decider", config["deciders"][generator_name]))
    if pair:
        sides.append(("decider", config["deciders"][checker_name]))
    identity = {}
    where = []
    for role, entry in sides:
        identity[role] = model_identity(entry)
        where.append(trust_of(config, entry))
        meta["providers"][role] = entry.get("provider")
        meta["plugins"][role] = entry.get("plugin")
        meta["settings"][role] = entry.get("settings") or {}
        meta["pricing"][role] = entry.get("pricing") or {}
        # what changes the answers: the model, how it is called and what it is charged. A timeout does not.
        parts[role] = {"identity": identity[role], "plugin": entry.get("plugin"),
                       "settings": {k: v for k, v in (entry.get("settings") or {}).items() if k != "timeout_secs"},
                       "pricing": entry.get("pricing") or {}}
    if pair:
        # a pair judged on recorded answers and one run live are different experiments; so are two sets of recorded answers
        parts["method"] = "recorded" if prefix == "composed" else "live"
        if prefix == "composed":
            parts["answers_sha256"] = (result.get("answers") or {}).get(generator_name)
            meta["answers_from"] = {"model": generator_name, "sha256": parts["answers_sha256"]}
    else:
        identity = identity["generator" if prefix == "generator" else "decider"]
        parts = next(iter(parts.values()))
    where = "remote" if "remote" in where else "local" if where and set(where) == {"local"} else "unknown"
    return KIND[prefix], identity, where, meta, parts


def host_text(result):
    """One line for the machine: what the evaluator read, then what the config says (the GPU, for example)."""
    host = result.get("host") or {}
    parts = [host.get("cpu"), f"{host.get('cores')} threads" if host.get("cores") else None,
             f"{host.get('memory_gib')} GiB" if host.get("memory_gib") else None]
    text = ", ".join(p for p in parts if p)
    extra = (result.get("configuration", {}).get("run", {}).get("machine") or "").strip()
    return "; ".join(p for p in (extra, text) if p) or None


def compact(row):
    """A run as an entry keeps it: what the pages and the scorecard read, and a short excerpt of the answer."""
    keep = {k: row[k] for k in ("id", "case_id", "repeat", "category", "action", "shell", "passed", "error", "expect",
                                "behavior_expect", "decision_expected", "behavior_status", "decision", "decision_metrics",
                                "cost_usd", "attempts", "tool_case") if k in row and row[k] is not None}
    keep["checks"] = [{"name": c["name"], "ok": False, **({"severity": c["severity"]} if c.get("severity") else {})}
                      for c in row.get("checks", []) if not c["ok"]]  # only what failed
    if row.get("tool_calls"):
        keep["tool_calls"] = [c[:200] for c in row["tool_calls"][:8]]
    keep["command"] = (row.get("command") or "")[:TEXT]
    keep["output"] = (row.get("output") or "")[:TEXT]
    if row.get("timings"):
        keep["timings"] = {k: row["timings"][k] for k in ("llm_ms", "first_token_ms", "input_tokens", "output_tokens") if k in row["timings"]}
    if row.get("style"):
        keep["style"] = {k: row["style"][k] for k in ("shape", "prose_words") if k in row["style"]}
    keep["calls"] = [{k: c[k] for k in ("role", "wall_ms", "cost_usd", "error", "status", "replay") if k in c}
                     | ({"input_tokens": c["usage"].get("input_tokens"), "output_tokens": c["usage"].get("output_tokens")} if c.get("usage") else {})
                     for c in row.get("calls", [])]
    return keep


def fingerprint(result, kind, parts):
    env = result.get("environment") or {}
    whole = {"kind": kind, "suite": result["suite_hash"], "binary": env.get("binary_sha256") or env.get("binary_version"),
             "max_output_tokens": result["configuration"]["run"].get("max_output_tokens"), "model": parts}
    return hashlib.sha256(json.dumps(whole, sort_keys=True).encode()).hexdigest()[:16], whole


def differences(old, new):
    """What differs between two fingerprint parts, for the message."""
    out = []
    for key in sorted(set(old) | set(new)):
        if old.get(key) != new.get(key):
            out.append({"suite": "the suite of cases", "binary": "the shellmate binary", "max_output_tokens": "max_output_tokens",
                        "model": "the model settings or prices", "kind": "the kind"}.get(key, key))
    return out


def build_entry(result, profile, runs):
    kind, identity, where, meta, parts = describe(result, profile)
    env = result.get("environment") or {}
    fp, whole = fingerprint(result, kind, parts)
    date = result["date"]
    return {
        "schema": SCHEMA, "kind": kind, "identity": identity, "label": label(identity), "where": where,
        "suite_hash": result["suite_hash"], "fingerprint": fp, "fingerprint_parts": whole,
        "metadata": {**meta, "price_date": result["configuration"]["run"].get("price_date"),
                     "repeats": result["configuration"]["run"].get("repeats"), "machines": [m for m in [host_text(result)] if m]},
        "environment": {"binary_version": env.get("binary_version"), "binary_sha256": env.get("binary_sha256"),
                        "image_id": (result.get("image") or {}).get("id"), "packages": len((env.get("packages") or "").strip().splitlines())},
        "measured": {"first": date, "last": date},
        "evaluations": [{"id": result.get("evaluation_id"), "date": date, "source": result.get("_source"), "runs": len(runs)}],
        "runs": [compact(r) for r in runs],
    }


def finish(entry):
    entry["summary"] = scorecard.scorecard(entry["kind"], entry["runs"])
    return entry


def path_of(store, entry):
    return Path(store) / entry["kind"] / f"{file_slug(entry['identity'])}.json"


def write(path, entry):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(entry, indent=1, sort_keys=False) + "\n")


def merge_entry(old, new, force_replace=False):
    """(entry, message): pool when the experiment is the same, otherwise replace."""
    if old is None:
        return finish(new), "new"
    if old["fingerprint"] != new["fingerprint"] or force_replace:
        why = "forced" if force_replace and old["fingerprint"] == new["fingerprint"] else "changed: " + ", ".join(differences(old["fingerprint_parts"], new["fingerprint_parts"]))
        return finish(new), f"replaced ({why})"
    have = {r["id"] for r in old["runs"]}
    added = [r for r in new["runs"] if r["id"] not in have]
    if not added:
        return finish(old), "unchanged (these runs are already in it)"
    seen = {e["id"] for e in old["evaluations"]}
    old["runs"] += added
    old["evaluations"] += [e for e in new["evaluations"] if e["id"] not in seen]
    old["measured"]["last"] = max(old["measured"]["last"], new["measured"]["last"])
    old["measured"]["first"] = min(old["measured"]["first"], new["measured"]["first"])
    old["metadata"] = {**new["metadata"], "machines": sorted(set(old["metadata"].get("machines", [])) | set(new["metadata"].get("machines", [])))}
    old["environment"] = new["environment"]
    return finish(old), f"pooled (+{len(added)} runs, now {len(old['runs'])})"


def complete(result, profile, runs, suite, cases):
    """Whether a profile has every run it should: each of its cases, as many times as the repeats. A report that was
    interrupted still holds the models that finished; only the one that was running is incomplete. When the suite
    is not the current one the cases cannot be counted, so the report's own flag decides."""
    if result["suite_hash"] != suite:
        return not result.get("partial")
    wanted = sum(1 for c in cases if ("candidate" in c) == profile.startswith("decider:"))
    return len(runs) >= wanted * result["configuration"]["run"]["repeats"]


def add(paths, store, force_replace=False, allow_partial=False):
    """Publish the live runs of report directories. Returns [(path, message)] and notes about what was skipped."""
    import aggregate  # the same rules for finding and reading result files
    results, notes = aggregate.load(paths)
    messages = []
    suite = current_suite()
    cases = scoring.load_cases(HERE / "cases", HERE / "fixtures")
    for result in results:
        for profile, runs in report.groups(result).items():
            if not runs:
                continue
            if not allow_partial and not complete(result, profile, runs, suite, cases):
                notes.append(f"skipped {profile} of {result['_source']}: interrupted before all its runs were done (--allow-partial adds it anyway)")
                continue
            entry = build_entry(result, profile, runs)
            path = path_of(store, entry)
            old = json.loads(path.read_text()) if path.exists() else None
            merged, message = merge_entry(old, entry, force_replace)
            write(path, merged)
            messages.append((path, message))
    return messages, notes


def current_suite():
    c = HERE / "cases"
    return scoring.fingerprint(scoring.load_cases(c, HERE / "fixtures"), HERE / "fixtures")


def entries(store):
    return [(p, json.loads(p.read_text())) for p in sorted(Path(store).glob("*/*.json"))]


def status(store, suite):
    rows = []
    for path, e in entries(store):
        current = e["suite_hash"] == suite
        rows.append((e["kind"], e["where"], e["label"], e["summary"]["runs"], e["measured"]["last"][:10], "current" if current else "older suite: left out of the pages, run it again"))
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)
    a = sub.add_parser("add", help="publish the runs of report directories")
    a.add_argument("paths", nargs="+")
    a.add_argument("--into", type=Path, default=STORE)
    a.add_argument("--replace", action="store_true", help="replace an existing entry instead of pooling into it")
    a.add_argument("--allow-partial", action="store_true", help="also publish a run that was interrupted")
    s = sub.add_parser("status", help="list the published entries")
    s.add_argument("--into", type=Path, default=STORE)
    args = ap.parse_args()
    try:
        if args.command == "add":
            messages, notes = add(args.paths, args.into, args.replace, args.allow_partial)
            for n in notes:
                print(n, file=sys.stderr)
            for path, message in messages:
                print(f"{path.relative_to(args.into)}: {message}")
            if not messages:
                raise ValueError("nothing was published")
        else:
            rows = status(args.into, current_suite())
            if not rows:
                print("nothing is published yet")
            for kind, where, name, runs, date, state in rows:
                print(f"{kind:<9} {where:<7} {name:<60} {runs:>4} runs  {date}  {state}")
    except ValueError as error:
        print(f"publish: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
