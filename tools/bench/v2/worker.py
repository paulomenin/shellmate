"""Runs inside the networkless container. No provider credentials are available here."""
import hashlib
import http.client
import json
import os
import signal
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

LIMIT = 2 * 1024 * 1024
ENV = {"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/tmp/home", "XDG_RUNTIME_DIR": "/tmp/runtime",
       "XDG_CONFIG_HOME": "/tmp/home/config", "XDG_STATE_HOME": "/tmp/home/state", "LANG": "C.UTF-8",
       "TERM": "dumb", "NO_COLOR": "1", "SHELLMATE_TIME_SCALE": "5"}


def bounded(args, data=b"", timeout=120, cwd="/work"):
    """Spool to capped tmpfs rather than accumulating arbitrary model/command output in RAM."""
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        try:
            p = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=out, stderr=err, env=ENV, cwd=cwd, start_new_session=True)
            p.communicate(data, timeout=timeout)
            code, timed_out = p.returncode, False
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGKILL)
            p.communicate()
            code, timed_out = p.returncode, True
        out.seek(0)
        err.seek(0)
        a, b = out.read(LIMIT + 1), err.read(LIMIT + 1)
        return {"code": code, "stdout": a[:LIMIT].decode(errors="replace"), "stderr": b[:LIMIT].decode(errors="replace"),
                "timed_out": timed_out, "truncated": len(a) > LIMIT or len(b) > LIMIT}


class UnixConnection(http.client.HTTPConnection):
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect("/broker/api.sock")


class Forward(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        if not 0 < n <= 8 * 1024 * 1024:
            self.send_error(413)
            return
        conn = UnixConnection("broker", timeout=Forward.timeout)
        try:
            conn.request("POST", self.path, self.rfile.read(n), {"Content-Type": "application/json"})
            r = conn.getresponse()
            self.send_response(r.status)
            self.send_header("Content-Type", r.getheader("Content-Type", "application/json"))
            self.send_header("Connection", "close")
            self.end_headers()
            while chunk := r.read1(8192):
                self.wfile.write(chunk)
                self.wfile.flush()
            self.close_connection = True
        except OSError:
            self.close_connection = True
        finally:
            conn.close()


def manifest(root):
    out = {}
    for p in sorted(root.rglob("*")):
        name = str(p.relative_to(root))
        if p.is_symlink():
            out[name] = {"symlink": os.readlink(p)}
        elif p.is_file():
            digest = hashlib.sha256()
            with p.open("rb") as source:
                while chunk := source.read(65536):
                    digest.update(chunk)
            out[name] = {"sha256": digest.hexdigest(), "bytes": p.stat().st_size}
        if len(out) > 10000:
            raise ValueError("too many files in behavioral result")
    return out


WORK, HOME, RUNTIME, CONFIG, FIXTURES = "/work", "/tmp/home", "/tmp/runtime", "/tmp/eval.toml", "/fixtures"


def wipe(path):
    """Empty a directory without removing it (a mount point cannot be removed, or chmod-ed, by the user)."""
    p = Path(path)
    if p.is_dir() and not p.is_symlink():
        for entry in p.iterdir():
            if entry.is_dir() and not entry.is_symlink():
                shutil.rmtree(entry, ignore_errors=True)
            else:
                entry.unlink(missing_ok=True)


def prepare(payload, work=WORK, home=HOME, runtime=RUNTIME, config=CONFIG, fixtures=FIXTURES):
    """A clean slate for every case: nothing a case left behind (files, config, state) reaches the next one."""
    for p in (home, runtime, work):
        wipe(p)
        Path(p).mkdir(mode=0o700, parents=True, exist_ok=True)
    Path(config).unlink(missing_ok=True)
    fixture = Path(fixtures) / payload["fixture"]
    # Not copytree(fixture, work): it would chmod the mount point itself, which the user does not own.
    for entry in sorted(fixture.iterdir()):
        if entry.is_dir():
            shutil.copytree(entry, Path(work) / entry.name)
        else:
            shutil.copy2(entry, work)


def start_forward(timeout):
    Forward.timeout = timeout
    server = ThreadingHTTPServer(("127.0.0.1", 0), Forward)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def invoke(payload, server):
    """Run shellmate the way the shell glue does, then its own check of the command. Returns the result."""
    Forward.timeout = payload["timeout"]
    config = payload["config"].replace("@BROKER@", f"http://127.0.0.1:{server.server_port}")
    Path(CONFIG).write_text(config)
    fields = {"v": "1", "action": payload["action"], "shell": payload["shell"], "cwd": WORK, "cursor": "0"}
    envelope = ("".join(f"{k}={v}\0" for k, v in fields.items()) + f"buffer={payload['input']}\0").encode()
    started = time.monotonic()
    result = bounded(["/usr/local/bin/shellmate", "--config", CONFIG, "invoke", "--timings=json"],
                     envelope, payload["timeout"])
    result["wall_ms"] = round((time.monotonic() - started) * 1000, 2)
    command = result["stdout"].strip()
    result["facts"] = None
    if command:
        checked = bounded(["/usr/local/bin/shellmate", "check", "--shell", payload["shell"], "--json", command], timeout=20)
        try:
            facts = json.loads(checked["stdout"])
            if checked["code"] != 0 or checked["timed_out"] or not isinstance(facts, dict) or facts.get("risk") not in {"low", "medium", "high"}:
                raise ValueError("invalid check")
            result["facts"] = facts
        except ValueError:
            result["check_error"] = True
    if payload.get("environment"):
        # Constant for the whole image, so the runner asks for it once.
        result["programs"] = {n: shutil.which(n) is not None for n in ("bash", "fish", "zsh", "git", "tar", "jq", "rg", "python3", "ss", "systemctl")}
        result["packages"] = Path("/opt-packages.txt").read_text()
        result["binary_version"] = bounded(["/usr/local/bin/shellmate", "--version"], timeout=10)["stdout"].strip()
    return result


def serve():
    """One request per line on stdin, one result per line on stdout, for a container that handles many cases.
    It runs shellmate only. A model's command is never executed here: that is the one-shot `execute` mode in a
    fresh container."""
    server = start_forward(120)
    for line in sys.stdin:
        try:
            payload = json.loads(line)
            if payload.get("mode") != "invoke":
                result = {"code": None, "error": "a session container runs invoke requests only"}
            else:
                prepare(payload)
                result = invoke(payload, server)
        except Exception as error:  # one bad request must not end the session; the runner restarts it if needed
            result = {"code": None, "error": f"worker error: {type(error).__name__}"}
        sys.stdout.write(json.dumps(result) + "\n")
        sys.stdout.flush()


def main():
    if sys.argv[1:] == ["serve"]:
        return serve()
    payload = json.load(sys.stdin)
    prepare(payload)
    if payload["mode"] == "execute":
        # This mode is reachable only after the host runner's explicit per-command approval.
        result = bounded([payload["shell"], "-c", payload["command"]], timeout=payload["timeout"])
        result["files"] = manifest(Path(WORK))
        print(json.dumps(result))
        return
    server = start_forward(payload["timeout"])
    result = invoke(payload, server)
    server.shutdown()
    server.server_close()
    print(json.dumps(result))


if __name__ == "__main__":
    main()
