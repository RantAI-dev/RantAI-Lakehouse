"""Tests for `secret_resolver` (WS3 plan review Z6) -- the
Python port of the allowlisted credential-suffix resolver
`lakehouse_core::secret::AllowlistedSecretResolver`/`pattern_matches`
enforces API-side. See `secret_resolver.py`'s module docstring for the
full trust model and the bug this replaces.
"""

from __future__ import annotations

import pytest

from dispar_orchestrate.secret_resolver import SecretRefRejected, resolve_secret_ref, resolve_secrets


def test_resolve_secret_ref_reads_an_allowlisted_env_var(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CONNECTOR_MYSQL_PASSWORD", "s3cret")
    assert resolve_secret_ref("env:CONNECTOR_MYSQL_PASSWORD") == "s3cret"


def test_resolve_secret_ref_resolves_a_derived_shaped_ref(monkeypatch: pytest.MonkeyPatch) -> None:
    # ADR 0002 Addendum 3 (`docs/adr/0002-secretref-resolution.md`): a
    # user-created connector's credential name is derived from its own
    # generated id (`lakehouse_store::connectors::derive_secret_ref`,
    # Rust-side) -- `conn-orders-k3x9` derives
    # `env:CONNECTOR_CONN_ORDERS_K3X9_PASSWORD`, the addendum's own
    # documented example. No production change was needed here: this
    # name already matches `CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`'
    # existing `env:CONNECTOR_*_PASSWORD` pattern, the same way it
    # matches the Rust-side allowlist -- this test proves that, rather
    # than asserting it only in a comment.
    monkeypatch.setenv("CONNECTOR_CONN_ORDERS_K3X9_PASSWORD", "derived-value")
    assert resolve_secret_ref("env:CONNECTOR_CONN_ORDERS_K3X9_PASSWORD") == "derived-value"


def test_resolve_secret_ref_still_resolves_a_seeded_shaped_ref(monkeypatch: pytest.MonkeyPatch) -> None:
    # The seeded connectors (`conn-pg-lakehouse`, `conn-s3-warehouse`)
    # keep their pre-addendum, non-derived refs
    # (`0022_prune_connector_seed.sql`/`0023_connector_dedicated_secret_refs.sql`)
    # -- unaffected by Addendum 3, still resolvable the same way.
    monkeypatch.setenv("CONNECTOR_PG_PASSWORD", "seeded-value")
    assert resolve_secret_ref("env:CONNECTOR_PG_PASSWORD") == "seeded-value"


def test_resolve_secret_ref_refuses_a_name_with_no_credential_suffix() -> None:
    # No pattern admits a bare CONNECTOR_* name that does not end in one
    # of the credential suffixes -- the exact class of hole Z6/X1 close:
    # no env var name may be derived from a connector id and read
    # unconditionally, regardless of whether it happens to be set.
    with pytest.raises(SecretRefRejected):
        resolve_secret_ref("env:CONNECTOR_CONN_X_HOST")


def test_resolve_secret_ref_refuses_the_probe_flag_by_name() -> None:
    # Same regression X1 found API-side, now proven Python-side too: this
    # name starts with CONNECTOR_ but does not end in a credential
    # suffix, so no pattern admits it.
    with pytest.raises(SecretRefRejected):
        resolve_secret_ref("env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS")


def test_resolve_secret_ref_refuses_a_missing_ref() -> None:
    with pytest.raises(SecretRefRejected):
        resolve_secret_ref(None)


def test_resolve_secret_ref_refuses_an_unset_but_allowlisted_env_var(monkeypatch: pytest.MonkeyPatch) -> None:
    # `env:CONNECTOR_MSSQL_PASSWORD` DOES end in `_PASSWORD`, so the shape
    # check admits it -- but it is unset in this environment, which is a
    # SEPARATE failure reason from "not allowlisted". Both raise
    # SecretRefRejected; a future ingest_factory.py (WS3 plan review Z6)
    # treats both as status="rejected" uniformly, since either way the
    # run cannot proceed with a real credential.
    monkeypatch.delenv("CONNECTOR_MSSQL_PASSWORD", raising=False)
    with pytest.raises(SecretRefRejected):
        resolve_secret_ref("env:CONNECTOR_MSSQL_PASSWORD")


def test_resolve_secret_ref_refuses_an_unsupported_scheme() -> None:
    with pytest.raises(SecretRefRejected):
        resolve_secret_ref("vault:secret/data/connector_mysql_password")


def test_resolve_secret_ref_refuses_a_file_ref_that_escapes_the_base_dir() -> None:
    with pytest.raises(SecretRefRejected):
        resolve_secret_ref("file:/run/secrets/connector_../../etc/passwd")


def test_resolve_secrets_zips_both_slots_for_a_two_field_mapping(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CONNECTOR_S3_ACCESS_KEY", "AKIA...")
    monkeypatch.setenv("CONNECTOR_S3_SECRET_KEY", "s3cret")
    resolved = resolve_secrets(
        "files",
        None,
        "env:CONNECTOR_S3_ACCESS_KEY",
        "env:CONNECTOR_S3_SECRET_KEY",
    )
    assert resolved == {"accessKey": "AKIA...", "secretKey": "s3cret"}


def test_resolve_secrets_raises_when_the_secondary_slot_is_needed_but_missing() -> None:
    # adapter/auth needs two secret fields; secondary secretRef is None --
    # never resolve with a partial dict, raise instead.
    with pytest.raises(SecretRefRejected):
        resolve_secrets("files", None, "env:CONNECTOR_S3_ACCESS_KEY", None)


def test_resolve_secrets_propagates_the_unknown_combination_error() -> None:
    with pytest.raises(ValueError):
        resolve_secrets("rest", "not-a-real-auth-type", "env:CONNECTOR_X_TOKEN", None)


def test_resolve_secrets_returns_empty_for_kafka_none_auth_with_no_secret_ref_at_all() -> None:
    # ("kafka", "none") maps to an empty fields tuple -- no secretRef is
    # required for a PLAINTEXT broker, so this must not raise
    # SecretRefRejected for a missing primary ref.
    assert resolve_secrets("kafka", "none", None, None) == {}
