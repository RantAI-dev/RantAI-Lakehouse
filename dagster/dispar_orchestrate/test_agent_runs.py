"""Unit tests for `dagster/dispar_orchestrate/agent_runs.py`'s schedule
factory (T3.3, copilot-operations-handover plan).

The non-negotiable property under test: Dagster evaluates every
job/schedule in ONE code location at once (`bronze_ingest_job`,
`bronze_maintenance_job`, `replication_slot_check_job`, `gold_export_job`
all live alongside `agent_run_job` — see `definitions.py`), so anything
this module does at import time (`agent_run_schedules =
build_agent_run_schedules(...)`) must NEVER raise — an unreachable
`lakehouse-api`, or one that answers 401, must still leave the whole code
location loadable, with zero agent schedules and a loud warning instead of
an exception.

No real network: `requests.get`/`requests.post` are monkeypatched with
canned responses/exceptions, the same style `test_maintenance.py` uses for
`_ch_query`.

Run with: `cd dagster && python -m unittest dispar_orchestrate.test_agent_runs -v`
"""

from __future__ import annotations

import unittest
from unittest import mock

import requests

from dispar_orchestrate import agent_runs


def _cfg(run_token: str = "a-real-token") -> agent_runs.AgentRunConfig:
    return agent_runs.AgentRunConfig(
        api_url="http://lakehouse-api.invalid:8080",
        run_token=run_token,
    )


class HeadersTest(unittest.TestCase):
    def test_empty_token_sends_no_headers(self) -> None:
        self.assertEqual(agent_runs._headers(_cfg(run_token="")), {})

    def test_set_token_sends_both_bearer_and_run_token_headers(self) -> None:
        headers = agent_runs._headers(_cfg(run_token="tok-123"))
        self.assertEqual(headers["Authorization"], "Bearer tok-123")
        self.assertEqual(headers["x-run-token"], "tok-123")


class FetchSchedulableEmployeesTest(unittest.TestCase):
    def test_unset_token_returns_empty_list_with_no_http_call(self) -> None:
        with mock.patch.object(agent_runs.requests, "get") as mocked_get:
            result = agent_runs._fetch_schedulable_employees(_cfg(run_token=""))
        self.assertEqual(result, [])
        mocked_get.assert_not_called()

    def test_unreachable_api_returns_empty_list_and_does_not_raise(self) -> None:
        """Proves the "API is unreachable at code-load time" resilience
        case: a connection failure must degrade to zero schedules, never
        propagate."""
        with mock.patch.object(
            agent_runs.requests,
            "get",
            side_effect=requests.ConnectionError("connection refused"),
        ):
            result = agent_runs._fetch_schedulable_employees(_cfg())
        self.assertEqual(result, [])

    def test_unauthenticated_401_returns_empty_list_and_does_not_raise(self) -> None:
        """Proves the "API is unauthenticated at code-load time" resilience
        case: a 401 (wrong/stale `AGENT_RUN_TOKEN`) must degrade to zero
        schedules, never propagate."""
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.side_effect = requests.HTTPError(
            "401 Client Error: Unauthorized"
        )
        with mock.patch.object(agent_runs.requests, "get", return_value=response):
            result = agent_runs._fetch_schedulable_employees(_cfg())
        self.assertEqual(result, [])

    def test_non_json_body_returns_empty_list_and_does_not_raise(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        response.json.side_effect = ValueError("not json")
        with mock.patch.object(agent_runs.requests, "get", return_value=response):
            result = agent_runs._fetch_schedulable_employees(_cfg())
        self.assertEqual(result, [])

    def test_non_list_body_returns_empty_list_and_does_not_raise(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        response.json.return_value = {"not": "a list"}
        with mock.patch.object(agent_runs.requests, "get", return_value=response):
            result = agent_runs._fetch_schedulable_employees(_cfg())
        self.assertEqual(result, [])

    def test_filters_to_employees_with_a_schedule_cron(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        response.json.return_value = [
            {"id": "emp-a", "scheduleCron": "0 6 * * *"},
            {"id": "emp-b", "scheduleCron": None},
            {"id": "emp-c"},
            {"id": "emp-d", "scheduleCron": "0 7 * * *"},
        ]
        with mock.patch.object(agent_runs.requests, "get", return_value=response):
            result = agent_runs._fetch_schedulable_employees(_cfg())
        self.assertEqual([e["id"] for e in result], ["emp-a", "emp-d"])

    def test_sends_the_bearer_credential_it_authenticates_with(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        response.json.return_value = []
        with mock.patch.object(agent_runs.requests, "get", return_value=response) as mocked_get:
            agent_runs._fetch_schedulable_employees(_cfg(run_token="tok-xyz"))
        _, kwargs = mocked_get.call_args
        self.assertEqual(kwargs["headers"]["Authorization"], "Bearer tok-xyz")
        self.assertEqual(kwargs["headers"]["x-run-token"], "tok-xyz")


class BuildAgentRunSchedulesTest(unittest.TestCase):
    def test_builds_one_schedule_definition_per_schedulable_employee(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        response.json.return_value = [
            {"id": "emp-daily-quality-check", "scheduleCron": "0 6 * * *"},
        ]
        with mock.patch.object(agent_runs.requests, "get", return_value=response):
            schedules = agent_runs.build_agent_run_schedules(_cfg())
        self.assertEqual(len(schedules), 1)
        schedule = schedules[0]
        self.assertEqual(schedule.name, "agent_run_schedule__emp_daily_quality_check")
        self.assertEqual(schedule.cron_schedule, "0 6 * * *")
        self.assertEqual(schedule.job_name, "agent_run_job")

    def test_unreachable_api_builds_zero_schedules_without_raising(self) -> None:
        """The end-to-end resilience proof at the public entry point this
        module's own top-level `agent_run_schedules = ...` line calls:
        an unreachable API must not raise out of `build_agent_run_schedules`,
        the exact call `agent_runs.py` makes at module-import (Dagster
        code-load) time."""
        with mock.patch.object(
            agent_runs.requests,
            "get",
            side_effect=requests.ConnectionError("connection refused"),
        ):
            schedules = agent_runs.build_agent_run_schedules(_cfg())
        self.assertEqual(schedules, [])

    def test_unauthenticated_api_builds_zero_schedules_without_raising(self) -> None:
        """Same proof as above, for a 401 rather than a connection
        failure — both are code-load-time conditions this module must
        survive with zero schedules, per the module doc comment."""
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.side_effect = requests.HTTPError(
            "401 Client Error: Unauthorized"
        )
        with mock.patch.object(agent_runs.requests, "get", return_value=response):
            schedules = agent_runs.build_agent_run_schedules(_cfg())
        self.assertEqual(schedules, [])

    def test_unset_token_builds_zero_schedules_with_no_http_call(self) -> None:
        """Schedules stay inert when the token is unset — no call is even
        attempted."""
        with mock.patch.object(agent_runs.requests, "get") as mocked_get:
            schedules = agent_runs.build_agent_run_schedules(_cfg(run_token=""))
        self.assertEqual(schedules, [])
        mocked_get.assert_not_called()


if __name__ == "__main__":
    unittest.main()
