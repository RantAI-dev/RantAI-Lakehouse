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

import unittest
from datetime import datetime, timedelta, timezone
from unittest import mock

import requests
from dagster import build_op_context

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
    module doc's "Per-table policy + Trino verbs" section. A policy-fetch
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
    """Per-table `schedule` cadence for the Trino verbs only."""

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

    def test_no_policy_keeps_todays_clickhouse_probe_reason_and_makes_no_trino_call(
        self,
    ) -> None:
        with mock.patch.object(maintenance, "_trino_execute") as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", None, "Code: 48 ... transactional catalog",
                datetime.now(timezone.utc), None,
            )
        self.assertEqual(skip, "expire_snapshots (Code: 48 ... transactional catalog)")
        self.assertIsNone(verb_run)
        mocked.assert_not_called()

    def test_a_policy_with_no_snapshots_to_keep_behaves_like_no_policy(self) -> None:
        policy = {"snapshotsToKeep": None, "compactSmallFiles": False, "schedule": None}
        with mock.patch.object(maintenance, "_trino_execute") as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy, "ch probe reason",
                datetime.now(timezone.utc), None,
            )
        self.assertEqual(skip, "expire_snapshots (ch probe reason)")
        self.assertIsNone(verb_run)
        mocked.assert_not_called()

    def test_a_table_with_fewer_snapshots_than_the_keep_count_is_skipped(self) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        policy = {"snapshotsToKeep": 5, "schedule": None}
        with mock.patch.object(
            maintenance, "_trino_execute", return_value=[["2026-01-09 00:00:00.000 UTC"]]
        ) as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy, "unused", now, None
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
                _cfg(), "bronze", "orders", policy, "unused", now, None
            )
        self.assertIsNotNone(skip)
        self.assertIn("refused", skip or "")
        self.assertEqual(verb_run and verb_run["outcome"], "refused")
        self.assertEqual(mocked.call_count, 2)  # one read, one attempted ALTER — no retry

    def test_a_successful_expiry_is_not_also_recorded_as_skipped(self) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        policy = {"snapshotsToKeep": 1, "schedule": None}
        rows = [["2026-01-09 23:59:00.000 UTC"], ["2026-01-01 00:00:00.000 UTC"]]

        def fake_trino_execute(_cfg, sql):
            if "SELECT committed_at" in sql:
                return rows
            return []

        with mock.patch.object(maintenance, "_trino_execute", side_effect=fake_trino_execute):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy, "unused", now, None
            )
        self.assertIsNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "applied")

    def test_trino_being_unreachable_is_a_degraded_skip_not_a_failure(self) -> None:
        policy = {"snapshotsToKeep": 1, "schedule": None}
        with mock.patch.object(
            maintenance,
            "_trino_execute",
            side_effect=maintenance.TrinoUnavailableError("trino unreachable: connection refused"),
        ):
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy, "unused", datetime.now(timezone.utc), None
            )
        self.assertEqual(skip, "expire_snapshots/optimize skipped: trino unreachable")
        self.assertEqual(verb_run and verb_run["outcome"], "skipped")

    def test_a_weekly_schedule_still_within_its_window_skips_without_a_trino_call(
        self,
    ) -> None:
        now = datetime(2026, 1, 10, tzinfo=timezone.utc)
        policy = {"snapshotsToKeep": 1, "schedule": "weekly"}
        with mock.patch.object(maintenance, "_trino_execute") as mocked:
            skip, verb_run = maintenance._expire_snapshots_result(
                _cfg(), "bronze", "orders", policy, "unused", now, now - timedelta(days=1)
            )
        self.assertIsNotNone(skip)
        self.assertEqual(verb_run and verb_run["outcome"], "skipped")
        mocked.assert_not_called()


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
    """`run_bronze_maintenance`, exercising the per-table policy branch
    end to end with every I/O boundary stubbed — no network."""

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
            maintenance, "discover_bronze_tables", return_value=["bronze.orders"]
        ), mock.patch.object(
            maintenance, "_fetch_policy_index",
            return_value=(
                {("bronze", "orders"): {"snapshotsToKeep": None, "compactSmallFiles": True, "schedule": None, "orphanAgeHours": None}},
                None,
            ),
        ), mock.patch.object(
            maintenance, "_ensure_catalog_database"
        ), mock.patch.object(
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
            maintenance.run_bronze_maintenance(self._fake_context())

        mocked_trino.assert_called_once()
        sql = mocked_trino.call_args.args[1]
        self.assertIn("EXECUTE optimize", sql)
        mocked_record_verb_run.assert_called_once()
        verb_runs = mocked_record_verb_run.call_args.kwargs["verb_runs"]
        self.assertEqual(
            [v["verb"] for v in verb_runs if v["outcome"] == "applied"], ["optimize"]
        )

    def test_a_table_with_no_policy_row_makes_no_trino_call(self) -> None:
        with mock.patch.object(
            maintenance, "discover_bronze_tables", return_value=["bronze.orders"]
        ), mock.patch.object(
            maintenance, "_fetch_policy_index", return_value=({}, None)
        ), mock.patch.object(
            maintenance, "_ensure_catalog_database"
        ), mock.patch.object(
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
            maintenance, "_trino_execute"
        ) as mocked_trino:
            maintenance.run_bronze_maintenance(self._fake_context())

        mocked_trino.assert_not_called()
        mocked_record_verb_run.assert_not_called()
        skipped_verbs = mocked_record_run.call_args.kwargs["skipped_verbs"]
        self.assertEqual(
            skipped_verbs,
            [
                "expire_snapshots (ch refuses)",
                "OPTIMIZE (returns OK on 26.8 but does not bin-pack — not a "
                "compaction remedy; Trino owns that, see ADR 0009)",
            ],
        )

    def test_a_401_policy_fetch_failure_raises_a_dagster_failure_before_any_table_runs(
        self,
    ) -> None:
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
                maintenance.run_bronze_maintenance(self._fake_context())
        mocked_probe.assert_not_called()

    def test_a_5xx_policy_fetch_failure_is_recorded_and_orphan_removal_still_runs(
        self,
    ) -> None:
        with mock.patch.object(
            maintenance, "discover_bronze_tables", return_value=["bronze.orders"]
        ), mock.patch.object(
            maintenance, "_fetch_policy_index", return_value=({}, "HTTPError")
        ), mock.patch.object(
            maintenance, "_ensure_catalog_database"
        ), mock.patch.object(
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
            maintenance.run_bronze_maintenance(self._fake_context())

        self.assertEqual(mocked_rof.call_count, 2)  # dry-run + applied, unconditional
        mocked_trino.assert_not_called()
        skipped_verbs = mocked_record_run.call_args.kwargs["skipped_verbs"]
        self.assertIn("policy fetch failed: HTTPError", skipped_verbs)


if __name__ == "__main__":
    unittest.main()
