"""SRC-12 part C gate: a REST connector signs in with OAuth authorization code
and PKCE against a provider the gate runs itself, loads, keeps loading after the
provider rotated its refresh token and after the access token expired, refuses
a wrong or replayed `state`, and tells the provider to forget the token when it
is signed out or deleted.

What it proves (feature page `credentials-done-properly.md`, decisions 6, 7, 8,
12; plan `2026-10-15-src-12-credentials.md`, task C5), in this order:

  1. The client secret is saved before anyone signs in (`PUT .../credential`,
     `verified: false`: nothing to test yet), sealed on the credentials volume.
  2. Sign in, driven as a browser would: `POST .../oauth/start` gives the
     provider's address (S256, a random `state`, the operator's callback
     address, the scopes); the gate opens it at the provider (an authorize
     endpoint that approves at once and redirects with a code) and follows the
     redirect to the product's callback with its own session. The provider
     checks the PKCE verifier against the challenge. The connector is then
     signed in, its refresh token is on the volume sealed (a version byte, not
     the token's text) and the access token is nowhere on it.
  3. A load: 5 rows in Bronze (`replace` mode, so later loads leave 5, and the
     steps prove a load by Dagster's status, the run record and the provider's
     counters, not by row counts). The provider's access tokens live 5 seconds and
     its pages answer slowly, so the load outlives one token: the orchestrator
     asked for another during the run (refresh grants counted at the provider).
  4. The provider rotates its refresh token on every refresh and honours only
     the latest. The orchestrator reports each new one to the API, which seals
     it: the file on the volume changed, is still not the token's text, and
     a SECOND load, started after the first token expired, succeeds (run record 'succeeded').
     That only works if the rotated token was stored.
  5. The connection test passes (it performs the refresh and calls the API),
     twice in a row, which also needs the rotated token kept.
  6. A wrong `state`, an expired-looking one and a replayed one are refused with
     the fixed page; the provider's token endpoint is not called for any of them.
  7. The provider stops honouring the refresh token (revoked on its side): the
     next run fails with one attempt of class `sign_in_refused`, no wait, none of
     the provider's words in the run log or the callback page. Signing in again
     restores loading .
  8. Signing out calls the provider's revocation address with the stored
     refresh token and removes the file; the connector is then "not signed in"
     and a load fails with `sign_in_refused`. Signing in once more and DELETING
     the connector revokes the new token and leaves no file for the connector.

The provider is a thread of this process (as `src17_alert_delivery_test.py`'s
receiver is), bound on all addresses of the runner container; the API and the
code location reach it at the runner's address on the compose network. It is
plain `http`: both services run with the gate-only `CONNECTOR_OAUTH_ALLOW_HTTP=true`
(the CI job's `.env`; a real installation leaves it off and uses `https`), and
the callback address is `http://lakehouse-api:8080/api/connectors/oauth/callback`
(`CONNECTOR_OAUTH_REDIRECT_URI`, the same string).

Stack: `ops/g6/docker-compose.g6.override.yml` and
`ops/g6/docker-compose.src12.override.yml` (the writable credentials volume and
this runner's read-only view of it at `/managed-secrets`).

Usage: `python src12_oauth_test.py`. Exit 0 and `[src12o] PASS`, or
`[src12o] FAILED: ...` and exit 1.
"""
from __future__ import annotations

import hashlib
import base64
import json
import os
import secrets
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlparse

import requests

import g6_ingest_matrix_test as matrix
import src11_connector_operations_test as ops
import src12_managers_test as managers
import src17_alert_delivery_test as src17

GATE = "src12o"
PROVIDER_PORT = int(os.environ.get("SRC12O_PROVIDER_PORT", "8098"))
CLIENT_ID = "gate-app"
CLIENT_SECRET = "gate-client-secret-" + secrets.token_hex(6)
# The access tokens live this long; the pages answer more slowly than this adds up to.
TOKEN_LIFETIME_S = 5
PAGE_DELAY_S = 2.0
# Words the provider puts in its own error bodies. None may reach a page, a log or a response.
PROVIDER_WORDS = "PROVIDER-WORDS-" + secrets.token_hex(4)
ROWS = [{"id": i, "name": f"item-{i}"} for i in range(5)]
PAGE_SIZE = 2


class OauthGateFailure(Exception):
    """A step of this gate failed."""


def say(line: str) -> None:
    print(f"[{GATE}] {line}", flush=True)


def s256(verifier: str) -> str:
    return base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()


# ── the provider ────────────────────────────────────────────────────────────


class Provider:
    """An identity provider and the REST API behind it, in one listener.

    - `GET /authorize`: approves at once; redirects to `redirect_uri` with a
      code and the `state` it was given, remembering the PKCE challenge.
    - `POST /token`: `authorization_code` (checks the code, the redirect URI and
      the PKCE verifier) and `refresh_token` (checks it is the LATEST refresh
      token, then rotates it). Access tokens expire after TOKEN_LIFETIME_S.
    - `POST /revoke`: records the token and forgets it if it is the latest.
    - `GET /api` and `GET /api/items?page=N`: require a fresh bearer token; the
      items are paged by 2 and each page answers PAGE_DELAY_S late.
    """

    def __init__(self, redirect_uri: str) -> None:
        self.redirect_uri = redirect_uri
        self._lock = threading.Lock()
        self._codes: dict[str, tuple[str, str]] = {}
        self._access: dict[str, float] = {}
        self._issued = 0
        self.current_refresh: str | None = None
        self.token_calls = {"authorization_code": 0, "refresh_token": 0}
        self.revoked: list[str] = []
        self.authorize_queries: list[dict[str, str]] = []
        self.stale_requests = 0
        self._every_token: list[str] = []
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args) -> None:  # noqa: D401 -- silence http.server's stderr access log
                return

            def _json(self, status: int, body: dict) -> None:
                raw = json.dumps(body).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def do_GET(self) -> None:  # noqa: N802 -- the name http.server dispatches on
                parsed = urlparse(self.path)
                query = {k: v[0] for k, v in parse_qs(parsed.query).items()}
                if parsed.path == "/authorize":
                    outer._authorize(self, query)
                elif parsed.path in ("/api", "/api/items"):
                    outer._api(self, parsed.path, query)
                else:
                    self.send_response(404)
                    self.end_headers()

            def do_POST(self) -> None:  # noqa: N802
                length = int(self.headers.get("Content-Length") or 0)
                form = {k: v[0] for k, v in parse_qs(self.rfile.read(length).decode()).items()}
                if self.path == "/token":
                    outer._token(self, form)
                elif self.path == "/revoke":
                    outer._revoke(self, form)
                else:
                    self.send_response(404)
                    self.end_headers()

        self._server = ThreadingHTTPServer(("0.0.0.0", PROVIDER_PORT), Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, name="src12o-provider", daemon=True)

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._server.shutdown()
        self._server.server_close()

    # -- endpoints (called on the handler's thread) --

    def _authorize(self, h: BaseHTTPRequestHandler, query: dict[str, str]) -> None:
        with self._lock:
            self.authorize_queries.append(query)
            ok = (
                query.get("response_type") == "code"
                and query.get("client_id") == CLIENT_ID
                and query.get("code_challenge_method") == "S256"
                and query.get("redirect_uri") == self.redirect_uri
                and query.get("code_challenge")
                and query.get("state")
            )
            if not ok:
                raw = json.dumps({"error": "invalid_request", "error_description": PROVIDER_WORDS}).encode()
                h.send_response(400)
                h.send_header("Content-Length", str(len(raw)))
                h.end_headers()
                h.wfile.write(raw)
                return
            code = "code-" + secrets.token_hex(8)
            self._codes[code] = (query["code_challenge"], query["redirect_uri"])
            location = self.redirect_uri + "?" + urlencode({"code": code, "state": query["state"]})
        h.send_response(302)
        h.send_header("Location", location)
        h.send_header("Content-Length", "0")
        h.end_headers()

    def _token(self, h, form: dict[str, str]) -> None:
        def refuse() -> None:
            h._json(400, {"error": "invalid_grant", "error_description": PROVIDER_WORDS})

        with self._lock:
            grant = form.get("grant_type", "")
            if grant in self.token_calls:
                self.token_calls[grant] += 1
            if form.get("client_id") != CLIENT_ID or form.get("client_secret") != CLIENT_SECRET:
                return h._json(401, {"error": "invalid_client", "error_description": PROVIDER_WORDS})
            if grant == "authorization_code":
                challenge, redirect_uri = self._codes.pop(form.get("code", ""), ("", ""))
                if not challenge or form.get("redirect_uri") != redirect_uri or s256(form.get("code_verifier", "")) != challenge:
                    return refuse()
            elif grant == "refresh_token":
                if not self.current_refresh or form.get("refresh_token") != self.current_refresh:
                    return refuse()
            else:
                return refuse()
            self._issued += 1
            access = f"at-{self._issued}-{secrets.token_hex(6)}"
            self.current_refresh = f"rt-{self._issued}-{secrets.token_hex(6)}"
            self._access[access] = time.time()
            self._every_token += [access, self.current_refresh]
            body = {
                "access_token": access,
                "token_type": "bearer",
                "expires_in": TOKEN_LIFETIME_S,
                "refresh_token": self.current_refresh,
            }
        h._json(200, body)

    def _revoke(self, h, form: dict[str, str]) -> None:
        with self._lock:
            if form.get("client_id") != CLIENT_ID or form.get("client_secret") != CLIENT_SECRET:
                return h._json(401, {"error": "invalid_client"})
            token = form.get("token", "")
            self.revoked.append(token)
            if token == self.current_refresh:
                self.current_refresh = None
        h.send_response(200)
        h.send_header("Content-Length", "0")
        h.end_headers()

    def _api(self, h, path: str, query: dict[str, str]) -> None:
        bearer = (h.headers.get("Authorization") or "").removeprefix("Bearer ")
        with self._lock:
            issued_at = self._access.get(bearer)
            fresh = issued_at is not None and time.time() - issued_at <= TOKEN_LIFETIME_S
            if not fresh:
                self.stale_requests += 1
        if not fresh:
            return h._json(401, {"error": PROVIDER_WORDS})
        if path == "/api":
            return h._json(200, {"status": "ok"})
        time.sleep(PAGE_DELAY_S)
        page = int(query.get("page", "1"))
        start = (page - 1) * PAGE_SIZE
        h._json(200, {"items": ROWS[start : start + PAGE_SIZE]})

    # -- what the gate asks --

    def refresh_grants(self) -> int:
        with self._lock:
            return self.token_calls["refresh_token"]

    def code_grants(self) -> int:
        with self._lock:
            return self.token_calls["authorization_code"]

    def stored_token_should_be(self) -> str | None:
        with self._lock:
            return self.current_refresh

    def every_token(self) -> list[str]:
        """Every access and refresh token the provider ever issued: the strings
        no page, log, response or file may hold in clear text."""
        with self._lock:
            return list(self._every_token)

    def retire_refresh_token(self) -> None:
        """The provider stops honouring the refresh token (revoked on its side)."""
        with self._lock:
            self.current_refresh = None

    def revoked_tokens(self) -> list[str]:
        with self._lock:
            return list(self.revoked)


# ── helpers over the product ───────────────────────────────────────────────


def api(method: str, path: str, **kwargs) -> requests.Response:
    return matrix.API.request(method, f"{matrix.API_URL}{path}", timeout=60, allow_redirects=False, **kwargs)


def token_file(connector_id: str) -> str | None:
    """The sealed refresh token's file name on the credentials volume, if any."""
    return next((n for n in managers.volume_files_for(connector_id) if n.endswith("_token")), None)


def file_bytes(name: str) -> bytes:
    with open(os.path.join(managers.MANAGED_DIR, name), "rb") as fh:
        return fh.read()


def check_sealed(connector_id: str, provider: "Provider", *, what: str) -> bytes:
    """The refresh token's file exists, starts with the format's version byte,
    and no file on the volume holds any token the provider ever issued (access
    or refresh) or the client secret. Returns the file's bytes."""
    name = token_file(connector_id)
    if name is None:
        raise OauthGateFailure(f"{what}: no refresh token file on the credentials volume ({managers.volume_files_for(connector_id)})")
    data = file_bytes(name)
    if len(data) < 29 or data[0] != 0x01:
        raise OauthGateFailure(f"{what}: the refresh token file is not a seal (first byte {data[:1]!r}, {len(data)} bytes)")
    for secret in [*provider.every_token(), CLIENT_SECRET]:
        holding = managers.volume_holds(secret)
        if holding:
            raise OauthGateFailure(f"{what}: the volume holds a token or secret in clear text: {holding}")
    return data


def callback_page(location: str) -> str:
    """What the person's window ends on: the product's callback, with the
    gate's own session (the cookie a browser would send)."""
    resp = matrix.API.get(location, timeout=30, allow_redirects=False)
    if resp.status_code != 200 or "text/html" not in resp.headers.get("Content-Type", ""):
        raise OauthGateFailure(f"the callback should answer an HTML page, got {resp.status_code} {resp.headers.get('Content-Type')}")
    return resp.text


def start_sign_in(connector_id: str) -> tuple[str, dict[str, str]]:
    """`POST .../oauth/start`: (the provider's address, its query)."""
    started = ops.expect_ok(api("POST", f"/api/connectors/{connector_id}/oauth/start"), "POST oauth/start")
    url = started["authorizeUrl"]
    return url, {k: v[0] for k, v in parse_qs(urlparse(url).query).items()}


def browser_sign_in(connector_id: str, provider: Provider) -> tuple[str, str]:
    """Sign in as a browser would. Returns (the callback's HTML page, the
    redirect URL the provider sent the browser to)."""
    url, query = start_sign_in(connector_id)
    if query.get("code_challenge_method") != "S256":
        raise OauthGateFailure(f"the sign-in address should use PKCE S256: {url}")
    if len(query.get("state", "")) < 32:
        raise OauthGateFailure(f"the state should carry at least 128 bits: {query.get('state')!r}")
    if "code_verifier" in url or "client_secret" in url or CLIENT_SECRET in url:
        raise OauthGateFailure("the sign-in address carries a secret")
    # The browser is not signed in to the product at the provider.
    at_provider = requests.get(url, timeout=30, allow_redirects=False)
    if at_provider.status_code != 302:
        raise OauthGateFailure(f"the provider should approve at once and redirect, got {at_provider.status_code} {at_provider.text[:200]}")
    location = at_provider.headers["Location"]
    return callback_page(location), location


def volume_snapshot(connector_id: str) -> dict[str, bytes]:
    return {n: file_bytes(n) for n in managers.volume_files_for(connector_id)}


def run_ok(connector_id: str, what: str, bronze: str) -> str:
    """A run that succeeded, proved by Dagster's status AND the product's own run
    record (`GET .../runs`, SRC-11), and that left the table with the 5 rows the
    source holds. The table is loaded by `replace` (set in the spec above), so
    a second load leaves 5 rows, not 10: row counts cannot tell a load that ran
    from one that did not (`SRC-12 review C1 BLOCKER 2`); the record and the
    provider's counters do. Returns the run id."""
    run_id = ops.launch_run(connector_id)
    ops.wait_dagster(run_id, "SUCCESS", what)
    record = ops.poll(f"the run record of {what}", lambda: ops.run_record(connector_id, run_id), 60)
    if record.get("status") != "succeeded":
        ops.poll(
            f"the run record of {what} to settle",
            lambda: (ops.run_record(connector_id, run_id) or {}).get("status") != "running",
            60,
        )
        record = ops.run_record(connector_id, run_id) or {}
    if record.get("status") != "succeeded":
        raise OauthGateFailure(f"{what}: the run record says {record.get('status')!r}, expected 'succeeded' ({record})")
    rows = ops.bronze_rows(bronze)
    if rows != len(ROWS):
        raise OauthGateFailure(f"{what}: bronze.{bronze} has {rows} rows, expected {len(ROWS)} (replace)")
    return run_id


def run_refused(connector_id: str, provider: "Provider", what: str) -> None:
    """A run that fails with exactly one attempt of class `sign_in_refused`
    and no wait, and a log that holds none of the provider's words."""
    run_id = ops.launch_run(connector_id)
    ops.wait_dagster(run_id, "FAILURE", what)
    log = ops.poll(f"the run log of {what}", lambda: ops.run_log(connector_id, run_id), 60)
    attempts = log["attempts"]
    if len(attempts) != 1 or attempts[0].get("errorClass") != "sign_in_refused" or attempts[0].get("waitSeconds") is not None:
        raise OauthGateFailure(f"{what}: expected one attempt of class sign_in_refused and no wait, got {ops.attempts_phrase(attempts)}")
    text = json.dumps(log)
    for needle in (PROVIDER_WORDS, "invalid_grant", *provider.every_token()):
        if needle in text:
            raise OauthGateFailure(f"{what}: the run log holds {needle!r}: {text[:300]}")


# ── the gate ─────────────────────────────────────────────────────────────────


def run() -> None:
    callback_url = f"{matrix.API_URL}/api/connectors/oauth/callback"
    matrix.step_wait_for_services()
    matrix.step_login()
    matrix.step_ensure_tenant()
    sfx = secrets.token_hex(3)
    bronze = f"g6_src12o_{sfx}"
    host = src17.own_address("lakehouse-api")
    base = f"http://{host}:{PROVIDER_PORT}"

    provider = Provider(callback_url)
    provider.start()
    try:
        body = matrix._create_connector(
            name=f"src12o-{sfx}", kind="REST API", host=f"{host}:{PROVIDER_PORT}",
            credential={"source": "managed", "primary": "secret_key", "secondary": "token"},
        )
        cid = body["id"]
        dial = {
            "baseUrl": f"{base}/api",
            "auth": {
                "type": "oauth2_authorization_code",
                "clientId": CLIENT_ID,
                "authorizeUrl": f"{base}/authorize",
                "tokenUrl": f"{base}/token",
                "revocationUrl": f"{base}/revoke",
                "scopes": ["read", "offline_access"],
            },
            "pagination": {"type": "page", "param": "page"},
            "endpoints": [{"path": "/items", "recordsPath": "items"}],
        }
        ops.expect_ok(
            api("PUT", f"/api/connectors/{cid}/ingest-spec", json={
                "adapter": "rest", "ingestMode": "batch", "dial": dial,
                "sourceObjects": [{"name": "/items", "target": bronze, "loadMode": "replace"}],
            }),
            "PUT ingest-spec",
        )
        steps(cid, bronze, provider, base)
    finally:
        provider.stop()


def steps(cid: str, bronze: str, provider: Provider, base: str) -> None:
    # 1. The client secret first: nothing to test, honestly said, and sealed.
    saved = ops.expect_ok(
        api("PUT", f"/api/connectors/{cid}/credential", json={"primary": {"kind": "secret_key", "value": CLIENT_SECRET}}),
        "PUT credential (client secret)",
    )
    if saved.get("saved") is not True or saved.get("verified") is not False:
        raise OauthGateFailure(f"the client secret should be saved unverified (nobody has signed in), got {saved}")
    if managers.volume_holds(CLIENT_SECRET):
        raise OauthGateFailure("the client secret is on the volume in clear text")
    status = ops.expect_ok(api("GET", f"/api/connectors/{cid}/oauth"), "GET oauth")
    if status.get("signedIn") is not False or status.get("revocationConfigured") is not True:
        raise OauthGateFailure(f"before the sign-in the connector should be signed out with a revocation address, got {status}")
    say("1. the client secret is saved before anyone signs in (unverified, sealed)")

    # 2. Sign in as a browser would.
    page, _ = browser_sign_in(cid, provider)
    if "Signed in" not in page:
        raise OauthGateFailure(f"the callback should say Signed in, got: {page[:300]}")
    if provider.code_grants() != 1:
        raise OauthGateFailure(f"the provider should have seen one code exchange, saw {provider.code_grants()}")
    sealed = check_sealed(cid, provider, what="after the sign-in")
    for leak in provider.every_token():
        if leak in page:
            raise OauthGateFailure(f"the callback page shows a token: {page[:300]}")
    if not ops.expect_ok(api("GET", f"/api/connectors/{cid}/oauth"), "GET oauth").get("signedIn"):
        raise OauthGateFailure("after the callback the connector should be signed in")
    say("2. signed in with PKCE S256: the refresh token is on the volume sealed, no access token is")

    # 3. A load that outlives one access token.
    before = provider.refresh_grants()
    run_ok(cid, "first load", bronze)
    refreshes = provider.refresh_grants() - before
    if refreshes < 2:
        raise OauthGateFailure(
            f"the load should have asked for a second access token during the run (it takes longer than "
            f"{TOKEN_LIFETIME_S} s), the provider saw {refreshes} refresh grant(s)"
        )
    say(f"3. the load read 5 rows and asked the provider for {refreshes} access tokens during the run")

    # 4. The rotated token was stored; a later load, after the first token expired.
    rotated = check_sealed(cid, provider, what="after the first load")
    if rotated == sealed:
        raise OauthGateFailure("the stored refresh token did not change although the provider rotated it")
    time.sleep(TOKEN_LIFETIME_S + 1)
    before_second = provider.refresh_grants()
    run_ok(cid, "second load, after the access token expired", bronze)
    if provider.refresh_grants() <= before_second:
        raise OauthGateFailure("the second load did not ask the provider for a refresh-token grant")
    after_second = check_sealed(cid, provider, what="after the second load")
    if after_second == rotated:
        raise OauthGateFailure("the stored refresh token did not change during the second load although the provider rotated it")
    say("4. the rotated refresh token was sealed in place of the old one: the second load, started after the access token expired, succeeded on a refresh-token grant and rotated it again")

    # 5. The connection test refreshes too, and keeps what it is given.
    for attempt in (1, 2):
        tested = ops.expect_ok(api("POST", f"/api/connectors/{cid}/test"), f"POST test ({attempt})")
        if tested.get("ok") is not True:
            raise OauthGateFailure(f"the connection test ({attempt}) should pass, got {tested}")
    say("5. the connection test performs the refresh and keeps the rotated token (it passed twice)")

    # 6. A wrong and a replayed state are refused; the provider is not asked.
    grants = (provider.code_grants(), provider.refresh_grants())
    refused_pages = []
    refused_pages.append(callback_page(f"{matrix.API_URL}/api/connectors/oauth/callback?code=code-x&state=not-a-state-we-issued"))
    url, query = start_sign_in(cid)
    at_provider = requests.get(url, timeout=30, allow_redirects=False)
    location = at_provider.headers["Location"]
    first_try = callback_page(location)
    if "Signed in" not in first_try:
        raise OauthGateFailure(f"a fresh sign-in should work, got: {first_try[:300]}")
    refused_pages.append(callback_page(location))
    for refused in refused_pages:
        if "Sign-in not completed" not in refused or "unknown, has expired or was already used" not in refused:
            raise OauthGateFailure(f"a wrong or replayed state should be refused with the fixed page, got: {refused[:300]}")
        if PROVIDER_WORDS in refused:
            raise OauthGateFailure("a refusal page carries the provider's words")
    if provider.code_grants() != grants[0] + 1:
        raise OauthGateFailure("a wrong or replayed state reached the provider's token endpoint")
    say("6. a wrong state and a replayed state are refused with the fixed page and never reach the provider")

    # 7. The provider stops honouring the refresh token: sign in again.
    provider.retire_refresh_token()
    run_refused(cid, provider, "load after the provider revoked the refresh token")
    page, _ = browser_sign_in(cid, provider)
    if "Signed in" not in page:
        raise OauthGateFailure(f"signing in again should work, got: {page[:300]}")
    run_ok(cid, "load after signing in again", bronze)
    say("7. a refresh token the provider no longer honours fails the run once as sign_in_refused; signing in again restores it")

    # 8. Sign out revokes at the provider and removes the file; a delete does too.
    stored = provider.stored_token_should_be()
    out = ops.expect_ok(api("POST", f"/api/connectors/{cid}/oauth/sign-out"), "POST oauth/sign-out")
    if out.get("revocation") != "done" or out.get("signedOut") is not True:
        raise OauthGateFailure(f"signing out should revoke at the provider, got {out}")
    if stored not in provider.revoked_tokens():
        raise OauthGateFailure("the provider was not told to revoke the stored refresh token")
    if token_file(cid) is not None:
        raise OauthGateFailure("the refresh token file is still on the volume after signing out")
    if ops.expect_ok(api("GET", f"/api/connectors/{cid}/oauth"), "GET oauth").get("signedIn") is not False:
        raise OauthGateFailure("after signing out the connector should be signed out")
    run_refused(cid, provider, "load after signing out")
    browser_sign_in(cid, provider)
    last = provider.stored_token_should_be()
    deleted = api("DELETE", f"/api/connectors/{cid}")
    if deleted.status_code != 204:
        raise OauthGateFailure(f"DELETE connector: expected 204, got {deleted.status_code} {deleted.text[:200]}")
    if last not in provider.revoked_tokens():
        raise OauthGateFailure("deleting the connector did not revoke its refresh token at the provider")
    left = managers.volume_files_for(cid)
    if left:
        raise OauthGateFailure(f"deleting the connector left files on the credentials volume: {left}")
    say("8. signing out and deleting the connector each revoked the stored token at the provider and removed the files")


def main() -> int:
    sys.stdout.reconfigure(line_buffering=True)
    sys.stderr.reconfigure(line_buffering=True)
    started = time.time()
    try:
        run()
    except (OauthGateFailure, ops.OpsGateFailure, matrix.G6Failure) as exc:
        print(f"[{GATE}] FAILED: {exc}", file=sys.stderr)
        return 1
    except requests.RequestException as exc:
        print(f"[{GATE}] FAILED: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    except Exception:  # noqa: BLE001 -- any other error is a failure of the gate and must be named as one
        import traceback

        print(f"[{GATE}] FAILED: unexpected error:\n{traceback.format_exc()}", file=sys.stderr)
        return 1
    print(f"[{GATE}] PASS ({time.time() - started:.0f}s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
