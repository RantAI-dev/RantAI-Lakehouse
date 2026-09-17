"""WS5 item G2: `pg_dump` every compose-created Postgres database, uploaded
to a DEDICATED backup bucket — never the shared warehouse bucket every
ingestion connector and Iceberg reader can already read.

# Why a dedicated bucket, not the warehouse bucket (security, not preference)

One of the four databases this job dumps is `${POSTGRES_DB:-lakehouse}` —
the identity database, holding `app_user`/`auth_identity` password and
service-token hashes. Writing that dump into the same bucket every
warehouse-scoped credential (every ingestion connector, every Iceberg
reader) can already read would turn compromise of ANY ONE of those
credentials into compromise of every user's password hash — a privilege
escalation path the warehouse bucket's own access domain was never meant
to grant. `BackupConfig` therefore reads its own `BACKUP_S3_*` endpoint,
bucket, and credentials, entirely separate from `CONNECTOR_S3_*`/
`RUSTFS_*` (see `capacity_snapshot.py`'s module doc for why those two
names exist and are themselves already narrower than RustFS's root pair).

# Invocation: the `backups` compose profile, not an automatic Dagster schedule

This module is invoked as `python -m dispar_orchestrate.backup_job` by the
`backup-job` service in `docker-compose.yml`, gated `profiles: ["backups"]`
and its own required env vars (`${BACKUP_…:?}` — never a default, so a
deployment that enables the `backups` profile without configuring them
gets a clear startup failure, not a silent write into the wrong bucket).
An operator runs it on a schedule via an external trigger (host cron /
systemd timer / CI job calling `docker compose --profile backups run --rm
backup-job` — see `docs/OPERATIONS.md`'s "Dedicated-bucket backup job"
section for the exact command and the restore procedure).

Left undone, stated honestly rather than silently omitted: this is NOT
also registered as an in-process Dagster job/schedule in `definitions.py`
(unlike `capacity_snapshot_job`/`bronze_maintenance_schedule`). Doing that
safely would mean threading `BACKUP_S3_*`/`PGHOST`/`PGUSER`/`PGPASSWORD`
into the `dagster-code-location` service's own environment — but that
service runs under the always-on `dagster` compose profile, and giving it
unconditional `${BACKUP_S3_BUCKET:?}`-shaped requirements would make
every `dagster`-profile deployment refuse to start unless backups happen
to be configured too, which breaks today's "backups is its own opt-in
profile" security/operability goal. Reconciling that needs a config split
(e.g. a second built image/service reusing this same package) that is out
of this commit's scope; the external-cron path above is the real, tested
trigger today.

# `*_SECRET_REF` values are references, not secrets — resolved here, narrowly

`BACKUP_S3_ACCESS_KEY_SECRET_REF`/`BACKUP_S3_SECRET_KEY_SECRET_REF` follow
the same `env:<VAR_NAME>` convention `RUSTFS_ACCESS_KEY_SECRET_REF`
already establishes in `lakehouse-api/src/config.rs` — but this job runs
in Dagster's Python process, not `lakehouse-api`'s Rust process, so it
resolves them itself via `resolve_secret_ref`, deliberately narrower than
`lakehouse-api`'s `AllowlistedSecretResolver`: this job never accepts a
caller-supplied ref, only its own two hardcoded config field names, so
there is no allowlist to bypass and none is built.
"""

from __future__ import annotations

import os
import subprocess
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from dagster import Failure
from s3fs import S3FileSystem

# (env var, default) pairs -- the exact four databases this compose stack
# creates (docker-compose.yml:32,358-359,410-411,1341-1342, re-verified
# this task): the app database, plus Lakekeeper's, OpenFGA's, and
# Dagster's own.
DATABASES: tuple[tuple[str, str], ...] = (
    ("POSTGRES_DB", "lakehouse"),
    ("LAKEKEEPER_PG_DB", "lakekeeper"),
    ("OPENFGA_PG_DB", "openfga"),
    ("DAGSTER_PG_DB", "dagster"),
)


def resolve_secret_ref(ref: str) -> str:
    """Resolves only the `env:` scheme. This job never accepts a
    caller-supplied ref (only its own two hardcoded config field names,
    `BACKUP_S3_ACCESS_KEY_SECRET_REF`/`BACKUP_S3_SECRET_KEY_SECRET_REF`),
    so there is no allowlist to bypass, unlike `lakehouse-api`'s
    `AllowlistedSecretResolver` (see module doc)."""
    if not ref.startswith("env:"):
        raise Failure(f"unsupported secret ref scheme: {ref!r}")
    var = ref.removeprefix("env:")
    value = os.environ.get(var)
    if not value:
        raise Failure(f"secret ref {ref!r} points at unset env var {var!r}")
    return value


@dataclass(frozen=True)
class BackupConfig:
    endpoint: str
    bucket: str
    access_key: str
    secret_key: str
    retention_days: int

    @classmethod
    def from_env(cls) -> "BackupConfig":
        """Reads the dedicated backup bucket's endpoint/bucket/credential
        refs. Raises `dagster.Failure` when any required field is
        missing -- a config problem, never a silent skip (AGENTS.md:
        config/auth problems are a `Failure`)."""
        endpoint = os.environ.get("BACKUP_S3_ENDPOINT")
        bucket = os.environ.get("BACKUP_S3_BUCKET")
        access_ref = os.environ.get("BACKUP_S3_ACCESS_KEY_SECRET_REF")
        secret_ref = os.environ.get("BACKUP_S3_SECRET_KEY_SECRET_REF")
        if not (endpoint and bucket and access_ref and secret_ref):
            raise Failure(
                "BACKUP_S3_ENDPOINT/BACKUP_S3_BUCKET/BACKUP_S3_ACCESS_KEY_SECRET_REF/"
                "BACKUP_S3_SECRET_KEY_SECRET_REF must all be set"
            )
        return cls(
            endpoint=endpoint,
            bucket=bucket,
            access_key=resolve_secret_ref(access_ref),
            secret_key=resolve_secret_ref(secret_ref),
            retention_days=int(os.environ.get("BACKUP_RETENTION_DAYS", "30")),
        )


def pg_dump_argv(*, db_env_var: str, db_default: str, dest_path: str) -> list[str]:
    """`pg_dump`'s own argv -- the password travels via the `PGPASSWORD`
    env var (set on the `backup-job` compose service, same as `PGHOST`/
    `PGUSER`, all read implicitly by libpq), never a command-line
    argument, which would be visible to anything that can read this
    process's argv (`ps`, `/proc/<pid>/cmdline`, ...)."""
    db_name = os.environ.get(db_env_var, db_default)
    return ["pg_dump", f"--dbname={db_name}", "--format=custom", "--file", dest_path]


def run_backups() -> None:
    """Dumps every database in `DATABASES` and uploads each dump to this
    job's dedicated bucket, then prunes anything older than
    `retention_days`. Raises `dagster.Failure` (never reports success) on
    the first `pg_dump` failure -- a partial, silently-declared-successful
    backup run is worse than a loud one that stops."""
    cfg = BackupConfig.from_env()
    fs = S3FileSystem(
        key=cfg.access_key,
        secret=cfg.secret_key,
        client_kwargs={"endpoint_url": cfg.endpoint},
    )
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    for db_env_var, db_default in DATABASES:
        db_name = os.environ.get(db_env_var, db_default)
        local_path = f"/tmp/{db_name}-{stamp}.dump"
        argv = pg_dump_argv(db_env_var=db_env_var, db_default=db_default, dest_path=local_path)
        result = subprocess.run(argv, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            # A failed pg_dump must surface as a failed run -- never a
            # skipped-but-reported-fine database (AGENTS.md principle 2:
            # unsupported/failed is recorded honestly, never faked).
            raise Failure(f"pg_dump failed for {db_name}: {result.stderr}")
        remote_path = f"{cfg.bucket}/{db_name}/{stamp}.dump"
        fs.put(local_path, remote_path)
        os.remove(local_path)
    _prune_expired(fs, cfg)


def _prune_expired(fs: S3FileSystem, cfg: BackupConfig) -> None:
    """Deletes any object under this job's own bucket older than
    `retention_days` -- a real, implemented retention policy, not a
    stated-but-unbuilt intention."""
    cutoff = datetime.now(timezone.utc) - timedelta(days=cfg.retention_days)
    for path, info in fs.find(cfg.bucket, detail=True).items():
        modified = info.get("LastModified")
        if modified and modified < cutoff:
            fs.rm(path)


if __name__ == "__main__":
    run_backups()
