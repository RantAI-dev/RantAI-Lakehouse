#!/usr/bin/env python3
"""G3 synthetic small-file load generator + before/after measurement.

`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §3 P4 acceptance (G3): "Generate
a synthetic small-file load equivalent to ~14 days of CDC-rate writes
against a Bronze table. Measure, before and after maintenance: data file
count per partition; query planning time per partition."

# Why pyiceberg directly, not dlt

`dagster/dispar_orchestrate/dlt_pipeline.py` stamps `_ingested_at =
datetime.now(timezone.utc)` for every row in a run — one partition per run.
Simulating 14 *distinct* day-partitions, each accumulating many small CDC
micro-batch commits (the actual shape of R2's small-file risk), needs
control over the partition value per commit that dlt's pipeline does not
expose. This script uses `pyiceberg`'s `RestCatalog` directly — the same
library dlt's `iceberg` destination uses internally (see ADR 0005), already
present in the `dagster-code-location` image (`dlt[pyiceberg]`) — so no
extra dependency is introduced.

# CDC-rate model

Real CDC micro-batch cadence (Debezium/dlt incremental flushes) is on the
order of one small commit every ~10-15 minutes -> ~100 commits/day. This
script models that as `COMMITS_PER_DAY` (default 20, scaled down from ~100
to keep a REST-catalog-commit-per-append loop practical to run in CI/local
verification; each commit is still exactly the same shape a real CDC flush
produces: one Iceberg `append` -> one new data file -> one new manifest
entry). The file-count-per-partition and planning-time-degradation signal
this test is measuring is a function of *file count*, not of how long the
simulated 14 days actually took to generate — the CI-practical rate proves
the same defect faster.

Run inside the compose network (Lakekeeper/RustFS advertise
compose-internal hostnames — see `dlt_pipeline.py`'s module doc for the
identical constraint), via the `g3-test-runner` compose service, itself
built from the `dagster/Dockerfile` image so `pyiceberg` is already pinned
and present (see ADR 0005 on why versions aren't duplicated here).
"""

from __future__ import annotations

import argparse
import json
import os
import random
import time
from datetime import datetime, timedelta, timezone

import pyarrow as pa
from pyiceberg.catalog import load_catalog
from pyiceberg.partitioning import PartitionField, PartitionSpec
from pyiceberg.schema import Schema
from pyiceberg.transforms import DayTransform
from pyiceberg.types import (
    DoubleType,
    LongType,
    NestedField,
    StringType,
    TimestampType,
)

TABLE_NAME = os.environ.get("G3_TABLE_NAME", "g3_loadtest")
NAMESPACE = "bronze"
DAYS = int(os.environ.get("G3_DAYS", "14"))
COMMITS_PER_DAY = int(os.environ.get("G3_COMMITS_PER_DAY", "20"))
ROWS_PER_COMMIT = int(os.environ.get("G3_ROWS_PER_COMMIT", "40"))


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


def _catalog():
    return load_catalog(
        "default",
        **{
            "type": "rest",
            "uri": _env("LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"),
            "warehouse": _env("LAKEKEEPER_WAREHOUSE", "default"),
            "s3.endpoint": _env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            "s3.access-key-id": _env("RUSTFS_ACCESS_KEY", "rustfsadmin"),
            "s3.secret-access-key": _env("RUSTFS_SECRET_KEY", "rustfsadmin"),
            "s3.region": "us-east-1",
            "s3.path-style-access": "true",
            "s3.force-virtual-addressing": "false",
        },
    )


SCHEMA = Schema(
    NestedField(1, "_ingested_at", TimestampType(), required=True),
    NestedField(2, "id", LongType(), required=True),
    NestedField(3, "customer", StringType(), required=False),
    NestedField(4, "amount", DoubleType(), required=False),
)

PARTITION_SPEC = PartitionSpec(
    PartitionField(
        source_id=1, field_id=1000, transform=DayTransform(), name="_ingested_at_day"
    )
)


def _get_or_create_table(cat):
    cat.create_namespace_if_not_exists(NAMESPACE)
    ident = (NAMESPACE, TABLE_NAME)
    if cat.table_exists(ident):
        cat.purge_table(ident)
    return cat.create_table(ident, schema=SCHEMA, partition_spec=PARTITION_SPEC)


_ARROW_SCHEMA = pa.schema(
    [
        pa.field("_ingested_at", pa.timestamp("us"), nullable=False),
        pa.field("id", pa.int64(), nullable=False),
        pa.field("customer", pa.string(), nullable=True),
        pa.field("amount", pa.float64(), nullable=True),
    ]
)


def _batch(day_ts: datetime, start_id: int, n: int) -> pa.Table:
    ids = list(range(start_id, start_id + n))
    return pa.table(
        {
            "_ingested_at": pa.array([day_ts] * n, type=pa.timestamp("us")),
            "id": pa.array(ids, type=pa.int64()),
            "customer": pa.array([f"customer_{i % 137}" for i in ids], type=pa.string()),
            "amount": pa.array(
                [round(random.uniform(1, 999), 2) for _ in ids], type=pa.float64()
            ),
        },
        schema=_ARROW_SCHEMA,
    )


def generate() -> None:
    cat = _catalog()
    table = _get_or_create_table(cat)
    base_day = datetime.now(timezone.utc).replace(
        hour=0, minute=0, second=0, microsecond=0
    ) - timedelta(days=DAYS)
    next_id = 1
    total_files = 0
    t0 = time.time()
    for d in range(DAYS):
        day_ts = base_day + timedelta(days=d)
        for _c in range(COMMITS_PER_DAY):
            table.append(_batch(day_ts, next_id, ROWS_PER_COMMIT))
            next_id += ROWS_PER_COMMIT
            total_files += 1
    elapsed = time.time() - t0
    print(
        json.dumps(
            {
                "event": "generate_complete",
                "table": f"{NAMESPACE}.{TABLE_NAME}",
                "days": DAYS,
                "commits_per_day": COMMITS_PER_DAY,
                "rows_per_commit": ROWS_PER_COMMIT,
                "total_append_commits": total_files,
                "total_rows": next_id - 1,
                "elapsed_s": round(elapsed, 1),
            }
        )
    )


def measure_files() -> None:
    """Data file count per partition — via `table.inspect.files()`, the
    exact per-data-file manifest listing pyiceberg exposes (partition
    column values included), not an approximation."""
    cat = _catalog()
    table = cat.load_table((NAMESPACE, TABLE_NAME))
    table.refresh()
    files_tbl = table.inspect.files()
    partition_col = files_tbl.column("partition")
    per_partition: dict[str, int] = {}
    for value in partition_col.to_pylist():
        key = str(value)
        per_partition[key] = per_partition.get(key, 0) + 1
    print(
        json.dumps(
            {
                "event": "file_count",
                "total_data_files": files_tbl.num_rows,
                "partitions": len(per_partition),
                "per_partition": per_partition,
            },
            default=str,
        )
    )


def generate_control() -> None:
    """A second table, `bronze.g3_control`, same row totals and same
    partitioning as the small-file load (`generate()`), but written as ONE
    commit per partition instead of `COMMITS_PER_DAY` — i.e. the file
    layout compaction *would* produce if ClickHouse could compact. This is
    the comparison baseline for "how much does small-file accumulation
    degrade planning time", independent of whether any measured
    maintenance verb can reach that state itself (G3's finding is that
    none can)."""
    cat = _catalog()
    cat.create_namespace_if_not_exists(NAMESPACE)
    ident = (NAMESPACE, "g3_control")
    if cat.table_exists(ident):
        cat.purge_table(ident)
    table = cat.create_table(ident, schema=SCHEMA, partition_spec=PARTITION_SPEC)
    base_day = datetime.now(timezone.utc).replace(
        hour=0, minute=0, second=0, microsecond=0
    ) - timedelta(days=DAYS)
    next_id = 1
    rows_per_day = COMMITS_PER_DAY * ROWS_PER_COMMIT
    for d in range(DAYS):
        day_ts = base_day + timedelta(days=d)
        table.append(_batch(day_ts, next_id, rows_per_day))
        next_id += rows_per_day
    print(json.dumps({"event": "control_complete", "table": f"{NAMESPACE}.g3_control"}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "action", choices=["generate", "measure-files", "generate-control"]
    )
    args = parser.parse_args()
    if args.action == "generate":
        generate()
    elif args.action == "generate-control":
        generate_control()
    else:
        measure_files()
