"""Tests for `secret_resolver` (WS3 plan review Z6) -- the
Python port of the allowlisted credential-suffix resolver
`lakehouse_core::secret::AllowlistedSecretResolver`/`pattern_matches`
enforces API-side. See `secret_resolver.py`'s module docstring for the
full trust model and the bug this replaces.
"""

from __future__ import annotations

import base64
import json
import os
from pathlib import Path

import pytest
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from dispar_orchestrate import secret_resolver
from dispar_orchestrate.secret_resolver import (
    SecretRefRejected,
    open_managed_credential,
    resolve_secret_ref,
    resolve_secrets,
)


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


# ── SRC-12 decision 4: managed credentials are sealed at rest ────────────────
#
# The vector is the one `lakehouse-api`'s `secret_crypto.rs` tests also open:
# written once with this package's AESGCM, so passing here AND there is what
# proves the two implementations read the same format.

_VECTOR_PATH = (
    Path(__file__).resolve().parents[2]
    / "rust/crates/lakehouse-api/tests/fixtures/connector_secret_vector.json"
)


def _vector() -> dict[str, str]:
    return json.loads(_VECTOR_PATH.read_text())


def _other_key() -> str:
    return base64.b64encode(bytes([7]) * 32).decode()


def test_the_shared_vector_written_by_the_api_format_opens_here(monkeypatch: pytest.MonkeyPatch) -> None:
    vector = _vector()
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY", vector["key_base64"])
    monkeypatch.delenv("CONNECTOR_SECRETS_KEY_PREVIOUS", raising=False)
    sealed = base64.b64decode(vector["sealed_base64"])
    assert open_managed_credential(vector["file_name"], sealed) == vector["plaintext"]


def test_a_sealed_file_copied_under_another_connectors_name_does_not_open(monkeypatch: pytest.MonkeyPatch) -> None:
    vector = _vector()
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY", vector["key_base64"])
    sealed = base64.b64decode(vector["sealed_base64"])
    with pytest.raises(SecretRefRejected, match="cannot be decrypted with this installation's key"):
        open_managed_credential(vector["other_file_name"], sealed)


def test_the_previous_key_opens_a_file_sealed_before_a_key_change(monkeypatch: pytest.MonkeyPatch) -> None:
    vector = _vector()
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY", _other_key())
    sealed = base64.b64decode(vector["sealed_base64"])
    with pytest.raises(SecretRefRejected, match="cannot be decrypted"):
        open_managed_credential(vector["file_name"], sealed)
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY_PREVIOUS", vector["key_base64"])
    assert open_managed_credential(vector["file_name"], sealed) == vector["plaintext"]


@pytest.mark.parametrize("bad_key", ["not base64 !", "c2hvcnQ=", base64.b64encode(bytes(33)).decode()])
def test_a_malformed_key_is_refused_with_a_fixed_text_that_never_echoes_it(
    monkeypatch: pytest.MonkeyPatch, bad_key: str
) -> None:
    vector = _vector()
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY", bad_key)
    with pytest.raises(SecretRefRejected) as excinfo:
        open_managed_credential(vector["file_name"], base64.b64decode(vector["sealed_base64"]))
    assert "key is missing or invalid" in str(excinfo.value)
    assert bad_key not in str(excinfo.value)


def test_a_missing_key_is_refused_with_the_fixed_text_and_the_setting_goes_to_the_log(
    monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    vector = _vector()
    monkeypatch.delenv("CONNECTOR_SECRETS_KEY", raising=False)
    with pytest.raises(SecretRefRejected, match="key is missing or invalid"):
        open_managed_credential(vector["file_name"], base64.b64decode(vector["sealed_base64"]))
    assert "CONNECTOR_SECRETS_KEY is not set" in caplog.text


def test_bytes_that_are_not_a_seal_are_refused_not_read_as_text(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY", _vector()["key_base64"])
    for not_sealed in (b"", b"plain-password", bytes([1]) * 5, bytes([9]) * 40):
        with pytest.raises(SecretRefRejected, match="not in a format this build reads"):
            open_managed_credential("connector_managed_conn_x_password", not_sealed)


def _point_secrets_dir_at(monkeypatch: pytest.MonkeyPatch, directory: Path) -> None:
    # `/run/secrets` does not exist outside the container, so the test moves
    # both the base directory and the one allowlist pattern that names it.
    # The production constant is untouched; this only gives the same rule a
    # directory the test can write.
    monkeypatch.setattr(secret_resolver, "_SECRETS_BASE_DIR", str(directory))
    monkeypatch.setattr(
        secret_resolver, "CONNECTOR_ALLOWED_SECRET_REF_PATTERNS", (f"file:{directory}/connector_*",)
    )


def test_resolve_secret_ref_opens_a_managed_file_and_reads_an_operator_file_as_before(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    vector = _vector()
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY", vector["key_base64"])
    _point_secrets_dir_at(monkeypatch, tmp_path)
    (tmp_path / vector["file_name"]).write_bytes(base64.b64decode(vector["sealed_base64"]))
    (tmp_path / "connector_conn_ops_password").write_text("operator-pass\n")

    assert resolve_secret_ref(f"file:{tmp_path}/{vector['file_name']}") == vector["plaintext"]
    assert resolve_secret_ref(f"file:{tmp_path}/connector_conn_ops_password") == "operator-pass"


def test_resolve_secret_ref_refuses_a_plain_file_under_a_managed_name(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY", _vector()["key_base64"])
    _point_secrets_dir_at(monkeypatch, tmp_path)
    (tmp_path / "connector_managed_conn_old_password").write_text("plain-password")
    with pytest.raises(SecretRefRejected) as excinfo:
        resolve_secret_ref(f"file:{tmp_path}/connector_managed_conn_old_password")
    assert "plain-password" not in str(excinfo.value)


def test_resolve_secret_ref_refuses_a_managed_file_under_the_wrong_key(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    vector = _vector()
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY", _other_key())
    monkeypatch.delenv("CONNECTOR_SECRETS_KEY_PREVIOUS", raising=False)
    _point_secrets_dir_at(monkeypatch, tmp_path)
    (tmp_path / vector["file_name"]).write_bytes(base64.b64decode(vector["sealed_base64"]))
    with pytest.raises(SecretRefRejected, match="cannot be decrypted with this installation's key") as excinfo:
        resolve_secret_ref(f"file:{tmp_path}/{vector['file_name']}")
    assert vector["plaintext"] not in str(excinfo.value)
    assert os.environ["CONNECTOR_SECRETS_KEY"] not in str(excinfo.value)


def test_a_value_sealed_here_round_trips_through_the_same_open_function(monkeypatch: pytest.MonkeyPatch) -> None:
    # Python never seals in production; this only checks that open does not
    # depend on anything but the documented layout.
    key = bytes([11]) * 32
    monkeypatch.setenv("CONNECTOR_SECRETS_KEY", base64.b64encode(key).decode())
    nonce = bytes(range(12))
    name = "connector_managed_conn_r_password"
    sealed = bytes([1]) + nonce + AESGCM(key).encrypt(nonce, b"round-trip", name.encode())
    assert open_managed_credential(name, sealed) == "round-trip"
