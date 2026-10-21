"""The Python vendor table against the shared fixture, and the `saas` secret
mapping (`SRC-3` AD2, T1.8)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from dispar_orchestrate import secret_map
from dispar_orchestrate.saas import vendors

FIXTURE = Path(__file__).resolve().parents[2] / "ops" / "fixtures" / "saas_vendors.json"


def test_the_table_equals_the_shared_fixture() -> None:
    assert vendors._TABLE == json.loads(FIXTURE.read_text(encoding="utf-8"))["vendors"]


def test_there_are_ten_vendors_with_unique_ids_and_connector_types() -> None:
    assert len(vendors.VENDORS) == 10
    assert len({v.connector_type for v in vendors.VENDORS.values()}) == 10


TENANT_GUID = "00000000-0000-0000-0000-000000000001"

SAMPLE = {
    "salesforce": "acme", "servicenow": "acme", "zendesk": "acme", "confluence": "acme", "sharepoint": "acme/00000000-0000-0000-0000-000000000001",
    "workday": "wd2-impl-services1.workday.com/acme_tenant", "ga4": "123456789", "hubspot": "123456789",
    "dynamics365": "acme.crm4.dynamics.com/contoso.example.invalid", "netsuite": "1234567_SB1",
}  # fmt: skip


@pytest.mark.parametrize("vendor_id", sorted(SAMPLE))
def test_every_vendor_accepts_a_good_account_and_builds_https_addresses_only(vendor_id: str) -> None:
    v = vendors.vendor(vendor_id)
    parts = v.validate_account(SAMPLE[vendor_id])
    for template in (v.api_base, v.oauth["authorizeUrl"], v.oauth["tokenUrl"], v.oauth["revocationUrl"] or "https://x"):
        url = v.render(template, parts)
        assert url.startswith("https://") and "{" not in url, url


@pytest.mark.parametrize("vendor_id", sorted(SAMPLE))
@pytest.mark.parametrize(
    "value",
    ["evil.example.invalid", "https://acme", "http://acme", "acme@evil", "user:pass@acme", "acme:8443",
     "acme/path", "acme.evil", " acme", "acme ", "", "-acme", "acme-", "acme\n", "ac me"],
)  # fmt: skip
def test_every_vendor_refuses_a_value_outside_its_host_pattern(vendor_id: str, value: str) -> None:
    with pytest.raises(vendors.InvalidAccount):
        vendors.vendor(vendor_id).validate_account(value)


def test_the_near_misses_of_workday_dataverse_and_netsuite_are_refused() -> None:
    for value in ["wd2.workday.com", "wd2.workday.com/", "wd2.workday.com/a/b", "a.b.workday.com/t",
                  "wd2.workday.com.evil.invalid/t", "wd2.example.invalid/t", "wd2.workday.com:443/t"]:  # fmt: skip
        with pytest.raises(vendors.InvalidAccount):
            vendors.vendor("workday").validate_account(value)
    for value in ["acme.dynamics.com", "acme.crm123.dynamics.com", "acme.crmx.dynamics.com",
                  "acme.crm.dynamics.com.evil.invalid", "a.b.crm.dynamics.com", "acme.api.crm.dynamics.com"]:  # fmt: skip
        with pytest.raises(vendors.InvalidAccount):
            vendors.vendor("dynamics365").validate_account(f"{value}/{TENANT_GUID}")
    assert vendors.vendor("dynamics365").validate_account(f"Acme.CRM.dynamics.com/{TENANT_GUID}").account == "acme.crm.dynamics.com"
    assert vendors.vendor("netsuite").validate_account("1234567_SB1").account == "1234567-sb1"
    with pytest.raises(vendors.InvalidAccount):
        vendors.vendor("netsuite").validate_account("1234567_")


def test_an_unknown_vendor_and_a_vendor_without_a_module_are_unsupported(monkeypatch: pytest.MonkeyPatch) -> None:
    with pytest.raises(vendors.UnsupportedVendor):
        vendors.vendor("nobody")
    # Slice 6 gave the last of the ten a module; a vendor id added to the table first would be in this case.
    monkeypatch.delitem(vendors.MODULE_PATHS, "netsuite")
    monkeypatch.delitem(vendors._MODULES, "netsuite", raising=False)
    with pytest.raises(vendors.UnsupportedVendor):
        vendors.module_for("netsuite")
    assert set(vendors.MODULE_PATHS) | {"netsuite"} == set(vendors.VENDORS), "every other vendor of the table has a module"
    with pytest.raises(vendors.UnsupportedVendor):
        vendors.register("nobody", object())


def test_the_secret_mapping_resolves_saas_slots_through_the_vendor_table() -> None:
    assert secret_map.secret_field_names("saas", "oauth2_authorization_code") == ("clientSecret", "refreshToken")
    assert secret_map.secret_field_names("saas", "integration:zendesk") == ("email", "apiToken")
    assert secret_map.secret_field_names("saas", "integration:hubspot") == ("token",)
    for bad in ("integration:nobody", "bearer", None):
        with pytest.raises(ValueError):
            secret_map.secret_field_names("saas", bad)
    # Decision 4: Salesforce has no integration credential.
    assert vendors.vendor("salesforce").secret_key(oauth=False) is None


def test_the_gate_rebase_does_nothing_unless_the_gate_only_switch_and_the_vendor_are_named(monkeypatch: pytest.MonkeyPatch) -> None:
    """SRC-3 V-HubSpot.4: `SAAS_GATE_ORIGINS` goes round "hosts come only from the table", so it is
    closed twice and falls back to the real host on anything odd."""
    real = "https://api.hubapi.com/oauth/v1/token?x=1"
    origins = '{"hubspot": "http://saas-stub-g6:8101"}'
    monkeypatch.delenv("CONNECTOR_OAUTH_ALLOW_HTTP", raising=False)
    monkeypatch.setenv("SAAS_GATE_ORIGINS", origins)
    assert vendors.rebase_for_gate("hubspot", real) == real, "without the switch the variable is ignored"
    monkeypatch.setenv("CONNECTOR_OAUTH_ALLOW_HTTP", "true")
    assert vendors.rebase_for_gate("hubspot", real) == "http://saas-stub-g6:8101/oauth/v1/token?x=1"
    assert vendors.rebase_for_gate("zendesk", real) == real, "a vendor the variable does not name is untouched"
    for odd in ("not json", "[]", '{"hubspot": 5}', '{"hubspot": "ftp://x:1"}', '{"hubspot": "http://u:p@x:1"}', '{"hubspot": "http://x:1/prefix"}', '{"hubspot": "http://x:1/?q=1"}'):
        monkeypatch.setenv("SAAS_GATE_ORIGINS", odd)
        assert vendors.rebase_for_gate("hubspot", real) == real, odd
    monkeypatch.setenv("CONNECTOR_OAUTH_ALLOW_HTTP", "yes")
    monkeypatch.setenv("SAAS_GATE_ORIGINS", origins)
    assert vendors.rebase_for_gate("hubspot", real) == real, "only the exact word true opens the switch"


# ---- the Microsoft tenant (slice 5): a validated part of the account ---------------------------------


@pytest.mark.parametrize(("vendor_id", "inner"), [("sharepoint", "Acme"), ("dynamics365", "Acme.CRM4.dynamics.com")])
def test_a_microsoft_account_carries_a_tenant_that_forms_the_token_address(vendor_id: str, inner: str) -> None:
    v = vendors.vendor(vendor_id)
    for tenant in (TENANT_GUID.upper(), "contoso.example.invalid", "a.b.example.invalid"):
        parts = v.validate_account(f"{inner}/{tenant}")
        assert parts.tenant == tenant.lower() and parts.account == inner.lower()
        assert v.token_url(parts) == f"https://login.microsoftonline.com/{tenant.lower()}/oauth2/v2.0/token"
        assert v.render(v.oauth["authorizeUrl"], parts).startswith(f"https://login.microsoftonline.com/{tenant.lower()}/")


@pytest.mark.parametrize("vendor_id", ["sharepoint", "dynamics365"])
@pytest.mark.parametrize(
    "tenant",
    ["", "common", "organizations", "consumers", "contoso", "contoso.", ".contoso.com", "1.2.3.4", "a..b.com", "-a.example.invalid",
     "contoso.example.invalid/x", "contoso.example.invalid?x=1", "contoso.example.invalid#x", "contoso.example.invalid:443",
     "user@contoso.example.invalid", "../x.example.invalid", "00000000-0000-0000-0000-00000000000g", "00000000000000000000000000000001",
     "0000000-00000-0000-0000-000000000001", "contoso.example.invalid\n", " contoso.example.invalid", "x" * 64 + ".com"],
)  # fmt: skip
def test_a_tenant_that_is_not_a_guid_or_a_domain_name_is_refused(vendor_id: str, tenant: str) -> None:
    inner = "acme" if vendor_id == "sharepoint" else "acme.crm4.dynamics.com"
    with pytest.raises(vendors.InvalidAccount):
        vendors.vendor(vendor_id).validate_account(f"{inner}/{tenant}")


@pytest.mark.parametrize("vendor_id", ["sharepoint", "dynamics365"])
def test_a_microsoft_account_without_a_tenant_is_refused(vendor_id: str) -> None:
    for value in ("acme", "acme.crm4.dynamics.com", "acme/", "/" + TENANT_GUID):
        with pytest.raises(vendors.InvalidAccount):
            vendors.vendor(vendor_id).validate_account(value)


def test_the_microsoft_client_credentials_scope_is_dot_default_of_the_vendors_own_resource() -> None:
    graph, dataverse = vendors.vendor("sharepoint"), vendors.vendor("dynamics365")
    assert graph.integration_scope(graph.validate_account(SAMPLE["sharepoint"])) == "https://graph.microsoft.com/.default"
    parts = dataverse.validate_account(SAMPLE["dynamics365"])
    assert dataverse.integration_scope(parts) == "https://acme.crm4.dynamics.com/.default"
    assert [v.id for v in vendors.VENDORS.values() if (v.integration or {}).get("scope")] == ["dynamics365", "sharepoint"]
