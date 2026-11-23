"""Shared helpers for the shellmate benchmarks: statistics, running the binary with timing and peak memory,
a fake LLM server, and facts about the machine. Standard library only (Python 3.11+)."""
from __future__ import annotations

import json
import math
import os
import platform
import shutil
import signal
import sys
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = 1  # bump when the result files change shape


# ------------------------------------------------------------------------------------------- statistics
def percentile(sorted_xs: list[float], q: float) -> float:
    """Linear interpolation between closest ranks; q in [0, 100]."""
    if not sorted_xs:
        return math.nan
    if len(sorted_xs) == 1:
        return sorted_xs[0]
    k = (len(sorted_xs) - 1) * q / 100
    lo, hi = math.floor(k), math.ceil(k)
    return sorted_xs[lo] + (sorted_xs[hi] - sorted_xs[lo]) * (k - lo)


def stats(xs: list[float]) -> dict:
    """Summary of a sample. `mad` (median absolute deviation) is the robust spread; `noise` is mad/p50, a
    quick way to see whether a comparison between two runs can be trusted."""
    xs = sorted(x for x in xs if x is not None and not math.isnan(x))
    if not xs:
        return {"n": 0}
    p50 = percentile(xs, 50)
    mad = percentile(sorted(abs(x - p50) for x in xs), 50)
    mean = sum(xs) / len(xs)
    var = sum((x - mean) ** 2 for x in xs) / (len(xs) - 1) if len(xs) > 1 else 0.0
    return {
        "n": len(xs),
        "min": xs[0],
        "p50": p50,
        "p90": percentile(xs, 90),
        "p95": percentile(xs, 95),
        "max": xs[-1],
        "mean": mean,
        "stdev": math.sqrt(var),
        "mad": mad,
        "noise": (mad / p50) if p50 else 0.0,
    }


# ------------------------------------------------------------------------------------------ the machine
def _cpu_model() -> str:
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.lower().startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def _mem_gib() -> float | None:
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal"):
                return round(int(line.split()[1]) / 1024 / 1024, 1)
    except OSError:
        pass
    return None


def _git(*args: str) -> str | None:
    try:
        out = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, timeout=10)
        return out.stdout.strip() if out.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def host_info() -> dict:
    return {
        "cpu": _cpu_model(),
        "cores": os.cpu_count(),
        "mem_gib": _mem_gib(),
        "os": f"{platform.system()} {platform.release()}",
        "python": platform.python_version(),
        "load_1m": round(os.getloadavg()[0], 2) if hasattr(os, "getloadavg") else None,
    }


def build_info(binary: Path) -> dict:
    version = None
    try:
        version = subprocess.run([str(binary), "--version"], capture_output=True, text=True, timeout=10).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        pass
    return {
        "version": version,
        "commit": _git("rev-parse", "--short", "HEAD"),
        "dirty": bool(_git("status", "--porcelain")),
        "binary": str(binary),
        "binary_bytes": binary.stat().st_size if binary.exists() else None,
    }


def find_binary(explicit: str | None, build: bool = True, profile: str = "release") -> Path:
    """`--bin`, then $SHELLMATE_BIN, then target/<profile>/shellmate (built first unless build=False)."""
    chosen = explicit or os.environ.get("SHELLMATE_BIN")
    if chosen:
        p = Path(chosen).resolve()
        if not p.is_file():
            raise SystemExit(f"binary not found: {p}")
        return p
    if build:
        flag = ["--release"] if profile == "release" else []
        r = subprocess.run(["cargo", "build", "-q", "-p", "shellmate", *flag], cwd=ROOT)
        if r.returncode:
            raise SystemExit("cargo build failed")
    p = ROOT / "target" / profile / "shellmate"
    if not p.is_file():
        raise SystemExit(f"{p} does not exist (build it, or pass --bin)")
    return p


# ------------------------------------------------------------------------------------ running the binary
@dataclass
class Result:
    code: int
    stdout: str
    stderr: str
    wall_ms: float
    timed_out: bool = False
    timings: dict | None = None


# The default cwd is only sent in the envelope, never opened here.
def envelope(action: str, text: str, shell: str = "fish", cwd: str = "/tmp", extra: dict | None = None) -> bytes:  # NOSONAR
    """The glue protocol's NUL-separated envelope (`buffer` last)."""
    fields = {"v": "1", "action": action, "shell": shell, "cwd": cwd, "cursor": "0", **(extra or {})}
    return ("".join(f"{k}={v}\0" for k, v in fields.items()) + f"buffer={text}\0").encode()


def run_binary(args: list[str], stdin: bytes = b"", env: dict | None = None, timeout: float = 120.0,
               cwd: str | None = None) -> Result:
    """Run a command to completion and report its wall time. Output goes to files, never to pipes that could
    fill up. Detached from the controlling terminal (own session), as the shell glue's captured runs are."""
    with tempfile.TemporaryDirectory(prefix="sm-bench-") as d:
        d = Path(d)
        (d / "in").write_bytes(stdin)
        with open(d / "in", "rb") as fin, open(d / "out", "wb") as fout, open(d / "err", "wb") as ferr:
            t0 = time.perf_counter()
            p = subprocess.Popen(args, stdin=fin, stdout=fout, stderr=ferr, env=env, cwd=cwd,
                                 start_new_session=True)
            timed_out = threading.Event()

            def kill():
                timed_out.set()
                try:
                    os.killpg(p.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

            timer = threading.Timer(timeout, kill)
            timer.start()
            try:
                _, status, _ru = os.wait4(p.pid, 0)
            finally:
                timer.cancel()
            wall = (time.perf_counter() - t0) * 1000
            p.returncode = os.waitstatus_to_exitcode(status)
        return Result(
            code=p.returncode,
            stdout=(d / "out").read_text(errors="replace"),
            stderr=(d / "err").read_text(errors="replace"),
            wall_ms=wall,
            timed_out=timed_out.is_set(),
        )


def peak_rss_mib(args: list[str], stdin: bytes = b"", env: dict | None = None, cwd: str | None = None,
                 timeout: float = 60.0) -> float | None:
    """Peak resident memory (`VmHWM`) of one run, in MiB, read from /proc while the process lives. The value
    is a high-water mark, so the last reading before exit is the peak. (`wait4` rusage is not usable: a child
    started from this Python process inherits the interpreter's own peak.) None where /proc is unavailable
    or the process ended before it could be read."""
    if not Path("/proc/self/status").exists():
        return None
    with tempfile.TemporaryDirectory(prefix="sm-rss-") as d:
        d = Path(d)
        (d / "in").write_bytes(stdin)
        with open(d / "in", "rb") as fin, open(os.devnull, "wb") as null:
            p = subprocess.Popen(args, stdin=fin, stdout=null, stderr=null, env=env, cwd=cwd, start_new_session=True)
            me = os.path.realpath(sys.executable)
            peak, deadline = None, time.monotonic() + timeout
            while p.poll() is None and time.monotonic() < deadline:
                try:
                    # until exec the child is still a copy of this interpreter: ignore those readings
                    if os.path.realpath(os.readlink(f"/proc/{p.pid}/exe")) != me:
                        for line in Path(f"/proc/{p.pid}/status").read_text().splitlines():
                            if line.startswith("VmHWM:"):
                                peak = int(line.split()[1]) / 1024
                                break
                except (OSError, ValueError):
                    pass
                time.sleep(0.0003)
            if p.poll() is None:
                os.killpg(p.pid, signal.SIGKILL)
                p.wait()
            return peak


def parse_timings(stderr: str) -> dict | None:
    """The `{"timings": {...}}` line `invoke --timings=json` prints."""
    for line in reversed(stderr.splitlines()):
        line = line.strip()
        if line.startswith('{"timings"'):
            try:
                return json.loads(line)["timings"]
            except (ValueError, KeyError):
                return None
    return None


def isolated_env(home: Path, extra: dict | None = None) -> dict:
    """A clean environment: own HOME and runtime dir, so a benchmark never touches the real config, audit log
    or conversations, and a single run cannot influence the next."""
    run = home / "run"
    run.mkdir(mode=0o700, parents=True, exist_ok=True)
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(home),
        "XDG_RUNTIME_DIR": str(run),
        "XDG_CONFIG_HOME": str(home / "config"),
        "XDG_STATE_HOME": str(home / "state"),
        "XDG_DATA_HOME": str(home / "data"),
        "LANG": "C.UTF-8",
        "TERM": "dumb",
    }
    env.update(extra or {})
    return env


# -------------------------------------------------------------------------------------------- LLM servers
def _sse(obj: dict) -> bytes:
    return ("data: " + json.dumps(obj) + "\n\n").encode()


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def handle(self):
        try:
            super().handle()
        except (ConnectionResetError, BrokenPipeError):
            pass

    def do_GET(self):  # a /models probe, should anything ask
        self.send_response(200)
        body = b'{"data":[]}'
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0) or 0)
        body = json.loads(self.rfile.read(n) or b"{}")
        self.server.owner.handle_chat(self, body)  # type: ignore[attr-defined]


class FakeLLM:
    """An OpenAI-compatible server on localhost whose replies come from `reply(request_body) -> text`.

    `ttft_ms` delays the first piece and `tokens_per_s` paces the rest (0 = as fast as possible), so a run can
    measure the client's own overhead (instant) or imitate a model (paced). `piece_chars` is the size of each
    streamed piece. Every request body is kept in `requests`."""

    def __init__(self, reply, ttft_ms: float = 0.0, tokens_per_s: float = 0.0, piece_chars: int = 4):
        self.reply, self.ttft_ms, self.tokens_per_s, self.piece_chars = reply, ttft_ms, tokens_per_s, piece_chars
        self.requests: list[dict] = []
        self._srv = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        self._srv.owner = self  # type: ignore[attr-defined]
        self._srv.daemon_threads = True
        self._thread = threading.Thread(target=self._srv.serve_forever, daemon=True)

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self._srv.server_address[1]}"

    def __enter__(self):
        self._thread.start()
        return self

    def __exit__(self, *exc):
        self._srv.shutdown()
        self._srv.server_close()

    def handle_chat(self, h: BaseHTTPRequestHandler, body: dict):
        self.requests.append(body)
        text = self.reply(body)
        pieces = [text[i:i + self.piece_chars] for i in range(0, len(text), self.piece_chars)] or [""]
        usage = {"prompt_tokens": max(1, len(json.dumps(body.get("messages", []))) // 4),
                 "completion_tokens": max(1, len(text) // 4)}
        if not body.get("stream"):
            out = json.dumps({"choices": [{"message": {"role": "assistant", "content": text},
                                           "finish_reason": "stop"}], "usage": usage}).encode()
            if self.ttft_ms:
                time.sleep(self.ttft_ms / 1000)
            h.send_response(200)
            h.send_header("Content-Type", "application/json")
            h.send_header("Content-Length", str(len(out)))
            h.end_headers()
            h.wfile.write(out)
            return
        h.send_response(200)
        h.send_header("Content-Type", "text/event-stream")
        h.send_header("Transfer-Encoding", "chunked")
        h.end_headers()

        def chunk(b: bytes):
            h.wfile.write(b"%x\r\n" % len(b) + b + b"\r\n")
            h.wfile.flush()

        try:
            if self.ttft_ms:
                time.sleep(self.ttft_ms / 1000)
            gap = (self.piece_chars / 4) / self.tokens_per_s if self.tokens_per_s else 0
            for i, p in enumerate(pieces):
                chunk(_sse({"choices": [{"delta": {"content": p}}]}))
                if gap and i < len(pieces) - 1:
                    time.sleep(gap)
            chunk(_sse({"choices": [{"delta": {}, "finish_reason": "stop"}]}))
            chunk(_sse({"choices": [], "usage": usage}))
            chunk(b"data: [DONE]\n\n")
            h.wfile.write(b"0\r\n\r\n")
        except (BrokenPipeError, ConnectionResetError):
            pass


# --------------------------------------------------------------------------------------------- misc
def write_json(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(data, indent=2, sort_keys=False) + "\n")
    tmp.replace(path)  # a half-written result file never replaces a good one


def which(name: str) -> str | None:
    return shutil.which(name)
