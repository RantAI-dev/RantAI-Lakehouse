"""`saas/vendors/workday.py` (`SRC-3` slice 6, V-Workday.1 to .3): reading a Reports-as-a-Service
report through the real `SaasSession` over a FAKE Workday (the report address with the tenant, the
single `Report_Entry` response, Basic authentication). No network. The loader test runs
`load_via_sink` for real on `test_load_modes_measurement.Lab`; CI runs it in `dagster-loader-tests`.

The shape of the answer (`Report_Entry`) and the OAuth token request are the developer's model of
Workday's documentation, which is behind a login (plan section 11, handoff slice 6): the fake is
that model, not a recording."""

from __future__ import annotations

import base64
from typing import Any
from urllib.parse import urlparse

import pytest

from dispar_orchestrate import schema_observer
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import workday
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_zendesk import Reply, resolver

HOST = "wd2-impl-services1.workday.com"
ORIGIN = f"https://{HOST}"
ACCOUNT = f"{HOST}/acme_tenant"
REPORT = "isu_owner/Headcount_Report"
DIAL = {"vendor": "workday", "account": ACCOUNT, "objects": [REPORT], "auth": {"type": "integration"}}
SECRETS = {"username": "isu_gate@acme_tenant", "password": "not-a-real-password"}
BASIC = "Basic " + base64.b64encode(b"isu_gate@acme_tenant:not-a-real-password").decode()


class Workday:
    """RaaS as modelled: `GET /ccx/service/customreport2/<tenant>/<owner>/<report>?format=json`
    answers once with every row under `Report_Entry`; Basic authentication."""

    def __init__(self) -> None:
        self.reports: dict[str, list[dict[str, Any]]] = {REPORT: []}
        self.log: list[str] = []
        self.headers: list[dict] = []
        self.status: int | None = None
        self.body: Any = None

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        parsed = urlparse(url)
        self.log.append(f"{parsed.path}?{(params or {}).get('format')}")
        self.headers.append(headers)
        self.timeouts = getattr(self, "timeouts", []) + [timeout]
        if self.status is not None:
            return Reply(self.status, {"error": "STUB-VENDOR-WORDS"})
        if headers.get("Authorization") != BASIC:
            return Reply(401, {"error": "STUB-VENDOR-WORDS"})
        prefix = "/ccx/service/customreport2/acme_tenant/"
        name = parsed.path.removeprefix(prefix) if parsed.path.startswith(prefix) else None
        if name not in self.reports:
            return Reply(404, {"error": "no such report STUB-VENDOR-WORDS"})
        return Reply(200, self.body if self.body is not None else {"Report_Entry": self.reports[name]})


@pytest.fixture
def api() -> Workday:
    return Workday()


def open_(api: Workday, dial: dict = DIAL, secrets: dict = SECRETS):  # noqa: ANN202
    return saas_adapter.open_session(dial, secrets, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)


def read(api: Workday, name: str = REPORT) -> list[dict]:
    module, _vendor, parts, session = open_(api)
    return list(module.read(session, parts, DIAL, module.declaration(session, parts, DIAL, name), since=None))


def test_the_vendor_module_is_found_by_the_registry() -> None:
    assert vendors.module_for("workday") is workday


def test_a_report_is_read_once_at_the_tenants_address_with_basic_authentication(api: Workday) -> None:
    api.reports[REPORT] = [{"Employee_ID": "21001", "Name": "A"}, {"Employee_ID": "21002", "Name": "B"}]
    assert [r["Employee_ID"] for r in read(api)] == ["21001", "21002"]
    assert api.log == [f"/ccx/service/customreport2/acme_tenant/{REPORT}?json"]
    assert api.headers[0] == {"Authorization": BASIC}


def test_the_session_waits_longer_than_the_default_for_a_report_that_is_built_on_request(api: Workday) -> None:
    read(api)
    assert api.timeouts == [600]


def test_an_empty_report_is_zero_rows_and_an_answer_without_report_entry_is_refused(api: Workday) -> None:
    assert read(api) == []
    api.body = {"unexpected": []}
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(api)
    api.body = ["not", "an", "object"]
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(api)


def test_a_report_is_always_re_read_in_full_with_the_reason_and_has_no_key(api: Workday) -> None:
    module, _v, parts, session = open_(api)
    decl = module.declaration(session, parts, DIAL, REPORT)
    assert not decl.incremental and decl.primary_key == () and decl.cursor is None
    assert decl.load_description().startswith("re-read in full: Workday cannot filter a report")
    assert api.log == [], "declaring a report does not run it"


def test_the_reports_the_connector_names_are_its_objects(api: Workday) -> None:
    module, _v, parts, session = open_(api)
    dial = {**DIAL, "objects": [REPORT, "other/Second_Report"]}
    assert [d.name for d in module.list_objects(session, parts, dial)] == [REPORT, "other/Second_Report"]
    assert module.list_objects(session, parts, {**DIAL, "objects": []}) == []


@pytest.mark.parametrize(
    "name",
    ["", "Report", "/Report", "owner/", "owner/../x", "owner/a/b", "o wner/Report", "owner/Report name", "owner/Report?x=1", "owner/%2e%2e", "../owner/Report", "owner/" + "a" * 151],
)
def test_a_name_that_is_not_owner_slash_report_is_refused_before_any_request(api: Workday, name: str) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(api, name)
    assert api.log == []


def test_an_unknown_report_is_a_rejected_setting_that_does_not_repeat_the_vendor(api: Workday) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused) as err:
        read(api, "isu_owner/Nothing_Here")
    assert "STUB-VENDOR-WORDS" not in str(err.value)


def test_the_access_failures_have_their_own_fixed_classes(api: Workday) -> None:
    from dispar_orchestrate.saas import session as saas_session

    for status, expected in ((401, saas_session.SaasSignInRefused), (403, saas_session.SaasPermissionMissing)):
        api.status = status
        with pytest.raises(expected) as err:
            read(api)
        assert "STUB-VENDOR-WORDS" not in str(err.value)


def test_a_wrong_password_is_the_fixed_sign_in_failure(api: Workday) -> None:
    from dispar_orchestrate.saas import session as saas_session

    module, _v, parts, session = open_(api, secrets={**SECRETS, "password": "wrong"})
    with pytest.raises(saas_session.SaasSignInRefused):
        list(module.read(session, parts, DIAL, module.declaration(session, parts, DIAL, REPORT), since=None))


def test_a_load_with_the_replace_mode_replaces_the_table_each_run(api: Workday) -> None:
    api.reports[REPORT] = [{"Employee_ID": "1"}]
    built = saas_adapter.build_source(DIAL, SECRETS, [{"name": REPORT, "target": "wd", "loadMode": "replace"}], gate=accept_all, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)
    assert built is not None and not built.observed_before_load, "Workday does not describe its fields"
    assert list(built.source) == [{"Employee_ID": "1"}]


def test_an_incremental_load_of_a_report_is_refused(api: Workday) -> None:
    from dispar_orchestrate.adapters.sink import UnsupportedLoadMode

    with pytest.raises(UnsupportedLoadMode):
        saas_adapter.build_source(
            DIAL, SECRETS, [{"name": REPORT, "target": "wd", "loadMode": "incremental_dedup", "incrementalKey": "Updated", "primaryKey": ["Employee_ID"]}],
            gate=accept_all, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None,
        )


def accept_all(table: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


@pytest.mark.loader
def test_a_second_run_replaces_the_report_table_so_a_removed_row_is_gone(lab: Lab, api: Workday) -> None:
    api.reports[REPORT] = [{"Employee_ID": "1", "Name": "A"}, {"Employee_ID": "2", "Name": "B"}]
    spec = {"name": REPORT, "target": "wd_headcount", "loadMode": "replace"}

    def run() -> Any:
        built = saas_adapter.build_source(DIAL, SECRETS, [spec], gate=accept_all, resolve_checked=resolver, http_get=api.get, sleep=lambda _s: None)
        assert built is not None
        return load_via_sink(built.source, "wd_headcount", lab.config, f._load_plan("saas", spec))

    assert run().rows == 2
    api.reports[REPORT] = [{"Employee_ID": "2", "Name": "B changed"}]
    run()
    assert [(r["employee_id"], r["name"]) for r in lab.rows("wd_headcount")] == [("2", "B changed")]


# ---- sign-in: the vendor table's row (modelled, not verified against Workday's documentation) -----

TOKEN_POSTS: list[dict] = []


@pytest.fixture(autouse=True)
def token_endpoint(monkeypatch: pytest.MonkeyPatch) -> None:
    TOKEN_POSTS.clear()

    def post(url: str, **kw: Any) -> Reply:
        TOKEN_POSTS.append({"url": url, "data": kw.get("data"), "headers": kw.get("headers")})
        return Reply(200, {"access_token": "wd-access", "token_type": "Bearer", "expires_in": 3600})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)


def test_the_vendor_row_names_the_tenants_oauth_addresses_and_basic_client_authentication() -> None:
    row_ = vendors.vendor("workday")
    parts = row_.validate_account(ACCOUNT)
    assert row_.render(row_.oauth["authorizeUrl"], parts) == f"https://{HOST}/acme_tenant/authorize"
    assert row_.token_url(parts) == f"https://{HOST}/ccx/oauth2/acme_tenant/token"
    assert row_.oauth["clientAuth"] == "basic" and row_.oauth["revocationUrl"] is None
    assert row_.integration["secretFields"] == ["username", "password"]


def test_the_browser_sign_in_refreshes_at_the_tenants_token_address_with_the_client_in_the_header(api: Workday) -> None:
    api.reports[REPORT] = [{"Employee_ID": "1"}]
    dial = {**DIAL, "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}
    secrets = {"clientSecret": "not-a-real-secret", "refreshToken": "refresh-1"}
    module, _v, parts, session = saas_adapter.open_session(dial, secrets, resolve_checked=resolver, http_get=lambda url, *, headers, params, timeout: Reply(200, {"Report_Entry": [{"Employee_ID": "1"}]}), sleep=lambda _s: None)
    assert [r["Employee_ID"] for r in module.read(session, parts, dial, module.declaration(session, parts, dial, REPORT), since=None)] == ["1"]
    assert TOKEN_POSTS[0]["url"] == f"https://{HOST}/ccx/oauth2/acme_tenant/token"
    assert TOKEN_POSTS[0]["data"] == {"grant_type": "refresh_token", "refresh_token": "refresh-1"}, "the client is not in the body"
    assert TOKEN_POSTS[0]["headers"]["Authorization"].startswith("Basic ")
