"""Tests for `dispar_orchestrate.adapters.oracle`.
No network: `sql_database` and `resolve_checked` are both injected/
monkeypatched, so nothing here dials a real socket. Each test asserts
one specific behaviour of `build_source`'s design (connect by resolved
IP, TLS verified against an operator-supplied DN, checking_resolver-
wrapped) -- never merely "it did not crash"."""
from __future__ import annotations

import pytest

from dispar_orchestrate.adapters.oracle import OracleServiceNameError, OracleTlsConfigError, build_source
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


def test_an_incremental_cursor_in_the_preview_spelling_is_found_only_after_the_mapping(tmp_path):
    # SRC-4 T1.3: `sql_database` looks the cursor up as `table.c[name]`, exactly. Oracle's
    # preview says `CREATED_AT` while the reflected column is `created_at` (SQLite stands in
    # for the reflected spelling; no server). The sink's `_apply_cursor` is the real one.
    import sqlite3

    from dlt.extract.exceptions import ResourceExtractionError
    from dlt.sources.sql_database import sql_database

    from dispar_orchestrate.adapters import sink
    from dispar_orchestrate.adapters.oracle import in_reflected_spelling

    db = tmp_path / "src.db"
    with sqlite3.connect(db) as conn:
        conn.execute("create table orders (id integer primary key, created_at integer)")
        conn.executemany("insert into orders values (?, ?)", [(1, 10), (2, 20)])
    obj = {"name": "ORDERS", "target": "orders", "loadMode": "incremental", "incrementalKey": "CREATED_AT"}

    def rows_for(cursor: str) -> list[dict]:
        source = sql_database(credentials={"drivername": "sqlite", "database": str(db)}, table_names=["orders"])
        sink._apply_cursor(source, cursor)
        return list(source.orders)

    with pytest.raises(ResourceExtractionError, match="CREATED_AT"):
        rows_for(obj["incrementalKey"])
    plan = sink.LoadPlan.from_source_object(in_reflected_spelling(obj))
    assert plan.cursor == "created_at"
    assert [r["id"] for r in rows_for(plan.cursor)] == [1, 2]


def test_a_tls_dial_hands_oracledb_a_tcps_address_not_a_tcp_descriptor():
    # SRC-4 T1.4: `makedsn` always writes (PROTOCOL=TCP), and thin mode ignores `protocol` and the
    # DN for such a descriptor, so the keywords below (and `dsn=None`) are what make it TLS.
    from dispar_orchestrate.adapters.oracle import connection_parts

    spec = {**_SPEC, "port": 2484, "sslMode": "required", "sslServerCertDn": "CN=ora.invalid"}
    _credentials, engine_kwargs, _resolved = connection_parts(spec, {"password": "x"}, resolve_checked=_resolve)
    assert engine_kwargs["connect_args"] == {
        "dsn": None,
        "host": "10.0.0.9",  # the checked literal, never the name
        "port": 2484,
        "service_name": "ORCLPDB1",
        "protocol": "tcps",
        "ssl_server_cert_dn": "CN=ora.invalid",
    }
    plain = connection_parts(_SPEC, {"password": "x"}, resolve_checked=_resolve)[1]
    assert "connect_args" not in plain  # no sslMode: the dial stays plaintext, as documented


def _tls_server(tmp_path, monkeypatch, common_name: str):
    """A TLS listener on loopback with a certificate from a throwaway CA, and that CA as the
    default trust store (`SSL_CERT_FILE`, as the gate does for its fixture). It completes the
    handshake and closes: what the test reads is whether `oracledb` got that far, and how it
    answered the certificate. Returns `(port, handshakes)`."""
    import datetime
    import socket
    import ssl
    import threading

    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import rsa
    from cryptography.x509.oid import NameOID

    def name(cn: str) -> x509.Name:
        return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, cn)])

    def key():
        return rsa.generate_private_key(public_exponent=65537, key_size=2048)

    now = datetime.datetime.now(datetime.timezone.utc)
    ca_key, server_key = key(), key()
    ca = (
        x509.CertificateBuilder().subject_name(name("g6-test-ca")).issuer_name(name("g6-test-ca"))
        .public_key(ca_key.public_key()).serial_number(1)
        .not_valid_before(now - datetime.timedelta(minutes=1)).not_valid_after(now + datetime.timedelta(days=1))
        .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
        .sign(ca_key, hashes.SHA256())
    )
    leaf = (
        x509.CertificateBuilder().subject_name(name(common_name)).issuer_name(ca.subject)
        .public_key(server_key.public_key()).serial_number(2)
        .not_valid_before(now - datetime.timedelta(minutes=1)).not_valid_after(now + datetime.timedelta(days=1))
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .sign(ca_key, hashes.SHA256())
    )
    ca_file, cert_file, key_file = tmp_path / "ca.pem", tmp_path / "server.pem", tmp_path / "server.key"
    ca_file.write_bytes(ca.public_bytes(serialization.Encoding.PEM))
    cert_file.write_bytes(leaf.public_bytes(serialization.Encoding.PEM))
    key_file.write_bytes(
        server_key.private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()
        )
    )
    monkeypatch.setenv("SSL_CERT_FILE", str(ca_file))

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert_file, key_file)
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(5)
    handshakes: list[str] = []

    def serve() -> None:
        while True:
            try:
                connection, _ = listener.accept()
            except OSError:
                return
            try:
                with context.wrap_socket(connection, server_side=True) as tls:
                    handshakes.append("ok")
                    tls.recv(16)
            except (ssl.SSLError, OSError):
                handshakes.append("refused")

    threading.Thread(target=serve, daemon=True).start()
    return listener, handshakes


def _dial_tls(port: int, dn: str) -> str:
    """Open the engine `connection_parts` describes and return `oracledb`'s own message."""
    from sqlalchemy import create_engine
    from sqlalchemy.engine import URL
    from sqlalchemy.exc import DBAPIError

    from dispar_orchestrate.adapters.oracle import connection_parts

    spec = {**_SPEC, "host": "ora.invalid", "port": port, "sslMode": "required", "sslServerCertDn": dn}
    credentials, engine_kwargs, _ = connection_parts(
        spec, {"password": "x"}, resolve_checked=lambda h, p: ResolvedAddress(ip="127.0.0.1", port=p, family=2)
    )
    engine = create_engine(URL.create(**credentials), **engine_kwargs)
    try:
        with pytest.raises(DBAPIError) as failure:
            engine.connect()
    finally:
        engine.dispose()
    return str(failure.value.orig)


def test_a_tls_dial_really_encrypts_and_checks_the_certificate_dn(tmp_path, monkeypatch):
    # SRC-4 T1.4, measured not assumed: a loopback TLS server signed by a private CA. The old
    # call (a `makedsn` descriptor plus `ssl_server_cert_dn`) never produced a handshake.
    import time

    listener, handshakes = _tls_server(tmp_path, monkeypatch, "oracle-tls-g6")
    try:
        port = listener.getsockname()[1]

        # The right DN: the handshake completes and the driver goes on to speak TNS, which the
        # stand-in server does not, so the connection ends (DPY-4011) -- but not on the certificate.
        right = _dial_tls(port, "CN=oracle-tls-g6")
        time.sleep(0.3)
        assert handshakes == ["ok"]
        assert "DPY-6002" not in right

        # A DN the certificate does not carry: refused by the driver, whatever the chain.
        wrong = _dial_tls(port, "CN=somebody-else")
        assert "DPY-6002" in wrong
    finally:
        listener.close()


@pytest.mark.parametrize("name", ["a)(b", "a=b", "a b", "(a", "", "A" * 129])
def test_build_source_refuses_a_service_name_that_is_not_an_identifier(monkeypatch, name):
    # SRC-4 slice 1 review SHOULD-FIX 3: a fixed error that does not repeat the value.
    monkeypatch.setattr("dispar_orchestrate.adapters.oracle.sql_database", lambda **_: pytest.fail("must not be called"))
    with pytest.raises(OracleServiceNameError) as err:
        build_source(
            {"driver": "oracle", "host": "ora.invalid", "port": 1521, "database": name, "user": "reader"},
            secrets={"password": "s3cret"},
            source_objects=[],
            resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
        )
    assert name == "" or name not in str(err.value)
