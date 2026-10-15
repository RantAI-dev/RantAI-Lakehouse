"""`SRC-12` part B4: the Google Secret Manager reader against recorded
responses of the documented interface (JWT-bearer token exchange, `:access`).
No network, and *not run against the real service*; requests go through the
`manager_http` fake."""

from __future__ import annotations

import base64
import json
from pathlib import Path

import pytest
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa

from dispar_orchestrate import secret_manager
from dispar_orchestrate.secret_resolver import SecretRefRejected, resolve_secret_ref

_TOKEN_URI = "https://oauth2.test/token"
_ACCESS_URL = "https://secretmanager.googleapis.com/v1/projects/my-project-1/secrets/orders-db/versions/latest:access"
_REF = "gcp-sm:projects/my-project-1/secrets/orders-db/versions/latest"

_KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)


def _key_file(tmp_path: Path, **overrides: object) -> Path:
    pem = _KEY.private_bytes(
        serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()
    ).decode()
    body = {
        "type": "service_account",
        "client_email": "reader@my-project-1.iam.gserviceaccount.com",
        "private_key": pem,
        "token_uri": _TOKEN_URI,
        **overrides,
    }
    path = tmp_path / "sa.json"
    path.write_text(json.dumps(body))
    return path


@pytest.fixture(autouse=True)
def gcp_env(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv("SECRET_MANAGER", "gcp")
    monkeypatch.setenv("GOOGLE_APPLICATION_CREDENTIALS", str(_key_file(tmp_path)))
    monkeypatch.delenv("GCE_METADATA_HOST", raising=False)


def _b64(text: str) -> str:
    return base64.b64encode(text.encode()).decode()


def _routes(manager_http, secret: str = "pw-from-gcp", token: str = "ya29.tok") -> None:
    manager_http.add("POST", _TOKEN_URI, json={"access_token": token, "expires_in": 3599, "token_type": "Bearer"})
    manager_http.add(
        "GET",
        _ACCESS_URL,
        json={"name": "projects/1/secrets/orders-db/versions/3", "payload": {"data": _b64(secret)}},
    )


def test_exchanges_a_signed_assertion_then_reads_the_version(manager_http) -> None:
    _routes(manager_http)
    assert resolve_secret_ref(_REF) == "pw-from-gcp"
    token_call, read_call = manager_http.calls
    form = token_call["data"]
    assert form["grant_type"] == "urn:ietf:params:oauth:grant-type:jwt-bearer"
    header, claims, signature = form["assertion"].split(".")
    pad = lambda part: part + "=" * (-len(part) % 4)  # noqa: E731
    assert json.loads(base64.urlsafe_b64decode(pad(header))) == {"alg": "RS256", "typ": "JWT"}
    decoded = json.loads(base64.urlsafe_b64decode(pad(claims)))
    assert decoded["iss"] == "reader@my-project-1.iam.gserviceaccount.com"
    assert decoded["aud"] == _TOKEN_URI
    assert decoded["scope"] == "https://www.googleapis.com/auth/cloud-platform"
    assert decoded["exp"] - decoded["iat"] == 3600
    # The signature verifies with the key file's public half.
    _KEY.public_key().verify(
        base64.urlsafe_b64decode(pad(signature)), f"{header}.{claims}".encode(), padding.PKCS1v15(), hashes.SHA256()
    )
    assert read_call["headers"] == {"Authorization": "Bearer ya29.tok"}
    assert read_call["allow_redirects"] is False


def test_every_read_exchanges_and_reads_again(manager_http) -> None:
    manager_http.add("POST", _TOKEN_URI, json={"access_token": "t"})
    manager_http.add("GET", _ACCESS_URL, json={"payload": {"data": _b64("first")}}, times=1)
    manager_http.add("GET", _ACCESS_URL, json={"payload": {"data": _b64("second")}})
    assert resolve_secret_ref(_REF) == "first"
    assert resolve_secret_ref(_REF) == "second"
    assert len([c for c in manager_http.calls if c["method"] == "POST"]) == 2


def test_a_version_number_reads_that_version(manager_http) -> None:
    manager_http.add("POST", _TOKEN_URI, json={"access_token": "t"})
    manager_http.add(
        "GET",
        "https://secretmanager.googleapis.com/v1/projects/my-project-1/secrets/orders_db/versions/7:access",
        json={"payload": {"data": _b64("v7")}},
    )
    assert resolve_secret_ref("gcp-sm:projects/my-project-1/secrets/orders_db/versions/7") == "v7"


@pytest.mark.parametrize(
    ("status", "reason"),
    [
        (404, "manager_not_found"),
        (400, "manager_not_found"),
        (403, "manager_forbidden"),
        (401, "manager_sign_in"),
        (500, "manager_unreachable"),
        (307, "manager_unreachable"),
    ],
)
def test_each_read_failure_is_classified_by_status_and_googles_text_is_dropped(
    manager_http, status: int, reason: str
) -> None:
    manager_http.add("POST", _TOKEN_URI, json={"access_token": "t"})
    manager_http.add("GET", _ACCESS_URL, status=status, json={"error": {"message": "google-own-words"}})
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS[reason])
    assert "google-own-words" not in str(excinfo.value)


def test_a_refused_token_exchange_is_may_not_sign_in(manager_http) -> None:
    manager_http.add("POST", _TOKEN_URI, status=400, json={"error": "invalid_grant", "error_description": "own words"})
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS["manager_sign_in"])
    assert "own words" not in str(excinfo.value)


@pytest.mark.parametrize("bad", [{"type": "authorized_user"}, {"private_key": "not a pem"}, {"client_email": ""}])
def test_a_key_file_that_is_not_a_usable_service_account_is_may_not_sign_in(
    manager_http, monkeypatch: pytest.MonkeyPatch, tmp_path: Path, bad: dict
) -> None:
    monkeypatch.setenv("GOOGLE_APPLICATION_CREDENTIALS", str(_key_file(tmp_path, **bad)))
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS["manager_sign_in"])
    assert manager_http.calls == []


def test_a_missing_key_file_is_may_not_sign_in(manager_http, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv("GOOGLE_APPLICATION_CREDENTIALS", str(tmp_path / "absent.json"))
    with pytest.raises(SecretRefRejected, match="may not sign in"):
        resolve_secret_ref(_REF)


def test_without_a_key_file_the_metadata_identity_is_used(manager_http, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("GOOGLE_APPLICATION_CREDENTIALS")
    monkeypatch.setenv("GCE_METADATA_HOST", "metadata.test")
    manager_http.add(
        "GET",
        "http://metadata.test/computeMetadata/v1/instance/service-accounts/default/token",
        json={"access_token": "meta-tok", "expires_in": 3000},
    )
    manager_http.add("GET", _ACCESS_URL, json={"payload": {"data": _b64("via-metadata")}})
    assert resolve_secret_ref(_REF) == "via-metadata"
    meta_call, read_call = manager_http.calls
    assert meta_call["headers"] == {"Metadata-Flavor": "Google"}
    assert meta_call["timeout"] == 2
    assert read_call["headers"] == {"Authorization": "Bearer meta-tok"}


def test_no_key_file_and_no_metadata_service_is_may_not_sign_in(manager_http, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("GOOGLE_APPLICATION_CREDENTIALS")
    with pytest.raises(SecretRefRejected, match="may not sign in"):
        resolve_secret_ref(_REF)


def test_a_payload_that_is_not_text_is_not_a_single_text(manager_http) -> None:
    manager_http.add("POST", _TOKEN_URI, json={"access_token": "t"})
    manager_http.add("GET", _ACCESS_URL, json={"payload": {"data": base64.b64encode(b"\xff\xfe\x00").decode()}})
    with pytest.raises(SecretRefRejected, match="not a single text"):
        resolve_secret_ref(_REF)


def test_the_assertion_and_the_token_never_appear_in_an_error(manager_http) -> None:
    manager_http.add("POST", _TOKEN_URI, json={"access_token": "ya29.leaky"})
    manager_http.add("GET", _ACCESS_URL, status=403, json={})
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert "ya29.leaky" not in str(excinfo.value)
    assert "BEGIN" not in str(excinfo.value)
