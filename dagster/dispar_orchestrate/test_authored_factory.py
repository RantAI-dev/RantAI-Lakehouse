"""Unit tests for `authored_factory.py`'s job factory (WS4 items E1/E3,
grand plan §6). Same non-negotiable property `test_agent_runs.py`
establishes for `agent_runs.py`: an unreachable/unauthenticated
`lakehouse-api` must leave the whole Dagster code location loadable, with
zero authored jobs and a loud warning -- never an import-time exception.

No real network: `requests.get` is monkeypatched with canned responses/
exceptions, the same style `test_agent_runs.py` uses. No real ClickHouse:
`authored_factory._ch_exec`/`_ch_query_json` are monkeypatched, the same
style `test_maintenance.py` uses for `_ch_query`.

The final class, `TestDependencySensor`, exercises R3 plan 2a
(`build_authored_dependency_sensor`): a `run_status_sensor` per pipeline
with non-empty `depends_on` that applies ALL semantics ("has every
upstream had a SUCCESS run that finished AFTER this downstream's
most-recent start?") and yields either a `RunRequest(run_key=<upstream
run id>)` or a `SkipReason` naming the upstream that is still behind.
`context.instance` is replaced by a stub that records the `RunsFilter`s
the sensor builds and returns a fixed `DagsterRun` per call; no real
Dagster instance, no real run storage.

Run with: `~/.cache/rantai-dagster-venv/bin/python -m pytest
dagster/dispar_orchestrate/test_authored_factory.py -v`
"""

from __future__ import annotations

import unittest
from typing import Any, Sequence
from unittest import mock

import requests
from dagster import (
    DagsterRunStatus,
    DefaultScheduleStatus,
    JobSelector,
    RunRequest,
    RunsFilter,
    build_op_context,
    job,
)

from dispar_orchestrate import authored_factory, authored_transforms
from dispar_orchestrate.authored_factory import (
    DAGSTER_LOCATION,
    DAGSTER_REPO,
    _upstream_job_selector,
    build_authored_dependency_sensor,
)
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


# ── R3 plan 2a: dependency-sensor tests (appended below) ──────────────
#
# The sensor body is a closure over `pipeline["id"]` and `pipeline["depends_on"]`;
# every other field of the pipeline dict is irrelevant to it. These tests
# build the minimum dict the body reads on top of HEAD's `_ready_pipeline`
# (so the runnable-payload fixture stays the single source of truth for the
# `definition` shape) and never call the network -- `context.instance` is
# replaced by `StubInstance`, which records every `RunsFilter` the sensor
# builds and returns a fixed list per call.


class _FakeDagsterRun:
    """Stand-in for `dagster.DagsterRun` carrying only the fields the
    sensor body reads (`job_name`, `start_time`, `end_time`, `run_id`,
    `status`). The factory module never constructs one of these -- it
    only consumes whatever `context.instance.get_runs` returns -- so a
    minimal stub is enough."""

    def __init__(
        self,
        job_name: str,
        run_id: str,
        status: DagsterRunStatus,
        start_time: float | None,
        end_time: float | None,
    ) -> None:
        self.job_name = job_name
        self.run_id = run_id
        self.status = status
        self.start_time = start_time
        self.end_time = end_time


class _StubInstance:
    """Drop-in for `context.instance`. Records every `(filter, limit)`
    pair the sensor asks for and returns the run list the test set up
    for that pair. Indexed by `RunsFilter.job_name` so the sensor's
    per-upstream query and per-downstream query can be answered
    independently."""

    def __init__(self, runs_by_job: dict[str, list[_FakeDagsterRun]]) -> None:
        self._runs_by_job = runs_by_job
        self.calls: list[tuple[RunsFilter | None, int | None]] = []

    def get_runs(
        self,
        filters: RunsFilter | None = None,
        cursor: str | None = None,
        limit: int | None = None,
        bucket_by: Any = None,
        ascending: bool = False,
    ) -> Sequence[_FakeDagsterRun]:
        self.calls.append((filters, limit))
        job_name = filters.job_name if filters else None
        return list(self._runs_by_job.get(job_name, []))


class _StubContext:
    """Carries the two attributes the sensor body reads:
    `dagster_run` (the upstream run that triggered THIS tick) and
    `instance` (the run-storage stub)."""

    def __init__(self, dagster_run: _FakeDagsterRun, instance: _StubInstance) -> None:
        self.dagster_run = dagster_run
        self.instance = instance


def _dep_pipeline(pid: str, depends_on: list[str]) -> dict[str, Any]:
    """A pipeline dict shaped for the sensor body. The body reads
    `pipeline["id"]` and `pipeline["depends_on"]`; everything else is
    HEAD's `_ready_pipeline` shape so the `definition` fixture stays
    the single source of truth."""
    return {**_ready_pipeline(), "id": pid, "depends_on": depends_on}


def _build_dep_sensor(pid: str, depends_on: list[str]):
    """Helper: build the pipeline dict, then build and return the
    sensor `RunStatusSensorDefinition`. The sensor body is a closure
    over `pid` and `depends_on`, so this is the only factory the
    tests need. `request_job` MUST be a real `JobDefinition` (not a
    `JobSelector`) because Dagster 1.13.20's `SensorDefinition`
    superclass coerces it through `AutomationTarget.from_coercible`,
    which only handles `JobDefinition`/`UnresolvedAssetJobDefinition`
    -- a placeholder `@job` named the same way the factory would
    name it is enough."""
    @job(name=f"authored__{pid.replace('-', '_')}")
    def _placeholder_job() -> None:
        return None
    return build_authored_dependency_sensor(
        _dep_pipeline(pid, depends_on), authored_job=_placeholder_job
    )


class TestDependencySensor:
    """Sensor-body tests for R3 plan 2a. Plain pytest class (no
    `unittest.TestCase`) so the per-test asserts render with pytest's
    own introspection instead of `self.assertEqual`'s."""

    def test_returns_none_when_depends_on_is_empty(self) -> None:
        """A pipeline with `depends_on=[]` produces NO `run_status_sensor`:
        `Definitions(sensors=...)` is the union of every chain's first
        downstream, and "no chain" means "no sensor". An empty list there
        is the factory's whole point for pipelines without upstream wiring.
        """
        assert build_authored_dependency_sensor(_dep_pipeline("pl-empty", []), None) is None

    def test_returns_a_sensor_when_depends_on_is_set(self) -> None:
        """A non-empty `depends_on` produces a sensor with the
        `authored__<safe_id>_after` name and `JobSelector`s for `monitored_jobs`
        (verified by inspecting the `RunStatusSensorDefinition`'s internal
        `_monitored_jobs` -- the public surface has no `monitored_jobs`
        property in dagster 1.13.20)."""
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        assert sensor is not None
        assert sensor.name == "authored__pl_down_1_after"
        assert sensor.default_status.name == "RUNNING"
        assert list(sensor._monitored_jobs) == [  # type: ignore[attr-defined]
            JobSelector(
                location_name=DAGSTER_LOCATION,
                repository_name=DAGSTER_REPO,
                job_name="authored__pl_up_a",
            )
        ]
        assert sensor.job is not None
        assert sensor.job.name == "authored__pl_down_1"

    def test_upstream_job_selector_resolves_authored_id_through_dagster_safe_name(self) -> None:
        """`pl-up-a` -> `authored__pl_up_a`; the same `_dagster_safe_name`
        rule the job/schedule names use, so the `run_status_sensor`'s
        `monitored_jobs` actually matches what the factory built."""
        assert _upstream_job_selector("pl-up-a") == JobSelector(
            location_name=DAGSTER_LOCATION,
            repository_name=DAGSTER_REPO,
            job_name="authored__pl_up_a",
        )

    def test_upstream_job_selector_passes_a_dagster_native_id_through_verbatim(self) -> None:
        """Dagster-native upstreams (e.g. `ingest_job`) are already valid
        job names; the factory uses the id as the `job_name` field on the
        selector rather than re-running `_dagster_safe_name`."""
        assert _upstream_job_selector("ingest_job") == JobSelector(
            location_name=DAGSTER_LOCATION,
            repository_name=DAGSTER_REPO,
            job_name="ingest_job",
        )

    def test_yields_run_request_with_run_key_equal_to_upstream_run_id_when_all_fresh(self) -> None:
        """Happy path: the downstream has one prior run (start_time=100);
        upstream has one SUCCESS run that ended at 200 (>100). Yield
        `RunRequest(run_key=<upstream run id>)`. `run_key` MUST equal the
        triggering upstream run id -- Dagster's own dedup keeps a re-firing
        upstream from launching the downstream twice for the same upstream
        run; using any other key (the downstream id, the upstream job name,
        a fresh UUID) breaks that."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                "authored__pl_up_a": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        request = out[0]
        assert isinstance(request, RunRequest)
        assert request.run_key == "upstream-1"

    def test_yields_skip_reason_naming_the_stale_upstream_when_one_is_behind(self) -> None:
        """One upstream's latest SUCCESS ends at 50 (< downstream start
        100). The chain must NOT fire; the `SkipReason` must name the
        upstream that is behind (so the UI can render "waiting on pl-up-b").
        A bare "wait" would force the operator to read the sensor's
        `monitored_jobs` to figure out which dependency stalled."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                # `pl-up-a` is fresh (200 > 100); `pl-up-b` is stale (50 <
                # 100). The first stale upstream encountered is named in
                # the skip reason -- the body returns immediately on the
                # first failure to keep the per-tick work bounded.
                "authored__pl_up_a": [upstream_run],
                "authored__pl_up_b": [
                    _FakeDagsterRun(
                        job_name="authored__pl_up_b",
                        run_id="upstream-b-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=40.0,
                        end_time=50.0,
                    ),
                ],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a", "pl-up-b"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        reason = out[0]
        assert not isinstance(reason, RunRequest)
        # `SkipReason.skip_message` is the documented attribute (str).
        assert "pl-up-b" in reason.skip_message
        # The body should have asked for the stale upstream's runs and
        # stopped -- no need to query `pl-up-a` again on the same tick.
        upstream_b_calls = [
            c
            for c in instance.calls
            if c[0] is not None and c[0].job_name == "authored__pl_up_b"
        ]
        assert upstream_b_calls, "the sensor must have queried the stale upstream"

    def test_yields_run_request_immediately_on_first_downstream_run(self) -> None:
        """When the downstream has NEVER run, `downstream_start` is `None`.
        The chain must fire on this upstream's tick: there is no prior
        downstream start to compare against, so "after the downstream's
        start" is trivially satisfied. Without this rule, the very first
        run of any chained downstream would never fire -- it would always
        see its upstream's latest SUCCESS as "before the downstream's
        start" because there is no downstream start."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=10.0,
            end_time=20.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [],
                "authored__pl_up_a": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "upstream-1"

    def test_queries_only_success_runs_for_upstreams(self) -> None:
        """The freshness walk asks `context.instance.get_runs` with
        `statuses=[DagsterRunStatus.SUCCESS]` for every upstream. A FAILURE
        run on the upstream does not refresh the chain -- otherwise a
        failing upstream whose last successful run was ages ago would
        silence the sensor into a false-green. The downstream query has no
        status filter (it just needs the latest start)."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                "authored__pl_up_a": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        list(body(_StubContext(upstream_run, instance)))
        upstream_queries = [
            f
            for f, _limit in instance.calls
            if f is not None and f.job_name == "authored__pl_up_a"
        ]
        assert upstream_queries, "the sensor must have queried the upstream"
        for f in upstream_queries:
            assert f.statuses == [DagsterRunStatus.SUCCESS]

    def test_treats_missing_upstream_success_run_as_stale(self) -> None:
        """An upstream that has NEVER had a SUCCESS run is stale by
        definition. The `latest is None` branch must produce a
        `SkipReason` -- the chain cannot fire on hope."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                # No `authored__pl_up_a` entry -> instance.get_runs
                # returns `[]`. The walk must still treat this as stale.
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        assert "pl-up-a" in out[0].skip_message

    def test_uses_job_selector_with_dagster_native_id_when_upstream_is_native(self) -> None:
        """When `depends_on` lists a Dagster-native job (e.g. `ingest_job`,
        not a `pl-` id), the sensor watches that job's SUCCESS runs
        verbatim -- Dagster itself resolves the selector to a job that is
        not in this code location, which is the whole point of
        `JobSelector(job_name=id)` (vs the authored branch, which goes
        through `_dagster_safe_name`)."""
        upstream_run = _FakeDagsterRun(
            job_name="ingest_job",
            run_id="ingest-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                "ingest_job": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["ingest_job"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "ingest-1"
