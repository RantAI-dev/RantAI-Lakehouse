"""Unit tests for `dagster/dispar_orchestrate/alerts_run.py`'s config,
headers, and `run_alerts` shape (WS0 item 11).

No real network: `requests.post` is monkeypatched with canned
responses/exceptions, the same style `test_agent_runs.py` uses for
`requests.get`.

Run with:
`~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate/test_alerts_run.py -q`
"""

from __future__ import annotations

import unittest
from unittest import mock
from unittest.mock import MagicMock

import requests

from dispar_orchestrate import alerts_run
from dispar_orchestrate.alerts_run import AlertsRunConfig, _headers, run_alerts


def _cfg(run_token: str = "a-real-token") -> AlertsRunConfig:
    return AlertsRunConfig(
        run_token=run_token,
        api_url="http://lakehouse-api.invalid:8080",
    )


class _FakeContext:
    """A minimal stand-in for Dagster's op `context`: only `log.info` is
    exercised by `run_alerts`."""

    def __init__(self) -> None:
        self.log = MagicMock()


class AlertsRunConfigTests(unittest.TestCase):
    def test_from_env_reads_the_alerts_run_token(self) -> None:
        with mock.patch.dict("os.environ", {"ALERTS_RUN_TOKEN": "tok"}, clear=False):
            cfg = AlertsRunConfig.from_env()
        self.assertEqual(cfg.run_token, "tok")

    def test_from_env_defaults_the_token_to_empty(self) -> None:
        with mock.patch.dict("os.environ", {}, clear=True):
            cfg = AlertsRunConfig.from_env()
        self.assertEqual(cfg.run_token, "")

    def test_from_env_defaults_the_api_url(self) -> None:
        with mock.patch.dict("os.environ", {}, clear=True):
            cfg = AlertsRunConfig.from_env()
        self.assertEqual(cfg.api_url, "http://lakehouse-api:8080")


class HeadersTests(unittest.TestCase):
    def test_unset_token_sends_no_headers(self) -> None:
        self.assertEqual(_headers(_cfg(run_token="")), {})

    def test_set_token_sends_both_bearer_and_run_token_headers(self) -> None:
        headers = _headers(_cfg(run_token="tok-123"))
        self.assertEqual(headers["Authorization"], "Bearer tok-123")
        self.assertEqual(headers["x-run-token"], "tok-123")


class RunAlertsTests(unittest.TestCase):
    def test_posts_to_the_alerts_run_endpoint_with_both_headers_and_timeout(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        response.json.return_value = {"ran": 3, "results": []}
        context = _FakeContext()

        with mock.patch.object(
            alerts_run.requests, "post", return_value=response
        ) as mocked_post:
            result = run_alerts(context, _cfg(run_token="tok-xyz"))

        mocked_post.assert_called_once_with(
            "http://lakehouse-api.invalid:8080/api/alerts/run",
            headers={
                "x-run-token": "tok-xyz",
                "Authorization": "Bearer tok-xyz",
            },
            timeout=30,
        )
        self.assertEqual(result, {"ran": 3, "results": []})
        context.log.info.assert_called_once()

    def test_a_401_response_propagates_as_an_http_error(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.side_effect = requests.HTTPError(
            "401 Client Error: Unauthorized"
        )
        context = _FakeContext()

        with mock.patch.object(alerts_run.requests, "post", return_value=response):
            with self.assertRaises(requests.HTTPError):
                run_alerts(context, _cfg())


if __name__ == "__main__":
    unittest.main()
