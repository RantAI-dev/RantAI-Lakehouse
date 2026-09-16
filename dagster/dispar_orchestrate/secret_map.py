"""The ONE mapping from `(adapter, auth type)` to the named secret fields
an adapter's `build_source` needs (WS3 plan review Z6).

An earlier revision built an environment-variable name by interpolating a
principal-chosen connector id (`f"CONNECTOR_{connector_id.upper()}_PASSWORD"`)
and read it with a silent `""` default -- bypassing the allowlist entirely
(the exact class of hole `lakehouse_core::secret::pattern_matches` and
`lakehouse_api::state::CONNECTOR_ALLOWED_SECRET_REF_PATTERNS` close
API-side) and turning a missing credential into an empty one rather than
an error. It was also incomplete: a `rest` connector needs `apiKey` /
`token` / `username`+`password` / `clientId`+`clientSecret` depending on
`dial.auth.type`, and a `files` connector needs `accessKey`+`secretKey` --
supplying only `password` meant every REST connector raised `KeyError` and
every files connector got `None` credentials.

Position 0 always maps to a connector's `secretRef` (primary); position 1,
when present, to `secretRefSecondary` -- the same two-slot shape
`rust/crates/lakehouse-store/src/connectors.rs`'s `ConnectorDialInfo`
already carries. Mirrored on the Rust side by
`ingest_spec::secret_field_names` (`rust/crates/lakehouse-store/src/ingest_spec.rs`,
same commit) and pinned against it exactly the way `column_gate.py` is
pinned against `cdc.rs::NESTED_TYPE_MARKERS` (X9's discipline) -- so a
`rest` connector saved with `auth.type = "basic"` but only one `secretRef`
set is rejected at SAVE time (`set_ingest_spec`'s validation), not
discovered as a `KeyError` at run time.
"""

from __future__ import annotations

SECRET_FIELD_NAMES: dict[tuple[str, str | None], tuple[str, ...]] = {
    ("sql", None): ("password",),
    ("cdc", None): ("password",),
    ("files", None): ("accessKey", "secretKey"),
    ("rest", "api_key"): ("apiKey",),
    ("rest", "bearer"): ("token",),
    ("rest", "basic"): ("username", "password"),
    ("rest", "oauth2_client_credentials"): ("clientId", "clientSecret"),
    ("sheets", None): ("serviceAccountJson",),
}


def secret_field_names(adapter: str, auth_type: str | None) -> tuple[str, ...]:
    """The ordered secret field names `build_source` needs for this
    adapter (and, for `rest`, this `dial.auth.type`).

    Raises `ValueError` on an unknown combination -- a hard, immediate
    failure, never a silent empty tuple that would later degrade to a
    `KeyError` deep inside an adapter.
    """
    key = (adapter, auth_type if adapter == "rest" else None)
    fields = SECRET_FIELD_NAMES.get(key)
    if fields is None:
        raise ValueError(f"no secret field mapping for adapter={adapter!r} auth_type={auth_type!r}")
    return fields
