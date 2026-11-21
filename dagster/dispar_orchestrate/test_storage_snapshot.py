"""Unit tests for `dagster/dispar_orchestrate/storage_snapshot.py` (OPS-8 part C).

The job measures the size of every layer once a day: the Iceberg namespaces
(`bronze`, `gold`) at their current snapshot's `total-files-size`, and the
ClickHouse databases (`silver`, `serving`) at their own `system.parts`. No
network and no Dagster instance here: the catalog and `ClickHouse` are faked
at their own boundaries — `requests.post`/`requests.get` for the catalog's
token, its warehouse prefix and its table metadata documents,
`maintenance._ch_query` for the catalog database's creation and the catalog's
own table list, and `ClickHouseTarget.query_json` for the `system.parts`
totals. One of the two ClickHouse fakes knows the columns ClickHouse 26.8
has (`system.tables.engine`, and NO `type`) and refuses a statement that
names one the system table does not have, the way the live stack refused
`type = 'Table'`: a fake that answers every statement with a canned row
cannot catch a statement that names a column the server does not have.

The OBJECT STORE is faked at its own boundary the same way: a filesystem
that holds the bytes under each key and answers the recursive listing
`capacity_snapshot.list_stored_objects` reads. A key whose listing fails
fails the way a refused listing fails, with a backend-specific error type,
because the module catches EVERY exception there (the object store raises
types this package does not name).

Run with:
`~/.venvs/rantai-lakehouse/bin/python -m pytest dagster/dispar_orchestrate/test_storage_snapshot.py -q`
"""

from __future__ import annotations

import importlib
import os
import re
from unittest import mock

import pytest
import requests
from dagster import Failure

# The ingest job's own run limit must not be set for these tests: the point
# of the run-limit tests below is that the two jobs read TWO settings. The
# ingest job's tag is read when `ingest_factory` is imported, so the
# variable is cleared BEFORE that import happens.
os.environ.pop("INGEST_RUN_MAX_RUNTIME_SECONDS", None)

from dispar_orchestrate import (
    bronze_catalog,
    capacity_snapshot,
    ingest_factory,
    maintenance,
    storage_snapshot,
)

# The tag the orchestrator's run monitoring reads (`SRC-39`).
TAG = "dagster/max_runtime"

# The table's metadata document, in the two shapes a writer may leave it:
# the summary on the current snapshot, and the spec's own copy of it.
# The bucket every table of these tests lives in. The "stored on disk"
# number is the bytes under a table's own location, and the location names
# the bucket: a table whose location names another bucket is not a table of
# the bucket this job measures.
BUCKET = "lakehouse-warehouse"


def _location(table: str) -> str:
    """The location one table's files live under."""
    return f"s3://{BUCKET}/warehouse/bronze/{table}"


def _metadata(size: str, table: str = "orders", location: str | None = None) -> dict:
    """A table metadata document whose current snapshot holds `size` bytes.

    The document carries the table's own LOCATION as well: the "stored on
    disk" number is measured from that location, and a document without
    one cannot be measured at all."""
    return {
        "metadata": {
            "current-snapshot-id": 7,
            "snapshots": [{"snapshot-id": 7, "summary": {"total-files-size": size}}],
            "snapshot-summary": {"total-files-size": size},
            "location": location if location is not None else _location(table),
        }
    }


def _cfg(**overrides) -> maintenance.MaintenanceConfig:
    defaults = dict(
        ch=bronze_catalog.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
        lakekeeper_catalog_uri="http://lakekeeper.invalid/catalog",
        lakekeeper_warehouse="default",
        rustfs_endpoint="http://rustfs.invalid:9000",
        ch_oauth_client_id="clickhouse-reader",
        ch_oauth_server_uri="http://oidc-mock.invalid/token",
    )
    defaults.update(overrides)
    return maintenance.MaintenanceConfig(**defaults)


def _fake_catalog(monkeypatch, metadata: dict[str, dict]) -> None:
    """Fakes the Iceberg REST catalog: its token, its warehouse prefix and
    one metadata document per table. A table with no document fails the read
    the way a table whose metadata cannot be read fails it."""

    token = mock.Mock()
    token.raise_for_status = mock.Mock()
    token.json.return_value = {"access_token": "fake-token"}
    config = mock.Mock()
    config.raise_for_status = mock.Mock()
    config.json.return_value = {"defaults": {"prefix": "wh-prefix"}}

    def _get(url, **_kwargs):
        if url.endswith("/v1/config"):
            return config
        document = metadata.get(url.rsplit("/", 1)[-1])
        if document is None:
            raise requests.HTTPError("404 Client Error: Not Found")
        answer = mock.Mock()
        answer.raise_for_status = mock.Mock()
        answer.json.return_value = document
        return answer

    monkeypatch.setattr(requests, "post", mock.Mock(return_value=token))
    monkeypatch.setattr(requests, "get", _get)


# The columns ClickHouse 26.8 has in the two system tables this job reads.
# `DESCRIBE system.tables` on 26.8.36 lists `engine` (the engine name) and
# NO `type` column, and a statement that names a column the system table
# does not have is refused with UNKNOWN_IDENTIFIER — the way the live stack
# refused `type = 'Table'` and failed every run of this job.
_SYSTEM_TABLE_COLUMNS = {
    "system.tables": frozenset(
        {
            "database",
            "name",
            "uuid",
            "engine",
            "is_temporary",
            "dependencies_database",
            "engine_full",
            "total_rows",
            "total_bytes",
            "total_bytes_on_disk",
        }
    ),
    "system.parts": frozenset(
        {
            "database",
            "table",
            "uuid",
            "name",
            "part_type",
            "active",
            "bytes_on_disk",
            "rows",
            "modification_time",
        }
    ),
}


def _fake_clickhouse_26_8(
    monkeypatch, tables: dict[str, int], bytes_: dict[str, int]
) -> None:
    """A ClickHouse 26.8 that knows its own columns: a statement naming a
    column the system table does not have is refused with
    UNKNOWN_IDENTIFIER, exactly as the live stack refused it."""

    def _query_json(_target, statement):
        for name, columns in _SYSTEM_TABLE_COLUMNS.items():
            if name not in statement:
                continue
            for column in re.findall(
                r"([A-Za-z_][A-Za-z0-9_]*)\s*(?:=|<>|!=|NOT\s+LIKE|LIKE)\s*'", statement
            ):
                if column not in columns:
                    raise requests.HTTPError(
                        "Code: 47. DB::Exception: Unknown expression or function "
                        f"identifier `{column}` in scope (UNKNOWN_IDENTIFIER)"
                    )
            layer = re.search(r"database\s*=\s*'([^']*)'", statement)
            name_of_layer = layer.group(1) if layer else ""
            if name == "system.tables":
                return [{"table_count": str(tables.get(name_of_layer, 0))}]
            return [{"total_bytes": str(bytes_.get(name_of_layer, 0))}]
        raise AssertionError(f"unexpected query: {statement}")

    monkeypatch.setattr(bronze_catalog.ClickHouseTarget, "query_json", _query_json)


def _fake_clickhouse(
    monkeypatch, catalog_tables: str, parts: dict[str, dict]
) -> None:
    """Fakes the three `ClickHouse` reads: the creation of the catalog
    database, the catalog's own table list, and the `system.parts` totals
    of one layer."""

    def _query(_cfg, sql):
        if "SHOW TABLES" in sql:
            return catalog_tables
        if "CREATE DATABASE" in sql:
            return ""
        raise AssertionError(f"unexpected query: {sql}")

    def _query_json(_target, statement):
        for layer, row in parts.items():
            if f"'{layer}'" in statement:
                return [row]
        raise AssertionError(f"unexpected query: {statement}")

    monkeypatch.setattr(maintenance, "_ch_query", _query)
    monkeypatch.setattr(bronze_catalog.ClickHouseTarget, "query_json", _query_json)


class _FakeFilesystem:
    """The object store, faked the way `s3fs` reads it.

    The FIRST segment of a path is the BUCKET, exactly as `s3fs` reads
    it: `find("warehouse/bronze/orders")` lists the bucket named
    `warehouse`, not the bucket named `lakehouse-warehouse`. The fake
    therefore keeps its objects under the full `<bucket>/<key>` path, and
    a path whose first segment names a bucket it does not hold FAILS the
    listing, the way `s3fs` fails on a bucket that does not exist.

    `buckets` names the buckets this store holds. A second one is the
    DECOY: a bucket whose name is the table's first folder, so a listing
    that lost the bucket prefix answers for THAT bucket instead of
    failing -- the case a fake that holds one bucket cannot see.

    `unlistable` names the paths whose listing fails, the way a refused
    listing fails: with a backend-specific error type, because the module
    catches every exception there (the object store raises types this
    package does not name).

    `find` is the recursive listing `capacity_snapshot.list_stored_objects`
    reads and `ls` is the cheap root listing the probe does -- the two
    calls the object store makes, and no more."""

    def __init__(
        self,
        objects: dict[str, int],
        unlistable: tuple[str, ...] = (),
        buckets: tuple[str, ...] = (BUCKET,),
    ) -> None:
        self.objects = objects
        self.unlistable = unlistable
        self.buckets = set(buckets)
        # The calls this store received: the root listing (the probe) and
        # the recursive listing of one table's location.
        self.listed: list[str] = []

    def ls(self, path, **_kwargs):
        self.listed.append(path)
        if path in self.unlistable:
            raise OSError(f"Access Denied: {path}")
        return [{"name": path, "type": "directory"}]

    def find(self, path, detail=True):
        assert detail is True
        self.listed.append(path)
        if path in self.unlistable:
            raise OSError(f"Access Denied: {path}")
        bucket = path.partition("/")[0]
        if bucket not in self.buckets:
            raise FileNotFoundError(f"No such bucket: {bucket}")
        prefix = f"{path}/"
        return {
            key: {"size": size, "type": "file"}
            for key, size in self.objects.items()
            if key.startswith(prefix)
        }


def _store(
    objects: dict[str, int] | None = None,
    unlistable: tuple[str, ...] = (),
    buckets: tuple[str, ...] = (BUCKET,),
    bucket: str = BUCKET,
) -> storage_snapshot.ObjectStore:
    """The object store this job reads, faked: the bytes under each
    `<bucket>/<key>` path, and the paths whose listing fails."""
    return storage_snapshot.ObjectStore(
        _FakeFilesystem(objects or {}, unlistable, buckets), bucket
    )


def test_the_size_of_a_layer_is_the_sum_of_the_tables_of_that_layer(monkeypatch):
    """The catalog holds three namespaces; each layer's number is the sum of
    the tables of THAT namespace alone, and a namespace this deployment does
    not have as a layer is not measured at all."""
    _fake_catalog(
        monkeypatch,
        {
            "orders": _metadata("100", table="orders"),
            "customers": _metadata("250", table="customers"),
            "mart_wisman": _metadata("7", table="mart_wisman"),
        },
    )
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\nbronze.customers\ngold.mart_wisman\nscratch.tmp\n",
        parts={
            "silver": {"table_count": "2", "total_bytes": "4096"},
            "serving": {"table_count": "3", "total_bytes": "8192"},
        },
    )

    rows = storage_snapshot.measure_layers(_cfg(), _store())
    by_layer = {row.layer: row for row in rows}
    assert set(by_layer) == {"bronze", "gold", "silver", "serving"}
    assert by_layer["bronze"].total_bytes == 350
    assert by_layer["bronze"].table_count == 2
    assert by_layer["gold"].total_bytes == 7
    assert by_layer["gold"].table_count == 1
    assert by_layer["silver"].total_bytes == 4096
    assert by_layer["silver"].table_count == 2
    assert by_layer["serving"].total_bytes == 8192
    assert by_layer["serving"].table_count == 3


def test_a_table_whose_metadata_cannot_be_read_is_skipped_and_never_counted_as_zero_bytes(
    monkeypatch,
):
    """A table whose metadata cannot be read is counted as skipped, with the
    reason, and its bytes are left out of the layer's total: a layer that
    could not be measured whole must not look measured and empty."""
    _fake_catalog(monkeypatch, {"orders": _metadata("100")})
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\nbronze.broken\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )

    rows = storage_snapshot.measure_layers(_cfg(), _store())
    bronze = next(row for row in rows if row.layer == "bronze")
    assert bronze.total_bytes == 100
    assert bronze.table_count == 2
    assert bronze.tables_skipped == 1
    assert bronze.skipped_reason != ""


def test_the_catalog_database_is_created_before_the_catalog_is_listed(monkeypatch):
    """The database the table list is read from does not exist on a fresh
    stack: it is the bronze-maintenance job that creates it, and that job
    runs two hours AFTER this one. The job therefore creates it itself,
    before it lists -- otherwise `SHOW TABLES FROM icecat_maintenance`
    answers `UNKNOWN_DATABASE`, no row is ever written, and the usage
    page's storage section answers 503."""
    statements = []

    def _query(_cfg, sql):
        statements.append(sql)
        return ""

    monkeypatch.setattr(maintenance, "_ch_query", _query)
    monkeypatch.setattr(
        bronze_catalog.ClickHouseTarget, "query_json", lambda _target, _statement: []
    )

    storage_snapshot.measure_layers(_cfg(), _store())

    created = [sql for sql in statements if "CREATE DATABASE" in sql]
    listed = [sql for sql in statements if "SHOW TABLES" in sql]
    assert created, f"the catalog database was never created: {statements}"
    assert listed, f"the catalog was never listed: {statements}"
    assert statements.index(created[0]) < statements.index(listed[0]), statements
    assert "icecat_maintenance" in created[0]
    assert "icecat_maintenance" in listed[0]


def test_a_missing_catalog_token_fails_the_run_and_is_not_a_skip(monkeypatch):
    """No `CH_OAUTH_CLIENT_ID` means no catalog token, and no table of any
    Iceberg layer can be read. That is a configuration failure -- a
    `dagster.Failure` naming the variable to set -- not a run that reports
    "every table skipped, zero bytes" (`docs/CODE-STANDARD.md`: a
    configuration/auth failure is a `Failure`, only a degraded but
    continuable failure is a skip)."""
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )

    with pytest.raises(Failure) as err:
        storage_snapshot.measure_layers(_cfg(ch_oauth_client_id=""), _store())

    assert "CH_OAUTH_CLIENT_ID" in str(err.value)


def test_a_catalog_that_refuses_the_token_fails_the_run(monkeypatch):
    """The token endpoint refuses (a 401, a 500): the run fails with the
    two variables to check, and the upstream text stays out of the
    message."""
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )
    token = mock.Mock()
    token.raise_for_status = mock.Mock(
        side_effect=requests.HTTPError("401 Client Error: Unauthorized")
    )
    token.json.return_value = {"access_token": "fake-token"}
    monkeypatch.setattr(requests, "post", mock.Mock(return_value=token))

    with pytest.raises(Failure) as err:
        storage_snapshot.measure_layers(_cfg(), _store())

    message = str(err.value)
    assert "CH_OAUTH_SERVER_URI" in message
    assert "CH_OAUTH_CLIENT_ID" in message
    assert "401" not in message, f"the upstream text reached the message: {message}"


def test_a_catalog_without_a_warehouse_prefix_fails_the_run(monkeypatch):
    """The catalog answers `/v1/config` without a warehouse prefix: its
    tables cannot be addressed, so the run fails naming the two variables
    that decide it."""
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )
    token = mock.Mock()
    token.raise_for_status = mock.Mock()
    token.json.return_value = {"access_token": "fake-token"}
    config = mock.Mock()
    config.raise_for_status = mock.Mock()
    config.json.return_value = {"defaults": {}}
    monkeypatch.setattr(requests, "post", mock.Mock(return_value=token))
    monkeypatch.setattr(requests, "get", mock.Mock(return_value=config))

    with pytest.raises(Failure) as err:
        storage_snapshot.measure_layers(_cfg(), _store())

    message = str(err.value)
    assert "LAKEKEEPER_WAREHOUSE" in message
    assert "LAKEKEEPER_CATALOG_URI" in message


def test_the_measurement_reads_the_catalog_through_the_public_helpers(monkeypatch):
    """`storage_snapshot` reaches for the PUBLIC names of the modules it
    reads -- `maintenance.mint_lakekeeper_read_token` and
    `maintenance.resolve_catalog_prefix`. A private name of another module
    is a name that may change under this one (`AGENTS.md` rule 4: use the
    helper that already exists)."""
    used = []

    monkeypatch.setattr(
        maintenance,
        "mint_lakekeeper_read_token",
        lambda cfg: used.append("token") or "fake-token",
    )
    monkeypatch.setattr(
        maintenance,
        "resolve_catalog_prefix",
        lambda cfg, token: used.append("prefix") or "wh-prefix",
    )
    monkeypatch.setattr(maintenance, "_ch_query", lambda _cfg, sql: "bronze.orders\n")
    monkeypatch.setattr(
        bronze_catalog.ClickHouseTarget, "query_json", lambda _target, _statement: []
    )
    document = mock.Mock()
    document.raise_for_status = mock.Mock()
    document.json.return_value = _metadata("100")
    monkeypatch.setattr(requests, "get", mock.Mock(return_value=document))

    rows = storage_snapshot.measure_layers(_cfg(), _store())

    assert used == ["token", "prefix"], used
    bronze = next(row for row in rows if row.layer == "bronze")
    assert bronze.total_bytes == 100


def test_the_clickhouse_layers_are_read_through_the_public_target_method(monkeypatch):
    """The ClickHouse layers are read through `ClickHouseTarget.query_json`,
    the public method `bronze_catalog` owns. The module-level
    `_ch_query_json` is that method's private wrapper: it stays inside
    `bronze_catalog` (`AGENTS.md` rule 4 -- use the helper that already
    exists, and do not reach into another module's private names)."""
    read = []
    private = []

    def _query_json(self, statement):
        read.append(statement)
        return [{"table_count": "1", "total_bytes": "2"}]

    def _private(_target, statement):
        private.append(statement)
        return []

    monkeypatch.setattr(bronze_catalog.ClickHouseTarget, "query_json", _query_json)
    monkeypatch.setattr(bronze_catalog, "_ch_query_json", _private)
    monkeypatch.setattr(maintenance, "_ch_query", lambda _cfg, sql: "")

    rows = storage_snapshot.measure_layers(_cfg(), _store())

    assert read, "the layers are read through the public method"
    assert private == [], "another module's private helper was used"
    assert {row.layer for row in rows} == {"bronze", "gold", "silver", "serving"}


def test_the_target_method_answers_the_rows_and_its_private_wrapper_answers_the_same(
    monkeypatch,
):
    """`ClickHouseTarget.query_json` is the public way to read rows, and
    `bronze_catalog._ch_query_json` is its private wrapper: the module's
    own callers keep working, and both answer the same rows."""
    answer = mock.Mock()
    answer.raise_for_status = mock.Mock()
    answer.json.return_value = {"data": [{"table_count": "2", "total_bytes": "4096"}]}
    posted = []

    def _post(url, **kwargs):
        posted.append((url, kwargs))
        return answer

    monkeypatch.setattr(requests, "post", _post)
    target = bronze_catalog.ClickHouseTarget(
        url="http://ch.invalid", user="default", password=""
    )

    rows = target.query_json("SELECT 1")

    assert rows == [{"table_count": "2", "total_bytes": "4096"}]
    assert posted[0][0] == "http://ch.invalid"
    assert posted[0][1]["timeout"] == 30
    assert "FORMAT JSON" in posted[0][1]["data"].decode("utf-8")

    # The private wrapper DELEGATES to that method: one reading rule, in
    # one place, and not a second copy of the request beside it.
    monkeypatch.setattr(
        bronze_catalog.ClickHouseTarget,
        "query_json",
        lambda self, statement: [{"delegated": True}],
    )
    assert bronze_catalog._ch_query_json(target, "SELECT 1") == [{"delegated": True}]


def test_the_clickhouse_layers_are_read_with_the_columns_clickhouse_26_8_has(monkeypatch):
    """The layer's tables are counted with `system.tables.engine`, the
    column ClickHouse 26.8 actually has. 26.8 has NO `type` column in
    `system.tables`, and a statement naming it is refused with
    UNKNOWN_IDENTIFIER, which failed every run of this job on the silver
    and serving layers. The fake above knows 26.8's own columns and
    refuses a statement that names one it does not have, so this test
    fails when the statement goes back to `type = 'Table'`."""
    _fake_clickhouse_26_8(
        monkeypatch,
        tables={"silver": 3, "serving": 6},
        bytes_={"silver": 9619, "serving": 13803},
    )
    monkeypatch.setattr(maintenance, "_ch_query", lambda _cfg, sql: "")

    rows = storage_snapshot.measure_layers(_cfg(), _store())

    by_layer = {row.layer: row for row in rows}
    assert by_layer["silver"].table_count == 3
    assert by_layer["silver"].total_bytes == 9619
    assert by_layer["silver"].tables_skipped == 0
    assert by_layer["silver"].skipped_reason == ""
    assert by_layer["serving"].table_count == 6
    assert by_layer["serving"].total_bytes == 13803
    assert by_layer["serving"].tables_skipped == 0


def test_a_clickhouse_layer_that_cannot_be_read_is_a_skip_and_not_a_zero_byte_layer(
    monkeypatch,
):
    """A layer whose own size cannot be read is a SKIP with the fixed
    reason, not a layer of zero bytes: the tables of the layer are still
    counted, every one of them is marked as not measured, the bytes that
    could not be read are left out of the total instead of being counted
    as a measured zero, and the other layer is still measured — the loop
    continues."""
    seen = []

    def _query_json(_target, statement):
        seen.append(statement)
        if "system.parts" in statement and "'silver'" in statement:
            raise requests.HTTPError(
                "Code: 47. DB::Exception: Unknown expression or function "
                "identifier `bytes_on_disk` in scope (UNKNOWN_IDENTIFIER)"
            )
        if "system.parts" in statement:
            return [{"total_bytes": "13803"}]
        return [{"table_count": "3"}]

    monkeypatch.setattr(bronze_catalog.ClickHouseTarget, "query_json", _query_json)
    monkeypatch.setattr(maintenance, "_ch_query", lambda _cfg, sql: "")

    rows = storage_snapshot.measure_layers(_cfg(), _store())

    by_layer = {row.layer: row for row in rows}
    silver = by_layer["silver"]
    assert silver.table_count == 3
    assert silver.total_bytes == 0
    assert silver.tables_skipped == 3
    assert silver.skipped_reason == storage_snapshot.SKIP_REASON_LAYER_UNREADABLE
    # The upstream text never reaches the reason the API hands out.
    assert "UNKNOWN_IDENTIFIER" not in silver.skipped_reason
    assert "bytes_on_disk" not in silver.skipped_reason
    # The other layer is measured: a degraded failure never stops the loop.
    assert by_layer["serving"].tables_skipped == 0
    assert by_layer["serving"].total_bytes == 13803


def test_the_writer_asks_for_the_canonical_schema_before_it_writes(monkeypatch):
    """The table's shape is the one `bronze_catalog` owns, and the writer
    asks for it (the R10 ensure-schema step) before it writes a row."""
    checked = []
    inserted = []
    monkeypatch.setattr(
        bronze_catalog, "_assert_or_create_all", lambda target, schemas: checked.append(schemas)
    )
    monkeypatch.setattr(bronze_catalog, "_ch_exec", lambda target, sql: inserted.append(sql))

    bronze_catalog.record_storage_layer_snapshot(
        day="2026-11-16",
        rows=[
            bronze_catalog.StorageLayerRow(
                layer="bronze",
                table_count=2,
                total_bytes=350,
                tables_skipped=0,
                skipped_reason="",
                stored_bytes=580,
            )
        ],
        target=bronze_catalog.ClickHouseTarget(
            url="http://ch.invalid", user="default", password=""
        ),
    )

    assert checked == [(bronze_catalog._STORAGE_LAYER_SNAPSHOT_SCHEMA,)]
    assert len(inserted) == 1
    assert "bronze_meta.storage_layer_snapshot" in inserted[0]
    assert "'bronze'" in inserted[0]


def test_the_writer_writes_the_stored_bytes_column_by_name(monkeypatch):
    """The row carries BOTH numbers, and the INSERT names the column
    instead of relying on the column order: the column is the LAST one of
    the canonical shape, and a row whose bytes are not in the object store
    (a ClickHouse layer) writes SQL `NULL` there -- not a zero, which
    would say "this layer holds no bytes"."""
    inserted = []
    monkeypatch.setattr(bronze_catalog, "_assert_or_create_all", lambda target, schemas: None)
    monkeypatch.setattr(bronze_catalog, "_ch_exec", lambda target, sql: inserted.append(sql))

    bronze_catalog.record_storage_layer_snapshot(
        day="2026-11-16",
        rows=[
            bronze_catalog.StorageLayerRow(
                layer="bronze",
                table_count=2,
                total_bytes=350,
                tables_skipped=0,
                skipped_reason="",
                stored_bytes=580,
            ),
            bronze_catalog.StorageLayerRow(
                layer="silver",
                table_count=3,
                total_bytes=4096,
                tables_skipped=0,
                skipped_reason="",
                stored_bytes=None,
            ),
        ],
        target=bronze_catalog.ClickHouseTarget(
            url="http://ch.invalid", user="default", password=""
        ),
    )

    insert = inserted[0]
    assert "stored_bytes" in insert, insert
    assert "580" in insert, insert
    assert "NULL" in insert, insert
    # The column is named in the column list, not only in the values.
    assert "(day, layer, table_count, total_bytes, tables_skipped, skipped_reason, " in insert
    assert "measured_at, stored_bytes) VALUES" in insert, insert


def test_the_schema_is_the_one_the_plan_names():
    """The table's own shape: one row per layer per day, with the day's
    CURRENT data, the bytes stored on disk, how many tables could not be
    measured, and why. The schema is owned by `bronze_catalog` (the R10
    single-owner rule) and it is NOT folded into `EXPECTED_SCHEMAS`, which
    holds only the registry tables `register_bronze_table` writes.

    `stored_bytes` is LAST, because the table already exists on the
    deployments that ran this job before it measured the bytes on disk,
    and ClickHouse's `ADD COLUMN` appends the new column at the end."""
    schema = bronze_catalog._STORAGE_LAYER_SNAPSHOT_SCHEMA
    assert schema.table_name == "bronze_meta.storage_layer_snapshot"
    assert schema.columns == (
        ("day", "Date"),
        ("layer", "String"),
        ("table_count", "UInt32"),
        ("total_bytes", "UInt64"),
        ("tables_skipped", "UInt32"),
        ("skipped_reason", "String"),
        ("measured_at", "DateTime64(3, 'UTC')"),
        ("stored_bytes", "Nullable(UInt64)"),
    )
    assert schema.engine == "ReplacingMergeTree"
    assert schema.order_by == ("layer", "day")
    assert schema not in bronze_catalog.EXPECTED_SCHEMAS
    # The column the table may be MISSING on an existing deployment is
    # the one this plan adds, and it is the only one.
    assert schema.additive_columns == (("stored_bytes", "Nullable(UInt64)"),)


# ── The two numbers: the current data and the bytes on disk ───────────────
#
# A table's current snapshot counts the files the LIVE snapshot points at.
# The bytes under the table's own location also hold the older snapshots'
# files, the metadata and the orphan files, so the second number is the
# bigger one. They are reported separately because they measure different
# things, and the page shows both.


def test_the_stored_number_is_bigger_than_the_current_number_when_a_table_keeps_history(
    monkeypatch,
):
    """Two tables, one of them with two snapshots' worth of files: the
    "current data" number (the current snapshot's `total-files-size`) is
    BELOW the "stored on disk" number (the bytes under the table's own
    location), because the location also holds the older snapshot's files
    and the metadata."""
    _fake_catalog(
        monkeypatch,
        {
            "orders": _metadata("100", table="orders"),
            "customers": _metadata("250", table="customers"),
        },
    )
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\nbronze.customers\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )
    store = _store(
        {
            # orders: the live snapshot's 100 bytes, an older snapshot's
            # 200 bytes, and the metadata document.
            f"{BUCKET}/warehouse/bronze/orders/data/current.parquet": 100,
            f"{BUCKET}/warehouse/bronze/orders/data/older.parquet": 200,
            f"{BUCKET}/warehouse/bronze/orders/metadata/v1.metadata.json": 30,
            # customers: only the live snapshot, so its two numbers agree.
            f"{BUCKET}/warehouse/bronze/customers/data/current.parquet": 250,
        }
    )

    rows = storage_snapshot.measure_layers(_cfg(), store)

    bronze = next(row for row in rows if row.layer == "bronze")
    assert bronze.total_bytes == 350
    assert bronze.stored_bytes == 580
    assert bronze.stored_bytes > bronze.total_bytes
    assert bronze.tables_skipped == 0
    # ONE listing per table, and ONE probe of the bucket root before
    # them: the run time of this job is the sum of those listings, so a
    # second listing of the same location would double it.
    #
    # The listed path carries the BUCKET, because `s3fs` reads the first
    # segment of a scheme-less path as the bucket name: the bare key
    # would list a bucket named after the table's first folder.
    assert store._fs.listed == [
        BUCKET,
        f"{BUCKET}/warehouse/bronze/orders",
        f"{BUCKET}/warehouse/bronze/customers",
    ], store._fs.listed
    for path in store._fs.listed[1:]:
        assert path.startswith(f"{BUCKET}/"), path


def test_a_bucket_named_after_the_table_folder_does_not_answer_the_listing(monkeypatch):
    """The listed path carries the bucket, because `s3fs` reads the FIRST
    segment of a scheme-less path as the bucket name. A store that holds
    a SECOND bucket named after the table's first folder (`warehouse`)
    answers the bare-key listing with THAT bucket's objects, and the
    "stored on disk" number would be another bucket's bytes.

    The measured bucket's own bytes are the ones counted."""
    _fake_catalog(
        monkeypatch,
        {"orders": _metadata("100", table="orders")},
    )
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )
    store = _store(
        {
            # The measured bucket: the table's own files.
            f"{BUCKET}/warehouse/bronze/orders/data/current.parquet": 100,
            f"{BUCKET}/warehouse/bronze/orders/data/older.parquet": 200,
            # The DECOY bucket, named after the table's first folder: its
            # key is exactly what the bare-key path would list.
            "warehouse/bronze/orders/data/decoy.parquet": 999,
        },
        buckets=(BUCKET, "warehouse"),
    )

    rows = storage_snapshot.measure_layers(_cfg(), store)

    bronze = next(row for row in rows if row.layer == "bronze")
    assert bronze.stored_bytes == 300
    assert bronze.total_bytes == 100
    assert bronze.tables_skipped == 0
    assert store._fs.listed == [
        BUCKET,
        f"{BUCKET}/warehouse/bronze/orders",
    ], store._fs.listed


def test_a_table_whose_files_cannot_be_listed_is_skipped_and_is_in_neither_total(monkeypatch):
    """One table's listing fails: that table is skipped with its fixed
    reason, and its bytes are in NEITHER total -- not a zero in the stored
    number, and not its current size in the other one. A table that could
    not be measured whole must never look measured and empty."""
    _fake_catalog(
        monkeypatch,
        {
            "orders": _metadata("100", table="orders"),
            "broken": _metadata("999", table="broken"),
        },
    )
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\nbronze.broken\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )
    store = _store(
        {f"{BUCKET}/warehouse/bronze/orders/data/current.parquet": 100},
        unlistable=(f"{BUCKET}/warehouse/bronze/broken",),
    )

    rows = storage_snapshot.measure_layers(_cfg(), store)

    bronze = next(row for row in rows if row.layer == "bronze")
    assert bronze.tables_skipped == 1
    assert bronze.skipped_reason == storage_snapshot.SKIP_REASON_NOT_LISTED
    assert bronze.total_bytes == 100
    assert bronze.stored_bytes == 100
    assert "999" not in str(bronze), bronze


def test_a_table_outside_the_measured_bucket_is_skipped(monkeypatch):
    """A table whose location names ANOTHER bucket cannot be measured
    against this bucket's size: its bytes would not be comparable with the
    bucket's, so the table is skipped with its own fixed reason and
    contributes to neither total."""
    _fake_catalog(
        monkeypatch,
        {"orders": _metadata("100", table="orders", location="s3://other-bucket/wh/orders")},
    )
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )

    rows = storage_snapshot.measure_layers(_cfg(), _store())

    bronze = next(row for row in rows if row.layer == "bronze")
    assert bronze.tables_skipped == 1
    assert bronze.skipped_reason == storage_snapshot.SKIP_REASON_OUTSIDE_BUCKET
    assert bronze.total_bytes == 0
    assert bronze.stored_bytes == 0


def test_the_skip_reasons_of_the_iceberg_layers_are_fixed_sentences(monkeypatch):
    """The reasons that reach the table (and from there the API) are four
    fixed sentences for the Iceberg layers and one for the ClickHouse
    layers. No upstream text -- a catalog error, a path, an S3 error --
    ever reaches them."""
    _fake_catalog(
        monkeypatch,
        {
            "orders": _metadata("100", table="orders"),
            "nosize": {
                "metadata": {
                    "current-snapshot-id": 7,
                    "snapshots": [{"snapshot-id": 7, "summary": {}}],
                    "location": _location("nosize"),
                }
            },
            "nolocation": {
                "metadata": {
                    "current-snapshot-id": 7,
                    "snapshots": [{"snapshot-id": 7, "summary": {"total-files-size": "5"}}],
                    "snapshot-summary": {"total-files-size": "5"},
                }
            },
        },
    )
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\nbronze.nosize\nbronze.nolocation\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )

    rows = storage_snapshot.measure_layers(_cfg(), _store())

    bronze = next(row for row in rows if row.layer == "bronze")
    assert bronze.tables_skipped == 2
    assert set(bronze.skipped_reason.split("; ")) == {
        storage_snapshot.SKIP_REASON_NO_SIZE,
        storage_snapshot.SKIP_REASON_UNREADABLE,
    }
    assert (
        storage_snapshot.SKIP_REASON_UNREADABLE
        == "the catalog's metadata could not be read"
    )
    assert storage_snapshot.SKIP_REASON_NO_SIZE == "the snapshot summary holds no size"
    assert (
        storage_snapshot.SKIP_REASON_NOT_LISTED == "the table's files could not be listed"
    )
    assert (
        storage_snapshot.SKIP_REASON_OUTSIDE_BUCKET
        == "the table lives outside the measured bucket"
    )
    assert (
        storage_snapshot.SKIP_REASON_LAYER_UNREADABLE
        == "the layer's own size could not be read"
    )


def test_the_clickhouse_layers_have_one_number_and_no_stored_number(monkeypatch):
    """`silver` and `serving` are ClickHouse databases: their bytes sit on
    the ClickHouse server's own disk, not in the warehouse bucket, so they
    have ONE number and their stored number is `None` ("not measured
    here"), never a zero ("this layer holds no bytes")."""
    _fake_catalog(monkeypatch, {})
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="",
        parts={
            "silver": {"table_count": "3", "total_bytes": "4096"},
            "serving": {"table_count": "6", "total_bytes": "8192"},
        },
    )

    rows = storage_snapshot.measure_layers(_cfg(), _store())

    by_layer = {row.layer: row for row in rows}
    assert by_layer["silver"].stored_bytes is None
    assert by_layer["serving"].stored_bytes is None
    assert by_layer["silver"].total_bytes == 4096


def test_an_object_store_that_refuses_the_listing_fails_the_run_and_is_not_a_skip(monkeypatch):
    """The object store's own failure -- a wrong endpoint, a refused
    credential -- is not a per-table skip: it fails the run with the
    environment variables to set, rather than reporting "every table
    skipped, zero bytes" (`docs/CODE-STANDARD.md`: a configuration/auth
    failure is a `Failure`)."""
    _fake_catalog(monkeypatch, {})
    _fake_clickhouse(
        monkeypatch,
        catalog_tables="bronze.orders\n",
        parts={
            "silver": {"table_count": "1", "total_bytes": "10"},
            "serving": {"table_count": "1", "total_bytes": "20"},
        },
    )
    store = _store({}, unlistable=(BUCKET,))

    with pytest.raises(Failure) as err:
        storage_snapshot.measure_layers(_cfg(), store)

    message = str(err.value)
    assert "CONNECTOR_S3_ACCESS_KEY" in message
    assert "CONNECTOR_S3_SECRET_KEY" in message
    assert "CH_RUSTFS_S3_ENDPOINT" in message
    assert "LAKEHOUSE_WAREHOUSE_BUCKET" in message
    assert "Access Denied" not in message, f"the upstream text reached the message: {message}"


def test_the_object_store_is_built_through_the_capacity_jobs_public_helper(monkeypatch):
    """The "stored on disk" number and the bucket size are read through ONE
    setup: `capacity_snapshot.build_filesystem`, the public helper of the
    module that owns the bucket. A second copy of the credential and
    endpoint logic would let the two numbers disagree for a reason nobody
    could name (`AGENTS.md` rule 4)."""
    built = []

    def _build(cfg):
        built.append(cfg)
        return _FakeFilesystem({}, ())

    monkeypatch.setattr(capacity_snapshot, "build_filesystem", _build)
    monkeypatch.setenv("CONNECTOR_S3_ACCESS_KEY", "connector-key")
    monkeypatch.setenv("CONNECTOR_S3_SECRET_KEY", "connector-secret")
    monkeypatch.setenv("CH_RUSTFS_S3_ENDPOINT", "http://rustfs.invalid:9000")
    monkeypatch.setenv("LAKEHOUSE_WAREHOUSE_BUCKET", BUCKET)

    store = storage_snapshot.ObjectStore.from_capacity_config(
        capacity_snapshot.CapacityConfig.from_env()
    )

    assert len(built) == 1, built
    assert built[0].rustfs_access_key == "connector-key"
    assert built[0].rustfs_secret_key == "connector-secret"
    assert built[0].rustfs_endpoint == "http://rustfs.invalid:9000"
    assert store._bucket == BUCKET


def test_the_stored_objects_are_listed_through_the_capacity_jobs_public_helper(monkeypatch):
    """The rule for "what counts as a stored object" is the capacity job's
    own: `capacity_snapshot.list_stored_objects`. The directory marker
    `s3fs` may return is not an object, so it is in neither the bytes nor
    the count -- and the two callers cannot disagree about it."""
    listed = []

    def _list(fs, path):
        listed.append(path)
        return 42, 1

    monkeypatch.setattr(capacity_snapshot, "list_stored_objects", _list)
    store = _store()

    assert store.stored_bytes(_location("orders")) == (42, "")
    assert listed == [f"{BUCKET}/warehouse/bronze/orders"], listed


@pytest.fixture
def reload_storage_snapshot(monkeypatch):
    """Re-imports the module, so the job's tags are built with the
    environment this test sets: a job's tags are read once, when the module
    is imported."""

    def _reload(**env):
        for name, value in env.items():
            monkeypatch.setenv(name, value)
        return importlib.reload(storage_snapshot)

    yield _reload
    monkeypatch.delenv(storage_snapshot.RUN_LIMIT_ENV, raising=False)
    importlib.reload(storage_snapshot)


def test_the_job_carries_its_own_run_limit_and_the_ingest_job_keeps_its_own(
    reload_storage_snapshot,
):
    """The new job carries a `dagster/max_runtime` tag of its own: a run
    whose process dies would otherwise hold a concurrency slot forever
    (`SRC-39`). Thirty minutes is far above a sum over table metadata and
    far below the six hours `ingest_job` gets."""
    module = reload_storage_snapshot()
    assert module.storage_snapshot_job.tags[TAG] == "1800"
    assert ingest_factory.ingest_job.tags[TAG] == str(
        ingest_factory.DEFAULT_RUN_LIMIT_SECONDS
    )


def test_the_run_limit_of_the_job_comes_from_the_setting(reload_storage_snapshot):
    module = reload_storage_snapshot(**{storage_snapshot.RUN_LIMIT_ENV: "900"})
    assert module.storage_snapshot_job.tags[TAG] == "900"
    # The ingest job's own limit is its own setting, not this one.
    assert ingest_factory.ingest_job.tags[TAG] == str(
        ingest_factory.DEFAULT_RUN_LIMIT_SECONDS
    )


def test_the_clickhouse_layers_are_measured_from_system_parts(monkeypatch):
    """`silver` and `serving` are ClickHouse databases, not Iceberg
    namespaces: their size is their own `system.parts`, and the number of
    tables is the database's own table list."""
    _fake_catalog(monkeypatch, {})
    seen = []

    def _query_json(_target, statement):
        seen.append(statement)
        if "'silver'" in statement:
            return [{"table_count": "4", "total_bytes": "1024"}]
        return [{"table_count": "6", "total_bytes": "2048"}]

    monkeypatch.setattr(maintenance, "_ch_query", lambda _cfg, sql: "")
    monkeypatch.setattr(bronze_catalog.ClickHouseTarget, "query_json", _query_json)

    rows = storage_snapshot.measure_layers(_cfg(), _store())
    by_layer = {row.layer: row for row in rows}
    assert by_layer["silver"].total_bytes == 1024
    assert by_layer["silver"].table_count == 4
    assert by_layer["serving"].total_bytes == 2048
    assert by_layer["serving"].table_count == 6
    assert any("system.parts" in statement for statement in seen)
