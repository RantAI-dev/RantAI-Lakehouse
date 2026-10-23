"""`saas/vendors/dynamics365.py` (`SRC-3` slice 5, V-Dynamics.1 to .3): tables from `EntityDefinitions`, the schema
described before the load, `modifiedon` filtering, Dataverse change tracking with its delta link in dlt state (deletes
marked), the fallback for a table without tracking, a stale link as a full resync, and both Entra sign-ins (the
customer's tenant in the token address), through the real `SaasSession` over a FAKE Dataverse and a FAKE Entra token
endpoint. No network. The loader tests run `load_via_sink` for real on `test_load_modes_measurement.Lab`; CI runs them in
`dagster-loader-tests`."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Any
from urllib.parse import parse_qs, quote, urlparse

import pytest
import requests

from dispar_orchestrate import ingest_errors, schema_observer, ssrf_guard
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import session as saas_session
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import dynamics365
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_zendesk import Reply, resolver

HOST = "acme.crm4.dynamics.com"
ORIGIN = f"https://{HOST}"
TENANT = "contoso.example.invalid"
ACCOUNT = f"{HOST}/{TENANT}"
DIAL = {"vendor": "dynamics365", "account": ACCOUNT, "auth": {"type": "integration"}}
OAUTH_DIAL = {"vendor": "dynamics365", "account": ACCOUNT, "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}
SECRETS = {"clientId": "app-id", "clientSecret": "app-secret"}
OAUTH_SECRETS = {"clientSecret": "client-secret", "refreshToken": "refresh-1"}
BASE = datetime(2026, 10, 1, 10, 0, 0, tzinfo=timezone.utc)
SEEN = datetime(2026, 10, 20, 12, 0, 0, tzinfo=timezone.utc)


def urlencode(query: Any) -> str:
    """A query string as Dataverse writes it: `$` and `,` stay as they are."""
    pairs = query.items() if hasattr(query, "items") else query
    return "&".join(f"{quote(str(k), safe='$,')}={quote(str(v), safe='$,')}" for k, v in pairs)


def stamp(minutes: float) -> str:
    return (BASE + timedelta(minutes=minutes)).strftime("%Y-%m-%dT%H:%M:%SZ")


def attrs(*columns: tuple[str, str], extra: list[dict] | None = None) -> list[dict]:
    return [{"LogicalName": n, "AttributeType": t, "AttributeOf": None, "IsValidForRead": True} for n, t in columns] + (extra or [])


class Table:
    def __init__(self, logical: str, entity_set: str, key: str, attributes: list[dict], tracked: bool = False) -> None:
        self.logical, self.entity_set, self.key, self.attributes, self.tracked = logical, entity_set, key, attributes, tracked
        self.rows: dict[str, dict[str, Any]] = {}
        self.version = 0
        self.vers: dict[str, int] = {}
        self.deleted: set[str] = set()

    def put(self, row_id: int, minutes: float, **columns: Any) -> None:
        self.version += 1
        rid = f"00000000-0000-0000-0000-{row_id:012d}"
        self.deleted.discard(rid)
        self.vers[rid] = self.version
        self.rows[rid] = {self.key: rid, "@odata.etag": f'W/"{self.version}"', "modifiedon": stamp(minutes), "name": f"row {row_id}", **columns}

    def delete(self, row_id: int) -> None:
        self.version += 1
        rid = f"00000000-0000-0000-0000-{row_id:012d}"
        self.vers[rid] = self.version
        self.deleted.add(rid)

    def definition(self) -> dict[str, Any]:
        return {"LogicalName": self.logical, "EntitySetName": self.entity_set, "PrimaryIdAttribute": self.key, "ChangeTrackingEnabled": self.tracked, "Attributes": self.attributes}


class Dataverse:
    """The pieces of the Dataverse Web API the connector uses, as documented: `EntityDefinitions` (not paged), entity sets
    paged by `Prefer: odata.maxpagesize` and `@odata.nextLink`, `$filter`/`$orderby`, and change tracking
    (`Prefer: odata.track-changes` refuses `$filter`/`$orderby`; a delta link carries `$deltatoken`; a deleted row is
    `{"@odata.context": ".../$deletedEntity", "id", "reason"}`)."""

    def __init__(self, cap: int = 2) -> None:
        self.cap = cap
        common = (("name", "String"), ("modifiedon", "DateTime"), ("createdon", "DateTime"))
        self.tables = {
            "account": Table(
                "account", "accounts", "accountid",
                attrs(("accountid", "Uniqueidentifier"), *common, ("revenue", "Money"), ("ownerid", "Owner"), ("donotemail", "Boolean"), ("entityimage", "Image"), ("employees", "Integer"),
                      extra=[{"LogicalName": "owneridname", "AttributeType": "String", "AttributeOf": "ownerid", "IsValidForRead": True}]),
            ),
            "contact": Table("contact", "contacts", "contactid", attrs(("contactid", "Uniqueidentifier"), *common), tracked=True),
            "legacy_thing": Table("legacy_thing", "legacy_things", "legacy_thingid", attrs(("legacy_thingid", "Uniqueidentifier"), ("name", "String"))),
        }
        for i in range(1, 6):
            self.tables["account"].put(i, i, revenue=1000.0 * i, _ownerid_value="11111111-1111-4111-8111-111111111111", donotemail=False, employees=i)
            self.tables["contact"].put(i, i)
        self.tables["legacy_thing"].put(1, 1)
        self.log: list[str] = []
        self.requests: list[dict[str, Any]] = []
        self.reject_expand = False
        self.expired = False
        self.fail_after: int | None = None
        self.delta_requests = 0
        self.status: dict[str, int] = {}
        self.link_to: str | None = None

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        parsed = urlparse(url)
        query = {k: v[0] for k, v in parse_qs(parsed.query, keep_blank_values=True).items()} | {k: str(v) for k, v in (params or {}).items()}
        self.requests.append({"path": parsed.path, "query": dict(query), "headers": dict(headers)})
        self.log.append(f"{parsed.netloc}{parsed.path}?{urlencode(sorted(query.items()))}")
        for suffix, status in self.status.items():
            if parsed.path.endswith(suffix):
                return Reply(status, {"error": {"code": "0x0", "message": "STUB-VENDOR-WORDS"}})
        prefix = "/api/data/v9.2/"
        if not parsed.path.startswith(prefix):
            return Reply(404, {})
        resource = parsed.path[len(prefix) :]
        if resource == "WhoAmI":
            return Reply(200, {"UserId": "u", "OrganizationId": "o"})
        if resource == "EntityDefinitions":
            if self.reject_expand and "$expand" in query:
                return Reply(400, {"error": {"message": "STUB-VENDOR-WORDS"}})
            value = []
            for t in self.tables.values():
                entry = {k: v for k, v in t.definition().items() if k != "Attributes"}
                if "$expand" in query:
                    entry["Attributes"] = [a for a in t.attributes if a["LogicalName"] == "modifiedon"]
                value.append(entry)
            return Reply(200, {"value": value})
        if resource.startswith("EntityDefinitions(LogicalName='"):
            name = resource.split("'")[1]
            return Reply(200, self.tables[name].definition()) if name in self.tables else Reply(404, {"error": {"message": "STUB-VENDOR-WORDS"}})
        table = next((t for t in self.tables.values() if t.entity_set == resource), None)
        return self._rows(table, parsed.path, query, headers) if table else Reply(404, {})

    def _rows(self, table: Table, path: str, query: dict[str, str], headers: dict) -> Reply:
        prefer = headers.get("Prefer", "")
        tracking = "odata.track-changes" in prefer
        token = query.get("$deltatoken")
        if tracking and any(k in query for k in ("$filter", "$orderby", "$expand", "$top")):
            return Reply(400, {"error": {"message": "The query parameter isn't supported when Change Tracking is enabled. STUB-VENDOR-WORDS"}})
        if tracking or token:
            if not table.tracked:
                return Reply(400, {"error": {"message": "STUB-VENDOR-WORDS"}})
            self.delta_requests += 1
            if self.fail_after is not None and self.delta_requests > self.fail_after:
                return Reply(503, {"error": {"message": "STUB-VENDOR-WORDS"}})
            if token and self.expired:
                return Reply(400, {"error": {"message": "The change token is too old. STUB-VENDOR-WORDS"}})
        size = int(prefer.split("odata.maxpagesize=")[1].split(",")[0]) if "odata.maxpagesize=" in prefer else 5000
        size = min(size, self.cap)
        if token:
            since = int(token)
            rows: list[dict] = []
            for rid, ver in sorted(table.vers.items(), key=lambda kv: kv[1]):
                if ver <= since:
                    continue
                rows.append({"@odata.context": f"{ORIGIN}/api/data/v9.2/$metadata#{table.entity_set}/$deletedEntity", "id": rid, "reason": "deleted"} if rid in table.deleted else table.rows[rid])
        else:
            rows = [r for rid, r in table.rows.items() if rid not in table.deleted]
            if "$filter" in query:
                _, op, value = query["$filter"].split(" ")
                assert op == "ge"
                rows = [r for r in rows if r["modifiedon"] >= value]
            order = [o.split(" ")[0] for o in query.get("$orderby", f"{table.key} asc").split(",")]
            rows.sort(key=lambda r: tuple(r[o] for o in order))
        select = query.get("$select", "").split(",") if query.get("$select") else None
        if select:
            rows = [r if "@odata.context" in r else {k: v for k, v in r.items() if k in select or k.startswith("@")} for r in rows]
        start = int(query.get("$skiptoken", "0"))
        chunk = rows[start : start + size]
        body: dict[str, Any] = {"@odata.context": f"{ORIGIN}/api/data/v9.2/$metadata#{table.entity_set}", "value": chunk}
        origin = self.link_to or ORIGIN
        base_query = {k: v for k, v in query.items() if k != "$skiptoken"}
        if start + len(chunk) < len(rows):
            body["@odata.nextLink"] = f"{origin}{path}?{urlencode({**base_query, '$skiptoken': start + len(chunk)})}"
        elif tracking or token:
            body["@odata.deltaLink"] = f"{origin}{path}?{urlencode({**{k: v for k, v in base_query.items()}, '$deltatoken': table.version})}"
        return Reply(200, body)


@pytest.fixture
def api() -> Dataverse:
    return Dataverse()


TOKEN_POSTS: list[dict] = []


@pytest.fixture(autouse=True)
def entra(monkeypatch: pytest.MonkeyPatch) -> None:
    TOKEN_POSTS.clear()

    def post(url: str, **kw: Any) -> Reply:
        form = kw.get("data") or {}
        TOKEN_POSTS.append({"url": url, "form": dict(form)})
        if url != f"https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/token":
            return Reply(400, {"error": "invalid_request", "error_description": "STUB-VENDOR-WORDS"})
        if form.get("grant_type") == "client_credentials":
            ok = form.get("client_id") == "app-id" and form.get("client_secret") == "app-secret" and form.get("scope") == f"https://{HOST}/.default"
            return Reply(200, {"token_type": "Bearer", "expires_in": 3599, "access_token": "app-only-access"}) if ok else Reply(401, {"error": "invalid_client", "error_description": "STUB-VENDOR-WORDS"})
        if form.get("grant_type") == "refresh_token" and form.get("refresh_token") == "refresh-1":
            return Reply(200, {"access_token": "delegated-access", "refresh_token": "refresh-2", "expires_in": 3599})
        return Reply(400, {"error": "invalid_grant", "error_description": "STUB-VENDOR-WORDS"})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)


@pytest.fixture(autouse=True)
def clock(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(dynamics365, "now", lambda: SEEN)


def open_(api: Dataverse, dial: dict = DIAL, secrets: dict = SECRETS, **kw: Any):  # noqa: ANN202
    return saas_adapter.open_session(dial, secrets, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None, **kw)


def read(api: Dataverse, name: str, since: Any = None, state: dict | None = None, dial: dict = DIAL, secrets: dict = SECRETS) -> list[dict]:
    module, _vendor, parts, session = open_(api, dial, secrets)
    return list(module.read(session, parts, dial, module.declaration(session, parts, dial, name), since=since, state=state))


def test_the_vendor_module_is_found_by_the_registry_and_keeps_state() -> None:
    assert vendors.module_for("dynamics365") is dynamics365 and dynamics365.uses_state is True


# ---- objects and the description -------------------------------------------------------------------


def test_every_public_table_is_listed_with_how_it_loads_and_whether_deletes_are_seen(api: Dataverse) -> None:
    module, _v, parts, session = open_(api)
    listed = {d.name: d for d in module.list_objects(session, parts, DIAL)}
    assert set(listed) == {"account", "contact", "legacy_thing"}
    assert (listed["account"].cursor, listed["account"].deletes, listed["account"].primary_key) == ("modifiedon", False, ("accountid",))
    assert not listed["account"].delta, "a table without change tracking is filtered by modifiedon"
    assert (listed["contact"].cursor, listed["contact"].deletes, listed["contact"].delta) == ("modifiedon", True, True), "decision 7: change tracking reports deletes; BLOCKER 17: from its delta link"
    assert listed["legacy_thing"].load_description() == "re-read in full: legacy_thing has no modifiedon column to filter by"
    first = api.requests[0]["query"]
    assert first["$filter"] == "IsPrivate eq false and IsIntersect eq false" and "modifiedon" in first["$expand"]
    assert api.requests[0]["headers"]["OData-Version"] == "4.0"


def test_a_list_dataverse_refuses_to_expand_still_lists_the_tables_as_incremental_and_the_declaration_decides_at_load(api: Dataverse) -> None:
    api.reject_expand = True
    module, _v, parts, session = open_(api)
    listed = {d.name: d for d in module.list_objects(session, parts, DIAL)}
    assert all(d.cursor == "modifiedon" for d in listed.values())
    assert module.declaration(session, parts, DIAL, "legacy_thing").cursor is None


@pytest.mark.parametrize("name", ["", "Account", "account'", "account)", "../account", "account/x", "a" * 65, "1account", "account\n", "acc ount", "account;x"])
def test_a_table_name_that_is_not_a_logical_name_is_refused_before_it_reaches_a_path(api: Dataverse, name: str) -> None:
    module, _v, parts, session = open_(api)
    with pytest.raises(saas_adapter.SaasSetupRefused):
        module.declaration(session, parts, DIAL, name)
    assert api.log == []


def test_a_table_dataverse_does_not_know_is_a_rejected_setting(api: Dataverse) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(api, "nosuchtable")


def test_the_description_names_the_columns_a_read_selects_before_the_load(api: Dataverse) -> None:
    module, _v, parts, session = open_(api)
    fields = {f.name: f for f in module.describe(session, parts, DIAL, "account")}
    assert set(fields) == {"accountid", "name", "modifiedon", "createdon", "revenue", "_ownerid_value", "donotemail", "employees", "is_deleted"}
    assert "owneridname" not in fields and "entityimage" not in fields, "a display companion and an image are not read"
    assert (fields["revenue"].type_name, fields["modifiedon"].type_name, fields["donotemail"].type_name, fields["employees"].type_name) == ("double", "timestamp", "bool", "bigint")
    assert fields["accountid"].nullable is False and fields["is_deleted"].nullable is False


# ---- reading ------------------------------------------------------------------------------------------


def test_a_first_read_pages_by_the_next_link_with_the_same_preference_on_every_page_in_key_order(api: Dataverse) -> None:
    got = read(api, "account")
    assert [r["name"] for r in got] == [f"row {i}" for i in range(1, 6)] and len([r for r in api.requests if r["path"].endswith("/accounts")]) == 3
    for request in (r for r in api.requests if r["path"].endswith("/accounts")):
        assert request["headers"]["Prefer"] == "odata.maxpagesize=500", "the same preference on every page"
    first = got[0]
    assert first["modifiedon"] == BASE + timedelta(minutes=1) and first["is_deleted"] is False and "@odata.etag" not in first
    assert first["_ownerid_value"] == "11111111-1111-4111-8111-111111111111"
    assert [r["query"].get("$orderby") for r in api.requests if r["path"].endswith("/accounts")][0] == "accountid asc"


def test_a_later_read_filters_by_modifiedon_and_orders_by_it_and_the_key(api: Dataverse) -> None:
    got = read(api, "account", since=BASE + timedelta(minutes=4))
    assert [r["name"] for r in got] == ["row 4", "row 5"]
    query = next(r["query"] for r in api.requests if r["path"].endswith("/accounts"))
    assert query["$filter"] == "modifiedon ge 2026-10-01T10:04:00Z" and query["$orderby"] == "modifiedon asc,accountid asc"
    api.tables["account"].put(2, 30, name="row 2 changed")
    assert [r["name"] for r in read(api, "account", since=(BASE + timedelta(minutes=10)).isoformat())] == ["row 2 changed"]


def test_a_table_without_a_change_time_is_read_in_full(api: Dataverse) -> None:
    got = read(api, "legacy_thing", since=BASE)
    assert [r["name"] for r in got] == ["row 1"] and all("$filter" not in r["query"] for r in api.requests if r["path"].endswith("/legacy_things"))
    assert "modifiedon" not in got[0]


def test_a_table_without_tracking_never_claims_a_delete_even_with_state(api: Dataverse) -> None:
    state: dict = {}
    got = read(api, "account", state=state)
    assert len(got) == 5 and state == {}, "the plain filtered path keeps no link"
    assert all("odata.track-changes" not in r["headers"].get("Prefer", "") for r in api.requests)


def test_a_link_to_another_host_stops_the_run(api: Dataverse) -> None:
    api.link_to = "https://evil.example.invalid"
    with pytest.raises(ssrf_guard.SsrfBlocked) as err:
        read(api, "account")
    assert "evil" not in str(err.value)


def test_the_access_failures_have_their_own_fixed_classes(api: Dataverse) -> None:
    for status, expected, klass in ((401, saas_session.SaasSignInRefused, "sign_in_refused"), (403, saas_session.SaasPermissionMissing, "permission_missing")):
        api.status["/accounts"] = status
        with pytest.raises(expected) as err:
            read(api, "account")
        assert "STUB-VENDOR-WORDS" not in str(err.value) and ingest_errors.classify(err.value).error_class == klass


# ---- change tracking: the delta link in state, deletes ---------------------------------------------------


def tracked_read(api: Dataverse, state: dict) -> list[dict]:
    return read(api, "contact", since=None, state=state)


def test_a_first_tracked_run_asks_for_tracking_without_a_filter_and_keeps_the_link_after_the_last_page(api: Dataverse) -> None:
    state: dict = {}
    got = tracked_read(api, state)
    assert [r["name"] for r in got] == [f"row {i}" for i in range(1, 6)]
    contact_requests = [r for r in api.requests if r["path"].endswith("/contacts")]
    assert len(contact_requests) == 3 and contact_requests[0]["headers"]["Prefer"] == "odata.track-changes,odata.maxpagesize=500"
    assert not any(k in contact_requests[0]["query"] for k in ("$filter", "$orderby")), "tracking takes neither"
    assert state["delta"]["link"].endswith(f"$deltatoken={api.tables['contact'].version}") and state["delta"]["select"] == "contactid,name,modifiedon,createdon"


def test_a_later_tracked_run_opens_the_saved_link_and_a_delete_is_marked_not_removed(api: Dataverse) -> None:
    state: dict = {}
    tracked_read(api, state)
    api.tables["contact"].put(2, 40, name="row 2 changed")
    api.tables["contact"].put(6, 41)
    api.tables["contact"].delete(3)
    api.requests.clear()
    got = {r["contactid"]: r for r in tracked_read(api, state)}
    assert sorted(got) == [f"00000000-0000-0000-0000-{n:012d}" for n in (2, 3, 6)]
    assert got["00000000-0000-0000-0000-000000000003"] == {"contactid": "00000000-0000-0000-0000-000000000003", "is_deleted": True, "modifiedon": SEEN}
    assert got["00000000-0000-0000-0000-000000000002"]["name"] == "row 2 changed" and got["00000000-0000-0000-0000-000000000002"]["is_deleted"] is False
    delta = [r for r in api.requests if r["path"].endswith("/contacts")]
    assert delta[0]["query"]["$deltatoken"] == "5" and "odata.track-changes" not in delta[0]["headers"]["Prefer"]
    assert state["delta"]["link"].endswith("$deltatoken=8")


def test_a_tracked_read_that_fails_halfway_keeps_the_old_delta_link(api: Dataverse) -> None:
    state: dict = {}
    tracked_read(api, state)
    saved = dict(state["delta"])
    for i in range(10, 16):
        api.tables["contact"].put(i, 60 + i)
    api.fail_after = api.delta_requests + 1
    with pytest.raises(requests.HTTPError):
        tracked_read(api, state)
    assert state["delta"] == saved, "plan section 8 item 6: the link moves only after the whole feed was handed on"
    api.fail_after = None
    assert len(tracked_read(api, state)) == 6


def test_a_link_dataverse_no_longer_serves_is_a_full_resync_with_a_new_link(api: Dataverse) -> None:
    state: dict = {}
    tracked_read(api, state)
    api.tables["contact"].put(7, 70)
    api.expired = True
    got = tracked_read(api, state)
    assert len(got) == 6 and state["delta"]["link"].endswith(f"$deltatoken={api.tables['contact'].version}")
    api.expired = False
    assert tracked_read(api, state) == []


def test_a_resync_that_is_refused_too_is_a_failure(api: Dataverse) -> None:
    state = {"delta": {"link": f"{ORIGIN}/api/data/v9.2/contacts?$deltatoken=1", "select": "contactid,name,modifiedon,createdon"}}
    api.status["/contacts"] = 400
    with pytest.raises(requests.HTTPError):
        tracked_read(api, state)
    assert "delta" not in state


def test_a_link_saved_for_other_columns_is_not_reused(api: Dataverse) -> None:
    state = {"delta": {"link": f"{ORIGIN}/api/data/v9.2/contacts?$deltatoken=5", "select": "contactid"}}
    assert len(tracked_read(api, state)) == 5
    assert state["delta"]["select"] == "contactid,name,modifiedon,createdon"
    assert not any(r["query"].get("$deltatoken") == "5" for r in api.requests if r["path"].endswith("/contacts"))


def test_a_saved_link_to_another_host_stops_the_run_before_a_request_to_it(api: Dataverse) -> None:
    state = {"delta": {"link": "https://evil.example.invalid/api/data/v9.2/contacts?$deltatoken=1", "select": "contactid,name,modifiedon,createdon"}}
    with pytest.raises(ssrf_guard.SsrfBlocked):
        tracked_read(api, state)
    assert not any(r["path"].endswith("/contacts") for r in api.requests)


def test_a_tracked_table_read_without_state_is_read_in_full_and_sees_no_delete(api: Dataverse) -> None:
    api.tables["contact"].delete(2)
    got = read(api, "contact")
    assert len(got) == 4 and all(r["is_deleted"] is False for r in got)
    assert all("odata.track-changes" not in r["headers"].get("Prefer", "") for r in api.requests)


# ---- sign-in: the customer's tenant in the Entra address ---------------------------------------------------


def test_client_credentials_go_to_the_customers_tenant_with_the_orgs_dot_default_scope(api: Dataverse) -> None:
    read(api, "legacy_thing")
    assert TOKEN_POSTS[0]["url"] == f"https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/token"
    assert TOKEN_POSTS[0]["form"] == {"grant_type": "client_credentials", "client_id": "app-id", "client_secret": "app-secret", "scope": f"https://{HOST}/.default"}
    assert {r["headers"]["Authorization"] for r in api.requests} == {"Bearer app-only-access"}


def test_the_browser_sign_in_refreshes_at_the_tenant_and_a_new_refresh_token_is_reported(api: Dataverse) -> None:
    reported: list[str] = []
    module, _v, parts, session = open_(api, OAUTH_DIAL, OAUTH_SECRETS, report_refresh_token=reported.append)
    decl = module.declaration(session, parts, OAUTH_DIAL, "legacy_thing")
    assert len(list(module.read(session, parts, OAUTH_DIAL, decl, since=None, state=None))) == 1 and reported == ["refresh-2"]
    assert TOKEN_POSTS[0]["form"]["grant_type"] == "refresh_token" and "scope" not in TOKEN_POSTS[0]["form"]
    assert {r["headers"]["Authorization"] for r in api.requests} == {"Bearer delegated-access"}


def test_a_refused_client_secret_is_the_fixed_sign_in_failure(api: Dataverse) -> None:
    from dispar_orchestrate.adapters import rest as rest_adapter

    _m, _v, _p, session = open_(api, DIAL, {**SECRETS, "clientSecret": "wrong"})
    with pytest.raises(rest_adapter.OAuthSignInRefused) as err:
        session.get_json("/api/data/v9.2/WhoAmI")
    assert "STUB-VENDOR-WORDS" not in str(err.value) and ingest_errors.classify(err.value).error_class == "sign_in_refused"


def test_an_account_without_a_valid_tenant_is_a_rejected_setting(api: Dataverse) -> None:
    for account in (HOST, f"{HOST}/common", f"{HOST}/x"):
        with pytest.raises(saas_adapter.SaasSetupRefused):
            open_(api, {**DIAL, "account": account})
    assert TOKEN_POSTS == []


def test_the_vendor_row_matches_microsofts_documentation() -> None:
    row = vendors.vendor("dynamics365")
    assert row.oauth["authorizeUrl"] == "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"
    assert row.oauth["tokenUrl"] == "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token" and row.oauth["revocationUrl"] is None
    assert row.oauth["scopes"] == ["https://{account}/user_impersonation", "offline_access"] and row.integration["scope"] == "https://{account}/.default"
    assert row.whoami == {"method": "GET", "path": "/api/data/v9.2/WhoAmI"}


# ---- the whole path: real dlt state and Iceberg ------------------------------------------------------------


def accept_all(table: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj(name: str, target: str, key: str) -> dict[str, Any]:
    return {"name": name, "target": target, "loadMode": "incremental_dedup", "incrementalKey": "modifiedon", "primaryKey": [key]}


def run(lab: Lab, api: Dataverse, name: str, target: str, key: str) -> Any:
    o = obj(name, target, key)
    built = saas_adapter.build_source(DIAL, SECRETS, [o], gate=accept_all, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)
    assert built is not None
    return load_via_sink(built.source, target, lab.config, f._load_plan("saas", o))


@pytest.mark.loader
def test_a_second_run_of_a_table_without_tracking_reads_only_the_newer_rows_and_merges(lab: Lab, api: Dataverse) -> None:
    assert run(lab, api, "account", "dv_accounts", "accountid").rows == 5
    api.tables["account"].put(2, 90, name="row 2 changed")
    api.tables["account"].put(6, 91)
    api.requests.clear()
    run(lab, api, "account", "dv_accounts", "accountid")
    rows = {r["name"] for r in lab.rows("dv_accounts")}
    assert len(lab.rows("dv_accounts")) == 6 and "row 2 changed" in rows and "row 2" not in rows, "merged on the key: no duplicate"
    assert "$filter" in next(r["query"] for r in api.requests if r["path"].endswith("/accounts")), "the second run filtered by modifiedon"


@pytest.mark.loader
def test_a_tracked_table_second_run_reads_the_changes_and_marks_the_delete(lab: Lab, api: Dataverse) -> None:
    assert run(lab, api, "contact", "dv_contacts", "contactid").rows == 5
    api.tables["contact"].put(2, 90, name="row 2 changed")
    api.tables["contact"].delete(3)
    api.requests.clear()
    run(lab, api, "contact", "dv_contacts", "contactid")
    rows = {r["contactid"][-1]: r for r in lab.rows("dv_contacts")}
    assert sorted(rows) == ["1", "2", "3", "4", "5"] and rows["3"]["is_deleted"] is True and rows["1"]["is_deleted"] is False and rows["2"]["name"] == "row 2 changed"
    assert any("$deltatoken" in r["query"] for r in api.requests if r["path"].endswith("/contacts")), "the second run opened the saved link"


@pytest.mark.loader
def test_a_tracked_change_stamped_before_the_last_runs_newest_row_is_not_dropped(lab: Lab, api: Dataverse) -> None:
    """PR #216 final review BLOCKER 17, case 1: change tracking reports a row edited while run 1
    was paging, whose `modifiedon` is below the newest row run 1 read. dlt's cursor filter dropped it."""
    run(lab, api, "contact", "dv_contacts", "contactid")
    api.tables["contact"].put(2, 90, name="newest at run 1")
    run(lab, api, "contact", "dv_contacts", "contactid")
    api.tables["contact"].put(4, 60, name="edited during run 1")
    run(lab, api, "contact", "dv_contacts", "contactid")
    rows = {r["contactid"][-1]: r for r in lab.rows("dv_contacts")}
    assert rows["4"]["name"] == "edited during run 1"


@pytest.mark.loader
def test_a_tracked_change_after_a_delete_is_not_dropped_even_though_the_delete_is_stamped_now(lab: Lab, api: Dataverse) -> None:
    """PR #216 final review BLOCKER 17, case 2: the deleted row carries `SEEN` as its `modifiedon`,
    later than any real change; a change after it is below the last run's highest."""
    run(lab, api, "contact", "dv_contacts", "contactid")
    api.tables["contact"].delete(3)
    run(lab, api, "contact", "dv_contacts", "contactid")
    api.tables["contact"].put(2, 60, name="changed after the delete")
    run(lab, api, "contact", "dv_contacts", "contactid")
    rows = {r["contactid"][-1]: r for r in lab.rows("dv_contacts")}
    assert rows["2"]["name"] == "changed after the delete" and rows["3"]["is_deleted"] is True


@pytest.mark.loader
def test_a_tracked_run_that_dies_before_it_loads_does_not_move_the_delta_link(lab: Lab, api: Dataverse, monkeypatch: pytest.MonkeyPatch) -> None:
    """Plan section 8 item 6: the read finished and the load failed; the next run asks from the last committed link."""
    run(lab, api, "contact", "dv_contacts", "contactid")
    api.tables["contact"].put(2, 90, name="row 2 changed")
    api.tables["contact"].put(6, 91)
    real = sink_module._bronze_pipeline

    def dying(name: str, config: Any) -> Any:
        pipeline = real(name, config)

        def run_then_die(source: Any, **kw: Any) -> Any:
            pipeline.extract(source, **kw)
            pipeline.normalize()
            raise RuntimeError("the destination went away")

        pipeline.run = run_then_die  # type: ignore[method-assign]
        return pipeline

    monkeypatch.setattr(sink_module, "_bronze_pipeline", dying)
    with pytest.raises(RuntimeError):
        run(lab, api, "contact", "dv_contacts", "contactid")
    monkeypatch.setattr(sink_module, "_bronze_pipeline", real)
    api.requests.clear()
    run(lab, api, "contact", "dv_contacts", "contactid")
    rows = {r["contactid"][-1]: r["name"] for r in lab.rows("dv_contacts")}
    assert sorted(rows) == ["1", "2", "3", "4", "5", "6"] and rows["2"] == "row 2 changed", "the changes of the dead run arrive"
    assert any(r["query"].get("$deltatoken") == "5" for r in api.requests if r["path"].endswith("/contacts")), "asked from the last COMMITTED link"


@pytest.mark.loader
def test_a_stale_link_in_a_real_run_reloads_every_row_and_continues(lab: Lab, api: Dataverse) -> None:
    run(lab, api, "contact", "dv_contacts", "contactid")
    api.tables["contact"].put(2, 90, name="row 2 changed")
    api.expired = True
    run(lab, api, "contact", "dv_contacts", "contactid")
    assert len(lab.rows("dv_contacts")) == 5 and any(r["name"] == "row 2 changed" for r in lab.rows("dv_contacts"))
    api.expired = False
    api.tables["contact"].put(7, 95)
    run(lab, api, "contact", "dv_contacts", "contactid")
    assert len(lab.rows("dv_contacts")) == 6
