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
        # the dictionary: table -> (fields it defines, its parent)
        self.dictionary: dict[str, tuple[list[str], str | None]] = {
            "incident": (["number", "short_description"], "task"),
            "task": (["sys_id", "sys_updated_on", "state"], None),
        }

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        parsed = urlparse(url)
        query = {k: v[0] for k, v in parse_qs(parsed.query).items()} | {k: str(v) for k, v in (params or {}).items()}
        table = parsed.path.rsplit("/", 1)[-1]
        self.log.append(f"{table}?{query.get('sysparm_query')}&offset={query.get('sysparm_offset', 0)}")
        if table in self.status:
            return Reply(self.status[table], {"error": {"message": "STUB-VENDOR-WORDS"}})
        if table == "sys_dictionary":
            name = re.match(r"name=([a-z_]+)\^", query["sysparm_query"]).group(1)  # type: ignore[union-attr]
            return Reply(200, {"result": [{"element": e} for e in self.dictionary.get(name, ([], None))[0]] + [{"element": ""}]})
        if table == "sys_db_object":
            name = query["sysparm_query"].split("=", 1)[1]
            parent = self.dictionary.get(name, ([], None))[1]
            return Reply(200, {"result": [{"super_class.name": parent or ""}]})
        if table not in self.tables:
            return Reply(400, {"error": {"message": "Invalid table STUB-VENDOR-WORDS"}})
        rows = list(self.tables[table])
        match = re.match(r"sys_updated_on>=([^\^]+)\^", query.get("sysparm_query", ""))
        if match:
            rows = [r for r in rows if r.get("sys_updated_on", "") >= match.group(1)]
        rows.sort(key=lambda r: (r.get("sys_updated_on", ""), r.get("sys_id", "")))
        offset, limit = int(query.get("sysparm_offset", 0)), int(query["sysparm_limit"]) if "sysparm_limit" in query else self.page
        limit = min(limit, self.page)
        chunk = rows[offset : offset + limit]
        visible = [r for r in chunk if r.get("sys_id") not in self.hidden]
        if "sysparm_fields" in query:
            wanted = query["sysparm_fields"].split(",")
            visible = [{k: v for k, v in r.items() if k in wanted} for r in visible]
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


def reads(api: ServiceNow) -> list[str]:
    """The log without the one-record sample `declaration` asks for."""
    return [line for line in api.log if not line.startswith(tuple(f"{t}?None" for t in api.tables))]


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
    assert reads(api)[0].startswith("incident?ORDERBYsys_updated_on^ORDERBYsys_id")


def test_a_short_page_does_not_end_the_read_because_the_access_check_runs_after_the_limit(api: ServiceNow) -> None:
    api.tables["incident"] = [row(i, f"2026-10-0{i} 10:00:00") for i in range(1, 8)]
    api.hidden = {row(i, "")["sys_id"] for i in (4, 5, 6)}  # a whole page the account may not see
    assert [r["number"] for r in read(api)] == ["INC0000001", "INC0000002", "INC0000003", "INC0000007"]


def test_a_later_run_asks_only_for_records_updated_since_the_cursor(api: ServiceNow) -> None:
    api.tables["incident"] = [row(i, f"2026-10-0{i} 10:00:00") for i in range(1, 6)]
    got = read(api, since="2026-10-03 10:00:00")
    assert [r["number"] for r in got] == ["INC0000003", "INC0000004", "INC0000005"]
    assert reads(api)[0].startswith("incident?sys_updated_on>=2026-10-03 10:00:00^ORDERBYsys_updated_on^ORDERBYsys_id")


def test_the_cursor_may_arrive_as_a_datetime_or_iso_text_and_is_written_in_utc(api: ServiceNow) -> None:
    from datetime import datetime, timedelta, timezone

    api.tables["incident"] = [row(1, "2026-10-01 10:00:00")]
    read(api, since=datetime(2026, 10, 1, 12, 0, 0, tzinfo=timezone(timedelta(hours=2))))
    read(api, since="2026-10-01T10:00:00Z")
    assert reads(api) and all(line.startswith("incident?sys_updated_on>=2026-10-01 10:00:00^") for line in reads(api))


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


# ---- objects: declaration, list, describe ------------------------------------------------------


def test_a_table_with_a_change_time_loads_incrementally_and_one_without_is_read_in_full(api: ServiceNow) -> None:
    api.tables["incident"] = [row(1, "2026-10-01 10:00:00")]
    api.tables["sys_choice"] = [{"sys_id": "x1", "label": "a"}]
    module, _v, parts, session = open_(api)
    assert module.declaration(session, parts, DIAL, "incident").load_description() == "incremental"
    full = module.declaration(session, parts, DIAL, "sys_choice")
    assert full.load_description() == "re-read in full: sys_choice has no sys_updated_on field to filter by"
    assert [r["sys_id"] for r in module.read(session, parts, DIAL, full, since=None)] == ["x1"]
    assert reads(api)[-1].startswith("sys_choice?ORDERBYsys_id&")


def test_a_table_without_sys_id_is_a_rejected_setting_and_an_unknown_one_too(api: ServiceNow) -> None:
    api.tables["v_odd"] = [{"name": "x"}]
    module, _v, parts, session = open_(api)
    for name in ("v_odd", "nothing_here"):
        with pytest.raises(saas_adapter.SaasSetupRefused):
            module.declaration(session, parts, DIAL, name)


def test_an_empty_table_is_taken_to_be_a_normal_incremental_one(api: ServiceNow) -> None:
    module, _v, parts, session = open_(api)
    assert module.declaration(session, parts, DIAL, "incident").incremental


def test_the_five_suggested_tables_and_the_ones_named_in_the_dial_are_listed(api: ServiceNow) -> None:
    module, _v, parts, session = open_(api)
    names = [d.name for d in module.list_objects(session, parts, {**DIAL, "objects": ["u_custom", "incident"]})]
    assert names == ["incident", "change_request", "problem", "sys_user", "cmdb_ci", "u_custom"]
    assert api.log == [], "listing makes no request"
    with pytest.raises(saas_adapter.SaasSetupRefused):
        module.list_objects(session, parts, {**DIAL, "objects": ["Bad Name"]})


def test_describe_walks_the_parent_tables_and_types_the_change_time(api: ServiceNow) -> None:
    module, _v, parts, session = open_(api)
    fields = {f.name: f.type_name for f in module.describe(session, parts, DIAL, "incident")}
    assert fields == {"number": "text", "short_description": "text", "sys_id": "text", "sys_updated_on": "timestamp", "state": "text"}


def test_describe_is_none_when_the_account_may_not_read_the_dictionary(api: ServiceNow) -> None:
    api.status["sys_dictionary"] = 403
    module, _v, parts, session = open_(api)
    assert module.describe(session, parts, DIAL, "incident") is None


def test_a_loop_in_the_table_hierarchy_is_refused(api: ServiceNow) -> None:
    api.dictionary = {"a": (["x"], "b"), "b": (["y"], "a")}
    module, _v, parts, session = open_(api)
    with pytest.raises(saas_adapter.SaasSetupRefused):
        module.describe(session, parts, DIAL, "a")


def test_a_described_load_is_observed_before_it_and_keeps_only_the_described_columns(api: ServiceNow) -> None:
    api.tables["incident"] = [row(1, "2026-10-01 10:00:00", secret_note="x")]
    seen: list[list[str]] = []

    def gate(table: schema_observer.ReflectedTable) -> schema_observer.Decision:
        seen.append([c.name for c in table.columns])
        return schema_observer.Decision(action="load", columns=[c.name for c in table.columns], changes=[])

    built = saas_adapter.build_source(DIAL, SECRETS, [obj()], gate=gate, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)
    assert built is not None and built.observed_before_load and "sys_id" in seen[0]
    assert all("secret_note" not in r for r in built.source)


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


# ---- sign-in: the vendor table's row (checked against ServiceNow's documentation, 2026-10-21) -----


def test_the_vendor_row_names_the_instances_oauth_addresses_and_no_rfc_7009_revocation() -> None:
    row_ = vendors.vendor("servicenow")
    parts = row_.validate_account("acme")
    assert row_.render(row_.oauth["authorizeUrl"], parts) == "https://acme.service-now.com/oauth_auth.do"
    assert row_.token_url(parts) == "https://acme.service-now.com/oauth_token.do"
    # ServiceNow revokes by GET `oauth_revoke_token.do?token=...`, which the product's RFC 7009 call is not.
    assert row_.oauth["revocationUrl"] is None and row_.oauth["clientAuth"] == "post"
    assert row_.integration == {"secretKey": "integration:servicenow", "secretFields": ["clientId", "clientSecret"]}
    assert row_.whoami["path"].startswith("/api/now/table/sys_user?")


def test_the_browser_sign_in_refreshes_at_the_instances_token_address(api: ServiceNow) -> None:
    api.tables["incident"] = [row(1, "2026-10-01 10:00:00")]
    dial = {"vendor": "servicenow", "account": "acme", "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}
    secrets = {"clientSecret": SECRETS["clientSecret"], "refreshToken": "refresh-1"}
    TOKEN_POSTS.clear()
    module, _v, parts, session = saas_adapter.open_session(dial, secrets, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)
    assert [r["number"] for r in module.read(session, parts, dial, module.declaration(session, parts, dial, "incident"), since=None)] == ["INC0000001"]
    assert TOKEN_POSTS[0]["url"] == "https://acme.service-now.com/oauth_token.do"
    assert TOKEN_POSTS[0]["data"]["grant_type"] == "refresh_token" and TOKEN_POSTS[0]["data"]["client_id"] == "app"
