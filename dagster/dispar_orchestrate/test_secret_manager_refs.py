"""`SRC-12` part B: the reference grammar, the allowlist and the fixed error
texts, checked against the list the API's tests read
(`rust/crates/lakehouse-api/tests/fixtures/secret_manager_refs.json`). No
network.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from dispar_orchestrate import secret_manager
from dispar_orchestrate.secret_resolver import (
    CONNECTOR_ALLOWED_SECRET_REF_PATTERNS,
    SecretRefRejected,
    _pattern_matches,
    resolve_secret_ref,
)

_FIXTURE = json.loads(
    (
        Path(__file__).resolve().parents[2] / "rust/crates/lakehouse-api/tests/fixtures/secret_manager_refs.json"
    ).read_text()
)


@pytest.mark.parametrize("case", _FIXTURE["valid"], ids=lambda c: c["ref"])
def test_every_valid_reference_of_the_shared_list_parses_to_the_listed_parts(case: dict) -> None:
    assert secret_manager.parse_ref(case["manager"], case["ref"]) == case["parts"]
    assert secret_manager.kind_of_ref(case["ref"]) == case["manager"]


@pytest.mark.parametrize("case", _FIXTURE["refused"], ids=lambda c: c["ref"])
def test_every_refused_reference_of_the_shared_list_is_refused(case: dict) -> None:
    with pytest.raises(secret_manager.RefError):
        secret_manager.parse_ref(case["manager"], case["ref"])


def test_a_reference_of_another_managers_scheme_or_too_long_is_refused() -> None:
    with pytest.raises(secret_manager.RefError):
        secret_manager.parse_ref("aws", "vault:secret/a#k")
    with pytest.raises(secret_manager.RefError):
        secret_manager.parse_ref("vault", "vault:secret/" + "a" * 2000 + "#k")


def test_the_fixed_texts_are_the_shared_ones() -> None:
    assert secret_manager.TEXTS == _FIXTURE["texts"]


def _admits(kind: str | None, secret_ref: str) -> bool:
    patterns = CONNECTOR_ALLOWED_SECRET_REF_PATTERNS + secret_manager.allowed_patterns(kind)
    return any(_pattern_matches(p, secret_ref) for p in patterns)


def test_the_allowlist_admits_only_the_configured_managers_scheme() -> None:
    for secret_ref in _FIXTURE["base_allowed"]:
        for kind in (None, "vault"):
            assert _admits(kind, secret_ref)
    for secret_ref in _FIXTURE["base_refused"]:
        for kind in (None, *secret_manager.KINDS):
            assert not _admits(kind, secret_ref)
    assert len(_FIXTURE["allowlist"]) >= 14
    for case in _FIXTURE["allowlist"]:
        assert _admits(case["setting"], case["ref"]) == case["allowed"], case


def test_configured_kind_reads_the_setting_and_fails_closed_on_an_unknown_one() -> None:
    assert secret_manager.configured_kind({}) is None
    assert secret_manager.configured_kind({"SECRET_MANAGER": " "}) is None
    assert secret_manager.configured_kind({"SECRET_MANAGER": "vault"}) == "vault"
    assert secret_manager.configured_kind({"SECRET_MANAGER": "Vault"}) is None


def test_a_manager_reference_without_the_manager_set_is_refused_by_the_allowlist(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("SECRET_MANAGER", raising=False)
    with pytest.raises(SecretRefRejected, match="not an allowlisted"):
        resolve_secret_ref("vault:secret/a#k")


def test_another_managers_reference_is_refused_even_when_a_manager_is_set(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SECRET_MANAGER", "vault")
    with pytest.raises(SecretRefRejected, match="not an allowlisted"):
        resolve_secret_ref("aws-sm:prod/orders")


def test_a_manager_without_a_reader_in_this_build_is_refused_not_answered(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(secret_manager, "_READERS", {})
    monkeypatch.setattr(secret_manager, "_MODULES", {})
    monkeypatch.setenv("SECRET_MANAGER", "vault")
    with pytest.raises(SecretRefRejected, match="not a reference this build can read"):
        resolve_secret_ref("vault:secret/a#k")


def test_a_failure_reaches_the_caller_as_its_fixed_text_and_nothing_else(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    for reason, text in secret_manager.TEXTS.items():
        def reader(parsed: object, environ: object, reason: str = reason) -> str:
            raise secret_manager.ManagerFailure(reason)

        monkeypatch.setattr(secret_manager, "_READERS", {"vault": reader})
        monkeypatch.setenv("SECRET_MANAGER", "vault")
        with pytest.raises(SecretRefRejected) as excinfo:
            resolve_secret_ref("vault:secret/a#k")
        assert str(excinfo.value).endswith(text)


def test_a_manager_value_is_trimmed_and_read_at_every_call(monkeypatch: pytest.MonkeyPatch) -> None:
    answers = iter([" first \n", "second"])
    monkeypatch.setattr(secret_manager, "_READERS", {"vault": lambda parsed, environ: next(answers)})
    monkeypatch.setenv("SECRET_MANAGER", "vault")
    assert resolve_secret_ref("vault:secret/a#k") == "first"
    assert resolve_secret_ref("vault:secret/a#k") == "second"
