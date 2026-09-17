"""Unit tests for `gold_export.py`'s scheduled trigger — WS6 Task 5.

`gold_export_job` has been registered without a schedule since ADR 0010
(no route auth wiring existed for it, so a nightly 401 would have gone
unnoticed). This module tests only the schedule's own shape: that it
exists, fires on the documented cadence, and is registered
`DefaultScheduleStatus.RUNNING` — the same convention
`bronze_maintenance_schedule`, `capacity_snapshot_schedule`, and
`alerts_run_schedule` already follow, and for the same reason: a schedule
created stopped never fires until someone notices and toggles it in the
Dagster UI, and from the outside a stopped schedule looks identical to a
healthy one. `default_status` must be passed the `DefaultScheduleStatus`
enum member, not a bare string — the installed Dagster version raises
`dagster._core.errors.ParameterCheckError` for a string (confirmed by
running the check directly against this venv before writing this test;
`test_alerts_run.py`'s WS0 fix hit the identical failure)."""

from __future__ import annotations

import unittest

from dagster import DefaultScheduleStatus

from dispar_orchestrate.gold_export import gold_export_job, gold_export_schedule


class GoldExportScheduleTests(unittest.TestCase):
    def test_the_schedule_targets_the_gold_export_job(self) -> None:
        self.assertIs(gold_export_schedule.job, gold_export_job)

    def test_the_schedule_fires_daily_at_04_00(self) -> None:
        self.assertEqual(gold_export_schedule.cron_schedule, "0 4 * * *")

    def test_the_schedule_default_status_is_running_not_a_bare_string(self) -> None:
        # A bare "RUNNING" string satisfies neither `is` nor equality
        # against the enum member the way a careless `== "RUNNING"` check
        # might suggest — assert the real enum object, matching how the
        # installed Dagster version itself validates the constructor
        # argument (ParameterCheckError otherwise).
        self.assertIs(gold_export_schedule.default_status, DefaultScheduleStatus.RUNNING)


if __name__ == "__main__":
    unittest.main()
