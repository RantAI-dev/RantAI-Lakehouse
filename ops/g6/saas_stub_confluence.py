"""The mock Confluence of the SaaS gate (`SRC-3` V-Confluence.4), registered into
`saas_stub.py`. It behaves as Atlassian documents Confluence Cloud's REST API v2
(https://developer.atlassian.com/cloud/confluence/rest/v2/intro/):

- `GET /wiki/api/v2/{spaces,pages,blogposts,footer-comments,inline-comments}` at the site, with HTTP
  Basic (email + API token), `limit`, `cursor`, `sort=-modified-date`, and `results` with a relative
  `_links.next` (pages are capped at `STATE.page_cap` however large `limit` is);
- the same paths behind Atlassian's OAuth gateway `/ex/confluence/<cloud id>/wiki/api/v2/...` with a
  bearer token, where the relative `_links.next` does NOT carry the gateway prefix (the connector must
  add it; the real form was not observed, so the mock models the harder one);
- `GET /oauth/token/accessible-resources` (the sites a token reaches, with ids and scopes);
- `POST /oauth/token`, a JSON body: the `refresh_token` grant, which rotates the refresh token (the old
  one is disabled) and answers 403 `invalid_grant` for a retired one, as Atlassian documents;
- failures in Atlassian's shapes (401, 403, 429 with `Retry-After`); every error body carries
  `STUB-VENDOR-WORDS` so the gate can prove the product repeats none.

Credentials are placeholders that are only strings here.
"""
from __future__ import annotations

import base64
from datetime import datetime, timedelta, timezone
from urllib.parse import parse_qs

import saas_stub as stub

EMAIL = "agent@example.invalid"
API_TOKEN = "stub-api-token"
CLIENT_ID = "stub-client"
CLIENT_SECRET = "stub-client-secret"
CLOUD_ID = "0f1e2d3c-4b5a-4968-8776-655443322110"
RESOURCES = ("spaces", "pages", "blogposts", "footer-comments", "inline-comments")


def _iso(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000Z")


def _page(i: int, moment: datetime, **extra) -> dict:
    return {
        "id": str(1000 + i), "status": "current", "title": f"page {i}", "spaceId": "9", "authorId": "u1", "createdAt": _iso(moment),
        "version": {"number": 1, "authorId": "u2", "createdAt": _iso(moment)},
        "body": {"storage": {"representation": "storage", "value": f"<p>page {i}</p>"}},
        "_links": {"webui": f"/spaces/X/pages/{1000 + i}"}, **extra,
    }  # fmt: skip


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        now = datetime.now(timezone.utc)
        self.page_cap = 2
        self.store = {r: [] for r in RESOURCES}
        self.store["pages"] = [_page(i, now - timedelta(days=14 - i)) for i in range(1, 6)]
        self.store["spaces"] = [{"id": "9", "key": "X", "name": "Space X", "status": "current", "icon": {"path": "/x"}}]
        self.store["footer-comments"] = [
            {"id": "5001", "status": "current", "pageId": "1001", "version": {"number": 1, "createdAt": _iso(now - timedelta(days=9))}, "body": {"storage": {"value": "a comment"}}}
        ]
        self.revoked = False
        self.forbidden = False
        self.rate_limits_left = 0
        self.refresh_token = "stub-cf-refresh-0"
        self.rotations = 0
        # Atlassian rotates the refresh token on every refresh. The product steps keep their refresh
        # token in a `file:` secret, which cannot take the new one, so they turn this off (admin `rotate`).
        self.rotate = True
        self.log: list[str] = []


STATE = State()


def _words(message: str) -> dict:
    return {"code": 0, "message": f"{message} STUB-VENDOR-WORDS"}


def _authorised(handler, *, gateway: bool) -> bool:  # noqa: ANN001
    header = handler.headers.get("Authorization")
    if gateway:
        return header == f"Bearer stub-cf-access-{STATE.rotations}"
    return header == "Basic " + base64.b64encode(f"{EMAIL}:{API_TOKEN}".encode()).decode()


def _gate(handler, parsed, *, gateway: bool) -> bool:  # noqa: ANN001
    """Record the request; answer 401, 403 or 429 where armed. True when it may go on."""
    with stub.STATE_LOCK:
        STATE.log.append(f"{handler.command} {parsed.path}?{parsed.query}")
        if STATE.revoked or not _authorised(handler, gateway=gateway):
            stub.send_json(handler, _words("Unauthorized"), 401)
            return False
        if STATE.forbidden:
            stub.send_json(handler, _words("Forbidden"), 403)
            return False
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            stub.send_json(handler, _words("Rate limit exceeded"), 429, {"Retry-After": "1"})
            return False
    return True


def _list(handler, parsed, resource: str) -> None:  # noqa: ANN001
    query = {k: v[0] for k, v in parse_qs(parsed.query).items()}
    with stub.STATE_LOCK:
        rows = list(STATE.store[resource])
        if query.get("sort") == "-modified-date":
            rows.sort(key=lambda r: r["version"]["createdAt"], reverse=True)
        start = int(query.get("cursor", "0"))
        page = rows[start : start + min(int(query.get("limit", STATE.page_cap)), STATE.page_cap)]
        body = {"results": page, "_links": {"base": "https://acme.atlassian.net/wiki"}}
        if start + len(page) < len(rows):
            sort = f"&sort={query['sort']}" if "sort" in query else ""
            body["_links"]["next"] = f"/wiki/api/v2/{resource}?limit={query.get('limit')}{sort}&cursor={start + len(page)}"
    stub.send_json(handler, body)


def _site(handler, parsed) -> None:  # noqa: ANN001
    resource = parsed.path.rsplit("/", 1)[-1]
    if _gate(handler, parsed, gateway=False) and resource in STATE.store:
        _list(handler, parsed, resource)


def _gateway(handler, parsed) -> None:  # noqa: ANN001
    # /ex/confluence/<cloud id>/wiki/api/v2/<resource> (the route sees the path the vendor received)
    parts = parsed.path.split("/")
    if not _gate(handler, parsed, gateway=True):
        return
    if len(parts) < 4 or parts[3] != CLOUD_ID or parts[-1] not in STATE.store:
        return stub.send_json(handler, _words("Not Found"), 404)
    _list(handler, parsed, parts[-1])


def _resources(handler, parsed) -> None:  # noqa: ANN001
    with stub.STATE_LOCK:
        STATE.log.append(f"GET {parsed.path}")
        ok = not STATE.revoked and handler.headers.get("Authorization") == f"Bearer stub-cf-access-{STATE.rotations}"
    if not ok:
        return stub.send_json(handler, _words("Unauthorized"), 401)
    stub.send_json(handler, [
        {"id": "ffffffff-0000-4000-8000-000000000000", "url": "https://other.atlassian.net", "name": "other", "scopes": ["read:page:confluence"]},
        {"id": CLOUD_ID, "url": "https://acme.atlassian.net", "name": "acme", "scopes": ["read:page:confluence", "read:space:confluence", "read:comment:confluence"]},
    ])  # fmt: skip


def _token(handler, parsed) -> None:  # noqa: ANN001
    """`POST /oauth/token`, the refresh grant. JSON in, as Atlassian requires; each refresh issues a
    new access and refresh token and disables the old refresh token."""
    with stub.STATE_LOCK:
        STATE.log.append(f"POST {parsed.path}")
    body = None
    if "application/json" in (handler.headers.get("Content-Type") or ""):
        try:
            body = stub.read_json(handler)
        except ValueError:
            body = None
    else:
        stub.read_body(handler)
    with stub.STATE_LOCK:
        ok = (
            isinstance(body, dict) and not STATE.revoked and body.get("grant_type") == "refresh_token"
            and body.get("refresh_token") == STATE.refresh_token
            and body.get("client_id") == CLIENT_ID and body.get("client_secret") == CLIENT_SECRET
        )  # fmt: skip
        if not ok:
            return stub.send_json(handler, {"error": "invalid_grant", "error_description": "STUB-VENDOR-WORDS"}, 403)
        if STATE.rotate:
            STATE.rotations += 1
            STATE.refresh_token = f"stub-cf-refresh-{STATE.rotations}"
        out = {"access_token": f"stub-cf-access-{STATE.rotations}", "refresh_token": STATE.refresh_token, "expires_in": 3600, "scope": "read:page:confluence offline_access"}
    stub.send_json(handler, out)


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not an Atlassian route."""
    if handler.command == "GET":
        with stub.STATE_LOCK:
            return stub.send_json(handler, {"log": list(STATE.log)})
    body = stub.read_json(handler)
    action = body.get("action")
    with stub.STATE_LOCK:
        if action == "reset":
            STATE.reset()
        elif action == "clear_log":
            STATE.log.clear()
        elif action == "put":  # add or change a page; Confluence stamps the new version itself
            fresh = _page(int(body["id"]), datetime.now(timezone.utc), **body.get("fields", {}))
            STATE.store["pages"] = [p for p in STATE.store["pages"] if p["id"] != fresh["id"]] + [fresh]
        elif action == "add_field":  # Confluence adds a property to its objects
            STATE.store["pages"] = [{**p, body["name"]: "x"} for p in STATE.store["pages"]]
        elif action == "refresh_token":  # tell the gate which refresh token is current
            return stub.send_json(handler, {"refresh_token": STATE.refresh_token})
        elif action == "rotate":
            STATE.rotate = bool(body.get("on", True))
        elif action == "revoke":
            STATE.revoked = bool(body.get("on", True))
        elif action == "forbid":
            STATE.forbidden = bool(body.get("on", True))
        elif action == "rate_limit":
            STATE.rate_limits_left = int(body.get("count", 1))
        else:
            return stub.send_json(handler, {"error": "unknown action"}, 400)
    stub.send_json(handler, {"ok": True})


stub.ROUTES[("GET", "/confluence/wiki/api/v2")] = _site
stub.ROUTES[("GET", "/confluence/ex/confluence")] = _gateway
stub.ROUTES[("GET", "/confluence/oauth/token/accessible-resources")] = _resources
stub.ROUTES[("POST", "/confluence/oauth/token")] = _token
stub.ROUTES[("GET", "/__admin/confluence")] = _admin
stub.ROUTES[("POST", "/__admin/confluence")] = _admin
