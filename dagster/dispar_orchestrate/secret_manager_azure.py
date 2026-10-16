"""Azure Key Vault -- the orchestrator's reader (`SRC-12` part B5). Twin of
`rust/crates/lakehouse-api/src/secret_manager/azure.rs`; see
`secret_manager.py` for what is secret and what is never returned.

`GET https://{vault}.vault.azure.net/secrets/{name}[/{version}]?api-version=7.4`
with an Entra ID access token for `https://vault.azure.net`, read at every
use. The token comes from

1. an app registration (`AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
   `AZURE_CLIENT_SECRET`): the client-credentials grant at
   `AZURE_AUTHORITY_HOST` (default `https://login.microsoftonline.com`), or
2. a managed identity: the App Service / Functions endpoint
   (`IDENTITY_ENDPOINT` with `IDENTITY_HEADER`) or the instance metadata
   service; `AZURE_CLIENT_ID` alone selects a user-assigned identity.

Written as two small HTTP calls instead of pulling in `azure-identity` and
`azure-keyvault-secrets`. The vault name becomes a host name, so the reference
grammar limits it to letters, digits and `-` (3 to 24 characters). Only the
public cloud's `vault.azure.net` is built (a limit). Tested against recorded
responses of the documented interface and *not run against the real service*
(decision 3).
"""

from __future__ import annotations

from collections.abc import Mapping
from urllib.parse import quote

from dispar_orchestrate.secret_manager import ManagerFailure, request_json, text_value

_RESOURCE = "https://vault.azure.net"
_DEFAULT_AUTHORITY = "https://login.microsoftonline.com"
_IMDS = "http://169.254.169.254"
_METADATA_TIMEOUT = 2  # seconds: a host with no identity service must fail fast
_API_VERSION = "7.4"


def _access_token(body: object) -> str:
    token = body.get("access_token") if isinstance(body, dict) else None
    if not isinstance(token, str) or not token:
        raise ManagerFailure("manager_sign_in")
    return token


def _token(environ: Mapping[str, str]) -> str:
    tenant = environ.get("AZURE_TENANT_ID", "").strip()
    client_id = environ.get("AZURE_CLIENT_ID", "").strip()
    client_secret = environ.get("AZURE_CLIENT_SECRET", "").strip()
    if tenant and client_id and client_secret:
        authority = (environ.get("AZURE_AUTHORITY_HOST", "").strip() or _DEFAULT_AUTHORITY).rstrip("/")
        body = request_json(
            "POST",
            f"{authority}/{quote(tenant, safe='')}/oauth2/v2.0/token",
            data={
                "grant_type": "client_credentials",
                "client_id": client_id,
                "client_secret": client_secret,
                "scope": f"{_RESOURCE}/.default",
            },
            sign_in=True,
        )
        return _access_token(body)
    # No app registration: the platform's identity. No answer from the identity
    # service means the platform gave the product none ("may not sign in").
    endpoint = environ.get("IDENTITY_ENDPOINT", "").strip()
    header = environ.get("IDENTITY_HEADER", "").strip()
    try:
        if endpoint and header:
            params = {"resource": _RESOURCE, "api-version": "2019-08-01"}
            if client_id:
                params["client_id"] = client_id
            body = request_json(
                "GET",
                endpoint + "?" + "&".join(f"{k}={quote(v, safe='')}" for k, v in params.items()),
                headers={"X-IDENTITY-HEADER": header},
                sign_in=True,
                timeout=_METADATA_TIMEOUT,
            )
        else:
            query = f"api-version=2018-02-01&resource={quote(_RESOURCE, safe='')}"
            if client_id:
                query += f"&client_id={quote(client_id, safe='')}"
            body = request_json(
                "GET",
                f"{_IMDS}/metadata/identity/oauth2/token?{query}",
                headers={"Metadata": "true"},
                sign_in=True,
                timeout=_METADATA_TIMEOUT,
            )
    except ManagerFailure:
        raise ManagerFailure("manager_sign_in") from None
    return _access_token(body)


def read(parsed: Mapping[str, str | None], environ: Mapping[str, str]) -> str:
    token = _token(environ)
    # vault and secret are validated name parts (letters, digits, `-`), so the
    # host and the path cannot be steered anywhere else.
    url = f"https://{parsed['vault']}.vault.azure.net/secrets/{parsed['secret']}"
    if parsed.get("version"):
        url += f"/{parsed['version']}"
    body = request_json("GET", f"{url}?api-version={_API_VERSION}", headers={"Authorization": f"Bearer {token}"})
    value = body.get("value") if isinstance(body, dict) else None
    if value is None:
        raise ManagerFailure("manager_not_found")
    return text_value(value)
