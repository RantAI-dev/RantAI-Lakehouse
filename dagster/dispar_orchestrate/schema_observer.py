"""dagster/dispar_orchestrate/schema_observer.py -- the orchestrator's side
of `SRC-8` (schema changes at the source): reflect what a source table
looks like, tell `lakehouse-api`, and obey its answer.

# Who decides

The API decides, the orchestrator observes (plan
`docs/superpowers/plans/2026-10-09-src-8-source-schema-changes.md`, section
2). `post_observation` sends the table's columns and primary key to
`POST /api/connectors/{id}/schema-observations` and returns the API's
`Decision`: `load` (every column, or only the listed ones) or `wait`. The
comparison, the connector's policy, the pause, the notice and the alert all
live in `lakehouse-api`; nothing here compares columns.

# Two phases (feature page decision D4)

- `before_load` -- batch SQL sources, where the table's columns can be read
  before any row moves. The answer is binding: `wait` means the table is not
  loaded, a column list means only those columns are loaded.
- `after_load` -- every other source (files, REST, MongoDB, Kafka, SFTP): the
  columns are what the load just produced. The data is already in Bronze,
  so the answer is advisory and a failure to post it never fails the run
  (see `ingest_factory._observe_after_load`).

# Reflection without a second connection

`ReflectionCollector.callback` is a `table_adapter_callback` for
`dlt.sources.sql_database`. `dlt` calls it with the SQLAlchemy `Table` it
reflected while it BUILDS the source (measured with `dlt` 1.30.0 in
`test_schema_drift_sql_source.py`, which also pins that no row has been read
yet), so the columns come from the one reflection `dlt` does anyway, over
the connection each adapter has already guarded (`adapters/sql.py`,
`adapters/oracle.py`, `dlt_pipeline.run_bronze_ingest`). This module opens no
connection of its own: that is what keeps reflection inside the SSRF guard.
`keep_only` is the same hook used to hold columns back: a column removed from
the reflected `Table` is neither selected nor given a type hint.

# Fail closed (AGENTS.md principle 3)

If the API cannot be asked, the table is NOT loaded unchecked.
`ObservationUnreachable` (no connection, timeout, a 5xx) is transient and
raised so the op's `DEFAULT_RETRY_POLICY` retries it; `ObservationRefused`
(a 4xx, an unset token, an answer that is not a decision) would be answered
the same way again, so `ingest_factory.ingest_source_object` turns it into
`Failure(allow_retries=False)`. Neither message holds the token, the URL's
query or an upstream response body (principle 4).
"""

from __future__ import annotations

from collections.abc import Callable, Sequence
from dataclasses import dataclass, field
from typing import Any

import dlt
import requests

PHASES = ("before_load", "after_load")

# Seconds. The same bound `ingest_factory._fetch_one_connector` uses for the
# API calls of the same run.
_TIMEOUT = 10


class ObservationUnreachable(Exception):
    """The API could not be asked, or answered with a server error: worth a
    retry. The message names the kind of failure only."""


class ObservationRefused(Exception):
    """The API answered, but not with a usable decision (a 4xx, an unset
    service token, a body that is not `load`/`wait`): a retry would get the
    same answer. The message names the status or the problem only."""


class ReflectionMissing(ObservationRefused):
    """`dlt` built the source without handing the callback the table, so
    there is nothing to observe. Raised instead of loading unchecked."""


def loaded_column_name(name: str) -> str | None:
    """The name `dlt` gives a source column in the Bronze table (SRC-8,
    task 11): its own naming convention for a schema, `OrderDate` ->
    `order_date`. The Schema tab lists the Bronze table's columns, so the
    API can only mark a column inactive by THIS name, and nothing outside
    `dlt` reproduces the convention.

    A default `dlt.Schema` is what the pipeline's source schema starts as
    (convention from `dlt`'s configuration, `snake_case` unless
    `SCHEMA__NAMING` says otherwise), so it is asked rather than a copy of
    the rules kept here; `test_schema_loaded_name.py` loads a table through
    the sink's kind of destination and checks the names match. The one
    difference not covered: the schema is not given the destination's
    maximum identifier length, so a name past `dlt`'s default limit of that
    convention could differ; the API accepts names of at most 256
    characters.

    `None` when `dlt` cannot normalise the name: the API then marks nothing
    for that column (never a guess).
    """
    try:
        return dlt.Schema("bronze").naming.normalize_identifier(name)
    except Exception:  # noqa: BLE001 -- a name `dlt` refuses just gets no mark
        return None


@dataclass(frozen=True)
class ReflectedColumn:
    """One column as SQLAlchemy reflected it. `type_name` is the database's
    own spelling (`str(column.type)`); lower-casing and comparing is the
    API's job (`lakehouse-store::schema_diff`).

    `loaded_name` is the name the Bronze table will give the column
    (`loaded_column_name`); after a load it is the name itself, already the
    loaded one. `None` is left out of the wire body (`SRC-8` task 11)."""

    name: str
    type_name: str
    nullable: bool
    loaded_name: str | None = None

    def to_wire(self) -> dict[str, Any]:
        wire: dict[str, Any] = {"name": self.name, "typeName": self.type_name, "nullable": self.nullable}
        if self.loaded_name is not None:
            wire["loadedName"] = self.loaded_name
        return wire


@dataclass(frozen=True)
class ReflectedTable:
    columns: tuple[ReflectedColumn, ...]
    primary_key: tuple[str, ...]


@dataclass(frozen=True)
class Decision:
    """The API's answer. `columns` is `None` for "all of them"; `changes` is
    the API's list of changes as it sent it (names and kinds, no source
    data)."""

    action: str
    columns: list[str] | None
    changes: list[dict[str, Any]]


def _type_name(column: Any) -> str:
    try:
        return str(column.type)
    except Exception:  # noqa: BLE001 -- a dialect type that cannot compile still has a class name
        return type(column.type).__name__


@dataclass
class ReflectionCollector:
    """A `table_adapter_callback` that records each reflected table and
    changes nothing about it."""

    tables: list[ReflectedTable] = field(default_factory=list)

    def callback(self, table: Any) -> None:
        self.tables.append(
            ReflectedTable(
                columns=tuple(
                    # `nullable` can be None once `dlt` strips nullability
                    # hints; only an explicit NOT NULL is "not nullable".
                    ReflectedColumn(
                        name=c.name,
                        type_name=_type_name(c),
                        nullable=c.nullable is not False,
                        loaded_name=loaded_column_name(c.name),
                    )
                    for c in table.columns
                ),
                primary_key=tuple(c.name for c in table.primary_key.columns),
            )
        )

    def only(self) -> ReflectedTable:
        """The one table this source was built for.

        # Errors

        Raises `ReflectionMissing` unless exactly one table was reflected.
        """
        if len(self.tables) != 1:
            raise ReflectionMissing(
                f"expected the source build to reflect exactly one table, it reflected {len(self.tables)}"
            )
        return self.tables[0]


def keep_only(columns: Sequence[str]) -> Callable[[Any], None]:
    """A `table_adapter_callback` that removes every column not in `columns`
    from the reflected `Table`, so `dlt` neither SELECTs nor creates it
    (measured: `test_a_table_adapter_callback_that_removes_a_column_...`)."""
    keep = set(columns)

    def adapter(table: Any) -> None:
        for column in list(table._columns):
            if column.name not in keep:
                table._columns.remove(column)

    return adapter


@dataclass(frozen=True)
class ObserverConfig:
    """`api_url`/`service_token`: the very values `ingest_factory`
    reads for `GET /api/connectors/ingestible` (`IngestFactoryConfig`), copied
    from it so the environment is read in one place."""

    api_url: str
    service_token: str

    @classmethod
    def from_env(cls) -> "ObserverConfig":
        # Function-local import: `ingest_factory` imports this module, so a
        # module-level import would be a cycle. `IngestFactoryConfig` is the
        # single reader of `LAKEHOUSE_API_URL` / `INGEST_SERVICE_TOKEN`.
        from dispar_orchestrate.ingest_factory import IngestFactoryConfig

        base = IngestFactoryConfig.from_env()
        return cls(api_url=base.api_url, service_token=base.service_token)

    def headers(self) -> dict[str, str]:
        # `ingest_factory._headers` is the one place the bearer header is
        # built (the ingestible GET sends the same one).
        from dispar_orchestrate.ingest_factory import _headers

        return _headers(self)  # type: ignore[arg-type] -- reads `.service_token` only


def _parse_decision(body: Any) -> Decision:
    if not isinstance(body, dict) or body.get("action") not in ("load", "wait"):
        raise ObservationRefused("the schema-observation answer was not a load/wait decision")
    columns = body.get("columns")
    if columns is not None and not (isinstance(columns, list) and all(isinstance(c, str) for c in columns)):
        raise ObservationRefused("the schema-observation answer carried a malformed column list")
    changes = body.get("changes", [])
    if not isinstance(changes, list):
        raise ObservationRefused("the schema-observation answer carried a malformed change list")
    return Decision(action=body["action"], columns=columns, changes=changes)


def _post_json(cfg: ObserverConfig, path: str, payload: dict[str, Any], what: str) -> Any:
    """POST `payload` to the API with the ingest service token and return the
    parsed body. `what` names the call in the (fixed) error messages.

    # Errors

    As [`post_observation`].
    """
    if not cfg.service_token:
        raise ObservationRefused(f"INGEST_SERVICE_TOKEN is unset, so {what} cannot be sent")
    try:
        resp = requests.post(f"{cfg.api_url}{path}", json=payload, headers=cfg.headers(), timeout=_TIMEOUT)
        resp.raise_for_status()
        return resp.json()
    except requests.HTTPError as exc:
        status = getattr(exc.response, "status_code", None)
        if isinstance(status, int) and 400 <= status < 500:
            raise ObservationRefused(f"the API refused the {what} (HTTP {status})") from None
        raise ObservationUnreachable(f"the API failed on the {what} (HTTP {status})") from None
    except ValueError:
        # `resp.json()` on a body that is not JSON. Listed BEFORE
        # `RequestException`: `requests.JSONDecodeError` is both.
        raise ObservationRefused(f"the answer to the {what} was not JSON") from None
    except requests.RequestException as exc:
        raise ObservationUnreachable(f"the API could not be reached ({type(exc).__name__})") from None


def post_observation(
    cfg: ObserverConfig,
    connector_id: str,
    object_name: str,
    columns: Sequence[ReflectedColumn],
    primary_key: Sequence[str],
    phase: str,
    run_id: str | None,
) -> Decision:
    """Tell the API what `object_name` looks like and return its decision.

    # Errors

    Raises `ObservationRefused` if the service token is unset (the route is
    for the ingest service identity only; sending nothing would be a 401
    anyway), on a 4xx, or when the answer is not a decision; raises
    `ObservationUnreachable` when the API cannot be reached or answers 5xx.
    """
    if phase not in PHASES:
        raise ValueError(f"phase must be one of {', '.join(PHASES)}, got {phase!r}")
    payload: dict[str, Any] = {
        "object": object_name,
        "columns": [c.to_wire() for c in columns],
        "primaryKey": list(primary_key),
        "phase": phase,
    }
    if run_id:
        payload["runId"] = run_id
    body = _post_json(cfg, f"/api/connectors/{connector_id}/schema-observations", payload, "schema observation")
    return _parse_decision(body)


# Most table names one request carries: the API refuses more (`MAX_TABLES` in
# `routes/schema_changes.rs`). Keep both in step.
MAX_TABLES_PER_REQUEST = 2000


@dataclass(frozen=True)
class NewTablesAnswer:
    """`added` are the tables now selected by the connector; `not_added` are
    `{"table", "reason"}` objects (the API's fixed texts)."""

    added: list[str]
    not_added: list[dict[str, Any]]


def post_new_tables(
    cfg: ObserverConfig, connector_id: str, tables: Sequence[str], run_id: str | None
) -> NewTablesAnswer:
    """Tell the API about tables that appeared in a schema the connector
    already loads from (`SRC-8` decision D2, policy `apply_all`) and return
    which it added to the connector.

    # Errors

    As [`post_observation`]; also `ValueError` for more than
    `MAX_TABLES_PER_REQUEST` names (the caller sends them in chunks).
    """
    if len(tables) > MAX_TABLES_PER_REQUEST:
        raise ValueError(f"at most {MAX_TABLES_PER_REQUEST} tables per request, got {len(tables)}")
    payload: dict[str, Any] = {"tables": list(tables)}
    if run_id:
        payload["runId"] = run_id
    body = _post_json(cfg, f"/api/connectors/{connector_id}/schema-observations/tables", payload, "new-table report")
    added, not_added = (body.get("added"), body.get("notAdded")) if isinstance(body, dict) else (None, None)
    if not (isinstance(added, list) and all(isinstance(t, str) for t in added)) or not (
        isinstance(not_added, list) and all(isinstance(n, dict) for n in not_added)
    ):
        raise ObservationRefused("the answer to the new-table report was not an added/notAdded list")
    return NewTablesAnswer(added=added, not_added=not_added)
