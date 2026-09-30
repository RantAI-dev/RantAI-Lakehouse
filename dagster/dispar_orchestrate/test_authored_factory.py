"""Unit tests for `authored_factory.py`'s job factory (WS4 items E1/E3,
grand plan §6). Same non-negotiable property `test_agent_runs.py`
establishes for `agent_runs.py`: an unreachable/unauthenticated
`lakehouse-api` must leave the whole Dagster code location loadable, with
zero authored jobs and a loud warning -- never an import-time exception.

No real network: `requests.get` is monkeypatched with canned responses/
exceptions, the same style `test_agent_runs.py` uses. No real ClickHouse:
`authored_factory._ch_exec`/`_ch_query_json` are monkeypatched, the same
style `test_maintenance.py` uses for `_ch_query`.

Run with: `~/.cache/rantai-dagster-venv/bin/python -m pytest
dagster/dispar_orchestrate/test_authored_factory.py -v`
"""

from __future__ import annotations

import unittest
from unittest import mock

import requests
from dagster import Backoff, DefaultScheduleStatus, Jitter, RetryPolicy, build_op_context

from dispar_orchestrate import authored_factory, authored_transforms
from dispar_orchestrate.bronze_catalog import ClickHouseTarget


def _cfg(run_token: str = "a-real-token") -> authored_factory.AuthoredPipelineConfig:
    return authored_factory.AuthoredPipelineConfig(
        api_url="http://lakehouse-api.invalid:8080", run_token=run_token,
    )


class HeadersTest(unittest.TestCase):
    def test_empty_token_sends_no_headers(self) -> None:
        self.assertEqual(authored_factory._headers(_cfg(run_token="")), {})

    def test_real_token_sends_both_headers(self) -> None:
        headers = authored_factory._headers(_cfg(run_token="tok"))
        self.assertEqual(headers["Authorization"], "Bearer tok")
        self.assertEqual(headers["x-run-token"], "tok")


class FetchAuthoredPipelinesTest(unittest.TestCase):
    def test_empty_token_returns_empty_list_with_no_http_call(self) -> None:
        with mock.patch.object(authored_factory.requests, "get") as mocked_get:
            result = authored_factory._fetch_authored_pipelines(_cfg(run_token=""))
        self.assertEqual(result, [])
        mocked_get.assert_not_called()

    def test_unreachable_api_returns_empty_list_and_does_not_raise(self) -> None:
        with mock.patch.object(
            authored_factory.requests, "get", side_effect=requests.ConnectionError("refused"),
        ):
            result = authored_factory._fetch_authored_pipelines(_cfg())
        self.assertEqual(result, [])

    def test_non_200_returns_empty_list(self) -> None:
        """Covers a 403 the `authored-pipeline-scheduler`
        identity (`pipeline:write` only) gets from `pipeline:read`-gated
        `GET /api/pipelines` -- see `authored_factory.py`'s module doc."""
        resp = mock.Mock(status_code=403)
        resp.raise_for_status.side_effect = requests.HTTPError("403")
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            self.assertEqual(authored_factory._fetch_authored_pipelines(_cfg()), [])

    def test_non_json_body_returns_empty_list(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.raise_for_status.return_value = None
        resp.json.side_effect = ValueError("not json")
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            self.assertEqual(authored_factory._fetch_authored_pipelines(_cfg()), [])

    def test_non_list_pipelines_field_returns_empty_list(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.raise_for_status.return_value = None
        resp.json.return_value = {"pipelines": "not-a-list"}
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            self.assertEqual(authored_factory._fetch_authored_pipelines(_cfg()), [])

    def test_a_pipeline_missing_the_ready_status_is_excluded(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [
            {"id": "pl-a", "status": "draft", "definition": {}},
            {"id": "pl-b", "status": "ready", "definition": {
                "sourceZone": "silver", "sourceTable": "orders",
                "targetZone": "serving", "targetTable": "orders_clean",
                "transforms": ["select(id,name)"], "fbicEnabled": False,
                "incrementalColumn": None, "connectorId": None,
            }},
        ]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            result = authored_factory._fetch_authored_pipelines(_cfg())
        self.assertEqual([p["id"] for p in result], ["pl-b"])

    def test_a_ready_pipeline_with_no_definition_is_excluded(self) -> None:
        """A Dagster-job row unioned into the same `/api/pipelines` list
        (`dagster_pipeline_row`) has no `definition` at all -- must never
        be mistaken for a ready authored pipeline."""
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [
            {"id": "bronze_ingest_job", "status": "unknown", "kind": "batch"},
        ]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            result = authored_factory._fetch_authored_pipelines(_cfg())
        self.assertEqual(result, [])


def _ready_pipeline(**overrides: object) -> dict[str, object]:
    definition = {
        "sourceZone": "silver", "sourceTable": "orders",
        "targetZone": "serving", "targetTable": "orders_clean",
        "transforms": ["select(id,name)"], "fbicEnabled": False,
        "incrementalColumn": None, "connectorId": None,
        # Plan 1c (R2, day-1): the per-pipeline retry cap. Absent here
        # so legacy callers of `_ready_pipeline()` continue to assert
        # the migration's `DEFAULT 2` shape (matching the store's own
        # `tests/pipelines.rs::create_pipeline_defaults_max_retries_to_two`).
        "maxRetries": 2,
    }
    definition.update(overrides)
    return {"id": "pl-dedupe-select-abc123", "status": "ready", "definition": definition}


class BuildAuthoredJobsTest(unittest.TestCase):
    def test_build_authored_jobs_produces_one_job_per_ready_pipeline(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [_ready_pipeline()]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            jobs = authored_factory.build_authored_jobs(_cfg())
        self.assertEqual(len(jobs), 1)
        self.assertEqual(jobs[0].name, "authored__pl_dedupe_select_abc123")

    def test_dagster_safe_name_replaces_every_non_alnum_underscore_char(self) -> None:
        self.assertEqual(
            authored_factory._dagster_safe_name("pl-a.b c-1"), "pl_a_b_c_1",
        )


class BuildSelectSqlTest(unittest.TestCase):
    def test_select_rename_cast_filter_dedupe_compose_into_one_statement(self) -> None:
        transforms = [
            authored_transforms.parse_transform("select(id,amount)"),
            authored_transforms.parse_transform("rename(amount,amount_usd)"),
            authored_transforms.parse_transform("cast(amount,Float64)"),
            authored_transforms.parse_transform("filter(status = 'active')"),
            authored_transforms.parse_transform("dedupe(id)"),
        ]
        sql = authored_factory._build_select_sql("silver.`orders`", transforms)
        self.assertEqual(
            sql,
            "SELECT id, CAST(amount AS Float64) AS amount_usd FROM silver.`orders` "
            "WHERE status = 'active' ORDER BY id LIMIT 1 BY id",
        )

    def test_no_transforms_selects_star_with_no_where_or_order(self) -> None:
        sql = authored_factory._build_select_sql("silver.`orders`", [])
        self.assertEqual(sql, "SELECT * FROM silver.`orders`")


class OpForPipelineTest(unittest.TestCase):
    """Executes the built op's own function directly (not through a full
    Dagster job run) -- same level `test_maintenance.py` exercises its ops
    at: a real `dagster.build_op_context()`, since `@op`-decorated
    functions require a genuine execution context for direct invocation
    and reject a bare mock (see `test_maintenance.py`'s own note on this).
    `_ch_exec`/`_ch_query_json` are monkeypatched, the way
    `test_maintenance.py` monkeypatches `_ch_query`."""

    def test_ready_pipeline_reads_transforms_writes_and_reports_real_rows(self) -> None:
        pipeline = _ready_pipeline()
        run_fn = authored_factory._op_for_pipeline(pipeline)

        counts = iter([[{"n": "0"}], [{"n": "3"}]])  # before, after
        exec_calls: list[str] = []
        with mock.patch.object(authored_factory, "_ch_exec", side_effect=lambda t, s: exec_calls.append(s)), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")):
            result = run_fn(build_op_context())

        self.assertEqual(result["rows"], 3)
        self.assertEqual(result["skipped_verbs"], [])
        self.assertTrue(any("INSERT INTO serving.`orders_clean`" in c for c in exec_calls))

    def test_fbic_enabled_pipeline_records_a_skipped_verb_not_a_call(self) -> None:
        pipeline = _ready_pipeline(fbicEnabled=True)
        run_fn = authored_factory._op_for_pipeline(pipeline)

        counts = iter([[{"n": "0"}], [{"n": "1"}]])
        with mock.patch.object(authored_factory, "_ch_exec"), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")):
            result = run_fn(build_op_context())

        self.assertEqual(result["skipped_verbs"], [authored_factory.FBIC_UNSUPPORTED_REASON])

    def test_a_rejected_transform_raises_a_non_retryable_failure_wrapping_transform_error(
        self,
    ) -> None:
        """PART D: `TransformError` from `parse_transform` is a config
        rejection -- the transform string is wrong, retrying with the
        same string gets the same answer. The `_run` op body wraps it
        in `Failure(allow_retries=False)`, the original `TransformError`
        is preserved as `__cause__` (the message names the offending
        field), and the run fails on the first attempt without burning
        60s on retries that cannot succeed."""
        from dagster import Failure

        pipeline = _ready_pipeline(transforms=["exec(rm -rf /)"])
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with self.assertRaises(Failure) as ctx:
            run_fn(build_op_context())
        self.assertFalse(ctx.exception.allow_retries)
        self.assertIsInstance(ctx.exception.__cause__, authored_transforms.TransformError)

    def test_connector_sourced_pipeline_raises_a_non_retryable_failure_wrapping_authored_job_error(
        self,
    ) -> None:
        """PART D: an authored pipeline that names a `connectorId` is
        not implemented by this build (see `CONNECTOR_SOURCE_UNSUPPORTED_REASON`)
        -- a missing-feature gap, never a transient failure. Wrapped in
        `Failure(allow_retries=False)` so the run fails loudly on the
        first attempt."""
        from dagster import Failure

        pipeline = _ready_pipeline(connectorId="conn-1")
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with self.assertRaises(Failure) as ctx:
            run_fn(build_op_context())
        self.assertFalse(ctx.exception.allow_retries)
        self.assertIn("connector-sourced", str(ctx.exception))
        self.assertIsInstance(ctx.exception.__cause__, authored_factory.AuthoredJobError)

    def test_an_unsafe_identifier_raises_a_non_retryable_failure_and_reaches_no_sql(
        self,
    ) -> None:
        """PART D: an unsafe ClickHouse identifier in any of the four
        identifier fields is a config rejection -- retrying with the
        same string gets the same answer. Wrapped in
        `Failure(allow_retries=False)`, and `_ch_exec` is never
        called (the guard fires before any SQL is built)."""
        from dagster import Failure

        pipeline = _ready_pipeline(targetTable="orders; DROP TABLE x")
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with mock.patch.object(authored_factory, "_ch_exec") as mocked_exec:
            with self.assertRaises(Failure) as ctx:
                run_fn(build_op_context())
        self.assertFalse(ctx.exception.allow_retries)
        self.assertIsInstance(ctx.exception.__cause__, authored_factory.AuthoredJobError)
        mocked_exec.assert_not_called()


class RunnableFetchTest(unittest.TestCase):
    def test_the_factory_reads_the_runnable_route_and_keeps_paused_pipelines(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [
            _ready_pipeline(),
            {**_ready_pipeline(), "id": "pl-paused-1", "status": "paused"},
            {**_ready_pipeline(), "id": "pl-draft-1", "status": "draft"},
        ]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp) as mocked_get:
            result = authored_factory._fetch_authored_pipelines(_cfg())
        self.assertTrue(mocked_get.call_args.args[0].endswith("/api/pipelines/runnable"))
        self.assertEqual(
            [p["id"] for p in result], ["pl-dedupe-select-abc123", "pl-paused-1"]
        )


class AuthoredScheduleTest(unittest.TestCase):
    def _job(self, pipeline: dict[str, object]) -> object:
        return authored_factory.build_authored_job(pipeline)

    def test_a_ready_pipeline_with_a_cron_gets_a_running_schedule(self) -> None:
        pipeline = {**_ready_pipeline(), "schedule": "0 2 * * *"}
        schedule = authored_factory.build_authored_schedule(pipeline, self._job(pipeline))
        self.assertIsNotNone(schedule)
        self.assertEqual(schedule.name, "authored__pl_dedupe_select_abc123_schedule")
        self.assertEqual(schedule.cron_schedule, "0 2 * * *")
        self.assertEqual(schedule.default_status, DefaultScheduleStatus.RUNNING)

    def test_a_paused_pipeline_gets_a_stopped_schedule(self) -> None:
        pipeline = {**_ready_pipeline(), "status": "paused", "schedule": "*/15 * * * *"}
        schedule = authored_factory.build_authored_schedule(pipeline, self._job(pipeline))
        self.assertEqual(schedule.default_status, DefaultScheduleStatus.STOPPED)

    def test_an_on_demand_pipeline_gets_no_schedule(self) -> None:
        for label in ("manual", "On demand", "", None):
            pipeline = {**_ready_pipeline(), "schedule": label}
            self.assertIsNone(authored_factory.build_authored_schedule(pipeline, self._job(pipeline)))

    def test_a_cron_dagster_refuses_is_skipped_not_raised(self) -> None:
        pipeline = {**_ready_pipeline(), "schedule": "99 99 * * *"}
        self.assertIsNone(authored_factory.build_authored_schedule(pipeline, self._job(pipeline)))

    def test_jobs_and_schedules_come_from_one_fetch(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [
            {**_ready_pipeline(), "schedule": "0 2 * * *"},
            {**_ready_pipeline(), "id": "pl-manual-1", "schedule": "manual"},
        ]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp) as mocked_get:
            jobs, schedules = authored_factory.build_authored_definitions(_cfg())
        self.assertEqual(mocked_get.call_count, 1)
        self.assertEqual(len(jobs), 2)
        self.assertEqual([s.name for s in schedules], ["authored__pl_dedupe_select_abc123_schedule"])


class PerPipelineRetryPolicyTest(unittest.TestCase):
    """Plan 1c (R2, day-1): `_op_for_pipeline` reads each authored
    pipeline's `definition.maxRetries` (migration `0051_pipeline_max_retries.sql`)
    and passes it as `RetryPolicy.max_retries`, while keeping the
    store-wide `delay`/`backoff`/`jitter` from `op_metadata.DEFAULT_RETRY_POLICY`.
    Two boundaries to assert: `0` (the migration's floor; means
    "never retry") and `5` (the `CHECK` constraint's ceiling).

    The op the factory builds is itself a `dagster.OpDefinition`, and
    `OpDefinition.retry_policy` is the seam the test pins — the
    decorator argument is stored there verbatim and is read by
    `dagster`'s job executor at run time, so a wrong value here would
    surface as a real production-policy bug rather than a typing
    mistake."""

    def test_max_retries_zero_means_never_retry(self) -> None:
        # Plan 1c floor (`0..=5`): `maxRetries=0` is "never retry",
        # the row explicitly opts out of any self-healing. The op
        # builder still constructs a `RetryPolicy` rather than `None`,
        # so the rest of the policy vocabulary stays uniform.
        pipeline = _ready_pipeline(maxRetries=0)
        op = authored_factory._op_for_pipeline(pipeline)
        self.assertEqual(op.retry_policy.max_retries, 0)

    def test_max_retries_five_is_the_check_constraint_ceiling(self) -> None:
        # Plan 1c ceiling: `5` is the highest value the route layer
        # accepts (`!(0..=5).contains(...) -> 400`) and the column
        # CHECK permits. The op builder must surface it intact, with
        # the same `delay`/`backoff`/`jitter` as the default.
        pipeline = _ready_pipeline(maxRetries=5)
        op = authored_factory._op_for_pipeline(pipeline)
        self.assertEqual(op.retry_policy.max_retries, 5)
        base = RetryPolicy(
            max_retries=2, delay=30, backoff=Backoff.EXPONENTIAL, jitter=Jitter.PLUS_MINUS,
        )
        # The 1c override is ONLY the count, not delay/backoff/jitter —
        # a synchronised retry storm across all authored pipelines
        # cannot return because one of them widened the policy window.
        self.assertEqual(op.retry_policy.delay, base.delay)
        self.assertEqual(op.retry_policy.backoff, base.backoff)
        self.assertEqual(op.retry_policy.jitter, base.jitter)
