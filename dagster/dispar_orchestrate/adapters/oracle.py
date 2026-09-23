"""dagster/dispar_orchestrate/adapters/oracle.py -- wraps dlt's
`sql_database` with the `oracle+oracledb` SQLAlchemy dialect (thin mode,
no Oracle Instant Client). SSRF: one design, not a
branch: `resolve_checked` runs first; the connection's own `host`
field is the CHECKED IP LITERAL (never the original hostname), so
nothing downstream can re-resolve or DNS-rebind it; the whole call is
additionally wrapped in `ssrf_guard.checking_resolver` as a
belt-and-suspenders check that costs nothing if thin mode's internal
transport never calls `socket.getaddrinfo` and catches it if it does.

**Proved, not assumed: what `oracledb==2.5.1` thin
mode actually does.** The installed wheel ships no `.pyx` source (it is
a compiled `thin_impl.cpython-314-x86_64-linux-gnu.so`), so the sdist
(`pip download oracledb==2.5.1 --no-binary :all:`) was unpacked and its
real Cython source read directly:
`src/oracledb/impl/thin/protocol.pyx`, `Protocol._connect_tcp`
(the synchronous path -- the one SQLAlchemy's `oracle+oracledb` dialect
uses; dlt's `sql_database` never opens an async engine) resolves the
connection with:

    connect_info = (host, port)
    ...
    sock = socket.create_connection(connect_info, timeout)

`socket.create_connection` is the Python standard library's own
function (`socket.py`), not a C-level bypass -- its body does
`for res in getaddrinfo(host, port, 0, SOCK_STREAM):`, an UNQUALIFIED
reference resolved from the enclosing `socket` module's own globals at
call time, i.e. `socket.getaddrinfo` -- exactly the attribute
`ssrf_guard.checking_resolver`/`pinned_resolution` monkeypatch. So thin
mode's real dial DOES go through the checkable resolver on this path;
`checking_resolver` is genuine defence in depth here, not a no-op.
(A second branch exists for `description.use_tcp_fast_open`, which
calls `sock.sendto(..., connect_info)` on a raw socket without going
through `socket.create_connection`/`getaddrinfo` at all -- irrelevant to
this module because `host` is already the checked IP literal below, so
neither path ever has a hostname left to resolve.)

TLS certificate verification: `spec.get("sslServerCertDn")`
is an OPERATOR-SUPPLIED Distinguished Name (`SqlDial.ssl_server_cert_dn`,
`rust/crates/lakehouse-store/src/ingest_spec.rs`), passed to oracledb UNCHANGED -- never synthesized from
`spec['host']` (a bare `CN=<hostname>` would not match a real
certificate's full DN, which ordinarily carries `OU=`/`O=`/`C=`
components too). `ssl_server_cert_dn` is a real, documented
`oracledb.connect()` keyword argument (`connect_params.py`, verified
against the same unpacked sdist) whose own doc comment states: "if the
ssl_server_cert_dn parameter is not provided, host name matching is"
performed instead -- i.e. against the CONNECTION's `host` field, which
this module deliberately sets to an IP literal, so an unset DN here
would silently degrade to matching a certificate against an IP address,
never the name it was actually issued for. Required whenever `sslMode`
requires TLS: if it is missing, `build_source` refuses BEFORE calling
`sql_database` at all -- `OracleTlsConfigError`, stating that the
server's identity cannot be verified when connecting by a checked IP
literal. Plaintext (`sslMode` unset/`"disable"`) sets no DN check at
all -- stated, not silently dropped.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from dlt.sources.sql_database import sql_database

from dispar_orchestrate import ssrf_guard


class OracleTlsConfigError(Exception):
    """`sslMode` requires TLS but no `sslServerCertDn` was supplied --
    refused, never a silent connection with the server's identity
    unverified."""


@dataclass(frozen=True)
class AdapterBuildResult:
    source: Any
    resolved: ssrf_guard.ResolvedAddress


def build_source(
    spec: dict,
    secrets: dict[str, str],
    source_objects: list[dict],
    *,
    resolve_checked=ssrf_guard.resolve_checked,
) -> AdapterBuildResult:
    resolved = resolve_checked(spec["host"], spec["port"])
    table_names = [obj["name"].split(".")[-1] for obj in source_objects]

    tls_required = bool(spec.get("sslMode")) and spec["sslMode"] != "disable"
    engine_kwargs: dict[str, Any] = {}
    if tls_required:
        cert_dn = spec.get("sslServerCertDn")
        if not cert_dn:
            raise OracleTlsConfigError(
                "sslMode requires TLS but no sslServerCertDn was configured -- the server's "
                "identity cannot be verified when connecting by IP; set sslServerCertDn to the "
                "certificate's expected Distinguished Name, or set sslMode to disable"
            )
        # The operator-supplied DN, passed through EXACTLY as configured
        # -- never derived from spec["host"].
        engine_kwargs["connect_args"] = {"ssl_server_cert_dn": cert_dn}

    # `credentials` is a plain Python dict of STRUCTURED fields handed to SQLAlchemy's
    # `create_engine`, never an interpolated DSN string -- a
    # `user`/`password`/`database` value containing `@`/`/`/`:` cannot
    # break out of its own field. No `_odbc_quote` equivalent is
    # needed, for the same reason `mongodb.py` needs none: only
    # `mssql`'s raw ODBC connection string (`adapters/sql.py`) lacks a
    # structured-parameter escape hatch.
    credentials = {
        "drivername": "oracle+oracledb",
        "username": spec["user"],
        "password": secrets["password"],
        "host": resolved.ip,  # the CHECKED literal IP -- never spec["host"]
        "port": spec["port"],
        "database": spec["database"],
    }
    with ssrf_guard.checking_resolver():
        source = sql_database(credentials=credentials, table_names=table_names, engine_kwargs=engine_kwargs)
    return AdapterBuildResult(source=source, resolved=resolved)
