"""Reading a connector's credential from the installation's secret manager --
the orchestrator's half of `SRC-12` part B (decisions 1 to 3 and 12; ADR 0002
addendum 6). The API's half is `rust/crates/lakehouse-api/src/secret_manager/`;
the two are kept the same on purpose, and `secret_manager_refs.json` (read by
both test suites) is what keeps the reference grammar, the allowlist and the
error texts from drifting.

One manager per installation, chosen by the operator with `SECRET_MANAGER`
(`vault`, `aws`, `gcp`, `azure`; unset means none), signed in to with the
operator's environment. A connector's credential may be a *reference*
(`vault:<mount>/<path>#<key>`, `aws-sm:<id>[#<key>]`,
`gcp-sm:projects/<p>/secrets/<n>/versions/<v>`, `azure-kv:<vault>/<name>[/<v>]`)
that names a place in that manager. The value is read at every use -- every
run start -- with no cache and no fallback to an older copy (decision 1): a
manager that cannot be reached fails the run with a fixed text.

What is secret: the value the manager returns, the operator's sign-in
settings (a Vault token or secret id, AWS keys, a service-account key, an
Azure client secret) and every token they produce. None is logged, returned
in an error or kept after the read. A reference (a path, a name, a version
id) is not secret. A manager's own error text is classified by status and
error code and then dropped; only the fixed texts below ever leave this
module (principle 4, decision 12).

Requests go only to addresses the operator set (`VAULT_ADDR`, the AWS
endpoint, the cloud providers' fixed hosts). A reference chooses a path or a
name, never a host; the one name that becomes part of a host (the Azure
vault) is limited to letters, digits and `-` by the grammar. Redirects are
not followed, so a sign-in token is never sent on to another server.
"""

from __future__ import annotations

import importlib
import logging
import os
import re
from collections.abc import Callable, Mapping
from typing import Any

import requests

_log = logging.getLogger(__name__)

KINDS: tuple[str, ...] = ("vault", "aws", "gcp", "azure")
_SCHEMES: dict[str, str] = {
    "vault": "vault:",
    "aws": "aws-sm:",
    "gcp": "gcp-sm:",
    "azure": "azure-kv:",
}

# Seconds one request to a manager may take (the API uses the same).
REQUEST_TIMEOUT = 15

# Fixed texts, word for word the API's `SecretFailure` (`lakehouse-core`); the
# shared fixture checks them from both sides.
TEXTS: dict[str, str] = {
    "wrong_key": "the stored credential cannot be decrypted with this installation's key",
    "key_unavailable": "this installation's connector secrets key is missing or invalid, so stored credentials cannot be read",
    "unreadable_file": "the stored credential file is not in a format this build reads",
    "manager_unreachable": "the secret manager cannot be reached",
    "manager_sign_in": "the product may not sign in to the secret manager",
    "manager_not_found": "the referenced secret does not exist in the secret manager",
    "manager_forbidden": "the product may not read the referenced secret",
    "not_single_text": "the referenced secret is not a single text value",
}


class ManagerFailure(Exception):
    """A read of the secret manager failed. The message is always one of
    `TEXTS`; `reason` is its key."""

    def __init__(self, reason: str) -> None:
        super().__init__(TEXTS[reason])
        self.reason = reason


class RefError(ValueError):
    """A reference that does not fit its manager's grammar."""


def configured_kind(environ: Mapping[str, str] | None = None) -> str | None:
    """The manager `SECRET_MANAGER` names, or None.

    An unknown value is None too (fail closed: no manager scheme is
    allowlisted); the API refuses to start on it, so the two never disagree
    for long.
    """
    env = os.environ if environ is None else environ
    raw = env.get("SECRET_MANAGER", "").strip()
    if not raw:
        return None
    if raw not in KINDS:
        _log.warning("SECRET_MANAGER is not one of %s; no secret manager reference is accepted", ", ".join(KINDS))
        return None
    return raw


def scheme_of(kind: str) -> str:
    return _SCHEMES[kind]


def kind_of_ref(secret_ref: str) -> str | None:
    """The manager a reference's scheme belongs to, if any (a pure predicate)."""
    for kind, scheme in _SCHEMES.items():
        if secret_ref.startswith(scheme):
            return kind
    return None


def allowed_patterns(kind: str | None) -> tuple[str, ...]:
    """The allowlist patterns the configured manager adds: its scheme and a
    star, and nothing when no manager is set. Twin of the API's
    `connector_allowed_patterns`; the grammar below, not the pattern, decides
    which strings under the scheme are references."""
    return () if kind is None else (f"{_SCHEMES[kind]}*",)


_MAX_REF_LEN = 1024
_SEGMENT = r"[A-Za-z0-9_.-]+"
_KEY_RE = re.compile(r"[A-Za-z0-9_.-]{1,128}")
_SEGMENT_RE = re.compile(_SEGMENT)
_AWS_ID_RE = re.compile(r"[A-Za-z0-9/_+=.@:-]{1,2048}")
_GCP_RE = re.compile(
    r"projects/([A-Za-z0-9_.:-]{1,64})/secrets/([A-Za-z0-9_-]{1,255})/versions/(latest|[0-9]{1,20})"
)
_AZURE_RE = re.compile(r"([A-Za-z0-9-]{3,24})/([A-Za-z0-9-]{1,127})(?:/([A-Za-z0-9]{1,64}))?")


def _is_segment(s: str) -> bool:
    return s not in (".", "..") and _SEGMENT_RE.fullmatch(s) is not None


def _is_key(s: str) -> bool:
    return _is_segment(s) and _KEY_RE.fullmatch(s) is not None


def parse_ref(kind: str, secret_ref: str) -> dict[str, str | None]:
    """Parse `secret_ref` as a reference of `kind`; the parts are named as in
    the shared fixture. Raises `RefError` for anything else."""
    if len(secret_ref) > _MAX_REF_LEN or not secret_ref.startswith(_SCHEMES[kind]):
        raise RefError
    rest = secret_ref[len(_SCHEMES[kind]) :]
    if kind == "vault":
        location, sep, key = rest.partition("#")
        mount, slash, path = location.partition("/")
        if not (sep and slash and _is_key(key) and _is_segment(mount)):
            raise RefError
        if not all(_is_segment(part) for part in path.split("/")):
            raise RefError
        return {"mount": mount, "path": path, "key": key}
    if kind == "aws":
        secret_id, sep, key = rest.partition("#")
        if _AWS_ID_RE.fullmatch(secret_id) is None or (sep and not _is_key(key)):
            raise RefError
        return {"secret_id": secret_id, "key": key if sep else None}
    if kind == "gcp":
        match = _GCP_RE.fullmatch(rest)
        if match is None:
            raise RefError
        return {"project": match[1], "secret": match[2], "version": match[3]}
    match = _AZURE_RE.fullmatch(rest)
    if match is None:
        raise RefError
    return {"vault": match[1], "secret": match[2], "version": match[3]}


def request_json(
    method: str,
    url: str,
    *,
    headers: Mapping[str, str] | None = None,
    json: Any = None,
    data: Any = None,
    sign_in: bool = False,
    timeout: float = REQUEST_TIMEOUT,
    not_found: tuple[int, ...] = (404,),
) -> Any:
    """One request to a manager, answered as parsed JSON or as a fixed failure.

    Every manager's calls go through here: bounded in time, redirects not
    followed (a sign-in token must not be sent on to another server), and the
    manager's own error text dropped. `sign_in` marks a call whose refusal
    means the product may not sign in (a login or token exchange) rather than
    that it may not read: 400/401/403 are then `manager_sign_in`. `not_found` lists the
    statuses that mean "no such secret" (a manager may use another besides
    404). `timeout` is shorter for a platform's metadata service, which must fail fast on a
    host that has none.
    """
    try:
        response = requests.request(
            method,
            url,
            headers=dict(headers or {}),
            json=json,
            data=data,
            timeout=timeout,
            allow_redirects=False,
        )
    except requests.RequestException:
        raise ManagerFailure("manager_unreachable") from None
    try:
        response.raise_for_status()
    except requests.HTTPError:
        raise ManagerFailure(_failure_for_status(response.status_code, sign_in, not_found)) from None
    if response.status_code >= 300:  # a redirect is not an answer
        raise ManagerFailure("manager_unreachable")
    try:
        return response.json()
    except ValueError:
        raise ManagerFailure("manager_unreachable") from None


def _failure_for_status(status: int, sign_in: bool, not_found: tuple[int, ...]) -> str:
    if sign_in:
        return "manager_sign_in" if status in (400, 401, 403) else "manager_unreachable"
    if status in not_found:
        return "manager_not_found"
    return {401: "manager_sign_in", 403: "manager_forbidden"}.get(status, "manager_unreachable")


def text_value(value: Any) -> str:
    """The secret as a text, or the fixed "not a single text" failure."""
    if isinstance(value, str):
        return value
    raise ManagerFailure("not_single_text")


# Each manager's reader lives in its own module (`secret_manager_<kind>.py`)
# with `read(parsed, environ) -> str`. `_READERS` lets a test put a fake in
# its place; `_MODULES` is what a build ships.
_MODULES: dict[str, str] = {
    "vault": "dispar_orchestrate.secret_manager_vault",
    "aws": "dispar_orchestrate.secret_manager_aws",
}
_READERS: dict[str, Callable[[Mapping[str, str | None], Mapping[str, str]], str]] = {}


def read_secret(kind: str, parsed: Mapping[str, str | None], environ: Mapping[str, str] | None = None) -> str:
    """Read the secret `parsed` names from the manager `kind`, now.

    Raises `ManagerFailure` with a fixed text, or `RefError` when this build
    has no client for `kind`. The caller trims the value.
    """
    reader = _READERS.get(kind)
    if reader is None:
        module = _MODULES.get(kind)
        if module is None:
            raise RefError
        reader = importlib.import_module(module).read
    return reader(parsed, os.environ if environ is None else environ)
