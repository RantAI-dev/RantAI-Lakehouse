"""Uploaded file -> Bronze Iceberg, driven entirely by run config.

Phase 1 of the file-upload feature. `POST /api/uploads/{id}/ingest`
launches this job with the object key the console stored and the parse
options a human confirmed on the preview screen — nothing here reads the
environment to decide WHAT to ingest, which is exactly what phase 0's
run-config plumbing made possible.

# Why this reuses dlt rather than writing Iceberg itself

`dlt_pipeline.run_bronze_ingest` already knows how to write Bronze through
Lakekeeper: the REST-catalog env dance (`_install_catalog_env`), the
`_ingested_at` stamp, day partitioning, `format-version: 2`. Writing a
second Bronze writer here would mean two paths to keep in step, and the
day they diverge is the day a table written by one is unreadable the way
the other expects. This module supplies a different SOURCE (rows parsed
from a file) to the same destination.

# The parse is told, not guessed

Encoding, delimiter and header row arrive as config. The API's preview
endpoint detects them and the console shows its guess, but the value used
here is whatever the user confirmed — a detection that silently overrides
a human is worse than no detection, because the resulting Bronze table
looks successful and is wrong.

# Everything lands as text

No type inference. A material number like `0250161` is not the integer
250161, and a SAP date `07.02.2025` is not February's seventh day in every
locale. Bronze keeps the source's own bytes as strings and Silver does the
typing, where the rules are visible and re-runnable (see
`sap_models.py`).
"""

import csv
import io
from typing import Any

import dlt
import psycopg2
import s3fs
from dagster import Config, job, op
from dlt.destinations import filesystem
from dlt.destinations.adapters import iceberg_adapter, iceberg_partition

from dispar_orchestrate.bronze_catalog import register_bronze_table
from dispar_orchestrate.dlt_pipeline import (
    BronzeIngestConfig,
    _install_catalog_env,
    _stamp_ingested_at,
)

# A cap on rows read from one uploaded file. The API caps the file at
# 50 MB; this caps what a pathological file (one 50 MB line, a million
# tiny rows) can turn into in memory before anything is written.
MAX_ROWS = 2_000_000


class FileIngestParams(Config):
    """What to ingest, and how to read it. All of it caller-supplied.

    `storage_key` is trusted to be inside the uploads prefix because the
    API generated it (`routes::uploads::storage_key`) — it is never a name
    the user chose. This job still refuses a key outside that prefix, so a
    mistake on the caller's side cannot make it read arbitrary objects out
    of the warehouse bucket, Bronze data included.
    """

    #: Postgres `file_upload.id`, for writing the outcome back.
    upload_id: str
    #: Object key inside the warehouse bucket, e.g. `uploads/<tenant>/<id>.csv`.
    storage_key: str
    #: Bronze table to create. Validated by the API as a plain identifier.
    bronze_table_name: str
    #: `utf-8` or `utf-16`.
    encoding: str = "utf-8"
    #: Field delimiter, one character.
    delimiter: str = ","
    #: Zero-based index of the header row among the decoded lines.
    header_row: int = 0


UPLOAD_PREFIX = "uploads/"


def _read_object(cfg: BronzeIngestConfig, storage_key: str) -> bytes:
    """Fetch the uploaded object's bytes from the warehouse bucket."""
    if not storage_key.startswith(UPLOAD_PREFIX) or ".." in storage_key:
        raise ValueError(
            f"storage_key must start with {UPLOAD_PREFIX!r} and contain no '..': {storage_key!r}"
        )
    fs = s3fs.S3FileSystem(
        key=cfg.rustfs_access_key,
        secret=cfg.rustfs_secret_key,
        client_kwargs={"endpoint_url": cfg.rustfs_endpoint},
    )
    with fs.open(f"{cfg.warehouse_bucket}/{storage_key}", "rb") as handle:
        return handle.read()


def _decode(raw: bytes, encoding: str) -> str:
    """Decode with the encoding the user confirmed.

    `utf-16` handles the BOM itself; `errors="replace"` keeps one bad byte
    in a 35 000-row export from failing the whole load — the replacement
    character is visible in Bronze, where a human can see it.
    """
    codec = "utf-16" if encoding.lower().replace("_", "-") == "utf-16" else "utf-8"
    return raw.decode(codec, errors="replace")


def _column_names(header: list[str]) -> list[str]:
    """Header cells turned into stable, unique, SQL-safe column names.

    Real exports have blank header cells, duplicated labels and names like
    `Material description` or `A.scrap`. Every one of those has to become
    something a ClickHouse/Iceberg column can be called, without two
    columns colliding.
    """
    names: list[str] = []
    seen: dict[str, int] = {}
    for index, cell in enumerate(header):
        base = "".join(c.lower() if c.isalnum() else "_" for c in cell.strip()).strip("_")
        while "__" in base:
            base = base.replace("__", "_")
        if not base or base[0].isdigit():
            base = f"col_{index}" if not base else f"col_{base}"
        seen[base] = seen.get(base, 0) + 1
        names.append(base if seen[base] == 1 else f"{base}_{seen[base]}")
    return names


def parse_rows(text: str, delimiter: str, header_row: int) -> tuple[list[str], list[dict[str, str]]]:
    """Split decoded text into column names and row dicts.

    Rows shorter than the header are padded and longer ones truncated,
    rather than dropped: a ragged line in a 35 000-row export is normal,
    and losing it silently is worse than carrying it with empty cells that
    a Silver model can filter on.
    """
    reader = csv.reader(io.StringIO(text), delimiter=(delimiter or ",")[0])
    all_rows = list(reader)
    if header_row >= len(all_rows):
        raise ValueError(f"header_row {header_row} is past the end of the file ({len(all_rows)} lines)")
    columns = _column_names(all_rows[header_row])
    rows: list[dict[str, str]] = []
    for raw_row in all_rows[header_row + 1 :]:
        if not any(cell.strip() for cell in raw_row):
            continue
        padded = list(raw_row[: len(columns)]) + [""] * max(0, len(columns) - len(raw_row))
        rows.append({name: value.strip() for name, value in zip(columns, padded)})
        if len(rows) >= MAX_ROWS:
            break
    return columns, rows


def _write_bronze(cfg: BronzeIngestConfig, rows: list[dict[str, str]]) -> dict[str, Any]:
    """Load `rows` into Bronze through the same dlt/Lakekeeper path
    `run_bronze_ingest` uses for database sources."""
    _install_catalog_env(cfg)
    destination = filesystem(
        bucket_url=f"s3://{cfg.warehouse_bucket}/bronze",
        credentials={
            "aws_access_key_id": cfg.rustfs_access_key,
            "aws_secret_access_key": cfg.rustfs_secret_key,
            "endpoint_url": cfg.rustfs_endpoint,
        },
    )

    @dlt.resource(name=cfg.bronze_table_name, write_disposition="append")
    def uploaded_rows():
        yield from rows

    resource = uploaded_rows()
    resource.add_map(_stamp_ingested_at)
    iceberg_adapter(
        resource,
        partition=[iceberg_partition.day("_ingested_at")],
        table_properties={"format-version": "2"},
    )
    pipeline = dlt.pipeline(
        pipeline_name=f"file_ingest_{cfg.bronze_table_name}",
        destination=destination,
        dataset_name="bronze",
    )
    load_info = pipeline.run(resource, table_format="iceberg")
    return {"load_id": str(load_info.loads_ids[0]) if load_info.loads_ids else ""}


def _update_upload(cfg: BronzeIngestConfig, upload_id: str, status: str, error: str | None) -> None:
    """Write the outcome back to Postgres `file_upload`.

    A direct database write, not an API call, for the same reason
    `bronze_catalog` writes the ClickHouse registry directly: the job
    already holds credentials for that database, and routing this through
    HTTP would need a service identity and a token purely to report its
    own result. The cost is that this module now knows one console table's
    shape — noted here because that coupling is real, not free.
    """
    with psycopg2.connect(
        host=cfg.source_db_host,
        port=cfg.source_db_port,
        user=cfg.source_db_user,
        password=cfg.source_db_password,
        dbname=cfg.source_db_name,
    ) as conn:
        with conn.cursor() as cur:
            cur.execute(
                "UPDATE file_upload SET status = %s, error = %s, updated_at = now() "
                "WHERE id = %s",
                (status, error, upload_id),
            )


@op
def ingest_uploaded_file(context, config: FileIngestParams) -> dict:
    """Read the uploaded object, parse it, write Bronze, register it."""
    cfg = BronzeIngestConfig.from_env()
    cfg = cfg.__class__(
        **{
            **cfg.__dict__,
            "bronze_table_name": config.bronze_table_name,
            "source_schema": "upload",
            "source_table": config.upload_id,
        }
    )
    try:
        raw = _read_object(cfg, config.storage_key)
        text = _decode(raw, config.encoding)
        columns, rows = parse_rows(text, config.delimiter, config.header_row)
        if not rows:
            raise ValueError("no data rows found below the header row")
        context.log.info(
            f"parsed {len(rows)} rows x {len(columns)} columns from {config.storage_key}"
        )
        summary = _write_bronze(cfg, rows)
        register_bronze_table(
            slug=config.bronze_table_name.replace("_", "-"),
            title=config.bronze_table_name.replace("_", " ").title(),
            description=(
                f"Bronze Iceberg table ingested from uploaded file "
                f"{config.storage_key} (upload {config.upload_id})."
            ),
            bronze_table_name=config.bronze_table_name,
            row_count=len(rows),
            author="upload",
            # Every column is text at this layer — see the module doc.
            columns=[(name, "text", "") for name in columns],
        )
        _update_upload(cfg, config.upload_id, "ingested", None)
    except Exception as err:  # noqa: BLE001 - the outcome must reach the console
        # The console shows `failed` with this message; re-raising after
        # recording it keeps the Dagster run red, so the failure is visible
        # in both places rather than only one.
        _update_upload(cfg, config.upload_id, "failed", str(err)[:500])
        raise

    context.add_output_metadata(
        {
            "bronze_table": config.bronze_table_name,
            "rows": len(rows),
            "columns": len(columns),
        }
    )
    return {"bronze_table_name": config.bronze_table_name, "rows": len(rows), **summary}


@job
def file_ingest_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `file_ingest_job`. Launched
    only with a run config (`POST /api/uploads/{id}/ingest`) — it has no
    environment defaults to fall back on, because there is no such thing
    as "the" uploaded file."""
    ingest_uploaded_file()
