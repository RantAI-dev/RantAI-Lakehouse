"""dagster/dispar_orchestrate/adapters/sql.py -- wraps dlt's
`sql_database` source for a `sql`-adapter connector (mysql/mariadb via
`pymysql`, SQL Server via Microsoft ODBC Driver 18/`pyodbc`; Postgres
batch stays on `dlt_pipeline.py`'s existing path in THIS workstream --
see "Open question 2, resolved" below).

SSRF (WS3 plan review Z1) and PER-DRIVER PINNING (WS3 plan review Z7):
`dial.host` is a `connector:manage` principal's own choice, dialed from
inside the `dagster-code-location` container (see `ssrf_guard.py`'s
module docstring for the full threat model). `build_source` calls
`ssrf_guard.resolve_checked` BEFORE building any credentials, and pins
the CONNECTION ITSELF to the checked address for every driver -- not
just relying on the caller's later `ssrf_guard.pinned_resolution` wrap
(a future `ingest_factory.py`, WS3 plan review Z1), which only protects a
driver that resolves through `socket.getaddrinfo` (`pymysql`, the
pure-Python mysql/mariadb driver).

`ssrf_guard.pinned_resolution`'s own docstring states the limitation
this module exists to close: **`psycopg2`/libpq and Microsoft's ODBC
Driver 18 do not resolve through `socket.getaddrinfo` at all** -- they
are C libraries that call the OS resolver directly, bypassing that
monkeypatch entirely. Wrapping either driver's connect call in
`pinned_resolution` and calling it "pinned" would be a no-op that still
lets a rebind attacker point the real TCP connection anywhere between
the check and the dial. So each SQL driver here gets pinning through the
mechanism its own connection API actually offers:

- **`mysql`/`mariadb` (`pymysql`):** pure Python, resolves via
  `socket.getaddrinfo` -- a future `ingest_factory.py`'s
  `pinned_resolution` wrap gets FULL pinning with no compensating code
  in this module. Verified by absence in
  `test_build_source_needs_no_extra_pinning_for_pymysql`:
  no `engine_kwargs` at all for this driver.
- **`mssql` (Microsoft ODBC Driver 18, `pyodbc`):** connects via a raw
  ODBC connection string built by `_mssql_connection_string` below,
  carrying `Server=tcp:<checked-ip>,<port>` directly -- it never calls a
  resolver of its own at all, so there is nothing for `pinned_resolution`
  to intercept even if it were wrapped around this call.
  `HostNameInCertificate=<host>` keeps TLS certificate verification
  targeting the real hostname even though the socket dials the IP.

(A `postgresql` driver, if this module ever grows one, would need the
SAME kind of per-driver pin: `psycopg2`/libpq accepts a separate
`hostaddr` connect parameter alongside `host` -- `host` stays the name
for TLS/SNI, `hostaddr` pins the checked IP, and libpq performs no DNS
lookup of its own. `_DRIVERNAMES` below has no `"postgresql"` entry
because that pin is applied at `dlt_pipeline.py`'s own, separate
Postgres call site by a separate follow-up task, not in this module --
see "Open question 2, resolved" below.)

# ODBC injection (WS3 plan review Z13)

An ODBC connection string is `KEY=value;`-delimited, so `;`, `{` and `}`
are structurally significant. `database`/`user`/`password` are
principal-controlled (or, for `password`, resolved from a
principal-chosen `secretRef`) and are brace-quoted (`_odbc_quote`) before
interpolation: unquoted, a `database` value of
`"x;TrustServerCertificate=yes"` would inject an attribute AHEAD of this
module's own fixed `Encrypt=yes;TrustServerCertificate=no;`, switching
off certificate verification for the very credential the string sends.
`host` cannot be brace-quoted the same way -- some ODBC driver managers
do not apply `{}`-quoting uniformly to every keyword -- so it is
validated against `_validate_hostname` instead and REFUSED outright if
it does not match a plain DNS-hostname/IPv4-literal shape. `host` also
never reaches `_odbc_quote` as a candidate, since a rejected shape is
strictly safer than proving a quoting rule for it.

`_validate_hostname` is `rust/crates/lakehouse-store/src/ingest_spec.rs`'s
`validate_hostname` (WS3 plan review Z13) ported VERBATIM: RFC 1123 labels
(letters/digits/hyphens, no leading/trailing hyphen, 1-63 chars each)
separated by `.`, total length <= 253. Its allowlist -- alphanumerics and
`-` per label only -- is why an IPv4 literal such as `10.0.0.9` passes
(each dot-separated label is all digits, which the alphanumeric check
allows) while an IPv6 literal never can: `:` is not in the allowlist,
and `:` is itself a delimiter in an ODBC connection string
(`Server=tcp:<host>,<port>`), so refusing it is exactly the property
this rule exists for. Keep both copies in sync (same discipline as
`secret_resolver.py`/`secret.rs`'s `CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`,
WS3 plan review Z6, and `column_gate.py`/`cdc.rs`'s
`NESTED_TYPE_MARKERS`, WS3 plan review X9): a Rust-side change to
`validate_hostname` must land in the SAME commit as a change here.

Open question 2, resolved (WS3 plan review Z10): Postgres batch STAYS on
`dlt_pipeline.py`'s existing, unchanged code path in this workstream --
deferred to a separate follow-up task, not migrated here, to keep this
task's diff small. This module's `_DRIVERNAMES` therefore has no
`"postgresql"` entry.
"""

from __future__ import annotations

import re
import urllib.parse
from dataclasses import dataclass
from typing import Any, Callable

from dlt.sources.sql_database import sql_database

from dispar_orchestrate import ssrf_guard

_DRIVERNAMES = {
    "mysql": "mysql+pymysql",
    "mariadb": "mysql+pymysql",
}


@dataclass(frozen=True)
class AdapterBuildResult:
    """The dlt source `build_source` produced, plus the address
    `resolve_checked` validated (a caller such as `ingest_factory.py`
    uses `resolved` to wrap the actual dial in `ssrf_guard.pinned_resolution`
    for the drivers that need it -- see this module's docstring)."""

    source: Any
    resolved: ssrf_guard.ResolvedAddress


# Ported verbatim from rust/crates/lakehouse-store/src/ingest_spec.rs's
# `validate_hostname` (WS3 plan review Z13) -- see the module docstring's "ODBC
# injection" section for why this exact allowlist (alphanumerics and `-`
# per label) is the rule this module needs, not a looser or a different
# one. Keep both in sync.
_HOSTNAME_LABEL_RE = re.compile(r"^[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?$")


def _validate_hostname(field: str, host: str) -> None:
    if not host or len(host) > 253 or not all(_HOSTNAME_LABEL_RE.match(label) for label in host.split(".")):
        raise ValueError(f"{field} is not a valid hostname: {host!r}")


def _reject_control_characters(field: str, value: str) -> None:
    if any(ord(c) < 0x20 or ord(c) == 0x7F for c in value):
        raise ValueError(f"{field} contains a control character: {value!r}")


def _odbc_quote(value: str) -> str:
    """ODBC connection-string brace-quoting (WS3 plan review Z13): `{` +
    value with every `}` doubled + `}`. Once a value is wrapped this way,
    `;` and `=` inside it are ordinary characters, not delimiters -- an
    injection-shaped value like `x;TrustServerCertificate=yes` stays
    entirely inside its own `KEY={...}` pair and can never start a new
    attribute."""
    return "{" + value.replace("}", "}}") + "}"


def _mssql_connection_string(spec: dict, secrets: dict[str, str], resolved: ssrf_guard.ResolvedAddress) -> str:
    """A raw ODBC connection string, not SQLAlchemy's structured
    host/port fields -- those would re-encode `host` themselves (and
    `mssql+pyodbc` has no `hostaddr`-equivalent parameter).
    `Server=tcp:<ip>,<port>` pins the connection directly to the checked
    address; `HostNameInCertificate=<host>` keeps TLS certificate
    verification targeting the real hostname.

    See the module docstring's "ODBC injection" section for why
    `database`/`user`/`password` are brace-quoted (`_odbc_quote`) and why
    `host` is validated (`_validate_hostname`) rather than quoted. The
    fixed security attributes (`Encrypt=yes;TrustServerCertificate=no;`)
    are written FIRST, before any interpolated value, as defence in depth
    alongside the quoting -- never relying on quoting alone to keep them
    from being shadowed.
    """
    _validate_hostname("host", spec["host"])
    _reject_control_characters("database", spec["database"])
    _reject_control_characters("user", spec["user"])

    odbc = (
        "DRIVER={ODBC Driver 18 for SQL Server};"
        "Encrypt=yes;TrustServerCertificate=no;"
        f"SERVER=tcp:{resolved.ip},{spec['port']};"
        f"HostNameInCertificate={spec['host']};"
        f"DATABASE={_odbc_quote(spec['database'])};"
        f"UID={_odbc_quote(spec['user'])};"
        f"PWD={_odbc_quote(secrets['password'])};"
    )
    return "mssql+pyodbc:///?odbc_connect=" + urllib.parse.quote_plus(odbc)


def build_source(
    spec: dict,
    secrets: dict[str, str],
    source_objects: list[dict],
    *,
    resolve_checked: Callable[[str, int], ssrf_guard.ResolvedAddress] = ssrf_guard.resolve_checked,
) -> AdapterBuildResult:
    """Build a dlt `sql_database` source for `spec['driver']`
    (`mysql`/`mariadb`/`mssql`), SSRF-checked and pinned per-driver.

    `resolve_checked` runs BEFORE any credential is built -- a blocked
    host never gets as far as `sql_database` being called at all (see
    `test_build_source_checks_the_host_before_building_any_credentials`).
    """
    driver = spec["driver"]
    resolved = resolve_checked(spec["host"], spec["port"])
    table_names = [obj["name"].split(".")[-1] for obj in source_objects]

    if driver == "mssql":
        credentials: Any = _mssql_connection_string(spec, secrets, resolved)
        source = sql_database(credentials=credentials, table_names=table_names)
        return AdapterBuildResult(source=source, resolved=resolved)

    drivername = _DRIVERNAMES.get(driver)
    if drivername is None:
        raise ValueError(f"sql adapter does not know driver {driver!r}")
    credentials = {
        "drivername": drivername,
        "username": spec["user"],
        "password": secrets["password"],
        "host": spec["host"],
        "port": spec["port"],
        "database": spec["database"],
    }
    # pymysql (mysql/mariadb) is pure Python and resolves via
    # socket.getaddrinfo -- a future ingest_factory.py's
    # ssrf_guard.pinned_resolution wrap gets FULL pinning here with no
    # compensating engine_kwargs (see the module docstring's per-driver
    # pinning list).
    source = sql_database(credentials=credentials, table_names=table_names)
    return AdapterBuildResult(source=source, resolved=resolved)
