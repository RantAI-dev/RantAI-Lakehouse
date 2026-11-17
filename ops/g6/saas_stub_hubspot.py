"""The mock HubSpot of the SaaS gate (`SRC-3` V-HubSpot.4), registered into
`saas_stub.py`. It behaves as HubSpot documents its API
(https://developers.hubspot.com/docs/api/overview): every property value is
text, an empty text for no value; list pages carry `paging.next.link`; the CRM
search takes a `GTE` filter, one sort and `after`, returns at most 10,000
results and never returns an archived record; the properties API lists
`{name, type}`; a refused sign-in is a 401 with HubSpot's error shape, a
revoked refresh token a 400 with `BAD_REFRESH_TOKEN`; a rate limit is a 429
with `Retry-After`. Every error body carries the words `STUB-VENDOR-WORDS` so
the gate can prove the product repeats none of them.

Objects: deals and contacts (contacts use `lastmodifieddate`, the other objects
`hs_lastmodifieddate`, as HubSpot does), the owners, and the deal pipelines.
The credentials are placeholders: a private-app token and a refresh token that
are only strings here, never anything real.
"""
from __future__ import annotations

import json
import time
from datetime import datetime, timezone
from urllib.parse import parse_qs, urlparse

import saas_stub as stub

PRIVATE_APP_TOKEN = "stub-private-app-token"
REFRESH_TOKEN = "stub-refresh-token"
ACCESS_TOKEN = "stub-access-token"
PORTAL_ID = 123456
SEARCH_CAP = 10_000

_DEAL_PROPS = {
    "amount": "number",
    "closedate": "datetime",
    "createdate": "datetime",
    "dealname": "string",
    "dealstage": "enumeration",
    "hs_lastmodifieddate": "datetime",
    "hs_object_id": "string",
    "pipeline": "string",
    "is_renewal": "bool",
}
_CONTACT_PROPS = {
    "createdate": "datetime",
    "email": "string",
    "firstname": "string",
    "hs_object_id": "string",
    "lastmodifieddate": "datetime",
    "lastname": "string",
}
_CURSOR = {"deals": "hs_lastmodifieddate", "contacts": "lastmodifieddate"}


def _iso(ms: int) -> str:
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms % 1000:03d}Z"


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        base = int(time.time() * 1000) - 86_400_000  # a day ago, ten minutes apart
        self.props = {"deals": dict(_DEAL_PROPS), "contacts": dict(_CONTACT_PROPS)}
        self.records: dict[str, dict[str, dict]] = {
            "deals": {str(i): {"hs_object_id": str(i), "dealname": f"deal {i}", "amount": str(i * 100), "is_renewal": "false", "_ms": base + i * 600_000} for i in (1, 2, 3)},
            "contacts": {str(i): {"hs_object_id": str(i), "email": f"person{i}@example.invalid", "firstname": f"P{i}", "_ms": base + i * 600_000} for i in (1, 2)},
        }
        self.archived: set[str] = set()
        self.revoked = False
        self.rate_limits_left = 0
        self.log: list[str] = []


STATE = State()


def _words(message: str) -> dict:
    return {"status": "error", "message": f"{message} STUB-VENDOR-WORDS", "correlationId": "stub-correlation-id"}


def _authorised(handler) -> bool:  # noqa: ANN001
    """401 in HubSpot's shape for a missing, wrong or revoked credential; the
    429 of an armed rate limit. True when the request may go on."""
    token = (handler.headers.get("Authorization") or "").removeprefix("Bearer ")
    with stub.STATE_LOCK:
        if STATE.revoked or token not in (PRIVATE_APP_TOKEN, ACCESS_TOKEN):
            stub.send_json(handler, {**_words("Authentication credentials not found."), "category": "INVALID_AUTHENTICATION"}, 401)
            return False
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            body = {**_words("You have reached your secondly limit."), "errorType": "RATE_LIMIT", "policyName": "SECONDLY"}
            stub.send_json(handler, body, 429, {"Retry-After": "1"})
            return False
    return True


def _note(handler, parsed) -> None:  # noqa: ANN001
    with stub.STATE_LOCK:
        STATE.log.append(f"{handler.command} {parsed.path}")


def _result(plural: str, rec_id: str, wanted: list[str]) -> dict:
    row = STATE.records[plural][rec_id]
    props = {name: row.get(name, "") for name in wanted if name in STATE.props[plural]}
    cursor = _CURSOR[plural]
    if cursor in wanted:
        props[cursor] = _iso(row["_ms"])
    return {"id": rec_id, "properties": props, "createdAt": _iso(row["_ms"]), "updatedAt": _iso(row["_ms"]), "archived": False}


def _details(handler, parsed) -> None:  # noqa: ANN001
    _note(handler, parsed)
    if _authorised(handler):
        stub.send_json(handler, {"portalId": PORTAL_ID, "accountType": "STANDARD", "timeZone": "us/eastern", "uiDomain": "app.hubspot.com", "dataHostingLocation": "na1"})


def _properties(handler, parsed) -> None:  # noqa: ANN001
    _note(handler, parsed)
    if not _authorised(handler):
        return
    plural = parsed.path.rsplit("/", 1)[1]
    with stub.STATE_LOCK:
        table = STATE.props.get(plural)
        if table is None:
            return stub.send_json(handler, _words("Object type not found."), 404)
        stub.send_json(handler, {"results": [{"name": n, "type": t, "label": n} for n, t in table.items()]})


def _list(handler, parsed) -> None:  # noqa: ANN001
    _note(handler, parsed)
    if not _authorised(handler):
        return
    plural = parsed.path.rsplit("/", 1)[1]
    query = {k: v[0] for k, v in parse_qs(parsed.query).items()}
    with stub.STATE_LOCK:
        if plural not in STATE.records:
            return stub.send_json(handler, _words("Object type not found."), 404)
        wanted = query.get("properties", "").split(",")
        ids = sorted(STATE.records[plural], key=int)
        start, limit = int(query.get("after", 0)), int(query.get("limit", 10))
        body: dict = {"results": [_result(plural, i, wanted) for i in ids[start : start + limit]]}
        if start + limit < len(ids):
            nxt = start + limit
            body["paging"] = {"next": {"after": str(nxt), "link": f"{stub.origin_of(handler)}{parsed.path}?after={nxt}&limit={limit}&properties={query.get('properties', '')}"}}
    stub.send_json(handler, body)


def _search(handler, parsed) -> None:  # noqa: ANN001
    _note(handler, parsed)
    raw = stub.read_body(handler)
    if not _authorised(handler):
        return
    plural = parsed.path.split("/")[-2]
    if len(raw) > 3000:
        return stub.send_json(handler, _words("Request body too long."), 400)
    body = json.loads(raw)
    flt = body["filterGroups"][0]["filters"][0]
    with stub.STATE_LOCK:
        if plural not in STATE.records:
            return stub.send_json(handler, _words("Object type not found."), 404)
        assert flt["operator"] == "GTE" and flt["propertyName"] == _CURSOR[plural], "the stub models the GTE filter on the cursor property"
        floor = int(flt["value"])
        # Archived records are not in `records` (they moved to `archived`): HubSpot's search never returns them.
        hits = sorted((i for i, r in STATE.records[plural].items() if r["_ms"] >= floor), key=lambda i: (STATE.records[plural][i]["_ms"], int(i)))
        start = int(body.get("after", 0))
        if start >= SEARCH_CAP:
            return stub.send_json(handler, _words("Cannot page beyond 10,000 results."), 400)
        limit = min(int(body.get("limit", 10)), 200)
        out: dict = {"total": len(hits), "results": [_result(plural, i, body["properties"]) for i in hits[start : start + limit]]}
        if start + limit < len(hits):
            out["paging"] = {"next": {"after": str(start + limit)}}
    stub.send_json(handler, out)


def _batch_read(handler, parsed) -> None:  # noqa: ANN001
    _note(handler, parsed)
    body = stub.read_json(handler)
    if not _authorised(handler):
        return
    plural = parsed.path.split("/")[-3]
    with stub.STATE_LOCK:
        stub.send_json(handler, {"status": "COMPLETE", "results": [_result(plural, i["id"], body["properties"]) for i in body["inputs"] if i["id"] in STATE.records[plural]]})


def _owners(handler, parsed) -> None:  # noqa: ANN001
    _note(handler, parsed)
    if _authorised(handler):
        stub.send_json(handler, {"results": [{"id": "1", "email": "owner@example.invalid", "firstName": "Pat", "lastName": "Owner", "userId": 11, "archived": False, "createdAt": _iso(0), "updatedAt": _iso(1000)}]})


def _pipelines(handler, parsed) -> None:  # noqa: ANN001
    _note(handler, parsed)
    if _authorised(handler):
        stub.send_json(handler, {"results": [{"id": "default", "label": "Sales pipeline", "displayOrder": 0, "stages": [{"id": "s1", "label": "New", "displayOrder": 0}]}]})


def _token(handler, parsed) -> None:  # noqa: ANN001
    """`POST /oauth/v1/token`: the refresh grant, with the client in the form
    body as HubSpot documents it."""
    _note(handler, parsed)
    form = stub.read_form(handler)
    with stub.STATE_LOCK:
        ok = (
            not STATE.revoked
            and form.get("grant_type") == "refresh_token"
            and form.get("refresh_token") == REFRESH_TOKEN
            and bool(form.get("client_id"))
            and bool(form.get("client_secret"))
        )
    if not ok:
        return stub.send_json(handler, {"status": "BAD_REFRESH_TOKEN", "message": "refresh token is invalid, expired or revoked STUB-VENDOR-WORDS"}, 400)
    stub.send_json(handler, {"access_token": ACCESS_TOKEN, "refresh_token": REFRESH_TOKEN, "expires_in": 1800, "token_type": "bearer"})


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not a HubSpot route."""
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
        elif action == "put":  # add or change a record, modified now
            store = STATE.records[body["object"]]
            row = store.setdefault(str(body["id"]), {"hs_object_id": str(body["id"])})
            row.update({k: str(v) for k, v in body.get("properties", {}).items()})
            row["_ms"] = int(time.time() * 1000)
        elif action == "archive":  # a delete at HubSpot: gone from list and search
            STATE.archived.add(str(body["id"]))
            STATE.records[body["object"]].pop(str(body["id"]), None)
        elif action == "add_property":
            STATE.props[body["object"]][body["name"]] = body.get("type", "string")
        elif action == "revoke":
            STATE.revoked = bool(body.get("on", True))
        elif action == "rate_limit":
            STATE.rate_limits_left = int(body.get("count", 1))
        else:
            return stub.send_json(handler, {"error": "unknown action"}, 400)
    stub.send_json(handler, {"ok": True})


stub.ROUTES[("GET", "/hubspot/account-info/v3/details")] = _details
stub.ROUTES[("GET", "/hubspot/crm/v3/properties")] = _properties
stub.ROUTES[("GET", "/hubspot/crm/v3/objects")] = _list
stub.ROUTES[("POST", "/hubspot/crm/v3/objects")] = lambda h, p: _search(h, p) if p.path.endswith("/search") else _batch_read(h, p)
stub.ROUTES[("GET", "/hubspot/crm/v3/owners")] = _owners
stub.ROUTES[("GET", "/hubspot/crm/v3/pipelines")] = _pipelines
stub.ROUTES[("POST", "/hubspot/oauth/v1/token")] = _token
stub.ROUTES[("GET", "/__admin/hubspot")] = _admin
stub.ROUTES[("POST", "/__admin/hubspot")] = _admin
