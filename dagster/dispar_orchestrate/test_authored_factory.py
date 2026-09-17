"""Unit tests for `authored_factory.py`'s job factory (WS4 item E1, grand
plan §6). Same non-negotiable property `test_agent_runs.py` establishes
for `agent_runs.py`: an unreachable/unauthenticated `lakehouse-api` must
leave the whole Dagster code location loadable, with zero authored jobs
and a loud warning -- never an import-time exception.

No real network: `requests.get` is monkeypatched with canned responses/
exceptions, the same style `test_agent_runs.py` uses.

Run with: `~/.cache/rantai-dagster-venv/bin/python -m pytest
dagster/dispar_orchestrate/test_authored_factory.py -v`
"""

from __future__ import annotations

import unittest
from unittest import mock

import requests

from dispar_orchestrate import authored_factory


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
