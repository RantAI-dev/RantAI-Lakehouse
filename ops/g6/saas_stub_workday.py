"""The mock Workday of the SaaS gate (`SRC-3` V-Workday.4), registered into `saas_stub.py`. It
behaves as the developer models Workday Reports-as-a-Service (Workday's own documentation is
behind a login and could not be read, so this is a model, not a recording; the plan's handoff
says so):

- `GET /ccx/service/customreport2/<tenant>/<owner>/<report>?format=json`: ONE response with every
  row under `Report_Entry` (no paging, no change filter), HTTP Basic for the integration system
  user (`<user>@<tenant>`) or a bearer token of the API client;
- `POST /ccx/oauth2/<tenant>/token`: the refresh-token grant, the client in the HTTP Basic header,
  no new refresh token in the answer;
- failures: 401, 403, 429 with `Retry-After`, 404 for a report that does not exist; every error body
  carries `STUB-VENDOR-WORDS` so the gate can prove the product repeats none.

Credentials are placeholders that are only strings here.
"""
from __future__ import annotations

import base64
from urllib.parse import parse_qs

import saas_stub as stub

TENANT = "acme_tenant"
OWNER_REPORT = "isu_owner/Headcount_Report"
ISU_USER = "isu_gate@acme_tenant"
ISU_PASSWORD = "wd-password"
OAUTH_CLIENT_ID = "stub-client"
OAUTH_CLIENT_SECRET = "stub-client-secret"
REFRESH_TOKEN = "stub-wd-refresh-0"
ACCESS_TOKEN = "wd-access"


def _basic(user: str, password: str) -> str:
    return "Basic " + base64.b64encode(f"{user}:{password}".encode()).decode()


def _person(i: int, **extra) -> dict:
    return {"Employee_ID": f"21{i:03d}", "Name": f"person {i}", "Cost_Center": "CC-100", **extra}


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        self.rows = [_person(i) for i in (1, 2, 3)]
        self.revoked = False
        self.forbidden = False
        self.rate_limits_left = 0
        self.log: list[str] = []


STATE = State()


def _words(message: str) -> dict:
    return {"error": "invalid_request", "error_description": message + " STUB-VENDOR-WORDS"}


def _report(handler, parsed) -> None:  # noqa: ANN001
    with stub.STATE_LOCK:
        STATE.log.append(f"GET {parsed.path}?{parsed.query}")
        auth = handler.headers.get("Authorization")
        if STATE.revoked or auth not in (_basic(ISU_USER, ISU_PASSWORD), f"Bearer {ACCESS_TOKEN}"):
            return stub.send_json(handler, _words("Invalid username or password"), 401)
        if STATE.forbidden:
            return stub.send_json(handler, _words("Not authorized for this report"), 403)
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            return stub.send_json(handler, _words("Too many requests"), 429, {"Retry-After": "1"})
        wanted = parsed.path.removeprefix(f"/ccx/service/customreport2/{TENANT}/")
        if wanted != OWNER_REPORT or parse_qs(parsed.query).get("format") != ["json"]:
            return stub.send_json(handler, _words("Report not found"), 404)
        stub.send_json(handler, {"Report_Entry": [dict(r) for r in STATE.rows]})


def _token(handler, parsed) -> None:  # noqa: ANN001
    """`POST /ccx/oauth2/<tenant>/token`: the refresh-token grant with the client in the Basic header."""
    form = stub.read_form(handler)
    with stub.STATE_LOCK:
        STATE.log.append(f"POST {parsed.path} grant_type={form.get('grant_type')}")
        ok = (
            not STATE.revoked
            and handler.headers.get("Authorization") == _basic(OAUTH_CLIENT_ID, OAUTH_CLIENT_SECRET)
            and "client_secret" not in form
            and form.get("grant_type") == "refresh_token"
            and form.get("refresh_token") == REFRESH_TOKEN
        )
    if not ok:
        return stub.send_json(handler, {"error": "invalid_client", "error_description": "STUB-VENDOR-WORDS"}, 401)
    stub.send_json(handler, {"access_token": ACCESS_TOKEN, "token_type": "Bearer", "expires_in": 3600})


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not a Workday route."""
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
        elif action == "put":  # add or change a row
            fresh = _person(int(body["id"]), **body.get("fields", {}))
            STATE.rows = [r for r in STATE.rows if r["Employee_ID"] != fresh["Employee_ID"]] + [fresh]
        elif action == "delete":  # a deleted row is simply not in the next answer
            STATE.rows = [r for r in STATE.rows if r["Employee_ID"] != _person(int(body["id"]))["Employee_ID"]]
        elif action == "add_field":
            STATE.rows = [{**r, body["name"]: "x"} for r in STATE.rows]
        elif action == "revoke":
            STATE.revoked = bool(body.get("on", True))
        elif action == "forbid":
            STATE.forbidden = bool(body.get("on", True))
        elif action == "rate_limit":
            STATE.rate_limits_left = int(body.get("count", 1))
        else:
            return stub.send_json(handler, {"error": "unknown action"}, 400)
    stub.send_json(handler, {"ok": True})


stub.ROUTES[("GET", "/workday/ccx/service/customreport2")] = _report
stub.ROUTES[("POST", f"/workday/ccx/oauth2/{TENANT}/token")] = _token
stub.ROUTES[("GET", "/__admin/workday")] = _admin
stub.ROUTES[("POST", "/__admin/workday")] = _admin
