"""Unit tests for `dagster/dispar_orchestrate/maintenance.py`'s Bronze-only
table discovery.

PR #30 review blocker: `discover_bronze_tables` used to run `SHOW TABLES`
over the WHOLE `DataLakeCatalog` database with no `bronze.` filter, so
applied maintenance (then `expire_snapshots`, now `remove_orphan_files`)
hit every namespace the catalog knows about — including ADR 0010's
`gold.*` Gold-export namespace. ADR 0009 and
`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` both scope P4 maintenance to
Bronze only; a scoping bug with no test is how this recurs.

No network, no ClickHouse, no pytest dependency this package doesn't
already carry — `_ch_query` is monkeypatched with a canned `SHOW TABLES`
response shaped exactly like the real one (verified against a live
ClickHouse 26.8 + Lakekeeper stack during this fix's own verification:
`SHOW TABLES FROM icecat_maintenance` returns a flat
`bronze.<name>`/`gold.<name>` list with no per-namespace separation).

Run with: `cd dagster && python -m unittest dispar_orchestrate.test_maintenance -v`
"""

from __future__ import annotations

import logging
import unittest
from datetime import datetime, timedelta, timezone
from unittest import mock

import requests
from dagster import build_op_context
from pyiceberg.table.refs import SnapshotRefType

from dispar_orchestrate import maintenance


def _cfg() -> maintenance.MaintenanceConfig:
    return maintenance.MaintenanceConfig(
        ch=maintenance.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
        lakekeeper_catalog_uri="http://lakekeeper.invalid/catalog",
        lakekeeper_warehouse="default",
        rustfs_endpoint="http://rustfs.invalid:9000",
        ch_oauth_client_id="",
        ch_oauth_server_uri="",
        api_url="http://lakehouse-api.invalid:8080",
        maintenance_token="unit-test-maintenance-token",
        trino_url="http://trino.invalid:8080",
    )


def _cfg_with_catalog_identity() -> maintenance.MaintenanceConfig:
    """The shared fixture plus the `dlt` principal's catalog identity — the
    one the pyiceberg fallback needs to reach Lakekeeper at all. Without it
    the fallback records an honest skip ("no catalog identity on this
    stack"), which is a different case and has its own test."""
    return maintenance.MaintenanceConfig(
        ch=maintenance.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
        lakekeeper_catalog_uri="http://lakekeeper.invalid/catalog",
        lakekeeper_warehouse="default",
        rustfs_endpoint="http://rustfs.invalid:9000",
        ch_oauth_client_id="",
        ch_oauth_server_uri="",
        api_url="http://lakehouse-api.invalid:8080",
        maintenance_token="unit-test-maintenance-token",
        trino_url="http://trino.invalid:8080",
        lakekeeper_token_file="/tokens/dlt.jwt",
        lakekeeper_token="unit-test-lakekeeper-token",
        rustfs_access_key="unit-test-access-key",
        rustfs_secret_key="unit-test-secret-key",
    )


class DiscoverBronzeTablesTest(unittest.TestCase):
    def test_gold_tables_are_excluded(self) -> None:
        """The catalog's flat namespace holds both `bronze.*` (this job's
        job) and `gold.*` (ADR 0010's Gold export target, a DIFFERENT
        job's data) in the SAME `SHOW TABLES` response — the review's exact
        failure mode was maintenance running against `gold.*` too."""
        raw = "bronze.g3a_orders\nbronze.maint_smoke\ngold.gold_export_smoke\ngold.mart_wisman\n"
        with mock.patch.object(maintenance, "_ch_query", return_value=raw) as mocked:
            tables = maintenance.discover_bronze_tables(_cfg())
        self.assertEqual(tables, ["bronze.g3a_orders", "bronze.maint_smoke"])
        self.assertTrue(all(t.startswith("bronze.") for t in tables))
        self.assertNotIn("gold.gold_export_smoke", tables)
        self.assertNotIn("gold.mart_wisman", tables)
        mocked.assert_called_once()

    def test_empty_catalog_returns_empty_list(self) -> None:
        with mock.patch.object(maintenance, "_ch_query", return_value=""):
            self.assertEqual(maintenance.discover_bronze_tables(_cfg()), [])

    def test_only_gold_tables_returns_empty_list(self) -> None:
        """A catalog holding ONLY Gold tables (e.g. right after a fresh
        Gold export, before any Bronze ingest) must maintain nothing —
        not fall back to maintaining whatever it finds."""
        with mock.patch.object(
            maintenance, "_ch_query", return_value="gold.gold_export_smoke\n"
        ):
            self.assertEqual(maintenance.discover_bronze_tables(_cfg()), [])

    def test_prefix_match_not_substring_match(self) -> None:
        """A table literally named e.g. `bronzefoo.x` (no dot after
        `bronze`) must NOT match — the filter is a namespace prefix
        (`bronze.`), not a bare substring/startswith-without-dot check."""
        raw = "bronzefoo.x\nbronze.real\n"
        with mock.patch.object(maintenance, "_ch_query", return_value=raw):
            tables = maintenance.discover_bronze_tables(_cfg())
        self.assertEqual(tables, ["bronze.real"])


class MeasureSnapshotGrowthTest(unittest.TestCase):
    """Unit tests for the snapshot/metadata-log growth measurement this fix
    adds (see `maintenance.py`'s module doc, "Lakekeeper-side snapshot
    expiry"): `expire_snapshots` is unsupported on 26.8 for catalog-backed
    tables and nothing in this stack reclaims accumulated snapshots, so
    this function's job is to make that growth VISIBLE, not to reclaim it.
    """

    def test_no_oauth_client_id_skips_without_any_http_call(self) -> None:
        """A pre-R1/authz-disabled stack (`ch_oauth_client_id` unset, same
        condition `probe_expire_snapshots_skip`'s caller already tolerates)
        must degrade to "not measured" rather than fail — and must not
        attempt to mint a token or call the catalog at all."""
        cfg = _cfg()  # ch_oauth_client_id="" in the shared fixture
        with mock.patch("requests.post") as mocked_post, mock.patch(
            "requests.get"
        ) as mocked_get:
            result = maintenance.measure_snapshot_growth(cfg, "bronze.g3a_orders")
        self.assertEqual(
            result, {"measured": False, "snapshot_count": 0, "metadata_log_count": 0}
        )
        mocked_post.assert_not_called()
        mocked_get.assert_not_called()

    def test_measures_snapshot_and_metadata_log_counts(self) -> None:
        """With auth configured: mint a token, resolve the warehouse
        prefix, then read the table's own metadata document and count
        `snapshots`/`metadata-log` entries — never derive these numbers
        from anything else."""
        cfg = maintenance.MaintenanceConfig(
            ch=maintenance.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
            lakekeeper_catalog_uri="http://lakekeeper.invalid/catalog",
            lakekeeper_warehouse="default",
            rustfs_endpoint="http://rustfs.invalid:9000",
            ch_oauth_client_id="clickhouse-reader",
            ch_oauth_server_uri="http://oidc-mock.invalid/token",
        )

        token_resp = mock.Mock()
        token_resp.raise_for_status = mock.Mock()
        token_resp.json.return_value = {"access_token": "fake-token"}

        config_resp = mock.Mock()
        config_resp.raise_for_status = mock.Mock()
        config_resp.json.return_value = {"defaults": {"prefix": "wh-prefix"}}

        table_resp = mock.Mock()
        table_resp.raise_for_status = mock.Mock()
        table_resp.json.return_value = {
            "metadata": {
                "snapshots": [{"snapshot-id": 1}, {"snapshot-id": 2}, {"snapshot-id": 3}],
                "metadata-log": [{"metadata-file": "a"}, {"metadata-file": "b"}],
            }
        }

        with mock.patch("requests.post", return_value=token_resp) as mocked_post, mock.patch(
            "requests.get", side_effect=[config_resp, table_resp]
        ) as mocked_get:
            result = maintenance.measure_snapshot_growth(cfg, "bronze.g3a_orders")

        self.assertEqual(
            result,
            {"measured": True, "snapshot_count": 3, "metadata_log_count": 2},
        )
        mocked_post.assert_called_once()
        self.assertEqual(mocked_get.call_count, 2)
        table_call_url = mocked_get.call_args_list[1].args[0]
        self.assertIn("/v1/wh-prefix/namespaces/bronze/tables/g3a_orders", table_call_url)

    def test_missing_snapshots_and_metadata_log_count_as_zero(self) -> None:
        """A table metadata document with no `snapshots`/`metadata-log`
        keys at all (not just empty lists) must count as zero, not raise —
        `dict.get(...) or []` covers both `None` and a missing key."""
        cfg = maintenance.MaintenanceConfig(
            ch=maintenance.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
            lakekeeper_catalog_uri="http://lakekeeper.invalid/catalog",
            lakekeeper_warehouse="default",
            rustfs_endpoint="http://rustfs.invalid:9000",
            ch_oauth_client_id="clickhouse-reader",
            ch_oauth_server_uri="http://oidc-mock.invalid/token",
        )

        token_resp = mock.Mock()
        token_resp.raise_for_status = mock.Mock()
        token_resp.json.return_value = {"access_token": "fake-token"}
        config_resp = mock.Mock()
        config_resp.raise_for_status = mock.Mock()
        config_resp.json.return_value = {"overrides": {"prefix": "wh-prefix"}}
        table_resp = mock.Mock()
        table_resp.raise_for_status = mock.Mock()
        table_resp.json.return_value = {"metadata": {}}

        with mock.patch("requests.post", return_value=token_resp), mock.patch(
            "requests.get", side_effect=[config_resp, table_resp]
        ):
            result = maintenance.measure_snapshot_growth(cfg, "bronze.g3a_orders")

        self.assertEqual(
            result, {"measured": True, "snapshot_count": 0, "metadata_log_count": 0}
        )


class FetchPolicyIndexTest(unittest.TestCase):
    """`_fetch_policy_index` fetches every configured maintenance policy
    ONCE per run and indexes it by `(namespace, tableName)` — see the
    module doc's "Per-table policy + maintenance verbs" section. A policy-fetch
    failure (401/403, a server error, an unreachable API, or no token
    configured) is fail-closed for auth (raises `Failure`) or recorded as
    a skip for everything else, never silently treated as "no policy
    configured"."""

    def test_indexes_policies_by_namespace_and_table_name(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.raise_for_status = mock.Mock()
        resp.json.return_value = {
            "policies": [
                {"namespace": "bronze", "tableName": "orders", "snapshotsToKeep": 10},
                {"namespace": "bronze", "tableName": "events", "compactSmallFiles": True},
            ]
        }
        with mock.patch("requests.get", return_value=resp) as mocked_get:
            index, error = maintenance._fetch_policy_index(_cfg())
        self.assertIsNone(error)
        self.assertEqual(index[("bronze", "orders")]["snapshotsToKeep"], 10)
        self.assertEqual(index[("bronze", "events")]["compactSmallFiles"], True)
        mocked_get.assert_called_once()
        called_headers = mocked_get.call_args.kwargs["headers"]
        self.assertEqual(called_headers["Authorization"], "Bearer unit-test-maintenance-token")

    def test_a_401_response_raises_a_dagster_failure(self) -> None:
        resp = mock.Mock(status_code=401)
        with mock.patch("requests.get", return_value=resp):
            with self.assertRaises(maintenance.Failure):
                maintenance._fetch_policy_index(_cfg())

    def test_a_401_response_marks_the_failure_non_retryable(self) -> None:
        """PART D: a 401 from the maintenance-policy endpoint is a
        configuration/auth problem (token wrong, role no longer
        grants the endpoint) -- retrying won't help. The raise must
        set `allow_retries=False` so `DEFAULT_RETRY_POLICY` does not
        waste its 60s budget on a problem the operator has to fix
        out-of-band. The 401/403 sites share one path so testing
        401 alone covers the contract."""
        from dagster import Failure

        resp = mock.Mock(status_code=401)
        with mock.patch("requests.get", return_value=resp):
            with self.assertRaises(Failure) as ctx:
                maintenance._fetch_policy_index(_cfg())
        self.assertFalse(ctx.exception.allow_retries)

    def test_a_403_response_raises_a_dagster_failure(self) -> None:
        resp = mock.Mock(status_code=403)
        with mock.patch("requests.get", return_value=resp):
            with self.assertRaises(maintenance.Failure):
                maintenance._fetch_policy_index(_cfg())

    def test_a_server_error_response_is_recorded_as_a_skip_not_treated_as_no_policy(
        self,
    ) -> None:
        resp = mock.Mock(status_code=500)
        resp.raise_for_status = mock.Mock(
            side_effect=requests.HTTPError("500 Server Error")
        )
        with mock.patch("requests.get", return_value=resp):
            index, error = maintenance._fetch_policy_index(_cfg())
        self.assertEqual(index, {})
        self.assertIsNotNone(error)

    def test_an_unreachable_api_is_recorded_as_a_skip_not_treated_as_no_policy(
        self,
    ) -> None:
        with mock.patch(
            "requests.get", side_effect=requests.ConnectionError("connection refused")
        ):
            index, error = maintenance._fetch_policy_index(_cfg())
        self.assertEqual(index, {})
        self.assertIsNotNone(error)

    def test_no_token_configured_returns_an_empty_index_without_a_network_call(
        self,
    ) -> None:
        cfg = maintenance.MaintenanceConfig(
            ch=maintenance.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
            lakekeeper_catalog_uri="http://lakekeeper.invalid/catalog",
            lakekeeper_warehouse="default",
            rustfs_endpoint="http://rustfs.invalid:9000",
            ch_oauth_client_id="",
            ch_oauth_server_uri="",
        )  # maintenance_token defaults to ""
        with mock.patch("requests.get") as mocked_get:
            index, error = maintenance._fetch_policy_index(cfg)
        self.assertEqual(index, {})
        self.assertIsNotNone(error)
        mocked_get.assert_not_called()


class ComputeRetentionThresholdTest(unittest.TestCase):
    """`_compute_retention_threshold` must never emit a `'0d'`-style
    threshold that would expire every snapshot but the current one — the
    threshold is the AGE of the Nth-newest snapshot, so exactly N
    survive."""

    def test_computes_the_age_of_the_nth_newest_snapshot_in_whole_seconds_rounded_up(
        self,
    ) -> None:
        now = datetime(2026, 1, 10, 0, 0, 0, tzinfo=timezone.utc)
        committed_at = [
            now - timedelta(seconds=10),
            now - timedelta(days=1, seconds=30, microseconds=500_000),
            now - timedelta(days=5),
        ]
        threshold, skip_reason = maintenance._compute_retention_threshold(
            committed_at, keep=2, now=now
        )
        self.assertIsNone(skip_reason)
        # The 2nd-newest snapshot is 1 day + 30.5s old -> rounds up to
        # 86431 whole seconds so it (and everything newer) survives.
        self.assertEqual(threshold, "86431s")

    def test_a_table_with_exactly_keep_snapshots_has_nothing_to_expire(self) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        committed_at = [now - timedelta(seconds=1), now - timedelta(days=1)]
        threshold, skip_reason = maintenance._compute_retention_threshold(
            committed_at, keep=2, now=now
        )
        self.assertIsNone(threshold)
        self.assertIn("nothing to expire", skip_reason or "")

    def test_a_table_with_fewer_than_keep_snapshots_has_nothing_to_expire(self) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        threshold, skip_reason = maintenance._compute_retention_threshold(
            [now - timedelta(seconds=1)], keep=10, now=now
        )
        self.assertIsNone(threshold)
        self.assertIn("nothing to expire", skip_reason or "")


class CadenceAllowsTest(unittest.TestCase):
    """Per-table `schedule` cadence for the expiry and compaction verbs only."""

    def test_a_null_schedule_always_allows_a_run(self) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        ok, reason = maintenance._cadence_allows(None, now, now - timedelta(seconds=1))
        self.assertTrue(ok)
        self.assertIsNone(reason)

    def test_no_prior_recorded_run_always_allows_a_run(self) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        ok, reason = maintenance._cadence_allows("weekly", now, None)
        self.assertTrue(ok)
        self.assertIsNone(reason)

    def test_weekly_schedule_blocks_a_run_less_than_seven_days_after_the_last_one(
        self,
    ) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        ok, reason = maintenance._cadence_allows("weekly", now, now - timedelta(days=6))
        self.assertFalse(ok)
        self.assertIsNotNone(reason)

    def test_weekly_schedule_allows_a_run_at_least_seven_days_after_the_last_one(
        self,
    ) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        ok, reason = maintenance._cadence_allows("weekly", now, now - timedelta(days=7))
        self.assertTrue(ok)
        self.assertIsNone(reason)

    def test_daily_schedule_blocks_a_run_less_than_twenty_hours_after_the_last_one(
        self,
    ) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        ok, reason = maintenance._cadence_allows("daily", now, now - timedelta(hours=19))
        self.assertFalse(ok)
        self.assertIsNotNone(reason)

    def test_daily_schedule_allows_a_run_at_least_twenty_hours_after_the_last_one(
        self,
    ) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        ok, reason = maintenance._cadence_allows("daily", now, now - timedelta(hours=20))
        self.assertTrue(ok)
        self.assertIsNone(reason)


class ExpireSnapshotsResultTest(unittest.TestCase):
    """`_expire_snapshots_result` decides and (if requested) executes the
    `expire_snapshots` verb via Trino."""

    def test_table_without_policy_expires_snapshots_older_than_seven_days(
        self,
    ) -> None:
        with mock.patch.object(maintenance, "_trino_execute", return_value=[]) as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", None,
                datetime.now(timezone.utc), None,
            )
        self.assertIsNone(skip)
        self.assertIsNotNone(verb_run)
        self.assertEqual(verb_run["outcome"], "applied")
        self.assertEqual(
            verb_run["detail"], "default retention 7d (no snapshotsToKeep policy)"
        )
        mocked.assert_called_once_with(
            mock.ANY,
            'ALTER TABLE iceberg.bronze."orders" EXECUTE expire_snapshots(retention_threshold => \x277d\x27)',
        )

    def test_a_table_without_a_policy_falls_back_to_pyiceberg_when_trino_is_unreachable(
        self,
    ) -> None:
        """Trino being unreachable is no longer the outcome of this verb:
        the pyiceberg fallback takes over and applies the SAME default
        retention (7 days, OPS-1 D1) the Trino path used. Only the
        snapshots older than that go — the ones the operator's policy
        keeps are untouched."""
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        snapshots = [
            _FakeSnapshot(3, int((now - timedelta(seconds=10)).timestamp() * 1000)),
            _FakeSnapshot(2, int((now - timedelta(days=8)).timestamp() * 1000)),
            _FakeSnapshot(1, int((now - timedelta(days=20)).timestamp() * 1000)),
        ]
        calls: dict = {}
        catalog = _FakeCatalog(_FakeTable(snapshots, {}, calls))
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(
            maintenance, "_pyiceberg_catalog", return_value=catalog
        ) as mocked_catalog:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                None,
                now,
                None,
            )
        self.assertIsNone(skip)
        self.assertIsNotNone(verb_run)
        self.assertEqual(verb_run["engine"], "pyiceberg")
        self.assertEqual(verb_run["outcome"], "applied")
        self.assertEqual(calls["by_ids"], [2, 1])
        mocked_catalog.assert_called_once()

    def test_table_without_policy_records_a_trino_refusal_as_refused(
        self,
    ) -> None:
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoQueryError("Query failed: retention shorter than min retention"),
        ):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", None,
                datetime.now(timezone.utc), None,
            )
        self.assertIsNotNone(skip)
        self.assertIn("refused", skip or "")
        self.assertIsNotNone(verb_run)
        self.assertEqual(verb_run["outcome"], "refused")
        self.assertEqual(verb_run["detail"], "Query failed: retention shorter than min retention")

    def test_policy_with_no_snapshots_to_keep_uses_default_retention(self) -> None:
        policy = {"snapshotsToKeep": None, "compactSmallFiles": False, "schedule": None}
        with mock.patch.object(maintenance, "_trino_execute", return_value=[]) as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy,
                datetime.now(timezone.utc), None,
            )
        self.assertIsNone(skip)
        self.assertIsNotNone(verb_run)
        self.assertEqual(verb_run["outcome"], "applied")
        self.assertEqual(
            verb_run["detail"], "default retention 7d (no snapshotsToKeep policy)"
        )
        mocked.assert_called_once_with(
            mock.ANY,
            'ALTER TABLE iceberg.bronze."orders" EXECUTE expire_snapshots(retention_threshold => \x277d\x27)',
        )

    def test_a_table_with_fewer_snapshots_than_the_keep_count_is_skipped(self) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        policy = {"snapshotsToKeep": 5, "schedule": None}
        with mock.patch.object(
            maintenance, "_trino_execute", return_value=[["2026-01-09 00:00:00.000 UTC"]]
        ) as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy, now, None
            )
        self.assertIn("nothing to expire", skip or "")
        self.assertEqual(verb_run and verb_run["outcome"], "skipped")
        mocked.assert_called_once()

    def test_a_refusal_is_recorded_and_the_threshold_is_never_retried_larger(self) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        policy = {"snapshotsToKeep": 1, "schedule": None}
        rows = [["2026-01-09 23:59:00.000 UTC"], ["2026-01-01 00:00:00.000 UTC"]]

        def fake_trino_execute(_cfg, sql):
            if "SELECT committed_at" in sql:
                return rows
            raise maintenance.TrinoQueryError(
                "Query failed: retention specified (60s) is shorter than the "
                "minimum retention configured in the system (7.00d)"
            )

        with mock.patch.object(
            maintenance, "_trino_execute", side_effect=fake_trino_execute
        ) as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy, now, None
            )
        self.assertIsNotNone(skip)
        self.assertIn("refused", skip or "")
        self.assertEqual(verb_run and verb_run["outcome"], "refused")
        self.assertEqual(mocked.call_count, 2)  # one read, one attempted ALTER — no retry

    def test_policy_with_snapshots_to_keep_still_uses_its_own_threshold(self) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        policy = {"snapshotsToKeep": 1, "schedule": None}
        rows = [["2026-01-09 23:59:00.000 UTC"], ["2026-01-01 00:00:00.000 UTC"]]

        def fake_trino_execute(_cfg, sql):
            if "SELECT committed_at" in sql:
                return rows
            return []

        with mock.patch.object(maintenance, "_trino_execute", side_effect=fake_trino_execute) as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy, now, None
            )
        self.assertIsNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "applied")
        self.assertEqual(verb_run and verb_run["detail"], "retention_threshold=60s")
        mocked.assert_called_with(
            mock.ANY,
            'ALTER TABLE iceberg.bronze."orders" EXECUTE expire_snapshots(retention_threshold => \x2760s\x27)',
        )

    def test_trino_being_unreachable_falls_back_and_never_raises(self) -> None:
        """Trino being unreachable is a degraded state, not a job failure:
        the pyiceberg fallback takes over, and when the catalog is
        unreachable too the honest `failed` outcome is RECORDED — the
        call still returns, so one table's broken catalog cannot take the
        whole maintenance run down."""
        policy = {"snapshotsToKeep": 1, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(
            maintenance, "_pyiceberg_catalog", side_effect=ConnectionError("catalog unreachable")
        ):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                datetime.now(timezone.utc),
                None,
            )
        self.assertIsNotNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "failed")
        self.assertEqual(verb_run and verb_run["engine"], "pyiceberg")

    def test_a_weekly_schedule_still_within_its_window_skips_without_a_trino_call(
        self,
    ) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        policy = {"snapshotsToKeep": 1, "schedule": "weekly"}
        with mock.patch.object(maintenance, "_trino_execute") as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy, now, now - timedelta(days=1)
            )
        self.assertIsNotNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "skipped")
        mocked.assert_not_called()


class _FakeSnapshot:
    """The two attributes `maintenance.py` reads off a pyiceberg `Snapshot`
    (`snapshot_id`, `timestamp_ms`) — the rest of the real object is
    irrelevant to the expiry and is deliberately absent."""

    def __init__(self, snapshot_id: int, timestamp_ms: int) -> None:
        self.snapshot_id = snapshot_id
        self.timestamp_ms = timestamp_ms


class _FakeRef:
    """A pyiceberg `SnapshotRef`: the head snapshot of a branch or a tag.
    Both are PROTECTED — `pyiceberg`'s `ExpireSnapshots.by_id` raises on
    them, so the code under test must filter them out itself."""

    def __init__(self, snapshot_id: int, ref_type: SnapshotRefType) -> None:
        self.snapshot_id = snapshot_id
        self.snapshot_ref_type = ref_type


class _FakeExpireSnapshots:
    def __init__(self, calls: dict, commit_error: Exception | None = None) -> None:
        self._calls = calls
        self._commit_error = commit_error

    def by_ids(self, snapshot_ids: list[int]) -> "_FakeExpireSnapshots":
        self._calls["by_ids"] = list(snapshot_ids)
        return self

    def commit(self) -> None:
        if self._commit_error is not None:
            raise self._commit_error
        self._calls["committed"] = True


class _FakeMaintenance:
    def __init__(self, calls: dict, commit_error: Exception | None = None) -> None:
        self._calls = calls
        self._commit_error = commit_error

    def expire_snapshots(self) -> _FakeExpireSnapshots:
        self._calls["expire_snapshots"] = True
        return _FakeExpireSnapshots(self._calls, self._commit_error)


class _FakeTable:
    """A pyiceberg `Table` stand-in: `snapshots()`, `refs()`,
    `current_snapshot()` and `maintenance.expire_snapshots()` — the whole
    surface the fallback touches.

    `current_snapshot` defaults to `None` (a table with no current
    snapshot) and `commit_error` makes the expiry commit raise, so the
    two failure paths are reachable without a network."""

    def __init__(
        self,
        snapshots: list[_FakeSnapshot],
        refs: dict[str, _FakeRef],
        calls: dict,
        current_snapshot: _FakeSnapshot | None = None,
        commit_error: Exception | None = None,
    ) -> None:
        self._snapshots = snapshots
        self._refs = refs
        self._current_snapshot = current_snapshot
        self.maintenance = _FakeMaintenance(calls, commit_error)

    def snapshots(self) -> list[_FakeSnapshot]:
        return list(self._snapshots)

    def refs(self) -> dict[str, _FakeRef]:
        return dict(self._refs)

    def current_snapshot(self) -> _FakeSnapshot | None:
        return self._current_snapshot


class _FakeCatalog:
    def __init__(self, table: _FakeTable) -> None:
        self._table = table
        self.loaded: list[tuple[str, str]] = []

    def load_table(self, identifier: tuple[str, str]) -> _FakeTable:
        self.loaded.append(identifier)
        return self._table


class PyIcebergExpiryFallbackTest(unittest.TestCase):
    """OPS-20: with Trino absent (its compose profile is off), the
    snapshot expiry falls back to `pyiceberg` — the same REST catalog the
    ingest job already writes through. Trino keeps the path when it
    answers; only an unreachable Trino switches engines."""

    NOW = datetime(2026, 1, 10, tzinfo=timezone.utc)

    def setUp(self) -> None:
        self.calls: dict = {}

    def _table_with_six_snapshots(
        self,
        current_snapshot: _FakeSnapshot | None = None,
        commit_error: Exception | None = None,
    ) -> _FakeTable:
        """Six snapshots of different ages, plus a branch head and a tag
        head that are BOTH old — the two protected ids a naive expiry
        would try to remove (and that `pyiceberg` refuses to expire)."""
        ages = {
            6: timedelta(seconds=10),
            5: timedelta(days=1),
            4: timedelta(days=2),
            3: timedelta(days=3),
            2: timedelta(days=10),
            1: timedelta(days=20),
        }
        snapshots = [
            _FakeSnapshot(snapshot_id, int((self.NOW - age).timestamp() * 1000))
            for snapshot_id, age in ages.items()
        ]
        refs = {
            "main": _FakeRef(6, SnapshotRefType.BRANCH),
            "old-branch": _FakeRef(2, SnapshotRefType.BRANCH),
            "release-1": _FakeRef(1, SnapshotRefType.TAG),
        }
        return _FakeTable(
            snapshots,
            refs,
            self.calls,
            current_snapshot=current_snapshot,
            commit_error=commit_error,
        )

    def test_the_fallback_expires_the_old_snapshots_minus_the_kept_and_the_protected_ones(
        self,
    ) -> None:
        """With Trino unreachable, the pyiceberg path removes exactly the
        snapshots that are older than the threshold AND neither among the
        newest `snapshotsToKeep` nor a branch/tag head: with keep=2 the
        2-day-old and the 3-day-old snapshots go, while the 10-day-old
        branch head and the 20-day-old tag head stay (protected)."""
        table = self._table_with_six_snapshots()
        catalog = _FakeCatalog(table)
        policy = {"snapshotsToKeep": 2, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(
            maintenance, "_pyiceberg_catalog", return_value=catalog
        ) as mocked_catalog:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                None,
            )
        self.assertIsNone(skip)
        self.assertEqual(verb_run and verb_run["engine"], "pyiceberg")
        self.assertEqual(verb_run and verb_run["outcome"], "applied")
        self.assertEqual(self.calls["by_ids"], [4, 3])
        self.assertTrue(self.calls["committed"])
        mocked_catalog.assert_called_once()
        self.assertEqual(catalog.loaded, [("bronze", "orders")])

    def test_the_result_detail_carries_the_count_the_threshold_and_the_reclaim_note(
        self,
    ) -> None:
        """The recorded detail says how many snapshots went, which
        threshold decided it, and that the FILES are reclaimed later by
        `remove_orphan_files` — whose 3-day age floor delays the space
        reclaim by about three days."""
        catalog = _FakeCatalog(self._table_with_six_snapshots())
        policy = {"snapshotsToKeep": 2, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(maintenance, "_pyiceberg_catalog", return_value=catalog):
            _skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                None,
            )
        detail = (verb_run or {}).get("detail", "")
        self.assertIn("2", detail)
        self.assertIn("86400s", detail)
        self.assertIn(str(maintenance.ORPHAN_FILE_AGE_FLOOR_SECONDS), detail)
        self.assertIn("remove_orphan_files", detail)

    def test_a_snapshot_committed_exactly_at_the_cutoff_survives_the_expiry(self) -> None:
        """The cutoff is EXCLUSIVE: a snapshot whose commit instant is
        exactly `now - threshold` is not OLDER than the threshold, so it
        survives. Iceberg's own expiry is strict on that boundary —
        `pyiceberg` 0.12.0's `ExpireSnapshots.older_than` expires a
        snapshot only when `snapshot.timestamp_ms < expire_from`
        (`pyiceberg/table/update/snapshot.py:1335`), and Iceberg's Java
        `RemoveSnapshots` (what Trino's `expire_snapshots` calls) keeps a
        snapshot while `timestampMillis() >= expireOlderThan`.

        With no `snapshotsToKeep` the threshold is the default 7 days, so
        the snapshot committed exactly 7 days ago lands exactly ON the
        cutoff: it must survive, while the 8-day-old one goes."""
        cutoff = self.NOW - timedelta(days=7)
        snapshots = [
            _FakeSnapshot(3, int((self.NOW - timedelta(seconds=10)).timestamp() * 1000)),
            _FakeSnapshot(2, int(cutoff.timestamp() * 1000)),
            _FakeSnapshot(1, int((self.NOW - timedelta(days=8)).timestamp() * 1000)),
        ]
        catalog = _FakeCatalog(_FakeTable(snapshots, {}, self.calls))
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(maintenance, "_pyiceberg_catalog", return_value=catalog):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                None,
                self.NOW,
                None,
            )
        self.assertIsNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "applied")
        self.assertEqual(self.calls["by_ids"], [1])

    def test_a_snapshot_tied_with_the_nth_newest_in_the_same_millisecond_survives(self) -> None:
        """`snapshotsToKeep=2`, and the 2nd- and 3rd-newest snapshots share
        ONE millisecond — which is exactly the cutoff, because the
        threshold is the 2nd-newest's age in whole seconds. The 3rd is
        not one of the two the policy keeps, so only the strict
        comparison keeps it: "the newest N survive" becomes "AT LEAST N
        survive". That is Iceberg's own behaviour at a tie, and it is
        why the comparison is strict."""
        tied = self.NOW - timedelta(seconds=100)
        snapshots = [
            _FakeSnapshot(4, int((self.NOW - timedelta(seconds=10)).timestamp() * 1000)),
            _FakeSnapshot(3, int(tied.timestamp() * 1000)),
            _FakeSnapshot(2, int(tied.timestamp() * 1000)),
            _FakeSnapshot(1, int((self.NOW - timedelta(seconds=200)).timestamp() * 1000)),
        ]
        catalog = _FakeCatalog(_FakeTable(snapshots, {}, self.calls))
        policy = {"snapshotsToKeep": 2, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(maintenance, "_pyiceberg_catalog", return_value=catalog):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                None,
            )
        self.assertIsNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "applied")
        self.assertEqual(self.calls["by_ids"], [1])

    def test_the_current_snapshot_is_never_expired_however_old_it_is(self) -> None:
        """The table's CURRENT snapshot is protected by its own id, not
        only through the branch heads: a format-version-1 table has no
        refs at all, and `pyiceberg`'s `RemoveSnapshotsUpdate` drops a
        snapshot from the metadata without touching `current_snapshot_id`
        — expiring it would leave the table pointing at a snapshot that
        no longer exists. Here the current snapshot is 5 days old, the
        threshold is 2 days and no branch protects it; the OTHER old
        snapshot still goes, so the guard is selective."""
        current = _FakeSnapshot(7, int((self.NOW - timedelta(days=5)).timestamp() * 1000))
        snapshots = [
            _FakeSnapshot(9, int((self.NOW - timedelta(seconds=10)).timestamp() * 1000)),
            _FakeSnapshot(8, int((self.NOW - timedelta(days=2)).timestamp() * 1000)),
            current,
            _FakeSnapshot(6, int((self.NOW - timedelta(days=20)).timestamp() * 1000)),
        ]
        catalog = _FakeCatalog(_FakeTable(snapshots, {}, self.calls, current_snapshot=current))
        policy = {"snapshotsToKeep": 2, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(maintenance, "_pyiceberg_catalog", return_value=catalog):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                None,
            )
        self.assertIsNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "applied")
        self.assertEqual(self.calls["by_ids"], [6])

    def test_a_failed_expiry_records_the_exception_type_and_not_its_message(self) -> None:
        """The recorded detail reaches
        `lake.bronze_meta.maintenance_verb_run` and is served by the
        API, so upstream (Lakekeeper/pyiceberg) text must never reach
        it: the detail carries the exception TYPE only — the same rule
        this module follows for the policy fetch (`f"{type(exc).__name__}"`)."""
        upstream_text = "lakekeeper refused the request: bearer token not-a-real-token-value"
        catalog = _FakeCatalog(self._table_with_six_snapshots(commit_error=RuntimeError(upstream_text)))
        policy = {"snapshotsToKeep": 2, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(maintenance, "_pyiceberg_catalog", return_value=catalog):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                None,
            )
        self.assertEqual(verb_run and verb_run["outcome"], "failed")
        detail = (verb_run or {}).get("detail", "")
        self.assertEqual(detail, "RuntimeError")
        self.assertNotIn(upstream_text, detail)
        self.assertNotIn("not-a-real-token-value", detail)
        self.assertNotIn("not-a-real-token-value", skip or "")

    def test_a_reachable_trino_keeps_the_trino_path_and_never_reaches_pyiceberg(self) -> None:
        """Trino answering is the normal case: its path stays exactly as it
        was and the catalog is never even built."""
        policy = {"snapshotsToKeep": 2, "schedule": None}
        rows = [
            ["2026-01-09 00:00:00.000 UTC"],
            ["2026-01-08 00:00:00.000 UTC"],
            ["2026-01-01 00:00:00.000 UTC"],
        ]
        with mock.patch.object(
            maintenance, "_trino_execute", return_value=rows
        ) as mocked_trino, mock.patch.object(
            maintenance, "_pyiceberg_catalog"
        ) as mocked_catalog:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                None,
            )
        self.assertIsNone(skip)
        self.assertEqual(verb_run and verb_run["engine"], "trino")
        self.assertEqual(verb_run and verb_run["outcome"], "applied")
        mocked_catalog.assert_not_called()
        self.assertTrue(mocked_trino.called)

    def test_a_table_with_no_more_snapshots_than_the_keep_count_is_skipped(self) -> None:
        """A table with exactly `snapshotsToKeep` snapshots has nothing to
        expire — skipped, never committed, and never a zero threshold."""
        snapshots = [
            _FakeSnapshot(2, int((self.NOW - timedelta(seconds=10)).timestamp() * 1000)),
            _FakeSnapshot(1, int((self.NOW - timedelta(days=1)).timestamp() * 1000)),
        ]
        catalog = _FakeCatalog(_FakeTable(snapshots, {}, self.calls))
        policy = {"snapshotsToKeep": 2, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(maintenance, "_pyiceberg_catalog", return_value=catalog):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                None,
            )
        self.assertIn("nothing to expire", skip or "")
        self.assertEqual(verb_run and verb_run["outcome"], "skipped")
        self.assertEqual(verb_run and verb_run["engine"], "pyiceberg")
        self.assertNotIn("committed", self.calls)

    def test_a_stack_without_a_catalog_identity_skips_instead_of_calling_the_catalog(
        self,
    ) -> None:
        """No `dlt` token on this stack means no catalog identity: an
        honest skip, and the catalog is never built."""
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(
            maintenance, "_pyiceberg_catalog"
        ) as mocked_catalog:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(),
                "bronze",
                "orders",
                {"snapshotsToKeep": 2, "schedule": None},
                self.NOW,
                None,
            )
        self.assertIsNotNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "skipped")
        self.assertEqual(verb_run and verb_run["engine"], "pyiceberg")
        mocked_catalog.assert_not_called()

    def test_an_unreachable_catalog_is_recorded_as_failed_and_never_raises(self) -> None:
        """A catalog that cannot be reached is an honest `failed` verb
        run; it must not raise, because one table's broken catalog must
        not take the whole maintenance run down."""
        policy = {"snapshotsToKeep": 2, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(
            maintenance, "_pyiceberg_catalog", side_effect=ConnectionError("catalog unreachable")
        ):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                None,
            )
        self.assertIsNotNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "failed")
        self.assertEqual(verb_run and verb_run["engine"], "pyiceberg")

    def test_an_unreadable_table_is_recorded_as_failed_and_never_raises(self) -> None:
        """The id computation sits inside the catalog try: a table whose
        snapshot metadata cannot be read is an honest `failed` verb run,
        never an exception that takes the whole maintenance run down."""
        policy = {"snapshotsToKeep": 2, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ), mock.patch.object(
            maintenance,
            "_pyiceberg_catalog",
            return_value=_FakeCatalog(self._table_with_six_snapshots()),
        ), mock.patch.object(
            maintenance,
            "_snapshot_ids_to_expire",
            side_effect=ValueError("metadata unreadable"),
        ):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                None,
            )
        self.assertIsNotNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "failed")
        self.assertEqual(verb_run and verb_run["engine"], "pyiceberg")
        self.assertEqual((verb_run or {}).get("detail"), "ValueError")

    def test_a_cadence_refusal_does_not_fall_back_to_pyiceberg(self) -> None:
        """The schedule cadence is a POLICY decision, not an engine
        problem: a table inside its weekly window is skipped without
        either engine being touched. The recorded run therefore names NO
        engine (`none`), not a guess at the one that would have run."""
        policy = {"snapshotsToKeep": 2, "schedule": "weekly"}
        with mock.patch.object(
            maintenance, "_trino_execute"
        ) as mocked_trino, mock.patch.object(
            maintenance, "_pyiceberg_catalog"
        ) as mocked_catalog:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg_with_catalog_identity(),
                "bronze",
                "orders",
                policy,
                self.NOW,
                self.NOW - timedelta(days=1),
            )
        self.assertIsNotNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "skipped")
        self.assertEqual(verb_run and verb_run["engine"], "none")
        mocked_trino.assert_not_called()
        mocked_catalog.assert_not_called()


class OptimizeResultTest(unittest.TestCase):
    """`_optimize_result` mirrors `_expire_snapshots_result`'s shape for
    the `optimize` verb."""

    def test_no_policy_keeps_todays_fixed_design_skip_message(self) -> None:
        with mock.patch.object(maintenance, "_trino_execute") as mocked:
            skip, verb_run = maintenance._optimize_result(
                _cfg(), "bronze", "orders", None, datetime.now(timezone.utc), None
            )
        self.assertIn("does not bin-pack", skip or "")
        self.assertIsNone(verb_run)
        mocked.assert_not_called()

    def test_compact_small_files_true_runs_optimize_via_trino(self) -> None:
        policy = {"compactSmallFiles": True, "schedule": None}
        with mock.patch.object(maintenance, "_trino_execute", return_value=[]) as mocked:
            skip, verb_run = maintenance._optimize_result(
                _cfg(), "bronze", "orders", policy, datetime.now(timezone.utc), None
            )
        self.assertIsNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "applied")
        mocked.assert_called_once()
        sql = mocked.call_args.args[1]
        self.assertIn("EXECUTE optimize", sql)

    def test_an_unreachable_trino_skips_only_the_optimize_verb(self) -> None:
        """The recorded skip names the OPTIMIZE verb and nothing else: the
        snapshot expiry is NOT skipped when Trino is unreachable — it
        runs through the catalog — so a skip text that claims the expiry
        was skipped too would put a false statement in the recorded
        run."""
        policy = {"compactSmallFiles": True, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable"),
        ):
            skip, verb_run = maintenance._optimize_result(
                _cfg(), "bronze", "orders", policy, datetime.now(timezone.utc), None
            )
        self.assertEqual(skip, "optimize skipped: trino unreachable")
        self.assertNotIn("expire_snapshots", skip or "")
        self.assertEqual(verb_run and verb_run["outcome"], "skipped")
        self.assertEqual(verb_run and verb_run["detail"], "trino unreachable")

    def test_a_failed_optimize_call_is_recorded_and_not_silently_dropped(self) -> None:
        policy = {"compactSmallFiles": True, "schedule": None}
        with mock.patch.object(
            maintenance, "_trino_execute", side_effect=maintenance.TrinoQueryError("boom")
        ):
            skip, verb_run = maintenance._optimize_result(
                _cfg(), "bronze", "orders", policy, datetime.now(timezone.utc), None
            )
        self.assertIn("boom", skip or "")
        self.assertEqual(verb_run and verb_run["outcome"], "failed")


class RunBronzeMaintenanceTrinoIntegrationTest(unittest.TestCase):
    """`_run_maintenance_for_table` (the per-table body extracted from
    the pre-PART-C `run_bronze_maintenance`), with every I/O boundary
    stubbed — no network. The PART C fan-out (`list_bronze_tables` ->
    `maintain_bronze_table`) drives this same helper from each mapped
    step, so these tests cover both the pre-fan-out op and every
    `maintain_bronze_table[<key>]` step at once."""

    def _fake_context(self):
        """A real `OpExecutionContext` (`dagster.build_op_context`), not a
        `mock.Mock` — `@op`-decorated functions require a genuine
        `BaseDirectExecutionContext` for direct invocation, `context.log`
        included, and reject a bare mock."""
        return build_op_context()

    def test_issues_a_trino_optimize_call_when_the_policy_requests_compaction(
        self,
    ) -> None:
        with mock.patch.object(
            maintenance, "probe_expire_snapshots_skip", return_value="ch refuses"
        ), mock.patch.object(
            maintenance, "run_remove_orphan_files", return_value={"table_name": "bronze.orders"}
        ), mock.patch.object(
            maintenance, "measure_snapshot_growth",
            return_value={"measured": False, "snapshot_count": 0, "metadata_log_count": 0},
        ), mock.patch.object(
            maintenance, "record_maintenance_run"
        ), mock.patch.object(
            maintenance, "record_maintenance_verb_run"
        ) as mocked_record_verb_run, mock.patch.object(
            maintenance, "latest_maintenance_run_at", return_value=None
        ), mock.patch.object(
            maintenance, "_trino_execute", return_value=[]
        ) as mocked_trino:
            maintenance._run_maintenance_for_table(
                cfg=maintenance.MaintenanceConfig.from_env(),
                table_name="bronze.orders",
                policy={"snapshotsToKeep": None, "compactSmallFiles": True, "schedule": None, "orphanAgeHours": None},
                policy_fetch_error=None,
            )

        self.assertEqual(mocked_trino.call_count, 2)
        sqls = [c.args[1] for c in mocked_trino.call_args_list]
        self.assertTrue(any("EXECUTE expire_snapshots" in s for s in sqls))
        self.assertTrue(any("EXECUTE optimize" in s for s in sqls))
        mocked_record_verb_run.assert_called_once()
        verb_runs = mocked_record_verb_run.call_args.kwargs["verb_runs"]
        self.assertEqual(
            [v["verb"] for v in verb_runs if v["outcome"] == "applied"],
            ["expire_snapshots", "optimize"],
        )

    def test_table_with_no_policy_row_runs_default_snapshot_expiry_via_trino(self) -> None:
        with mock.patch.object(
            maintenance, "probe_expire_snapshots_skip", return_value="ch refuses"
        ), mock.patch.object(
            maintenance, "run_remove_orphan_files", return_value={"table_name": "bronze.orders"}
        ), mock.patch.object(
            maintenance, "measure_snapshot_growth",
            return_value={"measured": False, "snapshot_count": 0, "metadata_log_count": 0},
        ), mock.patch.object(
            maintenance, "record_maintenance_run"
        ) as mocked_record_run, mock.patch.object(
            maintenance, "record_maintenance_verb_run"
        ) as mocked_record_verb_run, mock.patch.object(
            maintenance, "_trino_execute", return_value=[]
        ) as mocked_trino:
            maintenance._run_maintenance_for_table(
                cfg=maintenance.MaintenanceConfig.from_env(),
                table_name="bronze.orders",
                policy=None,
                policy_fetch_error=None,
            )

        mocked_trino.assert_called_once_with(
            mock.ANY,
            'ALTER TABLE iceberg.bronze."orders" EXECUTE expire_snapshots(retention_threshold => \x277d\x27)',
        )
        mocked_record_verb_run.assert_called_once()
        verb_runs = mocked_record_verb_run.call_args.kwargs["verb_runs"]
        self.assertEqual(
            [v["verb"] for v in verb_runs if v["outcome"] == "applied"], ["expire_snapshots"]
        )
        skipped_verbs = mocked_record_run.call_args.kwargs["skipped_verbs"]
        self.assertEqual(
            skipped_verbs,
            [
                "OPTIMIZE (returns OK on 26.8 but does not bin-pack — not a "
                "compaction remedy; Trino owns that, see ADR 0009)",
            ],
        )

    def test_the_snapshot_expiry_runs_before_the_orphan_file_removal(self) -> None:
        """Decision 4 of OPS-20: the expiry runs FIRST, so the files the
        expired snapshots referenced are orphans by the time
        `remove_orphan_files` runs. The orphan verb itself is unchanged —
        only the order moved."""
        calls: list[str] = []

        def note_trino(_cfg, _sql):
            calls.append("expire_snapshots")
            return []

        def note_orphan(_cfg, _table_name, *, dry_run):
            calls.append(f"remove_orphan_files(dry_run={dry_run})")
            return {"table_name": "bronze.orders"}

        with mock.patch.object(
            maintenance, "probe_expire_snapshots_skip", return_value="ch refuses"
        ), mock.patch.object(
            maintenance, "_trino_execute", side_effect=note_trino
        ), mock.patch.object(
            maintenance, "run_remove_orphan_files", side_effect=note_orphan
        ), mock.patch.object(
            maintenance, "measure_snapshot_growth",
            return_value={"measured": False, "snapshot_count": 0, "metadata_log_count": 0},
        ), mock.patch.object(
            maintenance, "record_maintenance_run"
        ), mock.patch.object(
            maintenance, "record_maintenance_verb_run"
        ), mock.patch.object(
            maintenance, "latest_maintenance_run_at", return_value=None
        ):
            maintenance._run_maintenance_for_table(
                cfg=_cfg_with_catalog_identity(),
                table_name="bronze.orders",
                policy={"snapshotsToKeep": 2, "compactSmallFiles": False, "schedule": None, "orphanAgeHours": None},
                policy_fetch_error=None,
            )

        self.assertIn("expire_snapshots", calls)
        self.assertIn("remove_orphan_files(dry_run=False)", calls)
        self.assertLess(
            calls.index("expire_snapshots"),
            calls.index("remove_orphan_files(dry_run=True)"),
        )

    def test_a_401_policy_fetch_failure_is_a_whole_run_failure_not_a_per_table_skip(
        self,
    ) -> None:
        # PART C's `list_bronze_tables` raises a `Failure` from
        # `_fetch_policy_index` BEFORE yielding any DynamicOutput -- the
        # whole-run shape is preserved. Verify by driving the fan-out
        # op directly with a `_fetch_policy_index` that raises and
        # asserting `probe_expire_snapshots_skip` (the first
        # per-table body call) is never reached.
        with mock.patch.object(
            maintenance, "discover_bronze_tables", return_value=["bronze.orders"]
        ), mock.patch.object(
            maintenance, "_ensure_catalog_database"
        ), mock.patch.object(
            maintenance, "probe_expire_snapshots_skip"
        ) as mocked_probe, mock.patch.object(
            maintenance,
            "_fetch_policy_index",
            side_effect=maintenance.Failure("maintenance policy fetch failed with HTTP 401"),
        ):
            with self.assertRaises(maintenance.Failure):
                list(maintenance.list_bronze_tables(self._fake_context()))
        mocked_probe.assert_not_called()

    def test_a_5xx_policy_fetch_failure_is_recorded_and_orphan_removal_still_runs(
        self,
    ) -> None:
        with mock.patch.object(
            maintenance, "probe_expire_snapshots_skip", return_value="ch refuses"
        ), mock.patch.object(
            maintenance, "run_remove_orphan_files", return_value={"table_name": "bronze.orders"}
        ) as mocked_rof, mock.patch.object(
            maintenance, "measure_snapshot_growth",
            return_value={"measured": False, "snapshot_count": 0, "metadata_log_count": 0},
        ), mock.patch.object(
            maintenance, "record_maintenance_run"
        ) as mocked_record_run, mock.patch.object(
            maintenance, "record_maintenance_verb_run"
        ), mock.patch.object(
            maintenance, "_trino_execute"
        ) as mocked_trino:
            maintenance._run_maintenance_for_table(
                cfg=maintenance.MaintenanceConfig.from_env(),
                table_name="bronze.orders",
                policy=None,
                policy_fetch_error="HTTPError",
            )

        self.assertEqual(mocked_rof.call_count, 2)  # dry-run + applied, unconditional
        mocked_trino.assert_not_called()
        skipped_verbs = mocked_record_run.call_args.kwargs["skipped_verbs"]
        self.assertIn("policy fetch failed: HTTPError", skipped_verbs)


class ExpireSnapshotsProbeOutcomeTest(unittest.TestCase):
    """The ClickHouse probe's outcome must be SURFACED, never dropped.

    `probe_expire_snapshots_skip` returns ClickHouse's OWN error body for
    its own `expire_snapshots` verb. That text is upstream error text: it
    may go to the operator-facing Dagster log, and NOWHERE else — not into
    `skipped_verbs`, not into a `maintenance_verb_run` / `maintenance_run`
    row, and so not into any API response
    (`GET /api/governance/maintenance`,
    `GET /api/lakehouse/tables/{ns}/{table}/maintenance`).

    The probe's own contract says so: "the caller logs whatever this
    returns either way". Before this change the caller computed the value,
    handed it to a parameter nobody read, and dropped it — a ClickHouse
    query per table per run whose result nobody could see.
    """

    # The marker is a distinctive fragment of ClickHouse's error body, so
    # "the probe's outcome is nowhere in what was recorded" is a precise
    # assertion and not a coincidence with the rest of the run's text.
    MARKER = "Code: 48. NOT_SUPPORTED"
    PROBE_OUTCOME = (
        "unsupported this run — real ClickHouse error: "
        f"{MARKER}: Iceberg expire_snapshots is not supported for Iceberg "
        "tables backed by a transactional catalog"
    )

    def _run(self, context=None) -> tuple[dict, list]:
        """The per-table body with every I/O boundary stubbed, plus what it
        recorded (`maintenance_run` and `maintenance_verb_run`)."""
        recorded_run: dict = {}
        recorded_verb_runs: list = []
        with mock.patch.object(
            maintenance, "probe_expire_snapshots_skip", return_value=self.PROBE_OUTCOME
        ), mock.patch.object(
            maintenance, "run_remove_orphan_files", return_value={"table_name": "bronze.orders"}
        ), mock.patch.object(
            maintenance, "measure_snapshot_growth",
            return_value={"measured": False, "snapshot_count": 0, "metadata_log_count": 0},
        ), mock.patch.object(
            maintenance, "record_maintenance_run",
            side_effect=lambda **kw: recorded_run.update(kw),
        ), mock.patch.object(
            maintenance, "record_maintenance_verb_run",
            side_effect=lambda **kw: recorded_verb_runs.append(kw),
        ), mock.patch.object(
            maintenance, "latest_maintenance_run_at", return_value=None
        ), mock.patch.object(
            maintenance, "_trino_execute", return_value=[]
        ):
            maintenance._run_maintenance_for_table(
                cfg=_cfg(),
                table_name="bronze.orders",
                policy=None,
                policy_fetch_error=None,
                context=context,
            )
        return recorded_run, recorded_verb_runs

    def test_the_probe_outcome_is_logged_for_the_operator(self) -> None:
        """The probe's outcome reaches the Dagster log: the caller logs
        whatever the probe returns, either way."""
        log = mock.Mock()
        context = mock.Mock()
        context.log = log

        self._run(context=context)

        logged = " ".join(str(call.args[0]) for call in log.info.call_args_list)
        self.assertIn(self.PROBE_OUTCOME, logged)

    def test_the_probe_outcome_never_reaches_a_recorded_row(self) -> None:
        """It is ClickHouse's own error body: it may not be recorded in
        anything the API serves."""
        log = mock.Mock()
        context = mock.Mock()
        context.log = log

        recorded_run, recorded_verb_runs = self._run(context=context)

        recorded = repr(recorded_run) + repr(recorded_verb_runs)
        self.assertNotIn(self.MARKER, recorded)

    def test_the_op_threads_its_own_context_into_the_per_table_body(self) -> None:
        """The production path: the mapped `maintain_bronze_table` op hands
        its own context down, so the probe's outcome reaches the run log of
        the step that ran it (the helper alone cannot log without one)."""
        context = build_op_context()
        records: list[logging.LogRecord] = []

        class _Capture(logging.Handler):
            def emit(self, record: logging.LogRecord) -> None:
                records.append(record)

        context.log.addHandler(_Capture())
        with mock.patch.object(
            maintenance, "probe_expire_snapshots_skip", return_value=self.PROBE_OUTCOME
        ), mock.patch.object(
            maintenance, "run_remove_orphan_files", return_value={"table_name": "bronze.orders"}
        ), mock.patch.object(
            maintenance, "measure_snapshot_growth",
            return_value={"measured": False, "snapshot_count": 0, "metadata_log_count": 0},
        ), mock.patch.object(
            maintenance, "record_maintenance_run"
        ), mock.patch.object(
            maintenance, "record_maintenance_verb_run"
        ), mock.patch.object(
            maintenance, "latest_maintenance_run_at", return_value=None
        ), mock.patch.object(
            maintenance, "_trino_execute", return_value=[]
        ):
            maintenance.maintain_bronze_table(
                context,
                {
                    "table_name": "bronze.orders",
                    "policy": None,
                    "policy_fetch_error": None,
                },
            )

        logged = " ".join(record.getMessage() for record in records)
        self.assertIn(self.PROBE_OUTCOME, logged)


class BronzeMaintenanceFanOutTest(unittest.TestCase):
    """PART C of `parts/1b-dagster-one-op-per-unit-and-retries.md`:
    `bronze_maintenance_job` is rebuilt as `list_bronze_tables` (fan-out,
    runs setup once) -> `maintain_bronze_table` (mapped, per-table body
    in its own failure unit) -> `summarize_bronze_maintenance` (collect).
    The setup work (`_ensure_catalog_database`, `_fetch_policy_index`)
    stays in the fan-out op -- one network round-trip per run, not one
    per table. A failed table's step fails without taking the others
    down, and zero tables yields a still-successful run."""

    def test_a_non_ascii_letter_is_replaced_with_an_underscore_for_dagsters_charset(self) -> None:
        """Dagster's `check_valid_chars` requires `^[A-Za-z0-9_]+$`.
        `str.isalnum()` is True for non-ASCII letters (`"é".isalnum()`,
        `"²".isalnum()`), so the helper must use the exact ASCII rule
        -- not `c.isalnum()`. This pins that contract: the helper turns
        a non-ASCII letter into `_`, not into itself. Bronze tables
        happen to be ASCII-clean today, but a future table name that
        violates the rule would otherwise produce a mapping key
        Dagster silently rejects."""
        from dispar_orchestrate import maintenance

        self.assertEqual(maintenance._sanitize_mapping_key("café"), "caf_")
        self.assertEqual(maintenance._sanitize_mapping_key("x²"), "x_")
        self.assertEqual(maintenance._sanitize_mapping_key("plain_ok"), "plain_ok")

    def test_two_tables_yield_two_mapped_steps_with_their_keys(self) -> None:
        with mock.patch.object(
            maintenance, "_ensure_catalog_database"
        ), mock.patch.object(
            maintenance, "discover_bronze_tables",
            return_value=["bronze.g3a_orders", "bronze.g3a_invoices"],
        ), mock.patch.object(
            maintenance, "_fetch_policy_index", return_value=({}, None)
        ), mock.patch.object(
            maintenance, "_run_maintenance_for_table",
            return_value={"table_name": "x", "dry_run": {}, "applied": {}, "skipped_verbs": [], "snapshot_growth": {"measured": False}},
        ):
            result = maintenance.bronze_maintenance_job.execute_in_process(
                raise_on_error=False,
            )
        self.assertTrue(result.success)
        succeeded = {
            e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"
        }
        self.assertIn("maintain_bronze_table[bronze_g3a_orders]", succeeded)
        self.assertIn("maintain_bronze_table[bronze_g3a_invoices]", succeeded)
        # The summarize step ran with a count of 2.
        self.assertIn("summarize_bronze_maintenance", succeeded)

    def test_one_failing_table_does_not_stop_the_others_rows_being_written(
        self,
    ) -> None:
        """One table's per-table body raises -> only that mapped step
        fails; the other table's `_run_maintenance_for_table` (and
        therefore its `record_maintenance_run` row) still runs."""
        def fake_per_table(cfg, table_name, policy, policy_fetch_error, context=None):
            if "orders" in table_name:
                # `allow_retries=False` -- the canonical pattern from
                # PART D, so the test doesn't burn the 60s default
                # retry budget of `maintain_bronze_table`.
                raise maintenance.Failure(
                    "simulated per-table failure", allow_retries=False
                )
            return {
                "table_name": table_name,
                "dry_run": {},
                "applied": {},
                "skipped_verbs": [],
                "snapshot_growth": {"measured": False},
            }

        with mock.patch.object(
            maintenance, "_ensure_catalog_database"
        ), mock.patch.object(
            maintenance, "discover_bronze_tables",
            return_value=["bronze.orders", "bronze.invoices"],
        ), mock.patch.object(
            maintenance, "_fetch_policy_index", return_value=({}, None)
        ), mock.patch.object(
            maintenance, "_run_maintenance_for_table", side_effect=fake_per_table
        ):
            result = maintenance.bronze_maintenance_job.execute_in_process(
                raise_on_error=False,
            )
        self.assertFalse(result.success)
        failed = {
            e.step_key for e in result.all_events if e.event_type_value == "STEP_FAILURE"
        }
        succeeded = {
            e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"
        }
        # The non-orders table still succeeded; the orders step failed.
        self.assertIn("maintain_bronze_table[bronze_orders]", failed)
        self.assertIn("maintain_bronze_table[bronze_invoices]", succeeded)

    def test_zero_tables_yields_zero_mapped_steps_and_the_job_still_succeeds(
        self,
    ) -> None:
        """A fresh catalog with no Bronze tables must run the job to
        completion (the schedule is `default_status=RUNNING`, so a
        zero-table result is the steady-state on a brand-new stack).
        `summarize_bronze_maintenance` collects an empty list and the
        job succeeds -- no mapped steps appear."""
        with mock.patch.object(
            maintenance, "_ensure_catalog_database"
        ), mock.patch.object(
            maintenance, "discover_bronze_tables", return_value=[]
        ), mock.patch.object(
            maintenance, "_fetch_policy_index", return_value=({}, None)
        ):
            result = maintenance.bronze_maintenance_job.execute_in_process(
                raise_on_error=False,
            )
        self.assertTrue(result.success)
        succeeded = {
            e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"
        }
        self.assertIn("summarize_bronze_maintenance", succeeded)
        self.assertIn("list_bronze_tables", succeeded)
        # No mapped maintain_bronze_table steps ran -- zero tables in,
        # zero mapped steps out.
        for key in succeeded:
            self.assertFalse(key.startswith("maintain_bronze_table["), key)


if __name__ == "__main__":
    unittest.main()
