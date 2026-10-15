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
`build_source`) or `auth["tokenUrl"]` (checked at every token request,
in `_token_request`); cursor pagination only ever re-sends to that
SAME base_url+path with a new query parameter taken from the body
(`cursorPath`), never a URL. Every request `allow_redirects=False`, and a
3xx response is treated as a REFUSAL (`SsrfBlocked`), not a thing to
follow -- a public `baseUrl`/`tokenUrl` answering
`302 Location: http://169.254.169.254/...` would otherwise be followed to
an unchecked host `pinned_resolution` never sees, since the redirect
target is a different hostname than the one that was resolved and pinned.

Implements exactly the three pagination shapes
`rust/crates/lakehouse-store/src/ingest_spec.rs`'s `RestPagination`
admits -- `none`/`page`/`cursor` -- and all five auth types
(api_key/bearer/oauth2_client_credentials/oauth2_authorization_code/basic) as a hand-rolled
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

OAUTH (SRC-12 part C, decisions 6 to 8, 12). `AuthSession` owns the access
token of the two OAuth kinds:

- `oauth2_client_credentials` asks `tokenUrl` for a token with the client id
  and secret; `oauth2_authorization_code` asks it with the stored REFRESH
  token (`grant_type=refresh_token`; the person's sign-in happened in the API).
- A token is fetched before the first request, fetched AGAIN when it has
  expired (`expires_in`, less a margin of 30 seconds or a fifth of it) and once more when the source answers
  401 to a request (`_authed_get`). Access tokens live in memory only, never
  in a file, a log line or a report.
- If the provider rotates the refresh token, the new one is sent to the API
  (`report_refresh_token`; the API seals it in place of the old one). This
  module never writes the credentials volume. A report that cannot be sent is
  logged by its fixed text and the run goes on with the token it has: the
  access token in hand is valid, and refusing the data would not bring the
  retired refresh token back. The next run then finds the connector signed
  out, which says "sign in again".
- A refresh the provider refuses (HTTP 400, 401 or 403 from the token
  address) raises `OAuthSignInRefused`, which `ingest_errors` classifies as
  `sign_in_refused` (not retried); its message is the fixed text "sign in
  again", never the provider's `error` or `error_description`. A connector
  nobody has signed in to has no refresh token to read, and is the same
  failure (`ingest_factory._resolve_object_secrets`).
- `tokenUrl` must be `https` (a gate-only `CONNECTOR_OAUTH_ALLOW_HTTP=true`
  admits `http`, for the acceptance gate's own provider), passes
  `resolve_checked`, is pinned, and no redirect is followed.
"""

from __future__ import annotations

import base64
import logging
import os
import time
from dataclasses import dataclass
from typing import Any, Callable
from urllib.parse import urlparse

import requests

from dispar_orchestrate import ssrf_guard

logger = logging.getLogger(__name__)

# The fixed text of a refused sign-in (SRC-12 decision 12): the provider's own
# words are never read into an exception, a log line or a report.
SIGN_IN_AGAIN = "sign in again"

# Seconds taken off a token's `expires_in` (at most a fifth of it) so it is
# replaced before the source sees it expire, and the bound on one token request.
_EXPIRY_MARGIN_SECONDS = 30
_TOKEN_TIMEOUT_SECONDS = 10


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


class OAuthSignInRefused(Exception):
    """The provider refused the stored credential (a refresh token that is
    revoked or expired, or a client the provider no longer accepts), or nobody
    has signed in to the connector yet. Not retried: the same answer would
    come back (`ingest_errors` classifies it `sign_in_refused`). The message
    is the fixed text `SIGN_IN_AGAIN`."""

    def __init__(self) -> None:
        super().__init__(SIGN_IN_AGAIN)


def _allow_http_token_url() -> bool:
    """The gate-only switch (`CONNECTOR_OAUTH_ALLOW_HTTP=true`, default off):
    the same word the API reads. A real installation never sets it."""
    return os.environ.get("CONNECTOR_OAUTH_ALLOW_HTTP", "").strip() == "true"


def _token_request(token_url: str, form: dict[str, str], *, resolve_checked) -> dict:
    """POST `form` to `token_url` and return the answer's JSON object.

    WS3 plan review Z5: `tokenUrl` is as caller-supplied as `baseUrl`, so it
    gets the same resolve_checked + pin + no-redirects discipline, plus an
    explicit `https` requirement (a client secret or a refresh token is never
    worth sending over plaintext even to a publicly-routable host).

    # Errors

    `ssrf_guard.SsrfBlocked` for a scheme that is not allowed, an address
    `resolve_checked` refuses or a redirect; `OAuthSignInRefused` for HTTP 400,
    401 or 403; `requests.RequestException` (timeout, connection, 5xx) and
    `ValueError` (not JSON) otherwise. None of the messages holds the
    provider's words.
    """
    parsed = urlparse(token_url)
    if parsed.scheme != "https" and not (parsed.scheme == "http" and _allow_http_token_url()):
        raise ssrf_guard.SsrfBlocked(f"oauth2 tokenUrl must be https, got {parsed.scheme!r} ({token_url})")
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    resolved = resolve_checked(parsed.hostname, port)
    with ssrf_guard.pinned_resolution(parsed.hostname, resolved):
        resp = requests.post(token_url, data=form, timeout=_TOKEN_TIMEOUT_SECONDS, allow_redirects=False)
    if 300 <= resp.status_code < 400:
        # Not `_refuse_redirect`: its message carries the Location, which is
        # the provider's text.
        raise ssrf_guard.SsrfBlocked(
            "the oauth2 token address answered with a redirect; refusing to follow a response-supplied "
            "redirect target (WS3 plan review Z5)"
        )
    if resp.status_code in (400, 401, 403):
        raise OAuthSignInRefused()
    resp.raise_for_status()
    body = resp.json()
    if not isinstance(body, dict) or not isinstance(body.get("access_token"), str) or not body["access_token"]:
        raise ValueError("the oauth2 token answer held no access token")
    return body


class AuthSession:
    """The headers a REST request carries, and (for the OAuth kinds) the access
    token behind them: fetched when first needed, again when it has expired,
    and again on demand (`refresh`) after a 401. See the module docstring's
    OAUTH section.

    `report_refresh_token`, when given, is called with a refresh token the
    provider issued in place of the one used; it is never called with anything
    else, and an exception from it is logged (fixed text) and not raised.
    """

    def __init__(
        self,
        auth: dict,
        secrets: dict[str, str],
        *,
        resolve_checked=ssrf_guard.resolve_checked,
        clock: Callable[[], float] = time.monotonic,
        report_refresh_token: Callable[[str], None] | None = None,
    ) -> None:
        self._auth = auth
        self._type = auth.get("type", "basic")
        self._secrets = secrets
        self._resolve_checked = resolve_checked
        self._clock = clock
        self._report = report_refresh_token
        self._refresh_token = secrets.get("refreshToken")
        self._headers: dict | None = None
        self._expires_at: float | None = None

    @property
    def refreshable(self) -> bool:
        """Whether a new access token can be asked for (the two OAuth kinds)."""
        return self._type in ("oauth2_client_credentials", "oauth2_authorization_code")

    def headers(self) -> dict:
        """The auth headers for the next request; an OAuth token that has
        expired is replaced first. Never logs or returns a resolved secret
        beyond placing it in the one header it belongs in."""
        expired = self._expires_at is not None and self._clock() >= self._expires_at
        if self._headers is None or (self.refreshable and expired):
            self._headers = self._build()
        return self._headers

    def refresh(self) -> None:
        """Forget the access token, so the next `headers()` asks for a new one."""
        self._headers = None
        self._expires_at = None

    def _build(self) -> dict:
        auth, secrets, auth_type = self._auth, self._secrets, self._type
        if auth_type == "api_key":
            return {auth.get("header", "X-Api-Key"): secrets["apiKey"]}
        if auth_type == "bearer":
            return {"Authorization": f"Bearer {secrets['token']}"}
        if auth_type == "basic":
            raw = f"{secrets['username']}:{secrets['password']}".encode()
            return {"Authorization": f"Basic {base64.b64encode(raw).decode()}"}
        if auth_type == "oauth2_client_credentials":
            body = _token_request(
                auth["tokenUrl"],
                {
                    "grant_type": "client_credentials",
                    "client_id": secrets["clientId"],
                    "client_secret": secrets["clientSecret"],
                },
                resolve_checked=self._resolve_checked,
            )
            return self._with_token(body)
        if auth_type == "oauth2_authorization_code":
            if not self._refresh_token:
                raise OAuthSignInRefused()
            body = _token_request(
                auth["tokenUrl"],
                {
                    "grant_type": "refresh_token",
                    "refresh_token": self._refresh_token,
                    "client_id": auth["clientId"],
                    "client_secret": secrets["clientSecret"],
                },
                resolve_checked=self._resolve_checked,
            )
            self._keep_rotated(body.get("refresh_token"))
            return self._with_token(body)
        raise ValueError(f"rest adapter does not know auth type {auth_type!r}")

    def _with_token(self, body: dict) -> dict:
        expires_in = body.get("expires_in")
        if isinstance(expires_in, (int, float)) and not isinstance(expires_in, bool) and expires_in > 0:
            # A margin, so the token is replaced before the source sees it expire:
            # 30 seconds, or a fifth of a shorter lifetime (a 5-second token is
            # replaced after 4, not at once).
            margin = min(_EXPIRY_MARGIN_SECONDS, expires_in * 0.2)
            self._expires_at = self._clock() + (expires_in - margin)
        else:
            # No stated lifetime: replaced only after a 401.
            self._expires_at = None
        return {"Authorization": f"Bearer {body['access_token']}"}

    def _keep_rotated(self, issued: object) -> None:
        if not isinstance(issued, str) or not issued or issued == self._refresh_token:
            return
        # Held in memory first: the next refresh of this run must use it.
        self._refresh_token = issued
        if self._report is None:
            return
        try:
            self._report(issued)
        except Exception as exc:  # noqa: BLE001 -- see the module docstring: logged by type, never raised
            logger.warning(
                "the provider rotated the refresh token and it could not be sent to the API (%s); "
                "the next run may need a new sign-in",
                type(exc).__name__,
            )


def _auth_headers_and_params(
    auth: dict, secrets: dict[str, str], *, resolve_checked=ssrf_guard.resolve_checked
) -> tuple[dict, dict]:
    """The headers (and no query parameters) a first request carries. Kept as
    the one-shot form of `AuthSession` for callers and tests that do not
    paginate."""
    return AuthSession(auth, secrets, resolve_checked=resolve_checked).headers(), {}


def _http_status(exc: BaseException) -> int | None:
    status = getattr(getattr(exc, "response", None), "status_code", None)
    return status if isinstance(status, int) else None


def _authed_get(http_get: Callable, session: AuthSession) -> Callable:
    """`http_get` with the session's headers, and one new access token when the
    source answers 401 (SRC-12 decision 7). A second 401 is raised: the
    credential, not the token, is what the source refuses."""

    def _call(url: str, *, params: dict):
        try:
            return http_get(url, headers=session.headers(), params=params)
        except requests.HTTPError as exc:
            if session.refreshable and _http_status(exc) == 401:
                session.refresh()
                return http_get(url, headers=session.headers(), params=params)
            raise

    return _call


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


def _paginate(base_url: str, endpoint: dict, pagination: dict, get: Callable):
    path = endpoint["path"]
    records_path = endpoint.get("recordsPath")
    ptype = pagination.get("type", "none")

    if ptype == "none":
        yield from _extract(get(base_url + path, params={}), records_path)
        return
    if ptype == "page":
        param = pagination["param"]
        page = 1
        while True:
            rows = _extract(get(base_url + path, params={param: page}), records_path)
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
            body = get(base_url + path, params=params)
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


def report_refresh_token_to_api(connector_id: str) -> Callable[[str], None]:
    """The report a run sends when the provider rotates the refresh token
    (`POST /api/connectors/{id}/oauth/refresh-token`, the service identity of
    the run reports; the API seals the token). The orchestrator never writes
    the credentials volume, so this is the only way a new refresh token is kept.

    Imported here, not at the top: `schema_observer` imports `ingest_factory`
    lazily and this module is imported by it.
    """

    def _report(refresh_token: str) -> None:
        from dispar_orchestrate import schema_observer

        schema_observer._post_json(
            schema_observer.ObserverConfig.from_env(),
            f"/api/connectors/{connector_id}/oauth/refresh-token",
            {"refreshToken": refresh_token},
            "rotated refresh token",
        )

    return _report


def _prepare(
    spec: dict,
    secrets: dict[str, str],
    *,
    resolve_checked,
    http_get: Callable | None,
    connector_id: str | None = None,
    report_refresh_token: Callable[[str], None] | None = None,
) -> tuple[Callable, ssrf_guard.ResolvedAddress]:
    """Resolve and check `baseUrl`, build the auth session, and pick the GET: the
    part of a read that `build_source` and `first_page_records` share, so a
    preview dials, authenticates and refuses redirects exactly as a load does.

    The first access token is fetched here (so a refused sign-in or a blocked
    token address is the failure of the build, as before), and replaced later
    by the returned GET when it expires or the source answers 401.

    The returned GET takes `(url, *, params)`; the headers are the session's.
    """
    parsed = urlparse(spec["baseUrl"])
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    resolved = resolve_checked(parsed.hostname, port)

    auth = spec["auth"]
    if report_refresh_token is None and connector_id and auth.get("type") == "oauth2_authorization_code":
        report_refresh_token = report_refresh_token_to_api(connector_id)
    session = AuthSession(auth, secrets, resolve_checked=resolve_checked, report_refresh_token=report_refresh_token)
    session.headers()

    def _real_http_get(url, *, headers, params):
        # WS3 plan review Z5: allow_redirects=False -- a redirect to an
        # unchecked host must never be followed silently.
        resp = requests.get(url, headers=headers, params=params, timeout=30, allow_redirects=False)
        _refuse_redirect(resp, what=f"GET {url}")
        resp.raise_for_status()
        return resp.json()

    return _authed_get(http_get or _real_http_get, session), resolved


def build_source(
    spec: dict,
    secrets: dict[str, str],
    source_objects: list[dict] | None = None,
    *,
    resolve_checked=ssrf_guard.resolve_checked,
    http_get: Callable | None = None,
    connector_id: str | None = None,
    report_refresh_token: Callable[[str], None] | None = None,
) -> AdapterBuildResult:
    # SRC-13 decision D3: a source object that NAMES a declared endpoint (its
    # `name` is that endpoint's `path`, which is what the console's Find flow
    # writes) loads that endpoint only. Any other object -- one saved before
    # SRC-13, whose `name` was a target name -- keeps the old behaviour and loads
    # every declared endpoint, so a saved connector does not change what it reads
    # until the object is picked again (the console says so on such a row).
    # The positional argument is still the ONE call shape ingest_factory.py uses
    # for every adapter (WS3 plan review Z6).
    names = {obj.get("name") for obj in (source_objects or [])}
    endpoints = [e for e in spec["endpoints"] if e["path"] in names]
    if not endpoints:
        endpoints = list(spec["endpoints"])
    get, resolved = _prepare(
        spec,
        secrets,
        resolve_checked=resolve_checked,
        http_get=http_get,
        connector_id=connector_id,
        report_refresh_token=report_refresh_token,
    )

    def _rows():
        for endpoint in endpoints:
            yield from _paginate(spec["baseUrl"], endpoint, spec["pagination"], get)

    return AdapterBuildResult(source=_rows(), resolved=resolved)


def first_page_records(
    spec: dict,
    secrets: dict[str, str],
    endpoint_path: str,
    *,
    limit: int,
    resolve_checked=ssrf_guard.resolve_checked,
    http_get: Callable | None = None,
    connector_id: str | None = None,
    report_refresh_token: Callable[[str], None] | None = None,
) -> list:
    """The records of the FIRST page of one declared endpoint, at most `limit`
    (SRC-13 decision D6: "REST: the first page"). The same address check, auth,
    no-redirect GET, first-request parameters (`page` pagination asks for page 1)
    and `recordsPath` a load uses, so what a preview shows is what a load would
    read first. One request only: the preview never follows a page or a cursor.

    The request is made here, so this function pins the resolution itself
    (`ssrf_guard.pinned_resolution`); a load pins in its caller because it reads
    later.

    # Errors

    `ValueError` when `endpoint_path` is not one of `spec["endpoints"]`;
    `ssrf_guard.SsrfBlocked` for a blocked or redirecting target; whatever the
    request raises (`requests.RequestException`, `KeyError`/`TypeError` for a body
    that is not the declared shape).
    """
    endpoint = next((e for e in spec["endpoints"] if e["path"] == endpoint_path), None)
    if endpoint is None:
        raise ValueError("the endpoint is not one the connector declares")
    get, resolved = _prepare(
        spec,
        secrets,
        resolve_checked=resolve_checked,
        http_get=http_get,
        connector_id=connector_id,
        report_refresh_token=report_refresh_token,
    )
    pagination = spec.get("pagination") or {}
    params = {pagination["param"]: 1} if pagination.get("type") == "page" else {}
    with ssrf_guard.pinned_resolution(urlparse(spec["baseUrl"]).hostname, resolved):
        body = get(spec["baseUrl"] + endpoint["path"], params=params)
    records = _extract(body, endpoint.get("recordsPath"))
    if not isinstance(records, list):
        raise TypeError("the endpoint's records are not a list")
    return records[:limit]
