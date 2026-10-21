"""`saas/vendors/servicenow.py` (`SRC-3` slice 4, V-ServiceNow.1 to .3): reading the Table
API through the real `SaasSession` over a FAKE ServiceNow (the encoded query, the `Link`
header paging, the access check that runs after the page limit). No network. The loader test
runs `load_via_sink` for real on `test_load_modes_measurement.Lab`; CI runs it in
`dagster-loader-tests`."""

from __future__ import annotations

import re
from typing import Any
from urllib.parse import parse_qs, urlencode, urlparse

import pytest

from dispar_orchestrate import schema_observer
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import session as saas_session
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import servicenow
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_zendesk import Reply, resolver

ORIGIN = "https://acme.service-now.com"
DIAL = {"vendor": "servicenow", "account": "acme", "auth": {"type": "integration"}}
SECRETS = {"clientId": "client", "clientSecret": "not-a-real-secret"}


def row(i: int, updated: str, **extra: Any) -> dict[str, Any]:
    return {"sys_id": f"sn{i:030d}", "number": f"INC{i:07d}", "short_description": f"incident {i}", "sys_updated_on": updated, **extra}


class ServiceNow:
    """The Table API as documented: `sysparm_query` with `field>=value^ORDERBY...`, a page
    limit applied BEFORE the access check, the `Link` header with `rel="next"`."""

    def __init__(self, page: int = 3) -> None:
        self.tables: dict[str, list[dict[str, Any]]] = {"incident": []}
        self.page = page
        self.hidden: set[str] = set()  # sys_ids the account may not see (dropped after the limit)
        self.log: list[str] = []
        self.link_to: str | None = None  # make the next link point elsewhere
        self.status: dict[str, int] = {}

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        parsed = urlparse(url)
        query = {k: v[0] for k, v in parse_qs(parsed.query).items()} | {k: str(v) for k, v in (params or {}).items()}
        table = parsed.path.rsplit("/", 1)[-1]
        self.log.append(f"{table}?{query.get('sysparm_query')}&offset={query.get('sysparm_offset', 0)}")
        if table in self.status:
            return Reply(self.status[table], {"error": {"message": "STUB-VENDOR-WORDS"}})
        if table not in self.tables:
            return Reply(400, {"error": {"message": "Invalid table STUB-VENDOR-WORDS"}})
        rows = list(self.tables[table])
        match = re.match(r"sys_updated_on>=([^\^]+)\^", query.get("sysparm_query", ""))
        if match:
            rows = [r for r in rows if r["sys_updated_on"] >= match.group(1)]
        rows.sort(key=lambda r: (r["sys_updated_on"], r["sys_id"]))
        offset, limit = int(query.get("sysparm_offset", 0)), int(query["sysparm_limit"]) if "sysparm_limit" in query else self.page
        limit = min(limit, self.page)
        chunk = rows[offset : offset + limit]
        visible = [r for r in chunk if r["sys_id"] not in self.hidden]
        reply_headers = {}
        if offset + limit < len(rows):
            nxt = self.link_to or ORIGIN
            more = urlencode({**query, "sysparm_limit": limit, "sysparm_offset": offset + limit})
            reply_headers["Link"] = f'<{nxt}{parsed.path}?{more}>;rel="next",<{ORIGIN}{parsed.path}?x=1>;rel="last"'
        return Reply(200, {"result": visible}, reply_headers)


TOKEN_POSTS: list[dict] = []


@pytest.fixture(autouse=True)
def token_endpoint(monkeypatch: pytest.MonkeyPatch) -> None:
    """ServiceNow's `oauth_token.do`: the client-credentials grant, form in, a bearer token out."""
    TOKEN_POSTS.clear()

    def post(url: str, **kw: Any) -> Reply:
        TOKEN_POSTS.append({"url": url, "data": kw.get("data"), "json": kw.get("json"), "headers": kw.get("headers")})
        if (kw.get("data") or {}).get("client_secret") != SECRETS["clientSecret"]:
            return Reply(401, {"error": "access_denied", "error_description": "STUB-VENDOR-WORDS"})
        return Reply(200, {"access_token": "snow-access", "token_type": "Bearer", "expires_in": 1800})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)


@pytest.fixture
def api() -> ServiceNow:
    return ServiceNow()


def open_(api: ServiceNow, dial: dict = DIAL, secrets: dict = SECRETS):  # noqa: ANN202
    return saas_adapter.open_session(dial, secrets, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)


def read(api: ServiceNow, name: str = "incident", since: Any = None) -> list[dict]:
    module, _vendor, parts, session = open_(api)
    return list(module.read(session, parts, DIAL, module.declaration(session, parts, DIAL, name), since=since))


def test_the_vendor_module_is_found_by_the_registry() -> None:
    assert vendors.module_for("servicenow") is servicenow


def test_client_credentials_ask_the_instances_token_address_with_the_client_in_the_form_body(api: ServiceNow) -> None:
    api.tables["incident"] = [row(1, "2026-10-01 10:00:00")]
    seen: list[dict] = []
    original = api.get
    api.get = lambda url, *, headers, params, timeout: (seen.append(headers), original(url, headers=headers, params=params, timeout=timeout))[1]  # type: ignore[method-assign]
    read(api)
    assert TOKEN_POSTS[0]["url"] == "https://acme.service-now.com/oauth_token.do"
    assert TOKEN_POSTS[0]["data"] == {"grant_type": "client_credentials", "client_id": "client", "client_secret": "not-a-real-secret"}
    assert seen[0] == {"Authorization": "Bearer snow-access"}


def test_a_refused_exchange_is_the_fixed_sign_in_failure(api: ServiceNow) -> None:
    from dispar_orchestrate.adapters import rest as rest_adapter

    module, _v, parts, session = open_(api, secrets={**SECRETS, "clientSecret": "wrong"})
    with pytest.raises(rest_adapter.OAuthSignInRefused) as err:
        list(module.read(session, parts, DIAL, module.declaration(session, parts, DIAL, "incident"), since=None))
    assert "STUB-VENDOR-WORDS" not in str(err.value) and api.log == []


def test_a_first_run_reads_every_record_in_order_across_the_link_header_pages(api: ServiceNow) -> None:
    api.tables["incident"] = [row(i, f"2026-10-0{i} 10:00:00") for i in range(1, 8)]
    got = read(api)
    assert [r["number"] for r in got] == [f"INC{i:07d}" for i in range(1, 8)]
    assert api.log[0].startswith("incident?ORDERBYsys_updated_on^ORDERBYsys_id")


def test_a_short_page_does_not_end_the_read_because_the_access_check_runs_after_the_limit(api: ServiceNow) -> None:
    api.tables["incident"] = [row(i, f"2026-10-0{i} 10:00:00") for i in range(1, 8)]
    api.hidden = {row(i, "")["sys_id"] for i in (4, 5, 6)}  # a whole page the account may not see
    assert [r["number"] for r in read(api)] == ["INC0000001", "INC0000002", "INC0000003", "INC0000007"]


def test_a_later_run_asks_only_for_records_updated_since_the_cursor(api: ServiceNow) -> None:
    api.tables["incident"] = [row(i, f"2026-10-0{i} 10:00:00") for i in range(1, 6)]
    got = read(api, since="2026-10-03 10:00:00")
    assert [r["number"] for r in got] == ["INC0000003", "INC0000004", "INC0000005"]
    assert api.log[0].startswith("incident?sys_updated_on>=2026-10-03 10:00:00^ORDERBYsys_updated_on^ORDERBYsys_id")


def test_the_cursor_may_arrive_as_a_datetime_or_iso_text_and_is_written_in_utc(api: ServiceNow) -> None:
    from datetime import datetime, timedelta, timezone

    api.tables["incident"] = [row(1, "2026-10-01 10:00:00")]
    read(api, since=datetime(2026, 10, 1, 12, 0, 0, tzinfo=timezone(timedelta(hours=2))))
    read(api, since="2026-10-01T10:00:00Z")
    assert all(line.startswith("incident?sys_updated_on>=2026-10-01 10:00:00^") for line in api.log)


def test_a_cursor_that_is_not_a_date_is_a_rejected_setting(api: ServiceNow) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(api, since="1; drop")


@pytest.mark.parametrize("name", ["", "Incident", "a b", "x;y", "../sys_user", "incident?x=1", "1abc", "a" * 81])
def test_a_table_name_that_is_not_plain_is_refused_before_any_request(api: ServiceNow, name: str) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(api, name)
    assert api.log == []


def test_an_unknown_table_is_a_rejected_setting_that_does_not_repeat_the_vendor(api: ServiceNow) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused) as err:
        read(api, "nothing_here")
    assert "STUB-VENDOR-WORDS" not in str(err.value)


def test_a_link_to_another_host_stops_the_run(api: ServiceNow) -> None:
    from dispar_orchestrate import ssrf_guard

    api.tables["incident"] = [row(i, f"2026-10-0{i} 10:00:00") for i in range(1, 8)]
    api.link_to = "https://evil.example.invalid"
    with pytest.raises(ssrf_guard.SsrfBlocked) as err:
        read(api)
    assert "evil" not in str(err.value)


def test_the_access_failures_have_their_own_fixed_classes(api: ServiceNow) -> None:
    api.tables["incident"] = [row(1, "2026-10-01 10:00:00")]
    for status, expected in ((401, saas_session.SaasSignInRefused), (403, saas_session.SaasPermissionMissing)):
        api.status["incident"] = status
        with pytest.raises(expected) as err:
            read(api)
        assert "STUB-VENDOR-WORDS" not in str(err.value)


def accept_all(table: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj(**extra: Any) -> dict[str, Any]:
    return {
        "name": "incident", "target": "snow_incident", "loadMode": "incremental_dedup", "incrementalKey": "sys_updated_on",
        "primaryKey": ["sys_id"], **extra,
    }


@pytest.mark.loader
def test_a_second_run_merges_the_changed_incident_and_stores_none_twice(lab: Lab, api: ServiceNow) -> None:
    api.tables["incident"] = [row(i, f"2026-10-0{i} 10:00:00") for i in range(1, 6)]

    def run() -> Any:
        built = saas_adapter.build_source(DIAL, SECRETS, [obj()], gate=accept_all, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)
        assert built is not None
        return load_via_sink(built.source, "snow_incident", lab.config, f._load_plan("saas", obj()))

    assert run().rows == 5
    api.tables["incident"][1] = row(2, "2026-10-09 10:00:00", short_description="changed")
    api.tables["incident"].append(row(6, "2026-10-09 11:00:00"))
    run()
    rows = {r["number"]: r["short_description"] for r in lab.rows("snow_incident")}
    assert len(lab.rows("snow_incident")) == 6 and rows["INC0000002"] == "changed", "merged on sys_id: no duplicate"
