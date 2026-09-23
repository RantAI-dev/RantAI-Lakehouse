"""Tests for `secret_map.secret_field_names` (WS3 plan review Z6, Task
F1b) -- the ONE mapping deciding which named secret fields an
`(adapter, auth type)` pair needs. See `secret_map.py`'s module docstring
for why this exists: the bug it replaces derived an env-var name from a
principal-chosen connector id and defaulted silently to `""`.
"""

from __future__ import annotations

import pytest

from dispar_orchestrate.secret_map import secret_field_names


def test_sql_and_cdc_need_only_a_password() -> None:
    assert secret_field_names("sql", None) == ("password",)
    assert secret_field_names("cdc", None) == ("password",)


def test_files_needs_an_access_key_pair() -> None:
    assert secret_field_names("files", None) == ("accessKey", "secretKey")


def test_rest_needs_fields_matching_its_auth_type() -> None:
    assert secret_field_names("rest", "api_key") == ("apiKey",)
    assert secret_field_names("rest", "bearer") == ("token",)
    assert secret_field_names("rest", "basic") == ("username", "password")
    assert secret_field_names("rest", "oauth2_client_credentials") == (
        "clientId",
        "clientSecret",
    )


def test_sheets_needs_a_service_account_json() -> None:
    assert secret_field_names("sheets", None) == ("serviceAccountJson",)


def test_unknown_combination_is_a_hard_error() -> None:
    with pytest.raises(ValueError):
        secret_field_names("rest", "not-a-real-auth-type")


def test_a_non_rest_adapter_ignores_the_auth_type_argument() -> None:
    # `secret_field_names` only consults `auth_type` for adapter == "rest"
    # -- a caller passing a stray auth_type for "sql" still gets the
    # single-password mapping rather than a spurious lookup miss.
    assert secret_field_names("sql", "bearer") == ("password",)


def test_mongo_needs_only_a_password() -> None:
    assert secret_field_names("mongodb", None) == ("password",)


def test_oracle_needs_only_a_password_like_every_other_sql_driver() -> None:
    # Oracle is `adapter="sql"`, `dial.driver="oracle"` -- the mapping key
    # is unchanged, it is still ("sql", None).
    assert secret_field_names("sql", None) == ("password",)


def test_kafka_none_auth_needs_no_secret_at_all() -> None:
    assert secret_field_names("kafka", "none") == ()


def test_kafka_sasl_plain_needs_username_and_password() -> None:
    assert secret_field_names("kafka", "sasl_plain") == ("username", "password")


def test_sftp_password_auth_needs_a_password() -> None:
    assert secret_field_names("sftp", "password") == ("password",)


def test_sftp_public_key_auth_needs_a_private_key() -> None:
    assert secret_field_names("sftp", "public_key") == ("privateKey",)


def test_kafka_with_an_unrecognized_auth_type_is_a_hard_error() -> None:
    with pytest.raises(ValueError):
        secret_field_names("kafka", "sasl_scram_sha256")
