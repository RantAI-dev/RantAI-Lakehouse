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


if __name__ == "__main__":
    unittest.main()
