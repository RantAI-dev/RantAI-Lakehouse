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


def _lakekeeper_token() -> str:
    """R1 (ADR 0011): Lakekeeper authorization is enforced by default in
    this stack (no "allow-all" mode) -- a bare unauthenticated `RestCatalog`
    call now 401s at `/v1/config` before this script can do anything.
    Mirrors `dagster/dispar_orchestrate/dlt_pipeline.py`'s
    `LAKEKEEPER_TOKEN_FILE` pattern (a pre-minted static bearer token read
    from a file, since this script is a one-shot process with no
    long-running shell step to interpolate a file into an env var itself)
    plus a direct `LAKEKEEPER_TOKEN` for callers that already hold the
    token value (e.g. a `docker compose run -e` invocation). Empty string
    on a pre-R1 or authz-disabled stack, exactly like `dlt_pipeline.py`."""
    token_file = _env("LAKEKEEPER_TOKEN_FILE", "")
    if token_file and os.path.exists(token_file):
        with open(token_file, encoding="utf-8") as f:
            return f.read().strip()
    return _env("LAKEKEEPER_TOKEN", "")


def _catalog():
    conf: dict[str, str] = {
        "type": "rest",
        "uri": _env("LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"),
        "warehouse": _env("LAKEKEEPER_WAREHOUSE", "default"),
        "s3.endpoint": _env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
        # CREDENTIAL HYGIENE (review finding): this used to read
        # `RUSTFS_ACCESS_KEY`/`RUSTFS_SECRET_KEY` directly — RustFS's own
        # ROOT credentials — even though this script only ever
        # creates/appends to Iceberg tables under the one warehouse
        # bucket. Reads the connector-scoped names instead
        # (`CONNECTOR_S3_ACCESS_KEY`/`CONNECTOR_S3_SECRET_KEY`, set in
        # `docker-compose.yml`'s `dagster-code-location` env block — this
        # script runs INSIDE that same container per `docs/plans/
        # G3-RESULT.md`). Those vars default to the RustFS root key only
        # because this stack's RustFS version has no non-proprietary-API
        # way to mint a narrower S3 identity (see `dlt_pipeline.py`'s
        # matching comment) — a deployment that can provision one only
        # needs to set the two vars, no script change.
        "s3.access-key-id": _env("CONNECTOR_S3_ACCESS_KEY", "rustfsadmin"),
        "s3.secret-access-key": _env("CONNECTOR_S3_SECRET_KEY", "rustfsadmin"),
        "s3.region": "us-east-1",
        "s3.path-style-access": "true",
        "s3.force-virtual-addressing": "false",
    }
    token = _lakekeeper_token()
    if token:
        conf["token"] = token
    return load_catalog("default", **conf)


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


def _per_partition_file_counts(table) -> dict[str, int]:
    files_tbl = table.inspect.files()
    partition_col = files_tbl.column("partition")
    per_partition: dict[str, int] = {}
    for value in partition_col.to_pylist():
        key = str(value)
        per_partition[key] = per_partition.get(key, 0) + 1
    return per_partition


def measure_files(table_name: str = TABLE_NAME) -> dict:
    """Data file count per partition — via `table.inspect.files()`, the
    exact per-data-file manifest listing pyiceberg exposes (partition
    column values included), not an approximation."""
    cat = _catalog()
    table = cat.load_table((NAMESPACE, table_name))
    table.refresh()
    per_partition = _per_partition_file_counts(table)
    result = {
        "event": "file_count",
        "table": f"{NAMESPACE}.{table_name}",
        "total_data_files": sum(per_partition.values()),
        "partitions": len(per_partition),
        "per_partition": per_partition,
    }
    print(json.dumps(result, default=str))
    return result


def _measure_planning(table_name: str) -> dict:
    """Planning-time proxy: the wall-clock cost of enumerating
    `table.scan().plan_files()` — pyiceberg's own manifest-list + manifest
    walk that produces the `FileScanTask`s a query engine would then read.
    This is the same class of work ClickHouse's `DataLakeCatalog` engine
    does before it can read a single row of an Iceberg-registered table
    (list manifests -> list data files -> intersect with the query), so it
    is a reasonable engine-agnostic proxy for "how much does small-file
    accumulation degrade query planning", without requiring a query engine
    to be up at all. `list(...)` forces full enumeration (a generator
    alone would not pay the real cost)."""
    cat = _catalog()
    table = cat.load_table((NAMESPACE, table_name))
    table.refresh()
    t0 = time.time()
    tasks = list(table.scan().plan_files())
    elapsed = time.time() - t0
    return {
        "table": f"{NAMESPACE}.{table_name}",
        "planned_file_scan_tasks": len(tasks),
        "planning_s": round(elapsed, 4),
    }


def measure_planning(table_name: str = TABLE_NAME) -> dict:
    result = {"event": "planning_time", **_measure_planning(table_name)}
    print(json.dumps(result, default=str))
    return result


def compare(threshold: float) -> None:
    """The actual G3 acceptance measurement: file count per partition and
    the planning-time proxy, BEFORE (`bronze.g3_control` — one commit per
    partition, the layout compaction would produce if any measured
    ClickHouse verb could bin-pack) and AFTER (`TABLE_NAME` —
    `COMMITS_PER_DAY` small commits per partition, the actual accumulated
    shape with no working in-engine compaction), with the plan's own 2x
    threshold asserted for real rather than existing only in prose. Both
    tables must already exist (`generate` then `generate-control`, or
    `docker-compose.yml`'s `g3-test-runner` running both).

    Reports the numbers it actually measures either way — including a
    failing/below-threshold ratio — rather than silently passing or
    tuning itself to match any previously-recorded figure."""
    cat = _catalog()
    small = _measure_planning(TABLE_NAME)
    small["file_counts"] = measure_files(TABLE_NAME)
    control_ident = (NAMESPACE, "g3_control")
    if not cat.table_exists(control_ident):
        raise SystemExit(
            f"g3_control table does not exist — run 'generate-control' first "
            f"(compare needs both the small-file table {small['table']!r} and "
            "the compacted-baseline control to compute a ratio)"
        )
    control = _measure_planning("g3_control")
    control["file_counts"] = measure_files("g3_control")

    ratio = (small["planning_s"] / control["planning_s"]) if control["planning_s"] > 0 else float("inf")
    passed = ratio >= threshold
    result = {
        "event": "compare",
        "small_file": small,
        "control": control,
        "planning_time_ratio": round(ratio, 3) if ratio != float("inf") else "inf",
        "threshold": threshold,
        "passed": passed,
    }
    # Always print the real numbers BEFORE deciding pass/fail — per the
    # task brief: report what was actually measured, even if it does not
    # reproduce the prose figures, rather than tuning the test to match them.
    print(json.dumps(result, default=str))
    if not passed:
        raise SystemExit(
            f"G3 FAILED: planning-time ratio {result['planning_time_ratio']} "
            f"is below the {threshold}x threshold "
            f"(small-file: {small['planning_s']}s over {small['file_counts']['total_data_files']} "
            f"files; control: {control['planning_s']}s over {control['file_counts']['total_data_files']} files)"
        )
    small_avg = small["file_counts"]["total_data_files"] / max(small["file_counts"]["partitions"], 1)
    control_avg = control["file_counts"]["total_data_files"] / max(control["file_counts"]["partitions"], 1)
    if small_avg <= control_avg:
        raise SystemExit(
            f"G3 FAILED: small-file table has {small_avg:.1f} files/partition on "
            f"average, not more than control's {control_avg:.1f} — the load "
            "generator did not actually produce a small-file accumulation to measure"
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
        "action",
        choices=[
            "generate",
            "generate-control",
            "measure-files",
            "measure-planning",
            "compare",
        ],
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=float(os.environ.get("G3_PLANNING_RATIO_THRESHOLD", "2.0")),
        help="compare: minimum small-file/control planning-time ratio to pass (default 2.0x, "
        "matching the plan's own G3 gate)",
    )
    args = parser.parse_args()
    if args.action == "generate":
        generate()
    elif args.action == "generate-control":
        generate_control()
    elif args.action == "measure-files":
        measure_files()
    elif args.action == "measure-planning":
        measure_planning()
    else:
        compare(args.threshold)
