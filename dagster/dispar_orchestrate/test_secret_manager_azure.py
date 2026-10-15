"""`SRC-12` part B5: the Azure Key Vault reader against recorded responses of
the documented interface (client-credentials token, managed-identity token,
get secret). No network, and *not run against the real service*; requests go
through the `manager_http` fake."""

from __future__ import annotations

import pytest

from dispar_orchestrate import secret_manager
from dispar_orchestrate.secret_resolver import SecretRefRejected, resolve_secret_ref

_TOKEN_URL = "https://login.test/tenant-1/oauth2/v2.0/token"
_SECRET_URL = "https://corp-vault.vault.azure.net/secrets/orders-db"
_REF = "azure-kv:corp-vault/orders-db"
_IMDS_URL = "http://169.254.169.254/metadata/identity/oauth2/token"


@pytest.fixture(autouse=True)
def azure_env(monkeypatch: pytest.MonkeyPatch) -> None:
    for name in ("IDENTITY_ENDPOINT", "IDENTITY_HEADER"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("SECRET_MANAGER", "azure")
    monkeypatch.setenv("AZURE_TENANT_ID", "tenant-1")
    monkeypatch.setenv("AZURE_CLIENT_ID", "client-1")
    monkeypatch.setenv("AZURE_CLIENT_SECRET", "client-secret-test")
    monkeypatch.setenv("AZURE_AUTHORITY_HOST", "https://login.test/")


def _routes(manager_http, value: str = "pw-from-azure") -> None:
    manager_http.add("POST", _TOKEN_URL, json={"access_token": "eyJ.tok", "expires_in": 3599, "token_type": "Bearer"})
    manager_http.add("GET", _SECRET_URL, json={"value": value, "id": "https://corp-vault.vault.azure.net/secrets/orders-db/abc"})


def test_gets_a_client_credentials_token_then_reads_the_secret(manager_http) -> None:
    _routes(manager_http)
    assert resolve_secret_ref(_REF) == "pw-from-azure"
    token_call, read_call = manager_http.calls
    assert token_call["data"] == {
        "grant_type": "client_credentials",
        "client_id": "client-1",
        "client_secret": "client-secret-test",
        "scope": "https://vault.azure.net/.default",
    }
    assert read_call["url"] == _SECRET_URL + "?api-version=7.4"
    assert read_call["headers"] == {"Authorization": "Bearer eyJ.tok"}
    assert read_call["allow_redirects"] is False


def test_a_version_in_the_reference_is_part_of_the_path(manager_http) -> None:
    manager_http.add("POST", _TOKEN_URL, json={"access_token": "t"})
    manager_http.add("GET", _SECRET_URL + "/0123456789abcdef0123456789abcdef", json={"value": "pinned"})
    assert resolve_secret_ref(_REF + "/0123456789abcdef0123456789abcdef") == "pinned"


def test_every_read_gets_a_token_and_reads_again(manager_http) -> None:
    manager_http.add("POST", _TOKEN_URL, json={"access_token": "t"})
    manager_http.add("GET", _SECRET_URL, json={"value": "old"}, times=1)
    manager_http.add("GET", _SECRET_URL, json={"value": "new"})
    assert resolve_secret_ref(_REF) == "old"
    assert resolve_secret_ref(_REF) == "new"
    assert len([c for c in manager_http.calls if c["method"] == "POST"]) == 2


@pytest.mark.parametrize(
    ("status", "reason"),
    [
        (404, "manager_not_found"),
        (403, "manager_forbidden"),
        (401, "manager_sign_in"),
        (500, "manager_unreachable"),
        (307, "manager_unreachable"),
    ],
)
def test_each_read_failure_is_classified_by_status_and_azures_text_is_dropped(
    manager_http, status: int, reason: str
) -> None:
    manager_http.add("POST", _TOKEN_URL, json={"access_token": "t"})
    manager_http.add("GET", _SECRET_URL, status=status, json={"error": {"code": "Forbidden", "message": "azure-own-words"}})
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS[reason])
    assert "azure-own-words" not in str(excinfo.value)


def test_a_refused_token_request_is_may_not_sign_in(manager_http) -> None:
    manager_http.add("POST", _TOKEN_URL, status=401, json={"error": "invalid_client", "error_description": "AADSTS7000215 own words"})
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS["manager_sign_in"])
    assert "AADSTS" not in str(excinfo.value)
    assert "client-secret-test" not in str(excinfo.value)


def test_a_value_that_is_not_text_is_not_a_single_text_and_a_missing_one_does_not_exist(manager_http) -> None:
    manager_http.add("POST", _TOKEN_URL, json={"access_token": "t"})
    manager_http.add("GET", _SECRET_URL, json={"value": 7}, times=1)
    with pytest.raises(SecretRefRejected, match="not a single text"):
        resolve_secret_ref(_REF)
    manager_http.add("GET", _SECRET_URL, json={"id": "x"})
    with pytest.raises(SecretRefRejected, match="does not exist"):
        resolve_secret_ref(_REF)


def test_the_instance_metadata_identity_is_used_without_an_app_registration(
    manager_http, monkeypatch: pytest.MonkeyPatch
) -> None:
    for name in ("AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET"):
        monkeypatch.delenv(name)
    manager_http.add("GET", _IMDS_URL, json={"access_token": "imds-tok", "resource": "https://vault.azure.net"})
    manager_http.add("GET", _SECRET_URL, json={"value": "via-imds"})
    assert resolve_secret_ref(_REF) == "via-imds"
    imds_call, read_call = manager_http.calls
    assert imds_call["headers"] == {"Metadata": "true"}
    assert "resource=https%3A%2F%2Fvault.azure.net" in imds_call["url"]
    assert "client_id" not in imds_call["url"]
    assert imds_call["timeout"] == 2
    assert read_call["headers"] == {"Authorization": "Bearer imds-tok"}


def test_a_user_assigned_identity_names_its_client_id(manager_http, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("AZURE_TENANT_ID")
    monkeypatch.delenv("AZURE_CLIENT_SECRET")
    manager_http.add("GET", _IMDS_URL, json={"access_token": "imds-tok"})
    manager_http.add("GET", _SECRET_URL, json={"value": "x"})
    resolve_secret_ref(_REF)
    assert "client_id=client-1" in manager_http.calls[0]["url"]


def test_the_app_service_identity_endpoint_is_used_when_the_platform_sets_it(
    manager_http, monkeypatch: pytest.MonkeyPatch
) -> None:
    for name in ("AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET"):
        monkeypatch.delenv(name)
    monkeypatch.setenv("IDENTITY_ENDPOINT", "http://identity.test/msi/token")
    monkeypatch.setenv("IDENTITY_HEADER", "hdr-1")
    manager_http.add("GET", "http://identity.test/msi/token", json={"access_token": "app-tok"})
    manager_http.add("GET", _SECRET_URL, json={"value": "x"})
    resolve_secret_ref(_REF)
    assert manager_http.calls[0]["headers"] == {"X-IDENTITY-HEADER": "hdr-1"}
    assert manager_http.calls[1]["headers"] == {"Authorization": "Bearer app-tok"}


def test_no_registration_and_no_identity_service_is_may_not_sign_in(
    manager_http, monkeypatch: pytest.MonkeyPatch
) -> None:
    for name in ("AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET"):
        monkeypatch.delenv(name)
    with pytest.raises(SecretRefRejected, match="may not sign in"):
        resolve_secret_ref(_REF)


def test_a_vault_that_is_not_a_plain_name_never_reaches_the_network(manager_http) -> None:
    for bad in ("azure-kv:corp-vault@evil.example/orders", "azure-kv:a.b.c/orders", "azure-kv:ab/orders"):
        with pytest.raises(SecretRefRejected):
            resolve_secret_ref(bad)
    assert manager_http.calls == []
