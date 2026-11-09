#!/usr/bin/env python3
"""Fake OpenAI-style SSE server. GET/POST /?n=40&delay=0.03&case=normal|evil|fast|stall"""
import json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs
LOG=sys.argv[2] if len(sys.argv)>2 else "/dev/null"
def log(m): open(LOG,"a").write(m+"\n")
TEXT=("To list the largest files in the current directory, use `du` with `sort`. "
      "Here is a longer explanation that should wrap across several terminal lines without breaking words "
      "in the middle: supercalifragilisticexpialidocious_and_a_very_long_identifier_that_exceeds_width_000000000000000000000000000. "
      "Naïve café résumé 日本語 emoji 🙂 end.\n\nSecond paragraph.\n- item one\n- item two\n").split(" ")
class H(BaseHTTPRequestHandler):
    protocol_version="HTTP/1.1"
    def log_message(self,*a): pass
    def do_GET(self): self.do_POST()
    def do_POST(self):
        q=parse_qs(urlparse(self.path).query); case=q.get("case",["normal"])[0]
        delay=float(q.get("delay",["0.03"])[0]); n=int(q.get("n",["0"])[0])
        self.send_response(200); self.send_header("Content-Type","text/event-stream"); self.send_header("Cache-Control","no-cache"); self.send_header("Transfer-Encoding","chunked"); self.end_headers()
        def chunk(b):
            self.wfile.write(b"%x\r\n"%len(b)+b+b"\r\n"); self.wfile.flush()
        def ev(s, split=False):
            d=("data: "+json.dumps({"choices":[{"delta":{"content":s}}]}, ensure_ascii=False)+"\n\n").encode()
            if split:  # split mid-event and mid-UTF-8 sequence
                m=len(d)//2; chunk(d[:m]); time.sleep(0.005); chunk(d[m:])
            else: chunk(d)
        try:
            time.sleep(float(q.get("ttfb",["0.4"])[0]))
            chunk(b": keepalive\n\n")
            words=TEXT if n==0 else (TEXT*50)[:n]
            for i,w in enumerate(words):
                tok=(w+" ") if i<len(words)-1 else w
                if case=="evil" and i==3: tok="\x1b[2J\x1b]0;pwned\x07\x1b[31mred\x00 "
                ev(tok, split=(case!="fast" and i%4==0))
                if case=="stall" and i==10: time.sleep(5)
                if case!="fast": time.sleep(delay)
            chunk(b"data: [DONE]\n\n"); self.wfile.write(b"0\r\n\r\n"); self.wfile.flush()
            log(f"{case}: completed")
        except (BrokenPipeError, ConnectionResetError):
            log(f"{case}: client disconnected")
ThreadingHTTPServer(("127.0.0.1",int(sys.argv[1])),H).serve_forever()
