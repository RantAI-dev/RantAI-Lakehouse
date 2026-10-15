"""`SRC-12` part B3: the AWS Secrets Manager reader -- the SigV4 signer against
AWS's published test vector and a signature `botocore` produced for the same
request, then the call against recorded `GetSecretValue` answers. No network;
requests go through the `manager_http` fake."""

from __future__ import annotations

import json
from datetime import UTC, datetime

import pytest

from dispar_orchestrate import secret_manager
from dispar_orchestrate.secret_manager_aws import sign
from dispar_orchestrate.secret_resolver import SecretRefRejected, resolve_secret_ref

_ENDPOINT = "http://aws.test:4566"
_REF = "aws-sm:prod/orders-db#password"


def test_the_signer_matches_the_published_get_vanilla_vector() -> None:
    # From AWS's SigV4 test suite ("get-vanilla"); the same constants are
    # asserted by the Rust signer's test.
    headers = sign(
        method="GET",
        host="example.amazonaws.com",
        path="/",
        query="",
        extra_headers={},
        payload=b"",
        region="us-east-1",
        service="service",
        access_key="AKIDEXAMPLE",
        secret_key="wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        session_token=None,
        now=datetime(2015, 8, 30, 12, 36, 0, tzinfo=UTC),
    )
    assert headers["Authorization"] == (
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, "
        "SignedHeaders=host;x-amz-date, "
        "Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
    )


def test_the_signer_matches_what_botocore_signed_for_the_same_secrets_manager_request() -> None:
    # Produced once with `botocore.auth.SigV4Auth` for exactly these inputs
    # (fixed clock, POST with a session token, COMPACT payload as the Rust
    # client sends it: SRC-12 review B2 BLOCKER 1, the first constant had been
    # made over a spaced payload); botocore is not a dependency.
    headers = sign(
        method="POST",
        host="secretsmanager.eu-west-1.amazonaws.com",
        path="/",
        query="",
        extra_headers={"content-type": "application/x-amz-json-1.1", "x-amz-target": "secretsmanager.GetSecretValue"},
        payload=json.dumps({"SecretId": "prod/x"}, separators=(",", ":")).encode(),
        region="eu-west-1",
        service="secretsmanager",
        access_key="AKID",
        secret_key="SECRET",
        session_token="TOK",
        now=datetime(2026, 10, 15, 10, 0, 0, tzinfo=UTC),
    )
    assert headers["Authorization"].endswith(
        "Signature=7e592597a21f89557ee9425eb46abc7d213f3270168424e38baf9df8f7865a6b"
    )
    assert headers["x-amz-security-token"] == "TOK"


@pytest.fixture(autouse=True)
def aws_env(monkeypatch: pytest.MonkeyPatch) -> None:
    for name in (
        "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
        "AWS_CONTAINER_CREDENTIALS_FULL_URI",
        "AWS_CONTAINER_AUTHORIZATION_TOKEN",
        "AWS_SESSION_TOKEN",
        "AWS_EC2_METADATA_SERVICE_ENDPOINT",
        "AWS_DEFAULT_REGION",
    ):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("SECRET_MANAGER", "aws")
    # SRC-12 review B1 BLOCKER 1: references are accepted only under a prefix.
    monkeypatch.setenv("SECRET_MANAGER_ALLOWED_PREFIXES", "aws-sm:")
    monkeypatch.setenv("AWS_REGION", "eu-west-1")
    monkeypatch.setenv("AWS_ACCESS_KEY_ID", "AKIDTEST")
    monkeypatch.setenv("AWS_SECRET_ACCESS_KEY", "secret-access-key-test")
    monkeypatch.setenv("AWS_ENDPOINT_URL_SECRETS_MANAGER", _ENDPOINT)


def _answer(manager_http, secret_string: str | None = "plain-text", **extra) -> None:
    body = {"Name": "prod/orders-db", "VersionId": "v1", **extra}
    if secret_string is not None:
        body["SecretString"] = secret_string
    manager_http.add("POST", _ENDPOINT + "/", json=body)


def test_reads_a_json_field_with_a_signed_request(manager_http) -> None:
    _answer(manager_http, json.dumps({"password": "pw-from-aws", "user": "u"}))
    assert resolve_secret_ref(_REF) == "pw-from-aws"
    (call,) = manager_http.calls
    assert json.loads(call["data"]) == {"SecretId": "prod/orders-db"}
    headers = call["headers"]
    assert headers["x-amz-target"] == "secretsmanager.GetSecretValue"
    assert headers["content-type"] == "application/x-amz-json-1.1"
    assert headers["Authorization"].startswith("AWS4-HMAC-SHA256 Credential=AKIDTEST/")
    assert "secretsmanager/aws4_request" in headers["Authorization"]
    assert "secret-access-key-test" not in json.dumps(headers)
    assert call["allow_redirects"] is False


def test_without_a_key_the_whole_secret_text_is_the_value(manager_http) -> None:
    _answer(manager_http, "just-a-password")
    assert resolve_secret_ref("aws-sm:prod/orders-db") == "just-a-password"


def test_every_read_goes_to_aws_and_a_changed_secret_is_seen_at_once(manager_http) -> None:
    manager_http.add("POST", _ENDPOINT + "/", json={"SecretString": "old"}, times=1)
    manager_http.add("POST", _ENDPOINT + "/", json={"SecretString": "new"})
    assert resolve_secret_ref("aws-sm:prod/orders-db") == "old"
    assert resolve_secret_ref("aws-sm:prod/orders-db") == "new"


def test_a_session_token_is_signed_and_sent(manager_http, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("AWS_SESSION_TOKEN", "session-tok")
    _answer(manager_http, "x")
    resolve_secret_ref("aws-sm:prod/orders-db")
    headers = manager_http.calls[0]["headers"]
    assert headers["x-amz-security-token"] == "session-tok"
    assert "x-amz-security-token" in headers["Authorization"]


@pytest.mark.parametrize(
    ("status", "code", "reason"),
    [
        (400, "com.amazon.coral.service#ResourceNotFoundException", "manager_not_found"),
        (400, "InvalidRequestException", "manager_not_found"),
        (400, "AccessDeniedException", "manager_forbidden"),
        (403, "AccessDeniedException", "manager_forbidden"),
        (400, "UnrecognizedClientException", "manager_sign_in"),
        (400, "ExpiredTokenException", "manager_sign_in"),
        (500, "InternalServiceError", "manager_unreachable"),
    ],
)
def test_each_failure_is_classified_by_its_error_type_and_awss_text_is_dropped(
    manager_http, status: int, code: str, reason: str
) -> None:
    manager_http.add("POST", _ENDPOINT + "/", status=status, json={"__type": code, "message": "aws-own-words"})
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS[reason])
    assert "aws-own-words" not in str(excinfo.value)


def test_a_binary_secret_a_non_json_text_for_a_key_and_a_non_string_field_are_not_a_single_text(
    manager_http,
) -> None:
    _answer(manager_http, None, SecretBinary="AAEC")
    with pytest.raises(SecretRefRejected, match="not a single text"):
        resolve_secret_ref(_REF)
    manager_http.add("POST", _ENDPOINT + "/", json={"SecretString": "not json"})
    with pytest.raises(SecretRefRejected, match="not a single text"):
        resolve_secret_ref(_REF)
    manager_http.add("POST", _ENDPOINT + "/", json={"SecretString": json.dumps({"password": 7})})
    with pytest.raises(SecretRefRejected, match="not a single text"):
        resolve_secret_ref(_REF)


def test_a_missing_field_does_not_exist(manager_http) -> None:
    _answer(manager_http, json.dumps({"user": "u"}))
    with pytest.raises(SecretRefRejected, match="does not exist"):
        resolve_secret_ref(_REF)


def test_a_task_role_is_used_before_environment_keys(manager_http, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("AWS_CONTAINER_CREDENTIALS_FULL_URI", "http://creds.test/role")
    monkeypatch.setenv("AWS_CONTAINER_AUTHORIZATION_TOKEN", "auth-tok")
    manager_http.add(
        "GET",
        "http://creds.test/role",
        json={"AccessKeyId": "AKIDROLE", "SecretAccessKey": "role-secret", "Token": "role-token"},
    )
    _answer(manager_http, "x")
    resolve_secret_ref("aws-sm:prod/orders-db")
    creds_call, read_call = manager_http.calls
    assert creds_call["headers"] == {"Authorization": "auth-tok"}
    assert read_call["headers"]["Authorization"].startswith("AWS4-HMAC-SHA256 Credential=AKIDROLE/")
    assert read_call["headers"]["x-amz-security-token"] == "role-token"


def test_the_instance_role_through_imds_when_there_are_no_keys(
    manager_http, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("AWS_ACCESS_KEY_ID")
    monkeypatch.delenv("AWS_SECRET_ACCESS_KEY")
    monkeypatch.setenv("AWS_EC2_METADATA_SERVICE_ENDPOINT", "http://imds.test")
    manager_http.add("PUT", "http://imds.test/latest/api/token", text="imds-token")
    manager_http.add("GET", "http://imds.test/latest/meta-data/iam/security-credentials/", text="role-name\n")
    manager_http.add(
        "GET",
        "http://imds.test/latest/meta-data/iam/security-credentials/role-name",
        json={"AccessKeyId": "AKIDIMDS", "SecretAccessKey": "imds-secret", "Token": "imds-token-2"},
    )
    _answer(manager_http, "x")
    assert resolve_secret_ref("aws-sm:prod/orders-db") == "x"
    assert manager_http.calls[1]["headers"] == {"X-aws-ec2-metadata-token": "imds-token"}
    assert "AKIDIMDS" in manager_http.calls[-1]["headers"]["Authorization"]


def test_no_keys_and_no_metadata_service_is_may_not_sign_in(manager_http, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("AWS_ACCESS_KEY_ID")
    monkeypatch.delenv("AWS_SECRET_ACCESS_KEY")
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS["manager_sign_in"])


def test_an_endpoint_nobody_answers_is_unreachable(manager_http) -> None:
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(_REF)
    assert str(excinfo.value).endswith(secret_manager.TEXTS["manager_unreachable"])
    assert "aws.test" not in str(excinfo.value)
