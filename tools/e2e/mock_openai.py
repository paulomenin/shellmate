#!/usr/bin/env python3
"""OpenAI-compatible mock for end-to-end tests: POST /chat/completions, streaming (SSE) or not.

usage: mock_openai.py PORT LAST_REQUEST_FILE
Behaviour is picked from the last user message:
  contains SLOWPLEASE   -> waits 6 s before answering (cancel tests)
  contains STREAMTEST   -> streams 12 pieces "piece-N " 0.25 s apart (progressive rendering)
  starts with "Explain this command" -> prose only
  otherwise             -> prose + a command block
"""
import json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LAST = sys.argv[2]

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def handle(self):
        try:
            super().handle()
        except (ConnectionResetError, BrokenPipeError):
            pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        open(LAST, "w").write(json.dumps(body))
        user = body["messages"][-1]["content"]
        pieces, delay = None, 0.0
        if "SLOWPLEASE" in user:
            time.sleep(6)
        if "STREAMTEST" in user:
            pieces, delay = [f"piece-{i} " for i in range(1, 13)], 0.25
        if pieces is None:
            if user.startswith("Result of `") or user.startswith("Tool call `") or user.startswith("Contents of `"):
                text = "Checked it.\n```shellmate-command\nls -la\n```"
            elif "READPLEASE" in user:
                text = "```shellmate-tool\nread Cargo.toml\n```"
            elif "EXECPLEASE" in user:
                text = "```shellmate-tool\nexec echo e2e-exec-ran\n```"
            elif user.startswith("Explain this command"):
                text = "It prints the disk usage of each entry and sorts them, largest first."
            else:
                text = ("This lists the largest entries first, using `du` and `sort` as you asked for.\n"
                        "```shellmate-command\ndu -sh * | sort -rh | head\n```")
            pieces = [text[i:i + 9] for i in range(0, len(text), 9)]
        if body.get("stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            def chunk(b):
                self.wfile.write(b"%x\r\n" % len(b) + b + b"\r\n"); self.wfile.flush()
            try:
                for p in pieces:
                    chunk(("data: " + json.dumps({"choices": [{"delta": {"content": p}}]}) + "\n\n").encode())
                    time.sleep(delay)
                chunk(b"data: [DONE]\n\n"); self.wfile.write(b"0\r\n\r\n")
            except (BrokenPipeError, ConnectionResetError):
                pass
        else:
            out = json.dumps({"choices": [{"message": {"role": "assistant", "content": "".join(pieces)}, "finish_reason": "stop"}]}).encode()
            self.send_response(200); self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(out))); self.end_headers(); self.wfile.write(out)

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
