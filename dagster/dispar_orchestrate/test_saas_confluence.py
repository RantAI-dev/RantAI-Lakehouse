"""`saas/vendors/confluence.py` (`SRC-3` slice 4, V-Confluence.1 to .3): reading Confluence's REST
API v2 through the real `SaasSession` over a FAKE Confluence (cursor paging by `_links.next`, the sort by
`-modified-date`, the gateway an Atlassian browser token needs, accessible-resources). No network.
The loader test runs `load_via_sink` for real on `test_load_modes_measurement.Lab`; CI runs it in
`dagster-loader-tests`."""

from __future__ import annotations

import base64
from datetime import datetime, timedelta, timezone
from typing import Any
from urllib.parse import parse_qs, urlparse

import pytest

from dispar_orchestrate import schema_observer, ssrf_guard
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import session as saas_session
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import confluence
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_zendesk import Reply, resolver

SITE = "https://acme.atlassian.net"
GATEWAY = "https://api.atlassian.com"
CLOUD_ID = "0f1e2d3c-4b5a-4968-8776-655443322110"
DIAL = {"vendor": "confluence", "account": "acme", "auth": {"type": "integration"}}
OAUTH_DIAL = {"vendor": "confluence", "account": "acme", "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}
SECRETS = {"email": "agent@example.invalid", "apiToken": "not-a-real-token"}
OAUTH_SECRETS = {"clientSecret": "client-secret", "refreshToken": "refresh-1"}
BASE = datetime(2026, 10, 1, 10, 0, 0, tzinfo=timezone.utc)


def stamp(days: float) -> str:
    return (BASE + timedelta(days=days)).strftime("%Y-%m-%dT%H:%M:%S.000Z")


def page(i: int, days: float, **extra: Any) -> dict[str, Any]:
    return {
        "id": str(1000 + i), "status": "current", "title": f"page {i}", "spaceId": "9", "authorId": "u1", "createdAt": stamp(0),
        "version": {"number": 1, "authorId": "u2", "createdAt": stamp(days)},
        "body": {"storage": {"representation": "storage", "value": f"<p>page {i}</p>"}},
        "_links": {"webui": f"/spaces/X/pages/{1000 + i}"}, **extra,
    }  # fmt: skip


class Confluence:
    """The v2 list endpoints as documented. `gateway=True` models the OAuth gateway: the paths carry
    `/ex/confluence/<cloud id>` and a relative `_links.next` does NOT (`next_has_prefix` flips it)."""

    def __init__(self, cap: int = 2) -> None:
        self.cap = cap
        self.store: dict[str, list[dict[str, Any]]] = {
            "pages": [], "blogposts": [], "spaces": [{"id": "9", "key": "X", "name": "Space X", "status": "current", "icon": {"path": "/x"}}],
            "footer-comments": [], "inline-comments": [],
        }
        self.log: list[str] = []
        self.headers: list[dict] = []
        self.sites: list[dict[str, Any]] = [{"id": CLOUD_ID, "url": SITE, "name": "acme", "scopes": ["read:page:confluence", "read:space:confluence"]}]
        self.next_has_prefix = False
        self.link_to: str | None = None
        self.status: dict[str, int] = {}

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        self.headers.append(headers)
        parsed = urlparse(url)
        query = {k: v[0] for k, v in parse_qs(parsed.query).items()} | {k: str(v) for k, v in (params or {}).items()}
        self.log.append(f"{parsed.netloc}{parsed.path}?{'&'.join(f'{k}={v}' for k, v in sorted(query.items()) if k != 'cursor')}{'&cursor=' + query['cursor'] if 'cursor' in query else ''}")
        if parsed.path == "/oauth/token/accessible-resources":
            return Reply(200, self.sites)
        path, prefix = parsed.path, ""
        if parsed.netloc == "api.atlassian.com":
            prefix = f"/ex/confluence/{CLOUD_ID}"
            if not path.startswith(prefix + "/"):
                return Reply(404, {})
            path = path[len(prefix) :]
        resource = path.removeprefix("/wiki/api/v2/")
        if resource in self.status:
            return Reply(self.status[resource], {"message": "STUB-VENDOR-WORDS"})
        if resource not in self.store:
            return Reply(404, {"message": "STUB-VENDOR-WORDS"})
        rows = list(self.store[resource])
        if query.get("sort") == "-modified-date":
            rows.sort(key=lambda r: r["version"]["createdAt"], reverse=True)
        start = int(query.get("cursor", "0"))
        chunk = rows[start : start + min(int(query.get("limit", self.cap)), self.cap)]
        body: dict[str, Any] = {"results": chunk, "_links": {"base": SITE + "/wiki"}}
        if start + len(chunk) < len(rows):
            rel = f"/wiki/api/v2/{resource}?limit={query.get('limit')}&sort={query.get('sort', '')}&cursor={start + len(chunk)}"
            body["_links"]["next"] = (self.link_to or "") + (prefix + rel if self.next_has_prefix and not self.link_to else rel)
        return Reply(200, body)


@pytest.fixture
def api() -> Confluence:
    return Confluence()


TOKEN_POSTS: list[dict] = []


@pytest.fixture(autouse=True)
def token_endpoint(monkeypatch: pytest.MonkeyPatch) -> None:
    """Atlassian's `/oauth/token`: JSON in; every refresh issues a new refresh token."""
    TOKEN_POSTS.clear()

    def post(url: str, **kw: Any) -> Reply:
        TOKEN_POSTS.append({"url": url, "json": kw.get("json"), "data": kw.get("data")})
        body = kw.get("json") or {}
        if body.get("refresh_token") != "refresh-1":
            return Reply(403, {"error": "invalid_grant", "error_description": "STUB-VENDOR-WORDS"})
        return Reply(200, {"access_token": "gateway-access", "refresh_token": "refresh-2", "expires_in": 3600})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)


def open_(api: Confluence, dial: dict = DIAL, secrets: dict = SECRETS, **kw: Any):  # noqa: ANN202
    return saas_adapter.open_session(dial, secrets, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None, **kw)


def read(api: Confluence, name: str, since: Any = None, dial: dict = DIAL, secrets: dict = SECRETS) -> list[dict]:
    module, _vendor, parts, session = open_(api, dial, secrets)
    return list(module.read(session, parts, dial, module.declaration(session, parts, dial, name), since=since))


def test_the_vendor_module_is_found_by_the_registry() -> None:
    assert vendors.module_for("confluence") is confluence


def test_the_email_and_api_token_are_basic_with_the_plain_email_at_the_site(api: Confluence) -> None:
    api.store["pages"] = [page(1, 1)]
    got = read(api, "pages")
    assert [r["id"] for r in got] == ["1001"]
    expected = "Basic " + base64.b64encode(b"agent@example.invalid:not-a-real-token").decode()
    assert api.headers[0]["Authorization"] == expected and api.log[0].startswith("acme.atlassian.net/wiki/api/v2/pages?")


def test_a_first_run_reads_newest_first_across_the_cursor_pages_and_flattens_the_records(api: Confluence) -> None:
    api.store["pages"] = [page(i, i) for i in range(1, 6)]
    got = read(api, "pages")
    assert [r["id"] for r in got] == ["1005", "1004", "1003", "1002", "1001"]
    first = got[0]
    assert first["modified_at"] == BASE + timedelta(days=5) and first["version_number"] == 1 and first["body"] == "<p>page 5</p>"
    assert not any(isinstance(v, (dict, list)) for v in first.values()), "nested values are flattened"
    assert "sort=-modified-date" in api.log[0] and "body-format=storage" in api.log[0]


def test_a_later_run_stops_at_the_first_record_older_than_the_cursor_and_asks_for_no_more_pages(api: Confluence) -> None:
    api.store["pages"] = [page(i, i) for i in range(1, 8)]
    got = read(api, "pages", since=BASE + timedelta(days=6))
    assert [r["id"] for r in got] == ["1007", "1006"] and len(api.log) == 2, "pages of two: the second holds the first old record"
    api.log.clear()
    assert [r["id"] for r in read(api, "pages", since=(BASE + timedelta(days=7)).isoformat())] == ["1007"]
    assert len(api.log) == 1, "the stop is in the first page; no second request"


def test_spaces_are_read_in_full_and_say_why(api: Confluence) -> None:
    module, _v, parts, session = open_(api)
    decl = module.declaration(session, parts, DIAL, "spaces")
    assert decl.load_description() == "re-read in full: Confluence cannot sort or filter spaces by change time"
    got = list(module.read(session, parts, DIAL, decl, since=BASE))
    assert got == [{"id": "9", "key": "X", "name": "Space X", "status": "current"}]
    assert "sort" not in api.log[0]


def test_the_comments_of_both_kinds_are_one_object_told_apart_by_their_type(api: Confluence) -> None:
    api.store["footer-comments"] = [{"id": "5", "status": "current", "pageId": "1001", "version": {"number": 1, "createdAt": stamp(1)}, "body": {"storage": {"value": "f"}}}]
    api.store["inline-comments"] = [{"id": "6", "status": "current", "pageId": "1001", "version": {"number": 1, "createdAt": stamp(2)}, "body": {"storage": {"value": "i"}}}]
    got = read(api, "comments")
    assert {(r["id"], r["comment_type"], r["body"]) for r in got} == {("5", "footer", "f"), ("6", "inline", "i")}


def test_the_four_objects_are_listed_and_blog_posts_and_comments_are_incremental(api: Confluence) -> None:
    module, _v, parts, session = open_(api)
    listed = {d.name: d.load_description() for d in module.list_objects(session, parts, DIAL)}
    assert listed["pages"] == listed["blogposts"] == listed["comments"] == "incremental"
    assert listed["spaces"].startswith("re-read in full: ") and set(listed) == {"spaces", "pages", "blogposts", "comments"}
    assert not any(d.deletes for d in module.list_objects(session, parts, DIAL))
    with pytest.raises(saas_adapter.SaasSetupRefused):
        module.declaration(session, parts, DIAL, "attachments")


def test_a_next_link_to_another_host_stops_the_run(api: Confluence) -> None:
    api.store["pages"] = [page(i, i) for i in range(1, 6)]
    api.link_to = "https://evil.example.invalid"
    with pytest.raises(ssrf_guard.SsrfBlocked) as err:
        read(api, "pages")
    assert "evil" not in str(err.value)


def test_the_access_failures_have_their_own_fixed_classes(api: Confluence) -> None:
    for status, expected in ((401, saas_session.SaasSignInRefused), (403, saas_session.SaasPermissionMissing)):
        api.status["pages"] = status
        with pytest.raises(expected) as err:
            read(api, "pages")
        assert "STUB-VENDOR-WORDS" not in str(err.value)


# ---- the browser sign-in: the gateway and the cloud id ----------------------------------------


def test_the_browser_sign_in_reads_behind_the_gateway_with_the_cloud_id_of_the_connectors_site(api: Confluence) -> None:
    api.store["pages"] = [page(i, i) for i in range(1, 4)]
    api.sites = [
        {"id": "ffffffff-0000-4000-8000-000000000000", "url": "https://other.atlassian.net", "scopes": ["read:page:confluence"]},
        {"id": "eeeeeeee-0000-4000-8000-000000000000", "url": SITE, "scopes": ["read:jira-work"]},  # a Jira-only grant
        {"id": CLOUD_ID, "url": SITE + "/", "scopes": ["read:page:confluence"]},
    ]
    got = read(api, "pages", dial=OAUTH_DIAL, secrets=OAUTH_SECRETS)
    assert [r["id"] for r in got] == ["1003", "1002", "1001"]
    assert api.log[0] == "api.atlassian.com/oauth/token/accessible-resources?"
    assert all(line.startswith(f"api.atlassian.com/ex/confluence/{CLOUD_ID}/wiki/api/v2/pages?") for line in api.log[1:])
    assert {h["Authorization"] for h in api.headers} == {"Bearer gateway-access"}


@pytest.mark.parametrize("has_prefix", [False, True])
def test_a_relative_next_link_is_followed_with_or_without_the_gateway_prefix_it_may_lack(api: Confluence, has_prefix: bool) -> None:
    api.next_has_prefix = has_prefix
    api.store["pages"] = [page(i, i) for i in range(1, 6)]
    assert len(read(api, "pages", dial=OAUTH_DIAL, secrets=OAUTH_SECRETS)) == 5
    assert all("/ex/confluence/" + CLOUD_ID + "/ex/" not in line for line in api.log), "the prefix is never doubled"
    assert len([line for line in api.log if "/pages?" in line]) == 3


def test_the_token_request_is_json_with_the_client_in_the_body_and_the_rotated_refresh_token_is_reported(api: Confluence) -> None:
    reported: list[str] = []
    open_(api, OAUTH_DIAL, OAUTH_SECRETS, report_refresh_token=reported.append)[3].get_json("/oauth/token/accessible-resources")
    assert TOKEN_POSTS[0]["url"] == "https://auth.atlassian.com/oauth/token" and TOKEN_POSTS[0]["data"] is None
    assert TOKEN_POSTS[0]["json"] == {"grant_type": "refresh_token", "refresh_token": "refresh-1", "client_id": "app", "client_secret": "client-secret"}
    assert reported == ["refresh-2"]


def test_a_site_the_account_cannot_reach_and_a_cloud_id_that_is_not_a_uuid_are_rejected_settings(api: Confluence) -> None:
    api.sites = [{"id": CLOUD_ID, "url": "https://other.atlassian.net", "scopes": ["read:page:confluence"]}]
    with pytest.raises(saas_adapter.SaasSetupRefused):
        open_(api, OAUTH_DIAL, OAUTH_SECRETS)
    for bad in ("../../x", CLOUD_ID.upper(), CLOUD_ID + "/x", ""):
        api.sites = [{"id": bad, "url": SITE, "scopes": ["read:page:confluence"]}]
        with pytest.raises(saas_adapter.SaasSetupRefused):
            open_(api, OAUTH_DIAL, OAUTH_SECRETS)


def test_a_refused_refresh_is_the_fixed_sign_in_failure(api: Confluence) -> None:
    from dispar_orchestrate.adapters import rest as rest_adapter

    with pytest.raises(rest_adapter.OAuthSignInRefused) as err:
        open_(api, OAUTH_DIAL, {**OAUTH_SECRETS, "refreshToken": "retired"})
    assert "STUB-VENDOR-WORDS" not in str(err.value)


def test_the_vendor_row_matches_atlassians_documentation() -> None:
    row_ = vendors.vendor("confluence")
    assert row_.oauth["authorizeUrl"] == "https://auth.atlassian.com/authorize?audience=api.atlassian.com&prompt=consent"
    assert row_.oauth["tokenUrl"] == "https://auth.atlassian.com/oauth/token" and row_.oauth["clientAuth"] == "post_json"
    assert row_.oauth["apiBase"] == "https://api.atlassian.com/ex/confluence/{cloudId}" and row_.oauth_api_origin() == GATEWAY
    assert "read:blogpost:confluence" not in row_.oauth["scopes"] and "offline_access" in row_.oauth["scopes"]
    assert {s for s in row_.oauth["scopes"]} >= {"read:page:confluence", "read:space:confluence", "read:comment:confluence"}
    assert [v.id for v in vendors.VENDORS.values() if v.oauth_api_origin()] == ["confluence"]


# ---- the whole path ---------------------------------------------------------------------------


def accept_all(table: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj() -> dict[str, Any]:
    return {"name": "pages", "target": "cf_pages", "loadMode": "incremental_dedup", "incrementalKey": "modified_at", "primaryKey": ["id"]}


@pytest.mark.loader
def test_a_second_run_merges_the_changed_page_and_stores_none_twice(lab: Lab, api: Confluence) -> None:
    api.store["pages"] = [page(i, i) for i in range(1, 6)]

    def run() -> Any:
        built = saas_adapter.build_source(DIAL, SECRETS, [obj()], gate=accept_all, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)
        assert built is not None
        return load_via_sink(built.source, "cf_pages", lab.config, f._load_plan("saas", obj()))

    assert run().rows == 5
    api.store["pages"][1] = page(2, 9, title="changed")
    api.store["pages"].append(page(6, 10))
    api.log.clear()
    run()
    rows = {r["id"]: r["title"] for r in lab.rows("cf_pages")}
    assert len(rows) == 6 and rows["1002"] == "changed", "merged on id: no duplicate"
    assert len(api.log) <= 3, "the read stopped near the cursor instead of walking every page"
