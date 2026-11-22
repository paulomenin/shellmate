"""Strict eval-only configuration. Never serialize resolved credentials."""
import getpass
import math
import os
import re
import stat
import tomllib
from datetime import date
from pathlib import Path
from urllib.parse import urlsplit

NAME = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9_.-]*$")
SETTINGS = {"reasoning_effort", "timeout_secs", "path"}


def keys(value, allowed, where):
    if not isinstance(value, dict) or set(value) - set(allowed):
        raise ValueError(f"{where}: expected a table with keys {', '.join(sorted(allowed))}")


def positive(value, where, integer=False):
    if isinstance(value, bool) or not isinstance(value, int if integer else (int, float)) or not math.isfinite(value) or value <= 0:
        raise ValueError(f"{where}: expected a positive {'integer' if integer else 'number'}")


def endpoint(value):
    if not isinstance(value, str):
        raise ValueError("provider endpoint must be a URL string")
    u = urlsplit(value)
    if u.username or u.password or u.query or u.fragment or not u.hostname:
        raise ValueError("provider endpoint must be a base URL without credentials, query or fragment")
    if u.scheme != "https" and not (u.scheme == "http" and u.hostname in {"localhost", "127.0.0.1", "::1"}):
        raise ValueError("provider endpoint needs HTTPS, except on loopback")


def load(path):
    path = Path(path).resolve()
    c = tomllib.loads(path.read_text())
    keys(c, {"version", "run", "providers", "models", "deciders"}, "config")
    if c.get("version") != 2:
        raise ValueError("config version must be 2")
    r = c.setdefault("run", {})
    keys(r, {"image", "repeats", "timeout_secs", "max_requests", "max_cost_usd", "max_output_tokens",
             "price_date", "cases", "fixtures", "models", "deciders", "modes", "warmup",
             "retries", "retry_wait_secs", "retry_max_wait_secs", "reuse_container", "report_every", "machine", "answers_from"}, "run")
    for k, default in {"image": "shellmate-evals", "repeats": 3, "timeout_secs": 120,
                       "max_requests": 1000, "max_output_tokens": 4096, "max_cost_usd": 1.0,
                       "modes": ["generator"], "deciders": [], "warmup": True,
                       "retries": 3, "retry_wait_secs": 5, "retry_max_wait_secs": 120,
                       "reuse_container": True, "report_every": 10}.items():
        r.setdefault(k, default)
    if "machine" in r and (not isinstance(r["machine"], str) or not r["machine"].strip() or len(r["machine"]) > 200):
        raise ValueError("run.machine must be a short text describing the hardware, e.g. \"RTX 4070 12 GB\"")
    for flag in ("warmup", "reuse_container"):
        if not isinstance(r[flag], bool):
            raise ValueError(f"run.{flag} must be a boolean")
    if "price_date" in r:
        if not isinstance(r["price_date"], str):
            raise ValueError('run.price_date must be a quoted YYYY-MM-DD string')
        date.fromisoformat(r["price_date"])
    for k in ("repeats", "max_requests", "max_output_tokens", "report_every"):
        positive(r[k], f"run.{k}", True)
    for k in ("timeout_secs", "max_cost_usd", "retry_wait_secs", "retry_max_wait_secs"):
        positive(r[k], f"run.{k}")
    if isinstance(r["retries"], bool) or not isinstance(r["retries"], int) or not 0 <= r["retries"] <= 20:
        raise ValueError("run.retries: expected an integer from 0 to 20")
    if not isinstance(r["image"], str) or not r["image"] or r["image"].startswith("-"):
        raise ValueError("run.image must name an existing Docker image")
    for k, default in (("cases", "cases"), ("fixtures", "fixtures")):
        if not isinstance(r.get(k, default), str):
            raise ValueError(f"run.{k} must be a path string")
        r[k] = str((path.parent / r.get(k, default)).resolve())
    providers = c.setdefault("providers", {})
    if not isinstance(providers, dict):
        raise ValueError("providers must be a table")
    for name, p in providers.items():
        if not NAME.fullmatch(name):
            raise ValueError("provider names must be simple identifiers")
        keys(p, {"endpoint", "trust", "api_key"}, f"providers.{name}")
        endpoint(p.get("endpoint", ""))
        if not isinstance(p.get("trust"), str) or p["trust"] not in {"local", "remote"}:
            raise ValueError(f"providers.{name}.trust must be local or remote")
        if p["trust"] == "local" and urlsplit(p["endpoint"]).hostname not in {"localhost", "127.0.0.1", "::1"}:
            raise ValueError("local providers must use loopback; otherwise use trust = remote")
        if "api_key" in p:
            ref = p["api_key"]
            if isinstance(ref, str) and ref.startswith("file:") and len(ref) > 5:
                # Relative to this config file, like the other paths. Read only when the key is needed.
                p["api_key"] = "file:" + str((path.parent / Path(ref[5:]).expanduser()).resolve())
            elif not isinstance(ref, str) or not re.fullmatch(r"(env|prompt):[A-Za-z_][A-Za-z0-9_]*", ref):
                raise ValueError("eval keys must be env:NAME, prompt:NAME or file:PATH references")
    for group in ("models", "deciders"):
        entries = c.setdefault(group, {})
        if not isinstance(entries, dict):
            raise ValueError(f"{group} must be a table")
        for name, m in entries.items():
            if not NAME.fullmatch(name):
                raise ValueError("model names must be simple identifiers")
            keys(m, {"provider", "model", "plugin", "settings", "pricing"} | ({"answers", "tool_calling"} if group == "models" else set()), f"{group}.{name}")
            if group == "models":
                # How the model is offered the tools. Not `auto`: asking the server is not something a run can repeat.
                if m.setdefault("tool_calling", "text") not in ("text", "native"):
                    raise ValueError(f"models.{name}.tool_calling must be text or native")
            if "answers" in m:
                # recorded answers of this model (a cassette from an earlier run): see the composed mode
                if not isinstance(m["answers"], str) or not m["answers"].strip():
                    raise ValueError(f"{group}.{name}.answers must be the path of a recorded cassette")
                m["answers"] = str((path.parent / Path(m["answers"]).expanduser()).resolve())
            if not isinstance(m.get("provider"), str) or m["provider"] not in providers or not isinstance(m.get("model"), str) or not m["model"]:
                raise ValueError(f"{group}.{name} needs an existing provider and model ID")
            m.setdefault("plugin", "openai-compatible")
            if not isinstance(m["plugin"], str) or m["plugin"] not in ({"openai-compatible"} if group == "models" else {"openai-compatible", "systemone"}):
                raise ValueError(f"unsupported {group} plugin")
            settings = m.setdefault("settings", {})
            keys(settings, SETTINGS, "settings")
            if "timeout_secs" in settings:
                positive(settings["timeout_secs"], "settings.timeout_secs", True)
            if "reasoning_effort" in settings and (m["plugin"] != "openai-compatible" or not isinstance(settings["reasoning_effort"], str) or settings["reasoning_effort"] not in {"none", "minimal", "low", "medium", "high", "xhigh"}):
                raise ValueError("invalid reasoning_effort for plugin")
            if "path" in settings:
                v = settings["path"]
                if m["plugin"] != "systemone" or not isinstance(v, str) or not v.startswith("/") or any(x in v for x in ("..", "//", "?", "#", "\\")) or any(x.isspace() or ord(x) < 32 for x in v):
                    raise ValueError("settings.path must be a plain System One API path")
            pricing = m.setdefault("pricing", {})
            keys(pricing, {"input_per_million", "output_per_million", "cached_input_per_million", "per_request"}, "pricing")
            for rate in pricing.values():
                if isinstance(rate, bool) or not isinstance(rate, (int, float)) or not math.isfinite(rate) or rate < 0:
                    raise ValueError("prices must be finite nonnegative numbers")
    r.setdefault("models", list(c["models"]))
    for group in ("models", "deciders"):
        selected = r[group]
        if not isinstance(selected, list) or any(not isinstance(x, str) or x not in c[group] for x in selected) or len(selected) != len(set(selected)):
            raise ValueError(f"run.{group} must list distinct configured names")
    if not isinstance(r["modes"], list) or not r["modes"] or any(not isinstance(x, str) or x not in {"generator", "decider", "combined", "composed"} for x in r["modes"]) or len(set(r["modes"])) != len(r["modes"]):
        raise ValueError("run.modes: choose generator, decider, combined and/or composed")
    if any(x in r["modes"] for x in ("generator", "combined")) and not r["models"]:
        raise ValueError("generator/combined modes need models")
    if any(x in r["modes"] for x in ("decider", "combined", "composed")) and not r["deciders"]:
        raise ValueError("decider/combined/composed modes need deciders")
    if "answers_from" in r:
        # a finished run: each model of run.models that has a cassette in it is replayed from there, unless it names its own
        if not isinstance(r["answers_from"], str) or not r["answers_from"].strip():
            raise ValueError("run.answers_from must be the path of a report directory")
        r["answers_from"] = str((path.parent / Path(r["answers_from"]).expanduser()).resolve())
        if not (Path(r["answers_from"]) / "cassettes").is_dir():
            raise ValueError("run.answers_from: no cassettes directory in it")
        for n in r["models"]:
            found = Path(r["answers_from"]) / "cassettes" / f"generator-{n}" / f"{n}.json"
            if "answers" not in c["models"][n] and found.is_file():
                c["models"][n]["answers"] = str(found)
    if "composed" in r["modes"]:
        recorded = [n for n in r["models"] if "answers" in c["models"][n]]
        if not recorded:
            raise ValueError("composed mode needs a model in run.models with `answers = \"path/to/cassette.json\"`, or run.answers_from = a report directory that holds its cassette")
        for n in recorded:
            if not Path(c["models"][n]["answers"]).is_file():
                raise ValueError(f"models.{n}.answers: no such file")
    return c


def read_key_file(path):
    """A key file must be a private regular file (as shellmate's own `file:` secrets), small, one line."""
    try:
        with open(path, "rb") as f:
            info = os.fstat(f.fileno())
            if not stat.S_ISREG(info.st_mode):
                raise ValueError("eval key file must be a regular file")
            if info.st_mode & 0o077:
                raise ValueError(f"eval key file {Path(path).name} must not be readable by group or others (chmod 600)")
            data = f.read(4097)
    except OSError:
        raise ValueError(f"cannot read eval key file {Path(path).name}") from None
    if len(data) > 4096:
        raise ValueError("eval key file is too large")
    return data.decode(errors="replace").strip()


def credentials(c, environ=None, prompt=getpass.getpass):
    """Resolve only providers needed by selected models. No keys enter containers."""
    environ = os.environ if environ is None else environ
    active = []
    # the generator of a composed profile is replayed from a file, so only generator/combined need its provider
    if any(m in c["run"]["modes"] for m in ("generator", "combined")):
        active.append("models")
    if any(m in c["run"]["modes"] for m in ("decider", "combined", "composed")):
        active.append("deciders")
    names = {c[g][n]["provider"] for g in active for n in c["run"][g]}
    out, resolved = {}, {}
    for name in sorted(names):
        ref = c["providers"][name].get("api_key")
        if not ref:
            out[name] = None
            continue
        scheme, var = ref.split(":", 1)
        if ref not in resolved:
            if scheme == "file":
                value, var = read_key_file(var), Path(var).name
            else:
                value = environ.get(var) if scheme == "env" else prompt(f"Eval key {var}: ")
            if not value or any(ord(x) < 32 or ord(x) == 127 for x in value):
                raise ValueError(f"missing or invalid eval credential {var}")
            resolved[ref] = value
        out[name] = resolved[ref]
    return out


def scrub(value, secrets):
    """Defense against an upstream echoing a credential in its response."""
    if isinstance(value, str):
        for secret in sorted((s for s in secrets if s), key=len, reverse=True):
            value = value.replace(secret, "[REDACTED]")
        return value
    if isinstance(value, dict):
        return {scrub(k, secrets): scrub(v, secrets) for k, v in value.items()}
    if isinstance(value, list):
        return [scrub(v, secrets) for v in value]
    return value
