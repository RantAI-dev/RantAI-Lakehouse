"""Unit tests for `dagster/dispar_orchestrate/adapters/sql.py` -- the
`sql`-adapter source builder for `mysql`/`mariadb`/`mssql` connectors
(`docs/superpowers/plans/2026-09-11-ws3-ingestion-tier1.md`'s
adapters/sql.py section, WS3 plan review Z13).

No network: `resolve_checked` is always injected (the real
`ssrf_guard.resolve_checked` calls `socket.getaddrinfo`), and
`sql_database` is monkeypatched so no driver import or connection attempt
ever happens. `mysql+pymysql`/`mssql+pyodbc` drivers are not installed in
this venv (a separate dependency task's job -- verified separately:
`import pymysql`/`import pyodbc` both raise `ModuleNotFoundError` here);
these tests never import either, since `sql_database` itself is faked
before `build_source` is called.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_adapters_sql.py -q
"""

from __future__ import annotations

import urllib.parse

import pytest

from dispar_orchestrate.adapters.sql import _mssql_connection_string, build_source
from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked


def test_build_source_wraps_sql_database_with_the_given_table_names(monkeypatch) -> None:
    captured = {}

    def fake_sql_database(**kwargs):
        captured.update(kwargs)
        return "the-dlt-source"

    monkeypatch.setattr("dispar_orchestrate.adapters.sql.sql_database", fake_sql_database)
    spec = {"driver": "mysql", "host": "db.internal", "port": 3306, "database": "shop", "user": "reader"}
    source_objects = [{"name": "shop.orders", "incrementalKey": "updated_at", "target": "orders"}]
    result = build_source(
        spec,
        secrets={"password": "s3cret"},
        source_objects=source_objects,
        resolve_checked=lambda host, port: ResolvedAddress(ip="10.20.30.40", port=port, family=2),
    )
    assert captured["credentials"]["drivername"] == "mysql+pymysql"
    assert captured["credentials"]["password"] == "s3cret"
    assert captured["table_names"] == ["orders"]
    assert result.source == "the-dlt-source"
    assert result.resolved.ip == "10.20.30.40"


def test_build_source_checks_the_host_before_building_any_credentials(monkeypatch) -> None:
    calls = []

    def fake_resolve_checked(host, port):
        calls.append((host, port))
        raise SsrfBlocked("refused")

    monkeypatch.setattr("dispar_orchestrate.adapters.sql.sql_database", lambda **_: pytest.fail("must not be called"))
    with pytest.raises(SsrfBlocked):
        build_source(
            {"driver": "mysql", "host": "169.254.169.254", "port": 3306, "database": "d", "user": "u"},
            secrets={"password": "x"},
            source_objects=[],
            resolve_checked=fake_resolve_checked,
        )
    assert calls == [("169.254.169.254", 3306)]


# WS3 plan review Z7: Microsoft ODBC Driver 18 connects
# via a raw ODBC connection string carrying `Server=tcp:<ip>,<port>`
# (bypassing SQLAlchemy's own host/port fields, which would otherwise
# re-resolve `host` themselves) plus `HostNameInCertificate=<host>` so
# TLS certificate verification still targets the real hostname.
def test_build_source_pins_mssql_via_a_raw_odbc_connection_string(monkeypatch) -> None:
    captured = {}
    monkeypatch.setattr(
        "dispar_orchestrate.adapters.sql.sql_database", lambda **kwargs: captured.update(kwargs) or "src"
    )
    build_source(
        {"driver": "mssql", "host": "mssql.internal", "port": 1433, "database": "d", "user": "u"},
        secrets={"password": "x"},
        source_objects=[],
        resolve_checked=lambda host, port: ResolvedAddress(ip="10.0.0.9", port=port, family=2),
    )
    # `credentials` is a `mssql+pyodbc:///?odbc_connect=<quoted>` SQLAlchemy
    # URL (the standard way SQLAlchemy's pyodbc dialect accepts a raw ODBC
    # connection string) -- unquote it back to the ODBC string before
    # asserting on its `KEY=value;` attributes.
    conn_str = urllib.parse.unquote_plus(captured["credentials"])
    assert "SERVER=tcp:10.0.0.9,1433" in conn_str
    assert "HostNameInCertificate=mssql.internal" in conn_str


def test_build_source_needs_no_extra_pinning_for_pymysql(monkeypatch) -> None:
    # mysql/mariadb (pymysql) is pure Python and resolves via
    # socket.getaddrinfo -- a future ingest_factory.py's
    # ssrf_guard.pinned_resolution wrap gets FULL pinning with no
    # compensating code here, unlike mssql above. Asserted by absence: no
    # engine_kwargs at all.
    captured = {}
    monkeypatch.setattr(
        "dispar_orchestrate.adapters.sql.sql_database", lambda **kwargs: captured.update(kwargs) or "src"
    )
    build_source(
        {"driver": "mysql", "host": "db.internal", "port": 3306, "database": "d", "user": "u"},
        secrets={"password": "x"},
        source_objects=[],
        resolve_checked=lambda host, port: ResolvedAddress(ip="10.0.0.9", port=port, family=2),
    )
    assert captured.get("engine_kwargs") is None


# WS3 plan review Z13 (blocker, security): _mssql_connection_string
# f-strings spec['database']/spec['user']/secrets['password'] unquoted
# into a `KEY=value;`-delimited ODBC string. A database value of
# "x;TrustServerCertificate=yes" would inject an attribute AHEAD of the
# fixed "Encrypt=yes;TrustServerCertificate=no;" this function writes,
# switching off certificate verification for the credential the job
# sends -- and a legitimate password containing ';' or '}' would break
# the string even with no malicious intent. Every interpolated free-form
# value is brace-quoted per the ODBC convention: `{` + value with every
# `}` doubled + `}` -- a literal `;` or `=` inside a braced value is
# just a character, never a delimiter.
def test_mssql_connection_string_keeps_an_injection_shaped_database_inside_one_braced_value() -> None:
    # `_mssql_connection_string` returns a `mssql+pyodbc:///?odbc_connect=`
    # SQLAlchemy URL (`urllib.parse.quote_plus`-encoded, the standard way
    # SQLAlchemy's pyodbc dialect accepts a raw ODBC connection string) --
    # unquote it back to inspect the ODBC attributes it actually carries.
    conn_str = urllib.parse.unquote_plus(
        _mssql_connection_string(
            {"host": "mssql.internal", "port": 1433, "database": "x;TrustServerCertificate=yes", "user": "u"},
            {"password": "p"},
            ResolvedAddress(ip="10.0.0.9", port=1433, family=2),
        )
    )
    assert "DATABASE={x;TrustServerCertificate=yes}" in conn_str
    # The FIXED attribute, written before any interpolated value, is
    # never shadowed by the injected text -- it is not even adjacent to
    # an unquoted ';' any more, since the whole injected string sits
    # inside DATABASE={...}.
    assert conn_str.index("TrustServerCertificate=no") < conn_str.index("DATABASE=")


def test_mssql_connection_string_round_trips_a_password_containing_semicolon_and_brace() -> None:
    conn_str = urllib.parse.unquote_plus(
        _mssql_connection_string(
            {"host": "mssql.internal", "port": 1433, "database": "d", "user": "u"},
            {"password": "p;w}ord"},
            ResolvedAddress(ip="10.0.0.9", port=1433, family=2),
        )
    )
    # '}' doubled per ODBC brace-quoting; ';' passes through unescaped
    # inside the braced value, since only '}' is special once quoted.
    assert "PWD={p;w}}ord}" in conn_str


def test_mssql_connection_string_refuses_a_host_containing_a_semicolon() -> None:
    with pytest.raises(ValueError, match="not a valid hostname"):
        _mssql_connection_string(
            {"host": "mssql.internal;TrustServerCertificate=yes", "port": 1433, "database": "d", "user": "u"},
            {"password": "p"},
            ResolvedAddress(ip="10.0.0.9", port=1433, family=2),
        )


def test_mssql_connection_string_refuses_a_control_character_in_database() -> None:
    with pytest.raises(ValueError, match="control character"):
        _mssql_connection_string(
            {"host": "mssql.internal", "port": 1433, "database": "d\r\nX", "user": "u"},
            {"password": "p"},
            ResolvedAddress(ip="10.0.0.9", port=1433, family=2),
        )


def test_build_source_refuses_an_unknown_driver(monkeypatch) -> None:
    monkeypatch.setattr("dispar_orchestrate.adapters.sql.sql_database", lambda **_: pytest.fail("must not be called"))
    with pytest.raises(ValueError, match="does not know driver"):
        build_source(
            {"driver": "oracle", "host": "db.internal", "port": 1521, "database": "d", "user": "u"},
            secrets={"password": "x"},
            source_objects=[],
            resolve_checked=lambda host, port: ResolvedAddress(ip="10.0.0.9", port=port, family=2),
        )
