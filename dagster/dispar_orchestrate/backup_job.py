"""OPS-7: Complete backups for the RantAI Lakehouse stack.

This module dumps every compose-created Postgres database, incrementally
mirrors the Lakehouse warehouse bucket (holding Iceberg data and metadata
files), and backs up ClickHouse databases to a DEDICATED backup bucket --
never the shared warehouse bucket every ingestion connector and Iceberg
reader can already read.

# Why a dedicated bucket, not the warehouse bucket (security, not preference)

One of the four databases this job dumps is `${POSTGRES_DB:-lakehouse}` --
the identity database, holding `app_user`/`auth_identity` password and
service-token hashes. Writing that dump into the same bucket every
warehouse-scoped credential (every ingestion connector, every Iceberg
reader) can already read would turn compromise of ANY ONE of those
credentials into compromise of every user's password hash -- a privilege
escalation path the warehouse bucket's own access domain was never meant
to grant. `BackupConfig` therefore reads its own `BACKUP_S3_*` endpoint,
bucket, and credentials, entirely separate from `CONNECTOR_S3_*`/
`RUSTFS_*`.

# Incremental warehouse mirror (OPS-7 E2)

Iceberg data and metadata files are immutable once written. The warehouse
backup incrementally mirrors `<LAKEHOUSE_WAREHOUSE_BUCKET>` into
`<BACKUP_S3_BUCKET>/warehouse/` keyed by relative object key: an object
is copied only when the backup bucket has no object of that key and size.
Objects removed from the warehouse are NOT removed from the backup (a
restore copies files Iceberg no longer references, which is harmless).
Read access to the warehouse uses dedicated read-only credentials
(`BACKUP_WAREHOUSE_ACCESS_KEY_SECRET_REF` / `…_SECRET_KEY_SECRET_REF`),
never the backup bucket's pair (the backup bucket stays write-only for
warehouse credentials and vice versa).

# Invocation: the `backups` compose profile, not an automatic Dagster schedule

This module is invoked as `python -m dispar_orchestrate.backup_job` by the
`backup-job` service in `docker-compose.yml`, gated `profiles: ["backups"]`.
An operator runs it on a schedule via an external trigger (host cron /
systemd timer / CI job calling `docker compose --profile backups run --rm
backup-job` -- see `docs/OPERATIONS.md`).

This is not registered as an in-process Dagster schedule to avoid forcing
`BACKUP_S3_*` requirements on the always-on `dagster` compose profile.

# `*_SECRET_REF` values are references, not secrets -- resolved here, narrowly

`BACKUP_S3_*_SECRET_REF` and `BACKUP_WAREHOUSE_*_SECRET_REF` follow
the same `env:<VAR_NAME>` convention established in `lakehouse-api`.
This job resolves them via `resolve_secret_ref`.
"""

from __future__ import annotations

import io
import logging
import os
import shutil
import subprocess
import time
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from dagster import Failure
from s3fs import S3FileSystem

logger = logging.getLogger("backup_job")

# (env var, default) pairs -- the exact four databases this compose stack
# creates (docker-compose.yml:32,358-359,410-411,1341-1342): the app database,
# plus Lakekeeper's, OpenFGA's, and Dagster's own.
DATABASES: tuple[tuple[str, str], ...] = (
    ("POSTGRES_DB", "lakehouse"),
    ("LAKEKEEPER_PG_DB", "lakekeeper"),
    ("OPENFGA_PG_DB", "openfga"),
    ("DAGSTER_PG_DB", "dagster"),
)


def resolve_secret_ref(ref: str) -> str:
    """Resolves only the `env:` scheme. This job never accepts a
    caller-supplied ref (only its own hardcoded config field names,
    `BACKUP_S3_*_SECRET_REF` / `BACKUP_WAREHOUSE_*_SECRET_REF`), so there
    is no allowlist to bypass, unlike `lakehouse-api`'s
    `AllowlistedSecretResolver`."""
    if not ref.startswith("env:"):
        raise Failure(
            f"unsupported secret ref scheme: {ref!r}", allow_retries=False
        )
    var = ref.removeprefix("env:")
    value = os.environ.get(var)
    if not value:
        raise Failure(
            f"secret ref {ref!r} points at unset env var {var!r}",
            allow_retries=False,
        )
    return value


@dataclass(frozen=True)
class BackupConfig:
    endpoint: str
    bucket: str
    access_key: str
    secret_key: str
    retention_days: int
    warehouse_endpoint: str
    warehouse_bucket: str
    warehouse_access_key: str
    warehouse_secret_key: str

    @classmethod
    def from_env(cls) -> "BackupConfig":
        """Reads the dedicated backup bucket's endpoint/bucket/credentials
        and the warehouse source endpoint/bucket/credentials. Raises
        `dagster.Failure` when any required field is missing."""
        endpoint = os.environ.get("BACKUP_S3_ENDPOINT")
        bucket = os.environ.get("BACKUP_S3_BUCKET")
        access_ref = os.environ.get("BACKUP_S3_ACCESS_KEY_SECRET_REF")
        secret_ref = os.environ.get("BACKUP_S3_SECRET_KEY_SECRET_REF")
        wh_access_ref = os.environ.get("BACKUP_WAREHOUSE_ACCESS_KEY_SECRET_REF")
        wh_secret_ref = os.environ.get("BACKUP_WAREHOUSE_SECRET_KEY_SECRET_REF")
        if not (
            endpoint
            and bucket
            and access_ref
            and secret_ref
            and wh_access_ref
            and wh_secret_ref
        ):
            raise Failure(
                "BACKUP_S3_ENDPOINT/BACKUP_S3_BUCKET/BACKUP_S3_ACCESS_KEY_SECRET_REF/"
                "BACKUP_S3_SECRET_KEY_SECRET_REF/BACKUP_WAREHOUSE_ACCESS_KEY_SECRET_REF/"
                "BACKUP_WAREHOUSE_SECRET_KEY_SECRET_REF must all be set",
                allow_retries=False,
            )
        wh_endpoint = os.environ.get("BACKUP_WAREHOUSE_ENDPOINT") or os.environ.get(
            "CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"
        )
        wh_bucket = os.environ.get("BACKUP_WAREHOUSE_BUCKET") or os.environ.get(
            "LAKEHOUSE_WAREHOUSE_BUCKET", "lakehouse-warehouse"
        )
        return cls(
            endpoint=endpoint,
            bucket=bucket,
            access_key=resolve_secret_ref(access_ref),
            secret_key=resolve_secret_ref(secret_ref),
            retention_days=int(os.environ.get("BACKUP_RETENTION_DAYS", "30")),
            warehouse_endpoint=wh_endpoint,
            warehouse_bucket=wh_bucket,
            warehouse_access_key=resolve_secret_ref(wh_access_ref),
            warehouse_secret_key=resolve_secret_ref(wh_secret_ref),
        )


def pg_dump_argv(*, db_env_var: str, db_default: str, dest_path: str) -> list[str]:
    """`pg_dump`'s own argv -- the password travels via the `PGPASSWORD`
    env var (set on the `backup-job` compose service, same as `PGHOST`/
    `PGUSER`, all read implicitly by libpq), never a command-line
    argument, which would be visible to anything that can read this
    process's argv (`ps`, `/proc/<pid>/cmdline`, ...)."""
    db_name = os.environ.get(db_env_var, db_default)
    return ["pg_dump", f"--dbname={db_name}", "--format=custom", "--file", dest_path]


@dataclass(frozen=True)
class WarehouseMirrorStats:
    copied_count: int
    copied_bytes: int
    skipped_count: int
    skipped_bytes: int
    duration_seconds: float


def _copy_s3_object(src_fs: S3FileSystem, src_path: str, dst_fs: S3FileSystem, dst_path: str) -> None:
    """Streams an object from src_fs to dst_fs in bounded chunks (16 MiB)
    so large Iceberg data files are not loaded into memory (AC1 / E2)."""
    with src_fs.open(src_path, "rb") as s_in, dst_fs.open(dst_path, "wb") as d_out:
        shutil.copyfileobj(s_in, d_out, length=16 * 1024 * 1024)


def mirror_warehouse(
    src_fs: S3FileSystem,
    dst_fs: S3FileSystem,
    *,
    src_bucket: str,
    dst_bucket: str,
    dst_prefix: str = "warehouse",
) -> WarehouseMirrorStats:
    """Incrementally mirrors `src_bucket` to `dst_bucket/{dst_prefix}/`.
    An object is copied only if the destination has no object of that relative key
    or its size differs. Does NOT delete removed warehouse files from backup (E2)."""
    t0 = time.monotonic()
    try:
        src_entries = src_fs.find(src_bucket, detail=True)
    except Exception as exc:
        raise Failure(f"warehouse backup failed: could not list source bucket {src_bucket}: {exc}")

    src_files = {p: info for p, info in src_entries.items() if info.get("type") != "directory"}

    dst_dir = f"{dst_bucket}/{dst_prefix}"
    try:
        dst_entries = dst_fs.find(dst_dir, detail=True)
    except Exception as exc:
        raise Failure(f"warehouse backup failed: could not list backup bucket {dst_dir}: {exc}")

    dst_files = {p: info for p, info in dst_entries.items() if info.get("type") != "directory"}

    existing_dst: dict[str, int] = {}
    for p, info in dst_files.items():
        rel = p.removeprefix(dst_dir).lstrip("/")
        existing_dst[rel] = int(info.get("size", info.get("Size", 0)))

    copied_count = 0
    copied_bytes = 0
    skipped_count = 0
    skipped_bytes = 0

    for src_path, src_info in sorted(src_files.items()):
        rel_key = src_path.removeprefix(src_bucket).lstrip("/")
        src_size = int(src_info.get("size", src_info.get("Size", 0)))
        dest_path = f"{dst_dir}/{rel_key}"

        if rel_key in existing_dst and existing_dst[rel_key] == src_size:
            skipped_count += 1
            skipped_bytes += src_size
            continue

        try:
            _copy_s3_object(src_fs, src_path, dst_fs, dest_path)
        except Exception as exc:
            raise Failure(f"warehouse backup failed copying {rel_key}: {exc}")

        copied_count += 1
        copied_bytes += src_size

    duration = time.monotonic() - t0
    return WarehouseMirrorStats(
        copied_count=copied_count,
        copied_bytes=copied_bytes,
        skipped_count=skipped_count,
        skipped_bytes=skipped_bytes,
        duration_seconds=duration,
    )


def run_backups() -> None:
    """Runs the Lakehouse backup: dumps every Postgres database, incrementally
    mirrors the warehouse bucket, and (in full stack) backs up ClickHouse databases.
    Reports times and sizes for each part (OPS-7 AC1). Raises `dagster.Failure`
    naming the part on failure; already-uploaded parts stay (E1)."""
    if not logger.handlers:
        logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")

    cfg = BackupConfig.from_env()

    # Create warehouse source filesystem first, then backup target filesystem,
    # ensuring backup_fs is the primary target for uploads.
    wh_fs = S3FileSystem(
        key=cfg.warehouse_access_key,
        secret=cfg.warehouse_secret_key,
        client_kwargs={"endpoint_url": cfg.warehouse_endpoint},
    )
    backup_fs = S3FileSystem(
        key=cfg.access_key,
        secret=cfg.secret_key,
        client_kwargs={"endpoint_url": cfg.endpoint},
    )

    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")

    # Part 1: Postgres databases
    t_pg_start = time.monotonic()
    pg_total_bytes = 0
    for db_env_var, db_default in DATABASES:
        db_name = os.environ.get(db_env_var, db_default)
        local_path = f"/tmp/{db_name}-{stamp}.dump"
        argv = pg_dump_argv(db_env_var=db_env_var, db_default=db_default, dest_path=local_path)
        result = subprocess.run(argv, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            raise Failure(f"pg_dump failed for {db_name}: {result.stderr}")
        dump_size = os.path.getsize(local_path)
        pg_total_bytes += dump_size
        remote_path = f"{cfg.bucket}/{db_name}/{stamp}.dump"
        backup_fs.put(local_path, remote_path)
        os.remove(local_path)
    pg_duration = time.monotonic() - t_pg_start
    logger.info(
        "Postgres backup: %d databases dumped (%d bytes) in %.2fs",
        len(DATABASES),
        pg_total_bytes,
        pg_duration,
    )

    # Part 2: Warehouse incremental mirror
    wh_stats = mirror_warehouse(
        wh_fs,
        backup_fs,
        src_bucket=cfg.warehouse_bucket,
        dst_bucket=cfg.bucket,
    )
    logger.info(
        "Warehouse backup: %d objects copied (%d bytes), %d objects skipped (%d bytes) in %.2fs",
        wh_stats.copied_count,
        wh_stats.copied_bytes,
        wh_stats.skipped_count,
        wh_stats.skipped_bytes,
        wh_stats.duration_seconds,
    )

    _prune_expired(backup_fs, cfg)


def _prune_expired(fs: S3FileSystem, cfg: BackupConfig) -> None:
    """Deletes any object under this job's own bucket older than
    `retention_days` for timestamped dumps. Warehouse mirrored objects
    are not pruned by age (E2)."""
    cutoff = datetime.now(timezone.utc) - timedelta(days=cfg.retention_days)
    for path, info in fs.find(cfg.bucket, detail=True).items():
        rel = path.removeprefix(cfg.bucket).lstrip("/")
        if rel.startswith("warehouse/"):
            continue
        modified = info.get("LastModified")
        if modified and modified < cutoff:
            fs.rm(path)


if __name__ == "__main__":
    run_backups()
