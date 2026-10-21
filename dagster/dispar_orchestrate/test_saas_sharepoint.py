"""`saas/vendors/sharepoint.py` (`SRC-3` slice 5, V-SharePoint.1 to .3): objects (the lists of the sites),
the Graph delta query with its link kept in dlt state, deletes marked, a 410 Gone as a full resync, and both
Entra sign-ins (the customer's tenant in the token address), through the real `SaasSession` over a FAKE Microsoft
Graph and a FAKE Entra token endpoint. No network. The loader tests run `load_via_sink` for real on
`test_load_modes_measurement.Lab` (plan section 8 item 6: the state survives a failed run); CI runs them in
`dagster-loader-tests`."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Any
from urllib.parse import parse_qs, urlparse

import pytest
import requests

from dispar_orchestrate import ingest_errors, schema_observer, ssrf_guard
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import session as saas_session
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import sharepoint
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_zendesk import Reply, resolver

ORIGIN = "https://graph.microsoft.com"
TENANT = "00000000-0000-0000-0000-0000000000a1"
ACCOUNT = f"acme/{TENANT}"
SITE = "acme.sharepoint.com,11111111-1111-4111-8111-111111111111,22222222-2222-4222-8222-222222222222"
OTHER_SITE = "acme.sharepoint.com,33333333-3333-4333-8333-333333333333,44444444-4444-4444-8444-444444444444"
LIST = "55555555-5555-4555-8555-555555555555"
OBJECT = f"{SITE}/{LIST}"
DIAL = {"vendor": "sharepoint", "account": ACCOUNT, "auth": {"type": "integration"}}
OAUTH_DIAL = {"vendor": "sharepoint", "account": ACCOUNT, "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}
SECRETS = {"clientId": "app-id", "clientSecret": "app-secret"}
OAUTH_SECRETS = {"clientSecret": "client-secret", "refreshToken": "refresh-1"}
BASE = datetime(2026, 10, 1, 10, 0, 0, tzinfo=timezone.utc)
SEEN = datetime(2026, 10, 20, 12, 0, 0, tzinfo=timezone.utc)


def stamp(minutes: float) -> str:
    return (BASE + timedelta(minutes=minutes)).strftime("%Y-%m-%dT%H:%M:%SZ")


class Graph:
    """The pieces of Graph that SharePoint uses, as documented: `GET /sites?search=`, the lists of a site,
    `items` and `items/delta` (a `deleted` facet for a deleted item, `@odata.nextLink` pages ending in an
    `@odata.deltaLink`, a 410 Gone for a token it cannot serve). `version` counts changes; a delta token is
    `v<version>`."""

    def __init__(self, cap: int = 2) -> None:
        self.cap = cap
        self.version = 0
        self.items: dict[str, dict[str, Any]] = {}
        self.sites: list[dict[str, Any]] = [
            {"id": SITE, "displayName": "Team", "isPersonalSite": False},
            {"id": "me.sharepoint.com,66666666-6666-4666-8666-666666666666,77777777-7777-4777-8777-777777777777", "isPersonalSite": True},
        ]
        self.lists: dict[str, list[dict[str, Any]]] = {
            SITE: [
                {"id": LIST, "displayName": "Tasks", "list": {"template": "genericList", "hidden": False}},
                {"id": "88888888-8888-4888-8888-888888888888", "displayName": "Documents", "list": {"template": "documentLibrary", "hidden": False}},
                {"id": "99999999-9999-4999-8999-999999999999", "displayName": "Hidden", "list": {"template": "genericList", "hidden": True}},
            ],
            OTHER_SITE: [{"id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "displayName": "Other", "list": {"template": "genericList", "hidden": False}}],
        }
        self.log: list[str] = []
        self.headers: list[dict] = []
        self.expired = False
        self.fail_after: int | None = None  # the n-th delta request fails with 503
        self.status: dict[str, int] = {}
        self.link_to: str | None = None
        self.delta_requests = 0

    def put(self, item_id: int, minutes: float, **fields: Any) -> None:
        self.version += 1
        self.items[str(item_id)] = {
            "id": str(item_id), "ver": self.version, "deleted": False, "createdDateTime": stamp(0), "lastModifiedDateTime": stamp(minutes),
            "eTag": f'"{{E{item_id}}},{self.version}"', "webUrl": f"{ORIGIN}/item/{item_id}", "contentType": {"id": "0x01", "name": "Item"},
            "fields": {"@odata.etag": "x", "Title": f"item {item_id}", **fields},
        }  # fmt: skip

    def delete(self, item_id: int) -> None:
        self.version += 1
        self.items[str(item_id)].update(ver=self.version, deleted=True)

    @staticmethod
    def _visible(entry: dict[str, Any]) -> dict[str, Any]:
        if entry["deleted"]:
            return {"id": entry["id"], "deleted": {"state": "deleted"}, "parentReference": {"siteId": "x"}}
        return {k: v for k, v in entry.items() if k not in ("ver", "deleted")}

    def _page(self, rows: list[dict], start: int, url_for: Any, last: str) -> Reply:
        chunk = rows[start : start + self.cap]
        body: dict[str, Any] = {"value": chunk}
        if start + len(chunk) < len(rows):
            body["@odata.nextLink"] = (self.link_to or ORIGIN) + url_for(start + len(chunk))
        else:
            body[last[0]] = (self.link_to or ORIGIN) + last[1]
        return Reply(200, body)

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        self.headers.append(headers)
        parsed = urlparse(url)
        query = {k: v[0] for k, v in parse_qs(parsed.query).items()} | {k: str(v) for k, v in (params or {}).items()}
        self.log.append(f"{parsed.netloc}{parsed.path}?{'&'.join(f'{k}={v}' for k, v in sorted(query.items()))}")
        for suffix, status in self.status.items():
            if parsed.path.endswith(suffix):
                return Reply(status, {"error": {"message": "STUB-VENDOR-WORDS"}})
        path = parsed.path
        if path == "/v1.0/sites":
            return Reply(200, {"value": [s for s in self.sites if s.get("id")]})
        if path == f"/v1.0/sites/{SITE.split(',')[0]}":
            return Reply(200, {"id": SITE, "displayName": "Team"})
        pieces = path.split("/")  # '', v1.0, sites, <site>, lists, <list>, items, [delta]
        if len(pieces) >= 5 and pieces[2] == "sites" and pieces[4] == "lists":
            site = pieces[3]
            if len(pieces) == 5:
                return Reply(200, {"value": self.lists.get(site, [])})
            if pieces[5] != LIST or site != SITE:
                return Reply(404, {"error": {"message": "STUB-VENDOR-WORDS"}})
            if pieces[6:] == ["items"]:
                rows = [self._visible(e) for e in self.items.values() if not e["deleted"]]
                start = int(query.get("skip", "0"))
                return self._page(rows, start, lambda n: f"/v1.0/sites/{SITE}/lists/{LIST}/items?skip={n}", ("@odata.deltaLink", "/never"))
            if pieces[6:] == ["items", "delta"]:
                return self._delta(query)
        return Reply(404, {})

    def _delta(self, query: dict[str, str]) -> Reply:
        self.delta_requests += 1
        if self.fail_after is not None and self.delta_requests > self.fail_after:
            return Reply(503, {"error": {"message": "STUB-VENDOR-WORDS"}})
        token = query.get("token")
        if token and self.expired:
            return Reply(410, {"error": {"code": "resyncRequired", "message": "STUB-VENDOR-WORDS"}}, {"Location": f"{ORIGIN}/never-followed"})
        since = int(token[1:]) if token else 0
        rows = [self._visible(e) for e in sorted(self.items.values(), key=lambda e: e["ver"]) if e["ver"] > since and (token or not e["deleted"])]
        start = int(query.get("skip", "0"))
        base = f"/v1.0/sites/{SITE}/lists/{LIST}/items/delta"
        return self._page(rows, start, lambda n: f"{base}?{'token=v' + str(since) + '&' if token else ''}skip={n}", ("@odata.deltaLink", f"{base}?token=v{self.version}"))


@pytest.fixture
def graph() -> Graph:
    g = Graph()
    for i in range(1, 6):
        g.put(i, i, Status="open", Owner={"LookupId": 7})
    return g


TOKEN_POSTS: list[dict] = []


@pytest.fixture(autouse=True)
def entra(monkeypatch: pytest.MonkeyPatch) -> None:
    """Entra's token endpoint at the customer's tenant: the client-credentials grant (no refresh token) and the
    refresh grant (a new refresh token each time)."""
    TOKEN_POSTS.clear()

    def post(url: str, **kw: Any) -> Reply:
        form = kw.get("data") or {}
        TOKEN_POSTS.append({"url": url, "form": dict(form), "json": kw.get("json")})
        if url != f"https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/token":
            return Reply(400, {"error": "invalid_request", "error_description": "STUB-VENDOR-WORDS"})
        if form.get("grant_type") == "client_credentials":
            ok = form.get("client_id") == "app-id" and form.get("client_secret") == "app-secret" and form.get("scope") == "https://graph.microsoft.com/.default"
            return Reply(200, {"token_type": "Bearer", "expires_in": 3599, "access_token": "app-only-access"}) if ok else Reply(401, {"error": "invalid_client", "error_description": "STUB-VENDOR-WORDS"})
        if form.get("grant_type") == "refresh_token" and form.get("refresh_token") == "refresh-1":
            return Reply(200, {"access_token": "delegated-access", "refresh_token": "refresh-2", "expires_in": 3599})
        return Reply(400, {"error": "invalid_grant", "error_description": "STUB-VENDOR-WORDS"})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)


@pytest.fixture(autouse=True)
def clock(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(sharepoint, "now", lambda: SEEN)


def open_(graph: Graph, dial: dict = DIAL, secrets: dict = SECRETS, **kw: Any):  # noqa: ANN202
    return saas_adapter.open_session(dial, secrets, resolve_checked=resolver, http_get=graph.get, sleep=lambda _s: None, **kw)


def read(graph: Graph, state: dict | None, name: str = OBJECT, dial: dict = DIAL, secrets: dict = SECRETS) -> list[dict]:
    module, _vendor, parts, session = open_(graph, dial, secrets)
    return list(module.read(session, parts, dial, module.declaration(session, parts, dial, name), since=None, state=state))


def test_the_vendor_module_is_found_by_the_registry_and_keeps_state() -> None:
    assert vendors.module_for("sharepoint") is sharepoint and sharepoint.uses_state is True


# ---- objects: the lists of the sites ------------------------------------------------------------------


def test_the_lists_of_the_found_sites_are_offered_without_libraries_hidden_lists_or_personal_sites(graph: Graph) -> None:
    module, _v, parts, session = open_(graph)
    listed = module.list_objects(session, parts, DIAL)
    assert [d.name for d in listed] == [OBJECT], "a document library and a hidden list are left out; a personal site is not searched"
    assert all(d.load_description() == "incremental" and d.deletes and d.primary_key == ("id",) for d in listed)
    assert "search=*" in graph.log[0] and graph.log[0].startswith("graph.microsoft.com/v1.0/sites?")


def test_sites_the_dial_names_replace_the_search(graph: Graph) -> None:
    module, _v, parts, session = open_(graph)
    dial = {**DIAL, "objects": [OTHER_SITE, "not a site", OBJECT]}
    assert [d.name for d in module.list_objects(session, parts, dial)] == ["acme.sharepoint.com,33333333-3333-4333-8333-333333333333,44444444-4444-4444-8444-444444444444/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"]
    assert not any("search" in line for line in graph.log)


@pytest.mark.parametrize(
    "name",
    ["", "1", f"{SITE}", f"{SITE}/", f"{SITE}/not-a-guid", f"../{SITE}/{LIST}", f"{SITE}/../{LIST}", f"x,y,z/{LIST}", f"{SITE}/{LIST}/delta", f"{SITE}/{LIST}?x=1", f"{SITE}/{LIST}\n"],
)
def test_an_object_name_that_is_not_a_site_id_and_a_list_guid_is_refused_before_it_reaches_a_path(graph: Graph, name: str) -> None:
    module, _v, parts, session = open_(graph)
    with pytest.raises(saas_adapter.SaasSetupRefused):
        module.declaration(session, parts, DIAL, name)
    assert graph.log == []


def test_sharepoint_does_not_describe_its_fields_so_a_changed_column_is_seen_after_the_load() -> None:
    assert not hasattr(sharepoint, "describe"), "decision 9 lists no SharePoint describe"


# ---- reading: the delta feed, its link in state, deletes -----------------------------------------------


def test_a_first_incremental_run_enumerates_the_feed_and_keeps_the_delta_link_only_when_it_is_all_read(graph: Graph) -> None:
    state: dict = {}
    got = read(graph, state)
    assert [r["id"] for r in got] == ["1", "2", "3", "4", "5"] and len([line for line in graph.log if "/delta" in line]) == 3
    assert state == {"delta_link": f"{ORIGIN}/v1.0/sites/{SITE}/lists/{LIST}/items/delta?token=v{graph.version}"}
    first = got[0]
    assert first["modified_at"] == BASE + timedelta(minutes=1) and first["is_deleted"] is False and first["field_Title"] == "item 1"
    assert first["field_Owner"] == '{"LookupId": 7}' and first["content_type"] == "Item" and "field_@odata.etag" not in first
    assert not any(isinstance(v, (dict, list)) for v in first.values()), "nested values are one JSON text"
    assert "$expand=fields" in graph.log[0] or "expand=fields" in graph.log[0]


def test_a_later_run_opens_the_saved_link_and_gets_only_the_changes_and_a_delete_is_marked_not_removed(graph: Graph) -> None:
    state: dict = {}
    read(graph, state)
    graph.put(2, 10, Title="item 2 changed")
    graph.put(6, 11)
    graph.delete(3)
    graph.log.clear()
    got = {r["id"]: r for r in read(graph, state)}
    assert sorted(got) == ["2", "3", "6"] and got["2"]["field_Title"] == "item 2 changed"
    assert got["3"] == {"id": "3", "is_deleted": True, "modified_at": SEEN}, "a deleted item keeps its id, the mark and the time it was seen"
    assert graph.log[0].startswith(f"graph.microsoft.com/v1.0/sites/{SITE}/lists/{LIST}/items/delta?token=v5"), "the saved link is opened as it is"
    assert state["delta_link"].endswith(f"token=v{graph.version}")


def test_a_read_that_fails_halfway_keeps_the_old_delta_link(graph: Graph) -> None:
    state: dict = {}
    read(graph, state)
    saved = state["delta_link"]
    for i in range(10, 16):
        graph.put(i, 20 + i)
    graph.fail_after = graph.delta_requests + 1  # the first page comes, the second is a 503
    with pytest.raises(requests.HTTPError):
        read(graph, state)
    assert state["delta_link"] == saved, "plan section 8 item 6: the link moves only after the whole feed was handed on"
    graph.fail_after = None
    assert sorted(r["id"] for r in read(graph, state)) == ["10", "11", "12", "13", "14", "15"]


def test_a_gone_link_is_a_full_resync_and_the_new_link_is_stored(graph: Graph) -> None:
    state: dict = {}
    read(graph, state)
    graph.put(7, 30)
    graph.expired = True
    got = read(graph, state)
    assert sorted(r["id"] for r in got) == ["1", "2", "3", "4", "5", "7"], "every item again, from a delta with no token"
    assert state["delta_link"].endswith(f"token=v{graph.version}")
    assert not any("never-followed" in line for line in graph.log), "the Location header of the 410 is not followed"
    graph.expired = False
    assert read(graph, state) == [], "the next run continues from the new link"


def test_a_resync_that_is_refused_too_is_a_failure_and_leaves_no_link(graph: Graph) -> None:
    state = {"delta_link": f"{ORIGIN}/v1.0/sites/{SITE}/lists/{LIST}/items/delta?token=v1"}
    graph.status["/delta"] = 410
    with pytest.raises(requests.HTTPError):
        read(graph, state)
    assert "delta_link" not in state


def test_a_saved_link_to_another_host_stops_the_run_before_any_request_is_made(graph: Graph) -> None:
    state = {"delta_link": "https://evil.example.invalid/v1.0/delta?token=v1"}
    with pytest.raises(ssrf_guard.SsrfBlocked) as err:
        read(graph, state)
    assert "evil" not in str(err.value) and graph.log == []


def test_a_next_link_to_another_host_stops_the_run(graph: Graph) -> None:
    graph.link_to = "https://evil.example.invalid"
    with pytest.raises(ssrf_guard.SsrfBlocked) as err:
        read(graph, {})
    assert "evil" not in str(err.value)


def test_a_run_that_is_not_incremental_reads_the_list_in_full_without_state_and_sees_no_delete(graph: Graph) -> None:
    graph.delete(2)
    got = read(graph, None)
    assert [r["id"] for r in got] == ["1", "3", "4", "5"] and all(r["is_deleted"] is False for r in got)
    assert not any("/delta" in line for line in graph.log) and any("items?" in line for line in graph.log)


def test_a_feed_without_a_link_to_continue_from_is_refused(graph: Graph, monkeypatch: pytest.MonkeyPatch) -> None:
    original = graph._delta

    def no_link(query: dict[str, str]) -> Reply:
        reply = original(query)
        reply._body.pop("@odata.deltaLink", None)
        return reply

    monkeypatch.setattr(graph, "_delta", no_link)
    state: dict = {}
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(graph, state)
    assert state == {}


def test_a_list_or_site_graph_does_not_know_is_a_rejected_setting_and_the_access_failures_have_fixed_classes(graph: Graph) -> None:
    other = f"{SITE}/bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(graph, {}, name=other)
    for status, expected, klass in (
        (401, saas_session.SaasSignInRefused, "sign_in_refused"),
        (403, saas_session.SaasPermissionMissing, "permission_missing"),
    ):
        graph.status["/delta"] = status
        with pytest.raises(expected) as err:
            read(graph, {})
        assert "STUB-VENDOR-WORDS" not in str(err.value) and ingest_errors.classify(err.value).error_class == klass


# ---- sign-in: the customer's tenant in the Entra address -----------------------------------------------


def test_client_credentials_go_to_the_customers_tenant_with_the_dot_default_scope_and_the_token_is_a_bearer(graph: Graph) -> None:
    read(graph, {})
    assert TOKEN_POSTS[0]["url"] == f"https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/token"
    assert TOKEN_POSTS[0]["form"] == {
        "grant_type": "client_credentials", "client_id": "app-id", "client_secret": "app-secret", "scope": "https://graph.microsoft.com/.default",
    }  # fmt: skip
    assert {h["Authorization"] for h in graph.headers} == {"Bearer app-only-access"}


def test_the_browser_sign_in_refreshes_at_the_tenant_and_a_new_refresh_token_is_reported(graph: Graph) -> None:
    reported: list[str] = []
    module, _v, parts, session = open_(graph, OAUTH_DIAL, OAUTH_SECRETS, report_refresh_token=reported.append)
    got = list(module.read(session, parts, OAUTH_DIAL, module.declaration(session, parts, OAUTH_DIAL, OBJECT), since=None, state={}))
    assert len(got) == 5 and reported == ["refresh-2"]
    form = TOKEN_POSTS[0]["form"]
    assert TOKEN_POSTS[0]["url"] == f"https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/token"
    assert form["grant_type"] == "refresh_token" and form["client_id"] == "app" and "scope" not in form
    assert {h["Authorization"] for h in graph.headers} == {"Bearer delegated-access"}


def test_a_refused_client_secret_is_the_fixed_sign_in_failure(graph: Graph) -> None:
    from dispar_orchestrate.adapters import rest as rest_adapter

    _m, _v, _p, session = open_(graph, DIAL, {**SECRETS, "clientSecret": "wrong"})
    with pytest.raises(rest_adapter.OAuthSignInRefused) as err:
        session.get_json("/v1.0/sites/acme.sharepoint.com")
    assert "STUB-VENDOR-WORDS" not in str(err.value) and ingest_errors.classify(err.value).error_class == "sign_in_refused"


def test_a_tenant_in_the_dial_that_is_not_valid_is_a_rejected_setting(graph: Graph) -> None:
    for account in ("acme", "acme/common", "acme/../x.example.invalid"):
        with pytest.raises(saas_adapter.SaasSetupRefused):
            open_(graph, {**DIAL, "account": account})
    assert TOKEN_POSTS == []


def test_the_vendor_row_matches_microsofts_documentation() -> None:
    row = vendors.vendor("sharepoint")
    assert row.oauth["authorizeUrl"] == "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"
    assert row.oauth["tokenUrl"] == "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token" and row.oauth["clientAuth"] == "post"
    assert row.oauth["revocationUrl"] is None, "the v2 platform has no RFC 7009 endpoint"
    assert row.oauth["scopes"] == ["https://graph.microsoft.com/Sites.Read.All", "offline_access"]
    assert row.integration["scope"] == "https://graph.microsoft.com/.default" and row.api_base == ORIGIN


# ---- the whole path: real dlt state and Iceberg ------------------------------------------------------


def accept_all(table: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj() -> dict[str, Any]:
    return {"name": OBJECT, "target": "sp_tasks", "loadMode": "incremental_dedup", "incrementalKey": "modified_at", "primaryKey": ["id"]}


def run(lab: Lab, graph: Graph) -> Any:
    built = saas_adapter.build_source(DIAL, SECRETS, [obj()], gate=accept_all, resolve_checked=resolver, http_get=graph.get, sleep=lambda _s: None)
    assert built is not None
    return load_via_sink(built.source, "sp_tasks", lab.config, f._load_plan("saas", obj()))


@pytest.mark.loader
def test_a_second_run_reads_only_the_changes_merges_them_and_marks_the_delete(lab: Lab, graph: Graph) -> None:
    assert run(lab, graph).rows == 5
    graph.put(2, 40, Title="item 2 changed")
    graph.put(6, 41)
    graph.delete(3)
    graph.log.clear()
    run(lab, graph)
    rows = {r["id"]: r for r in lab.rows("sp_tasks")}
    assert sorted(rows) == ["1", "2", "3", "4", "5", "6"], "merged on id: no duplicate, the deleted row still there"
    assert rows["2"]["field_title"] == "item 2 changed" and rows["3"]["is_deleted"] is True and rows["1"]["is_deleted"] is False
    assert not any("expand" in line and "token" not in line for line in graph.log), "the second run opened the saved link, not the whole list"


@pytest.mark.loader
def test_a_run_that_dies_before_it_loads_does_not_move_the_delta_link(lab: Lab, graph: Graph, monkeypatch: pytest.MonkeyPatch) -> None:
    """Plan section 8 item 6, the harder kind of failure: the read FINISHED (so the module set its state) and the load
    failed. dlt state is committed with the load and the sink drops the pending package, so the next run asks Graph
    for the same changes again and none is lost."""
    run(lab, graph)
    graph.put(2, 40, Title="item 2 changed")
    graph.put(6, 41)
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
        run(lab, graph)
    monkeypatch.setattr(sink_module, "_bronze_pipeline", real)
    assert {r["id"] for r in lab.rows("sp_tasks")} == {"1", "2", "3", "4", "5"}
    graph.log.clear()
    run(lab, graph)
    rows = {r["id"]: r for r in lab.rows("sp_tasks")}
    assert sorted(rows) == ["1", "2", "3", "4", "5", "6"] and rows["2"]["field_title"] == "item 2 changed", "the changes of the dead run arrive"
    assert any("token=v5" in line for line in graph.log), "the run asked from the link of the last COMMITTED load"


@pytest.mark.loader
def test_a_gone_link_in_a_real_run_reloads_every_item_and_continues(lab: Lab, graph: Graph) -> None:
    run(lab, graph)
    graph.put(2, 40, Title="item 2 changed")
    graph.expired = True
    run(lab, graph)
    rows = {r["id"]: r for r in lab.rows("sp_tasks")}
    assert len(rows) == 5 and rows["2"]["field_title"] == "item 2 changed"
    graph.expired = False
    graph.put(7, 50)
    run(lab, graph)
    assert len(lab.rows("sp_tasks")) == 6


def test_the_session_carries_no_request_header_the_vendor_did_not_ask_for(graph: Graph) -> None:
    read(graph, {})
    assert {k for h in graph.headers for k in h} == {"Authorization"}
