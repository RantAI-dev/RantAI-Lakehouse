#!/usr/bin/env python3
"""G10 acceptance test for SEC-3 (decision D1): a real sign-in through a real
OIDC provider, end to end, as the gate CI runs on every pull request.

`ops/oidc-mock` is not a real identity provider by its own admission, and G4
already proves the authorization-code round trip against it. What G4 does NOT
prove -- the issuer string, where the signing keys are published, which claim
carries the groups, and whether a group arrives as a name or an opaque id --
is exactly what this gate exercises against Keycloak
(`ops/sso/docker-compose.keycloak.override.yml`), driven the way a browser is:

1. the discovery document is reachable and its `issuer`, `authorization_endpoint`,
   `token_endpoint` and `jwks_uri` are the ones `lakehouse-api` is configured
   with (a wrong `jwks_uri`, e.g. `{issuer}/.well-known/jwks.json`, fails here);
2. user A signs in through Keycloak's real login form, the API sets its session
   cookie on the callback, and `GET /api/auth/me` carries the mapped role's
   `query:read` permission -- this is the group-claim path, real;
3. user B, in no group, signs in and gets NO mapped role;
4. a token minted for a different audience is refused (401);
5. a token past its expiry is refused (401), with the per-client short lifespan
   the realm sets for this step asserted rather than assumed.

# What this gate deliberately does not do

It does not test Entra ID or Google (decision D2: those run manually and are
recorded in `docs/plans/SEC-3-RESULT.md`), and it makes no change to
`ops/oidc-mock` or G4. It reads no Bronze Iceberg table, so the
`check_bare_iceberg_count` rule does not apply here.

# Budgets, and their basis

`READY_BUDGET_SECONDS` covers Keycloak's `start-dev` boot plus the API's boot
and migration on a cold stack. Measured: in the T2 run on 2026-10-16 the stack
was already ready at the gate's first poll (`up -d` to gate start was 34 s with
the images local and 74 s on the first, cold run that also pulled the Keycloak
and python images); 180 s is headroom over that worst observed cold start for
CI, where the images are pulled fresh. The expiry step's
`NEGATIVE_EXPIRY_MARGIN_SECONDS` is not a latency budget: it is the largest
lifespan that still counts as "short", so the realm's per-client
`access.token.lifespan` override is proven to have taken effect.
"""
from __future__ import annotations

import base64
import json
import os
import sys
import time
import urllib.parse
from html.parser import HTMLParser

import requests

API_URL = os.environ.get("API_URL", "http://lakehouse-api:8080")
KEYCLOAK_URL = os.environ.get("KEYCLOAK_URL", "http://keycloak:8080")
REALM = os.environ.get("KEYCLOAK_REALM", "lakehouse")
# The issuer, and the three endpoints the API is configured with in the gate's
# override. Keycloak serves keys at the OIDC `jwks_uri`
# (`.../protocol/openid-connect/certs`), NOT at `{issuer}/.well-known/jwks.json`
# (the API's default), which is one of the things this gate exists to prove.
ISSUER = f"{KEYCLOAK_URL}/realms/{REALM}"
AUTHORIZE_URL = f"{ISSUER}/protocol/openid-connect/auth"
TOKEN_URL = f"{ISSUER}/protocol/openid-connect/token"
JWKS_URL = f"{ISSUER}/protocol/openid-connect/certs"
DISCOVERY_URL = f"{ISSUER}/.well-known/openid-configuration"
API_CALLBACK = f"{API_URL}/api/auth/oidc/callback"

CLIENT_ID = os.environ.get("OIDC_CLIENT_ID", "lakehouse")
USER_A = os.environ.get("SSO_USER_A", "alice")
USER_A_PASSWORD = os.environ.get("SSO_USER_A_PASSWORD", "")
USER_B = os.environ.get("SSO_USER_B", "bob")
USER_B_PASSWORD = os.environ.get("SSO_USER_B_PASSWORD", "")
WRONG_AUD_CLIENT_ID = os.environ.get("WRONG_AUD_CLIENT_ID", "lakehouse-wrongaud")
EXPIRED_CLIENT_ID = os.environ.get("EXPIRED_CLIENT_ID", "lakehouse-expired")

# The role `groups=analysts` maps to (`OIDC_ROLE_MAP=analysts=Analyst`) grants
# these; `query:read` is the one this gate asserts on.
MAPPED_PERMISSION = "query:read"

READY_BUDGET_SECONDS = 180.0  # basis: 34-74 s measured cold start, see the module docstring
# See the module docstring: this bounds what still counts as the realm's "short"
# lifespan, not how long anything is allowed to take.
NEGATIVE_EXPIRY_MARGIN_SECONDS = 30


class G10Failure(Exception):
    pass


def _wait_for(name: str, check, timeout_s: float, interval_s: float = 2.0):
    """Poll `check` until it returns a non-None value or `timeout_s` elapses.

    Mirrors `ops/g4/g4_test.py::_wait_for` (each g* gate is a standalone
    entrypoint with no shared module to import it from).
    """
    deadline = time.time() + timeout_s
    last_err = None
    while time.time() < deadline:
        try:
            value = check()
            if value is not None:
                elapsed = timeout_s - (deadline - time.time())
                print(f"[g10] {name}: visible after {elapsed:.1f}s")
                return value
        except Exception as exc:  # noqa: BLE001
            last_err = exc
        time.sleep(interval_s)
    raise G10Failure(f"timed out after {timeout_s:.0f}s waiting for {name}: {last_err}")


def _api_health():
    try:
        resp = requests.get(f"{API_URL}/health", timeout=5)
    except requests.RequestException:
        return None
    return True if resp.status_code == 200 else None


def _keycloak_discovery():
    try:
        resp = requests.get(DISCOVERY_URL, timeout=5)
        if resp.status_code != 200:
            return None
        return resp.json()
    except (requests.RequestException, ValueError):
        return None


class _FormParser(HTMLParser):
    """Collect every <form>'s action URL and its hidden input values."""

    def __init__(self) -> None:
        super().__init__()
        self.forms: list[dict] = []
        self._current: dict | None = None

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = dict(attrs)
        if tag == "form":
            self._current = {"action": attributes.get("action", ""), "inputs": {}}
            self.forms.append(self._current)
        elif tag == "input" and self._current is not None:
            name = attributes.get("name")
            if name:
                self._current["inputs"][name] = attributes.get("value", "")

    def handle_endtag(self, tag: str) -> None:
        if tag == "form":
            self._current = None


def _find_login_form(html: str) -> dict | None:
    """The Keycloak username/password form, identified by its action path.

    Keycloak names the action `.../login-actions/authenticate?...`; fall back to
    the first form that carries a `password` input so a Keycloak UI change fails
    on the missing form rather than on a stale hunch about the path.
    """
    parser = _FormParser()
    parser.feed(html)
    for form in parser.forms:
        if "login-actions/authenticate" in form["action"]:
            return form
    for form in parser.forms:
        if "password" in form["inputs"]:
            return form
    return None


def _jwt_payload(token: str) -> dict:
    parts = token.split(".")
    if len(parts) != 3:
        raise G10Failure("expected a JWT (three dot-separated parts)")
    padding = "=" * (-len(parts[1]) % 4)
    return json.loads(base64.urlsafe_b64decode(parts[1] + padding))


def _audience_list(claim) -> list[str]:
    if claim is None:
        return []
    return [claim] if isinstance(claim, str) else list(claim)


def _sign_in_through_keycloak(session: requests.Session, username: str, password: str) -> None:
    """Drive one full authorization-code + PKCE login as a browser would, with
    `allow_redirects=False` at every hop so each redirect is asserted."""
    start = session.get(f"{API_URL}/api/auth/oidc/start", allow_redirects=False, timeout=10)
    if start.status_code != 302:
        raise G10Failure(f"/api/auth/oidc/start returned {start.status_code}, expected 302")
    redirect = start.headers.get("Location", "")
    if not redirect.startswith(AUTHORIZE_URL):
        raise G10Failure(f"start redirected to {redirect!r}, not Keycloak's authorize endpoint {AUTHORIZE_URL!r}")
    query = urllib.parse.parse_qs(urllib.parse.urlsplit(redirect).query)
    if query.get("code_challenge_method") != ["S256"]:
        raise G10Failure(f"the flow is not using PKCE S256: code_challenge_method={query.get('code_challenge_method')}")
    if query.get("client_id") != [CLIENT_ID]:
        raise G10Failure(f"the flow uses client_id {query.get('client_id')}, expected {[CLIENT_ID]}")

    page = session.get(redirect, allow_redirects=False, timeout=10)
    if page.status_code != 200:
        raise G10Failure(f"Keycloak's login page returned {page.status_code}, expected 200")
    form = _find_login_form(page.text)
    if form is None:
        raise G10Failure("no username/password form on Keycloak's login page")

    action = urllib.parse.urljoin(redirect, form["action"])
    data = dict(form["inputs"])
    data["username"] = username
    data["password"] = password
    posted = session.post(action, data=data, allow_redirects=False, timeout=10)
    if posted.status_code != 302:
        raise G10Failure(f"Keycloak's login POST returned {posted.status_code}, expected 302 (wrong password?)")
    callback = urllib.parse.urljoin(action, posted.headers.get("Location", ""))
    if not callback.startswith(API_CALLBACK):
        raise G10Failure(f"after login Keycloak redirected to {callback!r}, expected the API callback {API_CALLBACK!r}")

    finished = session.get(callback, allow_redirects=False, timeout=10)
    if finished.status_code != 302:
        raise G10Failure(f"{API_CALLBACK} returned {finished.status_code}, expected 302 (see the API log)")
    # The callback's 302 carries `Set-Cookie: lh_session=...`; `requests` folds
    # it into the session jar, so check the jar rather than parse the header.
    if not session.cookies.get("lh_session"):
        raise G10Failure("the callback did not set the lh_session cookie")


def step_wait_for_services() -> None:
    _wait_for("lakehouse-api /health", _api_health, READY_BUDGET_SECONDS)
    document = _wait_for("keycloak discovery document", _keycloak_discovery, READY_BUDGET_SECONDS)
    expected = {
        "issuer": ISSUER,
        "authorization_endpoint": AUTHORIZE_URL,
        "token_endpoint": TOKEN_URL,
        "jwks_uri": JWKS_URL,
    }
    for key, want in expected.items():
        got = document.get(key)
        if got != want:
            raise G10Failure(f"discovery {key}={got!r}, but the API is configured with {want!r}")
    print(f"[g10] discovery reachable; issuer and endpoints match the API's configuration")


def step_user_a_gets_the_mapped_role() -> None:
    session = requests.Session()
    _sign_in_through_keycloak(session, USER_A, USER_A_PASSWORD)
    me = session.get(f"{API_URL}/api/auth/me", timeout=10)
    if not me.ok:
        raise G10Failure(f"/api/auth/me returned {me.status_code} for {USER_A}")
    permissions = me.json().get("permissions", [])
    if MAPPED_PERMISSION not in permissions:
        raise G10Failure(
            f"{USER_A} signed in but /api/auth/me permissions {sorted(permissions)} lack {MAPPED_PERMISSION!r}"
        )
    print(f"[g10] {USER_A} signed in through Keycloak; permissions={sorted(permissions)}")


def step_user_b_has_no_mapped_role() -> None:
    session = requests.Session()
    _sign_in_through_keycloak(session, USER_B, USER_B_PASSWORD)
    me = session.get(f"{API_URL}/api/auth/me", timeout=10)
    if not me.ok:
        raise G10Failure(f"/api/auth/me returned {me.status_code} for {USER_B}")
    permissions = me.json().get("permissions", [])
    if MAPPED_PERMISSION in permissions:
        raise G10Failure(f"{USER_B} is in no group yet /api/auth/me grants {MAPPED_PERMISSION!r}: {sorted(permissions)}")
    print(f"[g10] {USER_B} (no group) signed in with no mapped role; permissions={sorted(permissions)}")


def _direct_grant(client_id: str) -> dict:
    """A resource-owner-password token for the negative cases. These clients are
    public and used ONLY to mint a token this service should refuse; the gate
    never signs in with them."""
    resp = requests.post(
        TOKEN_URL,
        data={
            "grant_type": "password",
            "client_id": client_id,
            "username": USER_A,
            "password": USER_A_PASSWORD,
            "scope": "openid",
        },
        timeout=10,
    )
    if not resp.ok:
        raise G10Failure(f"direct grant for {client_id} failed: {resp.status_code} {resp.text}")
    return resp.json()


def _me_with_bearer(token: str) -> requests.Response:
    return requests.get(
        f"{API_URL}/api/auth/me",
        headers={"Authorization": f"Bearer {token}"},
        timeout=10,
    )


def step_wrong_audience_token_refused() -> None:
    id_token = _direct_grant(WRONG_AUD_CLIENT_ID).get("id_token")
    if not id_token:
        raise G10Failure(f"{WRONG_AUD_CLIENT_ID} returned no id_token")
    audience = _jwt_payload(id_token).get("aud")
    response = _me_with_bearer(id_token)
    if response.status_code != 401:
        raise G10Failure(
            f"a token with aud={audience} (client {WRONG_AUD_CLIENT_ID}) was not refused: {response.status_code}"
        )
    print(f"[g10] a token minted for audience {audience} was refused with 401")


def step_expired_token_refused() -> None:
    tokens = _direct_grant(EXPIRED_CLIENT_ID)
    access_token = tokens.get("access_token")
    if not access_token:
        raise G10Failure(f"{EXPIRED_CLIENT_ID} returned no access_token")
    claims = _jwt_payload(access_token)
    lifespan = claims.get("exp", 0) - claims.get("iat", 0)
    if lifespan > NEGATIVE_EXPIRY_MARGIN_SECONDS:
        raise G10Failure(
            f"{EXPIRED_CLIENT_ID}'s access-token lifespan is {lifespan}s, over the {NEGATIVE_EXPIRY_MARGIN_SECONDS}s "
            "this step needs the realm's per-client override to be"
        )
    audience = _audience_list(claims.get("aud"))
    if CLIENT_ID not in audience:
        raise G10Failure(
            f"the expired-token access token has aud={audience}, which does not contain {CLIENT_ID!r} "
            "(its audience mapper did not apply), so a refusal could be for the wrong reason"
        )
    remaining = claims["exp"] - time.time()
    if remaining > 0:
        time.sleep(remaining + 1)
    response = _me_with_bearer(access_token)
    if response.status_code != 401:
        raise G10Failure(f"a token past its {lifespan}s expiry was not refused: {response.status_code}")
    print(f"[g10] a token past its {lifespan}s expiry was refused with 401")


def main() -> int:
    try:
        step_wait_for_services()
        step_user_a_gets_the_mapped_role()
        step_user_b_has_no_mapped_role()
        step_wrong_audience_token_refused()
        step_expired_token_refused()
    except G10Failure as exc:
        print(f"[g10] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g10] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
