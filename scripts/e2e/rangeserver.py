#!/usr/bin/env python3
"""Minimal static file server that honours HTTP Range requests (206), for e2e tests.

Usage: rangeserver.py PORT [DIRECTORY]
"""
import os
import re
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer


class RangeHandler(SimpleHTTPRequestHandler):
    def log_message(self, *args):  # keep test output quiet
        pass

    def do_GET(self):
        path = self.translate_path(self.path)
        rng = self.headers.get("Range")
        if not rng or not os.path.isfile(path):
            return super().do_GET()
        m = re.match(r"bytes=(\d+)-(\d*)$", rng.strip())
        if not m:
            self.send_response(416)
            self.end_headers()
            return
        size = os.path.getsize(path)
        start = int(m.group(1))
        end = int(m.group(2)) if m.group(2) else size - 1
        end = min(end, size - 1)
        if start > end:
            self.send_response(416)
            self.end_headers()
            return
        self.send_response(206)
        self.send_header("Content-Type", self.guess_type(path))
        self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Content-Length", str(end - start + 1))
        self.end_headers()
        with open(path, "rb") as f:
            f.seek(start)
            self.wfile.write(f.read(end - start + 1))


if __name__ == "__main__":
    port = int(sys.argv[1])
    if len(sys.argv) > 2:
        os.chdir(sys.argv[2])
    ThreadingHTTPServer(("127.0.0.1", port), RangeHandler).serve_forever()
