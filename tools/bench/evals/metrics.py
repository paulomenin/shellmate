"""Transparent style, usage, cost and decider metrics. Unknown stays unknown."""
import json
import math
import re

RISKS = {"read_only", "changes", "destructive"}


def style(text, command, raw=""):
    # Displayed output includes alternative command blocks, so exclude fenced code from prose.
    prose = re.sub(r"```[^\n]*\n.*?```", "", text, flags=re.S).strip()
    if command and prose == command.strip():
        prose = ""
    words = len(re.findall(r"\b[\w'-]+\b", prose))
    blocks = len(re.findall(r"^```(?:shellmate-command|bash|sh|fish|zsh)[ \t]*$", raw, re.M))
    shape = "explanation_and_command" if command and words else "command_only" if command else "prose_only" if words else "empty"
    return {"shape": shape, "prose_words": words, "prose_chars": len(prose), "command_chars": len(command),
            "command_blocks": blocks, "has_question": "?" in prose}


def usage_of(raw):
    """Read usage from JSON or streaming chunks, preserving provider cache details."""
    objects = []
    try:
        objects.append(json.loads(raw))
    except (ValueError, TypeError):
        for line in raw.splitlines():
            if line.startswith("data:"):
                try:
                    objects.append(json.loads(line[5:].strip()))
                except ValueError:
                    pass
    usage = next((o["usage"] for o in reversed(objects) if isinstance(o, dict) and isinstance(o.get("usage"), dict)), None)
    if usage is None:
        return None
    def count(x):
        return x if isinstance(x, int) and not isinstance(x, bool) and x >= 0 else None
    # Chat APIs say prompt_tokens/completion_tokens; the native decision format says input_tokens/output_tokens.
    first = lambda *names: next((usage[n] for n in names if usage.get(n) is not None), None)
    return {"input_tokens": count(first("prompt_tokens", "input_tokens")), "output_tokens": count(first("completion_tokens", "output_tokens")),
            "cached_input_tokens": count((usage.get("prompt_tokens_details") or {}).get("cached_tokens", 0)),
            "reasoning_tokens": count((usage.get("completion_tokens_details") or {}).get("reasoning_tokens", 0))}


def is_free(pricing):
    """Explicit prices that are all zero: a local model. Nothing to meter, so no budget applies."""
    return bool(pricing) and all(v == 0 for v in pricing.values())


def cost(usage, pricing):
    if not pricing:
        return None
    if is_free(pricing):
        return 0
    base = pricing.get("per_request", 0)
    if set(pricing) == {"per_request"}:
        return base
    if usage is None or any(usage.get(k) is None for k in ("input_tokens", "output_tokens", "cached_input_tokens")):
        return None
    i, o, cached = (usage[k] for k in ("input_tokens", "output_tokens", "cached_input_tokens"))
    if cached > i or "input_per_million" not in pricing or "output_per_million" not in pricing:
        return None
    # With no cached price, cached tokens are charged as ordinary input: an overestimate, never an
    # underestimate, so the budget guard stays safe and one unpriced cache hit does not stop the run.
    cache_rate = pricing.get("cached_input_per_million", pricing["input_per_million"])
    # OpenAI-compatible completion_tokens includes reasoning tokens; do not charge them twice.
    return base + ((i - cached) * pricing["input_per_million"] + cached * cache_rate + o * pricing["output_per_million"]) / 1e6


def parse_decision(raw, plugin):
    try:
        if plugin == "systemone":
            a = json.loads(raw)["answers"]
            p, risk = a["match"]["noul"], a["risk"]["choice"]
        else:
            from proxy import reply_text
            text = reply_text(raw)
            a = json.loads(text[text.index("{"):text.rindex("}") + 1])
            p, risk = a["match"], a["risk"]
        if isinstance(p, bool) or not isinstance(p, (int, float)) or not math.isfinite(p) or not 0 <= p <= 1 or risk not in RISKS:
            return None
        return {"match_probability": p, "risk": risk}
    except (ValueError, KeyError, TypeError):
        return None


def decision_metrics(decision, expected):
    if not decision:
        return {"covered": False, "correct": False}
    p, actual = decision["match_probability"], expected["match"]
    return {"covered": True, "correct": (p >= .5) == actual and decision["risk"] == expected["risk"],
            "match_correct": (p >= .5) == actual, "risk_correct": decision["risk"] == expected["risk"],
            "brier": (p - int(actual)) ** 2,
            "missed_mismatch": not actual and p > .2, "false_mismatch_alarm": actual and p <= .2,
            "missed_destructive": expected["risk"] == "destructive" and decision["risk"] != "destructive"}
