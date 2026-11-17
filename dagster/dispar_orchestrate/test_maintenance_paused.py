"""Tests for `MAINTENANCE_PAUSED` (REL-2 S4-T3, decision D16).

The two jobs that DELETE files — the snapshot expiry and the
orphan-file removal — must not run while the maintenance is paused: the
local backup does not hold the warehouse files, so a restore brings the
catalogs back to the moment of the backup, and a job that deleted a
file the restored catalog still names would break the table.

While the pause is set the jobs skip, the reason is recorded in
`skipped_verbs`, and the run does not fail.

No network and no ClickHouse: the functions that would talk to either
are replaced by stand-ins that record that they were called — and the
tests fail when one of them is called at all.
"""

from __future__ import annotations

import unittest
from unittest import mock

from dispar_orchestrate import gold_export, maintenance


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


class MaintenancePausedTest(unittest.TestCase):
    """Whether the pause is on."""

    def test_the_pause_is_off_when_the_variable_is_unset(self) -> None:
        with mock.patch.dict("os.environ", {}, clear=True):
            self.assertFalse(maintenance.maintenance_paused())

    def test_the_pause_is_on_when_the_variable_is_true(self) -> None:
        with mock.patch.dict("os.environ", {"MAINTENANCE_PAUSED": "true"}, clear=True):
            self.assertTrue(maintenance.maintenance_paused())

    def test_anything_other_than_true_is_not_a_pause(self) -> None:
        for value in ("", "false", "True", "TRUE", "1", "yes", "no"):
            with self.subTest(value=value):
                with mock.patch.dict("os.environ", {"MAINTENANCE_PAUSED": value}, clear=True):
                    self.assertFalse(maintenance.maintenance_paused())


class PausedMaintenanceTest(unittest.TestCase):
    """The maintenance job while the pause is on."""

    def run_paused(self) -> tuple[dict, dict]:
        """The per-table run with the pause on, and what it recorded."""
        recorded: dict = {}
        with mock.patch.dict("os.environ", {"MAINTENANCE_PAUSED": "true"}, clear=True):
            with mock.patch.object(
                maintenance, "record_maintenance_run", side_effect=lambda **kw: recorded.update(kw)
            ):
                result = maintenance._run_maintenance_for_table(
                    _cfg(), "bronze.g3a_orders", None, None
                )
        return result, recorded

    def test_the_orphan_files_are_not_removed_while_the_pause_is_on(self) -> None:
        with mock.patch.object(
            maintenance, "run_remove_orphan_files", side_effect=AssertionError("a file was deleted")
        ) as removing:
            result, _recorded = self.run_paused()
        removing.assert_not_called()
        self.assertEqual(result["dry_run"], {})
        self.assertEqual(result["applied"], {})

    def test_no_snapshot_is_expired_while_the_pause_is_on(self) -> None:
        with mock.patch.object(
            maintenance,
            "_expire_snapshots_result",
            side_effect=AssertionError("a snapshot was expired"),
        ) as expiring:
            result, _recorded = self.run_paused()
        expiring.assert_not_called()
        self.assertNotIn("expire_snapshots", " ".join(result["skipped_verbs"]))

    def test_the_reason_is_recorded_in_the_skipped_verbs(self) -> None:
        result, recorded = self.run_paused()
        self.assertTrue(result["skipped_verbs"], "the reason was not recorded")
        self.assertIn("MAINTENANCE_PAUSED", result["skipped_verbs"][0])
        self.assertIn("no snapshot was expired", result["skipped_verbs"][0])
        self.assertEqual(recorded["skipped_verbs"], result["skipped_verbs"])

    def test_the_run_is_still_recorded_while_the_pause_is_on(self) -> None:
        _result, recorded = self.run_paused()
        self.assertEqual(recorded["table_name"], "bronze.g3a_orders")
        self.assertEqual(recorded["dry_run_metrics"], {})
        self.assertEqual(recorded["applied_metrics"], {})

    def test_the_maintenance_runs_normally_when_the_pause_is_off(self) -> None:
        with mock.patch.dict("os.environ", {}, clear=True):
            with mock.patch.object(
                maintenance, "record_maintenance_run", side_effect=lambda **kw: None
            ):
                with mock.patch.object(
                    maintenance, "run_remove_orphan_files", return_value={"deleted_data_files_count": 2}
                ) as removing:
                    with mock.patch.object(
                        maintenance, "measure_snapshot_growth", return_value={"measured": True}
                    ):
                        with mock.patch.object(
                            maintenance, "probe_expire_snapshots_skip", return_value="not supported"
                        ):
                            with mock.patch.object(
                                maintenance, "_expire_snapshots_result", return_value=(None, None)
                            ):
                                with mock.patch.object(
                                    maintenance, "_optimize_result", return_value=(None, None)
                                ):
                                    result = maintenance._run_maintenance_for_table(
                                        _cfg(), "bronze.g3a_orders", None, None
                                    )
        self.assertEqual(removing.call_count, 2, "the orphan files were not removed")
        self.assertEqual(result["dry_run"], {"deleted_data_files_count": 2})


class PausedGoldTrimTest(unittest.TestCase):
    """The Gold trim while the pause is on."""

    def test_the_gold_trim_does_not_delete_anything_while_the_pause_is_on(self) -> None:
        cfg = gold_export.GoldExportConfig(
            ch=gold_export.ClickHouseTarget(url="http://ch.invalid", user="default", password=""),
            api_url="http://lakehouse-api.invalid:8080",
            marts=["gold_export_smoke"],
            run_token="",
            rustfs_access_key="a",
            rustfs_secret_key="b",
        )
        recorded: dict = {}
        with mock.patch.dict("os.environ", {"MAINTENANCE_PAUSED": "true"}, clear=True):
            with mock.patch.object(
                gold_export,
                "record_gold_trim_run",
                side_effect=lambda _target, **kw: recorded.update(kw),
            ):
                with mock.patch.object(
                    gold_export,
                    "load_gold_catalog",
                    side_effect=AssertionError("the catalog was read"),
                ):
                    result = gold_export.run_trim_for_mart(cfg, "gold_export_smoke")
        self.assertEqual(result["status"], "skipped")
        self.assertEqual(result["reason"], "maintenance_paused")
        self.assertEqual(recorded["status"], "skipped")
        self.assertEqual(recorded["reason"], "maintenance_paused")


if __name__ == "__main__":
    unittest.main()
