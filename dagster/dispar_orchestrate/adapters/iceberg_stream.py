"""An Iceberg write that does not hold the load in memory (`PR #186 review BLOCKER 6`).

# Why this exists

`dlt` 1.30.0 writes an Iceberg table in one job per table, handed ALL of the table's
normalized files at once, and the job (`IcebergLoadFilesystemJob.run`) does
`self.arrow_dataset.to_table()`: the whole load as one Arrow table, then a cast of it (a
second copy), then `pyiceberg`'s `append` or `overwrite` (a third, while it cuts and encodes
the data files). Profiled on 2026-10-23 with the upload job's own load path (plan section 11,
"BLOCKER 6, investigation"): the extract and normalize steps of `dlt` stay flat, and this
step is where the memory goes, at about five times the size of the data. No `dlt` setting
reaches it: file rotation (`file_max_items`) only makes more files, which the same job
still reads together. The Delta job streams (`RecordBatchReader`); the Iceberg one does not.

# What it does

`StreamingIcebergJob` is the Iceberg job with `arrow_dataset` swapped for a wrapper whose
`to_table()` hands back a `_Streamed` marker instead of the table, and the module-level
`write_iceberg_table` that `dlt` calls (looked up at call time) is replaced by one that
writes a `_Streamed` as ONE `pyiceberg` transaction, in groups of up to `FLUSH_BYTES` read
from the files in batches. Anything that is not a marker, and every `merge` load, goes the way
`dlt` went. The result is the table `dlt` made: a `replace` overwrites what the table held
and the groups add to it, inside ONE commit, so a load that fails part way commits nothing
(its data files stay in the bucket, unreferenced, for `maintenance.py`'s orphan clean-up,
as with `sink._replace_changing_types`).

`DATA-36`: that one commit is ONE snapshot, not one per group (`adapters/iceberg_snapshot.py`).
The groups used to be committed one by one, so a load of N groups left N snapshots behind and
the time-travel picker offered the half-loaded table to whoever picked one of them.

`install()` is idempotent and is called by `sink.py` before a pipeline is built. If a `dlt`
upgrade changes the job, the marker is not produced, the load goes back to the old path, and
`test_load_memory.py` fails: the memory test is the guard of this module.

# The schemas of the files (`PR #186 review SHOULD-FIX 2`)

The files of one load do not all have the same schema: `dlt` starts a new normalized file when a
table's column count changes, which a REST or Mongo source does when a later record brings a
field. `pa.Table.from_batches` refuses batches of different schemas, so the schemas are unified
first (`unified_schema`) and every batch is laid out as the unified one, a column a file does not
have becoming null in it. The table is evolved to that schema inside the same transaction, so the
schema and the rows commit together: `dlt` evolves the table with the schema of its dataset (the
first file's), which leaves a later file's new column out of the table and the write refused.
"""

from __future__ import annotations

import logging
from collections.abc import Iterator
from typing import Any

import pyarrow as pa
import pyarrow.parquet as pq

from dispar_orchestrate.adapters import iceberg_snapshot

logger = logging.getLogger(__name__)

# Rows read from the normalized files at a time. A wide row of text can be a few KiB, so the
# bytes of a group (below) are what bounds the memory, and this only bounds one read.
READ_ROWS = 20_000
# A group is written when it holds this many bytes (the same bound as
# `sink.RETYPE_FLUSH_BYTES`). Measured in plan section 11: the peak of a load is about this,
# twice, plus `dlt`'s own steps, whatever the size of the file.
FLUSH_BYTES = 32 * 1024 * 1024


class _Streamed:
    """Stands for the table a job would have read: the dataset it is read from, in batches."""

    def __init__(self, dataset: Any) -> None:
        self.dataset = dataset


class _StreamedDataset:
    """A dataset whose `to_table()` is a marker. The rest of the job reads only `schema`."""

    def __init__(self, dataset: Any) -> None:
        self._dataset = dataset

    @property
    def schema(self) -> pa.Schema:
        return self._dataset.schema

    def to_table(self) -> _Streamed:
        return _Streamed(self._dataset)


def unified_schema(dataset: Any) -> pa.Schema:
    """The one schema every file of `dataset` is read as.

    `PR #186 review SHOULD-FIX 2`: `dlt` starts a new normalized file when a table's column
    count changes (a later record of a REST or Mongo source brings a field), so the files of
    one load do not all have the same schema. `pa.Table.from_batches` refuses batches of
    different schemas, which is where the streamed write failed. The schemas are unified
    here instead: a column a file does not have becomes null in it, and the column is kept
    (`dlt`'s own path drops it, because the dataset it reads takes the schema of its first
    file). The promotion is the permissive one, as the dataset scanner's is."""
    schemas = []
    for path in dataset.files:
        with pq.ParquetFile(path, pre_buffer=False) as parquet_file:
            schemas.append(parquet_file.schema_arrow)
    if not schemas:
        return dataset.schema
    return pa.unify_schemas(schemas, promote_options="permissive")


def _fitted(batch: pa.RecordBatch, schema: pa.Schema) -> pa.RecordBatch:
    """`batch` with the unified schema: the columns it has cast to it, the ones it does not
    have as nulls (`PR #186 review SHOULD-FIX 2`)."""
    arrays = []
    for field in schema:
        if field.name in batch.schema.names:
            arrays.append(batch[field.name].cast(field.type))
        else:
            arrays.append(pa.nulls(batch.num_rows, type=field.type))
    return pa.RecordBatch.from_arrays(arrays, schema=schema)


def groups(dataset: Any, schema: pa.Schema | None = None) -> Iterator[pa.Table]:
    """The rows of `dataset`'s files as tables of up to `FLUSH_BYTES` bytes (the last smaller).

    Each file is read with `ParquetFile.iter_batches` and `pre_buffer=False`, not through the
    dataset scanner, for two measured reasons (plan section 11): the scanner reads ahead on its
    own threads without regard to how slowly its consumer writes (a 328 MiB file, written in
    6 MiB groups, held 300 MiB of Arrow memory at once; 36 MiB through `iter_batches`), and
    `pre_buffer=True`, the default, caches every byte range it fetched until the file is closed
    (the whole file again). Lists the files in the order `dlt` gave them.

    `PR #186 review SHOULD-FIX 2`: every batch is laid out as `schema`, the one schema the
    files of the load share (`unified_schema`), so files of different columns join into one
    table instead of raising."""
    if schema is None:
        schema = unified_schema(dataset)
    pending: list[pa.RecordBatch] = []
    size = 0
    for path in dataset.files:
        with pq.ParquetFile(path, pre_buffer=False) as parquet_file:
            for batch in parquet_file.iter_batches(batch_size=READ_ROWS):
                fitted = _fitted(batch, schema)
                pending.append(fitted)
                size += fitted.nbytes
                if size >= FLUSH_BYTES:
                    yield pa.Table.from_batches(pending, schema=schema)
                    pending, size = [], 0
    if pending:
        yield pa.Table.from_batches(pending, schema=schema)


def _write(table: Any, data: Any, write_disposition: str, original: Any) -> None:
    """`dlt.common.libs.pyiceberg.write_iceberg_table`, streaming a `_Streamed`.

    # Errors

    Whatever `pyiceberg` raises; nothing is committed then."""
    if not isinstance(data, _Streamed):
        original(table=table, data=data, write_disposition=write_disposition)
        return
    from dlt.common.libs.pyiceberg import (
        ensure_iceberg_compatible_arrow_data,
        ensure_iceberg_compatible_arrow_schema,
    )

    schema = unified_schema(data.dataset)
    with table.transaction() as transaction:
        # The table must carry every column the load writes: the files of one load do not
        # all have the same columns (`PR #186 review SHOULD-FIX 2`), and the unified schema
        # is their union. `dlt` evolves the table with the schema of its dataset (the first
        # file's), which leaves a later file's new column out of the table; the write is
        # then refused ("PyArrow table contains more columns"). The column is added here,
        # inside the transaction, so the schema and the rows commit together. Only the
        # columns the table does not have are added: the ones it has are left as they are.
        held = {field.name for field in table.schema().fields}
        missing = pa.schema([field for field in schema if field.name not in held])
        if missing.names:
            with transaction.update_schema() as update:
                update.union_by_name(ensure_iceberg_compatible_arrow_schema(missing))
        # `DATA-36`: the groups are written into ONE snapshot producer, so the load leaves
        # ONE snapshot and no half-loaded state of the table can be read (the time-travel
        # picker lists every snapshot). A load of no rows at all commits that one empty
        # snapshot too: it is the snapshot that carries a schema the load just evolved
        # (`history.ensure_columns` adds a column with a zero-row load, and a scan reads the
        # schema of the table's CURRENT SNAPSHOT, so without it the column is not seen).
        iceberg_snapshot.write_one_snapshot(
            table,
            transaction,
            (ensure_iceberg_compatible_arrow_data(group) for group in groups(data.dataset, schema)),
            replace=write_disposition == "replace",
        )


_installed = False


def install() -> None:
    """Make `dlt` write Iceberg tables through `StreamingIcebergJob`. Idempotent."""
    global _installed
    if _installed:
        return
    from dlt.common.libs import pyiceberg as dlt_pyiceberg
    from dlt.destinations.impl.filesystem import filesystem as dlt_filesystem

    original = dlt_pyiceberg.write_iceberg_table

    def write_iceberg_table(table: Any, data: Any, write_disposition: Any) -> None:
        _write(table, data, write_disposition, original)

    class StreamingIcebergJob(dlt_filesystem.IcebergLoadFilesystemJob):
        @property
        def arrow_dataset(self) -> Any:
            dataset = super().arrow_dataset
            # A merge keeps `dlt`'s path: `merge_iceberg_table` takes the table whole.
            if self._load_table["write_disposition"] == "merge":
                return dataset
            return _StreamedDataset(dataset)

    dlt_pyiceberg.write_iceberg_table = write_iceberg_table
    dlt_filesystem.IcebergLoadFilesystemJob = StreamingIcebergJob
    _installed = True
