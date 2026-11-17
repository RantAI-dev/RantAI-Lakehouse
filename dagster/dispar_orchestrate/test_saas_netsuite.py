"""`saas/vendors/netsuite.py` (`SRC-3` slice 6, V-NetSuite.1 to .3): reading SuiteQL through the
real `SaasSession` over a FAKE NetSuite (the `POST /services/rest/query/v1/suiteql` request with
`Prefer: transient`, `limit` and `offset`, `hasMore`, a null column left out, the signed
assertion of the certificate way checked with the public key). No network. The loader test runs
`load_via_sink` for real on `test_load_modes_measurement.Lab`; CI runs it in `dagster-loader-tests`.

The SQL the fake understands is only the shapes the module writes; no SuiteQL function was run
against a NetSuite account (decision 10), so the fake is the developer's model of the
documentation, not a recording. Keys are generated here at run time; no key is written down."""

from __future__ import annotations

import base64
import json
import re
from datetime import datetime, timezone
from typing import Any
from urllib.parse import parse_qs, urlparse

import pytest
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, padding, rsa, utils

from dispar_orchestrate import schema_observer
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import session as saas_session
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import netsuite
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_zendesk import Reply, resolver

ORIGIN = "https://1234567-sb1.suitetalk.api.netsuite.com"
TOKEN_URL = f"{ORIGIN}/services/rest/auth/oauth2/v1/token"
DIAL = {"vendor": "netsuite", "account": "1234567_SB1", "auth": {"type": "integration"}, "certificateId": "cert-1"}
OAUTH_DIAL = {"vendor": "netsuite", "account": "1234567_SB1", "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}


def _pem(key: Any) -> str:
    return key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()).decode()


RSA_KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)
EC_KEY = ec.generate_private_key(ec.SECP256R1())
SECRETS = {"clientId": "ns-client-id", "privateKey": _pem(RSA_KEY)}


def stamp(day: int, hour: int = 10) -> str:
    return f"2026-10-{day:02d} {hour:02d}:00:00"


def record(i: int, day: int, **extra: Any) -> dict[str, Any]:
    return {"id": str(i), "companyname": f"customer {i}", "lastmodifieddate": stamp(day), **extra}


class NetSuite:
    """SuiteQL over REST as modelled: tables of rows, `limit` and `offset` in the address, `hasMore`,
    `Prefer: transient` required, a null column left out, a `links` array on every item."""

    def __init__(self, page_cap: int = 1000) -> None:
        self.tables: dict[str, list[dict[str, Any]]] = {"customer": [], "note": [{"id": "1", "memo": "m"}]}
        self.log: list[str] = []
        self.posts: list[dict] = []
        self.page_cap = page_cap
        self.status: int | None = None

    def post(self, url: str, *, headers: dict, json: Any, timeout: float) -> Reply:  # noqa: A002
        parsed = urlparse(url)
        query = {k: v[0] for k, v in parse_qs(parsed.query).items()}
        sql = json["q"]
        self.posts.append({"path": parsed.path, "query": query, "headers": headers, "sql": sql, "timeout": timeout})
        self.log.append(sql)
        if self.status is not None:
            return Reply(self.status, {"o:errorDetails": [{"detail": "STUB-VENDOR-WORDS"}]})
        if headers.get("Prefer") != "transient":
            return Reply(400, {"title": "Prefer: transient is required STUB-VENDOR-WORDS"})
        match = re.match(r"SELECT (.+?) FROM ([a-z_]+)(?: WHERE (.+?))? ?(?:ORDER BY (.+))?$", sql)
        assert match, sql
        columns, table, where, _order = match.groups()
        rows = self.tables.get(table)
        if rows is None:
            return Reply(400, {"title": "Invalid table STUB-VENDOR-WORDS"})
        if columns in (f"{KEY}, {CHG}", KEY):
            wanted = columns.split(", ")
            if any(w not in (rows[0] if rows else {"id": 1, "lastmodifieddate": 1}) for w in wanted):
                return Reply(400, {"title": "Unknown identifier STUB-VENDOR-WORDS"})
        selected = list(rows)
        if where:
            since = re.search(r"TO_TIMESTAMP\('([^']+)'", where)
            if since:
                selected = [r for r in selected if r.get("lastmodifieddate", "") >= since.group(1)]
            after = re.search(rf"{table}\.id > (\d+)", where)
            if after:
                selected = [r for r in selected if int(r["id"]) > int(after.group(1))]
        if "lastmodifieddate" in (_order or ""):
            selected.sort(key=lambda r: (r.get("lastmodifieddate", ""), int(r["id"])))
        else:
            selected.sort(key=lambda r: int(r["id"]))
        offset, limit = int(query.get("offset", 0)), min(int(query.get("limit", 1000)), self.page_cap)
        chunk = selected[offset : offset + limit]
        items = []
        for r in chunk:
            item = {"links": [{"rel": "self", "href": "https://evil.example.invalid/x"}], **r}
            if "TO_CHAR" in columns and "lastmodifieddate" in r:
                item["modified_at"] = r["lastmodifieddate"]
            items.append(item)
        return Reply(200, {"links": [], "count": len(items), "hasMore": offset + limit < len(selected), "items": items, "offset": offset, "totalResults": len(selected)})


KEY, CHG = "id", "lastmodifieddate"


@pytest.fixture
def api() -> NetSuite:
    return NetSuite()


def open_(api: NetSuite, dial: dict = DIAL, secrets: dict = SECRETS):  # noqa: ANN202
    return saas_adapter.open_session(dial, secrets, resolve_checked=resolver, http_post=api.post, sleep=lambda _s: None)


TOKEN_POSTS: list[dict] = []


@pytest.fixture(autouse=True)
def token_endpoint(monkeypatch: pytest.MonkeyPatch) -> None:
    TOKEN_POSTS.clear()

    def post(url: str, **kw: Any) -> Reply:
        TOKEN_POSTS.append({"url": url, "data": kw.get("data"), "headers": kw.get("headers")})
        return Reply(200, {"access_token": "ns-access", "token_type": "bearer", "expires_in": 3600})

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", post)


def read(api: NetSuite, name: str = "customer", since: Any = None, dial: dict = DIAL) -> list[dict]:
    module, _vendor, parts, session = open_(api, dial)
    return list(module.read(session, parts, dial, module.declaration(session, parts, dial, name), since=since))


def reads(api: NetSuite) -> list[dict]:
    """The posts without the one-row probes `declaration` makes."""
    return [p for p in api.posts if p["query"].get("limit") != "1"]


def test_the_vendor_module_is_found_by_the_registry() -> None:
    assert vendors.module_for("netsuite") is netsuite


# ---- reading --------------------------------------------------------------------------------------------


def test_a_first_run_posts_suiteql_with_prefer_transient_and_pages_by_offset_until_has_more_ends(api: NetSuite, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(netsuite, "PAGE_SIZE", 3)
    api.tables["customer"] = [record(i, i) for i in range(1, 8)]
    got = read(api)
    assert [r["id"] for r in got] == [str(i) for i in range(1, 8)]
    first = reads(api)
    assert [p["query"] for p in first] == [{"limit": "3", "offset": "0"}, {"limit": "3", "offset": "3"}, {"limit": "3", "offset": "6"}]
    assert all(p["path"] == "/services/rest/query/v1/suiteql" and p["headers"]["Prefer"] == "transient" for p in first)
    assert first[0]["headers"]["Authorization"] == "Bearer ns-access"
    assert first[0]["sql"].startswith("SELECT customer.*, TO_CHAR(customer.lastmodifieddate, 'YYYY-MM-DD HH24:MI:SS') AS modified_at FROM customer ORDER BY customer.lastmodifieddate, customer.id")
    assert "WHERE" not in first[0]["sql"]


def test_records_lose_the_links_array_and_get_their_change_time_as_a_timestamp(api: NetSuite) -> None:
    api.tables["customer"] = [record(1, 1)]
    (row,) = read(api)
    assert "links" not in row and row["modified_at"] == datetime(2026, 10, 1, 10, tzinfo=timezone.utc) and row["companyname"] == "customer 1"


def test_a_record_without_a_change_time_gets_the_epoch_in_the_cursor_column_only(api: NetSuite) -> None:
    api.tables["customer"] = [record(1, 1), {"id": "2", "companyname": "no date"}]
    row = read(api)[0]  # sorted by change time: the epoch comes first
    assert row["id"] == "2" and row["modified_at"] == datetime(1970, 1, 1, tzinfo=timezone.utc) and "lastmodifieddate" not in row


def test_a_later_run_asks_only_for_records_changed_since_the_cursor(api: NetSuite) -> None:
    api.tables["customer"] = [record(i, i) for i in range(1, 6)]
    got = read(api, since="2026-10-03 10:00:00")
    assert [r["id"] for r in got] == ["3", "4", "5"]
    assert "WHERE customer.lastmodifieddate >= TO_TIMESTAMP('2026-10-03 10:00:00', 'YYYY-MM-DD HH24:MI:SS')" in reads(api)[0]["sql"]


def test_the_cursor_may_arrive_as_a_datetime_and_a_value_that_is_not_a_date_is_refused(api: NetSuite) -> None:
    api.tables["customer"] = [record(1, 1)]
    read(api, since=datetime(2026, 10, 1, 12, tzinfo=timezone.utc))
    assert "TO_TIMESTAMP('2026-10-01 12:00:00'" in reads(api)[0]["sql"]
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(api, since="1'); drop")


def test_a_table_without_a_change_time_is_read_in_full_by_key_with_offset_zero(api: NetSuite, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(netsuite, "PAGE_SIZE", 2)
    api.tables["note"] = [{"id": str(i), "memo": f"m{i}"} for i in range(1, 6)]
    got = read(api, "note")
    assert [r["id"] for r in got] == ["1", "2", "3", "4", "5"]
    sql = [p["sql"] for p in reads(api)]
    assert sql[0] == "SELECT note.* FROM note ORDER BY note.id"
    assert sql[1] == "SELECT note.* FROM note WHERE note.id > 2 ORDER BY note.id"
    assert all(p["query"]["offset"] == "0" for p in reads(api)), "keyset paging: the 100,000-row limit never applies"


def test_an_incremental_read_starts_again_from_the_last_change_time_before_the_row_limit(api: NetSuite, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(netsuite, "PAGE_SIZE", 2)
    monkeypatch.setattr(netsuite, "RESTART_AT", 4)
    api.tables["customer"] = [record(i, i) for i in range(1, 10)]
    got = read(api)
    assert sorted({r["id"] for r in got}, key=int) == [str(i) for i in range(1, 10)], "every row read, repeats allowed (merged on id)"
    assert any("TO_TIMESTAMP('2026-10-04 10:00:00'" in p["sql"] for p in reads(api)), "the second window began at the last change time"


def test_more_rows_at_one_change_time_than_one_query_returns_stops_the_run_with_a_plain_message(api: NetSuite, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(netsuite, "PAGE_SIZE", 2)
    monkeypatch.setattr(netsuite, "RESTART_AT", 4)
    api.tables["customer"] = [{"id": str(i), "lastmodifieddate": stamp(1)} for i in range(1, 10)]
    with pytest.raises(saas_adapter.SaasSetupRefused) as err:
        read(api)
    assert "share one change time" in str(err.value)


# ---- objects --------------------------------------------------------------------------------------------


def test_a_table_with_both_columns_is_incremental_one_with_an_id_only_is_read_in_full_and_an_unknown_one_is_refused(api: NetSuite) -> None:
    api.tables["customer"] = [record(1, 1)]
    module, _v, parts, session = open_(api)
    inc = module.declaration(session, parts, DIAL, "customer")
    assert inc.incremental and inc.cursor == "modified_at" and inc.primary_key == ("id",) and inc.lag_seconds == 86_400
    full = module.declaration(session, parts, DIAL, "note")
    assert not full.incremental and full.load_description() == "re-read in full: note has no lastmodifieddate column to filter by"
    with pytest.raises(saas_adapter.SaasSetupRefused) as err:
        module.declaration(session, parts, DIAL, "nothing_here")
    assert "STUB-VENDOR-WORDS" not in str(err.value)


@pytest.mark.parametrize("name", ["", "Customer", "a b", "x;y", "../customer", "customer--", "customer'", "1abc", "a" * 65])
def test_a_table_name_that_is_not_plain_is_refused_before_any_request(api: NetSuite, name: str) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused):
        read(api, name)
    assert api.posts == []


def test_the_suggested_tables_and_the_ones_named_in_the_dial_are_listed(api: NetSuite) -> None:
    module, _v, parts, session = open_(api)
    names = [d.name for d in module.list_objects(session, parts, {**DIAL, "objects": ["customrecord_x", "customer"]})]
    assert names == [*netsuite.SUGGESTED, "customrecord_x"]


def test_netsuite_has_no_describe_so_a_changed_column_is_seen_after_the_load() -> None:
    assert not hasattr(netsuite, "describe")


# ---- failures -------------------------------------------------------------------------------------------


def test_the_access_failures_have_their_own_fixed_classes(api: NetSuite) -> None:
    for status, expected in ((401, saas_session.SaasSignInRefused), (403, saas_session.SaasPermissionMissing), (429, saas_session.SaasRateLimited)):
        api.status = status
        with pytest.raises(expected) as err:
            read(api)
        assert "STUB-VENDOR-WORDS" not in str(err.value)


def test_a_link_in_an_item_is_data_and_never_dialled(api: NetSuite) -> None:
    api.tables["customer"] = [record(1, 1)]
    read(api)
    assert len(api.posts) == 2, "the probe and the read; the evil link in `links` was not followed"


# ---- the certificate way --------------------------------------------------------------------------------


def _decode(assertion: str) -> tuple[dict, dict, bytes, bytes]:
    header, claims, sig = assertion.split(".")
    pad = lambda t: t + "=" * (-len(t) % 4)  # noqa: E731
    return (
        json.loads(base64.urlsafe_b64decode(pad(header))),
        json.loads(base64.urlsafe_b64decode(pad(claims))),
        base64.urlsafe_b64decode(pad(sig)),
        f"{header}.{claims}".encode(),
    )


def test_an_rsa_key_signs_a_ps256_assertion_with_the_certificate_id_and_the_accounts_own_audience(api: NetSuite) -> None:
    api.tables["customer"] = [record(1, 1)]
    read(api)
    assert TOKEN_POSTS[0]["url"] == TOKEN_URL
    form = TOKEN_POSTS[0]["data"]
    assert form["grant_type"] == "client_credentials" and form["client_assertion_type"] == "urn:ietf:params:oauth:client-assertion-type:jwt-bearer"
    header, claims, signature, signing_input = _decode(form["client_assertion"])
    assert header == {"alg": "PS256", "typ": "JWT", "kid": "cert-1"}
    assert claims["iss"] == "ns-client-id" and claims["scope"] == "rest_webservices" and claims["aud"] == TOKEN_URL
    assert 0 < claims["exp"] - claims["iat"] <= 3600
    RSA_KEY.public_key().verify(signature, signing_input, padding.PSS(mgf=padding.MGF1(hashes.SHA256()), salt_length=32), hashes.SHA256())


def test_a_p256_key_signs_an_es256_assertion_in_raw_form() -> None:
    assertion = netsuite.build_assertion("c", "cert-2", EC_KEY, "rest_webservices", TOKEN_URL, 1_000_000)
    header, _claims, signature, signing_input = _decode(assertion)
    assert header["alg"] == "ES256" and header["kid"] == "cert-2" and len(signature) == 64
    r, s = int.from_bytes(signature[:32], "big"), int.from_bytes(signature[32:], "big")
    EC_KEY.public_key().verify(utils.encode_dss_signature(r, s), signing_input, ec.ECDSA(hashes.SHA256()))


def test_a_missing_certificate_id_or_an_unusable_key_is_a_fixed_rejected_setting_that_sends_nothing(api: NetSuite) -> None:
    for dial, secrets in (
        ({k: v for k, v in DIAL.items() if k != "certificateId"}, SECRETS),
        ({**DIAL, "certificateId": "  "}, SECRETS),
        (DIAL, {**SECRETS, "privateKey": "not a key, secret-looking-text"}),
        (DIAL, {**SECRETS, "privateKey": _pem(ec.generate_private_key(ec.SECP384R1()))}),
    ):
        with pytest.raises(saas_adapter.SaasSetupRefused) as err:
            open_(api, dial, secrets)
        assert "secret-looking-text" not in str(err.value)
    assert TOKEN_POSTS == [] and api.posts == []


def test_the_vendor_row_names_the_accounts_oauth_addresses_and_basic_client_authentication() -> None:
    row_ = vendors.vendor("netsuite")
    parts = row_.validate_account("1234567_SB1")
    assert row_.render(row_.oauth["authorizeUrl"], parts) == "https://1234567-sb1.app.netsuite.com/app/login/oauth2/authorize.nl"
    assert row_.token_url(parts) == TOKEN_URL and row_.oauth["clientAuth"] == "basic" and row_.oauth["scopes"] == ["rest_webservices"]
    assert row_.integration["secretFields"] == ["clientId", "privateKey"]


def test_the_browser_sign_in_refreshes_at_the_accounts_token_address_with_the_client_in_the_header(api: NetSuite) -> None:
    api.tables["customer"] = [record(1, 1)]
    secrets = {"clientSecret": "not-a-real-secret", "refreshToken": "refresh-1"}
    module, _v, parts, session = saas_adapter.open_session(OAUTH_DIAL, secrets, resolve_checked=resolver, http_post=api.post, sleep=lambda _s: None)
    assert [r["id"] for r in module.read(session, parts, OAUTH_DIAL, module.declaration(session, parts, OAUTH_DIAL, "customer"), since=None)] == ["1"]
    assert TOKEN_POSTS[0]["url"] == TOKEN_URL
    assert TOKEN_POSTS[0]["data"] == {"grant_type": "refresh_token", "refresh_token": "refresh-1"}
    assert TOKEN_POSTS[0]["headers"]["Authorization"].startswith("Basic ")


# ---- the load -------------------------------------------------------------------------------------------


def accept_all(table: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj(**extra: Any) -> dict[str, Any]:
    return {"name": "customer", "target": "ns_customer", "loadMode": "incremental_dedup", "incrementalKey": "modified_at", "primaryKey": ["id"], **extra}


@pytest.mark.loader
def test_a_second_run_merges_the_changed_customer_and_stores_none_twice(lab: Lab, api: NetSuite) -> None:
    api.tables["customer"] = [record(i, i) for i in range(1, 6)]

    def run() -> Any:
        built = saas_adapter.build_source(DIAL, SECRETS, [obj()], gate=accept_all, resolve_checked=resolver, http_post=api.post, sleep=lambda _s: None)
        assert built is not None and not built.observed_before_load
        return load_via_sink(built.source, "ns_customer", lab.config, f._load_plan("saas", obj()))

    assert run().rows == 5
    api.tables["customer"][1] = record(2, 9, companyname="changed")
    api.tables["customer"].append(record(6, 9))
    run()
    rows = {r["id"]: r["companyname"] for r in lab.rows("ns_customer")}
    assert len(lab.rows("ns_customer")) == 6 and rows["2"] == "changed", "merged on id: no duplicate"
