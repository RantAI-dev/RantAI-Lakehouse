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
import requests
from dagster import Failure

from dispar_orchestrate.backup_job import (
    DATABASES,
    BackupConfig,
    ClickHouseBackupStats,
    WarehouseMirrorStats,
    backup_clickhouse,
    build_clickhouse_backup_sql,
    filter_clickhouse_databases,
    mirror_warehouse,
    pg_dump_argv,
    quote_clickhouse_identifier,
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
    # The ClickHouse half of the config is `bronze_catalog.ClickHouseTarget`'s
    # own env convention (`CH_URL`/`CH_USER`/`CH_PASSWORD`), left at its
    # defaults here; a deployment overrides them per service.
    assert cfg.ch.url == "http://clickhouse:8123"
    assert cfg.ch.user == "default"
    assert cfg.ch.password == ""
    assert cfg.ch_timeout_seconds == 300


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
    monkeypatch.setattr(
        "dispar_orchestrate.backup_job.backup_clickhouse",
        lambda cfg, stamp, backup_fs: ClickHouseBackupStats(
            databases=["default"],
            object_count=3,
            size_bytes=1024,
            duration_seconds=0.01,
        ),
    )
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


# ---------------------------------------------------------------------------
# ClickHouse backup unit tests (OPS-7 T2)
# ---------------------------------------------------------------------------


def test_quote_clickhouse_identifier_doubles_a_backtick_inside_the_name():
    assert quote_clickhouse_identifier("serving") == "`serving`"
    assert quote_clickhouse_identifier("db`with`ticks") == "`db``with``ticks`"


def test_filter_clickhouse_databases_excludes_the_servers_own_and_the_catalog_views():
    entries = [
        ("system", "Atomic"),
        ("INFORMATION_SCHEMA", "Memory"),
        ("information_schema", "Memory"),
        ("icecat_api", "DataLakeCatalog"),
        ("bronze_meta", "Atomic"),
        ("default", "Atomic"),
    ]
    assert filter_clickhouse_databases(entries) == ["bronze_meta", "default"]


def test_the_backup_statement_is_built_from_the_config_endpoint_bucket_and_stamp():
    """The statement this job sends is derived only from its own config and
    the database name `system.databases` returned -- never from a caller --
    and the credentials travel inside it, which is why E3 requires the
    query-log masking measured on 26.8."""
    sql = build_clickhouse_backup_sql(
        "bronze_meta",
        endpoint="http://backup-s3:9000",
        bucket="dedicated-backups",
        timestamp="20261015T120000Z",
        access_key="my_ak",
        secret_key="my_sk",
    )
    assert sql == (
        "BACKUP DATABASE `bronze_meta` TO S3("
        "'http://backup-s3:9000/dedicated-backups/clickhouse/20261015T120000Z/bronze_meta/', "
        "'my_ak', 'my_sk')"
    )


def test_the_backup_statement_renders_every_string_as_an_escaped_sql_literal():
    """A credential carrying a quote or a backslash must not be able to
    end its own string literal and rewrite the statement (`_sql_string_literal`
    is reused from `bronze_catalog.py`, not re-invented)."""
    sql = build_clickhouse_backup_sql(
        "weird`db",
        endpoint="http://backup-s3:9000/",
        bucket="/dedicated-backups/",
        timestamp="20261015T120000Z",
        access_key="ak'xk",
        secret_key="sk\\xk",
    )
    assert sql == (
        "BACKUP DATABASE `weird``db` TO S3("
        "'http://backup-s3:9000/dedicated-backups/clickhouse/20261015T120000Z/weird%60db/', "
        "'ak\\'xk', 'sk\\\\xk')"
    )


def _fake_response(status_code: int, text: str) -> requests.Response:
    """A REAL `requests.Response`, so `raise_for_status()` behaves exactly
    as the code under test expects it to."""
    resp = requests.Response()
    resp.status_code = status_code
    resp._content = text.encode("utf-8")
    return resp


def test_clickhouse_backup_backs_up_each_eligible_database_and_reports_the_size_it_wrote(
    monkeypatch,
):
    _set_backup_env(monkeypatch)
    cfg = BackupConfig.from_env()

    calls = []

    def _fake_post(url, auth, data, timeout):
        body = data.decode("utf-8")
        calls.append({"url": url, "auth": auth, "data": body, "timeout": timeout})
        if "FROM system.databases" in body:
            return _fake_response(200, "console\tAtomic\nsystem\tAtomic\ndefault\tAtomic\n")
        return _fake_response(200, "")

    backup_fs = _FakeS3FileSystem()
    backup_fs.find_result = {
        "lakehouse-backups/clickhouse/20261015T120000Z/console/.backup": {
            "size": 40,
            "type": "file",
        },
        "lakehouse-backups/clickhouse/20261015T120000Z/default/data.bin": {
            "size": 250,
            "type": "file",
        },
        "lakehouse-backups/clickhouse/20261014T120000Z/lakehouse.dump": {
            "size": 9999,
            "type": "file",
        },
    }

    stats = backup_clickhouse(
        cfg, "20261015T120000Z", backup_fs, post_fn=_fake_post
    )

    assert stats.databases == ["console", "default"]
    assert calls[0]["auth"] == ("default", "")
    assert calls[0]["timeout"] == 300
    assert "BACKUP DATABASE `console`" in calls[1]["data"]
    assert "BACKUP DATABASE `default`" in calls[2]["data"]
    assert "clickhouse/20261015T120000Z/console/" in calls[1]["data"]
    # Only this run's prefix is measured, not the whole bucket.
    assert stats.object_count == 2
    assert stats.size_bytes == 290
    assert stats.duration_seconds >= 0.0


def test_clickhouse_backup_skips_the_backup_statements_when_no_database_is_eligible(
    monkeypatch,
):
    _set_backup_env(monkeypatch)
    cfg = BackupConfig.from_env()

    calls = []

    def _fake_post(url, auth, data, timeout):
        calls.append(data.decode("utf-8"))
        return _fake_response(200, "system\tAtomic\nINFORMATION_SCHEMA\tMemory\n")

    backup_fs = _FakeS3FileSystem()
    backup_fs.find_result = {
        "lakehouse-backups/clickhouse/20261015T120000Z/default/.backup": {
            "size": 10,
            "type": "file",
        }
    }
    stats = backup_clickhouse(cfg, "20261015T120000Z", backup_fs, post_fn=_fake_post)

    assert stats.databases == []
    assert len(calls) == 1


def test_clickhouse_backup_refuses_to_report_a_size_when_the_backup_wrote_nothing(
    monkeypatch,
):
    """A `BACKUP` that answered 200 and left the bucket empty is a broken
    backup; reporting zero bytes would hide it (never fabricate)."""
    _set_backup_env(monkeypatch)
    cfg = BackupConfig.from_env()

    def _fake_post(url, auth, data, timeout):
        if "FROM system.databases" in data.decode("utf-8"):
            return _fake_response(200, "default\tAtomic\n")
        return _fake_response(200, "")

    with pytest.raises(Failure, match="ClickHouse backup wrote nothing"):
        backup_clickhouse(
            cfg, "20261015T120000Z", _FakeS3FileSystem(), post_fn=_fake_post
        )


def test_clickhouse_backup_reports_the_status_and_never_the_body_when_the_listing_fails(
    monkeypatch,
):
    _set_backup_env(monkeypatch)
    cfg = BackupConfig.from_env()

    def _fake_post(url, auth, data, timeout):
        return _fake_response(500, "Code: 499 ... ACCESS_DENIED ... 'drillsk1234'")

    with pytest.raises(Failure) as exc:
        backup_clickhouse(
            cfg, "20261015T120000Z", _FakeS3FileSystem(), post_fn=_fake_post
        )

    assert "database listing" in str(exc.value)
    assert "HTTP status 500" in str(exc.value)
    assert "ACCESS_DENIED" not in str(exc.value), "upstream body must not be logged"


def test_clickhouse_backup_reports_the_status_and_never_the_body_when_a_backup_fails(
    monkeypatch,
):
    _set_backup_env(monkeypatch)
    cfg = BackupConfig.from_env()

    def _fake_post(url, auth, data, timeout):
        if "FROM system.databases" in data.decode("utf-8"):
            return _fake_response(200, "default\tAtomic\n")
        return _fake_response(403, "S3 url: 'http://...', 'my_ak', 'my_sk'")

    with pytest.raises(Failure) as exc:
        backup_clickhouse(
            cfg, "20261015T120000Z", _FakeS3FileSystem(), post_fn=_fake_post
        )

    assert "backup of database default" in str(exc.value)
    assert "HTTP status 403" in str(exc.value)
    assert "my_ak" not in str(exc.value), "a failed BACKUP echoes its credentials"


def test_clickhouse_backup_reports_a_transport_error_by_exception_type_only(monkeypatch):
    _set_backup_env(monkeypatch)
    cfg = BackupConfig.from_env()

    def _fake_post(url, auth, data, timeout):
        raise requests.ConnectionError("dial tcp clickhouse:8123: no route to host")

    with pytest.raises(Failure) as exc:
        backup_clickhouse(
            cfg, "20261015T120000Z", _FakeS3FileSystem(), post_fn=_fake_post
        )

    assert "database listing" in str(exc.value)
    assert "ConnectionError" in str(exc.value)
    assert "no route to host" not in str(exc.value)
