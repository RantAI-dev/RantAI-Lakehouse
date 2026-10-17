"""Unit tests for `ingest_errors.py` -- the one classification of an ingest
failure and the retry delays (`SRC-11` D1 and D2, `SRC-19`).

No network and no real driver connection: driver errors are built the way the
driver builds them (or, for `pyodbc`, as a class that claims the module).

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_ingest_errors.py -q
"""

from __future__ import annotations

import logging
import socket

import pytest
import requests

from dispar_orchestrate import ingest_errors as e
from dispar_orchestrate import schema_observer, ssrf_guard
from dispar_orchestrate.adapters import dedup
from dispar_orchestrate.adapters.sink import UnsupportedLoadMode
from dispar_orchestrate.column_gate import UnsupportedColumnType
from dispar_orchestrate.secret_resolver import SecretRefRejected


def _driver_error(module: str, name: str = "OperationalError", **attrs):
    """An exception of a class that claims `module` as its home, with `attrs`
    as attributes, without importing that driver."""
    return type(name, (Exception,), {"__module__": module, **attrs})


def _pg(sqlstate: str | None, message: str = "x"):
    """A `psycopg2` operational error carrying `sqlstate` as `pgcode`."""
    return _driver_error("psycopg2", pgcode=sqlstate)(message)


def _http_error(status: int) -> requests.HTTPError:
    response = requests.Response()
    response.status_code = status
    return requests.HTTPError(response=response)


def _wrapped_like_sqlalchemy(driver_error: Exception) -> Exception:
    import sqlalchemy.exc

    return sqlalchemy.exc.OperationalError("SELECT 1", {}, driver_error)


# One exception per class (and several per class for the transient ones, whose
# drivers each say it differently). The expected class is the second element.
_CASES = [
    # unreachable
    (ConnectionRefusedError(), e.UNREACHABLE),
    (socket.gaierror(-2, "Name or service not known"), e.UNREACHABLE),
    (OSError(113, "No route to host"), e.UNREACHABLE),
    (requests.ConnectionError(), e.UNREACHABLE),
    (_pg(None, "connection to server failed"), e.UNREACHABLE),
    (_driver_error("pymysql")(2003, "Can't connect"), e.UNREACHABLE),
    (_driver_error("pyodbc")("08001", "Unable to connect"), e.UNREACHABLE),
    (_driver_error("kafka", "NoBrokersAvailable")(), e.UNREACHABLE),
    (ConnectionResetError(), e.UNREACHABLE),
    # timeout
    (TimeoutError(), e.TIMEOUT),
    (requests.Timeout(), e.TIMEOUT),
    (requests.ConnectTimeout(), e.TIMEOUT),
    (_driver_error("pyodbc")("HYT00", "Login timeout expired"), e.TIMEOUT),
    (_pg("57014"), e.TIMEOUT),
    (_http_error(504), e.TIMEOUT),
    # source_busy
    (_driver_error("pymysql")(1205, "Lock wait timeout"), e.SOURCE_BUSY),
    (_pg("40P01"), e.SOURCE_BUSY),
    (_pg("53300"), e.SOURCE_BUSY),
    (_http_error(503), e.SOURCE_BUSY),
    (_http_error(429), e.SOURCE_BUSY),
    # api_unreachable
    (schema_observer.ObservationUnreachable("down"), e.API_UNREACHABLE),
    (e.ApiUnreachable("down"), e.API_UNREACHABLE),
    # sign_in_refused
    (_driver_error("pymysql")(1045, "Access denied"), e.SIGN_IN_REFUSED),
    (_driver_error("pyodbc")("28000", "Login failed"), e.SIGN_IN_REFUSED),
    (_pg("28P01"), e.SIGN_IN_REFUSED),
    (_pg("28000"), e.SIGN_IN_REFUSED),
    (_pg(None, 'FATAL:  password authentication failed for user "u"'), e.SIGN_IN_REFUSED),
    (_http_error(401), e.SIGN_IN_REFUSED),
    (_http_error(403), e.SIGN_IN_REFUSED),
    (_driver_error("paramiko", "AuthenticationException")(), e.SIGN_IN_REFUSED),
    (_driver_error("kafka", "SaslAuthenticationFailedError")(), e.SIGN_IN_REFUSED),
    # address_refused
    (ssrf_guard.SsrfBlocked("10.0.0.1 is blocked"), e.ADDRESS_REFUSED),
    # unsupported
    (UnsupportedColumnType("c", "struct<a:int>"), e.UNSUPPORTED),
    (UnsupportedLoadMode("mode"), e.UNSUPPORTED),
    (dedup.DedupRefused("no key"), e.UNSUPPORTED),
    (e.UnsupportedKafkaAuth("oauth"), e.UNSUPPORTED),
    # setting_rejected
    (SecretRefRejected("not allowlisted"), e.SETTING_REJECTED),
    (e.UnknownAdapter("adapter=x"), e.SETTING_REJECTED),
    (schema_observer.ObservationRefused("HTTP 403"), e.SETTING_REJECTED),
    (e.ApiRefused("no"), e.SETTING_REJECTED),
    (ValueError("unsupported file format"), e.SETTING_REJECTED),
    # other
    (RuntimeError("nobody taught this module about this"), e.OTHER),
    (KeyError("x"), e.OTHER),
    (_http_error(404), e.OTHER),
    (FileNotFoundError(2, "No such file"), e.OTHER),
    (_driver_error("pymysql")(1064, "syntax error"), e.OTHER),
]


@pytest.mark.parametrize(("exc", "expected"), _CASES, ids=[f"{type(x).__name__}-{c}" for x, c in _CASES])
def test_a_failure_is_named_with_its_class_and_retried_only_when_the_class_is_transient(exc, expected) -> None:
    verdict = e.classify(exc)
    assert verdict.error_class == expected
    assert verdict.retried is (expected in {e.UNREACHABLE, e.TIMEOUT, e.SOURCE_BUSY, e.API_UNREACHABLE})


def test_every_class_a_failure_can_get_is_in_the_closed_list_the_api_accepts() -> None:
    assert e.ERROR_CLASSES == {
        "unreachable", "timeout", "source_busy", "api_unreachable", "sign_in_refused", "address_refused",
        "unsupported", "setting_rejected", "schema_decision_pending", "other",
    }
    assert {c for _, c in _CASES} <= e.ERROR_CLASSES
    assert set(e.CLASS_PHRASES) == e.ERROR_CLASSES
    assert e.RETRIED_CLASSES == {"unreachable", "timeout", "source_busy", "api_unreachable"}


def test_a_table_waiting_for_a_schema_decision_is_a_class_that_is_not_retried() -> None:
    assert e.SCHEMA_DECISION_PENDING in e.ERROR_CLASSES
    assert e.SCHEMA_DECISION_PENDING not in e.RETRIED_CLASSES


def test_a_real_postgres_refused_sign_in_is_recognised_without_a_sqlstate() -> None:
    import psycopg2

    # What libpq raises while connecting: the message only, `pgcode` is None.
    refused = psycopg2.OperationalError(
        'connection to server at "db", port 5432 failed: FATAL:  password authentication failed for user "u"\n'
    )
    assert refused.pgcode is None
    assert e.classify(refused) == e.Classified(e.SIGN_IN_REFUSED, retried=False)
    assert e.classify(_wrapped_like_sqlalchemy(refused)).error_class == e.SIGN_IN_REFUSED


def test_a_postgres_error_with_a_sqlstate_is_decided_by_the_code_not_by_its_words() -> None:
    other_code = _pg("08006", "password authentication failed")  # a connection failure that mentions it
    assert e.classify(other_code).error_class == e.UNREACHABLE


def test_the_refusal_wording_is_read_from_a_postgres_error_only() -> None:
    assert e.classify(RuntimeError("password authentication failed")).error_class == e.OTHER
    assert e.classify(_driver_error("pymysql")(2003, "password authentication failed")).error_class == e.UNREACHABLE


def test_a_refusal_wrapped_by_the_orm_or_raise_from_wins_over_the_wrapper() -> None:
    denied = _driver_error("pymysql")(1045, "Access denied")
    assert e.classify(_wrapped_like_sqlalchemy(denied)).error_class == e.SIGN_IN_REFUSED
    try:
        try:
            raise denied
        except Exception as inner:
            raise ConnectionError("pipeline step failed") from inner
    except ConnectionError as outer:
        assert e.classify(outer) == e.Classified(e.SIGN_IN_REFUSED, retried=False)


def test_a_driver_error_wrapped_by_the_orm_keeps_its_own_class() -> None:
    assert e.classify(_wrapped_like_sqlalchemy(_driver_error("pymysql")(2003, "x"))).error_class == e.UNREACHABLE
    assert e.classify(_wrapped_like_sqlalchemy(_driver_error("pymysql")(1205, "x"))).error_class == e.SOURCE_BUSY


def test_a_cycle_in_the_cause_chain_ends_the_walk() -> None:
    cyclic = RuntimeError("a")
    cyclic.__cause__ = cyclic
    assert e.classify(cyclic).error_class == e.OTHER


def test_nothing_of_a_message_is_in_the_verdict() -> None:
    verdict = e.classify(_driver_error("pymysql")(1045, "Access denied for user 'secret_user'@'10.1.2.3'"))
    assert "secret_user" not in repr(verdict) and "10.1.2.3" not in repr(verdict)
    assert all("secret_user" not in phrase for phrase in e.CLASS_PHRASES.values())


# --- the delays (D1) -----------------------------------------------------------


@pytest.fixture
def delays_env(monkeypatch):
    def set_to(value: str | None) -> None:
        if value is None:
            monkeypatch.delenv(e.RETRY_DELAYS_ENV, raising=False)
        else:
            monkeypatch.setenv(e.RETRY_DELAYS_ENV, value)

    return set_to


def test_the_default_delays_are_one_five_and_fifteen_minutes(delays_env) -> None:
    delays_env(None)
    settings = e.RetrySettings.from_env()
    assert settings.delays == (60, 300, 900)
    assert settings.max_retries == 3
    assert [settings.delay_for(n) for n in range(4)] == [60, 300, 900, None]


def test_the_delays_come_from_the_setting(delays_env) -> None:
    delays_env("1, 2,3")
    assert e.RetrySettings.from_env().delays == (1, 2, 3)
    delays_env("0")
    settings = e.RetrySettings.from_env()
    assert settings.delays == (0,) and settings.max_retries == 1 and settings.delay_for(1) is None


@pytest.mark.parametrize("bad", ["abc", "1,,2", "1,x", "-5", "60,300,900,1800", "99999999", "1.5", "  "])
def test_a_setting_that_cannot_be_read_falls_back_to_the_default_and_says_so(delays_env, caplog, bad) -> None:
    delays_env(bad)
    with caplog.at_level(logging.WARNING, logger=e.logger.name):
        settings = e.RetrySettings.from_env()
    assert settings.delays == (60, 300, 900)
    if bad.strip():  # a blank value is the same as unset: nothing to complain about
        assert any(e.RETRY_DELAYS_ENV in r.getMessage() and "default" in r.getMessage() for r in caplog.records)


def test_a_bad_setting_is_not_echoed_into_the_log(delays_env, caplog) -> None:
    delays_env("hunter2")
    with caplog.at_level(logging.WARNING, logger=e.logger.name):
        e.RetrySettings.from_env()
    assert "hunter2" not in caplog.text
