"""Unit tests for `gold_export.py`'s scheduled trigger (WS6).

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
`test_alerts_run.py`'s WS0 fix hit the identical failure).

The fan-out tests drive the rebuilt `gold_export_job` end-to-end via
`execute_in_process` (no network, no real `lakehouse-api`), inspecting
the events Dagster emits so a failed mart is its own failed step and an
HTTP error still records a `maintenance_run` failure row.
"""

from __future__ import annotations

import unittest
from unittest import mock

from dagster import AssetMaterialization, DefaultScheduleStatus, build_op_context

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


class GoldExportFanOutTest(unittest.TestCase):
    """PART A of `parts/1b-dagster-one-op-per-unit-and-retries.md`:
    `gold_export_job` is rebuilt as `list_gold_marts` (fan-out) ->
    `export_gold_mart` (mapped, one step per mart) ->
    `summarize_gold_export` (collect). A failed mart is its own failed
    step, so re-running from failure redoes only what failed."""

    def test_two_marts_yield_two_mapped_steps_with_their_keys(self) -> None:
        from dispar_orchestrate import gold_export

        cfg = mock.Mock(marts=["mart_a", "mart_b"], ch=None)
        bodies = {"mart_a": {"rowsExported": 7}, "mart_b": {"rowsExported": 3}}
        with mock.patch.object(gold_export.GoldExportConfig, "from_env", return_value=cfg), \
                mock.patch.object(gold_export, "export_one_mart", side_effect=lambda c, m: bodies[m]), \
                mock.patch.object(gold_export, "record_maintenance_run") as mocked_record:
            result = gold_export_job.execute_in_process(raise_on_error=False)

        self.assertTrue(result.success)
        step_keys = {e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"}
        self.assertIn("export_gold_mart[mart_a]", step_keys)
        self.assertIn("export_gold_mart[mart_b]", step_keys)
        self.assertIn("list_gold_marts", step_keys)
        self.assertIn("summarize_gold_export", step_keys)
        # A real rows count yields a real materialization on that step.
        # Dagster's top-level `DagsterEvent` only carries typed handles;
        # the asset materialization's `metadata` dict lives on
        # `event_specific_data.materialization` -- accessed here instead
        # of `e.metadata`, which the event does not expose.
        materials = [
            (
                e.step_key,
                e.asset_key.to_user_string(),
                e.event_specific_data.materialization.metadata["rows"].value,
            )
            for e in result.all_events
            if e.event_type_value == "ASSET_MATERIALIZATION"
        ]
        self.assertIn(("export_gold_mart[mart_a]", "gold/mart_a", 7), materials)
        self.assertIn(("export_gold_mart[mart_b]", "gold/mart_b", 3), materials)
        # Two succeeded marts -> two maintenance_run rows, both with the
        # exact `rows_exported=...` message format the old op also wrote.
        rows_seen = [call.kwargs.get("skipped_verbs") for call in mocked_record.call_args_list]
        self.assertIn(["rows_exported=7"], rows_seen)
        self.assertIn(["rows_exported=3"], rows_seen)

    def test_an_http_error_is_bare_re_raised_not_wrapped_in_a_failure(self) -> None:
        """PART D of `parts/1b-dagster-one-op-per-unit-and-retries.md`:
        `export_gold_mart` does NOT wrap an HTTP error in
        `Failure(allow_retries=False)` -- it bare re-raises so
        `DEFAULT_RETRY_POLICY` can retry a transient 5xx (busy
        ClickHouse on the `lakehouse-api` side, the canonical case
        the plan names). The failure row is written first so the
        governance surface sees the failure immediately, not only
        after the policy's retries have all come up empty.

        Driving `export_gold_mart` directly via `build_op_context` is
        the only fast way to assert the not-wrapped contract: an
        in-process `execute_in_process` against `DEFAULT_RETRY_POLICY`
        would sleep 30s + 60s on a retryable HTTPError -- which is
        the policy's job, not the test's, and would push the suite
        past the minute."""
        import requests

        from dispar_orchestrate import gold_export

        cfg = mock.Mock(marts=["mart_a"], ch=None)
        err = requests.HTTPError("500 Server Error: backend down")
        err.response = mock.Mock(text="boom")
        with mock.patch.object(gold_export.GoldExportConfig, "from_env", return_value=cfg), \
                mock.patch.object(gold_export, "export_one_mart", side_effect=err), \
                mock.patch.object(gold_export, "record_maintenance_run") as mocked_record:
            with self.assertRaises(requests.HTTPError) as ctx:
                gold_export.export_gold_mart(build_op_context(), "mart_a")

        # The raised exception IS the HTTPError (not a `Failure`).
        self.assertIs(ctx.exception, err)
        # The failure row was recorded before the re-raise -- the
        # governance surface sees it immediately, not only after the
        # policy's retries have all come up empty.
        self.assertEqual(mocked_record.call_count, 1)
        skip = mocked_record.call_args.kwargs["skipped_verbs"]
        self.assertTrue(
            skip and skip[0].startswith("export_failed:"), skip
        )

    def test_a_failing_mart_fails_only_its_step_and_does_not_stop_the_others(
        self,
    ) -> None:
        """Fan-out isolation is a property of the fan-out itself, not
        of the error type -- `list_gold_marts()` -> `export_gold_mart[<key>].map(...)`
        -> `.collect()` runs each mapped step in its own failure unit,
        so one step's failure cannot block the others' success. The
        test drives `export_one_mart` to raise `Failure(allow_retries=False)`
        for one mart only so the mapped step fails on the first
        attempt -- this is the `DEFAULT_RETRY_POLICY` short-circuit
        path, not the policy's retry path, and is the only way to
        keep the suite runtime fast."""
        from dispar_orchestrate import gold_export

        cfg = mock.Mock(marts=["mart_a", "mart_b"], ch=None)

        def fake_export(_cfg, mart):
            if mart == "mart_a":
                raise gold_export.Failure(
                    "simulated per-mart failure", allow_retries=False
                )
            return {"rowsExported": 11}

        with mock.patch.object(gold_export.GoldExportConfig, "from_env", return_value=cfg), \
                mock.patch.object(gold_export, "export_one_mart", side_effect=fake_export), \
                mock.patch.object(gold_export, "record_maintenance_run") as mocked_record:
            result = gold_export_job.execute_in_process(raise_on_error=False)

        self.assertFalse(result.success)
        failed = {e.step_key for e in result.all_events if e.event_type_value == "STEP_FAILURE"}
        succeeded = {e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"}
        # Fan-out isolation: mart_a's mapped step failed; mart_b's
        # mapped step still succeeded. This is true regardless of the
        # error type, because the `.map(...)` form runs each step in
        # its own failure unit.
        self.assertIn("export_gold_mart[mart_a]", failed)
        self.assertIn("export_gold_mart[mart_b]", succeeded)
        # One maintenance_run row for the successful mart (the failed
        # mart's mapped step fails BEFORE this op reaches the
        # `record_maintenance_run` call -- that path is exercised by
        # `test_an_http_error_is_bare_re_raised_not_wrapped_in_a_failure`
        # above).
        recorded_skips = [call.kwargs.get("skipped_verbs") for call in mocked_record.call_args_list]
        self.assertIn(["rows_exported=11"], recorded_skips)

    def test_two_marts_that_collide_after_sanitization_raise_a_failure(self) -> None:
        """The mapping_key is the mart name with every character outside
        `[A-Za-z0-9_]` replaced by `_`. Two marts that collide after
        that mapping would mean a single Dagster step doing two
        mart's work -- which Dagster refuses, and this test refuses
        first, with a non-retryable `Failure` naming both."""
        from dispar_orchestrate import gold_export

        cfg = mock.Mock(marts=["mart-a", "mart.a"], ch=None)  # both -> mart_a
        with mock.patch.object(gold_export.GoldExportConfig, "from_env", return_value=cfg):
            # `list_gold_marts` is a `DynamicOut` op, so invoking it
            # returns a generator -- the body (including the
            # collision-time `raise Failure`) only executes when the
            # generator is exhausted, which `list()` here does.
            gen = gold_export.list_gold_marts(build_op_context())
            with self.assertRaises(gold_export.Failure) as ctx:
                list(gen)
        self.assertIn("mart-a", str(ctx.exception))
        self.assertIn("mart.a", str(ctx.exception))
        self.assertFalse(ctx.exception.allow_retries)

    def test_a_non_ascii_letter_is_replaced_with_an_underscore_for_dagsters_charset(self) -> None:
        """Dagster's `check_valid_chars` requires `^[A-Za-z0-9_]+$`.
        `str.isalnum()` is True for non-ASCII letters (`"é".isalnum()`,
        `"²".isalnum()`), so the helper must use the exact ASCII rule
        -- not `c.isalnum()`. This pins that contract: the helper turns
        a non-ASCII letter into `_`, not into itself."""
        from dispar_orchestrate import gold_export

        self.assertEqual(gold_export._sanitize_mapping_key("café"), "caf_")
        self.assertEqual(gold_export._sanitize_mapping_key("x²"), "x_")
        self.assertEqual(gold_export._sanitize_mapping_key("plain_ok"), "plain_ok")


if __name__ == "__main__":
    unittest.main()
