"""The mock ServiceNow of the SaaS gate (`SRC-3` V-ServiceNow.4), registered into
`saas_stub.py`. It behaves as ServiceNow documents its Table API
(https://www.servicenow.com/docs/r/xanadu/api-reference/rest-apis/c_TableAPI.html):

- `GET /api/now/table/{table}` with an encoded `sysparm_query` (`sys_updated_on>=<ts>^ORDERBY...`),
  `sysparm_limit`, `sysparm_fields`; the answer is `{"result": [...]}` with every value a string;
- the page limit is applied BEFORE the access check, and the `Link` header carries `rel="next"`
  as an absolute address (pages are capped at `STATE.page_cap` however large the limit);
- `sys_dictionary` and `sys_db_object` answer the table's fields and parent (`incident` extends `task`);
- `POST /oauth_token.do`, form body: the `client_credentials` grant and the `refresh_token` grant;
- failures in ServiceNow's shape: 401 `{"error": {"message": ..., "detail": ...}, "status": "failure"}`,
  403, 429 with `Retry-After`, 400 for an unknown table; every error body carries
  `STUB-VENDOR-WORDS` so the gate can prove the product repeats none.

No delete is modelled: the Table API does not report one in the table's own read (the connector
claims none). Credentials are placeholders that are only strings here.
"""
from __future__ import annotations

import re
from datetime import datetime, timedelta, timezone
from urllib.parse import parse_qs, urlencode

import saas_stub as stub

CLIENT_ID = "snow-client"
CLIENT_SECRET = "snow-secret"
OAUTH_CLIENT_ID = "stub-client"
OAUTH_CLIENT_SECRET = "stub-client-secret"
REFRESH_TOKEN = "stub-refresh-token"
ACCESS_TOKEN = "snow-access"


def _stamp(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M:%S")


def _incident(i: int, moment: datetime, **extra) -> dict:
    return {"sys_id": f"snow{i:028d}", "number": f"INC{i:07d}", "short_description": f"incident {i}", "state": "1", "sys_updated_on": _stamp(moment), **extra}


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        now = datetime.now(timezone.utc)
        self.page_cap = 2
        self.incidents = [_incident(i, now - timedelta(days=13 - i)) for i in (1, 2, 3)]
        self.users = [{"sys_id": "snowuser0000000000000000000000001", "sys_updated_on": _stamp(now)}]
        # table -> (fields it defines, parent)
        self.dictionary: dict[str, tuple[list[str], str | None]] = {
            "incident": (["number", "short_description"], "task"),
            "task": (["sys_id", "sys_updated_on", "state"], None),
        }
        self.revoked = False
        self.forbidden = False
        self.rate_limits_left = 0
        self.log: list[str] = []


STATE = State()


def _words(message: str) -> dict:
    return {"error": {"message": message, "detail": "STUB-VENDOR-WORDS"}, "status": "failure"}


def _gate(handler, parsed) -> bool:  # noqa: ANN001
    """Record the request; answer 401, 403 or 429 where armed. True when it may go on."""
    with stub.STATE_LOCK:
        STATE.log.append(f"{handler.command} {parsed.path}?{parsed.query}")
        if STATE.revoked or handler.headers.get("Authorization") != f"Bearer {ACCESS_TOKEN}":
            stub.send_json(handler, _words("User Not Authenticated"), 401)
            return False
        if STATE.forbidden:
            stub.send_json(handler, _words("User Not Authorized"), 403)
            return False
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            stub.send_json(handler, _words("Rate limit exceeded"), 429, {"Retry-After": "1"})
            return False
    return True


def _query(parsed) -> dict:  # noqa: ANN001
    return {k: v[0] for k, v in parse_qs(parsed.query).items()}


def _table(handler, parsed) -> None:  # noqa: ANN001
    if not _gate(handler, parsed):
        return
    name = parsed.path.rsplit("/", 1)[-1]
    query = _query(parsed)
    with stub.STATE_LOCK:
        if name == "sys_dictionary":
            table = re.match(r"name=([a-z_]+)\^", query["sysparm_query"]).group(1)  # type: ignore[union-attr]
            fields = STATE.dictionary.get(table, ([], None))[0]
            return stub.send_json(handler, {"result": [{"element": e} for e in fields] + [{"element": ""}]})
        if name == "sys_db_object":
            table = query["sysparm_query"].split("=", 1)[1]
            return stub.send_json(handler, {"result": [{"super_class.name": STATE.dictionary.get(table, ([], None))[1] or ""}]})
        store = {"incident": STATE.incidents, "sys_user": STATE.users}.get(name)
        if store is None:
            return stub.send_json(handler, _words("Invalid table"), 400)
        rows = list(store)
        since = re.match(r"sys_updated_on>=([^\^]+)\^", query.get("sysparm_query", ""))
        if since:
            rows = [r for r in rows if r["sys_updated_on"] >= since.group(1)]
        rows.sort(key=lambda r: (r["sys_updated_on"], r["sys_id"]))
        offset = int(query.get("sysparm_offset", 0))
        limit = min(int(query.get("sysparm_limit", STATE.page_cap)), STATE.page_cap)
        page = rows[offset : offset + limit]
        if "sysparm_fields" in query:
            wanted = query["sysparm_fields"].split(",")
            page = [{k: v for k, v in r.items() if k in wanted} for r in page]
        headers = {}
        if offset + limit < len(rows):
            more = urlencode({**query, "sysparm_limit": limit, "sysparm_offset": offset + limit})
            headers["Link"] = f'<{stub.origin_of(handler)}{parsed.path}?{more}>;rel="next",<{stub.origin_of(handler)}{parsed.path}?sysparm_offset=0>;rel="first"'
        stub.send_json(handler, {"result": page}, 200, headers)


def _token(handler, parsed) -> None:  # noqa: ANN001
    """`POST /oauth_token.do`: the client-credentials grant and the refresh grant, form body."""
    form = stub.read_form(handler)
    with stub.STATE_LOCK:
        STATE.log.append(f"POST {parsed.path} grant_type={form.get('grant_type')}")
        grant = form.get("grant_type")
        ok = not STATE.revoked and (
            (grant == "client_credentials" and form.get("client_id") == CLIENT_ID and form.get("client_secret") == CLIENT_SECRET)
            or (
                grant == "refresh_token" and form.get("refresh_token") == REFRESH_TOKEN
                and form.get("client_id") == OAUTH_CLIENT_ID and form.get("client_secret") == OAUTH_CLIENT_SECRET
            )
        )  # fmt: skip
    if not ok:
        return stub.send_json(handler, {"error": "access_denied", "error_description": "STUB-VENDOR-WORDS"}, 401)
    stub.send_json(handler, {"access_token": ACCESS_TOKEN, "token_type": "Bearer", "expires_in": 1800, "scope": "useraccount"})


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not a ServiceNow route."""
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
        elif action == "put":  # add or change an incident; ServiceNow stamps sys_updated_on itself
            fresh = _incident(int(body["id"]), datetime.now(timezone.utc), **body.get("fields", {}))
            STATE.incidents = [r for r in STATE.incidents if r["sys_id"] != fresh["sys_id"]] + [fresh]
        elif action == "add_field":
            STATE.dictionary["incident"][0].append(body["name"])
            STATE.incidents = [{**r, body["name"]: "x"} for r in STATE.incidents]
        elif action == "revoke":
            STATE.revoked = bool(body.get("on", True))
        elif action == "forbid":
            STATE.forbidden = bool(body.get("on", True))
        elif action == "rate_limit":
            STATE.rate_limits_left = int(body.get("count", 1))
        else:
            return stub.send_json(handler, {"error": "unknown action"}, 400)
    stub.send_json(handler, {"ok": True})


stub.ROUTES[("GET", "/servicenow/api/now/table")] = _table
stub.ROUTES[("POST", "/servicenow/oauth_token.do")] = _token
stub.ROUTES[("GET", "/__admin/servicenow")] = _admin
stub.ROUTES[("POST", "/__admin/servicenow")] = _admin
