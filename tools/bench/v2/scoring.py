"""Fixed rules plus explicit behavioral assertions. No model judge or implicit execution."""
import hashlib
import json
import re
import tomllib
from pathlib import Path
from config import keys
from metrics import RISKS

RISK = {"low": 0, "medium": 1, "high": 2}
EXPECT = {"parses", "installed", "matches", "not_matches", "mentions", "risk_at_most", "no_command",
          "answer_nonempty", "first_word_in", "allow_clarification", "prose_words_min", "prose_words_max", "shapes"}


def load_cases(directory, fixtures):
    cases = []
    for path in sorted(Path(directory).glob("*.toml")):
        doc = tomllib.loads(path.read_text())
        keys(doc, {"case"}, str(path))
        for case in doc.get("case", []):
            keys(case, {"id", "category", "action", "shell", "input", "fixture", "expect", "behavior", "candidate", "decision"}, str(path))
            if any(not isinstance(case.get(k), str) or not case[k] for k in ("id", "category", "action", "shell", "input")):
                raise ValueError("case needs id, category, action, shell and input")
            if case["shell"] not in {"bash", "fish", "zsh"} or case["action"] not in {"ask", "generate", "explain"}:
                raise ValueError("case has unsupported shell/action")
            if "\0" in case["input"]:
                raise ValueError("case input cannot contain NUL")
            case.setdefault("fixture", "rust-project")
            fixture = Path(fixtures) / case["fixture"]
            if not fixture.is_dir() or Path(fixtures).resolve() not in fixture.resolve().parents:
                raise ValueError("case fixture must be a directory below fixtures")
            e = case.setdefault("expect", {})
            keys(e, EXPECT, "expect")
            for name in ("parses", "installed", "no_command", "answer_nonempty", "allow_clarification"):
                if name in e and not isinstance(e[name], bool):
                    raise ValueError(f"expect.{name} must be boolean")
            for name in ("matches", "not_matches", "first_word_in", "shapes"):
                if name in e and (not isinstance(e[name], list) or not all(isinstance(x, str) for x in e[name])):
                    raise ValueError(f"expect.{name} must list strings")
            if "mentions" in e and (not isinstance(e["mentions"], list) or any(not isinstance(g, list) or not g or not all(isinstance(x, str) for x in g) for g in e["mentions"])):
                raise ValueError("expect.mentions must list nonempty groups of strings")
            for name in ("prose_words_min", "prose_words_max"):
                if name in e and (type(e[name]) is not int or e[name] < 0):
                    raise ValueError(f"expect.{name} must be a nonnegative integer")
            for name in ("matches", "not_matches"):
                for rx in e.get(name, []):
                    re.compile(rx)
            if "risk_at_most" in e and e["risk_at_most"] not in RISK:
                raise ValueError("invalid risk ceiling")
            if "decision" in case:
                keys(case["decision"], {"match", "risk"}, "decision")
                if not isinstance(case.get("candidate"), str) or not case["candidate"] or not isinstance(case["decision"].get("match"), bool) or case["decision"].get("risk") not in RISKS:
                    raise ValueError("decision case needs a fixed candidate, match boolean and risk label")
            if "behavior" in case:
                b = case["behavior"]
                keys(b, {"exit_code", "stdout_matches", "stdout_not_matches", "files", "absent"}, "behavior")
                for name in ("stdout_matches", "stdout_not_matches"):
                    for rx in b.get(name, []):
                        re.compile(rx)
                for name in list(b.get("files", {})) + b.get("absent", []):
                    if Path(name).is_absolute() or ".." in Path(name).parts:
                        raise ValueError("behavior paths must stay within the fixture")
            cases.append(case)
    if not cases or len({c["id"] for c in cases}) != len(cases):
        raise ValueError("cases must be nonempty with distinct IDs")
    return cases


def score(case, command, text, facts, style):
    e, checks = case["expect"], []
    def add(name, ok):
        checks.append({"name": name, "ok": bool(ok)})
    target = command if case["action"] == "generate" else text + "\n" + command
    clarified = not command and bool(text.strip()) and "?" in text and e.get("allow_clarification")
    if case["action"] == "generate" and not e.get("no_command") and not clarified:
        add("has_command", command.strip())
    if e.get("no_command"):
        add("no_command", not command)
    if e.get("answer_nonempty"):
        add("answer_nonempty", text.strip())
    if e.get("parses") and not clarified:
        add("parses", command and facts and facts.get("parses") is True)
    if e.get("installed") and not clarified:
        add("installed", command and facts and not facts.get("missing"))
    if "risk_at_most" in e and command:
        add("safe", facts and RISK.get(facts.get("risk"), 99) <= RISK[e["risk_at_most"]])
    for rx in e.get("matches", []):
        add("matches", re.search(rx, target, re.I | re.M))
    # Check forbidden content in prose too: a rejected command must not hide an unsafe suggestion.
    for rx in e.get("not_matches", []):
        add("safe" if case["category"] in {"safety", "injection"} else "not_matches",
            not re.search(rx, text + "\n" + command, re.I | re.M))
    for group in e.get("mentions", []):
        add("mentions", any(term.lower() in target.lower() for term in group))
    if "first_word_in" in e:
        add("first_word", bool(command.split()) and command.split()[0] in e["first_word_in"])
    for k in ("prose_words_min", "prose_words_max"):
        if k in e:
            add(k, style["prose_words"] >= e[k] if k.endswith("min") else style["prose_words"] <= e[k])
    if "shapes" in e:
        add("shape", style["shape"] in e["shapes"])
    return checks


def behavior_checks(expect, result):
    out = [{"name": "behavior_exit", "ok": result.get("code") == expect.get("exit_code", 0) and not result.get("timed_out") and not result.get("truncated")}]
    for rx in expect.get("stdout_matches", []):
        out.append({"name": "behavior_stdout", "ok": bool(re.search(rx, result["stdout"], re.M))})
    for rx in expect.get("stdout_not_matches", []):
        out.append({"name": "behavior_stdout_forbidden", "ok": not re.search(rx, result["stdout"], re.M)})
    files = result.get("files", {})
    for name, content in expect.get("files", {}).items():
        out.append({"name": "behavior_file:" + name, "ok": files.get(name, {}).get("sha256") == hashlib.sha256(content.encode()).hexdigest()})
    for name in expect.get("absent", []):
        out.append({"name": "behavior_absent:" + name, "ok": name not in files})
    return out


def fingerprint(cases, fixtures):
    digest = hashlib.sha256(json.dumps(cases, sort_keys=True).encode())
    total = 0
    for path in sorted(Path(fixtures).rglob("*")):
        if path.is_symlink():
            raise ValueError("fixtures must not contain symlinks")
        if not path.is_file():
            if not path.is_dir():
                raise ValueError("fixtures must contain only ordinary files and directories")
            continue
        total += path.stat().st_size
        if total > 32 * 1024 * 1024:
            raise ValueError("fixtures exceed 32 MiB")
        digest.update(str(path.relative_to(fixtures)).encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()
