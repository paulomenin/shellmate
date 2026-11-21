"""Container boundary (Docker or Podman) and human approval. No host shell ever sees candidate commands."""
import json
import os
import subprocess
import sys
import termios
import tty
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent


def engine(environ=None):
    """The container engine: `docker` (default) or `podman`, chosen by SHELLMATE_EVAL_ENGINE."""
    environ = os.environ if environ is None else environ
    name = environ.get("SHELLMATE_EVAL_ENGINE", "docker")
    if name not in ("docker", "podman"):
        raise ValueError("SHELLMATE_EVAL_ENGINE must be docker or podman")
    return name


def client_env(environ=None):
    """Do not give the Docker client eval API keys from the parent's environment."""
    environ = os.environ if environ is None else environ
    return {k: v for k, v in environ.items() if k in {"PATH", "HOME", "DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_CONFIG",
            "DOCKER_TLS_VERIFY", "DOCKER_CERT_PATH", "XDG_RUNTIME_DIR", "XDG_CONFIG_HOME", "XDG_DATA_HOME",
            "CONTAINERS_CONF", "CONTAINERS_STORAGE_CONF"}}


def image_info(image):
    r = subprocess.run([engine(), "image", "inspect", image], capture_output=True, text=True, timeout=20, env=client_env())
    if r.returncode:
        raise ValueError("eval image is unavailable; build it with the README command (images are never pulled by the runner)")
    info = json.loads(r.stdout)[0]
    return {"id": info["Id"], "digests": info.get("RepoDigests", []), "created": info.get("Created")}


def remove_container(name):
    """Remove a container, best effort: cleanup must never turn a finished evaluation into a crash.
    `podman rm --force` first asks the container to stop and waits 10 s for a process that ignores the signal
    (the worker is PID 1), so Podman is told not to wait. Docker's `rm --force` already kills at once."""
    args = [engine(), "rm", "--force"] + (["--time", "0"] if engine() == "podman" else []) + [name]
    try:
        subprocess.run(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60, env=client_env())
        return True
    except (subprocess.TimeoutExpired, OSError):
        print(f"evals: could not remove container {name}; remove it with `{engine()} rm --force {name}`", file=sys.stderr)
        return False


def docker_args(image, name, fixtures, broker=None, engine_name=None, serve=False):
    engine_name = engine_name or engine()
    args = [engine_name, "run", "--rm", "--pull=never", "--name", name, "--network=none", "--read-only",
            "--cap-drop=ALL", "--security-opt=no-new-privileges", "--pids-limit=64", "--memory=512m",
            "--memory-swap=512m", "--cpus=1", "--ulimit", "fsize=8388608:8388608", "--ulimit", "nofile=128:128",
            "--user", f"{os.getuid() or 65532}:{os.getgid() or 65532}",
            "--tmpfs", "/tmp:rw,nosuid,nodev,noexec,size=64m,mode=1777",
            "--tmpfs", "/work:rw,nosuid,nodev,noexec,size=64m,mode=1777",
            "--mount", f"type=bind,src={fixtures},dst=/fixtures,readonly"]
    if engine_name == "podman":
        # Rootless: the host uid is root in the container unless it is mapped to itself, and the fixtures
        # and the broker socket belong to the host uid.
        args += ["--userns=keep-id"]
    if broker:
        args += ["--mount", f"type=bind,src={broker},dst=/broker,readonly"]
    return args + ["-i", image, "python3", "-I", "/opt/eval/worker.py"] + (["serve"] if serve else [])


def run(image, fixtures, payload, broker=None):
    if payload["mode"] == "execute" and broker is not None:
        raise ValueError("execution sandboxes must never receive the provider socket")
    name = "sm-eval-" + uuid.uuid4().hex
    args = docker_args(image, name, fixtures, broker)
    try:
        result = subprocess.run(args, input=json.dumps(payload), capture_output=True, text=True,
                                timeout=payload["timeout"] + 30, env=client_env())
        if result.returncode:
            # Docker stderr may contain fixture paths; don't relay untrusted worker output to the terminal.
            raise ValueError("container failed; check the image, container engine permissions and worker compatibility")
        try:
            data = json.loads(result.stdout)
        except ValueError:
            raise ValueError("container returned an invalid result") from None
        return data
    except subprocess.TimeoutExpired:
        raise ValueError("container timed out") from None
    finally:
        # Killing the Docker client alone does not stop its container.
        remove_container(name)


class Session:
    """One long-lived container for many `invoke` requests (the step that runs only shellmate).

    Each request still starts from a clean slate inside it (the worker wipes /work, the home and runtime
    directories and the config first). If a request times out, the worker dies or its answer is unreadable, the
    container is removed and the next request starts a new one, so one bad case cannot poison the rest.
    Execution of a model's command never uses a session: that is `run`, in a fresh container each time."""

    LIMIT = 32 * 1024 * 1024

    def __init__(self, image, fixtures, broker, command=None):
        self.image, self.fixtures, self.broker, self.command = image, fixtures, broker, command
        self.process = None
        self.name = None
        self.served = 0

    def start(self):
        if self.process and self.process.poll() is None:
            return
        self.name = "sm-eval-" + uuid.uuid4().hex
        args = self.command or docker_args(self.image, self.name, self.fixtures, self.broker, serve=True)
        self.process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=client_env())

    def request(self, payload):
        import select
        import time
        if payload.get("mode") != "invoke":
            raise ValueError("a session runs invoke requests only")
        self.start()
        deadline = time.monotonic() + payload["timeout"] + 30
        try:
            self.process.stdin.write((json.dumps(payload) + "\n").encode())
            self.process.stdin.flush()
            fd, buffer = self.process.stdout.fileno(), b""
            while b"\n" not in buffer:
                left = deadline - time.monotonic()
                if left <= 0:
                    self.kill()
                    raise ValueError("container timed out")
                if not select.select([fd], [], [], min(left, 1.0))[0]:
                    continue
                chunk = os.read(fd, 1 << 20)
                if not chunk or len(buffer) > self.LIMIT:
                    self.kill()
                    raise ValueError("container failed; check the image, container engine permissions and worker compatibility")
                buffer += chunk
        except OSError:
            self.kill()
            raise ValueError("container failed; check the image, container engine permissions and worker compatibility") from None
        try:
            data = json.loads(buffer.split(b"\n", 1)[0])
        except ValueError:
            self.kill()
            raise ValueError("container returned an invalid result") from None
        if not isinstance(data, dict) or data.get("error"):
            self.kill()  # the worker survived an exception, but its state is no longer known to be clean
            raise ValueError("container failed; check the image, container engine permissions and worker compatibility")
        self.served += 1
        return data

    def kill(self):
        process, self.process = self.process, None
        if process is None:
            return
        process.kill()
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            pass
        for stream in (process.stdin, process.stdout):
            try:
                stream.close()
            except OSError:
                pass
        if self.command is None and self.name:
            # Killing the engine's client alone does not stop its container.
            remove_container(self.name)

    close = kill


def approve(command):
    """Only fresh, individually typed keys at a real terminal can approve a candidate."""
    try:
        with open("/dev/tty", "r+b", buffering=0) as terminal:
            fd = terminal.fileno()
            before = termios.tcgetattr(fd)
            terminal.write((f"\nExecute in disposable offline sandbox: {ascii(command)}\nType yes then Enter to approve; any other key denies: ").encode())
            try:
                tty.setraw(fd)
                termios.tcflush(fd, termios.TCIFLUSH)
                for expected in b"yes\r":
                    if os.read(fd, 32) != bytes([expected]):
                        return False
                return True
            finally:
                termios.tcsetattr(fd, termios.TCSANOW, before)
                terminal.write(b"\r\n")
    except (OSError, termios.error):
        return False
