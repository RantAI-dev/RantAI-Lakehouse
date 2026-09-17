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
from dagster import build_op_context

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
        """Covers the real, documented 403 the `authored-pipeline-scheduler`
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

    def test_a_rejected_transform_raises_rather_than_being_dropped(self) -> None:
        pipeline = _ready_pipeline(transforms=["exec(rm -rf /)"])
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with self.assertRaises(authored_transforms.TransformError):
            run_fn(build_op_context())

    def test_connector_sourced_pipeline_raises_an_honest_unsupported_error(self) -> None:
        pipeline = _ready_pipeline(connectorId="conn-1")
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with self.assertRaises(authored_factory.AuthoredJobError) as ctx:
            run_fn(build_op_context())
        self.assertIn("connector-sourced", str(ctx.exception))

    def test_an_unsafe_identifier_raises_rather_than_reaching_sql(self) -> None:
        pipeline = _ready_pipeline(targetTable="orders; DROP TABLE x")
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with mock.patch.object(authored_factory, "_ch_exec") as mocked_exec:
            with self.assertRaises(authored_factory.AuthoredJobError):
                run_fn(build_op_context())
        mocked_exec.assert_not_called()
