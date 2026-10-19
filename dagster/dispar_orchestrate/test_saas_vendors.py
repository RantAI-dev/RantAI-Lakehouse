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


SAMPLE = {
    "salesforce": "acme", "servicenow": "acme", "zendesk": "acme", "confluence": "acme", "sharepoint": "acme",
    "workday": "wd2-impl-services1.workday.com/acme_tenant", "ga4": "123456789", "hubspot": "123456789",
    "dynamics365": "acme.crm4.dynamics.com", "netsuite": "1234567_SB1",
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
            vendors.vendor("dynamics365").validate_account(value)
    assert vendors.vendor("dynamics365").validate_account("Acme.CRM.dynamics.com").account == "acme.crm.dynamics.com"
    assert vendors.vendor("netsuite").validate_account("1234567_SB1").account == "1234567-sb1"
    with pytest.raises(vendors.InvalidAccount):
        vendors.vendor("netsuite").validate_account("1234567_")


def test_an_unknown_vendor_and_a_vendor_without_a_module_are_unsupported() -> None:
    with pytest.raises(vendors.UnsupportedVendor):
        vendors.vendor("nobody")
    with pytest.raises(vendors.UnsupportedVendor):
        vendors.module_for("netsuite")  # no module until slice 6
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
