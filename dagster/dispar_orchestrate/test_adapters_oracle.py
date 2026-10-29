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


def test_the_table_adapter_callback_reaches_sql_database_and_is_left_out_when_absent(monkeypatch):
    captured = []
    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **kwargs: captured.append(kwargs) or "src")
    spec = {"driver": "oracle", "host": "ora.invalid", "port": 1521, "database": "ORCLPDB1", "user": "reader"}

    def callback(table):
        return None

    for cb in (callback, None):
        build_source(
            spec,
            secrets={"password": "s3cret"},
            source_objects=[{"name": "orders", "target": "orders"}],
            resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
            table_adapter_callback=cb,
        )
    assert captured[0]["table_adapter_callback"] is callback
    assert "table_adapter_callback" not in captured[1]


def test_build_source_sends_the_database_as_a_service_name_not_a_sid(monkeypatch):
    # SRC-9 finding G3: the URL path becomes a SID in SQLAlchemy's oracle+oracledb
    # dialect (DPY-6003 against a pluggable database); the dial's `database` is a
    # service name, so it must travel as the `service_name` query parameter. Built
    # through dlt's own credentials class and the real dialect, so the assertion is
    # on the DSN oracledb receives, not on our dict.
    from dlt.sources.credentials import ConnectionStringCredentials

    captured = {}
    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **kwargs: captured.update(kwargs) or "src")
    build_source(
        {"driver": "oracle", "host": "ora.invalid", "port": 1521, "database": "FREEPDB1", "user": "reader"},
        secrets={"password": "s3cret"},
        source_objects=[],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
    )
    assert "database" not in captured["credentials"]
    assert captured["credentials"]["query"] == {"service_name": "FREEPDB1"}

    creds = ConnectionStringCredentials()
    creds.update(captured["credentials"])
    url = creds.to_url()
    assert url.database is None and url.host == "10.0.0.9"
    from sqlalchemy.dialects.oracle.oracledb import OracleDialect_oracledb

    dialect = OracleDialect_oracledb(dbapi=__import__("oracledb"))
    _args, kwargs = dialect.create_connect_args(url)
    assert "SERVICE_NAME=FREEPDB1" in kwargs["dsn"] and "SID=" not in kwargs["dsn"]
    assert "10.0.0.9" in kwargs["dsn"] and "ora.invalid" not in kwargs["dsn"]


def _capture(monkeypatch):
    captured = []
    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **kwargs: captured.append(kwargs) or "src")
    return captured


_SPEC = {"driver": "oracle", "host": "ora.invalid", "port": 1521, "database": "ORCLPDB1", "user": "reader"}


def _resolve(h, p):
    return ResolvedAddress(ip="10.0.0.9", port=p, family=2)


def test_a_schema_qualified_object_is_read_from_that_schema(monkeypatch):
    # SRC-13 review 1 SHOULD-FIX 1: what the listing offers (`HR.EMPLOYEES`, a schema
    # other than the login's own) must load from that owner, by table name.
    captured = _capture(monkeypatch)
    build_source(_SPEC, {"password": "x"}, [{"name": "HR.EMPLOYEES", "target": "employees"}], resolve_checked=_resolve)
    assert captured[0]["schema"] == "HR"
    # Oracle's own spelling, as the listing shows it, is mapped to the one SQLAlchemy reflects by (lower case).
    assert captured[0]["table_names"] == ["employees"]


def test_a_bare_table_name_keeps_the_logins_own_schema(monkeypatch):
    captured = _capture(monkeypatch)
    build_source(_SPEC, {"password": "x"}, [{"name": "orders", "target": "orders"}], resolve_checked=_resolve)
    assert "schema" not in captured[0]


def test_objects_of_two_schemas_in_one_call_are_refused_not_half_loaded(monkeypatch):
    _capture(monkeypatch)
    with pytest.raises(ValueError, match="one schema per call"):
        build_source(
            _SPEC, {"password": "x"},
            [{"name": "HR.A", "target": "a"}, {"name": "SALES.B", "target": "b"}],
            resolve_checked=_resolve,
        )


def test_a_listed_upper_case_table_name_is_spelled_the_way_sqlalchemy_reflects_it(monkeypatch):
    # `MetaData.reflect(only=...)` compares against `Inspector.get_table_names`, which reports Oracle's
    # all-upper-case names in lower case; a name given as listed would be "not available". A mixed-case name
    # (created quoted in Oracle) is reported as it is and so is passed as it is.
    captured = _capture(monkeypatch)
    build_source(
        _SPEC, {"password": "x"},
        [{"name": "HR.EMPLOYEES", "target": "e"}, {"name": "HR.MixedCase", "target": "m"}, {"name": "HR.orders", "target": "o"}],
        resolve_checked=_resolve,
    )
    assert captured[0]["table_names"] == ["employees", "MixedCase", "orders"]


def test_column_names_from_the_preview_are_mapped_to_the_spelling_dlt_reflects():
    # SRC-4 T1.2/T1.3: the console offers Oracle's own `NOTE`, `ID`, `CREATED_AT`; the
    # reflected table says `note`, `id`, `created_at`. A name kept in the preview's spelling
    # would leave nothing out and fail to find the cursor column (`table.c[...]`).
    from dispar_orchestrate.adapters.oracle import in_reflected_spelling

    obj = {
        "name": "HR.ORDERS",
        "target": "orders",
        "excludedColumns": ["NOTE", "Mixed", "already_lower"],
        "primaryKey": ["ID", "REGION"],
        "incrementalKey": " CREATED_AT ",
        "loadMode": "incremental_dedup",
    }
    changed = in_reflected_spelling(obj)
    assert changed["excludedColumns"] == ["note", "Mixed", "already_lower"]
    assert changed["primaryKey"] == ["id", "region"]
    assert changed["incrementalKey"] == "created_at"
    # The object's own name is not a column: untouched, and the input is not modified.
    assert changed["name"] == "HR.ORDERS" and obj["incrementalKey"] == " CREATED_AT "


def test_an_object_without_column_names_passes_through_unchanged():
    from dispar_orchestrate.adapters.oracle import in_reflected_spelling

    obj = {"name": "ORDERS", "target": "orders"}
    assert in_reflected_spelling(obj) == obj


def test_a_column_left_out_by_its_oracle_spelling_is_left_out_of_the_load():
    # Through the gate's own function: `NOTE` from the console must remove the reflected `note`.
    from dispar_orchestrate import schema_observer
    from dispar_orchestrate.adapters.oracle import in_reflected_spelling
    from dispar_orchestrate.ingest_factory import _leave_out_columns

    reflected = schema_observer.ReflectedTable(
        columns=(
            schema_observer.ReflectedColumn(name="id", type_name="NUMBER", nullable=False),
            schema_observer.ReflectedColumn(name="note", type_name="VARCHAR2", nullable=True),
        ),
        primary_key=("id",),
    )
    decision = schema_observer.Decision(action="load", columns=None, changes=[])
    excluded = frozenset(in_reflected_spelling({"excludedColumns": ["NOTE"]})["excludedColumns"])
    kept, left_out = _leave_out_columns(decision, reflected, excluded)
    assert left_out == 1 and kept.columns == ["id"]
