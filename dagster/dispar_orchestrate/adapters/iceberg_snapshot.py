"""One Iceberg snapshot per load (`DATA-36`).

# Why this exists

A load written in groups committed ONE SNAPSHOT PER GROUP. `pyiceberg`'s
`Transaction.append` commits a snapshot of its own, and `Transaction.overwrite` commits a
DELETE snapshot first and an APPEND snapshot after it, so a load of N groups left N (or
N + 1) snapshots behind -- and every one of them was a state the table could be read at:
the time-travel picker lists every snapshot of a table
(`src/features/queries/time-travel-controls.tsx`), so a reader could choose a HALF-LOADED
table, one that holds the first groups of the load and not the last. A 2 GiB load added
about 64 snapshots.

# What it does

`write_one_snapshot` writes every group of a load into ONE snapshot producer, so the load
leaves ONE snapshot:

- an `append` is ONE `fast_append` producer;
- a `replace` is ONE `overwrite` producer that deletes every data file of the current
  snapshot and adds the load's own: ONE OVERWRITE snapshot, in which the old rows and the
  new rows are never both readable.

The memory bound of a load is unchanged: the groups are still written one at a time and
only the group's own Arrow table is held (`iceberg_stream.FLUSH_BYTES`,
`sink.RETYPE_FLUSH_BYTES`).

# The pinned private API

`pyiceberg.io.pyarrow._dataframe_to_data_files` and
`pyiceberg.io.pyarrow._check_pyarrow_schema_compatible` are PRIVATE functions of
`pyiceberg` 0.12.0, the version pinned in `dagster/pyproject.toml`. They are the two
halves of what `pyiceberg`'s own `Transaction.append` does, minus the commit: the check
(the load's Arrow table must fit the table's schema -- without it the write DROPS a
column the table does not have, silently) and the write of the group's data files.
`pyiceberg` offers no public way to add a data file to a producer, so they are pinned
here:

- the import is INSIDE the functions that use them, so a `pyiceberg` upgrade that moves
  them fails the FIRST load with an ImportError instead of the loading of the Dagster code
  location (which takes down every job in it);
- `test_iceberg_snapshot.py` fails loudly, before a load does, when either of them is
  gone or has another shape.
"""

from __future__ import annotations

import itertools
from collections.abc import Iterable
from typing import Any

import pyarrow as pa


def _live_data_files(transaction: Any, table: Any) -> list[Any]:
    """The data files the table's current snapshot holds.

    These are the files a `replace` has to delete: they are what the table reads now.
    `pyiceberg` deletes a data file only when it is NAMED (`delete_data_file`) and refuses
    the commit when a named file is not in the parent snapshot, so the list has to be the
    parent's own files, read from its manifests, and not a guess. The parent is the head of
    the `main` branch -- the snapshot `pyiceberg`'s own overwrite producer takes -- so the
    list and the producer always agree.

    A file that is not DATA (a delete file) is left out: `pyiceberg` deletes data files
    only, and naming a delete file would fail the commit."""
    from pyiceberg.manifest import DataFileContent
    from pyiceberg.table.refs import MAIN_BRANCH

    snapshot = transaction.table_metadata.snapshot_by_name(MAIN_BRANCH)
    if snapshot is None:
        return []
    files = []
    for manifest in snapshot.manifests(table.io):
        for entry in manifest.fetch_manifest_entry(table.io, discard_deleted=True):
            if entry.data_file.content == DataFileContent.DATA:
                files.append(entry.data_file)
    return files


def _check_compatible(table_metadata: Any, group: pa.Table) -> None:
    """The check `pyiceberg`'s own `Transaction.append` makes before it writes a table: the
    load's Arrow table must fit the table's schema. Without it a column the table does
    not have is DROPPED by the write, without a word (the write projects the data file on
    the table's schema).

    # Errors

    `ValueError` when the group does not fit the table's schema."""
    from pyiceberg.io.pyarrow import _check_pyarrow_schema_compatible
    from pyiceberg.table import DOWNCAST_NS_TIMESTAMP_TO_US_ON_WRITE
    from pyiceberg.utils.config import Config

    _check_pyarrow_schema_compatible(
        table_metadata.schema(),
        provided_schema=group.schema,
        downcast_ns_timestamp_to_us=Config().get_bool(DOWNCAST_NS_TIMESTAMP_TO_US_ON_WRITE) or False,
        format_version=table_metadata.format_version,
    )


def write_one_snapshot(
    table: Any,
    transaction: Any,
    groups: Iterable[pa.Table],
    *,
    replace: bool = False,
    snapshot_properties: dict[str, str] | None = None,
) -> None:
    """Write `groups` into `table` as ONE snapshot, inside `transaction`.

    The caller opens the transaction (it has to stage its own changes -- the schema the
    load evolves the table to -- before the rows are written) and calls this inside it. The
    transaction commits last, so a failure anywhere here commits nothing: the groups'
    data files stay in the bucket, unreferenced, for `maintenance.py`'s orphan clean-up.

    `replace` is a `replace` load: every data file of the current snapshot is deleted and
    the load's own files are added, which is ONE OVERWRITE snapshot. Without it the load
    is an `append`: ONE APPEND snapshot.

    `snapshot_properties` are the summary entries the ONE snapshot carries (`DATA-15` F5's
    `rantai.author` is the one the console reads). They are what the load carried before:
    ONE set, on the ONE snapshot. The load paths that write through here carry none today
    -- `dlt`'s own commits do not (`sink.SinkConfig`'s note) -- so the default is the
    empty set, and the paths that carry an author (`history.write_versions`) keep their
    own.

    # Errors

    Whatever `pyiceberg` raises; nothing is committed then."""
    from pyiceberg.io.pyarrow import _dataframe_to_data_files

    update = transaction.update_snapshot(snapshot_properties=dict(snapshot_properties or {}))
    if replace:
        producer = update.overwrite()
        for data_file in _live_data_files(transaction, table):
            producer.delete_data_file(data_file)
    else:
        producer = update.fast_append()
    # ONE counter for the whole load: `pyiceberg` names a data file
    # `00000-{task_id}-{write_uuid}.parquet` and the task id comes from this counter. A
    # counter per group gives every group's file the SAME name, and the second group then
    # overwrites the first group's rows (measured on pyiceberg 0.12.0, 2026-11-01: the
    # table held the last group's rows twice and the first group's rows not at all).
    counter = itertools.count(0)
    with producer:
        for group in groups:
            _check_compatible(transaction.table_metadata, group)
            for data_file in _dataframe_to_data_files(
                table_metadata=transaction.table_metadata,
                write_uuid=producer.commit_uuid,
                df=group,
                io=table.io,
                counter=counter,
            ):
                producer.append_data_file(data_file)
