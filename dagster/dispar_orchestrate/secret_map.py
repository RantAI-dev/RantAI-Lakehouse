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

`mongodb`, `sftp` and `kafka` (the Tier 2 adapters,
`dagster/dispar_orchestrate/adapters/{mongodb,sftp,kafka}.py`) extend the
same table: `mongodb` needs only a `password` (its username is a literal
`dial` field, not a secret -- see `adapters/mongodb.py::build_source`),
`sftp`'s two auth types each need a DIFFERENT single field name
(`password` vs. `privateKey`, `adapters/sftp.py::_connect_kwargs`), and
`kafka`'s `"none"` auth type is the one entry in this table that needs
NO secret at all -- a `PLAINTEXT` broker has nothing to resolve, so
`resolve_secrets` below must return `{}` for it without ever calling
`resolve_secret_ref`, rather than resolving a `secretRef` the connector
was never required to set.
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
    ("mongodb", None): ("password",),
    ("kafka", "none"): (),
    ("kafka", "sasl_plain"): ("username", "password"),
    ("sftp", "password"): ("password",),
    ("sftp", "public_key"): ("privateKey",),
}

# Adapters whose secret-field count depends on `dial.auth.type` -- every
# other adapter's lookup key always pairs with `None`, even if a caller
# passes a stray auth_type for one (see
# `test_a_non_rest_adapter_ignores_the_auth_type_argument`).
_AUTH_TYPE_KEYED_ADAPTERS = ("rest", "kafka", "sftp")


def secret_field_names(adapter: str, auth_type: str | None) -> tuple[str, ...]:
    """The ordered secret field names `build_source` needs for this
    adapter (and, for `rest`/`kafka`/`sftp`, this `dial.auth.type`).

    Raises `ValueError` on an unknown combination -- a hard, immediate
    failure, never a silent empty tuple that would later degrade to a
    `KeyError` deep inside an adapter. An empty tuple IS a legitimate,
    recognized result (`("kafka", "none")`) -- distinct from an unknown
    combination, which is `None` here, never an empty tuple pretending to
    be "no secret needed".
    """
    key = (adapter, auth_type if adapter in _AUTH_TYPE_KEYED_ADAPTERS else None)
    fields = SECRET_FIELD_NAMES.get(key)
    if fields is None:
        raise ValueError(f"no secret field mapping for adapter={adapter!r} auth_type={auth_type!r}")
    return fields
