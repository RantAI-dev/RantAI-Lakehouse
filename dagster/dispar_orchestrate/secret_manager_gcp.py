"""Google Secret Manager -- the orchestrator's reader (`SRC-12` part B4). Twin of
`rust/crates/lakehouse-api/src/secret_manager/gcp.rs`; see `secret_manager.py`
for what is secret and what is never returned.

`GET https://secretmanager.googleapis.com/v1/{secret version}:access` with an
OAuth access token, read at every use. The token comes from

1. the service-account key file at `GOOGLE_APPLICATION_CREDENTIALS`: a
   self-signed RS256 JWT exchanged at the key's `token_uri` (the documented
   JWT-bearer grant; signed with `cryptography`, already a dependency, so no
   Google client library is pulled in), or
2. the platform's metadata identity (`GCE_METADATA_HOST`, default
   `metadata.google.internal`), when no key file is set.

Only `type: service_account` key files are read; other credential types
(external accounts, user credentials) are refused as "may not sign in". The
project is part of the reference, never of the environment. This reader is
tested against recorded responses of the documented interface and has *not*
been run against the real service (decision 3).
"""

from __future__ import annotations

import base64
import json
import time
from collections.abc import Mapping
from pathlib import Path

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding

from dispar_orchestrate.secret_manager import ManagerFailure, request_json, text_value

_API_BASE = "https://secretmanager.googleapis.com"
_DEFAULT_TOKEN_URI = "https://oauth2.googleapis.com/token"
_SCOPE = "https://www.googleapis.com/auth/cloud-platform"
_METADATA_TIMEOUT = 2  # seconds: a host with no metadata service must fail fast


def _b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def signed_assertion(client_email: str, private_key_pem: str, token_uri: str, now: int) -> str:
    """The RS256 JWT a service account presents at `token_uri`."""
    header = _b64url(json.dumps({"alg": "RS256", "typ": "JWT"}, separators=(",", ":")).encode())
    claims = _b64url(
        json.dumps(
            {"iss": client_email, "scope": _SCOPE, "aud": token_uri, "iat": now, "exp": now + 3600},
            separators=(",", ":"),
        ).encode()
    )
    try:
        key = serialization.load_pem_private_key(private_key_pem.encode(), password=None)
        signature = key.sign(f"{header}.{claims}".encode(), padding.PKCS1v15(), hashes.SHA256())  # type: ignore[call-arg,union-attr]
    except (ValueError, TypeError, AttributeError):
        raise ManagerFailure("manager_sign_in") from None
    return f"{header}.{claims}.{_b64url(signature)}"


def _token_from_service_account(path: str) -> str:
    try:
        key_file = json.loads(Path(path).read_text())
    except (OSError, ValueError):
        raise ManagerFailure("manager_sign_in") from None
    if not isinstance(key_file, dict) or key_file.get("type") != "service_account":
        raise ManagerFailure("manager_sign_in")
    email, pem = key_file.get("client_email"), key_file.get("private_key")
    if not (isinstance(email, str) and email and isinstance(pem, str) and pem):
        raise ManagerFailure("manager_sign_in")
    token_uri = key_file.get("token_uri") or _DEFAULT_TOKEN_URI
    body = request_json(
        "POST",
        token_uri,
        data={
            "grant_type": "urn:ietf:params:oauth:grant-type:jwt-bearer",
            "assertion": signed_assertion(email, pem, token_uri, int(time.time())),
        },
        sign_in=True,
    )
    return _access_token(body)


def _access_token(body: object) -> str:
    token = body.get("access_token") if isinstance(body, dict) else None
    if not isinstance(token, str) or not token:
        raise ManagerFailure("manager_sign_in")
    return token


def _token_from_metadata(host: str) -> str:
    # No answer from the metadata service means the platform gave the product
    # no identity, which is "may not sign in", not an unreachable manager.
    try:
        body = request_json(
            "GET",
            f"http://{host}/computeMetadata/v1/instance/service-accounts/default/token",
            headers={"Metadata-Flavor": "Google"},
            sign_in=True,
            timeout=_METADATA_TIMEOUT,
        )
    except ManagerFailure:
        raise ManagerFailure("manager_sign_in") from None
    return _access_token(body)


def read(parsed: Mapping[str, str | None], environ: Mapping[str, str]) -> str:
    key_path = environ.get("GOOGLE_APPLICATION_CREDENTIALS", "").strip()
    if key_path:
        token = _token_from_service_account(key_path)
    else:
        token = _token_from_metadata(environ.get("GCE_METADATA_HOST", "").strip() or "metadata.google.internal")
    # project, secret and version are validated name parts (the grammar in
    # secret_manager.py), so the path stays under /v1/projects/.
    url = (
        f"{_API_BASE}/v1/projects/{parsed['project']}/secrets/{parsed['secret']}"
        f"/versions/{parsed['version']}:access"
    )
    # A disabled or destroyed version answers 400 (FAILED_PRECONDITION): it
    # cannot be read, which to a user is "does not exist".
    body = request_json("GET", url, headers={"Authorization": f"Bearer {token}"}, not_found=(400, 404))
    data = ((body or {}).get("payload") or {}).get("data") if isinstance(body, dict) else None
    if not isinstance(data, str):
        raise ManagerFailure("manager_not_found")
    try:
        return text_value(base64.b64decode(data, validate=True).decode())
    except (ValueError, UnicodeDecodeError):
        raise ManagerFailure("not_single_text") from None
