"""`SRC-12` part B2: the Vault reader, against recorded KV v2 answers. No
network; the requests go through the `manager_http` fake."""

from __future__ import annotations

import pytest

from dispar_orchestrate import secret_manager
from dispar_orchestrate.secret_manager import ManagerFailure
from dispar_orchestrate.secret_resolver import SecretRefRejected, resolve_secret_ref

_URL = "http://vault.test:8200/v1/secret/data/connectors/orders"
_REF = "vault:secret/connectors/orders#password"


def _kv2(data: dict) -> dict:
    return {"data": {"data": data, "metadata": {"version": 3}}}


@pytest.fixture(autouse=True)
def vault_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("SECRET_MANAGER", "vault")
    monkeypatch.setenv("VAULT_ADDR", "http://vault.test:8200/")
    monkeypatch.setenv("VAULT_TOKEN", "tok-1")
    monkeypatch.delenv("VAULT_ROLE_ID", raising=False)
    monkeypatch.delenv("VAULT_SECRET_ID", raising=False)


def test_reads_the_key_with_the_given_token(manager_http) -> None:
    manager_http.add("GET", _URL, json=_kv2({"password": "pw-from-vault", "user": "u"}))
    assert resolve_secret_ref(_REF) == "pw-from-vault"
    (call,) = manager_http.calls
    assert call["headers"] == {"X-Vault-Token": "tok-1"}
    assert call["allow_redirects"] is False
    assert call["timeout"] == secret_manager.REQUEST_TIMEOUT


def test_every_read_goes_to_vault_and_a_changed_secret_is_seen_at_once(manager_http) -> None:
    manager_http.add("GET", _URL, json=_kv2({"password": "old"}), times=1)
    manager_http.add("GET", _URL, json=_kv2({"password": "new"}))
    assert resolve_secret_ref(_REF) == "old"
    assert resolve_secret_ref(_REF) == "new"


def test_signs_in_with_approle_for_each_read_and_uses_the_token_it_gets(
    manager_http, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("VAULT_TOKEN")
    monkeypatch.setenv("VAULT_ROLE_ID", "role-1")
    monkeypatch.setenv("VAULT_SECRET_ID", "sid-1")
    manager_http.add("POST", "http://vault.test:8200/v1/auth/approle/login", json={"auth": {"client_token": "s.issued"}})
    manager_http.add("GET", _URL, json=_kv2({"password": "pw"}))
    assert resolve_secret_ref(_REF) == "pw"
    assert resolve_secret_ref(_REF) == "pw"
    logins = [c for c in manager_http.calls if c["method"] == "POST"]
    reads = [c for c in manager_http.calls if c["method"] == "GET"]
    assert len(logins) == 2 and logins[0]["json"] == {"role_id": "role-1", "secret_id": "sid-1"}
    assert all(c["headers"] == {"X-Vault-Token": "s.issued"} for c in reads)


def test_a_refused_approle_sign_in_is_may_not_sign_in(manager_http, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("VAULT_TOKEN")
    monkeypatch.setenv("VAULT_ROLE_ID", "r")
    monkeypatch.setenv("VAULT_SECRET_ID", "s")
    manager_http.add("POST", "http://vault.test:8200/v1/auth/approle/login", status=400, json={"errors": ["secret detail"]})
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS["manager_sign_in"])
    assert "secret detail" not in str(excinfo.value)


@pytest.mark.parametrize(
    ("status", "reason"),
    [
        (404, "manager_not_found"),
        (403, "manager_forbidden"),
        (401, "manager_sign_in"),
        (500, "manager_unreachable"),
        (503, "manager_unreachable"),
        (307, "manager_unreachable"),
    ],
)
def test_each_failure_is_classified_by_status_and_vaults_text_is_dropped(manager_http, status: int, reason: str) -> None:
    manager_http.add("GET", _URL, status=status, json={"errors": ["vault-own-words"]})
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS[reason])
    assert "vault-own-words" not in str(excinfo.value)


def test_a_missing_key_does_not_exist_and_a_non_text_value_is_not_a_single_text(manager_http) -> None:
    manager_http.add("GET", _URL, json=_kv2({"other": "x"}), times=1)
    with pytest.raises(SecretRefRejected, match="does not exist"):
        resolve_secret_ref(_REF)
    for not_text in (7, {"a": 1}, ["x"], None, True):
        manager_http.add("GET", _URL, json=_kv2({"password": not_text}), times=1)
        with pytest.raises(SecretRefRejected, match="not a single text"):
            resolve_secret_ref(_REF)


def test_an_address_nobody_answers_is_unreachable(manager_http) -> None:
    with pytest.raises(SecretRefRejected, match="cannot be reached") as excinfo:
        resolve_secret_ref(_REF)
    assert "vault.test" not in str(excinfo.value).split(": ", 1)[1]


def test_no_token_and_no_approle_is_may_not_sign_in(manager_http, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("VAULT_TOKEN")
    with pytest.raises(ManagerFailure) as excinfo:
        secret_manager.read_secret("vault", secret_manager.parse_ref("vault", _REF))
    assert excinfo.value.reason == "manager_sign_in"
    assert manager_http.calls == []
