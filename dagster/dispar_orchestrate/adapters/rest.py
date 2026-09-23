"""dagster/dispar_orchestrate/adapters/rest.py -- generic REST source for
a `rest`-adapter connector.

SSRF (Z1, Z5): `dial.baseUrl` is caller-supplied -- same threat model as
sql.py/files.py. `build_source` resolves and checks it BEFORE the first
request. requests/urllib3 resolve via socket.getaddrinfo -- FULL pinning
applies (the caller, ingest_factory.py, wraps iteration in
ssrf_guard.pinned_resolution using the returned address).

INVARIANT (WS3 plan review Z5): no URL taken from a response body or a
response HEADER is ever dialed by this module. Every request target is
either `spec["baseUrl"] + endpoint["path"]` (checked once, at the top of
`build_source`) or `auth["tokenUrl"]` (checked once, at the top of
`_auth_headers_and_params`); cursor pagination only ever re-sends to that
SAME base_url+path with a new query parameter taken from the body
(`cursorPath`), never a URL. Every request `allow_redirects=False`, and a
3xx response is treated as a REFUSAL (`SsrfBlocked`), not a thing to
follow -- a public `baseUrl`/`tokenUrl` answering
`302 Location: http://169.254.169.254/...` would otherwise be followed to
an unchecked host `pinned_resolution` never sees, since the redirect
target is a different hostname than the one that was resolved and pinned.

Implements exactly the three pagination shapes
`rust/crates/lakehouse-store/src/ingest_spec.rs`'s `RestPagination`
admits -- `none`/`page`/`cursor` -- and all four auth types
(api_key/bearer/oauth2_client_credentials/basic) as a hand-rolled
generator, not dlt.sources.rest_api -- this plan has not verified that
library's constructor matches these shapes exactly, and a hand-rolled
generator is fully specified and testable without it.

FIELD NAMES MATCH THE CONTRACT, NOT THIS MODULE'S OWN EARLIER GUESS
(the fix for the bug the G6 gate's `rest_stub.py` reports): `RestDial`
is `deny_unknown_fields`, so the console/API validate and store exactly
these keys, and this adapter must read the SAME ones:
- `RestEndpoint.records_path` (JSON `recordsPath`) -- the JSON path in a
  response body naming the record array; absent means the response body
  IS the array. This module used to read a `dataPath` key the contract
  has never had, so any endpoint that set `recordsPath` (the only name
  `PUT .../ingest-spec` accepts) silently extracted nothing here.
- `RestPagination::Page { param }` (JSON `{"type": "page", "param": ...}`)
  -- the query parameter name carrying the page number.
- `RestPagination::Cursor { cursor_field }` (JSON
  `{"type": "cursor", "cursorField": ...}`) -- the JSON path in a
  response body naming the next cursor. The contract gives cursor
  pagination no SEPARATE outgoing query-parameter name the way `page`
  has `param`, so this adapter re-sends the cursor value under that same
  `cursorField` name -- the one name the contract carries for this
  variant, not a second, invented one.
- There is no `offset` variant: `RestPagination` has exactly three
  variants (`none`/`page`/`cursor`; `#[serde(tag = "type")]`, so a
  `pagination.type` naming anything else, including `"offset"`, is
  rejected 400 by `Dial::parse` before an ingest-spec is ever stored).
  This module used to implement a fourth, `offset`-typed branch nothing
  on the contract side could ever select -- dead code presented as a
  supported shape. Removed rather than kept "for completeness": AGENTS.md
  principle 2 prefers "unsupported, honestly" over dead code that claims
  a capability the contract does not admit.
"""

from __future__ import annotations

import base64
from dataclasses import dataclass
from typing import Any, Callable
from urllib.parse import urlparse

import requests

from dispar_orchestrate import ssrf_guard


@dataclass(frozen=True)
class AdapterBuildResult:
    source: Any
    resolved: ssrf_guard.ResolvedAddress


def _refuse_redirect(resp, *, what: str) -> None:
    if 300 <= resp.status_code < 400:
        location = getattr(resp, "headers", {}).get("Location", "<none>")
        raise ssrf_guard.SsrfBlocked(
            f"{what} responded {resp.status_code} redirecting to {location!r}; refusing to "
            "follow a response-supplied redirect target (WS3 plan review Z5)"
        )


def _auth_headers_and_params(
    auth: dict, secrets: dict[str, str], *, resolve_checked=ssrf_guard.resolve_checked
) -> tuple[dict, dict]:
    """Never logs or returns a resolved secret beyond placing it in the
    one header/param it belongs in."""
    auth_type = auth.get("type", "basic")
    if auth_type == "api_key":
        return {auth.get("header", "X-Api-Key"): secrets["apiKey"]}, {}
    if auth_type == "bearer":
        return {"Authorization": f"Bearer {secrets['token']}"}, {}
    if auth_type == "basic":
        raw = f"{secrets['username']}:{secrets['password']}".encode()
        return {"Authorization": f"Basic {base64.b64encode(raw).decode()}"}, {}
    if auth_type == "oauth2_client_credentials":
        # WS3 plan review Z5: `tokenUrl` is as caller-supplied as
        # `baseUrl` -- same resolve_checked + pin + no-redirects
        # discipline, plus an explicit `https` requirement (a client
        # secret is never worth sending over plaintext even to a
        # publicly-routable host).
        token_url = auth["tokenUrl"]
        parsed = urlparse(token_url)
        if parsed.scheme != "https":
            raise ssrf_guard.SsrfBlocked(
                f"oauth2 tokenUrl must be https, got {parsed.scheme!r} ({token_url})"
            )
        port = parsed.port or 443
        resolved = resolve_checked(parsed.hostname, port)
        with ssrf_guard.pinned_resolution(parsed.hostname, resolved):
            resp = requests.post(
                token_url,
                data={
                    "grant_type": "client_credentials",
                    "client_id": secrets["clientId"],
                    "client_secret": secrets["clientSecret"],
                },
                timeout=10,
                allow_redirects=False,
            )
        _refuse_redirect(resp, what=f"oauth2 tokenUrl {token_url}")
        resp.raise_for_status()
        return {"Authorization": f"Bearer {resp.json()['access_token']}"}, {}
    raise ValueError(f"rest adapter does not know auth type {auth_type!r}")


def _extract(body: dict | list, json_path: str | None):
    """Walk a dotted JSON path (`RestEndpoint.records_path` or
    `RestPagination::Cursor.cursor_field`) from a response body. No path
    means the body itself IS the value (a bare record array at the root,
    or -- for a cursor -- a top-level cursor field with no path)."""
    node = body
    if json_path:
        for part in json_path.split("."):
            node = node[part]
    return node


def _paginate(base_url: str, endpoint: dict, pagination: dict, headers: dict, http_get: Callable):
    path = endpoint["path"]
    records_path = endpoint.get("recordsPath")
    ptype = pagination.get("type", "none")

    if ptype == "none":
        yield from _extract(http_get(base_url + path, headers=headers, params={}), records_path)
        return
    if ptype == "page":
        param = pagination["param"]
        page = 1
        while True:
            rows = _extract(http_get(base_url + path, headers=headers, params={param: page}), records_path)
            if not rows:
                return
            yield from rows
            page += 1
        return
    if ptype == "cursor":
        # The contract's Cursor variant carries only cursor_field (the
        # response-body path naming the next cursor) -- no separate
        # outgoing-parameter name the way Page carries `param` -- so the
        # cursor is re-sent under that SAME name (see this module's
        # docstring, "FIELD NAMES MATCH THE CONTRACT").
        cursor_field = pagination["cursorField"]
        params: dict = {}
        while True:
            body = http_get(base_url + path, headers=headers, params=params)
            rows = _extract(body, records_path)
            yield from rows
            try:
                cursor = _extract(body, cursor_field)
            except (KeyError, TypeError):
                # The next-cursor field is absent from this response --
                # the conventional "no more pages" signal, same as an
                # explicit null value, not a malformed response.
                cursor = None
            if not cursor:
                return
            params = {cursor_field: cursor}
        return
    raise ValueError(f"rest adapter does not know pagination type {ptype!r}")


def build_source(
    spec: dict,
    secrets: dict[str, str],
    source_objects: list[dict] | None = None,
    *,
    resolve_checked=ssrf_guard.resolve_checked,
    http_get: Callable | None = None,
) -> AdapterBuildResult:
    # WS3 plan review Z6: `source_objects` is accepted (and ignored) so
    # ingest_factory.py's ONE call shape (`adapter.build_source(dial,
    # secrets, [obj])`) works uniformly across sql/files/rest -- a REST
    # connector's "objects" are its `dial.endpoints`, declared in the
    # dial itself, not a separate `source_objects` list the way sql/files
    # need one; the parameter exists here purely for call-site arity, not
    # because this adapter reads it.
    del source_objects
    parsed = urlparse(spec["baseUrl"])
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    resolved = resolve_checked(parsed.hostname, port)

    headers, _params = _auth_headers_and_params(spec["auth"], secrets, resolve_checked=resolve_checked)

    def _real_http_get(url, *, headers, params):
        # WS3 plan review Z5: allow_redirects=False -- a redirect to an
        # unchecked host must never be followed silently.
        resp = requests.get(url, headers=headers, params=params, timeout=30, allow_redirects=False)
        _refuse_redirect(resp, what=f"GET {url}")
        resp.raise_for_status()
        return resp.json()

    get = http_get or _real_http_get

    def _rows():
        for endpoint in spec["endpoints"]:
            yield from _paginate(spec["baseUrl"], endpoint, spec["pagination"], headers, get)

    return AdapterBuildResult(source=_rows(), resolved=resolved)
