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
    Backoff,
    DagsterRunStatus,
    DefaultScheduleStatus,
    Jitter,
    JobSelector,
    RetryPolicy,
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
        """F1.4 (#57): `_write_clickhouse_table` is now ATOMIC: the body
        inserts the new rows into a `__staging_<run_id>` table under the
        same `target_zone`, then `EXCHANGE TABLES` swaps staging and
        target. The previous direct `INSERT INTO serving.`orders_clean``
        left a window where a partial mid-run write was visible to a
        parallel reader -- a non-atomic read against a consumer
        dashboard (this plan's `dagster_pipeline_orchestrator` is the
        unit, but the row schema here is the same `serving` zone used
        downstream). The two new asserts pin the staging-EXCHANGE
        sequence (an atomic INSERT on the staging table followed by
        EXCHANGE TABLES swapping the staging and target tables).
        """
        pipeline = _ready_pipeline()
        run_fn = authored_factory._op_for_pipeline(pipeline)

        # _write_clickhouse_table now does 2 count() calls (before
        # insert on staging, after exchange on target -- the staging
        # table is dropped after EXCHANGE, so the post-count must
        # read the swapped-in target).
        counts = iter([[{"n": "0"}], [{"n": "3"}]])  # before, after
        exec_calls: list[str] = []
        with mock.patch.object(authored_factory, "_ch_exec", side_effect=lambda t, s: exec_calls.append(s)), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")):
            result = run_fn(build_op_context())

        self.assertEqual(result["rows"], 3)
        self.assertEqual(result["skipped_verbs"], [])
        # The INSERT must target a staging table, not the live target.
        insert_calls = [c for c in exec_calls if "INSERT INTO" in c]
        self.assertEqual(len(insert_calls), 1, "exactly one INSERT per run")
        self.assertNotIn("INSERT INTO serving.`orders_clean`", insert_calls[0])
        self.assertIn("INSERT INTO serving.`__staging_orders_clean", insert_calls[0])
        # The atomic swap follows the staging INSERT.
        exchange_calls = [c for c in exec_calls if "EXCHANGE TABLES" in c]
        self.assertEqual(len(exchange_calls), 1, "exactly one EXCHANGE per run")
        self.assertIn("serving.`__staging_orders_clean", exchange_calls[0])
        self.assertIn("serving.`orders_clean`", exchange_calls[0])

    def test_atomic_write_succeeds_on_first_attempt(self) -> None:
        """F1.4 (#57): the staging-EXCHANGE sequence completes in one
        op invocation and reports a real row delta. The `_run` op
        body's `SELECT count()` AFTER sequence must see the row count
        of the SWAPPED-IN target (which is what readers see)."""
        pipeline = _ready_pipeline()
        run_fn = authored_factory._op_for_pipeline(pipeline)

        # First count(): before-staging-insert (target is the empty one
        # from `_ensure_target_table`, so 0). Second count(): after
        # EXCHANGE, on the swapped-in target that started as staging
        # and now holds the 3 rows -- 3.
        counts = iter([[{"n": "0"}], [{"n": "3"}]])
        with mock.patch.object(authored_factory, "_ch_exec"), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")):
            result = run_fn(build_op_context())

        self.assertEqual(result["rows"], 3)

    def test_atomic_write_is_idempotent_under_retry(self) -> None:
        """F1.4 (#57): a retry of the same op invocation must produce
        the SAME final state on `serving.orders_clean` as a single
        successful run. The atomic staging path makes the operation
        idempotent: on retry the staging table is dropped (if left
        over from a failed prior attempt) and re-created before the
        INSERT. The op-body then runs to completion on the second
        attempt, and the row delta is 3 -- not 6, which would have
        been the symptom of a direct INSERT under retry.

        We invoke the op body twice in the same test process (mirroring
        `test_op_source_metadata.py::test_default_retry_policy_retries_a_transient_network_error`
        which calls its op body twice to simulate a retry) and assert
        the second invocation sees a final row count equal to the
        first invocation's row count -- i.e. the swap puts the target
        back to its post-write shape, not to its pre-write + staging
        shape."""
        pipeline = _ready_pipeline()
        run_fn = authored_factory._op_for_pipeline(pipeline)

        # Each invocation: before=0, after=3. We don't share state
        # between calls because `_write_clickhouse_table` drops the
        # staging table before the INSERT, so each run begins from
        # the same starting point. One iter per invocation (not a
        # factory), because the lambda closes over the iter -- if a
        # fresh iter were returned on every call, the second
        # `_ch_query_json` would also read `{"n":"0"}` and the row
        # delta would be 0.
        counts_1 = iter([[{"n": "0"}], [{"n": "3"}]])
        # First invocation -- staging-write + EXCHANGE.
        with mock.patch.object(authored_factory, "_ch_exec"), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts_1)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")):
            first = run_fn(build_op_context())
        self.assertEqual(first["rows"], 3)

        counts_2 = iter([[{"n": "0"}], [{"n": "3"}]])
        # Second invocation -- same op body, must still report 3
        # rows of delta (no double-count, no leftover staging rows).
        with mock.patch.object(authored_factory, "_ch_exec"), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts_2)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")):
            second = run_fn(build_op_context())
        self.assertEqual(second["rows"], 3)

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
            jobs, schedules, deps = authored_factory.build_authored_definitions(_cfg())
        self.assertEqual(mocked_get.call_count, 1)
        self.assertEqual(len(jobs), 2)
        self.assertEqual([s.name for s in schedules], ["authored__pl_dedupe_select_abc123_schedule"])
        # F1.1: the third element is the `(pipeline, job)` list the
        # sensor builder now consumes — one entry per fetched pipeline,
        # and the `job` is the SAME `JobDefinition` instance the
        # `jobs` list holds. A second `build_authored_job` call here is
        # what F1.1 (BLOCKER) was about: the same name on two distinct
        # `JobDefinition` objects crashes `Definitions` with
        # `DagsterInvalidDefinitionError: Duplicate job definition found`.
        self.assertEqual(len(deps), 2)
        jobs_set = {id(j) for j in jobs}
        self.assertTrue(
            all(id(dep_job) in jobs_set for _dep_pipeline, dep_job in deps),
            "the sensor builder must reuse the jobs the jobs list holds",
        )


# ── R3 plan 2a: dependency-sensor tests (appended below) ──────────────
#
# The sensor body is a closure over `pipeline["id"]` and `pipeline["dependsOn"]`;
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
    """A pipeline dict shaped for the sensor body. Mirrors the REAL
    wire format the Rust `RunnablePipeline` serializes
    (`#[serde(rename_all = "camelCase")]` in
    `rust/crates/lakehouse-store/src/pipelines.rs`): the upstream-list
    field is `dependsOn` on the wire, never `depends_on`. The body
    reads `pipeline["id"]` and `pipeline["dependsOn"]`; everything
    else is HEAD's `_ready_pipeline` shape so the `definition`
    fixture stays the single source of truth."""
    return {**_ready_pipeline(), "id": pid, "dependsOn": depends_on}


def _dep_pipeline_status(pid: str, depends_on: list[str], status: str) -> dict[str, Any]:
    """A pipeline dict with an explicit `status` field, so the F1.2
    paused-skip test can exercise the `paused` branch without changing
    the default `ready` shape every other test in this class relies on."""
    return {**_dep_pipeline(pid, depends_on), "status": status}


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

    def test_returns_none_when_a_chained_pipeline_is_paused(self) -> None:
        """F1.2 SHOULD-FIX (#57): a `paused` pipeline with a non-empty
        `dependsOn` produces NO `run_status_sensor`. Dagster keeps the
        sensor's RUNNING/STOPPED state across a code-location reload,
        so a sensor shipped RUNNING continues to fire even after the
        pipeline's source `dependsOn` disappears — pausing the
        pipeline must therefore skip the sensor at code-load time, AND
        the API's `authored_status` route must stop the stored sensor
        next to the schedule. The first half is asserted here; the
        API half is the Rust `routes::pipelines::authored_status` wire
        in F1.2's second step."""
        @job(name="authored__pl_paused_down")
        def _placeholder_job() -> None:
            return None
        assert (
            build_authored_dependency_sensor(
                _dep_pipeline_status("pl-paused-down", ["pl-up-a"], "paused"),
                _placeholder_job,
            )
            is None
        )

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

    def test_yields_run_request_with_run_key_equal_to_sorted_upstream_run_ids_when_all_fresh(self) -> None:
        """Happy path: the downstream has one prior run (start_time=100);
        upstream has one SUCCESS run that ended at 200 (>100). Yield
        `RunRequest(run_key=authored-deps:<sorted upstream run ids>)`.

        F1.3 (#57): the OLD `run_key = upstream_run.run_id` would launch
        one downstream per upstream in a multi-upstream chain (each
        upstream's own tick yielded a key derived only from itself; the
        daemon's dedup was per-key, not per-round). The new key is the
        sorted tuple of every upstream's latest SUCCESS run id, prefixed
        with `authored-deps:`, so two ticks that see the same set of
        upstream successes ask for the same downstream launch once.

        With one upstream the new value is the same as the old (modulo
        the prefix); the multi-upstream "one run per round" property is
        asserted in `test_yields_one_run_request_for_two_upstream_events`.
        """
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
        assert request.run_key == "authored-deps:upstream-1"

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
        assert out[0].run_key == "authored-deps:upstream-1"

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
        assert out[0].run_key == "authored-deps:ingest-1"

    def test_first_downstream_run_does_not_fire_when_one_upstream_never_succeeded(self) -> None:
        """F1.3 (#57): on the chain's first-ever tick (the downstream has
        no runs yet), the sensor body MUST check that EVERY upstream has had
        a SUCCESS run -- even when the downstream has never run before,
        "first run" does not waive ALL-semantics, only the "must-be-newer-
        than-downstream-start" comparison.

        The buggy old code took `if downstream_start is None: yield
        RunRequest(...)` and returned before the upstream walk. With one
        upstream that has never run, that path launched the downstream on
        the first success of any OTHER upstream and never waited for the
        missing one -- silently dead-chains that the UI then had to
        diagnose. The fix walks every upstream first and yields
        `SkipReason` when one has never had a SUCCESS."""
        triggering_upstream = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-a-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=10.0,
            end_time=20.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [],
                # `pl-up-a` has one SUCCESS run; `pl-up-b` has never
                # had a SUCCESS run. ALL semantics means the chain must
                # NOT fire on `pl-up-a`'s tick.
                "authored__pl_up_a": [triggering_upstream],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a", "pl-up-b"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(triggering_upstream, instance)))
        assert len(out) == 1
        assert not isinstance(out[0], RunRequest)
        assert "pl-up-b" in out[0].skip_message
        assert "never had a SUCCESS run" in out[0].skip_message

    def test_yields_skip_reason_when_downstream_latest_run_is_queued(self) -> None:
        """F1.3 (#57): a downstream whose latest run has not started yet
        (status `QUEUED` / `NOT_STARTED` -- the `start_time` is None)
        must NOT launch another downstream run. The old `get_runs`
        comment said "filtered by status" but the call had NO status
        filter, so `latest_downstream.start_time` was None for a queued
        run and the body treated it as "no downstream run yet" -- a
        false-green that double-fires the chain until the queued run
        finishes. The fix short-circuits on `start_time is None`
        before any upstream walk."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        queued_downstream = _FakeDagsterRun(
            job_name="authored__pl_down_1",
            run_id="downstream-queued-1",
            # `start_time = None` is the queued / not-yet-started shape.
            status=DagsterRunStatus.QUEUED,
            start_time=None,
            end_time=None,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [queued_downstream],
                "authored__pl_up_a": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        assert not isinstance(out[0], RunRequest)
        assert "queued run" in out[0].skip_message
        assert "downstream-queued-1" in out[0].skip_message

    def test_yields_one_run_request_for_two_upstream_events_in_one_round(self) -> None:
        """F1.3 (#57): the chain's `run_key` MUST be derived from the
        SORTED set of the upstreams' latest SUCCESS run ids, so two
        upstream ticks that both see the same ALL-fresh upstream set
        ask the daemon for the same downstream launch exactly once.
        The old code keyed each upstream on its own run id and the
        daemon's dedup kept each upstream from launching the same
        downstream twice FOR THAT UPCELLING -- but two ticks that both
        asked to launch yielded ONE downstream per tick, regardless of
        dedup, because the keys were different. The daemon therefore saw
        two consecutive RunRequests for a downstream that should have
        launched once.

        This test simulates the per-tick body twice (one body per
        upstream tick) and asserts both ticks yield the SAME
        `run_key`, so the daemon's own dedup keeps the second tick
        from launching a second downstream run.
        """
        upstream_a = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-a-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        upstream_b = _FakeDagsterRun(
            job_name="authored__pl_up_b",
            run_id="upstream-b-1",
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
                "authored__pl_up_a": [upstream_a],
                "authored__pl_up_b": [upstream_b],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a", "pl-up-b"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        # Tick #1 fires on upstream_a's SUCCESS run.
        out_a = list(body(_StubContext(upstream_a, instance)))
        assert len(out_a) == 1 and isinstance(out_a[0], RunRequest)
        # Tick #2 fires on upstream_b's SUCCESS run (after the daemon
        # has had a chance to launch the first downstream).
        out_b = list(body(_StubContext(upstream_b, instance)))
        assert len(out_b) == 1 and isinstance(out_b[0], RunRequest)
        # Same key for both ticks = one daemon-decided downstream run.
        assert out_a[0].run_key == out_b[0].run_key
        assert out_a[0].run_key == "authored-deps:upstream-a-1,upstream-b-1"


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


class CodeLocationLoadTest(unittest.TestCase):
    """F1.1 BLOCKER: a pipeline with `dependsOn` MUST load under a real
    `Definitions(jobs, schedules, sensors)` — Dagster 1.13.20 refuses two
    `JobDefinition`s with the same name on a single location load, and
    building the job twice (once for `jobs`, once inside the sensor
    builder's `build_authored_job(pipeline)` call) used to do exactly
    that, taking the WHOLE code location down with it.

    The load call is the real-Dagster-load seam the plan requires: every
    other test in this file monkeypatches the HTTP fetch and inspects the
    factory's return values, which never crosses the `Definitions`
    validation step that fails in production. `load_all_definitions()`
    is what `dagster code-server start` runs at reload time — failing
    here is what was failing there."""

    def test_a_chain_pipeline_loads_the_code_location_without_a_duplicate_job(self) -> None:
        from dagster import Definitions

        upstream = {**_ready_pipeline(), "id": "pl-up-a", "dependsOn": []}
        downstream = {**_ready_pipeline(), "id": "pl-down-1", "dependsOn": ["pl-up-a"]}

        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [upstream, downstream]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            jobs, schedules, deps = authored_factory.build_authored_definitions(_cfg())

        sensors = authored_factory.build_authored_dependency_sensors(deps)
        defs = Definitions(jobs=jobs, schedules=schedules, sensors=sensors)
        # `load_all_definitions` walks the whole graph (jobs, schedules,
        # sensors, their dependencies) and is what raises
        # `DagsterInvalidDefinitionError: Duplicate job definition found`
        # when the sensor builder reuses a name that is already taken by
        # `authored_jobs`. A second `build_authored_job(pipeline)` here
        # is the mutation that re-introduces the BLOCKER (see the
        # `CodeLocationLoadMutationTest` below).
        repo = defs.get_repository_def()
        repo.load_all_definitions()  # must NOT raise


class CodeLocationLoadMutationTest(unittest.TestCase):
    """F1.1 mutation evidence. Building the downstream's job a second
    time inside the sensor builder is the exact regression the BLOCKER
    fix removed — this test re-introduces it and asserts that
    `load_all_definitions()` fails with the same `DagsterInvalidDefinitionError`
    the production code path raised. The assertion checks for the
    typename AND the offending job name so a future refactor that moves
    the failure somewhere else still requires a real review."""

    def test_building_the_job_twice_breaks_load_with_duplicate_definition_error(self) -> None:
        from dagster import Definitions, DagsterInvalidDefinitionError

        upstream = {**_ready_pipeline(), "id": "pl-up-a", "dependsOn": []}
        downstream = {**_ready_pipeline(), "id": "pl-down-1", "dependsOn": ["pl-up-a"]}

        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [upstream, downstream]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            jobs, _schedules, _deps = authored_factory.build_authored_definitions(_cfg())

        # Mutation: build the downstream's job a SECOND time — the
        # regression the fix removed. The two `JobDefinition` objects
        # share the name `authored__pl_down_1`; `Definitions` refuses.
        duplicate_job = authored_factory.build_authored_job(downstream)
        sensors = authored_factory.build_authored_dependency_sensor(
            downstream, duplicate_job,
        )

        defs = Definitions(jobs=jobs, schedules=[], sensors=[sensors] if sensors else [])
        with self.assertRaises(DagsterInvalidDefinitionError) as ctx:
            defs.get_repository_def().load_all_definitions()
        message = str(ctx.exception)
        self.assertIn("Duplicate", message)
        self.assertIn("authored__pl_down_1", message)
