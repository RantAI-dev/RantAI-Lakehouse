"""`HashiCorp` Vault, KV version 2 -- the orchestrator's reader (`SRC-12` part
B2). Twin of `rust/crates/lakehouse-api/src/secret_manager/vault.rs`; see
`secret_manager.py` for what is secret and what is never returned.

`GET {VAULT_ADDR}/v1/{mount}/data/{path}` with the operator's `VAULT_TOKEN`,
or with a token from an AppRole sign-in (`VAULT_ROLE_ID` + `VAULT_SECRET_ID`).
The token is used for one read and dropped. Vault answers 403 both for a token
it does not accept and for a policy that does not allow the path; a 403 on a
read is therefore "may not read it", a refused AppRole sign-in is "may not
sign in", and a missing key inside an existing secret is "does not exist".
"""

from __future__ import annotations

from collections.abc import Mapping

from dispar_orchestrate.secret_manager import ManagerFailure, request_json, text_value


def _token(addr: str, environ: Mapping[str, str]) -> str:
    token = environ.get("VAULT_TOKEN", "").strip()
    if token:
        return token
    role_id = environ.get("VAULT_ROLE_ID", "").strip()
    secret_id = environ.get("VAULT_SECRET_ID", "").strip()
    if not (role_id and secret_id):
        raise ManagerFailure("manager_sign_in")
    body = request_json(
        "POST",
        f"{addr}/v1/auth/approle/login",
        json={"role_id": role_id, "secret_id": secret_id},
        sign_in=True,
    )
    issued = ((body or {}).get("auth") or {}).get("client_token") if isinstance(body, dict) else None
    if not isinstance(issued, str) or not issued:
        raise ManagerFailure("manager_sign_in")
    return issued


def read(parsed: Mapping[str, str | None], environ: Mapping[str, str]) -> str:
    addr = environ.get("VAULT_ADDR", "").strip().rstrip("/")
    if not addr.startswith(("http://", "https://")):
        raise ManagerFailure("manager_unreachable")
    token = _token(addr, environ)
    # `mount` and `path` are name segments (the grammar in secret_manager.py
    # allows no `..`, `?`, `#` or empty segment), so this stays under /v1/.
    body = request_json(
        "GET",
        f"{addr}/v1/{parsed['mount']}/data/{parsed['path']}",
        headers={"X-Vault-Token": token},
    )
    fields = ((body or {}).get("data") or {}).get("data") if isinstance(body, dict) else None
    if not isinstance(fields, dict) or parsed["key"] not in fields:
        raise ManagerFailure("manager_not_found")
    return text_value(fields[parsed["key"]])
