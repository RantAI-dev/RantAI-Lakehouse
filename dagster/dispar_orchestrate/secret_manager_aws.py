"""AWS Secrets Manager -- the orchestrator's reader (`SRC-12` part B3). Twin of
`rust/crates/lakehouse-api/src/secret_manager/aws.rs`; see `secret_manager.py`
for what is secret and what is never returned.

One signed `GetSecretValue` call (SigV4, written here with `hmac`/`hashlib`
instead of pulling in `boto3`: the interface is one POST, and the signer is
checked against AWS's published test vector in the tests). Credentials, in
this order and fetched at each read:

1. a task or pod role, when the platform's environment names its endpoint
   (`AWS_CONTAINER_CREDENTIALS_RELATIVE_URI` or `..._FULL_URI`);
2. access keys in the environment (`AWS_ACCESS_KEY_ID`,
   `AWS_SECRET_ACCESS_KEY`, optional `AWS_SESSION_TOKEN`);
3. the instance role through IMDSv2.

(The standard chain tries the keys first; a task role is signalled by an
environment variable that only the platform sets, so putting it first costs no
probe and keeps the plan's "role first" without a metadata timeout on a host
that has none.) `AWS_ENDPOINT_URL_SECRETS_MANAGER` points the call at a
stand-in (the gate's `moto`); the web-identity (IRSA) and profile-file links
of the chain are not built and are listed as limits.

A secret's `SecretString` is returned whole, or -- with `#key` -- the named
string field of its JSON. A binary secret, invalid JSON for a `#key`, or a
field that is not a string is "not a single text".
"""

from __future__ import annotations

import hashlib
import hmac
import json
from collections.abc import Mapping
from datetime import UTC, datetime
from typing import Any

import requests

from dispar_orchestrate.secret_manager import REQUEST_TIMEOUT, ManagerFailure, request_json

_IMDS_DEFAULT = "http://169.254.169.254"
_ECS_HOST = "http://169.254.170.2"
_IMDS_TIMEOUT = 2  # seconds: a host with no metadata service must fail fast

# Error codes (`__type`) of GetSecretValue, by what they mean to a user.
_SIGN_IN = {
    "UnrecognizedClientException",
    "InvalidSignatureException",
    "ExpiredTokenException",
    "IncompleteSignature",
    "InvalidClientTokenId",
    "MissingAuthenticationToken",
    "SignatureDoesNotMatch",
}
_FORBIDDEN = {"AccessDeniedException", "AccessDenied"}
_NOT_FOUND = {"ResourceNotFoundException", "InvalidRequestException"}


def _sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _hmac(key: bytes, message: str) -> bytes:
    return hmac.new(key, message.encode(), hashlib.sha256).digest()


def sign(
    *,
    method: str,
    host: str,
    path: str,
    query: str,
    extra_headers: Mapping[str, str],
    payload: bytes,
    region: str,
    service: str,
    access_key: str,
    secret_key: str,
    session_token: str | None,
    now: datetime,
) -> dict[str, str]:
    """The headers (including `Authorization`) that sign this request, SigV4."""
    amz_date = now.strftime("%Y%m%dT%H%M%SZ")
    date = amz_date[:8]
    signed = {"host": host, "x-amz-date": amz_date}
    signed.update({name.lower(): " ".join(value.split()) for name, value in extra_headers.items()})
    if session_token:
        signed["x-amz-security-token"] = session_token
    names = sorted(signed)
    canonical_headers = "".join(f"{name}:{signed[name]}\n" for name in names)
    signed_headers = ";".join(names)
    canonical_request = "\n".join(
        [method, path, query, canonical_headers, signed_headers, _sha256_hex(payload)]
    )
    scope = f"{date}/{region}/{service}/aws4_request"
    string_to_sign = "\n".join(["AWS4-HMAC-SHA256", amz_date, scope, _sha256_hex(canonical_request.encode())])
    key = _hmac(("AWS4" + secret_key).encode(), date)
    for part in (region, service, "aws4_request"):
        key = _hmac(key, part)
    signature = hmac.new(key, string_to_sign.encode(), hashlib.sha256).hexdigest()
    headers = {name: signed[name] for name in names if name != "host"}
    headers["Authorization"] = (
        f"AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    )
    return headers


def _credentials(environ: Mapping[str, str]) -> tuple[str, str, str | None]:
    relative = environ.get("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "").strip()
    full = environ.get("AWS_CONTAINER_CREDENTIALS_FULL_URI", "").strip()
    if relative or full:
        headers = {}
        token = environ.get("AWS_CONTAINER_AUTHORIZATION_TOKEN", "").strip()
        if token:
            headers["Authorization"] = token
        body = request_json("GET", full or f"{_ECS_HOST}{relative}", headers=headers, sign_in=True)
        return _from_credential_document(body)
    key = environ.get("AWS_ACCESS_KEY_ID", "").strip()
    secret = environ.get("AWS_SECRET_ACCESS_KEY", "").strip()
    if key and secret:
        return key, secret, environ.get("AWS_SESSION_TOKEN", "").strip() or None
    return _instance_role(environ.get("AWS_EC2_METADATA_SERVICE_ENDPOINT", "").strip().rstrip("/") or _IMDS_DEFAULT)


def _instance_role(base: str) -> tuple[str, str, str | None]:
    token = request_json_text("PUT", f"{base}/latest/api/token", {"X-aws-ec2-metadata-token-ttl-seconds": "60"})
    headers = {"X-aws-ec2-metadata-token": token}
    role = request_json_text("GET", f"{base}/latest/meta-data/iam/security-credentials/", headers).splitlines()
    if not role or not role[0].strip():
        raise ManagerFailure("manager_sign_in")
    body = request_json("GET", f"{base}/latest/meta-data/iam/security-credentials/{role[0].strip()}", headers=headers, sign_in=True)
    return _from_credential_document(body)


def request_json_text(method: str, url: str, headers: Mapping[str, str]) -> str:
    """IMDS answers its token and role name as plain text, not JSON."""
    try:
        response = requests.request(method, url, headers=dict(headers), timeout=_IMDS_TIMEOUT, allow_redirects=False)
        response.raise_for_status()
    except requests.RequestException:
        # No role to sign in with: the platform gave the product none.
        raise ManagerFailure("manager_sign_in") from None
    if response.status_code >= 300:
        raise ManagerFailure("manager_sign_in")
    return response.text


def _from_credential_document(body: Any) -> tuple[str, str, str | None]:
    if not isinstance(body, dict):
        raise ManagerFailure("manager_sign_in")
    key, secret = body.get("AccessKeyId"), body.get("SecretAccessKey")
    if not (isinstance(key, str) and key and isinstance(secret, str) and secret):
        raise ManagerFailure("manager_sign_in")
    token = body.get("Token")
    return key, secret, token if isinstance(token, str) and token else None


def read(parsed: Mapping[str, str | None], environ: Mapping[str, str]) -> str:
    region = (environ.get("AWS_REGION") or environ.get("AWS_DEFAULT_REGION") or "").strip()
    if not region:
        raise ManagerFailure("manager_sign_in")
    endpoint = environ.get("AWS_ENDPOINT_URL_SECRETS_MANAGER", "").strip().rstrip("/")
    if not endpoint:
        endpoint = f"https://secretsmanager.{region}.amazonaws.com"
    if not endpoint.startswith(("http://", "https://")):
        raise ManagerFailure("manager_unreachable")
    host = endpoint.split("://", 1)[1]
    access_key, secret_key, session_token = _credentials(environ)
    # Compact, byte for byte what the Rust client sends (SRC-12 review B2 BLOCKER 1).
    payload = json.dumps({"SecretId": parsed["secret_id"]}, separators=(",", ":")).encode()
    extra = {"content-type": "application/x-amz-json-1.1", "x-amz-target": "secretsmanager.GetSecretValue"}
    headers = sign(
        method="POST",
        host=host,
        path="/",
        query="",
        extra_headers=extra,
        payload=payload,
        region=region,
        service="secretsmanager",
        access_key=access_key,
        secret_key=secret_key,
        session_token=session_token,
        now=datetime.now(UTC),
    )
    # `sign` returns the signed headers (content-type and x-amz-target
    # among them) plus Authorization; nothing is added after signing.
    body = _call(endpoint, headers, payload)
    text = body.get("SecretString") if isinstance(body, dict) else None
    if not isinstance(text, str):
        raise ManagerFailure("not_single_text")  # binary, or nothing readable
    key = parsed.get("key")
    if key is None:
        return text
    try:
        document = json.loads(text)
    except ValueError:
        raise ManagerFailure("not_single_text") from None
    if not isinstance(document, dict) or key not in document:
        raise ManagerFailure("manager_not_found")
    value = document[key]
    if not isinstance(value, str):
        raise ManagerFailure("not_single_text")
    return value


def _call(endpoint: str, headers: Mapping[str, str], payload: bytes) -> Any:
    """POST the signed request; classify an error by its `__type`, then drop
    AWS's own message."""
    try:
        response = requests.request(
            "POST", endpoint + "/", headers=dict(headers), data=payload, timeout=REQUEST_TIMEOUT, allow_redirects=False
        )
    except requests.RequestException:
        raise ManagerFailure("manager_unreachable") from None
    if response.status_code == 200:
        try:
            return response.json()
        except ValueError:
            raise ManagerFailure("manager_unreachable") from None
    code = ""
    try:
        document = response.json()
        code = str(document.get("__type", "")).split("#")[-1] if isinstance(document, dict) else ""
    except ValueError:
        pass
    if code in _SIGN_IN or response.status_code == 401:
        raise ManagerFailure("manager_sign_in")
    if code in _FORBIDDEN or response.status_code == 403:
        raise ManagerFailure("manager_forbidden")
    if code in _NOT_FOUND or response.status_code == 404:
        raise ManagerFailure("manager_not_found")
    raise ManagerFailure("manager_unreachable")
