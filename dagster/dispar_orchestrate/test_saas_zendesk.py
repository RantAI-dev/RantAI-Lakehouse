"""`saas/vendors/zendesk.py` (`SRC-3` V-Zendesk.2 and .3): objects, full and
incremental reads, deletes, the email + API token sign-in, through the real
`SaasSession` over a FAKE Zendesk (the incremental exports and the paged lists as
Zendesk documents them, from `test_saas_vendored_zendesk.FakeZendesk`). No network.
The loader test runs `load_via_sink` for real on `test_load_modes_measurement.Lab`;
CI runs it in `dagster-loader-tests`."""

from __future__ import annotations

import base64
import json
import time
from typing import Any
from urllib.parse import parse_qs, urlparse

import pytest

from dispar_orchestrate import ingest_errors, schema_observer
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendored.zendesk import settings as zd_settings
from dispar_orchestrate.saas.vendors import zendesk
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_zendesk import ORIGIN, FakeZendesk, Reply, iso, resolver, ticket

DIAL = {"vendor": "zendesk", "account": "acme", "auth": {"type": "integration"}}
B = 1_700_000_000  # a plausible epoch: a first run starts at 2000-01-01
SECRETS = {"email": "agent@example.invalid", "apiToken": "not-a-real-token"}


class Zendesk(FakeZendesk):
    """The fake of the vendored tests plus the cursor-based users export, a cursor-paged
    list (groups), the offset-paged ticket fields, `users/me` and failure switches."""

    def __init__(self, *args: Any, **kw: Any) -> None:
        super().__init__(*args, **kw)
        self.users: list[dict[str, Any]] = []
        self.groups: list[dict[str, Any]] = []
        self.fail: dict[str, int] = {}

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        path = urlparse(url).path
        if path in self.fail:
            self.headers.append(headers)
            return Reply(self.fail[path], {"error": "STUB-VENDOR-WORDS"})
        if path == "/api/v2/users/me.json":
            self.headers.append(headers)
            return Reply(body={"user": {"id": 7, "email": "agent@example.invalid"}})
        if path == "/api/v2/incremental/users/cursor.json":
            self.headers.append(headers)
            query = {**{k: v[0] for k, v in parse_qs(urlparse(url).query).items()}, **{k: str(v) for k, v in (params or {}).items()}}
            self.log.append(f"{path}?start_time={query.get('start_time')}&cursor={query.get('cursor')}")
            order = sorted(self.users, key=lambda u: (u["generated_timestamp"], u["id"]))
            position = int(query["cursor"]) if "cursor" in query else sum(1 for u in order if u["generated_timestamp"] < int(query["start_time"]))
            page = order[position : position + self.page_size]
            after = position + len(page)
            return Reply(body={"users": page, "after_cursor": str(after), "after_url": f"{ORIGIN}{path}?cursor={after}", "end_of_stream": after >= len(order)})
        if path == "/api/v2/groups.json":
            self.headers.append(headers)
            query = {**{k: v[0] for k, v in parse_qs(urlparse(url).query).items()}, **{k: str(v) for k, v in (params or {}).items()}}
            self.log.append(f"{path}?{'&'.join(sorted(query))}")
            start = int(query.get("after", 0))
            size = self.page_size
            page = self.groups[start : start + size]
            more = start + size < len(self.groups)
            return Reply(body={"groups": page, "meta": {"has_more": more}, "links": {"next": f"{ORIGIN}{path}?page%5Bsize%5D={size}&after={start + size}"}})
        if path == "/api/v2/ticket_fields.json":
            self.headers.append(headers)
            return Reply(body={"ticket_fields": [{"id": 1, "title": "Subject"}], "next_page": None})
        return super().get(url, headers=headers, params=params, timeout=timeout)


@pytest.fixture
def api() -> Zendesk:
    return Zendesk([], page_size=3)


def open_(api: Zendesk, dial: dict = DIAL, secrets: dict = SECRETS):  # noqa: ANN202
    return saas_adapter.open_session(dial, secrets, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)


def read(api: Zendesk, name: str, since: Any = None) -> list[dict]:
    module, _vendor, parts, session = open_(api)
    return list(module.read(session, parts, DIAL, module.declaration(session, parts, DIAL, name), since=since))


def test_the_vendor_module_is_found_by_the_registry() -> None:
    assert vendors.module_for("zendesk") is zendesk


def test_five_objects_are_incremental_the_rest_say_why_not_and_only_tickets_report_deletes() -> None:
    declared = {d.name: d for d in zendesk.list_objects(None, None, DIAL)}
    incremental = {n for n, d in declared.items() if d.incremental}
    assert incremental == {"tickets", "users", "organizations", "ticket_events", "ticket_metric_events"}
    assert {n for n, d in declared.items() if d.deletes} == {"tickets"}, "decision 7: only the deleted-status export"
    assert {"groups", "brands", "sla_policies", "macros", "views", "ticket_fields", "tags"} <= set(declared)
    assert declared["groups"].load_description() == "re-read in full: Zendesk has no incremental export for groups"
    assert declared["tags"].primary_key == ("name",) and declared["tickets"].primary_key == ("id",)
    assert not any(n.startswith(("calls", "chats", "phone", "greeting")) for n in declared), "Talk and Chat are not offered"


def test_an_unknown_object_is_a_rejected_setting_and_never_echoed() -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused) as raised:
        zendesk.declaration(None, None, DIAL, "../../etc/passwd")
    assert "passwd" not in str(raised.value)


def test_a_first_run_reads_every_ticket_once_even_at_a_page_boundary(api: Zendesk) -> None:
    api.tickets = [ticket(1, B + 100), ticket(2, B + 200), ticket(3, B + 300), ticket(4, B + 300), ticket(5, B + 400), ticket(6, B + 500)]
    records = read(api, "tickets")
    assert [r["id"] for r in records] == [1, 2, 3, 4, 5, 6], "the cursor-based export: nothing twice (upstream #659)"
    assert all("/cursor.json" in line for line in api.log)
    first = records[0]
    assert first["tags"] == "[]" and first["custom_fields"] == "[]", "tags and custom fields are one JSON text each"
    assert first["updated_at"].isoformat().startswith("2023-11-14T22:15:"), "dates are datetimes, as the verified source cleans them"


def test_a_later_run_starts_the_export_at_the_cursor_and_gets_the_changed_and_the_new(api: Zendesk) -> None:
    api.tickets = [ticket(i, B + 1_000 * i) for i in range(1, 6)]
    first = read(api, "tickets")
    cursor = max(r["updated_at"] for r in first)
    api.tickets[1] = ticket(2, B + 9_000, subject="changed")
    api.tickets.append(ticket(6, B + 9_500))
    api.log.clear()
    again = read(api, "tickets", since=cursor)
    assert [r["id"] for r in again] == [5, 2, 6], "the ticket at the cursor (merge removes it), the changed, the new"
    assert f"start_time={B + 5000}" in api.log[0], "the export starts at the cursor's time"


def test_a_deleted_ticket_arrives_in_the_same_read_with_its_status(api: Zendesk) -> None:
    api.tickets = [ticket(1, B + 100), ticket(2, B + 200)]
    api.tickets[1] = ticket(2, B + 300, status="deleted")
    got = {r["id"]: r["status"] for r in read(api, "tickets")}
    assert got == {1: "open", 2: "deleted"}
    assert zendesk.declaration(None, None, DIAL, "tickets").deletes is True


def test_users_are_read_through_their_cursor_export_and_events_through_the_deduplicated_time_based_one(api: Zendesk) -> None:
    api.users = [{"id": i, "name": f"u{i}", "updated_at": iso(B + 100 * i), "generated_timestamp": B + 100 * i} for i in range(1, 5)]
    assert [u["id"] for u in read(api, "users")] == [1, 2, 3, 4]
    api.others["ticket_events"] = [
        {"id": i, "created_at": iso(B + ts), "timestamp": B + ts, "generated_timestamp": B + ts} for i, ts in ((1, 100), (2, 200), (3, 300), (4, 300), (5, 400), (6, 500))
    ]
    assert [e["id"] for e in read(api, "ticket_events")] == [1, 2, 3, 4, 5, 6]
    assert [e["id"] for e in read(api, "ticket_events", since=B + 400)] == [5, 6], "an integer cursor starts the export at that second"


def test_a_cursor_paged_list_is_read_in_full(api: Zendesk) -> None:
    api.groups = [{"id": i, "name": f"g{i}"} for i in range(1, 8)]
    assert [g["id"] for g in read(api, "groups")] == list(range(1, 8))


def test_the_default_start_is_the_verified_sources_and_cursor_values_of_each_kind_are_understood() -> None:
    assert zendesk._start_time(None) == zd_settings.DEFAULT_START_DATE.int_timestamp
    assert zendesk._start_time(250) == 250 and zendesk._start_time("1970-01-01T00:04:10Z") == 250
    from datetime import datetime, timezone

    assert zendesk._start_time(datetime(1970, 1, 1, 0, 4, 10, tzinfo=timezone.utc)) == 250


# ---- V-Zendesk.3: sign-in with an email and an API token ----------------------------------------


def test_the_email_and_api_token_are_sent_as_zendesks_basic_credential_and_reach_the_account(api: Zendesk) -> None:
    _module, vendor, _parts, session = open_(api)
    assert session.get_json(vendor.whoami["path"])["user"]["id"] == 7
    expected = base64.b64encode(b"agent@example.invalid/token:not-a-real-token").decode()
    assert api.headers == [{"Authorization": f"Basic {expected}"}], "<email>/token as the user name, the token as the password"


@pytest.mark.parametrize("status,error,klass", [(401, "SaasSignInRefused", "sign_in_refused"), (403, "SaasPermissionMissing", "permission_missing")])
def test_a_refusal_is_a_fixed_failure_without_zendesks_words(api: Zendesk, status: int, error: str, klass: str) -> None:
    api.fail["/api/v2/groups.json"] = status
    with pytest.raises(Exception) as raised:
        read(api, "groups")
    assert type(raised.value).__name__ == error and "STUB-VENDOR-WORDS" not in str(raised.value)
    assert ingest_errors.classify(raised.value).error_class == klass


def test_a_429_with_retry_after_is_waited_out(api: Zendesk) -> None:
    api.groups = [{"id": 1, "name": "g"}]
    original, slept = api.get, []
    state = {"left": 1}

    def limited(url: str, **kw: Any) -> Reply:
        if state["left"]:
            state["left"] -= 1
            return Reply(429, {"error": "STUB-VENDOR-WORDS"}, {"Retry-After": "7"})
        return original(url, **kw)

    api.get = limited  # type: ignore[method-assign]
    module, _v, parts, session = saas_adapter.open_session(DIAL, SECRETS, resolve_checked=resolver, http_get=api.get, sleep=slept.append)
    assert [g["id"] for g in module.read(session, parts, DIAL, module.declaration(session, parts, DIAL, "groups"), since=None)] == [1]
    assert slept == [7.0]


def test_the_vendor_row_is_what_zendesks_documentation_says() -> None:
    """V-Zendesk.3, review item 5: each fact below was read in Zendesk's documentation on
    2026-10-19 (see the module doc of `saas/vendors/zendesk.py` for the pages)."""
    row = vendors.vendor("zendesk")
    assert row.oauth["authorizeUrl"] == "https://{account}.zendesk.com/oauth/authorizations/new"
    assert row.oauth["tokenUrl"] == "https://{account}.zendesk.com/oauth/tokens"
    assert row.oauth["scopes"] == ["read"], "read: all GET endpoints, the incremental exports included"
    assert row.oauth["clientAuth"] == "post_json", "POST /oauth/tokens must be formatted as JSON"
    assert row.oauth["revocationUrl"] is None, "Zendesk revokes with DELETE /api/v2/oauth/tokens/{id}, not RFC 7009"
    assert row.whoami == {"method": "GET", "path": "/api/v2/users/me.json"}
    assert row.integration == {"secretKey": "integration:zendesk", "secretFields": ["email", "apiToken"]}
    assert row.api_url(row.validate_account("Acme")) == "https://acme.zendesk.com", "a sub-domain, never a host a person types"


def test_the_browser_sign_in_sends_zendesks_token_request_as_json(api: Zendesk, monkeypatch: pytest.MonkeyPatch) -> None:
    sent: list[dict] = []

    def token(url: str, **kw: Any) -> Reply:
        sent.append({"url": url, **kw})
        return Reply(200, {"access_token": "a", "refresh_token": "r2", "expires_in": 1800})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", token)
    dial = {"vendor": "zendesk", "account": "acme", "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}
    module, _v, parts, session = saas_adapter.open_session(
        dial, {"clientSecret": "s", "refreshToken": "r"}, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None
    )
    api.groups = [{"id": 1}]
    list(module.read(session, parts, dial, module.declaration(session, parts, dial, "groups"), since=None))
    call = sent[0]
    assert call["url"] == "https://acme.zendesk.com/oauth/tokens" and "data" not in call
    assert call["json"] == {
        "grant_type": "refresh_token", "refresh_token": "r", "client_id": "app", "client_secret": "s",
    }, "client_secret_post_json: the client is in the JSON body (slice 2 review BLOCKER 3)"  # fmt: skip
    assert {h["Authorization"] for h in api.headers} == {"Bearer a"}


def test_a_revoked_browser_sign_in_is_the_fixed_failure(api: Zendesk, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: Reply(400, {"error": "invalid_grant STUB-VENDOR-WORDS"}))
    dial = {"vendor": "zendesk", "account": "acme", "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}
    module, _v, parts, session = saas_adapter.open_session(
        dial, {"clientSecret": "s", "refreshToken": "r"}, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None
    )
    with pytest.raises(Exception) as raised:
        list(module.read(session, parts, dial, module.declaration(session, parts, dial, "groups"), since=None))
    assert ingest_errors.classify(raised.value).error_class == "sign_in_refused" and "STUB" not in str(raised.value)
    assert api.headers == []


# ---- the whole path: adapter, dlt incremental, merge on the key, real Iceberg -------------------


def accept_all(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj() -> dict:
    return {"name": "tickets", "target": "zd_tickets", "loadMode": "incremental_dedup", "incrementalKey": "updated_at", "primaryKey": ["id"]}


@pytest.mark.loader
def test_a_second_run_merges_the_changed_ticket_and_never_stores_a_boundary_ticket_twice(lab: Lab, api: Zendesk) -> None:
    now = int(time.time()) - 86_400
    api.tickets = [ticket(1, now + 10), ticket(2, now + 20), ticket(3, now + 30), ticket(4, now + 30), ticket(5, now + 40), ticket(6, now + 50)]

    def run() -> Any:
        built = saas_adapter.build_source(DIAL, SECRETS, [obj()], gate=accept_all, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)
        assert built is not None
        return load_via_sink(built.source, "zd_tickets", lab.config, f._load_plan("saas", obj()))

    assert run().rows == 6
    assert sorted(r["id"] for r in lab.rows("zd_tickets")) == [1, 2, 3, 4, 5, 6], "six tickets, none twice"
    time.sleep(1.1)
    api.tickets[1] = ticket(2, now + 60, subject="changed", status="deleted")
    api.tickets.append(ticket(7, now + 70))
    run()
    rows = {r["id"]: (r["subject"], r["status"]) for r in lab.rows("zd_tickets")}
    assert rows[2] == ("changed", "deleted") and 7 in rows and len(lab.rows("zd_tickets")) == 7, "merged on id: no duplicate"
    assert json.loads(lab.rows("zd_tickets")[0]["tags"]) == []
