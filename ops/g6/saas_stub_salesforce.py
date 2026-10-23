"""The mock Salesforce of the SaaS gate (`SRC-3` V-Salesforce.4), registered into
`saas_stub.py`. It serves the REST calls the connector makes, at Salesforce's real
paths (`/services/data/v62.0/...`, `/services/oauth2/token`):

- `GET  /services/data/v62.0/limits`: the connection test ("who am I");
- `GET  /services/data/v62.0/sobjects`: describe global (`name`, `queryable`,
  `replicateable`, `custom`, `deprecatedAndHidden`);
- `GET  /services/data/v62.0/sobjects/<name>/describe`: the object's fields;
- `GET  /services/data/v62.0/query?q=<SOQL>` and `.../queryAll?q=<SOQL>`: the SOQL
  subset the connector writes (`SELECT <fields> FROM <object> [WHERE SystemModstamp >= <literal>]
  ORDER BY ...`), at most `page_cap` records a batch, `nextRecordsUrl` a PATH on this
  host (`/services/data/v62.0/query/01gSTUB...-<offset>`), `done` false until the last
  batch. `query` hides the records in the recycle bin; `queryAll` returns them with
  `IsDeleted = true`;
- `POST /services/oauth2/token`: the refresh-token grant, the client in the form body
  (`client_secret_post`). The answer carries `instance_url`, which is Salesforce's
  real-looking My Domain address (`https://acme.my.salesforce.com`), NOT the mock's:
  the gate proves the product adopts it through the `.salesforce.com` rule and dials
  the mock only because of the gate-only `SAAS_GATE_ORIGINS`. `evil_instance` makes it
  answer another host, to prove the run stops. Salesforce does not rotate the refresh
  token by default and neither does this mock.

Failures in Salesforce's shapes: an array of `{message, errorCode}` with 401
(`INVALID_SESSION_ID`) and 403 (`INSUFFICIENT_ACCESS`), a revoked refresh token is 400
`{"error": "invalid_grant", ...}`. Real Salesforce answers an exhausted API quota with
403 `REQUEST_LIMIT_EXCEEDED` (`quota` here; the product reports it as `rate_limited`, not
retried: slice 3 review BLOCKER 8); `rate_limit` here answers the generic 429 + `Retry-After` that
`SaasSession` waits out (decision 13, AD9). Every error body carries `STUB-VENDOR-WORDS`.

Objects: `Account` (incremental on `SystemModstamp`, has `IsDeleted`) and `Region__c` (a
custom object with no `SystemModstamp`, so it is re-read in full). Credentials are
placeholders that are only strings here.
"""
from __future__ import annotations

import re
import time
from datetime import datetime, timezone
from urllib.parse import parse_qs, urlparse

import saas_stub as stub

API = "/services/data/v62.0"
CLIENT_ID = "stub-client"
CLIENT_SECRET = "stub-client-secret"
REFRESH_TOKEN = "stub-refresh-token"
ACCESS_TOKEN = "stub-sf-access-token"
INSTANCE_URL = "https://acme.my.salesforce.com"

_ACCOUNT_FIELDS = {
    "Id": "id", "Name": "string", "IsDeleted": "boolean", "SystemModstamp": "datetime",
    "Industry__c": "string", "AnnualRevenue": "currency",
}  # fmt: skip
_REGION_FIELDS = {"Id": "id", "Name": "string"}


def _stamp(ms: int) -> str:
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms % 1000:03d}+0000"


def _ms_of(literal: str) -> int:
    return int(datetime.strptime(literal, "%Y-%m-%dT%H:%M:%S.%fZ").replace(tzinfo=timezone.utc).timestamp() * 1000)


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        base = int(time.time() * 1000) - 86_400_000
        self.page_cap = 2
        self.accounts = {i: {"Id": f"001STUB{i:03d}", "Name": f"account {i}", "IsDeleted": False, "Industry__c": "x", "AnnualRevenue": i * 1000.0, "_ms": base + i * 600_000} for i in (1, 2, 3)}
        self.regions = {1: {"Id": "a01STUB001", "Name": "north"}, 2: {"Id": "a01STUB002", "Name": "south"}, 3: {"Id": "a01STUB003", "Name": "east"}}
        self.account_fields = dict(_ACCOUNT_FIELDS)
        self.revoked = False
        self.forbidden = False
        self.quota_used_up = False
        self.evil_instance = False
        self.rate_limits_left = 0
        self.wanted: list[str] = []
        self.log: list[str] = []


STATE = State()


def _words(message: str, code: str) -> list:
    return [{"message": f"{message} STUB-VENDOR-WORDS", "errorCode": code}]


def _authorised(handler, parsed) -> bool:  # noqa: ANN001
    """Record the request; answer 401, 403 or 429 where armed. True when it may go on."""
    with stub.STATE_LOCK:
        # PR #216 final review BLOCKER 19: the query string too. The SOQL cursor filter
        # (`WHERE SystemModstamp >= ...`) is the only evidence that the reload asked for the changes
        # rather than reading everything again; logging the path alone cannot show it.
        STATE.log.append(f"{handler.command} {parsed.path}" + (f"?{parsed.query}" if parsed.query else ""))
        if STATE.revoked or handler.headers.get("Authorization") != f"Bearer {ACCESS_TOKEN}":
            stub.send_json(handler, _words("Session expired or invalid", "INVALID_SESSION_ID"), 401)
            return False
        if STATE.forbidden:
            stub.send_json(handler, _words("The user does not have access to this resource", "INSUFFICIENT_ACCESS"), 403)
            return False
        if STATE.quota_used_up:  # Salesforce's request limit is HTTP 403, not 429 (slice 3 review BLOCKER 8)
            stub.send_json(handler, _words("TotalRequests Limit exceeded.", "REQUEST_LIMIT_EXCEEDED"), 403)
            return False
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            stub.send_json(handler, _words("Too many requests", "REQUEST_LIMIT_EXCEEDED"), 429, {"Retry-After": "1"})
            return False
    return True


def _limits(handler, parsed) -> None:  # noqa: ANN001
    if _authorised(handler, parsed):
        stub.send_json(handler, {"DailyApiRequests": {"Max": 15000, "Remaining": 14990}})


def _sobjects(handler, parsed) -> None:  # noqa: ANN001
    if _authorised(handler, parsed):
        stub.send_json(handler, {"encoding": "UTF-8", "maxBatchSize": 200, "sobjects": [
            {"name": "Account", "label": "Account", "queryable": True, "replicateable": True, "custom": False},
            {"name": "Region__c", "label": "Region", "queryable": True, "replicateable": False, "custom": True},
            {"name": "Hidden__c", "label": "Hidden", "queryable": True, "replicateable": True, "custom": True, "deprecatedAndHidden": True},
        ]})  # fmt: skip


def _describe(handler, parsed) -> None:  # noqa: ANN001
    if not _authorised(handler, parsed):
        return
    name = parsed.path.split("/")[-2]
    with stub.STATE_LOCK:
        table = {"Account": STATE.account_fields, "Region__c": _REGION_FIELDS}.get(name)
        if table is None:
            return stub.send_json(handler, _words("The requested resource does not exist", "NOT_FOUND"), 404)
        stub.send_json(handler, {"name": name, "fields": [{"name": n, "type": t, "compoundFieldName": None} for n, t in table.items()]})


def _row(source: dict, wanted: list[str], kind: str) -> dict:
    out = {"attributes": {"type": kind, "url": f"{API}/sobjects/{kind}/{source['Id']}"}}
    for field in wanted:
        out[field] = _stamp(source["_ms"]) if field == "SystemModstamp" else source.get(field)
    return out


def _query(by_queryall: bool):  # noqa: ANN202
    def route(handler, parsed) -> None:  # noqa: ANN001
        include_deleted = by_queryall
        if not _authorised(handler, parsed):
            return
        locator = parsed.path.split(f"{API}/query")[1].lstrip("/").removeprefix("All").lstrip("/")
        query = {k: v[0] for k, v in parse_qs(parsed.query).items()}
        with stub.STATE_LOCK:
            if locator:  # a nextRecordsUrl: 01gSTUB<kind>-<offset>-<floor_ms>-<object>
                tag, offset, floor, kind = locator.split("-", 3)
                offset, floor = int(offset), int(floor)
                include_deleted = tag.endswith("A")  # the locator belongs to the query that made it
                wanted = STATE.wanted
            else:
                match = re.match(r"SELECT (.+?) FROM (\w+)\s*(WHERE SystemModstamp >= (\S+))?\s*(ORDER BY \w+ ASC)?\s*$", query["q"])
                if not match:
                    return stub.send_json(handler, _words("unexpected query", "MALFORMED_QUERY"), 400)
                wanted, kind = match.group(1).split(", "), match.group(2)
                offset, floor = 0, _ms_of(match.group(4)) if match.group(4) else 0
                STATE.wanted = wanted
            if kind == "Account":
                rows = sorted(STATE.accounts.values(), key=lambda r: (r["_ms"], r["Id"]))
                rows = [r for r in rows if (include_deleted or not r["IsDeleted"]) and r["_ms"] >= floor]
            elif kind == "Region__c":
                rows = [dict(r, _ms=0) for r in STATE.regions.values()]
            else:
                return stub.send_json(handler, _words("sObject type is not supported", "INVALID_TYPE"), 400)
            page = rows[offset : offset + STATE.page_cap]
            end = offset + len(page)
            body = {"totalSize": len(rows), "done": end >= len(rows), "records": [_row(r, wanted, kind) for r in page]}
            if not body["done"]:
                body["nextRecordsUrl"] = f"{API}/query/01gSTUB{'A' if include_deleted else 'Q'}-{end}-{floor}-{kind}"
        stub.send_json(handler, body)

    return route


def _token(handler, parsed) -> None:  # noqa: ANN001
    """`POST /services/oauth2/token`, the refresh grant with the client in the form body."""
    form = stub.read_form(handler)
    with stub.STATE_LOCK:
        STATE.log.append(f"POST {parsed.path}")
        ok = (
            not STATE.revoked and form.get("grant_type") == "refresh_token" and form.get("refresh_token") == REFRESH_TOKEN
            and form.get("client_id") == CLIENT_ID and form.get("client_secret") == CLIENT_SECRET
        )  # fmt: skip
        instance = "https://evil.example.invalid" if STATE.evil_instance else INSTANCE_URL
    if not ok:
        return stub.send_json(handler, {"error": "invalid_grant", "error_description": "expired access/refresh token STUB-VENDOR-WORDS"}, 400)
    stub.send_json(handler, {"access_token": ACCESS_TOKEN, "instance_url": instance, "id": "https://login.salesforce.com/id/00DSTUB/005STUB", "token_type": "Bearer", "scope": "api refresh_token", "issued_at": str(int(time.time() * 1000))})


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not a Salesforce route."""
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
        elif action == "put":  # add or change an account, modified now
            row = STATE.accounts.setdefault(int(body["id"]), {"Id": f"001STUB{int(body['id']):03d}", "IsDeleted": False, "Industry__c": "x", "AnnualRevenue": 0.0})
            row.update(body.get("fields", {}))
            row["_ms"] = int(time.time() * 1000)
        elif action == "delete":  # a delete at Salesforce: into the recycle bin, IsDeleted true, stamp moved
            row = STATE.accounts[int(body["id"])]
            row["IsDeleted"], row["_ms"] = True, int(time.time() * 1000)
        elif action == "add_field":
            STATE.account_fields[body["name"]] = body.get("type", "string")
            for row in STATE.accounts.values():
                row.setdefault(body["name"], "x")
        elif action == "revoke":
            STATE.revoked = bool(body.get("on", True))
        elif action == "forbid":
            STATE.forbidden = bool(body.get("on", True))
        elif action == "quota":
            STATE.quota_used_up = bool(body.get("on", True))
        elif action == "rate_limit":
            STATE.rate_limits_left = int(body.get("count", 1))
        elif action == "evil_instance":
            STATE.evil_instance = bool(body.get("on", True))
        else:
            return stub.send_json(handler, {"error": "unknown action"}, 400)
    stub.send_json(handler, {"ok": True})


stub.ROUTES[("GET", f"/salesforce{API}/limits")] = _limits
stub.ROUTES[("GET", f"/salesforce{API}/sobjects")] = lambda h, p: _describe(h, p) if p.path.endswith("/describe") else _sobjects(h, p)
stub.ROUTES[("GET", f"/salesforce{API}/query")] = _query(False)
stub.ROUTES[("GET", f"/salesforce{API}/queryAll")] = _query(True)
stub.ROUTES[("POST", "/salesforce/services/oauth2/token")] = _token
stub.ROUTES[("GET", "/__admin/salesforce")] = _admin
stub.ROUTES[("POST", "/__admin/salesforce")] = _admin
