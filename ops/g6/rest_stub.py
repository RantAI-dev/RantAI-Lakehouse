"""A fixed, offset-paginated JSON fixture server for the G6 gate's rest
adapter test -- no external dependency, stdlib only."""
from __future__ import annotations

import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

_ROWS = [{"id": i, "name": f"item-{i}"} for i in range(5)]


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 (stdlib method name)
        parsed = urlparse(self.path)
        if parsed.path != "/items":
            self.send_response(404)
            self.end_headers()
            return
        offset = int(parse_qs(parsed.query).get("offset", ["0"])[0])
        page = _ROWS[offset : offset + 2]
        body = json.dumps({"items": page}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
