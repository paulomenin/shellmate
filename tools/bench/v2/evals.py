#!/usr/bin/env python3
"""Evaluate models through shellmate in fresh networkless containers. Standard library only."""
import argparse
import contextlib
import copy
import random
import hashlib
import json
import os
import shutil
import sys
import tempfile
import time
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import config
import metrics
import proxy
import report
import sandbox
import scoring

HERE = Path(__file__).resolve().parent


def model_config(generator, checker, providers):
    q = json.dumps
    text = ['version = 1', '[context]', 'default_level = 1', '[audit]', 'enabled = false',
            '[conversation]', 'store = "none"', '[tools.man]', 'enabled = false',
            '[tools.exec]', 'enabled = false', '[tools.read]', 'enabled = false']
    for role, m in (("generator", generator), ("checker", checker)):
        if not m:
            continue
        p = providers[m["provider"]]
        text += [f'[providers.{role}]', f'trust = {q(p["trust"])}', f'endpoint = "@BROKER@/{role}"',
                 'max_context_level = 1', f'[models.{role}]', f'provider = "{role}"',
                 f'plugin = "builtin:{m["plugin"]}"', f'model = {q(m["model"])}']
        text += [f'{k} = {q(v)}' for k, v in m["settings"].items()]
    text += ['[llm]', 'chain = ["generator"]', '[validator]', f'enabled = {str(bool(checker)).lower()}',
             'chain = ["checker"]' if checker else 'chain = []', 'timeout = "120s"']
    return "\n".join(text) + "\n"


def routes_for(generator, checker, c, secrets):
    routes = {}
    for role, m in (("generator", generator), ("checker", checker)):
        if m:
            p = c["providers"][m["provider"]]
            routes[role] = {"model": m["model"], "plugin": m["plugin"], "endpoint": p["endpoint"],
                            "key": secrets.get(m["provider"]), "pricing": m["pricing"],
                            "path": m["settings"].get("path", "/v1/systemone" if m["plugin"] == "systemone" else "/v1/chat/completions")}
    return routes


def matrix(c, cases):
    r = c["run"]
    if "generator" in r["modes"]:
        for name in r["models"]:
            yield f"generator:{name}", name, None, [x for x in cases if "candidate" not in x]
    if "decider" in r["modes"]:
        for name in r["deciders"]:
            yield f"decider:{name}", None, name, [x for x in cases if "candidate" in x]
    if "combined" in r["modes"]:
        for name in r["models"]:
            for decider in r["deciders"]:
                yield f"combined:{name}+{decider}", name, decider, [x for x in cases if "candidate" not in x]


def parse_output(result, replay=False):
    stderr = result.get("stderr", "")
    timing = {}
    lines = []
    for line in stderr.replace("\r", "\n").splitlines():
        if line.startswith('{"timings"'):
            try:
                timing = json.loads(line)["timings"]
            except (ValueError, KeyError):
                pass
        elif not line.strip().endswith("thinking…"):
            lines.append(line)
    return result.get("stdout", "").strip(), "\n".join(lines).strip(), {} if replay else timing


def cassette_reply(text):
    return json.dumps({"choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]})


SLEEP = time.sleep  # a test replaces it


def invoke_failure(worker):
    """Why shellmate's run in the container does not count as an answer, or None. Said plainly, because the
    four causes need four different fixes."""
    if worker.get("timed_out"):
        return "shellmate did not finish in time (run.timeout_secs)"
    if worker.get("truncated"):
        return "shellmate's output exceeded the size limit"
    if worker.get("code") not in (0, 10):
        return f"shellmate exited with status {worker.get('code')}"
    if worker.get("check_error"):
        return "the command check (shellmate check) gave no usable result"
    return None


def retry_wait(row, attempt, run):
    """Seconds to wait before running a case again, or None when the failure is not worth a retry.
    Only a provider that said "later" (rate limit, 5xx) or did not answer is retried; never a refusal,
    a budget limit, a bad answer or a local failure. The whole case runs again in a fresh container, so
    the timings of the answer that counts are never stretched by a wait."""
    calls = row.get("calls", [])
    if not row.get("error") or not any(x.get("retryable") for x in calls) or any(x.get("error") == "eval budget reached" for x in calls):
        return None
    asked = max((x.get("retry_after") or 0 for x in calls), default=0)
    backoff = run["retry_wait_secs"] * 2 ** attempt + random.uniform(0, 1)
    return min(max(asked, backoff), run["retry_max_wait_secs"])


def stop_reason(budget, profile, row):
    """Why the evaluation must stop for its limits, or None. Says which limit and what to change."""
    if budget.unknown:
        unpriced = [x for x in row.get("calls", []) if not x.get("replay") and x.get("cost_usd") is None and not x.get("error")]
        why = ("the reply had no token usage, so a price per million tokens cannot give a cost"
               if unpriced and unpriced[0].get("usage") is None else "its usage did not fit the configured prices")
        return (f"stopped: the cost of a {profile} call is unknown ({why}). "
                "Set `pricing = { per_request = ... }` for that model (or all-zero prices if it is free)")
    if budget.requests >= budget.maximum:
        return f"stopped: the request limit was reached ({budget.requests} of run.max_requests = {budget.maximum} counted requests); raise run.max_requests"
    if budget.spent > budget.dollars or budget.exhausted:
        return (f"stopped: the estimated cost limit was reached (${budget.spent:.4f} spent of run.max_cost_usd = ${budget.dollars:g}, "
                f"{budget.requests} requests); raise run.max_cost_usd")
    return None


def approved(mode, command, facts, row):
    """`True`: a person types yes. `"auto"`: no person, but never for a command the product rates high risk."""
    if mode != "auto":
        row["approval"] = "person"
        return sandbox.approve(command)
    row["approval"] = "auto"
    if not facts or facts.get("risk") not in ("low", "medium"):
        row["approval_note"] = "not auto-approved: the product rates this command high risk or could not rate it"
        return False
    return True


def evaluate(c, cases, image, secrets, replay, behavior, result, outdir, blind):
    r = c["run"]
    budget = proxy.Budget(r["max_requests"], r["max_cost_usd"], r["max_output_tokens"])
    # Freeze the fixture set once per evaluation; containers get only this copy.
    done = 0

    def flush():
        report.write(result, outdir, blind)
        if replay is None:
            write_recordings(result, outdir)

    # The reports are rewritten every `report_every` rows and, whatever happens, when the evaluation ends.
    with tempfile.TemporaryDirectory(prefix="sm-eval-v2-") as temp, contextlib.ExitStack() as stack:
        stack.callback(flush)
        tmp = Path(temp)
        fixtures = tmp / "fixtures"
        shutil.copytree(r["fixtures"], fixtures)
        if scoring.fingerprint(cases, fixtures) != result["suite_hash"]:
            raise ValueError("fixtures changed while preparing the run")
        session = None
        for profile, gen_name, check_name, selected in matrix(c, cases):
            if not selected:
                raise ValueError(f"no cases selected for {profile}")
            generator = c["models"].get(gen_name)
            checker = c["deciders"].get(check_name)
            # A fixed synthetic generator lets the real product build/redact/parse decider requests.
            if not generator:
                generator = {"provider": "_fixed", "model": "fixed-candidate", "plugin": "openai-compatible", "settings": {}, "pricing": {"per_request": 0}}
            local = {**c, "providers": {**c["providers"], "_fixed": {"trust": "local", "endpoint": "http://127.0.0.1"}}}
            broker_dir = tmp / "broker"
            broker_dir.mkdir(exist_ok=True)
            if session:
                session.close()
            session = stack.enter_context(contextlib.closing(sandbox.Session(image, fixtures, broker_dir))) if r["reuse_container"] else None

            def run_invoke(payload, session=session):
                return session.request(payload) if session else sandbox.run(image, fixtures, payload, broker_dir)
            scheduled = selected
            if r["warmup"] and replay is None:
                first = selected[0]
                scheduled = [{**first, "id": "warmup", "warmup": True}] + selected
            for case in scheduled:
                for repeat in range(1 if case.get("warmup") else r["repeats"]):
                    identifier = hashlib.sha256(f"{result.get('evaluation_id', '')}:{result['suite_hash']}:{profile}:{case['id']}:{repeat}".encode()).hexdigest()[:24]
                    row = {"id": identifier, "profile": profile, "case_id": case["id"], "repeat": repeat,
                           "category": case["category"], "action": case["action"], "shell": case["shell"], "input": case["input"],
                           "fixture": case["fixture"], "warmup": bool(case.get("warmup")), "passed": False, "checks": []}
                    row["expect"] = case["expect"]
                    if "behavior" in case:
                        row["behavior_expect"] = case["behavior"]
                    if "decision" in case:
                        row["decision_expected"] = case["decision"]
                    base = copy.deepcopy(row)
                    failed = []
                    for attempt in range(r["retries"] + 1 if replay is None else 1):
                        row = copy.deepcopy(base)
                        replay_routes = {}
                        row["generator_name"], row["checker_name"] = gen_name, check_name
                        if not gen_name:
                            replay_routes["generator"] = cassette_reply(f"```shellmate-command\n{case['candidate']}\n```")
                        if replay is not None:
                            key = check_name if not gen_name else gen_name
                            cassette = replay.get(key, {})
                            answers = cassette.get("cases", {}).get(case["id"], [])
                            if gen_name and answers:
                                replies = answers[repeat % len(answers)]
                                if not isinstance(replies, list) or len(replies) != 1 or not isinstance(replies[0], str):
                                    row["error"] = "cassette needs exactly one generator reply per case"
                                else:
                                    replay_routes["generator"] = cassette_reply(replies[0])
                            elif gen_name:
                                row["error"] = "case missing from generator cassette"
                            if checker:
                                answers = replay.get(check_name, {}).get("decisions", {}).get(case["id"], [])
                                if answers and isinstance(answers[repeat % len(answers)], str):
                                    replay_routes["checker"] = answers[repeat % len(answers)]
                        if not row.get("error"):
                            broker_dir = tmp / "broker"
                            broker_dir.mkdir(exist_ok=True)
                            socket_path = broker_dir / "api.sock"
                            broker = None
                            try:
                                with proxy.Broker(socket_path, routes_for(generator, checker, local, secrets), budget,
                                                  r["timeout_secs"], replay_routes, replay_only=replay is not None) as broker:
                                    worker = run_invoke({"mode": "invoke", "fixture": case["fixture"],
                                        "action": case["action"], "input": case["input"], "shell": case["shell"],
                                        "environment": "environment" not in result,
                                        "timeout": r["timeout_secs"], "config": model_config(generator, checker, local["providers"])})
                                row["calls"] = broker.calls
                                command, text, timing = parse_output(worker, replay is not None)
                                raw = "\n".join(proxy.reply_text(x.get("response", "")) for x in broker.calls if x["role"] == "generator")
                                # Keep notes available for review; score the model prose independently.
                                prose = "\n".join(x for x in text.splitlines() if not x.startswith(("check:", "note:")))
                                row.update({"command": command, "output": text, "raw_answer": raw,
                                            "timings": timing, "wall_ms": worker.get("wall_ms") if replay is None else None,
                                            "style": metrics.style(raw or prose, command, raw), "programs": worker.get("programs"), "facts": worker.get("facts")})
                                if worker.get("packages") is not None:
                                    result.setdefault("environment", {"packages": worker.get("packages"), "binary_version": worker.get("binary_version")})
                                reason = invoke_failure(worker)
                                if reason:
                                    row["error"] = reason
                                if any(x.get("error") for x in broker.calls):
                                    statuses = sorted({x["status"] for x in broker.calls if x.get("status")})
                                    if any(x.get("error") == "eval budget reached" for x in broker.calls):
                                        row["error"] = "eval budget reached"
                                    else:
                                        said = next((x["upstream_message"] for x in broker.calls if x.get("upstream_message")), "")
                                        row["error"] = "provider request failed" + (f" (HTTP {', '.join(map(str, statuses))}{': ' + said if said else ''})" if statuses else "")
                                priced = [x.get("cost_usd") for x in broker.calls if not x.get("replay")]
                                row["cost_usd"] = sum(priced) if priced and all(x is not None for x in priced) else None
                                if gen_name:
                                    row["checks"] = scoring.score(case, command, prose, worker.get("facts"), row["style"])
                                else:
                                    row["checks"] = [{"name": "fixed_candidate", "ok": command == case["candidate"]}]
                                if checker:
                                    calls = [x for x in broker.calls if x["role"] == "checker"]
                                    decision = metrics.parse_decision(calls[-1].get("response", ""), checker["plugin"]) if calls else None
                                    row["decision"] = decision
                                    if "decision" in case:
                                        row["decision_metrics"] = metrics.decision_metrics(decision, case["decision"])
                                        row["checks"].append({"name": "decision_correct", "ok": row["decision_metrics"]["correct"]})
                                    elif command:
                                        row["checks"].append({"name": "decision_covered", "ok": decision is not None})
                                    else:
                                        row["decision_status"] = "not_applicable_without_command"
                                if "behavior" in case and gen_name and not case.get("warmup"):
                                    if behavior and command and not row.get("error") and approved(behavior, command, worker.get("facts"), row):
                                        executed = sandbox.run(image, fixtures, {"mode": "execute", "fixture": case["fixture"],
                                            "shell": case["shell"], "command": command, "timeout": min(r["timeout_secs"], 10)})
                                        row["behavior"] = executed
                                        row["behavior_status"] = "checked"
                                        row["checks"] += scoring.behavior_checks(case["behavior"], executed)
                                    else:
                                        row["behavior_status"] = "not_approved"
                            except ValueError as error:
                                row["error"] = str(error)
                            finally:
                                if broker:
                                    row["calls"] = broker.calls
                                    charged = [x.get("cost_usd") for x in broker.calls if not x.get("replay")]
                                    row["cost_usd"] = sum(charged) if charged and all(x is not None for x in charged) else None
                                socket_path.unlink(missing_ok=True)
                        wait = retry_wait(row, attempt, r) if attempt < r["retries"] and replay is None else None
                        if wait is None:
                            break
                        failed.append({"error": row["error"], "status": sorted({x["status"] for x in row.get("calls", []) if x.get("status")})})
                        print(f"{profile} {case['id']} {repeat + 1}: {row['error']}; retry {attempt + 1}/{r['retries']} in {wait:.0f}s", flush=True)
                        SLEEP(wait)
                    if failed:
                        row["attempts"], row["failed_attempts"] = len(failed) + 1, failed
                    row["passed"] = not row.get("error") and bool(row["checks"]) and all(k["ok"] for k in row["checks"]) and row.get("behavior_status") != "not_approved"
                    row = config.scrub(row, secrets.values())
                    result["runs"].append(row)
                    done += 1
                    if done == 1 or done % r["report_every"] == 0:
                        flush()
                    print(f"{profile} {case['id']} {repeat + 1}: {'PASS' if row['passed'] else 'INCOMPLETE' if row.get('behavior_status') == 'not_approved' else 'FAIL'}"
                          + (f" ({row['error']})" if row.get("error") else ""), flush=True)
                    reason = stop_reason(budget, profile, row)
                    if reason:
                        raise ValueError(f"{reason}. Completed runs are saved")


def write_recordings(result, directory):
    """One cassette directory per profile avoids mixing checker replies for different generators."""
    recordings = {}
    for row in result["runs"]:
        if row.get("warmup") or row.get("error"):
            continue
        for call in row.get("calls", []):
            name = row.get("generator_name" if call["role"] == "generator" else "checker_name")
            if not name or "response" not in call:
                continue
            key = (row["profile"], name)
            cassette = recordings.setdefault(key, {"model": name, "cases": {}, "decisions": {}})
            group = "cases" if call["role"] == "generator" else "decisions"
            replies = cassette[group].setdefault(row["case_id"], [])
            while len(replies) <= row["repeat"]:
                replies.append(None)
            replies[row["repeat"]] = [proxy.reply_text(call["response"])] if group == "cases" else call["response"]
    for (profile, name), cassette in recordings.items():
        destination = Path(directory) / "cassettes" / profile.replace(":", "-")
        destination.mkdir(parents=True, exist_ok=True)
        (destination / f"{name}.json").write_text(json.dumps(cassette, indent=2) + "\n")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--config", default=str(HERE / "models.example.toml"))
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--plan", action="store_true", help="validate configuration and show counts; no Docker or model calls")
    ap.add_argument("--replay", type=Path, help="directory of generator/decider cassettes; never calls a provider")
    ap.add_argument("--behavior", action="store_true", help="ask at a real terminal before each behavioral execution")
    ap.add_argument("--auto-approve", action="store_true",
                    help="with --behavior: run behavioral commands in the container without asking, except ones rated high risk")
    ap.add_argument("--blind", action="store_true", help="hide identifying metadata in the HTML review")
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()
    try:
        if args.auto_approve and not args.behavior:
            raise ValueError("--auto-approve needs --behavior")
        c = config.load(args.config)
        cases = scoring.load_cases(c["run"]["cases"], c["run"]["fixtures"])
        suite_hash = scoring.fingerprint(cases, c["run"]["fixtures"])
        profiles = list(matrix(c, cases))
        if args.list:
            for case in cases:
                print(f"{case['id']:<30} {case['category']:<12} {case['shell']}")
            return 0
        planned = sum(len(cs) * c["run"]["repeats"] for _, _, _, cs in profiles)
        if args.plan:
            print(json.dumps({"suite_hash": suite_hash, "profiles": [{"name": n, "cases": len(cs)} for n, _, _, cs in profiles],
                              "runs": planned, "request_limit": c["run"]["max_requests"], "cost_limit_usd": c["run"]["max_cost_usd"],
                              "image": c["run"]["image"]}, indent=2))
            return 0
        if any(not cs for _, _, _, cs in profiles):
            raise ValueError("one or more evaluation modes have no applicable cases")
        # Results contain prompts/answers intentionally; unlike audit logs, keep the whole directory private.
        os.umask(0o077)
        outdir = args.out or HERE / "results" / time.strftime("%Y%m%d-%H%M%S")
        if outdir.exists():
            raise ValueError("output directory already exists; choose a new --out to preserve previous reviews")
        replay = {p.stem: json.loads(p.read_text()) for p in args.replay.glob("*.json")} if args.replay else None
        if args.replay and not replay:
            raise ValueError("no cassettes found")
        secrets = {} if replay is not None else config.credentials(c)
        image = sandbox.image_info(c["run"]["image"])
        result = {"schema": 2, "suite_hash": suite_hash, "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                  "evaluation_id": uuid.uuid4().hex,
                  "mode": "replay" if replay is not None else "live", "partial": True, "image": image,
                  "configuration": c, "harness_hash": hashlib.sha256(b"".join(p.read_bytes() for p in sorted(HERE.glob("*.py")))).hexdigest(),
                  "runs": []}
        result["fixtures"] = {str(p.relative_to(c["run"]["fixtures"])): p.read_bytes()[:8192].decode(errors="replace")
                              for p in sorted(Path(c["run"]["fixtures"]).rglob("*")) if p.is_file()}
        try:
            evaluate(c, cases, image["id"], secrets, replay, ("auto" if args.auto_approve else args.behavior), result, outdir, args.blind)
            result["partial"] = False
        finally:
            report.write(config.scrub(result, secrets.values()), outdir, args.blind)
            if replay is None:
                write_recordings(result, outdir)
        print(f"Review: {outdir / 'review.html'}")
        return 0
    except (ValueError, OSError, KeyboardInterrupt) as error:
        print(f"eval v2: {error if not isinstance(error, KeyboardInterrupt) else 'interrupted; completed runs saved'}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
