"""The mock SharePoint of the SaaS gate (`SRC-3` V-SharePoint.4), registered into `saas_stub.py`. It behaves as
Microsoft documents Graph and Entra ID for what the connector uses:

- `POST /{tenant}/oauth2/v2.0/token` (Entra, a form body): the client-credentials grant (`scope` must be
  `https://graph.microsoft.com/.default`, no refresh token in the answer) and the refresh-token grant (a NEW
  refresh token on every refresh, the old one retired), only at the customer's tenant;
- `GET /v1.0/sites/<hostname>` (the root site), `GET /v1.0/sites?search=` (sites), `GET .../sites/<id>/lists`
  (the `list` facet with `template` and `hidden`), `GET .../lists/<id>/items` and `.../items/delta`
  (https://learn.microsoft.com/en-us/graph/api/listitem-delta): pages of two by `@odata.nextLink`, the last page
  with an `@odata.deltaLink`, a deleted item as `{"id", "deleted": {"state": "deleted"}}`, and `410 Gone` for a
  token the service cannot serve (admin `expire`);
- failures in Graph's and Entra's shapes (401, 403, 429 with `Retry-After`); every error body carries
  `STUB-VENDOR-WORDS` so the gate can prove the product repeats none.

Credentials are placeholders that are only strings here.
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone
from urllib.parse import parse_qs, urlparse

import saas_stub as stub

TENANT = "00000000-0000-0000-0000-0000000000a1"
CLIENT_ID = "stub-client"
CLIENT_SECRET = "stub-client-secret"
APP_ID = "sp-app"
APP_SECRET = "sp-secret"
SITE = "acme.sharepoint.com,11111111-1111-4111-8111-111111111111,22222222-2222-4222-8222-222222222222"
LIST = "55555555-5555-4555-8555-555555555555"
LIBRARY = "88888888-8888-4888-8888-888888888888"


def _iso(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        now = datetime.now(timezone.utc)
        self.page_cap = 2
        self.version = 0
        self.items: dict[str, dict] = {}
        for i in range(1, 6):
            self.put(i, now - timedelta(days=10 - i))
        self.revoked = False
        self.forbidden = False
        self.expired = False
        self.rate_limits_left = 0
        self.refresh_token = "stub-sp-refresh-0"
        self.rotations = 0
        # Entra replaces the refresh token on every refresh. The product steps keep theirs in a `file:` secret, which
        # cannot take the new one, so they turn this off (admin `rotate`); the vendor-level step proves the rotation.
        self.rotate = True
        self.extra_field: str | None = None
        self.log: list[str] = []

    def put(self, item_id: int, moment: datetime, **fields) -> None:
        self.version += 1
        self.items[str(item_id)] = {
            "id": str(item_id), "ver": self.version, "deleted": False, "createdDateTime": _iso(moment - timedelta(days=1)),
            "lastModifiedDateTime": _iso(moment), "eTag": f'"{{E{item_id}}},{self.version}"', "webUrl": f"https://acme.sharepoint.com/Lists/Tasks/{item_id}",
            "contentType": {"id": "0x01", "name": "Item"}, "fields": {"@odata.etag": "x", "Title": f"item {item_id}", "Status": "open", **fields},
        }  # fmt: skip

    def delete(self, item_id: int) -> None:
        self.version += 1
        self.items[str(item_id)].update(ver=self.version, deleted=True)


STATE = State()


def _words(message: str) -> dict:
    return {"error": {"code": "stub", "message": f"{message} STUB-VENDOR-WORDS"}}


def _valid_bearer(handler) -> bool:  # noqa: ANN001
    header = handler.headers.get("Authorization") or ""
    return header in ("Bearer stub-sp-app", *(f"Bearer stub-sp-user-{n}" for n in range(STATE.rotations + 1)))


def _gate(handler, parsed) -> bool:  # noqa: ANN001
    """Record the request; answer 401, 403 or 429 where armed. True when it may go on."""
    with stub.STATE_LOCK:
        STATE.log.append(f"{handler.command} {parsed.path}?{parsed.query}")
        if STATE.revoked or not _valid_bearer(handler):
            stub.send_json(handler, _words("Unauthorized"), 401)
            return False
        if STATE.forbidden:
            stub.send_json(handler, _words("Forbidden"), 403)
            return False
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            stub.send_json(handler, _words("Too many requests"), 429, {"Retry-After": "1"})
            return False
    return True


def _visible(entry: dict) -> dict:
    if entry["deleted"]:
        return {"id": entry["id"], "deleted": {"state": "deleted"}, "parentReference": {"siteId": "x"}}
    out = {k: v for k, v in entry.items() if k not in ("ver", "deleted")}
    if STATE.extra_field:
        out["fields"] = {**out["fields"], STATE.extra_field: "x"}
    return out


def _page(handler, rows: list[dict], start: int, next_url, last: tuple[str, str]) -> None:  # noqa: ANN001
    chunk = rows[start : start + STATE.page_cap]
    body: dict = {"value": chunk}
    origin = stub.origin_of(handler)
    if start + len(chunk) < len(rows):
        body["@odata.nextLink"] = origin + next_url(start + len(chunk))
    else:
        body[last[0]] = origin + last[1]
    stub.send_json(handler, body)


def _graph(handler, parsed) -> None:  # noqa: ANN001
    if not _gate(handler, parsed):
        return
    query = {k: v[0] for k, v in parse_qs(parsed.query).items()}
    pieces = parsed.path.split("/")  # '', v1.0, sites, ...
    with stub.STATE_LOCK:
        if parsed.path == "/v1.0/sites":
            if "search" not in query:  # the plain list is application-only; the connector must search
                return stub.send_json(handler, _words("Forbidden"), 403)
            return stub.send_json(handler, {"value": [
                {"id": SITE, "displayName": "Team", "isPersonalSite": False},
                {"id": "me.sharepoint.com,66666666-6666-4666-8666-666666666666,77777777-7777-4777-8777-777777777777", "isPersonalSite": True},
            ]})  # fmt: skip
        if parsed.path in ("/v1.0/sites/acme.sharepoint.com", f"/v1.0/sites/{SITE}"):
            return stub.send_json(handler, {"id": SITE, "displayName": "Team"})
        if len(pieces) < 5 or pieces[2] != "sites" or pieces[4] != "lists" or pieces[3] != SITE:
            return stub.send_json(handler, _words("Not Found"), 404)
        if len(pieces) == 5:
            return stub.send_json(handler, {"value": [
                {"id": LIST, "displayName": "Tasks", "name": "Tasks", "list": {"template": "genericList", "hidden": False}},
                {"id": LIBRARY, "displayName": "Documents", "list": {"template": "documentLibrary", "hidden": False}},
            ]})  # fmt: skip
        if pieces[5] != LIST:
            return stub.send_json(handler, _words("Not Found"), 404)
        base = f"/v1.0/sites/{SITE}/lists/{LIST}/items"
        if pieces[6:] == ["items"]:
            rows = [_visible(e) for e in STATE.items.values() if not e["deleted"]]
            return _page(handler, rows, int(query.get("skip", "0")), lambda n: f"{base}?skip={n}", ("@odata.deltaLink", base))
        if pieces[6:] == ["items", "delta"]:
            token = query.get("token")
            if token and STATE.expired:
                return stub.send_json(handler, {"error": {"code": "resyncRequired", "message": "STUB-VENDOR-WORDS"}}, 410, {"Location": stub.origin_of(handler) + base + "/delta"})
            since = int(token[1:]) if token else 0
            rows = [_visible(e) for e in sorted(STATE.items.values(), key=lambda e: e["ver"]) if e["ver"] > since and (token or not e["deleted"])]
            lead = f"token=v{since}&" if token else ""
            return _page(handler, rows, int(query.get("skip", "0")), lambda n: f"{base}/delta?{lead}skip={n}", ("@odata.deltaLink", f"{base}/delta?token=v{STATE.version}"))
    stub.send_json(handler, _words("Not Found"), 404)


def _token(handler, parsed) -> None:  # noqa: ANN001
    """`POST /<tenant>/oauth2/v2.0/token`, a form body."""
    form = stub.read_form(handler)
    with stub.STATE_LOCK:
        STATE.log.append(f"POST {parsed.path}")
        if parsed.path != f"/{TENANT}/oauth2/v2.0/token" or "application/x-www-form-urlencoded" not in (handler.headers.get("Content-Type") or ""):
            return stub.send_json(handler, {"error": "invalid_request", "error_description": "STUB-VENDOR-WORDS"}, 400)
        grant = form.get("grant_type")
        if grant == "client_credentials":
            ok = (not STATE.revoked and form.get("client_id") == APP_ID and form.get("client_secret") == APP_SECRET
                  and form.get("scope") == "https://graph.microsoft.com/.default")  # fmt: skip
            if not ok:
                return stub.send_json(handler, {"error": "invalid_client", "error_description": "AADSTS7000215 STUB-VENDOR-WORDS"}, 401)
            return stub.send_json(handler, {"token_type": "Bearer", "expires_in": 3599, "access_token": "stub-sp-app"})
        ok = (grant == "refresh_token" and not STATE.revoked and form.get("refresh_token") == STATE.refresh_token
              and form.get("client_id") == CLIENT_ID and form.get("client_secret") == CLIENT_SECRET)  # fmt: skip
        if not ok:
            return stub.send_json(handler, {"error": "invalid_grant", "error_description": "AADSTS70000 STUB-VENDOR-WORDS"}, 400)
        if STATE.rotate:
            STATE.rotations += 1
            STATE.refresh_token = f"stub-sp-refresh-{STATE.rotations}"
        # Without rotation the answer repeats the refresh token (RFC 6749 6 allows either).
        out = {"token_type": "Bearer", "expires_in": 3599, "access_token": f"stub-sp-user-{STATE.rotations}", "scope": "Sites.Read.All", "refresh_token": STATE.refresh_token}
    stub.send_json(handler, out)


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not a Microsoft route."""
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
        elif action == "put":  # add or change an item; Graph stamps the change itself
            STATE.put(int(body["id"]), datetime.now(timezone.utc), **body.get("fields", {}))
        elif action == "delete":
            STATE.delete(int(body["id"]))
        elif action == "add_field":  # SharePoint adds a column
            STATE.extra_field = body["name"]
        elif action == "expire":  # every saved delta token is now refused with 410
            STATE.expired = bool(body.get("on", True))
        elif action == "refresh_token":
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


stub.ROUTES[("GET", "/sharepoint/v1.0")] = _graph
stub.ROUTES[("POST", "/sharepoint/" + TENANT)] = _token
stub.ROUTES[("GET", "/__admin/sharepoint")] = _admin
stub.ROUTES[("POST", "/__admin/sharepoint")] = _admin
