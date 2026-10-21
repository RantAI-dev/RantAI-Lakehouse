"""The mock Zendesk of the SaaS gate (`SRC-3` V-Zendesk.4), registered into
`saas_stub.py`. It behaves as Zendesk documents its Support API
(https://developer.zendesk.com/api-reference/ticketing/ticket-management/incremental_exports/):

- the cursor-based incremental export (`/incremental/tickets/cursor.json`, `users`): the
  first request takes `start_time`, every page has `after_url` and `end_of_stream`;
- the time-based export (`/incremental/tickets.json`, `ticket_events.json`): a page ends
  at the `generated_timestamp` of its last item and the next page starts AT that time,
  so the items sharing it come twice, the very duplicate of dlt-hub/verified-sources#659
  (the gate asserts the mock still sends it, so a mock that stopped would not prove the fix);
- neither export returns data from the most recent minute ("to prevent race conditions"),
  so `put` back-dates a change by `age_seconds` (default 120);
- a deleted ticket stays in the export with `status = "deleted"`;
- cursor-paged lists (`/groups.json`: `meta.has_more`, `links.next`);
- failures in Zendesk's shapes: 401 `{"error": ...}`, 403, 429 with `Retry-After`; every
  error body carries `STUB-VENDOR-WORDS` so the gate can prove the product repeats none.

Pages are capped at `STATE.page_cap` (3) however large `per_page` is, so a handful of
records shows the paging. Credentials are placeholders: an e-mail and an API token that
are only strings here. The browser sign-in is mocked at `POST /oauth/tokens`, which like
Zendesk's takes a JSON body (a form body is refused with 400, as the real endpoint would
fail it) and rotates the refresh token: the old one stops working.
"""
from __future__ import annotations

import base64
import time
from datetime import datetime, timezone
from urllib.parse import parse_qs, urlparse

import saas_stub as stub

EMAIL = "agent@example.invalid"
API_TOKEN = "stub-api-token"
USER_ID = 7
CLIENT_ID = "stub-client"
CLIENT_SECRET = "stub-client-secret"


def _iso(ts: int) -> str:
    return datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _ticket(i: int, ts: int, **extra) -> dict:
    return {
        "id": i, "subject": f"ticket {i}", "status": "open", "created_at": _iso(ts), "updated_at": _iso(ts),
        "due_at": None, "generated_timestamp": ts, "custom_fields": [], "fields": [], "tags": [], **extra,
    }  # fmt: skip


class State:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        base = int(time.time()) - 86_400
        self.page_cap = 3
        # Tickets 3 and 4 share a time, so a time-based page boundary falls between equals.
        self.tickets = [_ticket(i, base + ts) for i, ts in ((1, 100), (2, 200), (3, 300), (4, 300), (5, 400), (6, 500))]
        self.events = [
            {"id": i, "ticket_id": 1, "created_at": _iso(base + ts), "timestamp": base + ts, "generated_timestamp": base + ts}
            for i, ts in ((1, 100), (2, 200), (3, 300), (4, 300), (5, 400), (6, 500))
        ]
        self.users = [{"id": i, "name": f"user {i}", "updated_at": _iso(base + 100 * i), "generated_timestamp": base + 100 * i} for i in range(1, 5)]
        self.groups = [{"id": i, "name": f"group {i}"} for i in range(1, 8)]
        self.revoked = False
        self.refresh_token = "stub-zd-refresh-0"
        self.rotations = 0
        self.forbidden = False
        self.rate_limits_left = 0
        self.extra_field: str | None = None
        self.log: list[str] = []


STATE = State()


def _words(message: str) -> dict:
    return {"error": f"{message} STUB-VENDOR-WORDS"}


def _gate(handler, parsed) -> bool:  # noqa: ANN001
    """Record the request; answer 401, 403 or 429 where armed. True when it may go on."""
    expected = "Basic " + base64.b64encode(f"{EMAIL}/token:{API_TOKEN}".encode()).decode()
    with stub.STATE_LOCK:
        STATE.log.append(f"{handler.command} {parsed.path}")
        bearer = f"Bearer stub-zd-access-{STATE.rotations}"
        if STATE.revoked or handler.headers.get("Authorization") not in (expected, bearer):
            stub.send_json(handler, _words("Couldn't authenticate you"), 401)
            return False
        if STATE.forbidden:
            stub.send_json(handler, {"error": {"title": "Forbidden", "message": "You do not have access to this page. STUB-VENDOR-WORDS"}}, 403)
            return False
        if STATE.rate_limits_left > 0:
            STATE.rate_limits_left -= 1
            stub.send_json(handler, "Number of allowed API requests per minute exceeded STUB-VENDOR-WORDS", 429, {"Retry-After": "1"})
            return False
    return True


def _query(parsed) -> dict:  # noqa: ANN001
    return {k: v[0] for k, v in parse_qs(parsed.query).items()}


def _visible(items: list[dict]) -> list[dict]:
    """Zendesk's export never returns the most recent minute."""
    horizon = int(time.time()) - 60
    return [i for i in items if i["generated_timestamp"] <= horizon]


def _me(handler, parsed) -> None:  # noqa: ANN001
    if _gate(handler, parsed):
        stub.send_json(handler, {"user": {"id": USER_ID, "email": EMAIL, "role": "admin"}})


def _decorate(item: dict) -> dict:
    return {**item, STATE.extra_field: "x"} if STATE.extra_field and "subject" in item else item


def _time_based(key: str):  # noqa: ANN202
    def route(handler, parsed) -> None:  # noqa: ANN001
        if not _gate(handler, parsed):
            return
        query = _query(parsed)
        with stub.STATE_LOCK:
            store = _visible(getattr(STATE, "tickets" if key == "tickets" else "events"))
            start = int(query["start_time"])
            hits = sorted((i for i in store if i["generated_timestamp"] >= start), key=lambda i: (i["generated_timestamp"], i["id"]))
            page = hits[: STATE.page_cap]
            end = page[-1]["generated_timestamp"] if page else start
            body = {key: [_decorate(i) for i in page], "count": len(page), "end_time": end, "end_of_stream": len(hits) <= STATE.page_cap}
            body["next_page"] = f"{stub.origin_of(handler)}{parsed.path}?start_time={end}&per_page={STATE.page_cap}"
        stub.send_json(handler, body)

    return route


def _cursor_based(key: str):  # noqa: ANN202
    def route(handler, parsed) -> None:  # noqa: ANN001
        if not _gate(handler, parsed):
            return
        query = _query(parsed)
        with stub.STATE_LOCK:
            store = sorted(_visible(getattr(STATE, key)), key=lambda i: (i["generated_timestamp"], i["id"]))
            position = int(query["cursor"]) if "cursor" in query else sum(1 for i in store if i["generated_timestamp"] < int(query["start_time"]))
            page = store[position : position + STATE.page_cap]
            after = position + len(page)
            body = {
                key: [_decorate(i) for i in page], "count": len(page), "after_cursor": str(after),
                "after_url": f"{stub.origin_of(handler)}{parsed.path}?cursor={after}", "end_of_stream": after >= len(store),
            }  # fmt: skip
        stub.send_json(handler, body)

    return route


def _groups(handler, parsed) -> None:  # noqa: ANN001
    if not _gate(handler, parsed):
        return
    query = _query(parsed)
    with stub.STATE_LOCK:
        start = int(query.get("after", 0))
        page = STATE.groups[start : start + STATE.page_cap]
        more = start + STATE.page_cap < len(STATE.groups)
        body = {
            "groups": page, "meta": {"has_more": more, "after_cursor": str(start + len(page))},
            "links": {"next": f"{stub.origin_of(handler)}{parsed.path}?page%5Bsize%5D={STATE.page_cap}&after={start + STATE.page_cap}"},
        }  # fmt: skip
    stub.send_json(handler, body)


def _token(handler, parsed) -> None:  # noqa: ANN001
    """`POST /oauth/tokens`, the refresh grant. JSON in, as Zendesk requires; each refresh
    issues a new access and refresh token and retires the old ones."""
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
            return stub.send_json(handler, {"error": "invalid_grant", "error_description": "STUB-VENDOR-WORDS"}, 400)
        STATE.rotations += 1
        STATE.refresh_token = f"stub-zd-refresh-{STATE.rotations}"
        out = {"access_token": f"stub-zd-access-{STATE.rotations}", "refresh_token": STATE.refresh_token, "expires_in": 1800, "scope": "read", "token_type": "bearer"}
    stub.send_json(handler, out)


def _admin(handler, parsed) -> None:  # noqa: ANN001
    """The gate's remote control over the mock's state; not a Zendesk route."""
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
        elif action == "put":  # add or change a ticket; back-dated so the one-minute rule shows it
            ts = int(time.time()) - int(body.get("age_seconds", 120))
            fresh = _ticket(int(body["id"]), ts, **body.get("fields", {}))
            STATE.tickets = [t for t in STATE.tickets if t["id"] != fresh["id"]] + [fresh]
        elif action == "delete":  # a delete at Zendesk: the ticket stays, with status deleted
            ts = int(time.time()) - int(body.get("age_seconds", 120))
            STATE.tickets = [_ticket(t["id"], ts, status="deleted") if t["id"] == int(body["id"]) else t for t in STATE.tickets]
        elif action == "refresh_token":  # tell the gate which refresh token is current
            return stub.send_json(handler, {"refresh_token": STATE.refresh_token})
        elif action == "add_field":
            STATE.extra_field = body["name"]
        elif action == "revoke":
            STATE.revoked = bool(body.get("on", True))
        elif action == "forbid":
            STATE.forbidden = bool(body.get("on", True))
        elif action == "rate_limit":
            STATE.rate_limits_left = int(body.get("count", 1))
        else:
            return stub.send_json(handler, {"error": "unknown action"}, 400)
    stub.send_json(handler, {"ok": True})


stub.ROUTES[("GET", "/zendesk/api/v2/users/me.json")] = _me
stub.ROUTES[("GET", "/zendesk/api/v2/incremental/tickets.json")] = _time_based("tickets")
stub.ROUTES[("GET", "/zendesk/api/v2/incremental/ticket_events.json")] = _time_based("ticket_events")
stub.ROUTES[("GET", "/zendesk/api/v2/incremental/tickets/cursor.json")] = _cursor_based("tickets")
stub.ROUTES[("GET", "/zendesk/api/v2/incremental/users/cursor.json")] = _cursor_based("users")
stub.ROUTES[("GET", "/zendesk/api/v2/groups.json")] = _groups
stub.ROUTES[("POST", "/zendesk/oauth/tokens")] = _token
stub.ROUTES[("GET", "/__admin/zendesk")] = _admin
stub.ROUTES[("POST", "/__admin/zendesk")] = _admin
