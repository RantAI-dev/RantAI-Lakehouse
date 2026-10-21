"""`saas/vendors/ga4.py` (`SRC-3` V-GA4.2 and V-GA4.3): reports as objects, the date
dimension as the cursor, the schema from the property's metadata before a load, and the
two sign-ins. Through the real `SaasSession` over a FAKE GA4 Data API
(`test_saas_vendored_google_analytics.FakeGa4`). No network. The loader test runs
`load_via_sink` for real on `test_load_modes_measurement.Lab`; CI runs it in
`dagster-loader-tests`."""

from __future__ import annotations

import base64
import json
from types import SimpleNamespace
from typing import Any
from urllib.parse import parse_qs, urlparse

import pytest
from dlt.common import pendulum

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa

from dispar_orchestrate import ingest_errors, schema_observer
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import ga4
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_google_analytics import FakeGa4, Reply, resolver, session_over

REPORT = "traffic = country, date | sessions, purchaseRevenue"
DIAL = {"vendor": "ga4", "account": "123", "objects": [REPORT], "auth": {"type": "integration"}}
PARTS = SimpleNamespace(account="123")


def days_ago(n: int) -> str:
    return pendulum.yesterday().subtract(days=n).strftime("%Y%m%d")


def test_the_vendor_module_is_found_by_the_registry() -> None:
    assert vendors.module_for("ga4") is ga4


def test_a_report_is_written_as_label_dimensions_and_metrics_and_gets_the_date_when_it_has_no_time_dimension() -> None:
    assert ga4.parse_report("t = country | sessions") == ("t", ["country", "date"], ["sessions"])
    assert ga4.parse_report("m = yearMonth, country | sessions, customEvent:plan") == ("m", ["yearMonth", "country"], ["sessions", "customEvent:plan"])


@pytest.mark.parametrize(
    "text",
    ["", "country | sessions", "t = country sessions", "t = country |", "1t = country | sessions", "t = coun try | sessions", "t = country | sessions; DROP", "t = a, a | sessions",
     "t = ../x | sessions", "t = country | " + ",".join(f"m{i}" for i in range(30))],
)  # fmt: skip
def test_a_text_that_is_not_a_report_is_a_fixed_rejected_setting(text: str) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused) as raised:
        ga4.parse_report(text)
    assert "DROP" not in str(raised.value) and "../" not in str(raised.value)


def test_the_connectors_reports_are_its_objects_each_incremental_on_its_time_dimension_keyed_by_its_dimensions() -> None:
    (decl,) = ga4.list_objects(None, PARTS, DIAL)
    assert (decl.name, decl.cursor, decl.primary_key, decl.deletes, decl.lag_seconds) == (REPORT, "date", ("country", "date"), False, 3 * 86_400)
    assert decl.load_description() == "incremental"
    monthly = ga4.declaration(None, PARTS, DIAL, "m = yearMonth | sessions")
    assert monthly.cursor == "yearMonth" and monthly.primary_key == ("yearMonth",)


def test_describe_types_the_columns_from_the_properties_metadata_and_refuses_a_name_it_does_not_have() -> None:
    session = session_over(FakeGa4())
    fields = {x.name: x for x in ga4.describe(session, PARTS, DIAL, REPORT)}
    assert fields["date"].type_name == "timestamp" and fields["country"].type_name == "text"
    assert fields["sessions_INTEGER"].type_name == "bigint" and fields["purchaseRevenue_CURRENCY"].type_name == "double"
    assert fields["country"].nullable is False and fields["sessions_INTEGER"].nullable is True
    with pytest.raises(saas_adapter.SaasSetupRefused):
        ga4.describe(session, PARTS, DIAL, "t = country | nosuchmetric")


def test_a_first_run_reads_from_the_start_of_analytics_and_a_later_one_from_the_lagged_cursor() -> None:
    fake = FakeGa4([(days_ago(n), "ID", 10 - n, float(n)) for n in range(6)])
    session = session_over(fake)
    decl = ga4.declaration(session, PARTS, DIAL, REPORT)
    first = list(ga4.read(session, PARTS, DIAL, decl, since=None))
    assert len(first) == 6 and fake.posts[0][1]["dateRanges"][0]["startDate"] == "2015-08-14"
    cursor = max(r["date"] for r in first)
    fake.posts.clear()
    lagged = cursor.subtract(seconds=decl.lag_seconds)  # what dlt hands over: the cursor minus the lag
    again = list(ga4.read(session, PARTS, DIAL, decl, since=lagged))
    assert fake.posts[0][1]["dateRanges"][0]["startDate"] == lagged.to_date_string()
    assert sorted(r["date"].strftime("%Y%m%d") for r in again) == sorted(days_ago(n) for n in range(4)), "the last days are read again, nothing older"


def test_a_lag_needs_a_cursor_and_is_never_negative() -> None:
    from dispar_orchestrate.saas.objects import ObjectDecl

    with pytest.raises(ValueError):
        ObjectDecl(name="x", primary_key=("id",), full_reason="no filter", lag_seconds=60)
    with pytest.raises(ValueError):
        ObjectDecl(name="x", primary_key=("id",), cursor="t", lag_seconds=-1)
    assert ObjectDecl(name="x", primary_key=("id",), cursor="t", lag_seconds=60).lag_seconds == 60


# ---- the whole path: adapter, dlt incremental, merge on the dimensions, real Iceberg -------------------


def accept_all(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj() -> dict:
    return {"name": REPORT, "target": "ga_traffic", "loadMode": "incremental_dedup", "incrementalKey": "date", "primaryKey": ["country", "date"]}


@pytest.mark.loader
def test_a_second_run_corrects_the_last_days_and_stores_no_row_twice(lab: Lab, monkeypatch: pytest.MonkeyPatch) -> None:  # noqa: F811
    import time

    fake = FakeGa4([(days_ago(n), c, 10, 1.0) for n in range(4) for c in ("ID", "US")])
    module = ga4
    monkeypatch.setattr(saas_adapter, "open_session", lambda dial, secrets, **kw: (module, vendors.vendor("ga4"), PARTS, session_over(fake)))

    def run() -> Any:
        built = saas_adapter.build_source(DIAL, {}, [obj()], gate=accept_all)
        assert built is not None
        return load_via_sink(built.source, "ga_traffic", lab.config, f._load_plan("saas", obj()))

    assert run().rows == 8
    assert len(lab.rows("ga_traffic")) == 8
    time.sleep(1.1)
    # Google corrected yesterday's numbers after the first run, and a new day arrived
    fake.rows = [(d, c, 99 if d == days_ago(0) else s, r) for d, c, s, r in fake.rows]
    run()
    rows = lab.rows("ga_traffic")
    assert len(rows) == 8, "merged on country + date: no row twice"
    assert {r["sessions_integer"] for r in rows if r["country"] == "ID" and str(r["date"]).startswith(pendulum.yesterday().to_date_string())} == {99}


# ---- V-GA4.3: sign-in ---------------------------------------------------------------------------------

TOKEN_URL = "https://oauth2.googleapis.com/token"
OAUTH_DIAL = {"vendor": "ga4", "account": "123", "objects": [REPORT], "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}
SECRETS = {"clientSecret": "not-a-real-secret", "refreshToken": "not-a-real-refresh-token"}


@pytest.fixture(scope="module")
def private_key() -> rsa.RSAPrivateKey:
    return rsa.generate_private_key(public_exponent=65537, key_size=2048)  # a throwaway key made for the test run


def key_file(private_key: rsa.RSAPrivateKey, **over: Any) -> str:
    pem = private_key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()).decode()
    return json.dumps({"type": "service_account", "client_email": "reader@example.invalid", "private_key": pem, "token_uri": "https://evil.example.invalid/token", **over})


def decode(jwt: str) -> tuple[dict, dict, bytes, bytes]:
    head, claims, sig = jwt.split(".")
    pad = lambda part: base64.urlsafe_b64decode(part + "=" * (-len(part) % 4))  # noqa: E731
    return json.loads(pad(head)), json.loads(pad(claims)), pad(sig), f"{head}.{claims}".encode()


def test_the_browser_sign_in_refreshes_at_googles_token_address_with_the_client_in_the_form_body(monkeypatch: pytest.MonkeyPatch) -> None:
    sent: list[dict] = []

    def post(url: str, **kw: Any) -> Reply:
        sent.append({"url": url, **kw})
        return Reply(200, {"access_token": "g1", "token_type": "Bearer", "expires_in": 3599})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)
    fake = FakeGa4([("20240102", "ID", 1, 1.0)])
    module, vendor, parts, session = saas_adapter.open_session(OAUTH_DIAL, SECRETS, resolve_checked=resolver, http_get=fake.get, http_post=fake.post, sleep=lambda _s: None)
    assert session.get_json(vendor.whoami["path"].replace("{account}", parts.account))["metrics"]
    assert sent[0]["url"] == TOKEN_URL
    assert sent[0]["data"] == {"grant_type": "refresh_token", "refresh_token": "not-a-real-refresh-token", "client_id": "app", "client_secret": "not-a-real-secret"}
    assert {h["Authorization"] for h in fake.headers} == {"Bearer g1"}
    assert session.origin == "https://analyticsdata.googleapis.com", "Google returns no instance_url and the module names no adopt suffix"


def test_a_service_account_key_is_signed_into_an_rs256_assertion_and_exchanged_for_a_bearer_token(monkeypatch: pytest.MonkeyPatch, private_key: rsa.RSAPrivateKey) -> None:
    sent: list[dict] = []

    def post(url: str, **kw: Any) -> Reply:
        sent.append({"url": url, **kw})
        return Reply(200, {"access_token": "sa1", "token_type": "Bearer", "expires_in": 3600})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)
    monkeypatch.setattr("dispar_orchestrate.saas.vendors.ga4.time.time", lambda: 1_700_000_000.0)
    fake = FakeGa4()
    _m, vendor, parts, session = saas_adapter.open_session(DIAL, {"serviceAccountJson": key_file(private_key)}, resolve_checked=resolver, http_get=fake.get, http_post=fake.post, sleep=lambda _s: None)
    session.get_json("/v1beta/properties/123/metadata")
    call = sent[0]
    assert call["url"] == TOKEN_URL, "the table's address, never the key file's token_uri"
    assert call["data"]["grant_type"] == "urn:ietf:params:oauth:grant-type:jwt-bearer"
    header, claims, signature, signed = decode(call["data"]["assertion"])
    assert header == {"alg": "RS256", "typ": "JWT"}
    assert claims == {"iss": "reader@example.invalid", "scope": "https://www.googleapis.com/auth/analytics.readonly", "aud": TOKEN_URL, "iat": 1_700_000_000, "exp": 1_700_003_000}
    assert claims["exp"] - claims["iat"] <= 3600, "Google: at most one hour after iat"
    private_key.public_key().verify(signature, signed, padding.PKCS1v15(), hashes.SHA256())  # raises on a bad signature
    assert {h["Authorization"] for h in fake.headers} == {"Bearer sa1"}


def test_the_token_is_reused_until_it_expires_and_asked_again_after_a_401(monkeypatch: pytest.MonkeyPatch, private_key: rsa.RSAPrivateKey) -> None:
    now = {"t": 1_700_000_000.0}
    tokens: list[str] = []

    def post(url: str, **kw: Any) -> Reply:
        tokens.append(kw["data"]["assertion"])
        return Reply(200, {"access_token": f"sa{len(tokens)}", "expires_in": 3600})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)
    from dispar_orchestrate.saas.vendors import ga4 as module

    headers, refresh = module.integration_auth(vendors.vendor("ga4"), {"serviceAccountJson": key_file(private_key)}, resolve_checked=resolver, clock=lambda: now["t"])
    assert headers() == headers() == {"Authorization": "Bearer sa1"} and len(tokens) == 1
    now["t"] += 3500  # inside the hour, before the 60 s margin
    assert headers() == {"Authorization": "Bearer sa1"}
    now["t"] += 100
    assert headers() == {"Authorization": "Bearer sa2"}
    refresh()
    assert headers() == {"Authorization": "Bearer sa3"} and len(tokens) == 3


@pytest.mark.parametrize("secret", ["", "not json", "[]", "{}", '{"type": "authorized_user", "client_email": "a", "private_key": "b"}', '{"type": "service_account", "client_email": "a", "private_key": "not a key"}'])
def test_a_key_file_that_is_not_one_is_a_fixed_rejected_setting_that_quotes_nothing(secret: str) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused) as raised:
        saas_adapter.open_session(DIAL, {"serviceAccountJson": secret}, resolve_checked=resolver)
    assert "not a valid key file" in str(raised.value) and "not a key" not in str(raised.value)


def test_a_key_that_is_not_rsa_is_refused() -> None:
    from cryptography.hazmat.primitives.asymmetric import ec

    pem = ec.generate_private_key(ec.SECP256R1()).private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()).decode()
    with pytest.raises(saas_adapter.SaasSetupRefused):
        saas_adapter.open_session(DIAL, {"serviceAccountJson": json.dumps({"type": "service_account", "client_email": "a@example.invalid", "private_key": pem})}, resolve_checked=resolver)


def test_google_refusing_the_assertion_is_the_fixed_sign_in_failure(monkeypatch: pytest.MonkeyPatch, private_key: rsa.RSAPrivateKey) -> None:
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: Reply(400, {"error": "invalid_grant", "error_description": "STUB-VENDOR-WORDS"}))
    _m, _v, _p, session = saas_adapter.open_session(DIAL, {"serviceAccountJson": key_file(private_key)}, resolve_checked=resolver, http_get=FakeGa4().get, sleep=lambda _s: None)
    with pytest.raises(Exception) as raised:
        session.get_json("/v1beta/properties/123/metadata")
    assert ingest_errors.classify(raised.value).error_class == "sign_in_refused" and "STUB" not in str(raised.value)


def test_a_revoked_browser_sign_in_is_the_fixed_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", lambda url, **kw: Reply(400, {"error": "invalid_grant", "error_description": "STUB-VENDOR-WORDS"}))
    _m, _v, _p, session = saas_adapter.open_session(OAUTH_DIAL, SECRETS, resolve_checked=resolver, http_get=FakeGa4().get, sleep=lambda _s: None)
    with pytest.raises(Exception) as raised:
        session.get_json("/v1beta/properties/123/metadata")
    assert ingest_errors.classify(raised.value).error_class == "sign_in_refused" and "STUB" not in str(raised.value)


def test_the_vendor_row_is_what_googles_documentation_says() -> None:
    """V-GA4.3, checked on 2026-10-21 against the pages cited in `saas/vendors/ga4.py`."""
    row = vendors.vendor("ga4")
    authorize = urlparse(row.oauth["authorizeUrl"])
    assert f"{authorize.scheme}://{authorize.netloc}{authorize.path}" == "https://accounts.google.com/o/oauth2/v2/auth"
    assert parse_qs(authorize.query) == {"access_type": ["offline"], "prompt": ["consent"]}, "without access_type=offline Google returns no refresh token"
    assert row.oauth["tokenUrl"] == TOKEN_URL and row.oauth["revocationUrl"] == "https://oauth2.googleapis.com/revoke"
    assert row.oauth["scopes"] == ["https://www.googleapis.com/auth/analytics.readonly"] and row.oauth["clientAuth"] == "post"
    assert row.whoami == {"method": "GET", "path": "/v1beta/properties/{account}/metadata"}
    assert row.integration == {"secretKey": "integration:ga4", "secretFields": ["serviceAccountJson"]}
    assert row.api_url(row.validate_account("123")) == "https://analyticsdata.googleapis.com"
