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
from unittest import mock

from dispar_orchestrate import maintenance


def _cfg() -> maintenance.MaintenanceConfig:
    return maintenance.MaintenanceConfig(
        ch=maintenance.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
        lakekeeper_catalog_uri="http://lakekeeper.invalid/catalog",
        lakekeeper_warehouse="default",
        rustfs_endpoint="http://rustfs.invalid:9000",
        ch_oauth_client_id="",
        ch_oauth_server_uri="",
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


if __name__ == "__main__":
    unittest.main()
