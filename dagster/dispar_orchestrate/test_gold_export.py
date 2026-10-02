"""Unit tests for `gold_export.py`'s scheduled trigger (WS6).

This module tests the schedule's own shape — that it exists, fires on
the documented cadence, and is registered `DefaultScheduleStatus.RUNNING`
(the same convention `bronze_maintenance_schedule`,
`capacity_snapshot_schedule`, and `alerts_run_schedule` already follow,
and for the same reason: a schedule created stopped never fires until
someone notices and toggles it in the Dagster UI, and from the outside a
stopped schedule looks identical to a healthy one. `default_status` must
be passed the `DefaultScheduleStatus` enum member, not a bare string —
the installed Dagster version raises
`dagster._core.errors.ParameterCheckError` for a string (confirmed by
running the check directly against this venv before writing this test;
`test_alerts_run.py`'s WS0 fix hit the identical failure) — plus the
auth headers the scheduled request carries (gold-publish-per-mart plan
T1: the same `GOLD_EXPORT_RUN_TOKEN` value must go out as BOTH the
`Authorization: Bearer` credential that clears `auth_gate`'s
`RequiresAuth` floor — `bootstrap_gold_export_service` in
`lakehouse-api::main` mints the matching identity at API boot — and the
`x-run-token` header that satisfies `check_export_token`'s token branch).

The fan-out tests drive the rebuilt `gold_export_job` end-to-end via
`execute_in_process` (no network, no real `lakehouse-api`), inspecting
the events Dagster emits so a failed mart is its own failed step and an
HTTP error still records a `maintenance_run` failure row.

The sensor tests (gold-publish-per-mart plan T6) verify that an authored
pipeline's SUCCESS yields one `RunRequest` for `gold_export_job`, while
non-authored jobs (`gold_export_job` itself, maintenance, alerts, etc.)
yield nothing.
"""

from __future__ import annotations

import unittest
from unittest import mock
from unittest.mock import MagicMock

import requests

from dagster import (
    AssetMaterialization,
    DefaultScheduleStatus,
    RunRequest,
    build_op_context,
)

from dispar_orchestrate import gold_export
from dispar_orchestrate.gold_export import (
    GoldExportConfig,
    _headers,
    evaluate_authored_success,
    gold_export_job,
    gold_export_schedule,
)


def _cfg(run_token: str = "a-real-token") -> GoldExportConfig:
    return GoldExportConfig(
        ch=None,  # type: ignore[arg-type]
        api_url="http://lakehouse-api.invalid:8080",
        marts=[],
        run_token=run_token,
    )


class _FakeContext:
    """A minimal stand-in for Dagster's op `context`: only `log.info` is
    exercised by `export_one_mart`."""

    def __init__(self) -> None:
        self.log = MagicMock()


class GoldExportHeaderTests(unittest.TestCase):
    """gold-publish-per-mart plan T1: `auth_gate`'s `RequiresAuth` floor
    runs before `check_export_token`'s own token check, so the scheduled
    request must carry the shared token BOTH ways (bearer credential for
    the floor, `x-run-token` for the handler gate) — the exact shape
    `alerts_run.py::_headers` already uses."""

    def test_set_token_sends_both_bearer_and_run_token_headers(self) -> None:
        headers = _headers(_cfg(run_token="tok-123"))
        self.assertEqual(headers["Authorization"], "Bearer tok-123")
        self.assertEqual(headers["x-run-token"], "tok-123")

    def test_unset_token_sends_no_headers(self) -> None:
        self.assertEqual(_headers(_cfg(run_token="")), {})

    def test_export_one_mart_posts_with_both_headers_and_timeout(self) -> None:
        """The header pair actually flows through `export_one_mart`'s
        POST — not just through `_headers` in isolation — and the
        scheduled request carries `ifChanged=true` (plan T4): only the
        schedule measures before it writes; the manual console trigger
        sends no such parameter."""
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        response.json.return_value = {"rowsExported": 5}

        with mock.patch.object(
            gold_export.requests, "post", return_value=response
        ) as mocked_post:
            body = gold_export.export_one_mart(_cfg(run_token="tok-xyz"), "mart_a")

        mocked_post.assert_called_once_with(
            "http://lakehouse-api.invalid:8080/api/gold/export/mart_a",
            params={"ifChanged": "true"},
            headers={
                "x-run-token": "tok-xyz",
                "Authorization": "Bearer tok-xyz",
            },
            timeout=60,
        )
        self.assertEqual(body, {"rowsExported": 5})

    def test_enabled_publications_are_fetched_with_the_same_auth_headers(self) -> None:
        """`list_gold_marts` reads the enabled mart list from
        `GET /api/gold/publications` (plan T5), which is a
        `RequiresAuth` route — the same two-header credential the POST
        carries must go out on this read too."""
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        response.json.return_value = {"publications": [{"mart": "mart_a"}]}

        with mock.patch.object(
            gold_export.requests, "get", return_value=response
        ) as mocked_get:
            marts = gold_export._enabled_publication_marts(
                _cfg(run_token="tok-xyz")
            )

        mocked_get.assert_called_once_with(
            "http://lakehouse-api.invalid:8080/api/gold/publications",
            headers={
                "x-run-token": "tok-xyz",
                "Authorization": "Bearer tok-xyz",
            },
            timeout=30,
        )
        self.assertEqual(marts, ["mart_a"])

    def test_a_publications_response_without_a_list_is_a_non_retryable_failure(
        self,
    ) -> None:
        """A body without the `publications` list is a protocol error, not
        a transient blip — a retry would read the same wrong shape, so it
        is a `Failure(allow_retries=False)`. Crucially it raises at all:
        a malformed answer must never degrade into "zero marts", which
        would silently skip every export."""
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        response.json.return_value = {"unexpected": True}

        with mock.patch.object(gold_export.requests, "get", return_value=response):
            with self.assertRaises(gold_export.Failure) as ctx:
                gold_export._enabled_publication_marts(_cfg())

        self.assertFalse(ctx.exception.allow_retries)


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
                mock.patch.object(gold_export, "_enabled_publication_marts", return_value=[]), \
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
                mock.patch.object(gold_export, "_enabled_publication_marts", return_value=[]), \
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
        with mock.patch.object(gold_export.GoldExportConfig, "from_env", return_value=cfg), \
                mock.patch.object(gold_export, "_enabled_publication_marts", return_value=[]):
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

    def test_nothing_enabled_and_no_override_runs_a_successful_zero_step_job(
        self,
    ) -> None:
        """The empty union is a real, honest outcome, not an error: a
        fresh deployment with nothing switched on and no override must
        produce a run that SUCCEEDS with zero mapped export steps (the
        plan's "empty fan-out finishes" acceptance) — never a failed run,
        which would page someone for a nothing (Dagster 1.13.20)."""
        from dispar_orchestrate import gold_export

        cfg = mock.Mock(marts=[], ch=None)
        with mock.patch.object(gold_export.GoldExportConfig, "from_env", return_value=cfg), \
                mock.patch.object(gold_export, "_enabled_publication_marts", return_value=[]), \
                mock.patch.object(gold_export, "export_one_mart") as mocked_export:
            result = gold_export_job.execute_in_process(raise_on_error=False)

        self.assertTrue(result.success)
        export_steps = {
            e.step_key
            for e in result.all_events
            if e.event_type_value == "STEP_SUCCESS" and e.step_key.startswith("export_gold_mart[")
        }
        self.assertEqual(export_steps, set())
        mocked_export.assert_not_called()
        self.assertIn("summarize_gold_export", {
            e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"
        })

    def test_enabled_publications_and_the_env_override_union_deduplicated(self) -> None:
        """The scheduled mart list is the UNION of the publications the
        console enabled and `GOLD_EXPORT_MARTS`, deduplicated by exact
        mart name — a mart listed in both places is one step, not two."""
        from dispar_orchestrate import gold_export

        cfg = mock.Mock(marts=["mart_b", "mart_env_only"], ch=None)
        exported: list[str] = []

        def fake_export(_cfg, mart):
            exported.append(mart)
            return {"rowsExported": 1}

        with mock.patch.object(gold_export.GoldExportConfig, "from_env", return_value=cfg), \
                mock.patch.object(
                    gold_export,
                    "_enabled_publication_marts",
                    return_value=["mart_a", "mart_b"],
                ), \
                mock.patch.object(gold_export, "export_one_mart", side_effect=fake_export), \
                mock.patch.object(gold_export, "record_maintenance_run"):
            result = gold_export_job.execute_in_process(raise_on_error=False)

        self.assertTrue(result.success)
        self.assertEqual(sorted(exported), ["mart_a", "mart_b", "mart_env_only"])

    def test_a_skipped_mart_records_unchanged_and_emits_no_materialization(
        self,
    ) -> None:
        """A `{skipped: true}` answer is the schedule's everyday success
        path (plan T4): it gets its `maintenance_run` row with
        `skipped_verbs=["unchanged"]` — the honest ledger of "looked,
        nothing to do" — but no `AssetMaterialization`, because nothing
        was written to Iceberg, and the run still succeeds."""
        from dispar_orchestrate import gold_export

        cfg = mock.Mock(marts=["mart_a"], ch=None)
        with mock.patch.object(gold_export.GoldExportConfig, "from_env", return_value=cfg), \
                mock.patch.object(gold_export, "_enabled_publication_marts", return_value=[]), \
                mock.patch.object(
                    gold_export,
                    "export_one_mart",
                    return_value={"skipped": True, "reason": "unchanged since 2026-10-02"},
                ), \
                mock.patch.object(gold_export, "record_maintenance_run") as mocked_record:
            result = gold_export_job.execute_in_process(raise_on_error=False)

        self.assertTrue(result.success)
        self.assertIn("export_gold_mart[mart_a]", {
            e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"
        })
        self.assertEqual(mocked_record.call_args.kwargs["skipped_verbs"], ["unchanged"])
        self.assertEqual(
            [e for e in result.all_events if e.event_type_value == "ASSET_MATERIALIZATION"],
            [],
        )

    def test_a_failed_publications_fetch_fails_the_run_instead_of_exporting_nothing(
        self,
    ) -> None:
        """`GET /api/gold/publications` going down must never look like
        "nothing is enabled": the fetch raises bare (retryable by
        `DEFAULT_RETRY_POLICY`), the run fails, and no export step runs —
        the silent-degradation direction is the one outcome this op
        refuses."""
        from dispar_orchestrate import gold_export

        cfg = mock.Mock(marts=[], ch=None)
        err = requests.HTTPError("503 Server Error: api down")
        err.response = mock.Mock(text="boom")
        with mock.patch.object(gold_export.GoldExportConfig, "from_env", return_value=cfg), \
                mock.patch.object(gold_export, "_enabled_publication_marts", side_effect=err):
            gen = gold_export.list_gold_marts(build_op_context())
            with self.assertRaises(requests.HTTPError) as ctx:
                list(gen)

        self.assertIs(ctx.exception, err)

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


class GoldExportAfterAuthoredSensorTests(unittest.TestCase):
    """gold-publish-per-mart plan T6: `gold_export_after_authored_sensor`
    fires after an authored pipeline succeeds, triggering `gold_export_job`
    so the nightly cadence is the safety net, not the only trigger. With
    T4's `ifChanged=true` in place, unchanged marts cost one cheap check
    each. The sensor must NOT fire on `gold_export_job` itself or on
    maintenance, backup, alerts, capacity, agent, or ingest jobs.

    The tests drive `evaluate_authored_success` directly (the split-out
    body, same pattern `pipeline_events.py::evaluate_finished_run` uses
    for plan 1f) with a duck-typed context stand-in — no real sensor
    context or `DagsterInstance` needed."""

    class _FakeRun:
        def __init__(self, run_id: str, job_name: str) -> None:
            self.run_id = run_id
            self.job_name = job_name

    class _FakeContext:
        def __init__(self, run_id: str, job_name: str) -> None:
            self.dagster_run = GoldExportAfterAuthoredSensorTests._FakeRun(
                run_id, job_name
            )

    def test_an_authored_pipeline_success_yields_one_run_request_with_the_triggering_run_as_key(
        self,
    ) -> None:
        """The happy path: an `authored__pl_foo` job succeeds. The sensor
        yields one `RunRequest(run_key=<upstream run id>)` — Dagster's
        own dedup keeps a re-firing upstream success from launching
        `gold_export_job` twice for the same upstream run."""
        ctx = self._FakeContext(job_name="authored__pl_foo", run_id="upstream-abc")
        results = list(evaluate_authored_success(ctx))
        run_requests = [r for r in results if isinstance(r, RunRequest)]
        self.assertEqual(len(run_requests), 1)
        self.assertEqual(run_requests[0].run_key, "upstream-abc")

    def test_gold_export_job_itself_yields_nothing(self) -> None:
        """`gold_export_job`'s own success must never trigger another
        `gold_export_job` run — that would be an infinite chain."""
        ctx = self._FakeContext(job_name="gold_export_job", run_id="gold-run-1")
        results = list(evaluate_authored_success(ctx))
        run_requests = [r for r in results if isinstance(r, RunRequest)]
        self.assertEqual(run_requests, [])

    def test_a_non_authored_job_yields_nothing(self) -> None:
        """Every non-authored job — maintenance, alerts, capacity, agent,
        ingest, backup — must be a no-op. Only `authored__<id>` jobs
        trigger the export. The test drives a single representative
        non-authored name; the `startswith("authored__")` check covers
        all of them uniformly."""
        ctx = self._FakeContext(job_name="bronze_maintenance_job", run_id="mt-1")
        results = list(evaluate_authored_success(ctx))
        run_requests = [r for r in results if isinstance(r, RunRequest)]
        self.assertEqual(run_requests, [])


if __name__ == "__main__":
    unittest.main()
