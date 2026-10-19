"""The mock vendor server of the SaaS gate (`g6-saas`, SRC-3 AD9), stdlib only
like `rest_stub.py`.

One server answers for every vendor, each on its own path prefix
(`/<vendor id>/...`), so the gate needs one container and one port however many
vendors it covers. Slice 1 (the shared parts, no vendor yet) serves only
`GET /health`. Each vendor's routes are added with the vendor (slices 2 to 6)
and behave as the vendor does: its sign-in, its listing, its paging by its own
shape, a cursor filter, a delete where the vendor reports deletes, a 429 with
`Retry-After`, a changed field. Nothing here is a real vendor's data.

# Registering a vendor

`ROUTES` maps `(method, path prefix)` to a handler `f(handler, parsed_url)`
that writes the whole response. A vendor adds its entries at import time of its
own section below; an unknown path answers 404 with an empty body.
"""
from __future__ import annotations

import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Callable
from urllib.parse import ParseResult, urlparse

Route = Callable[[BaseHTTPRequestHandler, ParseResult], None]

ROUTES: dict[tuple[str, str], Route] = {}


def send_json(handler: BaseHTTPRequestHandler, body: object, status: int = 200, headers: dict[str, str] | None = None) -> None:
    payload = json.dumps(body).encode()
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json")
    handler.send_header("Content-Length", str(len(payload)))
    for name, value in (headers or {}).items():
        handler.send_header(name, value)
    handler.end_headers()
    handler.wfile.write(payload)


def _health(handler: BaseHTTPRequestHandler, _parsed: ParseResult) -> None:
    send_json(handler, {"status": "ok", "vendors": sorted({prefix.split("/")[1] for (_m, prefix) in ROUTES if prefix.count("/") > 1})})


ROUTES[("GET", "/health")] = _health


class Handler(BaseHTTPRequestHandler):
    def _dispatch(self, method: str) -> None:
        parsed = urlparse(self.path)
        # The longest registered prefix wins, so a vendor can route a subtree.
        for (m, prefix), route in sorted(ROUTES.items(), key=lambda item: -len(item[0][1])):
            if m == method and (parsed.path == prefix or parsed.path.startswith(prefix.rstrip("/") + "/")):
                route(self, parsed)
                return
        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self) -> None:  # noqa: N802 (stdlib method name)
        self._dispatch("GET")

    def do_POST(self) -> None:  # noqa: N802
        self._dispatch("POST")

    def log_message(self, fmt: str, *args: object) -> None:  # quiet: the gate prints its own lines
        return


def main() -> None:
    port = int(os.environ.get("PORT", "8080"))
    ThreadingHTTPServer(("0.0.0.0", port), Handler).serve_forever()


if __name__ == "__main__":
    main()
