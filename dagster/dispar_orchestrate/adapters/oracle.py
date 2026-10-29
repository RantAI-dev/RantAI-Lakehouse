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

**The dial's `database` is a SERVICE NAME (`SRC-9` finding G3).** The console
labels the field "Database (service name)" (`oracle-dial-form.tsx`) and a
pluggable database is reached only by service name. SQLAlchemy's
`oracle+oracledb` dialect turns the URL path (`url.database`) into a SID
(`makedsn(sid=...)`) and `?service_name=` into a service name, so passing
`database` as the path made the g6 Oracle gate fail on its first run with
`DPY-6003: SID "FREEPDB1" is not registered with the listener` (ORA-12505).
The dial's `database` is therefore sent as `query={"service_name": ...}`,
and the URL path stays empty (the dialect rejects both together). Only the
descriptor changes: the host is still the checked IP literal. No deployment
could have relied on the SID reading: the type was never loaded by a gate
before `SRC-9` and the picker names the field a service name. Limit: a
database reachable only by SID (a container database with no service
registered, or a legacy listener) is not supported; it would need a separate
dial field, for example `sid`, which does not exist.

**Load modes and column choice (`SRC-4` T1.2, T1.3).** `incremental` and
`incremental_dedup` need nothing Oracle-specific in the sink: `sql_database`
pushes the cursor into the query like it does for every SQL driver
(`sink._apply_cursor`). What is Oracle's own is the spelling of column names:
the console takes them from the preview (`discover.py`), in Oracle's spelling
(`CREATED_AT`), while the reflected table, the rows and dlt's cursor lookup say
`created_at`. `in_reflected_spelling` maps `excludedColumns`, `primaryKey` and
`incrementalKey` the way `build_source` already maps table names, before
`ingest_factory` builds the plan and the before-load gate.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from dlt.sources.sql_database import sql_database
from sqlalchemy.dialects.oracle.oracledb import OracleDialect_oracledb

from dispar_orchestrate import ssrf_guard


class OracleTlsConfigError(Exception):
    """`sslMode` requires TLS but no `sslServerCertDn` was supplied --
    refused, never a silent connection with the server's identity
    unverified."""


@dataclass(frozen=True)
class AdapterBuildResult:
    source: Any
    resolved: ssrf_guard.ResolvedAddress


def connection_parts(
    spec: dict,
    secrets: dict[str, str],
    *,
    resolve_checked=ssrf_guard.resolve_checked,
) -> tuple[dict[str, Any], dict[str, Any], ssrf_guard.ResolvedAddress]:
    """The SQLAlchemy `credentials` and `engine_kwargs` for this dial, and the
    address `resolve_checked` approved: everything about the connection that the
    module docstring's SSRF and TLS paragraphs describe, in one place. Shared by
    `build_source` and the table discovery (`discover.py`, SRC-13), so a listing
    dials exactly as a load does: the checked IP literal, the service name, the
    operator's certificate DN. The caller opens the connection inside
    `ssrf_guard.checking_resolver()`.

    # Errors

    `OracleTlsConfigError` when `sslMode` requires TLS and `sslServerCertDn` is
    unset; whatever `resolve_checked` raises for a blocked host.
    """
    resolved = resolve_checked(spec["host"], spec["port"])

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
        # SERVICE NAME, not SID (SRC-9 finding G3): see the module docstring.
        # `database` stays unset because the dialect refuses both together.
        "query": {"service_name": spec["database"]},
    }
    return credentials, engine_kwargs, resolved


def in_reflected_spelling(obj: dict) -> dict:
    """A copy of a source object whose column names (`excludedColumns`,
    `incrementalKey`, `primaryKey`) are spelled the way `sql_database` reflects
    them (`SRC-4` T1.2, T1.3).

    The console takes column names from the preview (`discover.py _oracle`),
    which reads Oracle's own spelling, `CREATED_AT`. SQLAlchemy's Oracle dialect
    reports an all-upper-case name in lower case, so the reflected table, the
    rows and the cursor lookup (`table.c[...]`) say `created_at`, and a name left
    in the preview's spelling would leave nothing out (`ingest_factory.
    _leave_out_columns` compares exact names) or fail to find its cursor column.
    The dialect's own `normalize_name` is the same map `build_source` uses for
    table names, so a name that is already lower case, or a quoted mixed-case
    one, passes through unchanged.
    """
    normalize = OracleDialect_oracledb().normalize_name
    changed = dict(obj)
    for field in ("excludedColumns", "primaryKey"):
        names = obj.get(field)
        if isinstance(names, (list, tuple)):
            changed[field] = [normalize(n) if isinstance(n, str) else n for n in names]
    cursor = obj.get("incrementalKey")
    if isinstance(cursor, str) and cursor.strip():
        changed["incrementalKey"] = normalize(cursor.strip())
    return changed


def build_source(
    spec: dict,
    secrets: dict[str, str],
    source_objects: list[dict],
    *,
    resolve_checked=ssrf_guard.resolve_checked,
    table_adapter_callback=None,
) -> AdapterBuildResult:
    """`table_adapter_callback` (SRC-8, `schema_observer.py`): `sql_database`
    calls it with each reflected table while the source is built, which is
    below, inside `checking_resolver()` and over the checked IP literal -- the
    observation reads the columns from that reflection and opens no
    connection of its own."""
    credentials, engine_kwargs, resolved = connection_parts(spec, secrets, resolve_checked=resolve_checked)
    # SRC-13: the listing (`discover.py`) returns Oracle's own spelling, `EMPLOYEES`. SQLAlchemy's Oracle
    # dialect reports an all-upper-case name in lower case ("lower case is the case-insensitive spelling") and
    # `MetaData.reflect(only=[...])` refuses a name that is not in that list, so `EMPLOYEES` would not load
    # while `employees` does. The dialect's own `normalize_name` maps one to the other, so what is listed loads.
    normalize = OracleDialect_oracledb().normalize_name
    table_names = [normalize(obj["name"].split(".")[-1]) for obj in source_objects]
    # SRC-13 review 1 SHOULD-FIX 1 (D5: the listing offers every schema the user
    # can read): the object's `SCHEMA.TABLE` names the OWNER to read from. Before,
    # only the table part was kept and `sql_database` read the login's own schema,
    # so a table listed from another schema did not load (or loaded a same-named
    # table of the login's own). `sql_database` takes one `schema` per source, and
    # a load builds one source per object, so objects of two schemas in one call
    # are refused rather than half-loaded. A bare `TABLE` keeps the login's schema.
    schemas = {obj["name"].rpartition(".")[0] for obj in source_objects} - {""}
    if len(schemas) > 1:
        raise ValueError("oracle build_source reads one schema per call; got " + ", ".join(sorted(schemas)))
    schema_kwargs: dict[str, Any] = {"schema": next(iter(schemas))} if schemas else {}

    with ssrf_guard.checking_resolver():
        extra = {} if table_adapter_callback is None else {"table_adapter_callback": table_adapter_callback}
        extra.update(schema_kwargs)
        source = sql_database(
            credentials=credentials, table_names=table_names, engine_kwargs=engine_kwargs, **extra
        )
    return AdapterBuildResult(source=source, resolved=resolved)
