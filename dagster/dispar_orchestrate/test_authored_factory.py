"""Unit tests for `authored_factory.py`'s `authored_pipeline_job`: the
run-time definition fetch, the source/target resolution, the SQL it builds,
and the job's run config shape (the one `POST /api/pipelines/{id}/trigger`
sends).

No real network: `requests.get` is monkeypatched with canned responses/
exceptions, the same style `test_agent_runs.py` uses. No real ClickHouse:
`run_model`/`ensure_catalog_database` are injected, the way
`test_connector_catalog.py` injects its ClickHouse calls.

Run with: `~/.cache/rantai-dagster-venv/bin/python -m pytest
dagster/dispar_orchestrate/test_authored_factory.py -v`
"""

from __future__ import annotations

import unittest
from unittest import mock

import requests
from dagster import DagsterInstance, build_op_context

from dispar_orchestrate import authored_factory, authored_transforms
from dispar_orchestrate.bronze_catalog import ClickHouseTarget

CH = ClickHouseTarget("http://ch", "default", "")


def _cfg(run_token: str = "a-real-token") -> authored_factory.AuthoredPipelineConfig:
    return authored_factory.AuthoredPipelineConfig(
        api_url="http://lakehouse-api.invalid:8080", run_token=run_token,
    )


def _pipeline(**overrides: object) -> dict[str, object]:
    definition = {
        "sourceZone": "bronze", "sourceTable": "orders",
        "targetZone": "serving", "targetTable": "orders_clean",
        "transforms": ["select(id,name)"], "fbicEnabled": False,
        "incrementalColumn": None, "connectorId": None,
    }
    definition.update(overrides)
    return {"id": "pl-orders-clean-abc123", "name": "orders_clean", "definition": definition}


def _runnable_response(pipelines: list[dict[str, object]]) -> mock.Mock:
    resp = mock.Mock(status_code=200)
    resp.raise_for_status.return_value = None
    resp.json.return_value = pipelines
    return resp


class HeadersTest(unittest.TestCase):
    def test_empty_token_sends_no_headers(self) -> None:
        self.assertEqual(authored_factory._headers(_cfg(run_token="")), {})

    def test_real_token_sends_both_headers(self) -> None:
        headers = authored_factory._headers(_cfg(run_token="tok"))
        self.assertEqual(headers["Authorization"], "Bearer tok")
        self.assertEqual(headers["x-run-token"], "tok")


class FetchRunnablePipelineTest(unittest.TestCase):
    def test_an_unset_token_fails_the_run_without_an_http_call(self) -> None:
        with mock.patch.object(authored_factory.requests, "get") as mocked_get:
            with self.assertRaises(authored_factory.AuthoredJobError) as ctx:
                authored_factory.fetch_runnable_pipeline(_cfg(run_token=""), "pl-a")
        mocked_get.assert_not_called()
        self.assertIn("PIPELINE_RUN_TOKEN", str(ctx.exception))

    def test_returns_the_pipeline_with_that_id(self) -> None:
        wanted = _pipeline()
        other = {**_pipeline(), "id": "pl-other"}
        with mock.patch.object(
            authored_factory.requests, "get", return_value=_runnable_response([other, wanted]),
        ) as mocked_get:
            found = authored_factory.fetch_runnable_pipeline(_cfg(), wanted["id"])
        self.assertIs(found, wanted)
        self.assertEqual(
            mocked_get.call_args.args[0], "http://lakehouse-api.invalid:8080/api/pipelines/runnable",
        )

    def test_a_pipeline_that_is_not_listed_fails_as_not_ready(self) -> None:
        """A draft or paused pipeline is not in `/runnable`; its run must
        fail rather than execute a definition nobody activated."""
        with mock.patch.object(authored_factory.requests, "get", return_value=_runnable_response([])):
            with self.assertRaises(authored_factory.AuthoredJobError) as ctx:
                authored_factory.fetch_runnable_pipeline(_cfg(), "pl-draft")
        self.assertIn("not ready", str(ctx.exception))

    def test_a_refused_call_fails_the_run(self) -> None:
        resp = mock.Mock(status_code=403)
        resp.raise_for_status.side_effect = requests.HTTPError("403")
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            with self.assertRaises(requests.HTTPError):
                authored_factory.fetch_runnable_pipeline(_cfg(), "pl-a")


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


class PipelineModelTest(unittest.TestCase):
    def test_a_bronze_source_reads_the_iceberg_table_through_the_catalog_database(self) -> None:
        model, reads_bronze = authored_factory.pipeline_model(_pipeline())
        self.assertTrue(reads_bronze)
        self.assertEqual(model.select, "SELECT id, name FROM icecat_pipelines.`bronze.orders`")
        self.assertEqual(model.target, "serving.orders_clean")
        self.assertEqual(model.engine, authored_factory.TARGET_ENGINE)

    def test_a_silver_source_reads_the_clickhouse_table(self) -> None:
        model, reads_bronze = authored_factory.pipeline_model(_pipeline(sourceZone="silver"))
        self.assertFalse(reads_bronze)
        self.assertEqual(model.select, "SELECT id, name FROM silver.`orders`")

    def test_a_connector_is_lineage_and_does_not_stop_the_run(self) -> None:
        model, _ = authored_factory.pipeline_model(_pipeline(connectorId="conn-1"))
        self.assertEqual(model.target, "serving.orders_clean")

    def test_an_unknown_source_zone_is_refused(self) -> None:
        with self.assertRaises(authored_factory.AuthoredJobError) as ctx:
            authored_factory.pipeline_model(_pipeline(sourceZone="lake"))
        self.assertIn("source zone", str(ctx.exception))

    def test_a_target_zone_outside_the_model_schemas_is_refused(self) -> None:
        with self.assertRaises(authored_factory.AuthoredJobError) as ctx:
            authored_factory.pipeline_model(_pipeline(targetZone="gold"))
        self.assertIn("target zone", str(ctx.exception))

    def test_an_unsafe_identifier_is_refused(self) -> None:
        with self.assertRaises(authored_factory.AuthoredJobError):
            authored_factory.pipeline_model(_pipeline(targetTable="orders; DROP TABLE x"))

    def test_a_rejected_transform_raises_rather_than_being_dropped(self) -> None:
        with self.assertRaises(authored_transforms.TransformError):
            authored_factory.pipeline_model(_pipeline(transforms=["exec(rm -rf /)"]))


class ExecutePipelineTest(unittest.TestCase):
    def test_a_bronze_pipeline_creates_the_catalog_database_before_running(self) -> None:
        calls: list[str] = []
        model, rows = authored_factory.execute_pipeline(
            _pipeline(),
            ch=CH,
            ensure_catalog_database=lambda ch, db: calls.append(f"catalog {db}"),
            run_model=lambda ch, m: calls.append(f"run {m.target}") or 7,
        )
        self.assertEqual(calls, ["catalog icecat_pipelines", "run serving.orders_clean"])
        self.assertEqual(rows, 7)
        self.assertEqual(model.name, "pl-orders-clean-abc123")

    def test_a_silver_pipeline_needs_no_catalog_database(self) -> None:
        ensure = mock.Mock()
        authored_factory.execute_pipeline(
            _pipeline(sourceZone="silver"), ch=CH, ensure_catalog_database=ensure, run_model=lambda ch, m: 0,
        )
        ensure.assert_not_called()

    def test_clickhouses_own_message_reaches_the_run_log(self) -> None:
        response = mock.Mock(text="Code: 60. DB::Exception: Unknown table expression identifier\n")

        def refuse(ch, model):
            raise requests.HTTPError("404 Client Error", response=response)

        with self.assertRaises(authored_factory.AuthoredJobError) as ctx:
            authored_factory.execute_pipeline(
                _pipeline(), ch=CH, ensure_catalog_database=lambda ch, db: None, run_model=refuse,
            )
        self.assertIn("Unknown table expression identifier", str(ctx.exception))


class RunAuthoredPipelineTest(unittest.TestCase):
    def test_the_op_runs_its_configured_pipeline(self) -> None:
        pipeline = _pipeline(fbicEnabled=True)
        model = authored_factory.pipeline_model(pipeline)[0]
        with mock.patch.object(authored_factory, "fetch_runnable_pipeline", return_value=pipeline) as fetch, \
             mock.patch.object(authored_factory, "execute_pipeline", return_value=(model, 3)):
            result = authored_factory.run_authored_pipeline(
                build_op_context(op_config={"pipeline_id": pipeline["id"]}),
            )
        self.assertEqual(fetch.call_args.args[1], pipeline["id"])
        self.assertEqual(result["rows"], 3)
        self.assertEqual(result["skipped_verbs"], [authored_factory.FBIC_UNSUPPORTED_REASON])

    def test_the_job_accepts_the_run_config_the_api_sends(self) -> None:
        """The exact config `authored_run_config` builds in
        `rust/crates/lakehouse-api/src/routes/pipelines.rs`; a mismatch
        would make every launch a `RunConfigValidationInvalid`."""
        pipeline = _pipeline()
        model = authored_factory.pipeline_model(pipeline)[0]
        run_config = {"ops": {"run_authored_pipeline": {"config": {"pipeline_id": pipeline["id"]}}}}
        with mock.patch.object(authored_factory, "fetch_runnable_pipeline", return_value=pipeline), \
             mock.patch.object(authored_factory, "execute_pipeline", return_value=(model, 1)):
            result = authored_factory.authored_pipeline_job.execute_in_process(run_config=run_config)
        self.assertTrue(result.success)
        self.assertEqual(authored_factory.authored_pipeline_job.name, "authored_pipeline_job")

    def test_a_failed_run_logs_why_it_failed(self) -> None:
        """The console shows a run's log messages, not the failed step's
        error chain, so the reason must be a log message of its own."""
        instance = DagsterInstance.ephemeral()
        refusal = authored_factory.AuthoredJobError("pipeline 'pl-x' is not ready to run")
        run_config = {"ops": {"run_authored_pipeline": {"config": {"pipeline_id": "pl-x"}}}}
        with mock.patch.object(authored_factory, "fetch_runnable_pipeline", side_effect=refusal):
            result = authored_factory.authored_pipeline_job.execute_in_process(
                run_config=run_config, instance=instance, raise_on_error=False,
            )
        self.assertFalse(result.success)
        messages = [entry.user_message for entry in instance.all_logs(result.run_id)]
        self.assertIn("AuthoredJobError: pipeline 'pl-x' is not ready to run", messages)


if __name__ == "__main__":
    unittest.main()
