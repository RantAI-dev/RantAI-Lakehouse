"""Unit tests for `dagster/dispar_orchestrate/backup_job.py` (OPS-7).

No network: `S3FileSystem` and `subprocess.run` are monkeypatched with
fakes, matching `test_capacity_snapshot.py`'s own no-network testing rule
for this package.

Run with:
`~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_backup_job.py -q`
"""

from __future__ import annotations

import io
from datetime import datetime, timedelta, timezone

import pytest
from dagster import Failure

from dispar_orchestrate.backup_job import (
    DATABASES,
    BackupConfig,
    WarehouseMirrorStats,
    mirror_warehouse,
    pg_dump_argv,
    resolve_secret_ref,
    run_backups,
)


def _set_backup_env(monkeypatch) -> None:
    monkeypatch.setenv("BACKUP_S3_ENDPOINT", "http://backup-store.internal:9000")
    monkeypatch.setenv("BACKUP_S3_BUCKET", "lakehouse-backups")
    monkeypatch.setenv("BACKUP_S3_ACCESS_KEY_SECRET_REF", "env:BACKUP_ACCESS_KEY")
    monkeypatch.setenv("BACKUP_S3_SECRET_KEY_SECRET_REF", "env:BACKUP_SECRET_KEY")
    monkeypatch.setenv("BACKUP_ACCESS_KEY", "ak")
    monkeypatch.setenv("BACKUP_SECRET_KEY", "sk")
    monkeypatch.setenv(
        "BACKUP_WAREHOUSE_ACCESS_KEY_SECRET_REF", "env:BACKUP_WH_ACCESS_KEY"
    )
    monkeypatch.setenv(
        "BACKUP_WAREHOUSE_SECRET_KEY_SECRET_REF", "env:BACKUP_WH_SECRET_KEY"
    )
    monkeypatch.setenv("BACKUP_WH_ACCESS_KEY", "wak")
    monkeypatch.setenv("BACKUP_WH_SECRET_KEY", "wsk")


def test_backup_config_reads_all_required_fields_from_env(monkeypatch):
    _set_backup_env(monkeypatch)
    cfg = BackupConfig.from_env()
    assert cfg.bucket == "lakehouse-backups"
    assert cfg.endpoint == "http://backup-store.internal:9000"
    assert cfg.access_key == "ak"
    assert cfg.secret_key == "sk"
    assert cfg.retention_days == 30  # unset -- the one field allowed a default
    assert cfg.warehouse_endpoint == "http://rustfs:9000"
    assert cfg.warehouse_bucket == "lakehouse-warehouse"
    assert cfg.warehouse_access_key == "wak"
    assert cfg.warehouse_secret_key == "wsk"


def test_backup_config_honors_a_caller_supplied_retention_days(monkeypatch):
    _set_backup_env(monkeypatch)
    monkeypatch.setenv("BACKUP_RETENTION_DAYS", "7")
    assert BackupConfig.from_env().retention_days == 7


@pytest.mark.parametrize(
    "env_var",
    [
        "BACKUP_S3_ENDPOINT",
        "BACKUP_S3_BUCKET",
        "BACKUP_S3_ACCESS_KEY_SECRET_REF",
        "BACKUP_S3_SECRET_KEY_SECRET_REF",
        "BACKUP_WAREHOUSE_ACCESS_KEY_SECRET_REF",
        "BACKUP_WAREHOUSE_SECRET_KEY_SECRET_REF",
    ],
)
def test_backup_config_raises_failure_when_a_required_field_is_missing(
    monkeypatch, env_var
):
    _set_backup_env(monkeypatch)
    monkeypatch.delenv(env_var, raising=False)
    with pytest.raises(Failure, match="must all be set"):
        BackupConfig.from_env()


def test_resolve_secret_ref_rejects_anything_but_the_env_scheme():
    with pytest.raises(Failure):
        resolve_secret_ref("vault:some/path")


def test_resolve_secret_ref_config_rejections_mark_the_failure_non_retryable():
    with pytest.raises(Failure) as ctx:
        resolve_secret_ref("vault:some/path")
    assert ctx.value.allow_retries is False


def test_backup_config_from_env_missing_required_envs_marks_the_failure_non_retryable(
    monkeypatch,
):
    monkeypatch.delenv("BACKUP_S3_ENDPOINT", raising=False)
    monkeypatch.delenv("BACKUP_S3_BUCKET", raising=False)
    monkeypatch.delenv("BACKUP_S3_ACCESS_KEY_SECRET_REF", raising=False)
    monkeypatch.delenv("BACKUP_S3_SECRET_KEY_SECRET_REF", raising=False)
    monkeypatch.delenv("BACKUP_WAREHOUSE_ACCESS_KEY_SECRET_REF", raising=False)
    monkeypatch.delenv("BACKUP_WAREHOUSE_SECRET_KEY_SECRET_REF", raising=False)
    with pytest.raises(Failure, match="must all be set") as ctx:
        BackupConfig.from_env()
    assert ctx.value.allow_retries is False


def test_resolve_secret_ref_rejects_an_unset_target_env_var(monkeypatch):
    monkeypatch.delenv("BACKUP_MISSING", raising=False)
    with pytest.raises(Failure):
        resolve_secret_ref("env:BACKUP_MISSING")


def test_resolve_secret_ref_reads_the_referenced_env_var(monkeypatch):
    monkeypatch.setenv("BACKUP_SOME_VAR", "the-value")
    assert resolve_secret_ref("env:BACKUP_SOME_VAR") == "the-value"


def test_databases_lists_exactly_the_four_compose_created_databases():
    assert DATABASES == (
        ("POSTGRES_DB", "lakehouse"),
        ("LAKEKEEPER_PG_DB", "lakekeeper"),
        ("OPENFGA_PG_DB", "openfga"),
        ("DAGSTER_PG_DB", "dagster"),
    )


@pytest.mark.parametrize("db_env,db_default", DATABASES)
def test_pg_dump_argv_never_embeds_the_password_on_the_command_line(
    db_env, db_default, monkeypatch
):
    monkeypatch.setenv(db_env, db_default)
    argv = pg_dump_argv(
        db_env_var=db_env, db_default=db_default, dest_path="/tmp/out.dump"
    )
    assert "--format=custom" in argv
    assert not any(
        "PGPASSWORD" in a for a in argv
    ), "password must travel via PGPASSWORD env, never argv"
    assert f"--dbname={db_default}" in argv


def test_pg_dump_argv_uses_the_env_override_over_the_default(monkeypatch):
    monkeypatch.setenv("POSTGRES_DB", "renamed-app-db")
    argv = pg_dump_argv(
        db_env_var="POSTGRES_DB", db_default="lakehouse", dest_path="/tmp/out.dump"
    )
    assert "--dbname=renamed-app-db" in argv


class _FakeCompletedProcess:
    def __init__(self, returncode: int, stderr: str = "") -> None:
        self.returncode = returncode
        self.stderr = stderr


class _FakeS3FileSystem:
    """Records every `put`/`rm`/`open`; `find` returns a fixed, caller-set
    listing so retention pruning and mirroring can be exercised without a real bucket."""

    instances: list["_FakeS3FileSystem"] = []

    def __init__(self, **kwargs):
        self.kwargs = kwargs
        self.put_calls: list[tuple[str, str]] = []
        self.rm_calls: list[str] = []
        self.find_result: dict = {}
        self.files: dict[str, bytes] = {}
        self.open_calls: list[tuple[str, str]] = []
        _FakeS3FileSystem.instances.append(self)

    def put(self, local_path, remote_path):
        self.put_calls.append((local_path, remote_path))

    def find(self, path, detail=True):
        assert detail is True
        if not path:
            return self.find_result
        # Match exact path or prefix matches under path
        prefix = path.rstrip("/") + "/"
        return {
            k: v
            for k, v in self.find_result.items()
            if k == path or k.startswith(prefix)
        }

    def rm(self, path):
        self.rm_calls.append(path)

    def open(self, path: str, mode: str = "rb"):
        self.open_calls.append((path, mode))
        if "w" in mode:
            storage = self.files

            class _Writer(io.BytesIO):
                def __exit__(writer_self, exc_type, exc_val, exc_tb):
                    storage[path] = writer_self.getvalue()
                    return super().__exit__(exc_type, exc_val, exc_tb)

                def close(writer_self):
                    storage[path] = writer_self.getvalue()
                    super().close()

            return _Writer()
        data = self.files.get(path, b"")
        return io.BytesIO(data)


def test_warehouse_mirror_copies_a_new_object():
    src_fs = _FakeS3FileSystem()
    dst_fs = _FakeS3FileSystem()

    src_fs.find_result = {
        "lakehouse-warehouse/data/001.parquet": {"size": 1024, "type": "file"}
    }
    src_fs.files["lakehouse-warehouse/data/001.parquet"] = b"parquet-bytes-001"
    dst_fs.find_result = {}

    stats = mirror_warehouse(
        src_fs, dst_fs, src_bucket="lakehouse-warehouse", dst_bucket="lakehouse-backups"
    )

    assert stats.copied_count == 1
    assert stats.copied_bytes == 1024
    assert stats.skipped_count == 0
    assert stats.skipped_bytes == 0
    assert "lakehouse-backups/warehouse/data/001.parquet" in dst_fs.files
    assert dst_fs.files["lakehouse-backups/warehouse/data/001.parquet"] == b"parquet-bytes-001"


def test_warehouse_mirror_skips_one_already_there_with_the_same_size():
    src_fs = _FakeS3FileSystem()
    dst_fs = _FakeS3FileSystem()

    src_fs.find_result = {
        "lakehouse-warehouse/data/001.parquet": {"size": 1024, "type": "file"}
    }
    src_fs.files["lakehouse-warehouse/data/001.parquet"] = b"parquet-bytes-001"
    dst_fs.find_result = {
        "lakehouse-backups/warehouse/data/001.parquet": {"size": 1024, "type": "file"}
    }

    stats = mirror_warehouse(
        src_fs, dst_fs, src_bucket="lakehouse-warehouse", dst_bucket="lakehouse-backups"
    )

    assert stats.copied_count == 0
    assert stats.copied_bytes == 0
    assert stats.skipped_count == 1
    assert stats.skipped_bytes == 1024
    assert not any("w" in mode for _, mode in dst_fs.open_calls)


def test_warehouse_mirror_copies_one_whose_size_differs():
    src_fs = _FakeS3FileSystem()
    dst_fs = _FakeS3FileSystem()

    src_fs.find_result = {
        "lakehouse-warehouse/metadata/v2.metadata.json": {"size": 2048, "type": "file"}
    }
    src_fs.files["lakehouse-warehouse/metadata/v2.metadata.json"] = b"new-metadata-2048"
    dst_fs.find_result = {
        "lakehouse-backups/warehouse/metadata/v2.metadata.json": {"size": 1024, "type": "file"}
    }
    dst_fs.files["lakehouse-backups/warehouse/metadata/v2.metadata.json"] = b"old-metadata-1024"

    stats = mirror_warehouse(
        src_fs, dst_fs, src_bucket="lakehouse-warehouse", dst_bucket="lakehouse-backups"
    )

    assert stats.copied_count == 1
    assert stats.copied_bytes == 2048
    assert stats.skipped_count == 0
    assert (
        dst_fs.files["lakehouse-backups/warehouse/metadata/v2.metadata.json"]
        == b"new-metadata-2048"
    )


def test_warehouse_mirror_reports_count_and_bytes():
    src_fs = _FakeS3FileSystem()
    dst_fs = _FakeS3FileSystem()

    src_fs.find_result = {
        "lakehouse-warehouse/data/new.parquet": {"size": 500, "type": "file"},
        "lakehouse-warehouse/data/same.parquet": {"size": 1000, "type": "file"},
        "lakehouse-warehouse/data/changed.parquet": {"size": 2500, "type": "file"},
        "lakehouse-warehouse/dir/": {"size": 0, "type": "directory"},
    }
    src_fs.files["lakehouse-warehouse/data/new.parquet"] = b"new-data"
    src_fs.files["lakehouse-warehouse/data/same.parquet"] = b"same-data"
    src_fs.files["lakehouse-warehouse/data/changed.parquet"] = b"changed-data-2500"

    dst_fs.find_result = {
        "lakehouse-backups/warehouse/data/same.parquet": {"size": 1000, "type": "file"},
        "lakehouse-backups/warehouse/data/changed.parquet": {"size": 1200, "type": "file"},
    }

    stats = mirror_warehouse(
        src_fs, dst_fs, src_bucket="lakehouse-warehouse", dst_bucket="lakehouse-backups"
    )

    assert isinstance(stats, WarehouseMirrorStats)
    assert stats.copied_count == 2
    assert stats.copied_bytes == 3000  # 500 + 2500
    assert stats.skipped_count == 1
    assert stats.skipped_bytes == 1000
    assert stats.duration_seconds >= 0.0


def test_warehouse_mirror_raises_failure_when_source_listing_fails():
    src_fs = _FakeS3FileSystem()
    dst_fs = _FakeS3FileSystem()

    def _fail_find(*a, **k):
        raise ConnectionError("s3 unavailable")

    src_fs.find = _fail_find
    with pytest.raises(Failure, match="warehouse backup failed: could not list source bucket"):
        mirror_warehouse(
            src_fs, dst_fs, src_bucket="lakehouse-warehouse", dst_bucket="lakehouse-backups"
        )


def test_run_backups_dumps_every_database_and_uploads_to_the_dedicated_bucket(
    monkeypatch, tmp_path
):
    _set_backup_env(monkeypatch)
    _FakeS3FileSystem.instances.clear()
    monkeypatch.setattr("dispar_orchestrate.backup_job.S3FileSystem", _FakeS3FileSystem)

    def _fake_run(argv, capture_output, text, check):
        dest = argv[argv.index("--file") + 1]
        with open(dest, "wb") as f:
            f.write(b"fake-dump")
        return _FakeCompletedProcess(returncode=0)

    monkeypatch.setattr("dispar_orchestrate.backup_job.subprocess.run", _fake_run)
    monkeypatch.chdir(tmp_path)

    run_backups()

    fake_fs = _FakeS3FileSystem.instances[-1]
    assert fake_fs.kwargs["key"] == "ak"
    assert fake_fs.kwargs["secret"] == "sk"
    assert (
        fake_fs.kwargs["client_kwargs"]["endpoint_url"]
        == "http://backup-store.internal:9000"
    )
    assert len(fake_fs.put_calls) == len(DATABASES)
    for (_, remote_path), (db_env, db_default) in zip(fake_fs.put_calls, DATABASES):
        db_name = db_default
        assert remote_path.startswith(f"lakehouse-backups/{db_name}/")


def test_run_backups_raises_failure_and_stops_on_the_first_pg_dump_failure(monkeypatch):
    _set_backup_env(monkeypatch)
    _FakeS3FileSystem.instances.clear()
    monkeypatch.setattr("dispar_orchestrate.backup_job.S3FileSystem", _FakeS3FileSystem)
    monkeypatch.setattr(
        "dispar_orchestrate.backup_job.subprocess.run",
        lambda *a, **k: _FakeCompletedProcess(returncode=1, stderr="connection refused"),
    )

    with pytest.raises(Failure, match="pg_dump failed"):
        run_backups()

    assert not _FakeS3FileSystem.instances[-1].put_calls


def test_prune_expired_deletes_only_objects_older_than_retention_days(monkeypatch):
    from dispar_orchestrate.backup_job import _prune_expired

    _set_backup_env(monkeypatch)
    monkeypatch.setenv("BACKUP_RETENTION_DAYS", "30")
    cfg = BackupConfig.from_env()

    now = datetime.now(timezone.utc)
    fs = _FakeS3FileSystem()
    fs.find_result = {
        "lakehouse-backups/lakehouse/old.dump": {"LastModified": now - timedelta(days=40)},
        "lakehouse-backups/lakehouse/recent.dump": {"LastModified": now - timedelta(days=1)},
        "lakehouse-backups/warehouse/old.parquet": {"LastModified": now - timedelta(days=40)},
    }

    _prune_expired(fs, cfg)

    assert fs.rm_calls == ["lakehouse-backups/lakehouse/old.dump"]
