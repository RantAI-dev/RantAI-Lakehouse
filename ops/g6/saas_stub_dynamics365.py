"""The mock Dynamics 365 of the SaaS gate (`SRC-3` V-Dynamics.4), registered into `saas_stub.py`. It behaves as
Microsoft documents the Dataverse Web API and Entra ID for what the connector uses:

- `POST /{tenant}/oauth2/v2.0/token` (Entra, a form body): the client-credentials grant (`scope` must be
  `https://<org>/.default`, no refresh token in the answer) and the refresh-token grant (a NEW refresh token on every
  refresh, the old one retired), only at the customer's tenant;
- `GET /api/data/v9.2/WhoAmI`; `EntityDefinitions` (not paged; `$filter`/`$expand=Attributes(...)` as the connector
  asks) and `EntityDefinitions(LogicalName='x')`; the entity sets `accounts` (no change tracking, `modifiedon`
  filter and `$orderby`) and `contacts` (change tracking: `Prefer: odata.track-changes` refuses `$filter`/`$orderby`,
  `@odata.deltaLink` with a `$deltatoken`, a deleted row as `{"@odata.context": ".../$deletedEntity", "id", "reason"}`,
  a token it no longer serves answered 400 as admin `expire` asks), pages of two by `@odata.nextLink` with the
  `Prefer: odata.maxpagesize` of the request;
- failures in Dataverse's and Entra's shapes (401, 403, 429 with `Retry-After`); every error body carries
  `STUB-VENDOR-WORDS` so the gate can prove the product repeats none.

Credentials are placeholders that are only strings here.
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone
from urllib.parse import parse_qs, quote, urlparse

import saas_stub as stub

TENANT = "00000000-0000-0000-0000-0000000000b2"
HOST = "acme.crm4.dynamics.com"
CLIENT_ID = "stub-client"
CLIENT_SECRET = "stub-client-secret"
APP_ID = "dv-app"
APP_SECRET = "dv-secret"


def _iso(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _encode(query: dict) -> str:
    return "&".join(f"{quote(str(k), safe='$,')}={quote(str(v), safe='$,')}" for k, v in query.items())


class Table:
    def __init__(self, logical: str, entity_set: str, key: str, tracked: bool) -> None:
        self.logical, self.entity_set, self.key, self.tracked = logical, entity_set, key, tracked
        self.rows: dict[str, dict] = {}
        self.vers: dict[str, int] = {}
        self.deleted: set[str] = set()
        self.version = 0
        self.extra: str | None = None

    def put(self, row_id: int, moment: datetime, **columns) -> None:
        self.version += 1
        rid = f"00000000-0000-0000-0000-{row_id:012d}"
        self.deleted.discard(rid)
        self.vers[rid] = self.version
        self.rows[rid] = {self.key: rid, "@odata.etag": f'W/"{self.version}"', "modifiedon": _iso(moment), "name": f"row {row_id}", "revenue": 1000.0 * row_id, **columns}

    def delete(self, row_id: int) -> None:
        self.version += 1
        rid = f"00000000-0000-0000-0000-{row_id:012d}"
        self.vers[rid] = self.version
        self.deleted.add(rid)

    def attributes(self) -> list[dict]:
        def a(name: str, kind: str, of: str | None = None) -> dict:
            return {"LogicalName": name, "AttributeType": kind, "AttributeOf": of, "IsValidForRead": True}

        out = [a(self.key, "Uniqueidentifier"), a("name", "String"), a("modifiedon", "DateTime"), a("revenue", "Money"), a("ownerid", "Owner"), a("owneridname", "String", "ownerid")]
        return out + ([a(self.extra, "String")] if self.extra else [])

    def definition(self) -> dict:
        return {"LogicalName": self.logical, "EntitySetName": self.entity_set, "PrimaryIdAttribute": self.key, "ChangeTrackingEnabled": self.tracked}


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        now = datetime.now(timezone.utc)
        self.page_cap = 2
        self.tables = {"account": Table("account", "accounts", "accountid", False), "contact": Table("contact", "contacts", "contactid", True)}
        for table in self.tables.values():
            for i in range(1, 6):
                table.put(i, now - timedelta(days=10 - i), _ownerid_value="11111111-1111-4111-8111-111111111111")
        self.revoked = False
        self.forbidden = False
        self.expired = False
        self.rate_limits_left = 0
        self.refresh_token = "stub-dv-refresh-0"
        self.rotations = 0
        # Entra replaces the refresh token on every refresh. The product steps keep theirs in a `file:` secret, which
        # cannot take the new one, so they turn this off (admin `rotate`); the vendor-level step proves the rotation.
        self.rotate = True
        self.log: list[str] = []


STATE = State()


def _words(message: str) -> dict:
    return {"error": {"code": "0x80040217", "message": f"{message} STUB-VENDOR-WORDS"}}


def _gate(handler, parsed) -> bool:  # noqa: ANN001
    """Record the request; answer 401, 403 or 429 where armed. True when it may go on."""
    with stub.STATE_LOCK:
        STATE.log.append(f"{handler.command} {parsed.path}?{parsed.query}")
        header = handler.headers.get("Authorization") or ""
        valid = header == "Bearer stub-dv-app" or header in (f"Bearer stub-dv-user-{n}" for n in range(STATE.rotations + 1))
        if STATE.revoked or not valid:
            stub.send_json(handler, _words("Unauthorized"), 401)
            return False
        if STATE.forbidden:
            stub.send_json(handler, _words("Forbidden"), 403)
            return False
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            stub.send_json(handler, _words("Number of requests exceeded the limit"), 429, {"Retry-After": "1"})
            return False
    return True


def _rows(handler, table: Table, path: str, query: dict) -> None:  # noqa: ANN001
    prefer = handler.headers.get("Prefer") or ""
    tracking = "odata.track-changes" in prefer
    token = query.get("$deltatoken")
    if tracking and any(k in query for k in ("$filter", "$orderby", "$expand", "$top")):
        return stub.send_json(handler, _words("The query parameter isn't supported when Change Tracking is enabled."), 400)
    if tracking or token:
        if not table.tracked:
            return stub.send_json(handler, _words("Change tracking is not enabled"), 400)
        if token and STATE.expired:
            return stub.send_json(handler, _words("The change token is too old."), 400)
    size = int(prefer.split("odata.maxpagesize=")[1].split(",")[0]) if "odata.maxpagesize=" in prefer else 5000
    size = min(size, STATE.page_cap)
    origin = stub.origin_of(handler)
    if token:
        since = int(token)
        rows = []
        for rid, ver in sorted(table.vers.items(), key=lambda kv: kv[1]):
            if ver > since:
                rows.append({"@odata.context": f"{origin}/api/data/v9.2/$metadata#{table.entity_set}/$deletedEntity", "id": rid, "reason": "deleted"} if rid in table.deleted else table.rows[rid])
    else:
        rows = [r for rid, r in table.rows.items() if rid not in table.deleted]
        if "$filter" in query:
            _field, op, value = query["$filter"].split(" ")
            if op != "ge":
                return stub.send_json(handler, _words("Bad filter"), 400)
            rows = [r for r in rows if r["modifiedon"] >= value]
        order = [o.split(" ")[0] for o in query.get("$orderby", f"{table.key} asc").split(",")]
        rows.sort(key=lambda r: tuple(r[o] for o in order))
    if table.extra:
        rows = [r if "@odata.context" in r else {**r, table.extra: "x"} for r in rows]
    select = query["$select"].split(",") if query.get("$select") else None
    if select:
        rows = [r if "@odata.context" in r else {k: v for k, v in r.items() if k in select or k.startswith("@")} for r in rows]
    start = int(query.get("$skiptoken", "0"))
    chunk = rows[start : start + size]
    body: dict = {"@odata.context": f"{origin}/api/data/v9.2/$metadata#{table.entity_set}", "value": chunk}
    base_query = {k: v for k, v in query.items() if k != "$skiptoken"}
    if start + len(chunk) < len(rows):
        body["@odata.nextLink"] = f"{origin}{path}?{_encode({**base_query, '$skiptoken': start + len(chunk)})}"
    elif tracking or token:
        body["@odata.deltaLink"] = f"{origin}{path}?{_encode({**base_query, '$deltatoken': table.version})}"
    stub.send_json(handler, body)


def _api(handler, parsed) -> None:  # noqa: ANN001
    if not _gate(handler, parsed):
        return
    query = {k: v[0] for k, v in parse_qs(parsed.query, keep_blank_values=True).items()}
    resource = parsed.path[len("/api/data/v9.2/") :]
    with stub.STATE_LOCK:
        if resource == "WhoAmI":
            return stub.send_json(handler, {"UserId": "u", "OrganizationId": "o", "BusinessUnitId": "b"})
        if resource == "EntityDefinitions":
            value = []
            for table in STATE.tables.values():
                entry = table.definition()
                if "$expand" in query:
                    entry["Attributes"] = [a for a in table.attributes() if a["LogicalName"] == "modifiedon"]
                value.append(entry)
            return stub.send_json(handler, {"value": value})
        if resource.startswith("EntityDefinitions(LogicalName='"):
            table = STATE.tables.get(resource.split("'")[1])
            if table is None:
                return stub.send_json(handler, _words("Not Found"), 404)
            return stub.send_json(handler, {**table.definition(), "Attributes": table.attributes()})
        table = next((t for t in STATE.tables.values() if t.entity_set == resource), None)
        if table is None:
            return stub.send_json(handler, _words("Not Found"), 404)
        _rows(handler, table, parsed.path, query)


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
                  and form.get("scope") == f"https://{HOST}/.default")  # fmt: skip
            if not ok:
                return stub.send_json(handler, {"error": "invalid_client", "error_description": "AADSTS7000215 STUB-VENDOR-WORDS"}, 401)
            return stub.send_json(handler, {"token_type": "Bearer", "expires_in": 3599, "access_token": "stub-dv-app"})
        ok = (grant == "refresh_token" and not STATE.revoked and form.get("refresh_token") == STATE.refresh_token
              and form.get("client_id") == CLIENT_ID and form.get("client_secret") == CLIENT_SECRET)  # fmt: skip
        if not ok:
            return stub.send_json(handler, {"error": "invalid_grant", "error_description": "AADSTS70000 STUB-VENDOR-WORDS"}, 400)
        if STATE.rotate:
            STATE.rotations += 1
            STATE.refresh_token = f"stub-dv-refresh-{STATE.rotations}"
        # Without rotation the answer repeats the refresh token (RFC 6749 6 allows either).
        out = {"token_type": "Bearer", "expires_in": 3599, "access_token": f"stub-dv-user-{STATE.rotations}", "refresh_token": STATE.refresh_token}
    stub.send_json(handler, out)


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not a Microsoft route."""
    if handler.command == "GET":
        with stub.STATE_LOCK:
            return stub.send_json(handler, {"log": list(STATE.log)})
    body = stub.read_json(handler)
    action = body.get("action")
    with stub.STATE_LOCK:
        table = STATE.tables.get(body.get("table", "account"))
        if action == "reset":
            STATE.reset()
        elif action == "clear_log":
            STATE.log.clear()
        elif action == "put":  # add or change a row; Dataverse stamps `modifiedon` itself
            table.put(int(body["id"]), datetime.now(timezone.utc), **body.get("fields", {}))
        elif action == "delete":
            table.delete(int(body["id"]))
        elif action == "add_field":  # an admin adds a column to the table
            table.extra = body["name"]
        elif action == "expire":  # every saved delta token is now refused
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


stub.ROUTES[("GET", "/dynamics365/api/data/v9.2")] = _api
stub.ROUTES[("POST", "/dynamics365/" + TENANT)] = _token
stub.ROUTES[("GET", "/__admin/dynamics365")] = _admin
stub.ROUTES[("POST", "/__admin/dynamics365")] = _admin
