"""Native tool calls, as the evaluator reads them.

A model that takes tools natively does not write a `shellmate-tool` block. It answers with calls in the provider's
protocol (`tool_calls`, with JSON arguments). To score such a model by the same rules as one that writes text, each
reply is turned into the text it stands for: what the model said, then one block per call holding the line shellmate
makes of the call (`man tar`, `exec git status`). That line is made the way the tools make it, and the table in
`native_lines.json` is shared with the Rust tests to keep the two the same.

What is different for a native call is whether it was well formed: arguments that are valid JSON, a JSON object, with
the keys of the tool's schema and nothing else, and one call to a reply (shellmate runs only the first).
"""
import json

SCHEMAS = {  # tool -> (keys the schema lists)
    "man": {"page", "section"},
    "help": {"program", "subcommand"},
    "exec": {"command"},
    "read": {"path"},
}


def _words(arguments, name):
    """The words of a known tool's call, or ValueError saying why the arguments are not one. Mirrors `Tool::call_line`."""
    if not isinstance(arguments, dict):
        raise ValueError("the arguments must be a JSON object")
    extra = set(arguments) - SCHEMAS[name]
    if extra:
        raise ValueError(f"unknown field `{sorted(extra)[0]}`")

    def string(key, required=True, nullable=False):
        if key not in arguments or (arguments[key] is None and nullable):
            if required:
                raise ValueError(f"missing field `{key}`")
            return None
        if not isinstance(arguments[key], str):
            raise ValueError(f"invalid type for `{key}`: expected a string")
        return arguments[key]

    if name == "man":
        page, section = string("page"), string("section", required=False, nullable=True)
        return [section, page] if section is not None else [page]
    if name == "help":
        program = string("program")
        subs = arguments.get("subcommand", [])
        if not isinstance(subs, list) or not all(isinstance(s, str) for s in subs):
            raise ValueError("invalid type for `subcommand`: expected a list of strings")
        return [program, *subs]
    return [string("command" if name == "exec" else "path")]


def line_of(name, arguments):
    """What follows the tool's name in a call written in text. ValueError when the arguments are not a call."""
    if name not in SCHEMAS:
        raise ValueError(f"there is no tool called `{name}`")
    return " ".join(_words(arguments, name))


def arguments_of(name, line):
    """The inverse, for replaying a recorded text call natively: the arguments a model would send for `line`."""
    words = line.split()
    if name == "man":
        return {"section": words[0], "page": words[1]} if len(words) == 2 else {"page": line}
    if name == "help":
        return {"program": words[0], "subcommand": words[1:]} if words else {"program": ""}
    if name == "read":
        return {"path": line}
    return {"command": line}  # exec, and a tool that does not exist: the line is all there is to send


def calls_in(raw):
    """The native calls in a provider's answer (a JSON body, or the server-sent events of a stream), in order:
    each `{"id", "name", "arguments"}` with the arguments as the text the model wrote."""
    try:
        message = json.loads(raw)["choices"][0]["message"]
        return [{"id": c.get("id"), "name": c["function"]["name"], "arguments": _text(c["function"].get("arguments"))}
                for c in message.get("tool_calls") or [] if isinstance(c, dict) and isinstance(c.get("function"), dict)]
    except (ValueError, KeyError, IndexError, TypeError, AttributeError):
        pass
    calls = {}
    for line in str(raw).splitlines():
        if not line.startswith("data:") or line[5:].strip() == "[DONE]":
            continue
        try:
            delta = json.loads(line[5:])["choices"][0]["delta"]
        except (ValueError, KeyError, IndexError, TypeError):
            continue
        for fragment in delta.get("tool_calls") or []:
            if not isinstance(fragment, dict):
                continue
            call = calls.setdefault(fragment.get("index", len(calls)), {"id": None, "name": "", "arguments": ""})
            function = fragment.get("function") or {}
            call["id"] = fragment.get("id") or call["id"]
            call["name"] += function.get("name") or ""
            if isinstance(function.get("arguments"), str):
                call["arguments"] += function["arguments"]
            elif function.get("arguments") is not None:
                call["arguments"] = json.dumps(function["arguments"])
    return [calls[i] for i in sorted(calls) if calls[i]["name"]]


def _text(arguments):
    return arguments if isinstance(arguments, str) else json.dumps(arguments if arguments is not None else {})


def read(call):
    """One call as scored: its tool, the line it stands for, and whether the arguments were a call.

    A tool that does not exist is not a malformed call (the rule for tools that are offered says so); its line is
    its arguments as text, so it still reads as one call."""
    name = call["name"]
    try:
        parsed = json.loads(call["arguments"])
    except ValueError:
        return {"name": name, "line": f"{name} {_one_line(call['arguments'])}".strip(), "valid": False, "why": "not valid JSON"}
    if name not in SCHEMAS:
        flat = " ".join(str(v) for v in parsed.values()) if isinstance(parsed, dict) else _one_line(call["arguments"])
        return {"name": name, "line": f"{name} {flat}".strip(), "valid": True, "why": ""}
    try:
        return {"name": name, "line": f"{name} {line_of(name, parsed)}", "valid": True, "why": ""}
    except ValueError as error:
        return {"name": name, "line": f"{name} {_one_line(call['arguments'])}", "valid": False, "why": str(error)}


def _one_line(text):
    return " ".join(str(text).split())


def canonical(text, calls):
    """The reply as text: what the model said, and each native call as the block it stands for."""
    blocks = [f"```shellmate-tool\n{read(c)['line']}\n```" for c in calls]
    return "\n".join(x for x in [(text or "").strip(), *blocks] if x)


def reply_of(raw, text):
    """`(canonical text, per-call details)` of one answer, whichever way the model called its tools. In text mode the
    answer has no native calls and the text is unchanged."""
    calls = [read(c) for c in calls_in(raw)]
    return canonical(text, calls_in(raw)), calls
