"""Unit tests for `dagster/dispar_orchestrate/backup_job.py` (WS5 item G2).

No network: `S3FileSystem` and `subprocess.run` are monkeypatched with
fakes, matching `test_capacity_snapshot.py`'s own no-network testing rule
for this package.

Run with:
`~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_backup_job.py -q`
"""

from __future__ import annotations

import pytest
from dagster import Failure

from dispar_orchestrate.backup_job import (
    DATABASES,
    BackupConfig,
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


def test_backup_config_reads_all_required_fields_from_env(monkeypatch):
    _set_backup_env(monkeypatch)
    cfg = BackupConfig.from_env()
    assert cfg.bucket == "lakehouse-backups"
    assert cfg.endpoint == "http://backup-store.internal:9000"
    assert cfg.access_key == "ak"
    assert cfg.secret_key == "sk"
    assert cfg.retention_days == 30  # unset -- the one field allowed a default


def test_backup_config_honors_a_caller_supplied_retention_days(monkeypatch):
    _set_backup_env(monkeypatch)
    monkeypatch.setenv("BACKUP_RETENTION_DAYS", "7")
    assert BackupConfig.from_env().retention_days == 7


def test_backup_config_raises_failure_when_a_required_field_is_missing(monkeypatch):
    _set_backup_env(monkeypatch)
    monkeypatch.delenv("BACKUP_S3_BUCKET", raising=False)
    with pytest.raises(Failure):
        BackupConfig.from_env()


def test_resolve_secret_ref_rejects_anything_but_the_env_scheme():
    with pytest.raises(Failure):
        resolve_secret_ref("vault:some/path")


def test_resolve_secret_ref_rejects_an_unset_target_env_var(monkeypatch):
    monkeypatch.delenv("BACKUP_MISSING", raising=False)
    with pytest.raises(Failure):
        resolve_secret_ref("env:BACKUP_MISSING")


def test_resolve_secret_ref_reads_the_referenced_env_var(monkeypatch):
    monkeypatch.setenv("BACKUP_SOME_VAR", "the-value")
    assert resolve_secret_ref("env:BACKUP_SOME_VAR") == "the-value"


def test_databases_lists_exactly_the_four_compose_created_databases():
    # docker-compose.yml:32,358-359,410-411,1341-1342 -- the exact four
    # databases this compose stack creates, not a guess.
    assert DATABASES == (
        ("POSTGRES_DB", "lakehouse"),
        ("LAKEKEEPER_PG_DB", "lakekeeper"),
        ("OPENFGA_PG_DB", "openfga"),
        ("DAGSTER_PG_DB", "dagster"),
    )


@pytest.mark.parametrize("db_env,db_default", DATABASES)
def test_pg_dump_argv_never_embeds_the_password_on_the_command_line(db_env, db_default, monkeypatch):
    monkeypatch.setenv(db_env, db_default)
    argv = pg_dump_argv(db_env_var=db_env, db_default=db_default, dest_path="/tmp/out.dump")
    assert "--format=custom" in argv
    assert not any("PGPASSWORD" in a for a in argv), "password must travel via PGPASSWORD env, never argv"
    assert f"--dbname={db_default}" in argv


def test_pg_dump_argv_uses_the_env_override_over_the_default(monkeypatch):
    monkeypatch.setenv("POSTGRES_DB", "renamed-app-db")
    argv = pg_dump_argv(db_env_var="POSTGRES_DB", db_default="lakehouse", dest_path="/tmp/out.dump")
    assert "--dbname=renamed-app-db" in argv


class _FakeCompletedProcess:
    def __init__(self, returncode: int, stderr: str = "") -> None:
        self.returncode = returncode
        self.stderr = stderr


class _FakeS3FileSystem:
    """Records every `put`/`rm`; `find` returns a fixed, caller-set
    listing so retention pruning can be exercised without a real bucket."""

    instances: list["_FakeS3FileSystem"] = []

    def __init__(self, **kwargs):
        self.kwargs = kwargs
        self.put_calls: list[tuple[str, str]] = []
        self.rm_calls: list[str] = []
        self.find_result: dict = {}
        _FakeS3FileSystem.instances.append(self)

    def put(self, local_path, remote_path):
        self.put_calls.append((local_path, remote_path))

    def find(self, path, detail=True):
        assert detail is True
        return self.find_result

    def rm(self, path):
        self.rm_calls.append(path)


def test_run_backups_dumps_every_database_and_uploads_to_the_dedicated_bucket(monkeypatch, tmp_path):
    """The dedicated bucket, not the shared warehouse bucket -- see
    backup_job.py's module doc for why sharing that bucket's access
    domain with the identity database's dump is a privilege-escalation
    path."""
    _set_backup_env(monkeypatch)
    _FakeS3FileSystem.instances.clear()
    monkeypatch.setattr("dispar_orchestrate.backup_job.S3FileSystem", _FakeS3FileSystem)

    def _fake_run(argv, capture_output, text, check):
        # Simulate pg_dump writing its dump file, so `os.remove` below
        # has something real to clean up.
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
    assert fake_fs.kwargs["client_kwargs"]["endpoint_url"] == "http://backup-store.internal:9000"
    assert len(fake_fs.put_calls) == len(DATABASES)
    for (_, remote_path), (db_env, db_default) in zip(fake_fs.put_calls, DATABASES):
        db_name = db_default  # env unset in this test, so the default applies
        assert remote_path.startswith(f"lakehouse-backups/{db_name}/")


def test_run_backups_raises_failure_and_stops_on_the_first_pg_dump_failure(monkeypatch):
    """A failed pg_dump must surface as a failed Dagster run, never a
    silently-reported success for a partial backup."""
    _set_backup_env(monkeypatch)
    _FakeS3FileSystem.instances.clear()
    monkeypatch.setattr("dispar_orchestrate.backup_job.S3FileSystem", _FakeS3FileSystem)
    monkeypatch.setattr(
        "dispar_orchestrate.backup_job.subprocess.run",
        lambda *a, **k: _FakeCompletedProcess(returncode=1, stderr="connection refused"),
    )

    with pytest.raises(Failure, match="pg_dump failed"):
        run_backups()

    # No upload was attempted for the database whose dump failed.
    assert not _FakeS3FileSystem.instances[-1].put_calls


def test_prune_expired_deletes_only_objects_older_than_retention_days(monkeypatch):
    from datetime import datetime, timedelta, timezone

    from dispar_orchestrate.backup_job import _prune_expired

    _set_backup_env(monkeypatch)
    monkeypatch.setenv("BACKUP_RETENTION_DAYS", "30")
    cfg = BackupConfig.from_env()

    now = datetime.now(timezone.utc)
    fs = _FakeS3FileSystem()
    fs.find_result = {
        "lakehouse-backups/lakehouse/old.dump": {"LastModified": now - timedelta(days=40)},
        "lakehouse-backups/lakehouse/recent.dump": {"LastModified": now - timedelta(days=1)},
    }

    _prune_expired(fs, cfg)

    assert fs.rm_calls == ["lakehouse-backups/lakehouse/old.dump"]
