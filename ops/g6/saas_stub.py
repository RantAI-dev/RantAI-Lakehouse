"""The mock vendor server of the SaaS gate (`g6-saas`, SRC-3 AD9), stdlib only
like `rest_stub.py`.

One container answers for every vendor. A vendor's real API is at the vendor's
own host and the connector never takes an address from a person, so the gate
points a vendor at the stub with the gate-only `SAAS_GATE_ORIGINS` (see
`adapters/saas.py`); the stub therefore gives each vendor its OWN PORT
(`VENDOR_PORTS`) and the vendor's requests arrive with the vendor's real paths
(`/crm/v3/objects/deals`), not a prefix. Port 8080 serves `GET /health` and the
admin routes the gate uses to change the mock's state (`POST /__admin/<vendor>`).

Each vendor's routes live in `saas_stub_<vendor>.py` (slices 2 to 6) and behave
as the vendor does: its sign-in, its listing, its paging by its own shape, a
cursor filter, a delete where the vendor reports deletes, a 429 with
`Retry-After`, a changed field, the vendor's own error bodies (which the product
must never repeat). Nothing here is a real vendor's data or a real credential.

# Registering a vendor

`ROUTES` maps `(method, path prefix)` to a handler `f(handler, parsed_url)` that
writes the whole response. A vendor module registers its routes under
`/<vendor id>/<its path>` and its admin route under `/__admin/<vendor id>`; the
handler adds the `/<vendor id>` for a request that arrived on the vendor's port.
An unknown path answers 404 with an empty body.
"""
from __future__ import annotations

import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Callable
from urllib.parse import ParseResult, parse_qs, urlparse

Route = Callable[[BaseHTTPRequestHandler, ParseResult], None]

ROUTES: dict[tuple[str, str], Route] = {}

# One port per vendor, in the build order of decision 12. The gate reads this
# table to build `SAAS_GATE_ORIGINS`, so the two cannot drift.
VENDOR_PORTS: dict[str, int] = {
    "hubspot": 8101,
    "zendesk": 8102,
    "salesforce": 8103,
    "ga4": 8104,
    "servicenow": 8105,
    "confluence": 8106,
    "sharepoint": 8107,
    "dynamics365": 8108,
    "workday": 8109,
    "netsuite": 8110,
}
_PORT_TO_VENDOR = {port: vendor for vendor, port in VENDOR_PORTS.items()}

STATE_LOCK = threading.Lock()  # the handler threads share each vendor's state


def read_body(handler: BaseHTTPRequestHandler) -> bytes:
    return handler.rfile.read(int(handler.headers.get("Content-Length") or 0))


def read_json(handler: BaseHTTPRequestHandler) -> object:
    raw = read_body(handler)
    return json.loads(raw) if raw else {}


def read_form(handler: BaseHTTPRequestHandler) -> dict[str, str]:
    return {k: v[0] for k, v in parse_qs(read_body(handler).decode()).items()}


def origin_of(handler: BaseHTTPRequestHandler) -> str:
    """`http://<Host header>`: the origin the caller used, for the absolute
    next-page links vendors put in their answers (the product follows them only
    on the same origin)."""
    return f"http://{handler.headers.get('Host')}"


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
    vendors = sorted({prefix.split("/")[1] for (_m, prefix) in ROUTES if prefix.count("/") > 1 and not prefix.startswith("/__admin")})
    send_json(handler, {"status": "ok", "vendors": vendors})


ROUTES[("GET", "/health")] = _health


class Handler(BaseHTTPRequestHandler):
    def _dispatch(self, method: str) -> None:
        parsed = urlparse(self.path)
        vendor = _PORT_TO_VENDOR.get(self.server.server_port)
        path = f"/{vendor}{parsed.path}" if vendor else parsed.path
        # The longest registered prefix wins, so a vendor can route a subtree.
        for (m, prefix), route in sorted(ROUTES.items(), key=lambda item: -len(item[0][1])):
            if m == method and (path == prefix or path.startswith(prefix.rstrip("/") + "/")):
                route(self, parsed)
                return
        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self) -> None:  # noqa: N802 (stdlib method name)
        self._dispatch("GET")

    def do_POST(self) -> None:  # noqa: N802
        self._dispatch("POST")

    def do_DELETE(self) -> None:  # noqa: N802
        self._dispatch("DELETE")

    def log_message(self, fmt: str, *args: object) -> None:  # quiet: the gate prints its own lines
        return


def load_vendors() -> list[str]:
    """Import every `saas_stub_<vendor>.py` that exists (each registers its
    routes on import); the vendors that have none are simply not served."""
    loaded = []
    for vendor in VENDOR_PORTS:
        try:
            __import__(f"saas_stub_{vendor}")
        except ModuleNotFoundError as exc:
            if exc.name != f"saas_stub_{vendor}":
                raise
        else:
            loaded.append(vendor)
    return loaded


def main() -> None:
    admin_port = int(os.environ.get("PORT", "8080"))
    for vendor in load_vendors():
        server = ThreadingHTTPServer(("0.0.0.0", VENDOR_PORTS[vendor]), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
    ThreadingHTTPServer(("0.0.0.0", admin_port), Handler).serve_forever()


if __name__ == "__main__":
    # The vendor modules `import saas_stub`; run it under that name so they
    # register into the same ROUTES this process serves (as `__main__` it would
    # be a second copy with empty routes).
    import saas_stub

    saas_stub.main()
