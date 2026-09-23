"""A fixed, page-paginated JSON fixture server for the G6 gate's rest
adapter test -- no external dependency, stdlib only.

Serves records nested under an `"items"` key (`recordsPath: "items"` on
the connector's `dial.endpoints[].recordsPath`) and paginates by page
number (`pagination: {"type": "page", "param": "page"}`) -- the real
contract shape `rust/crates/lakehouse-store/src/ingest_spec.rs`'s
`RestEndpoint`/`RestPagination` admit, and the shape
`dagster/dispar_orchestrate/adapters/rest.py` now actually reads
(`ec25a5d` fixed that adapter to read `recordsPath`, not the `dataPath`
key the contract never accepted, and removed the dead `"offset"`
pagination branch nothing on the contract side could ever select). This
fixture previously served a bare array with `pagination: "none"`
specifically to route around those two bugs; now that they are fixed,
it exercises the real, documented shape instead.
"""
from __future__ import annotations

import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

_ROWS = [{"id": i, "name": f"item-{i}"} for i in range(5)]
_PAGE_SIZE = 2


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 (stdlib method name)
        parsed = urlparse(self.path)
        if parsed.path != "/items":
            self.send_response(404)
            self.end_headers()
            return
        page = int(parse_qs(parsed.query).get("page", ["1"])[0])
        start = (page - 1) * _PAGE_SIZE
        page_rows = _ROWS[start : start + _PAGE_SIZE]
        body = json.dumps({"items": page_rows}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
