#!/usr/bin/env python3
"""The showcase's echo server for Secrets and fetch, on 127.0.0.1:8765.

GET /echo answers whether an Authorization header came, never its value.
Run it beside `just showcase`: python3 -I examples/showcase/secrets-fetch/echo.py
"""
import http.server


class Echo(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        token = "with a token" if self.headers.get("Authorization") else "without a token"
        body = f"heard {self.path} {token}".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    http.server.HTTPServer(("127.0.0.1", 8765), Echo).serve_forever()
