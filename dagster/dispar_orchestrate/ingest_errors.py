"""dagster/dispar_orchestrate/ingest_errors.py -- how an ingest failure is
named, and whether it is tried again (`SRC-11` decisions D1 and D2, `SRC-19`).

One function, `classify`, turns any exception an ingest step can raise into
one word of a CLOSED list (`ERROR_CLASSES`, the list `lakehouse-api` accepts
in a run report: `rust/crates/lakehouse-store/src/connector_run.rs`) and says
whether that word is retried:

    retried (transient)   unreachable, timeout, source_busy, api_unreachable
    not retried           sign_in_refused, address_refused, unsupported,
                          setting_rejected, schema_decision_pending, other

`other` is a failure nobody taught this module about. It is NOT retried: a
retry of something unexplained only delays the failure alert, and the
class's name says plainly that it was not understood. The cost is that a
transient failure from a driver this module does not know fails at once; the
fix is a line in `_classify_one`, with a test.

Before `SRC-11` the classification was scattered: `ingest_source_object`
listed config-shaped types in an `except`, `_is_authentication_refusal`
covered three drivers, and the Kafka path classified nothing. All of it is
here now.

# What is read, and what never leaves

An exception is recognised by its TYPE and its driver ERROR CODE (SQLSTATE,
`pymysql` error number, `oracledb` code, HTTP status), never by message text,
with ONE exception: PostgreSQL's refused sign-in. `psycopg2` raises a refusal
while CONNECTING with no SQLSTATE at all (checked against the pinned 2.9.10:
`pgcode` and `diag.sqlstate` are `None`, libpq reports none for a failed
connection), so when the code is missing the server's own fixed wording
`password authentication failed` is looked for (`SRC-19`). That text is read
inside this process only; the result is the word `sign_in_refused`. The text
is never logged, stored or sent, and nothing in this module returns any part
of an exception's message.

Drivers are recognised by the module their exception class claims (`pymysql`,
`pyodbc`, `oracledb`, `psycopg2`, `kafka`, `paramiko`, `pymongo`), so this
file imports none of them (`pyodbc` cannot be imported without a system ODBC
library).

# Retry delays

`INGEST_RETRY_DELAYS_SECONDS` (default `60,300,900`: 1, 5 and 15 minutes, the
spec's) is read by `RetrySettings.from_env`. Dagster's `RetryPolicy` cannot
express a different wait per attempt, so the ingest ops raise
`dagster.RetryRequested(seconds_to_wait=...)` themselves, with the delay
`RetrySettings.delay_for` gives for the attempt. The setting exists so the
acceptance gate can shorten the waits; a value that cannot be read falls back
to the default and says so in the log.
"""

from __future__ import annotations

import logging
import os
import socket
from collections.abc import Iterator
from dataclasses import dataclass

from dispar_orchestrate import schema_observer, ssrf_guard
from dispar_orchestrate import ssrf_guard_mongo as ssrf_guard_mongo_module
from dispar_orchestrate import ssrf_guard_sftp as ssrf_guard_sftp_module
from dispar_orchestrate.adapters import dedup as dedup_adapter
from dispar_orchestrate.adapters import oracle as oracle_adapter
from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import UnsupportedLoadMode
from dispar_orchestrate.column_gate import UnsupportedColumnType
from dispar_orchestrate.saas import session as saas_session
from dispar_orchestrate.saas import vendors as saas_vendors
from dispar_orchestrate.secret_resolver import SecretRefRejected

logger = logging.getLogger(__name__)

# The closed list, in the order the API's documentation gives it. A value
# outside it is a 400 at the report route, so a typo here cannot be sent.
UNREACHABLE = "unreachable"
TIMEOUT = "timeout"
SOURCE_BUSY = "source_busy"
API_UNREACHABLE = "api_unreachable"
SIGN_IN_REFUSED = "sign_in_refused"
ADDRESS_REFUSED = "address_refused"
UNSUPPORTED = "unsupported"
SETTING_REJECTED = "setting_rejected"
SCHEMA_DECISION_PENDING = "schema_decision_pending"
OTHER = "other"

RETRIED_CLASSES = frozenset({UNREACHABLE, TIMEOUT, SOURCE_BUSY, API_UNREACHABLE})
ERROR_CLASSES = RETRIED_CLASSES | {
    SIGN_IN_REFUSED,
    ADDRESS_REFUSED,
    UNSUPPORTED,
    SETTING_REJECTED,
    SCHEMA_DECISION_PENDING,
    OTHER,
}

# The fixed phrase for each class, for a Dagster `Failure` description. The
# API has its own copy for the run log (`connector_run.rs`); neither holds
# source text.
CLASS_PHRASES = {
    UNREACHABLE: "the source could not be reached",
    TIMEOUT: "the source did not answer in time",
    SOURCE_BUSY: "the source was busy or failed",
    API_UNREACHABLE: "the product's own API could not be reached",
    SIGN_IN_REFUSED: "the source refused the connector's credential",
    ADDRESS_REFUSED: "the source's address was refused",
    UNSUPPORTED: "the table or load mode is not supported",
    SETTING_REJECTED: "a connector setting was rejected",
    SCHEMA_DECISION_PENDING: "the table waits for a schema decision",
    OTHER: "the load failed",
}


class UnknownAdapter(Exception):
    """`connector["adapter"]` names no dispatch arm `ingest_factory` knows how
    to run -- raised BY NAME rather than left to surface as a bare
    `KeyError` from a dict lookup, so a future adapter added to the
    `connector_type`/`connector` CHECK constraints (`0033_connector_ingest_spec.sql`,
    `0043_ingest_tier2_adapters.sql`) without a matching dispatch arm fails
    loudly and legibly at run time, never a silent no-op."""


class UnsupportedKafkaAuth(Exception):
    """The dial names a Kafka auth type this consumer cannot honour. Raised
    before any connection is attempted."""


class ApiUnreachable(Exception):
    """The product's own API could not be asked for the connector (a
    connection error or a 5xx). Transient. The message is fixed."""


class ApiRefused(Exception):
    """The product's own API answered, but the connector cannot be run from
    the answer (a 4xx, a connector that is not ingestible). A retry would get
    the same answer. The message is fixed."""


@dataclass(frozen=True)
class Classified:
    """`error_class` is one of `ERROR_CLASSES`; `retried` whether the class
    is transient (a retry may succeed)."""

    error_class: str
    retried: bool


# Bounded walk: SQLAlchemy's `.orig`, `raise ... from`, implicit contexts and
# `dlt`'s `.exception` are chains of a few links; the bound only guards
# against a cycle.
_MAX_CAUSE_DEPTH = 8


def _chain(exc: BaseException) -> Iterator[BaseException]:
    """`exc` and what it wraps, outermost first, each once."""
    seen: set[int] = set()
    pending: list[BaseException | None] = [exc]
    for _ in range(_MAX_CAUSE_DEPTH):
        if not pending:
            return
        current = pending.pop(0)
        if current is None or id(current) in seen:
            continue
        seen.add(id(current))
        yield current
        inner = getattr(current, "exception", None)
        pending.extend(
            [
                getattr(current, "orig", None),
                inner if isinstance(inner, BaseException) else None,
                current.__cause__,
                current.__context__,
            ]
        )


def _root(exc: BaseException) -> str:
    return type(exc).__module__.split(".")[0]


def _first_arg(exc: BaseException) -> object:
    return exc.args[0] if exc.args else None


def _class_names(exc: BaseException) -> set[str]:
    """Names of `exc`'s class and its bases that live in the same package."""
    root = _root(exc)
    return {c.__name__ for c in type(exc).__mro__ if c.__module__.split(".")[0] == root}


# --- driver codes ------------------------------------------------------------

# SQLSTATE a driver reports for a refused login (class 28, "invalid
# authorization specification"); SQL Server's error 18456 carries it.
_SQLSTATE_LOGIN_REFUSED = "28000"
# PostgreSQL's `invalid_password`.
_PG_INVALID_PASSWORD = "28P01"
# The server's own fixed refusal wording, read only when libpq gave no code.
_PG_REFUSAL_WORDING = "password authentication failed"
# `pymysql.constants.ER.ACCESS_DENIED_ERROR` / Oracle `ORA-01017`.
_MYSQL_ACCESS_DENIED = 1045
_ORACLE_LOGON_DENIED = "ORA-01017"
# MongoDB's `AuthenticationFailed`.
_MONGO_AUTH_FAILED = 18

# MySQL / MariaDB: 2002 "can't connect (socket)", 2003 "can't connect", 2006
# "server has gone away", 2013 "lost connection", 2055 "lost connection".
_MYSQL_UNREACHABLE = frozenset({2002, 2003, 2006, 2013, 2055})
# 1040 too many connections, 1205 lock wait timeout, 1213 deadlock, 1053
# server shutdown in progress.
_MYSQL_BUSY = frozenset({1040, 1053, 1205, 1213})

# PostgreSQL: 57014 query_canceled (statement timeout), 55P03 lock_not_available,
# 40001 serialization_failure, 40P01 deadlock_detected, 53300 too_many_connections,
# 57P01-03 admin shutdown / crash shutdown / cannot connect now.
_PG_TIMEOUT = frozenset({"57014"})
_PG_BUSY = frozenset({"55P03", "40001", "40P01", "53300", "57P01", "57P02", "57P03"})

# Oracle: ORA-12170 connect timeout; ORA-12541/12514/12543/12545/12528 no
# listener / unknown service / host or port unreachable; DPY-6005 cannot
# connect, DPY-4011 connection closed; ORA-00054/00060 resource busy / deadlock,
# ORA-12519/12520/00018/00020 no handler / session or process limit.
_ORACLE_TIMEOUT = frozenset({"ORA-12170", "DPY-4024"})
_ORACLE_UNREACHABLE = frozenset(
    {"ORA-12541", "ORA-12514", "ORA-12543", "ORA-12545", "ORA-12528", "DPY-6005", "DPY-4011"}
)
_ORACLE_BUSY = frozenset({"ORA-00054", "ORA-00060", "ORA-12519", "ORA-12520", "ORA-00018", "ORA-00020"})

_ERRNO_UNREACHABLE = frozenset(
    # ECONNREFUSED, ECONNRESET, ECONNABORTED, EHOSTUNREACH, ENETUNREACH,
    # ENETDOWN, EHOSTDOWN, EPIPE: the Linux numbers the `errno` module names.
    {111, 104, 103, 113, 101, 100, 112, 32}
)
_ERRNO_TIMEOUT = frozenset({110})  # ETIMEDOUT


def _sqlstate(exc: BaseException) -> str | None:
    """The SQLSTATE a `psycopg2` error carries, from `pgcode` or `diag`."""
    code = getattr(exc, "pgcode", None)
    if isinstance(code, str):
        return code
    diag = getattr(exc, "diag", None)
    code = getattr(diag, "sqlstate", None)
    return code if isinstance(code, str) else None


def _is_refusal_element(exc: BaseException) -> bool:
    """Whether this one exception (not what it wraps) is a database or broker
    saying the credential is WRONG. See `is_authentication_refusal`."""
    root = _root(exc)
    first = _first_arg(exc)
    if root == "pymysql" and first == _MYSQL_ACCESS_DENIED:
        return True
    if root == "pyodbc" and first == _SQLSTATE_LOGIN_REFUSED:
        return True
    if root == "oracledb" and getattr(first, "full_code", None) == _ORACLE_LOGON_DENIED:
        return True
    if root == "psycopg2":
        code = _sqlstate(exc)
        if code is not None:
            return code in (_PG_INVALID_PASSWORD, _SQLSTATE_LOGIN_REFUSED)
        # libpq gives no SQLSTATE for a failure while connecting; the
        # server's fixed refusal wording is the only signal (`SRC-19`). Read
        # here and nowhere else; never logged or returned.
        return _PG_REFUSAL_WORDING in str(exc)
    if root == "kafka" and _class_names(exc) & {"AuthenticationFailedError", "SaslAuthenticationFailedError"}:
        return True
    # SRC-13: MongoDB's `AuthenticationFailed` (code 18), what a wrong user or
    # password answers. The same answer every time, so a load does not retry it either.
    if root == "pymongo" and getattr(exc, "code", None) == _MONGO_AUTH_FAILED:
        return True
    return root == "paramiko" and "AuthenticationException" in _class_names(exc)


def is_authentication_refusal(exc: BaseException) -> bool:
    """Whether `exc`, or anything it wraps, is the source REFUSING THE
    CREDENTIAL (a wrong user or password).

    That answer does not change between attempts, so retrying it only delays
    the failure alert (the G6 gate measured a 2m23s run for a 10-second
    failure under the old two-retry policy, PR #101 CI). Decided by the
    driver's exception TYPE and error CODE; PostgreSQL's wording fallback is
    explained in this module's docstring.

    - `pymysql` (mysql/mariadb): `args[0] == 1045`.
    - `pyodbc` (mssql): `args[0]` (the SQLSTATE) is `28000`.
    - `oracledb`: first argument's `full_code == "ORA-01017"`.
    - `psycopg2` (postgres): SQLSTATE `28P01` or `28000` when the error has
      one, else the fixed server wording.
    - `kafka` SASL and `paramiko` SSH authentication failures.
    - `pymongo`: `OperationFailure.code == 18` (`AuthenticationFailed`).
    """
    return any(_is_refusal_element(e) for e in _chain(exc))


# `ORA-01031`: insufficient privileges. MongoDB's `Unauthorized` error code.
_ORACLE_INSUFFICIENT_PRIVILEGES = "ORA-01031"
_MONGO_UNAUTHORIZED = 13
# Kafka's ACL refusals: the credential works, the ACL does not allow the read.
_KAFKA_NOT_AUTHORIZED = frozenset(
    {"TopicAuthorizationFailedError", "ClusterAuthorizationFailedError", "GroupAuthorizationFailedError"}
)


def _is_permission_element(exc: BaseException) -> bool:
    """Whether this one exception (not what it wraps) is a source saying the
    credential is VALID but may not do this. See `is_permission_refusal`."""
    root = _root(exc)
    if root == "oracledb":
        return getattr(_first_arg(exc), "full_code", None) == _ORACLE_INSUFFICIENT_PRIVILEGES
    if root == "pymongo":
        return getattr(exc, "code", None) == _MONGO_UNAUTHORIZED
    if root == "kafka":
        return bool(_class_names(exc) & _KAFKA_NOT_AUTHORIZED)
    # An SFTP server's "permission denied" arrives as `IOError(EACCES)`, which
    # Python builds as `PermissionError`.
    return isinstance(exc, (PermissionError, saas_session.SaasPermissionMissing))


def is_permission_refusal(exc: BaseException) -> bool:
    """Whether `exc`, or anything it wraps, is the source saying the credential
    is accepted but not ALLOWED to read what was asked (`SRC-13` decision D10:
    "no permission at the source"), as opposed to refusing the credential itself
    (`is_authentication_refusal`). Decided by the driver's exception TYPE and
    error CODE, never by message text; nothing of the message is read.

    - `oracledb`: first argument's `full_code == "ORA-01031"`.
    - `pymongo`: `OperationFailure.code == 13` (`Unauthorized`).
    - `kafka`: a topic, cluster or group authorisation failure.
    - SFTP: a `PermissionError`.
    """
    return any(_is_permission_element(e) for e in _chain(exc))


# --- classification ----------------------------------------------------------


def _http_status(exc: BaseException) -> int | None:
    status = getattr(getattr(exc, "response", None), "status_code", None)
    return status if isinstance(status, int) else None


def _by_http_status(status: int) -> str:
    if status in (401, 403):
        return SIGN_IN_REFUSED
    if status in (408, 504):
        return TIMEOUT
    if status == 429 or status >= 500:
        return SOURCE_BUSY
    return OTHER


def _classify_driver(exc: BaseException, root: str) -> str | None:
    """The class for a driver's own error, by its error code."""
    first = _first_arg(exc)
    if root == "pymysql" and isinstance(first, int):
        if first in _MYSQL_UNREACHABLE:
            return UNREACHABLE
        if first in _MYSQL_BUSY:
            return SOURCE_BUSY
    if root == "pyodbc" and isinstance(first, str):
        if first.startswith("08"):
            return UNREACHABLE
        if first.startswith("HYT"):
            return TIMEOUT
        if first in ("40001", "40002"):
            return SOURCE_BUSY
    if root == "oracledb":
        code = getattr(first, "full_code", None)
        if code in _ORACLE_TIMEOUT:
            return TIMEOUT
        if code in _ORACLE_UNREACHABLE:
            return UNREACHABLE
        if code in _ORACLE_BUSY:
            return SOURCE_BUSY
    if root == "psycopg2":
        code = _sqlstate(exc)
        if code in _PG_TIMEOUT:
            return TIMEOUT
        if code in _PG_BUSY:
            return SOURCE_BUSY
        # A connection-level failure (no SQLSTATE, or class 08): the server
        # was not there to answer.
        if "OperationalError" in _class_names(exc) and (code is None or code.startswith("08")):
            return UNREACHABLE
    names = _class_names(exc)
    if root == "kafka":
        if names & {"NoBrokersAvailable", "KafkaConnectionError"}:
            return UNREACHABLE
        if names & {"KafkaTimeoutError", "RequestTimedOutError"}:
            return TIMEOUT
        if names & {"BrokerNotAvailableError", "NodeNotReadyError", "LeaderNotAvailableError"}:
            return SOURCE_BUSY
    if root == "paramiko" and "NoValidConnectionsError" in names:
        return UNREACHABLE
    if root == "pymongo":
        if names & {"ExecutionTimeout", "NetworkTimeout"}:
            return TIMEOUT
        if names & {"ConnectionFailure"}:  # includes AutoReconnect, ServerSelectionTimeoutError
            return UNREACHABLE
    if root == "pyiceberg" and names & {"ServiceUnavailableError", "ServerError"}:
        return SOURCE_BUSY
    return None


def _classify_one(exc: BaseException) -> str | None:
    """The class of this one exception, or `None` when it says nothing."""
    # The product's own types first: they say exactly what happened. The
    # specific ones before `ValueError` (`DedupRefused` and
    # `UnsupportedLoadMode` are `ValueError`s).
    if isinstance(exc, (schema_observer.ObservationUnreachable, ApiUnreachable)):
        return API_UNREACHABLE
    if isinstance(exc, (schema_observer.ObservationRefused, ApiRefused)):
        return SETTING_REJECTED
    if isinstance(exc, ssrf_guard.SsrfBlocked):
        return ADDRESS_REFUSED
    # SRC-12 part C: the provider refused the refresh token, or nobody has
    # signed in. The same word as a refused password: not retried, and the
    # connector page offers Sign in.
    if isinstance(exc, rest_adapter.OAuthSignInRefused):
        return SIGN_IN_REFUSED
    # SRC-3 (business-app vendors). The run-report classes are a CLOSED list the
    # API and a database CHECK enforce, and `permission_missing` and
    # `rate_limited` are not in it (plan T1.9 assumed they could be added; that
    # needs a migration, see the slice 1 handoff). Until the planner decides:
    # a refused credential and a missing permission are both "sign_in_refused"
    # (not retried; both need a person at the vendor), and a rate limit that
    # outlasted the bounded waits is "source_busy" (retried by the run's policy).
    if isinstance(exc, (saas_session.SaasSignInRefused, saas_session.SaasPermissionMissing)):
        return SIGN_IN_REFUSED
    if isinstance(exc, saas_session.SaasRateLimited):
        return SOURCE_BUSY
    if isinstance(
        exc,
        (
            UnsupportedColumnType,
            UnsupportedLoadMode,
            dedup_adapter.DedupRefused,
            UnsupportedKafkaAuth,
            saas_vendors.UnsupportedVendor,
        ),
    ):
        return UNSUPPORTED
    if isinstance(
        exc,
        (
            UnknownAdapter,
            SecretRefRejected,
            oracle_adapter.OracleTlsConfigError,
            ssrf_guard_sftp_module.HostKeyMismatch,
            ssrf_guard_mongo_module.MongoConfigRejected,
            saas_adapter.SaasSetupRefused,
        ),
    ):
        return SETTING_REJECTED
    if _is_refusal_element(exc):
        return SIGN_IN_REFUSED
    root = _root(exc)
    driver = _classify_driver(exc, root)
    if driver is not None:
        return driver
    status = _http_status(exc)
    if status is not None and root == "requests":
        return _by_http_status(status)
    names = _class_names(exc)
    if root == "requests" and "Timeout" in names:
        return TIMEOUT
    if root == "requests" and "ConnectionError" in names:
        return UNREACHABLE
    if root == "sqlalchemy" and "TimeoutError" in names:
        return TIMEOUT
    if root == "sqlalchemy" and getattr(exc, "connection_invalidated", False):
        return UNREACHABLE
    if isinstance(exc, TimeoutError):  # socket.timeout is this class since 3.10
        return TIMEOUT
    if isinstance(exc, (ConnectionError, socket.gaierror, socket.herror)):
        return UNREACHABLE
    if isinstance(exc, OSError):
        if exc.errno in _ERRNO_TIMEOUT:
            return TIMEOUT
        if exc.errno in _ERRNO_UNREACHABLE:
            return UNREACHABLE
        return None
    if isinstance(exc, ValueError):
        # Every adapter raises `ValueError` for rejected adapter
        # configuration (an unknown driver, auth type, pagination type or
        # file format; a hostname with a control character). One site is
        # data-shaped (`files.py`, the object-size cap) and is classed with
        # the rest: it fails the same way on every attempt for the same
        # object, so a retry would not help either.
        return SETTING_REJECTED
    return None


def classify(exc: BaseException) -> Classified:
    """Name `exc` with one of `ERROR_CLASSES` and say whether it is retried.

    The outermost exception that says something decides; a credential refusal
    anywhere in the chain wins over a transient-looking wrapper (a driver
    wrapped by SQLAlchemy or `dlt`). Nothing of the message is returned.
    """
    if any(_is_refusal_element(e) for e in _chain(exc)):
        return Classified(SIGN_IN_REFUSED, retried=False)
    for element in _chain(exc):
        found = _classify_one(element)
        if found is not None:
            return Classified(found, retried=found in RETRIED_CLASSES)
    return Classified(OTHER, retried=False)


# --- the delays ----------------------------------------------------------------

DEFAULT_RETRY_DELAYS_SECONDS = (60, 300, 900)
# Up to three retries is the spec (SRC-11 D1); the API accepts a wait up to a day.
MAX_RETRIES = 3
MAX_WAIT_SECONDS = 86_400
RETRY_DELAYS_ENV = "INGEST_RETRY_DELAYS_SECONDS"


def parse_delays(raw: str | None) -> tuple[int, ...] | None:
    """`"60,300,900"` as seconds, or `None` when `raw` cannot be used: not a
    comma list of whole numbers, none negative, none above a day, and one to
    three of them (the spec allows up to three retries)."""
    if raw is None or not raw.strip():
        return None
    try:
        delays = tuple(int(part.strip()) for part in raw.split(","))
    except ValueError:
        return None
    if not 1 <= len(delays) <= MAX_RETRIES or any(d < 0 or d > MAX_WAIT_SECONDS for d in delays):
        return None
    return delays


@dataclass(frozen=True)
class RetrySettings:
    """The wait before each retry of a failing table, in seconds; its length
    is the number of retries."""

    delays: tuple[int, ...] = DEFAULT_RETRY_DELAYS_SECONDS

    @classmethod
    def from_env(cls) -> "RetrySettings":
        raw = os.environ.get(RETRY_DELAYS_ENV)
        if raw is None or not raw.strip():
            return cls()
        parsed = parse_delays(raw)
        if parsed is None:
            # The value is the operator's own setting, but a malformed one is
            # not worth echoing into a log line the whole team reads.
            logger.warning(
                "%s cannot be used (expected one to %d whole numbers of seconds, 0 to %d, "
                "separated by commas); using the default %s",
                RETRY_DELAYS_ENV,
                MAX_RETRIES,
                MAX_WAIT_SECONDS,
                ",".join(str(d) for d in DEFAULT_RETRY_DELAYS_SECONDS),
            )
            return cls()
        return cls(delays=parsed)

    @property
    def max_retries(self) -> int:
        return len(self.delays)

    def delay_for(self, retry_number: int) -> int | None:
        """Seconds to wait before retry number `retry_number + 1` (0 for the
        first retry), or `None` when the retries are used up."""
        return self.delays[retry_number] if 0 <= retry_number < len(self.delays) else None
