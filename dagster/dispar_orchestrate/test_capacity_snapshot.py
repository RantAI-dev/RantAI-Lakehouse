"""Unit tests for `dagster/dispar_orchestrate/capacity_snapshot.py`.

No network: `S3FileSystem` and `_ch_exec` are monkeypatched with fakes,
per this package's own no-network testing rule (`test_maintenance.py`
does the same for its own HTTP-backed functions, via `mock.patch`
instead — this file uses bare pytest functions with the `monkeypatch`
fixture, since this is a brand-new file with no existing suite style to
match and pytest is what actually runs this package's tests).

Run with:
`~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_capacity_snapshot.py -q`
"""

from __future__ import annotations

import pytest
from dagster import Failure

from dispar_orchestrate import capacity_snapshot


def _cfg(**overrides) -> capacity_snapshot.CapacityConfig:
    defaults = dict(
        ch=capacity_snapshot.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
        bucket_name="lakehouse-warehouse",
        rustfs_endpoint="http://rustfs.invalid:9000",
        rustfs_access_key="test-access-key",
        rustfs_secret_key="test-secret-key",
    )
    defaults.update(overrides)
    return capacity_snapshot.CapacityConfig(**defaults)


def test_measure_bucket_lists_objects_via_s3fs(monkeypatch):
    """Uses RustFS's own S3 API via s3fs (already a transitive
    dependency through dlt[s3], now pinned directly), not Lakekeeper's
    admin-scoped Management API and not boto3 (not installed)."""

    class _FakeS3FileSystem:
        def __init__(self, **kwargs):
            self.kwargs = kwargs

        def find(self, path, detail=True):
            assert detail is True
            # A recursive listing under a bucket whose Iceberg layout
            # nests data and metadata files several prefixes deep
            # (warehouse/namespace/table/{data,metadata}/...) — two
            # objects under one table's data/ prefix, one under its
            # metadata/ prefix, and one zero-size directory MARKER
            # (fsspec's find() can return these for some backends) that
            # must NOT be counted as an object.
            return {
                f"{path}/ns/table/data/a.parquet": {"size": 100, "type": "file"},
                f"{path}/ns/table/data/b.parquet": {"size": 250, "type": "file"},
                f"{path}/ns/table/metadata/v1.metadata.json": {"size": 40, "type": "file"},
                f"{path}/ns/table/data/": {"size": 0, "type": "directory"},
            }

    monkeypatch.setattr(capacity_snapshot, "S3FileSystem", _FakeS3FileSystem)
    result = capacity_snapshot.measure_bucket(_cfg())
    assert result.bytes == 390  # 100 + 250 + 40 — the directory marker excluded
    assert result.objects == 3  # the directory marker is not an object


def test_record_capacity_snapshot_writes_one_row(monkeypatch):
    inserted = []
    monkeypatch.setattr(capacity_snapshot, "_ch_exec", lambda target, sql: inserted.append(sql))
    # `_assert_or_create_all` (the R10 ensure-schema step, shared with
    # every other `bronze_catalog` table) issues its own real
    # `system.tables`/`system.columns` reads via `bronze_catalog`'s OWN
    # `_ch_query_json`, not the `_ch_exec` patched above — no-op it here
    # so this stays a pure unit test of the INSERT this function issues,
    # not a live-ClickHouse integration test of schema creation.
    monkeypatch.setattr(capacity_snapshot, "_assert_or_create_all", lambda target, schemas: None)
    capacity_snapshot.record_capacity_snapshot(
        bucket_name="lakehouse-warehouse",
        bytes_=350,
        objects=2,
        target=capacity_snapshot.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
    )
    assert len(inserted) == 1
    assert "bronze_meta.capacity_snapshot" in inserted[0]
    assert "console.capacity_snapshot" not in inserted[0]


def test_capacity_config_from_env_reads_connector_credentials(monkeypatch):
    """The container gives this job `CONNECTOR_S3_ACCESS_KEY`/
    `CONNECTOR_S3_SECRET_KEY` (`docker-compose.yml`'s
    `dagster-code-location` environment), never `RUSTFS_ACCESS_KEY`/
    `RUSTFS_SECRET_KEY` — those are RustFS's own root credentials, which
    this job (like `dlt_pipeline.py` before it) does not need."""
    monkeypatch.setenv("CONNECTOR_S3_ACCESS_KEY", "connector-key")
    monkeypatch.setenv("CONNECTOR_S3_SECRET_KEY", "connector-secret")
    monkeypatch.delenv("RUSTFS_ACCESS_KEY", raising=False)
    monkeypatch.delenv("RUSTFS_SECRET_KEY", raising=False)
    cfg = capacity_snapshot.CapacityConfig.from_env()
    assert cfg.rustfs_access_key == "connector-key"
    assert cfg.rustfs_secret_key == "connector-secret"


def test_capacity_config_from_env_missing_credentials_raises_failure(monkeypatch):
    """AGENTS.md: a missing credential is a configuration problem —
    `dagster.Failure`, never a silent empty-string default that would
    authenticate to RustFS anonymously and fail listing with no clear
    reason."""
    monkeypatch.delenv("CONNECTOR_S3_ACCESS_KEY", raising=False)
    monkeypatch.delenv("CONNECTOR_S3_SECRET_KEY", raising=False)
    with pytest.raises(Failure):
        capacity_snapshot.CapacityConfig.from_env()


def test_capacity_config_from_env_empty_credentials_raises_failure(monkeypatch):
    """An explicitly-empty-string value is the same failure as unset —
    both mean "no credential", never "authenticate as the empty
    string"."""
    monkeypatch.setenv("CONNECTOR_S3_ACCESS_KEY", "")
    monkeypatch.setenv("CONNECTOR_S3_SECRET_KEY", "")
    with pytest.raises(Failure):
        capacity_snapshot.CapacityConfig.from_env()
