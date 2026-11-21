"""Host-only credential broker, reachable over one private Unix socket.

No redirects, arbitrary destinations, request headers, keys or provider errors reach the container.
Only the two configured model routes are accepted. Responses stream so TTFT remains meaningful.
"""
import http.client
import hashlib
import json
import socketserver
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler

import native
from config import scrub
from metrics import cost, is_free, usage_of

LIMIT = 8 * 1024 * 1024
RETRYABLE = {408, 425, 429, 500, 502, 503, 504}  # rate limit and server trouble: worth waiting for


def reply_text(raw):
    try:
        obj = json.loads(raw)
        return obj["choices"][0]["message"].get("content") or ""
    except (ValueError, KeyError, IndexError, TypeError):
        out = []
        for line in raw.splitlines():
            if line.startswith("data:"):
                try:
                    out.append(json.loads(line[5:])["choices"][0]["delta"].get("content") or "")
                except (ValueError, KeyError, IndexError, TypeError):
                    pass
        return "".join(out)


def sse_of(raw):
    """A recorded answer (a JSON body) as the server-sent events a streaming request gets: the words, then each native
    call as its server sends it (the id and the name first, then the arguments in pieces), then the end."""
    def event(delta, finish=None):
        return "data: " + json.dumps({"choices": [{"delta": delta, "finish_reason": finish}]}) + "\n\n"
    out = [event({"content": reply_text(raw)})] if reply_text(raw) else []
    calls = native.calls_in(raw)
    for index, call in enumerate(calls):
        out.append(event({"tool_calls": [{"index": index, "id": call["id"] or f"call_{index}", "type": "function",
                                          "function": {"name": call["name"], "arguments": ""}}]}))
        arguments = call["arguments"]
        for start in range(0, len(arguments), 7):
            out.append(event({"tool_calls": [{"index": index, "function": {"arguments": arguments[start:start + 7]}}]}))
    out.append(event({}, "tool_calls" if calls else "stop"))
    return "".join(out) + "data: [DONE]\n\n"


class BudgetError(ValueError):
    """The eval's own request or cost limit, as opposed to a provider failure."""


class Budget:
    def __init__(self, requests, dollars, output_tokens):
        self.maximum, self.dollars, self.output_tokens = requests, dollars, output_tokens
        self.requests, self.spent, self.held = 0, 0.0, 0.0
        self.unknown = False
        self.exhausted = False
        self.lock = threading.Lock()

    def reserve(self, body, pricing):
        # UTF-8 bytes plus overhead is deliberately conservative, but not a billing guarantee.
        upper = cost({"input_tokens": len(json.dumps(body).encode()) + 4096,
                      "output_tokens": self.output_tokens, "cached_input_tokens": 0}, pricing)
        with self.lock:
            if upper is None or self.unknown:
                self.exhausted = True
                raise BudgetError("missing usage/pricing: cannot continue within the eval budget")
            if self.requests >= self.maximum or self.spent + self.held + upper > self.dollars:
                self.exhausted = True
                raise BudgetError("eval request or estimated cost budget reached")
            self.requests += 1
            self.held += upper
        return upper

    def settle(self, reserved, actual):
        with self.lock:
            self.held -= reserved
            if actual is None:
                self.unknown = True
            else:
                self.spent += actual


def error_message(error):
    """A provider's error text, reduced to one short printable line (its `error.message` when it has one)."""
    try:
        raw = error.read(2000).decode("utf-8", errors="replace")
    except (OSError, ValueError, AttributeError):
        return ""
    try:
        found = json.loads(raw)["error"]
        raw = found["message"] if isinstance(found, dict) else str(found)
    except (ValueError, KeyError, TypeError):
        pass
    return "".join(c if c.isprintable() else " " for c in str(raw)).strip()[:200]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = False


class Broker:
    def __init__(self, socket_path, routes, budget, timeout, replay=None, replay_only=False):
        self.routes, self.budget, self.timeout, self.replay = routes, budget, timeout, replay or {}
        self.replay_only = replay_only
        self.calls = []
        self.served = {}
        self.lock = threading.Lock()
        self.keys = [r.get("key") for r in routes.values()]
        owner = self
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                owner.handle(self)
        self.server = Server(str(socket_path), Handler)
        # The default poll interval is 0.5 s, and shutdown() waits for one poll: half a second added to every case.
        self.thread = threading.Thread(target=lambda: self.server.serve_forever(poll_interval=0.01), daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *exc):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def handle(self, h):
        route = next(((name, r) for name, r in self.routes.items() if h.path == f"/{name}{r['path']}"), None)
        if route is None:
            h.send_error(404)
            return
        name, r = route
        # A free (all-zero price) route costs nothing, even when it fails, and is not counted against the limits.
        free = is_free(r.get("pricing"))
        started, reserved, call = time.monotonic(), None, {"role": name, "usage": None, "cost_usd": 0 if free else None}
        try:
            length = int(h.headers.get("Content-Length", "0"))
            if not 0 < length <= LIMIT:
                raise ValueError("request size refused")
            body = json.loads(h.rfile.read(length))
            if not isinstance(body, dict) or body.get("model") != r["model"]:
                raise ValueError("unexpected model")
            call["request_hash"] = hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()
            # What came back to the model after its last answer (a tool's result, or why a call was refused): the
            # request ends in a user message that follows an assistant one only when a tool ran.
            messages = body.get("messages") if isinstance(body.get("messages"), list) else []
            if len(messages) >= 3 and isinstance(messages[-2], dict) and messages[-2].get("role") == "assistant" and isinstance(messages[-1], dict):
                call["tool_result"] = str(messages[-1].get("content", ""))[:4000]
            if name in self.replay:
                replies = self.replay[name]
                if isinstance(replies, list):
                    # a case with tools takes several answers, one per request, in the order they were recorded
                    with self.lock:
                        position = self.served[name] = self.served.get(name, -1) + 1
                    if position >= len(replies):
                        raise ValueError("missing replay response")
                    replies = replies[position]
                raw = replies.encode()
                call["replay"] = True
                if body.get("stream"):
                    raw = sse_of(raw.decode()).encode()
                h.send_response(200)
                h.send_header("Content-Type", "text/event-stream" if body.get("stream") else "application/json")
                h.send_header("Content-Length", str(len(raw)))
                h.end_headers()
                h.wfile.write(raw)
            else:
                if self.replay_only:
                    raise ValueError("missing replay response")
                if r["plugin"] == "openai-compatible":
                    requested = body.get("max_tokens", self.budget.output_tokens)
                    body["max_tokens"] = min(requested, self.budget.output_tokens) if type(requested) is int and requested > 0 else self.budget.output_tokens
                reserved = None if free else self.budget.reserve(body, r["pricing"])
                headers = {"Content-Type": "application/json"}
                if r.get("key"):
                    headers["Authorization"] = f"Bearer {r['key']}"
                request = urllib.request.Request(r["endpoint"].rstrip("/") + r["path"],
                                                 data=json.dumps(body).encode(), headers=headers)
                # Ignore ambient proxy settings, which can otherwise expose credentials to another host.
                opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
                with opener.open(request, timeout=self.timeout) as response:
                    h.send_response(200)
                    h.send_header("Content-Type", response.headers.get("Content-Type", "application/json"))
                    h.send_header("Connection", "close")
                    h.end_headers()
                    chunks, size = [], 0
                    while chunk := response.read1(8192):
                        size += len(chunk)
                        if size > LIMIT or time.monotonic() - started > self.timeout:
                            raise ValueError("provider response limit reached")
                        chunks.append(chunk)
                        h.wfile.write(chunk)
                        h.wfile.flush()
                    raw = b"".join(chunks)
                    h.close_connection = True
            text = raw.decode("utf-8", errors="replace")
            call.update({"response": text, "usage": usage_of(text)})
            if not call.get("replay"):
                call["cost_usd"] = cost(call["usage"], r["pricing"])
        except (ValueError, OSError, http.client.HTTPException) as error:
            # Nothing the provider said reaches the container, and credentials are scrubbed before anything is
            # stored. The status number and a short, cleaned message go to the results, so a refusal can be
            # understood (400: which field, which model) without the runner guessing.
            call["error"] = "eval budget reached" if isinstance(error, BudgetError) else "provider request failed, was refused, or exceeded a limit"
            if isinstance(error, urllib.error.HTTPError):
                call["status"] = error.code
                call["retryable"] = error.code in RETRYABLE
                call["cost_usd"] = 0  # an error status means the request was refused: not billed, still a request
                call["upstream_message"] = error_message(error)
                try:  # only the number of seconds; a date or anything else is ignored
                    call["retry_after"] = min(max(int(error.headers.get("Retry-After", "")), 0), 3600)
                except (TypeError, ValueError, AttributeError):
                    pass
            elif isinstance(error, (urllib.error.URLError, TimeoutError, ConnectionError)):
                call["retryable"] = True  # no answer at all: the server is down or too slow
                call["cost_usd"] = 0  # never answered: not billed, still a request
            try:
                h.send_error(502, "eval provider request failed or budget reached")
            except OSError:
                pass
        finally:
            if reserved is not None:
                self.budget.settle(reserved, call["cost_usd"])
            call["wall_ms"] = round((time.monotonic() - started) * 1000, 2)
            with self.lock:
                self.calls.append(scrub(call, self.keys))
