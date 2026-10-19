"""The vendored Zendesk source (`saas/vendored/zendesk/`, `SRC-3` V-Zendesk.1):
its HTTP goes only through the `SaasSession` it is given (AD3, AD4), and the
duplicates of upstream issue dlt-hub/verified-sources#659 (time-based incremental
export) are found and fixed. No network: `FakeZendesk` answers the incremental
exports as Zendesk documents them
(https://developer.zendesk.com/documentation/api-basics/working-with-data/using-the-incremental-export-api/):
a time-based page ends at the `generated_timestamp` of its last item and the next
page starts AT that time, so items sharing it come twice; a cursor-based export
hands back an `after_url` and repeats nothing within a run."""

from __future__ import annotations

import ast
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

import pytest

from dispar_orchestrate import ssrf_guard
from dispar_orchestrate.saas.session import SaasSession
from dispar_orchestrate.saas.vendored.zendesk import settings as zd_settings
from dispar_orchestrate.saas.vendored.zendesk.helpers import talk_api

VENDORED = Path(talk_api.__file__).parent.parent
ORIGIN = "https://acme.zendesk.com"


class Reply:
    def __init__(self, status: int = 200, body: Any = None, headers: dict | None = None) -> None:
        self.status_code = status
        self._body = body if body is not None else {}
        self.headers = headers or {}

    def json(self) -> Any:
        return self._body

    def raise_for_status(self) -> None:
        assert self.status_code < 400


def iso(ts: int) -> str:
    from datetime import datetime, timezone

    return datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def ticket(i: int, ts: int, **extra: Any) -> dict[str, Any]:
    return {
        "id": i, "subject": f"t{i}", "status": "open", "created_at": iso(ts), "updated_at": iso(ts), "due_at": None,
        "generated_timestamp": ts, "custom_fields": [], "fields": [], "tags": [], **extra,
    }  # fmt: skip


class FakeZendesk:
    """The documented incremental exports, over the HTTP seam of `SaasSession`."""

    def __init__(self, tickets: list[dict[str, Any]], page_size: int = 3) -> None:
        self.tickets = tickets
        self.page_size = page_size
        self.log: list[str] = []
        self.headers: list[dict] = []

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        self.headers.append(headers)
        parsed = urlparse(url)
        query = {**{k: v[0] for k, v in parse_qs(parsed.query).items()}, **{k: str(v) for k, v in (params or {}).items()}}
        self.log.append(f"{parsed.path}?{'&'.join(f'{k}={v}' for k, v in sorted(query.items()))}")
        if parsed.path == "/api/v2/incremental/tickets.json":  # time-based
            start = int(query["start_time"])
            hits = sorted((t for t in self.tickets if t["generated_timestamp"] >= start), key=lambda t: (t["generated_timestamp"], t["id"]))
            page = hits[: self.page_size]
            end = page[-1]["generated_timestamp"] if page else start
            body = {"tickets": page, "count": len(page), "end_time": end, "end_of_stream": len(hits) <= self.page_size}
            body["next_page"] = f"{ORIGIN}{parsed.path}?start_time={end}&per_page={self.page_size}"
            return Reply(body=body)
        if parsed.path == "/api/v2/incremental/tickets/cursor.json":  # cursor-based
            position = int(query["cursor"]) if "cursor" in query else sum(1 for t in self.tickets if t["generated_timestamp"] < int(query["start_time"]))
            order = sorted(self.tickets, key=lambda t: (t["generated_timestamp"], t["id"]))
            page = order[position : position + self.page_size]
            after = position + len(page)
            return Reply(body={
                "tickets": page, "after_cursor": str(after), "after_url": f"{ORIGIN}{parsed.path}?cursor={after}",
                "end_of_stream": after >= len(order),
            })  # fmt: skip
        return Reply(404)


def resolver(host: str, port: int, **_kw: object) -> ssrf_guard.ResolvedAddress:
    return ssrf_guard.ResolvedAddress(ip="93.184.216.34", port=port, family=2)


def session_over(fake: FakeZendesk) -> SaasSession:
    return SaasSession(ORIGIN, lambda: {"Authorization": "Bearer t"}, http_get=fake.get, resolve_checked=resolver, sleep=lambda _s: None)


def test_no_vendored_zendesk_module_imports_an_http_library() -> None:
    forbidden = {"requests", "urllib3", "httpx", "http", "urllib"}
    for path in VENDORED.rglob("*.py"):
        for node in ast.walk(ast.parse(path.read_text())):
            names = [a.name for a in node.names] if isinstance(node, ast.Import) else [node.module or ""] if isinstance(node, ast.ImportFrom) else []
            for name in names:
                assert name.split(".")[0] not in forbidden, f"{path.name} imports {name}"
            if isinstance(node, ast.ImportFrom) and node.module == "dlt.sources.helpers":
                assert all(a.name not in ("requests", "client") for a in node.names), f"{path.name} imports dlt's requests"
            if isinstance(node, ast.ImportFrom) and node.module == "dlt.sources.helpers.requests":
                raise AssertionError(f"{path.name} imports dlt's requests")


def test_pages_come_through_the_session_and_a_link_to_another_host_is_refused() -> None:
    fake = FakeZendesk([ticket(i, 100 + i) for i in range(1, 8)])
    client = talk_api.ZendeskAPIClient(session_over(fake))
    pages = list(client.get_pages("/api/v2/incremental/tickets.json", "tickets", talk_api.PaginationType.STREAM, params={"start_time": 0}))
    assert fake.headers and all(h == {"Authorization": "Bearer t"} for h in fake.headers)
    assert pages, "paged through the session"
    evil = FakeZendesk([ticket(1, 1)])
    original = evil.get

    def to_another_host(url: str, **kw: Any) -> Reply:
        reply = original(url, **kw)
        reply._body["next_page"] = "https://evil.example.invalid/api/v2/incremental/tickets.json?start_time=1"
        reply._body["end_of_stream"] = False
        return reply

    evil.get = to_another_host  # type: ignore[method-assign]
    with pytest.raises(ssrf_guard.SsrfBlocked):
        list(talk_api.ZendeskAPIClient(session_over(evil)).get_pages("/api/v2/incremental/tickets.json", "tickets", talk_api.PaginationType.STREAM, params={"start_time": 0}))


def test_the_time_based_export_sends_the_tickets_at_a_page_boundary_twice() -> None:
    """The duplicate of dlt-hub/verified-sources#659, as upstream's `ticket_table`
    reads it (`PaginationType.STREAM` on `/incremental/tickets.json`): page 1 ends with
    ticket 3 (time 300), so page 2 starts AT 300 and ticket 3 comes again (and ticket
    5 at the next boundary). Zendesk documents it: "the exported data may contain
    duplicate items"."""
    fake = FakeZendesk([ticket(1, 100), ticket(2, 200), ticket(3, 300), ticket(4, 300), ticket(5, 400), ticket(6, 500)])
    client = talk_api.ZendeskAPIClient(session_over(fake))
    pages = client.get_pages("/api/v2/incremental/tickets.json", "tickets", talk_api.PaginationType.STREAM, params={"start_time": 0})
    ids = [t["id"] for page in pages for t in page]
    assert ids == [1, 2, 3, 3, 4, 5, 5, 6], f"the vendor repeats tickets at the boundaries: {ids}"
