"""Tests for `dispar_orchestrate.adapters.oracle`.
No network: `sql_database` and `resolve_checked` are both injected/
monkeypatched, so nothing here dials a real socket. Each test asserts
one specific behaviour of `build_source`'s design (connect by resolved
IP, TLS verified against an operator-supplied DN, checking_resolver-
wrapped) -- never merely "it did not crash"."""
from __future__ import annotations

import pytest

from dispar_orchestrate.adapters.oracle import OracleTlsConfigError, build_source
from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked


def test_build_source_checks_the_host_before_building_any_credentials(monkeypatch):
    calls = []

    def _resolve(host, port):
        calls.append((host, port))
        raise SsrfBlocked("refused")

    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **_: pytest.fail("must not be called"))
    with pytest.raises(SsrfBlocked):
        build_source(
            {"driver": "oracle", "host": "169.254.169.254", "port": 1521, "database": "ORCLPDB1", "user": "reader"},
            secrets={"password": "x"},
            source_objects=[],
            resolve_checked=_resolve,
        )
    assert calls == [("169.254.169.254", 1521)]


def test_build_source_connects_by_the_resolved_ip_literal_not_the_hostname(monkeypatch):
    # The connection's own `host` field is the CHECKED IP -- never
    # the original hostname -- so nothing downstream can re-resolve it.
    captured = {}
    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **kwargs: captured.update(kwargs) or "src")
    build_source(
        {"driver": "oracle", "host": "ora.invalid", "port": 1521, "database": "ORCLPDB1", "user": "reader"},
        secrets={"password": "s3cret"},
        source_objects=[{"name": "orders", "target": "orders"}],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
    )
    assert captured["credentials"]["drivername"] == "oracle+oracledb"
    assert captured["credentials"]["host"] == "10.0.0.9"  # the resolved IP, not "ora.invalid"
    assert captured["table_names"] == ["orders"]
    assert "connect_args" not in captured.get("engine_kwargs", {})  # sslMode unset -- plaintext, no DN check


def test_build_source_passes_through_the_operator_supplied_ssl_server_cert_dn_unchanged(monkeypatch):
    # The DN is OPERATOR-SUPPLIED, never synthesized
    # from spec["host"] -- python-oracledb matches the FULL certificate
    # DN, so a real multi-component subject (OU=/O=/C=, not just CN=)
    # must pass through exactly as the operator wrote it.
    captured = {}
    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **kwargs: captured.update(kwargs) or "src")
    build_source(
        {
            "driver": "oracle",
            "host": "ora.invalid",
            "port": 1521,
            "database": "ORCLPDB1",
            "user": "reader",
            "sslMode": "required",
            "sslServerCertDn": "CN=ora.invalid,OU=DB,O=Example Corp,C=US",
        },
        secrets={"password": "s3cret"},
        source_objects=[],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
    )
    assert captured["engine_kwargs"]["connect_args"]["ssl_server_cert_dn"] == "CN=ora.invalid,OU=DB,O=Example Corp,C=US"


def test_build_source_refuses_tls_with_no_ssl_server_cert_dn_stating_why(monkeypatch):
    # Refuse, never silently connect with TLS
    # negotiated but the server's identity unverified.
    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **_: pytest.fail("must not be called"))
    with pytest.raises(OracleTlsConfigError, match="cannot be verified when connecting by IP"):
        build_source(
            {
                "driver": "oracle",
                "host": "ora.invalid",
                "port": 1521,
                "database": "ORCLPDB1",
                "user": "reader",
                "sslMode": "required",  # no sslServerCertDn
            },
            secrets={"password": "s3cret"},
            source_objects=[],
            resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
        )


def test_build_source_wraps_the_call_in_checking_resolver(monkeypatch):
    # Belt-and-suspenders regardless of which resolver mechanism
    # thin mode uses internally -- proven by asserting the module's
    # build_source body actually enters ssrf_guard.checking_resolver.
    entered = []

    class _FakeCtx:
        def __enter__(self):
            entered.append(True)

        def __exit__(self, *a):
            return False

    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.ssrf_guard.checking_resolver", lambda **_: _FakeCtx())
    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **kwargs: "src")
    build_source(
        {"driver": "oracle", "host": "ora.invalid", "port": 1521, "database": "ORCLPDB1", "user": "reader"},
        secrets={"password": "x"},
        source_objects=[],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
    )
    assert entered == [True]


def test_build_source_never_interpolates_the_password_into_a_dsn_string(monkeypatch):
    # Structured credentials dict, never
    # an interpolated "oracle+oracledb://user:pass@host/db" DSN string.
    captured = {}
    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **kwargs: captured.update(kwargs) or "src")
    build_source(
        {"driver": "oracle", "host": "ora.invalid", "port": 1521, "database": "ORCLPDB1", "user": "reader"},
        secrets={"password": "p@ss/word:with?special&chars"},
        source_objects=[],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
    )
    assert captured["credentials"]["password"] == "p@ss/word:with?special&chars"
