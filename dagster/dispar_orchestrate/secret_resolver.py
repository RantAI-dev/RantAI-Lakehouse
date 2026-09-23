"""Resolves a connector's declared `secretRef`/`secretRefSecondary` (the
same fields `GET /api/connectors/{id}/ingest-spec` names but never
resolves) through a Python port of the credential-suffix allowlist rule
`AllowlistedSecretResolver` enforces API-side
(`rust/crates/lakehouse-core/src/secret.rs`'s `pattern_matches`,
`rust/crates/lakehouse-api/src/state.rs`'s
`CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`, both read in full).

Fails CLOSED: a future caller (a `dagster/dispar_orchestrate/ingest_factory.py`
this workstream has not yet added) turns a `SecretRefRejected` into a
`status="rejected"` `ingest_run` row, never a silent `""` -- the exact bug
this closes (WS3 plan review Z6): an earlier revision built an
environment-variable name by interpolating a
principal-chosen connector id and read it with `os.environ.get(name, "")`,
bypassing the allowlist entirely and turning a missing credential into an
empty one rather than an error. This module never derives an env-var name
from anything but the connector's own declared `secretRef`, and the
allowlist check runs before any read.

`file:` refs are resolved the SAME way `FileSecretResolver` does
API-side: canonicalize the base directory and the requested path, require
the latter under the former. Honest gap, stated once: as of this
workstream, `docker-compose.yml`'s `dagster-code-location` service mounts
no `/run/secrets` volume (verified: `grep -n "run/secrets"
docker-compose.yml` returns nothing) -- a `file:`-scheme secretRef is
therefore syntactically allowlisted but will always fail to resolve (the
file genuinely does not exist in this container) until an operator adds
that mount, exactly the "unsupported, honestly" posture AGENTS.md prefers
over pretending it works (AGENTS.md principle 2).

Never log a resolved value -- only the ref NAME appears in any exception
message here, matching `secret.rs`'s `SecretValue` guarantee that a
resolved credential is never `Debug`-printed or logged.
"""

from __future__ import annotations

import os
from pathlib import Path

from dispar_orchestrate.secret_map import secret_field_names

# Ported verbatim from rust/crates/lakehouse-api/src/state.rs's
# CONNECTOR_ALLOWED_SECRET_REF_PATTERNS -- keep both lists identical; a
# Rust-side change must land in the SAME commit as a change here (same
# discipline as column_gate.py's NESTED_TYPE_MARKERS, X9).
CONNECTOR_ALLOWED_SECRET_REF_PATTERNS: tuple[str, ...] = (
    "env:CONNECTOR_*_PASSWORD",
    "env:CONNECTOR_*_SECRET_KEY",
    "env:CONNECTOR_*_ACCESS_KEY",
    "env:CONNECTOR_*_API_KEY",
    "env:CONNECTOR_*_TOKEN",
    "file:/run/secrets/connector_*",
)

_SECRETS_BASE_DIR = "/run/secrets"


class SecretRefRejected(Exception):
    """A connector's declared secretRef is missing, not allowlisted, or
    (for file: refs) does not resolve under the secrets base directory."""


def _pattern_matches(pattern: str, value: str) -> bool:
    """Ported verbatim from `rust/crates/lakehouse-core/src/secret.rs`'s
    `pattern_matches`: exactly one `*` splitting a fixed prefix and a
    fixed suffix; zero or more than one `*` never matches."""
    if pattern.count("*") != 1:
        return False
    prefix, suffix = pattern.split("*", 1)
    return len(value) >= len(prefix) + len(suffix) and value.startswith(prefix) and value.endswith(suffix)


def resolve_secret_ref(secret_ref: str | None) -> str:
    """Resolve one `secretRef` string to its credential value.

    Never a silent default: a missing ref, a ref matching none of
    `CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`, an allowlisted-but-unset env
    var, or an escaping `file:` path all raise `SecretRefRejected` --
    there is no code path here that returns `""`.
    """
    if not secret_ref:
        raise SecretRefRejected("no secretRef configured")
    if not any(_pattern_matches(p, secret_ref) for p in CONNECTOR_ALLOWED_SECRET_REF_PATTERNS):
        raise SecretRefRejected(f"secretRef {secret_ref!r} is not an allowlisted credential-shaped name")
    if secret_ref.startswith("env:"):
        name = secret_ref[len("env:") :]
        value = os.environ.get(name, "")
        if not value:
            raise SecretRefRejected(f"secretRef {secret_ref!r} is allowlisted but unset in this environment")
        return value
    if secret_ref.startswith("file:"):
        raw_path = secret_ref[len("file:") :]
        try:
            base = Path(_SECRETS_BASE_DIR).resolve(strict=True)
            target = Path(raw_path).resolve(strict=True)
        except OSError as exc:
            raise SecretRefRejected(f"secretRef {secret_ref!r} does not resolve to a real file: {exc}") from exc
        if target != base and base not in target.parents:
            raise SecretRefRejected(f"secretRef {secret_ref!r} escapes the secrets base directory")
        return target.read_text().strip()
    raise SecretRefRejected(f"secretRef {secret_ref!r} has an unsupported scheme")


def resolve_secrets(
    adapter: str,
    auth_type: str | None,
    primary: str | None,
    secondary: str | None,
) -> dict[str, str]:
    """Resolve BOTH slots (when the `(adapter, auth type)` mapping needs
    two) and zip them with `secret_map.secret_field_names` -- the single
    call site a future `ingest_factory.py` (WS3 plan review Z6) uses.

    Raises `ValueError` if `(adapter, auth_type)` names no mapping
    (`secret_field_names`), or `SecretRefRejected` (never a partial dict)
    if either required slot fails to resolve.

    A zero-length `fields` tuple (`("kafka", "none")` -- a `PLAINTEXT`
    broker with nothing to resolve) returns `{}` immediately, without
    calling `resolve_secret_ref` at all: no secretRef is REQUIRED when no
    secret field is needed, so a connector saved with `secretRef` unset
    for this combination must not be rejected as if a credential were
    missing.
    """
    fields = secret_field_names(adapter, auth_type)
    if not fields:
        return {}
    values = [resolve_secret_ref(primary)]
    if len(fields) == 2:
        values.append(resolve_secret_ref(secondary))
    return dict(zip(fields, values))
