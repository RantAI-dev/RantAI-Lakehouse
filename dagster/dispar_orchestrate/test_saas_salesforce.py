"""`saas/vendors/salesforce.py` (`SRC-3` V-Salesforce.2 and .3): objects listed by the
org (custom ones included), how each loads, deletes through `queryAll`, the schema
before a load, and the browser sign-in with the host Salesforce returns adopted only
through the `.salesforce.com` / `.force.com` rule. Through the real `SaasSession` over
a FAKE Salesforce (the REST calls of `test_saas_vendored_salesforce.FakeSalesforce`).
No network. The loader test runs `load_via_sink` for real on
`test_load_modes_measurement.Lab`; CI runs it in `dagster-loader-tests`."""

from __future__ import annotations

from typing import Any
from urllib.parse import urlparse

import pytest

from dispar_orchestrate import ingest_errors, schema_observer, ssrf_guard
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import salesforce
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_salesforce import ORIGIN, V, FakeSalesforce, account, field
from dispar_orchestrate.test_saas_vendored_zendesk import Reply, resolver

DIAL = {"vendor": "salesforce", "account": "acme", "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}
SECRETS = {"clientSecret": "not-a-real-secret", "refreshToken": "not-a-real-refresh-token"}
TOKEN_URL = "https://acme.my.salesforce.com/services/oauth2/token"


class Salesforce(FakeSalesforce):
    """The fake plus describe global, a second sObject without `SystemModstamp`, and failure switches."""

    def __init__(self, *args: Any, **kw: Any) -> None:
        super().__init__(*args, **kw)
        self.fail: dict[str, int] = {}

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        path = urlparse(url).path
        if path in self.fail:
            self.headers.append(headers)
            return Reply(self.fail[path], [{"message": "STUB-VENDOR-WORDS", "errorCode": "X"}])
        if path == f"/services/data/v{V}/sobjects":
            self.headers.append(headers)
            return Reply(body={"sobjects": [
                {"name": "Account", "queryable": True, "replicateable": True},
                {"name": "Invoice__c", "queryable": True, "replicateable": True, "custom": True},
                {"name": "AccountHistoryLookup", "queryable": True, "replicateable": False},
                {"name": "Hidden__c", "queryable": True, "replicateable": True, "deprecatedAndHidden": True},
                {"name": "Event_Log", "queryable": False, "replicateable": False},
                {"name": "Bad-Name", "queryable": True, "replicateable": True},
            ]})  # fmt: skip
        if path == f"/services/data/v{V}/sobjects/Nothing__c/describe":
            return Reply(404, [{"message": "STUB-VENDOR-WORDS", "errorCode": "NOT_FOUND"}])
        if path == f"/services/data/v{V}/sobjects/User/describe":
            self.headers.append(headers)
            return Reply(body={"name": "User", "fields": [field("Id", "id"), field("Name", "string", "Name"), field("SystemModstamp", "datetime")]})
        if path == f"/services/data/v{V}/sobjects/Plain__c/describe":
            return Reply(body={"name": "Plain__c", "fields": [field("Id", "id"), field("Note__c")]})
        return super().get(url, headers=headers, params=params, timeout=timeout)


@pytest.fixture
def api() -> Salesforce:
    return Salesforce([], page_size=2)


def token_reply(instance_url: str | None = ORIGIN) -> Reply:
    body: dict[str, Any] = {"access_token": "a1", "token_type": "Bearer", "expires_in": 7200}
    if instance_url is not None:
        body["instance_url"] = instance_url
    return Reply(200, body)


@pytest.fixture
def sign_in(monkeypatch: pytest.MonkeyPatch) -> list[dict]:
    """Refresh-token grants answered with an access token and an `instance_url`."""
    sent: list[dict] = []

    def post(url: str, **kw: Any) -> Reply:
        sent.append({"url": url, **kw})
        return token_reply()

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)
    return sent


def open_(api: Salesforce, dial: dict = DIAL):  # noqa: ANN202
    return saas_adapter.open_session(dial, SECRETS, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)


def read(api: Salesforce, name: str, since: Any = None) -> list[dict]:
    module, _vendor, parts, session = open_(api)
    return list(module.read(session, parts, DIAL, module.declaration(session, parts, DIAL, name), since=since))


def test_the_vendor_module_is_found_by_the_registry() -> None:
    assert vendors.module_for("salesforce") is salesforce


def test_the_objects_are_the_orgs_queryable_ones_custom_objects_included(api: Salesforce, sign_in: list) -> None:
    _m, _v, parts, session = open_(api)
    declared = {d.name: d for d in salesforce.list_objects(session, parts, DIAL)}
    assert set(declared) == {"Account", "Invoice__c", "AccountHistoryLookup"}, "queryable, not hidden, plain API names"
    assert declared["Invoice__c"].load_description() == "incremental"
    assert declared["AccountHistoryLookup"].load_description() == "re-read in full: Salesforce keeps no SystemModstamp on AccountHistoryLookup"
    assert all(d.primary_key == ("Id",) for d in declared.values())


def test_an_object_with_a_modstamp_and_is_deleted_is_incremental_and_reports_deletes_and_one_without_is_whole(api: Salesforce, sign_in: list) -> None:
    _m, _v, parts, session = open_(api)
    account_decl = salesforce.declaration(session, parts, DIAL, "Account")
    assert (account_decl.cursor, account_decl.deletes) == ("SystemModstamp", True)
    user = salesforce.declaration(session, parts, DIAL, "User")
    assert (user.cursor, user.deletes) == ("SystemModstamp", False), "no IsDeleted: no queryAll, no delete claimed"
    plain = salesforce.declaration(session, parts, DIAL, "Plain__c")
    assert plain.cursor is None and plain.load_description().startswith("re-read in full: ")


def test_an_unknown_object_or_a_name_that_is_not_an_api_name_is_a_rejected_setting_never_echoed(api: Salesforce, sign_in: list) -> None:
    _m, _v, parts, session = open_(api)
    for name in ("Nothing__c", "../../etc/passwd", "Account; DROP", ""):
        with pytest.raises(saas_adapter.SaasSetupRefused) as raised:
            salesforce.declaration(session, parts, DIAL, name)
        assert "passwd" not in str(raised.value) and "STUB" not in str(raised.value)


def test_describe_answers_the_fields_a_load_selects_with_dlt_types_before_the_load(api: Salesforce, sign_in: list) -> None:
    _m, _v, parts, session = open_(api)
    fields = {x.name: x for x in salesforce.describe(session, parts, DIAL, "Account")}
    assert "Body" not in fields and "BillingAddress" not in fields, "the list the load selects"
    assert fields["IsDeleted"].type_name == "bool" and fields["SystemModstamp"].type_name == "timestamp" and fields["Industry__c"].type_name == "text"
    assert fields["Id"].nullable is False


def test_a_first_run_reads_the_recycle_bin_too_and_a_later_run_only_what_changed_since(api: Salesforce, sign_in: list) -> None:
    api.rows = [account(1, "2024-05-01T10:00:00.000+0000"), account(2, "2024-05-02T10:00:00.000+0000", IsDeleted=True), account(3, "2024-05-03T10:00:00.000+0000")]
    first = read(api, "Account")
    assert {r["Id"]: r["IsDeleted"] for r in first} == {"001001": False, "001002": True, "001003": False}
    assert api.queries[-1].count("WHERE") == 0
    api.queries.clear()
    again = read(api, "Account", since="2024-05-02T10:00:00.000000Z")
    assert {r["Id"] for r in again} == {"001002", "001003"}, "the cursor's own record (merge removes it), the newer, the deleted"
    assert "WHERE SystemModstamp >= 2024-05-02T10:00:00.000Z" in api.queries[0]


@pytest.mark.parametrize("status,error,klass", [(401, "SaasSignInRefused", "sign_in_refused"), (403, "SaasPermissionMissing", "permission_missing")])
def test_a_refusal_is_a_fixed_failure_without_salesforces_words(api: Salesforce, sign_in: list, status: int, error: str, klass: str) -> None:
    api.fail[f"/services/data/v{V}/sobjects"] = status
    _m, _v, parts, session = open_(api)
    with pytest.raises(Exception) as raised:
        salesforce.list_objects(session, parts, DIAL)
    assert type(raised.value).__name__ == error and "STUB-VENDOR-WORDS" not in str(raised.value)
    assert ingest_errors.classify(raised.value).error_class == klass


def test_a_used_up_api_quota_is_rate_limited_and_not_retried_but_another_403_is_still_a_missing_permission(api: Salesforce, sign_in: list) -> None:
    """Slice 3 review BLOCKER 8 (decision 13): Salesforce answers its request limit with HTTP 403
    `REQUEST_LIMIT_EXCEEDED`, not 429."""
    path = f"/services/data/v{V}/sobjects"
    api.fail[path] = 403
    original = api.get

    def quota(url: str, **kw: Any) -> Reply:
        if urlparse(url).path == path:
            return Reply(403, [{"message": "TotalRequests Limit exceeded. STUB-VENDOR-WORDS", "errorCode": "REQUEST_LIMIT_EXCEEDED"}])
        return original(url, **kw)

    api.get = quota  # type: ignore[method-assign]
    _m, _v, parts, session = open_(api)
    with pytest.raises(Exception) as raised:
        salesforce.list_objects(session, parts, DIAL)
    assert type(raised.value).__name__ == "SaasQuotaExhausted" and "STUB" not in str(raised.value)
    classified = ingest_errors.classify(raised.value)
    assert (classified.error_class, classified.retried) == ("rate_limited", False), "the quota is per 24 hours: not retried within the run"
    api.get = original  # type: ignore[method-assign]
    _m, _v, parts, session = open_(api)
    with pytest.raises(Exception) as other:
        salesforce.list_objects(session, parts, DIAL)
    assert type(other.value).__name__ == "SaasPermissionMissing"


def test_the_quota_check_reads_only_the_error_code() -> None:
    assert salesforce.is_quota_exhausted([{"errorCode": "REQUEST_LIMIT_EXCEEDED", "message": "x"}])
    for body in ([], {}, "REQUEST_LIMIT_EXCEEDED", [{"errorCode": "INSUFFICIENT_ACCESS"}], [{"message": "REQUEST_LIMIT_EXCEEDED"}], None):
        assert not salesforce.is_quota_exhausted(body)


# ---- V-Salesforce.3: sign-in ---------------------------------------------------------------------


def test_the_browser_sign_in_refreshes_at_the_tables_address_with_the_client_in_the_form_body(api: Salesforce, sign_in: list) -> None:
    api.rows = [account(1)]
    read(api, "Account")
    call = sign_in[0]
    assert call["url"] == TOKEN_URL
    assert call["data"] == {"grant_type": "refresh_token", "refresh_token": "not-a-real-refresh-token", "client_id": "app", "client_secret": "not-a-real-secret"}
    assert {h["Authorization"] for h in api.headers} == {"Bearer a1"}


def test_the_returned_instance_url_is_adopted_and_the_reads_go_to_it(monkeypatch: pytest.MonkeyPatch, api: Salesforce) -> None:
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: token_reply("https://acme-pod.my.salesforce.com"))
    _m, _v, _p, session = open_(api)
    assert session.origin == "https://acme-pod.my.salesforce.com"
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: token_reply("https://acme.example.force.com"))
    assert open_(api)[3].origin == "https://acme.example.force.com"


@pytest.mark.parametrize(
    "returned",
    [
        "https://evil.example.invalid",
        "https://salesforce.com.evil.example.invalid",
        "https://evilsalesforce.com",
        "https://salesforce.com",
        "http://acme.my.salesforce.com",
        "https://acme.my.salesforce.com:8443",
        "https://user@acme.my.salesforce.com",
    ],
)
def test_an_instance_url_outside_the_salesforce_rule_stops_the_run_before_any_read(monkeypatch: pytest.MonkeyPatch, api: Salesforce, returned: str) -> None:
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: token_reply(returned))
    with pytest.raises(ssrf_guard.SsrfBlocked) as raised:
        open_(api)
    assert "evil" not in str(raised.value)
    assert api.log == [], "nothing was asked of the host Salesforce named"


def test_a_sign_in_without_an_instance_url_keeps_the_table_address(monkeypatch: pytest.MonkeyPatch, api: Salesforce) -> None:
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: token_reply(None))
    assert open_(api)[3].origin == ORIGIN


def test_salesforce_offers_no_integration_credential_and_no_password_sign_in(api: Salesforce) -> None:
    """Decision 4 and the page's decision 3: browser sign-in only."""
    assert vendors.vendor("salesforce").integration is None
    with pytest.raises(saas_adapter.SaasSetupRefused):
        saas_adapter.open_session({"vendor": "salesforce", "account": "acme", "auth": {"type": "integration"}}, {"token": "x"}, resolve_checked=resolver)
    assert vendors.auth_type_of({"vendor": "salesforce", "auth": {"type": "integration"}}) is None


def test_a_revoked_browser_sign_in_is_the_fixed_failure(monkeypatch: pytest.MonkeyPatch, api: Salesforce) -> None:
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: Reply(400, {"error": "invalid_grant", "error_description": "STUB-VENDOR-WORDS"}))
    with pytest.raises(Exception) as raised:
        open_(api)
    assert ingest_errors.classify(raised.value).error_class == "sign_in_refused" and "STUB" not in str(raised.value)
    assert api.log == []


def test_the_instance_url_is_adopted_under_the_gate_switch_at_the_mock_but_only_after_the_rule(monkeypatch: pytest.MonkeyPatch, api: Salesforce) -> None:
    monkeypatch.setenv("CONNECTOR_OAUTH_ALLOW_HTTP", "true")
    monkeypatch.setenv("SAAS_GATE_ORIGINS", '{"salesforce": "http://mock.invalid:8103"}')
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: token_reply("https://acme.my.salesforce.com"))
    assert open_(api)[3].origin == "http://mock.invalid:8103"
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: token_reply("https://evil.example.invalid"))
    with pytest.raises(ssrf_guard.SsrfBlocked):
        open_(api)


def test_the_vendor_row_is_what_the_documentation_this_build_could_read_says() -> None:
    """V-Salesforce.3: the facts a test can pin. Salesforce's own documentation pages were not
    readable from the build machine (see the commit body); the paths are the REST calls
    `simple-salesforce` 1.12.10 makes and Salesforce's published OAuth endpoints."""
    row = vendors.vendor("salesforce")
    assert row.oauth["authorizeUrl"] == "https://{account}.my.salesforce.com/services/oauth2/authorize"
    assert row.oauth["tokenUrl"] == "https://{account}.my.salesforce.com/services/oauth2/token"
    assert row.oauth["revocationUrl"] == "https://{account}.my.salesforce.com/services/oauth2/revoke"
    assert row.oauth["scopes"] == ["api", "refresh_token"]
    assert row.oauth["clientAuth"] == "post"
    assert row.whoami == {"method": "GET", "path": f"/services/data/v{V}/limits"}
    assert row.api_url(row.validate_account("Acme")) == "https://acme.my.salesforce.com"


# ---- the whole path: adapter, dlt incremental, merge on Id, real Iceberg -------------------------


def accept_all(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj() -> dict:
    return {"name": "Account", "target": "sf_account", "loadMode": "incremental_dedup", "incrementalKey": "SystemModstamp", "primaryKey": ["Id"]}


@pytest.mark.loader
def test_a_second_run_merges_the_changed_account_marks_the_deleted_one_and_stores_none_twice(
    lab: Lab, api: Salesforce, monkeypatch: pytest.MonkeyPatch  # noqa: F811
) -> None:
    import time

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: token_reply())
    api.rows = [account(i, f"2024-05-0{i}T10:00:00.000+0000") for i in range(1, 5)]

    def run() -> Any:
        built = saas_adapter.build_source(DIAL, SECRETS, [obj()], gate=accept_all, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)
        assert built is not None
        return load_via_sink(built.source, "sf_account", lab.config, f._load_plan("saas", obj()))

    assert run().rows == 4
    assert sorted(r["id"] for r in lab.rows("sf_account")) == ["001001", "001002", "001003", "001004"]
    time.sleep(1.1)
    api.rows[1] = account(2, "2024-05-09T10:00:00.000+0000", Name="changed", IsDeleted=True)
    api.rows.append(account(5, "2024-05-10T10:00:00.000+0000"))
    run()
    rows = {r["id"]: (r["name"], r["is_deleted"]) for r in lab.rows("sf_account")}
    assert rows["001002"] == ("changed", True) and "001005" in rows and len(lab.rows("sf_account")) == 5, "merged on Id: no duplicate"
