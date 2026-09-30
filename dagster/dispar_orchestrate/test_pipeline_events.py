"""Unit tests for `dagster/dispar_orchestrate/pipeline_events.py`
(plans 1e + 1f).

No real network: `requests.post` is monkeypatched, the same style
`test_alerts_run.py` uses for the alerts-run op. The sensors
themselves need a real `DagsterInstance`; we exercise the *body* of
the sensors by calling `post_run_failed` / `post_run_finished`
directly and by calling the sensors' evaluation functions with fake
contexts, since constructing a real sensor context requires a live
Dagster instance.

Run with:
`~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate/test_pipeline_events.py -q`
"""

from __future__ import annotations

import unittest
from unittest import mock
from unittest.mock import MagicMock

import requests

from dispar_orchestrate import pipeline_events
from dispar_orchestrate.pipeline_events import (
    PipelineEventsConfig,
    _headers,
    evaluate_failed_run,
    evaluate_finished_run,
    post_run_failed,
    post_run_finished,
)


def _cfg(run_token: str = "tok-pf") -> PipelineEventsConfig:
    return PipelineEventsConfig(
        run_token=run_token,
        api_url="http://lakehouse-api.invalid:8080",
    )


class _FakeRun:
    def __init__(self, run_id: str, job_name: str) -> None:
        self.run_id = run_id
        self.job_name = job_name


class _FakeContext:
    """A minimal stand-in for the sensor contexts: only
    `dagster_run` and `log.info` / `log.warning` are exercised by the
    sensor bodies."""

    def __init__(self, run_id: str, job_name: str) -> None:
        self.dagster_run = _FakeRun(run_id, job_name)
        self.log = MagicMock()


class PipelineEventsConfigTests(unittest.TestCase):
    def test_from_env_reads_the_pipeline_run_token(self) -> None:
        with mock.patch.dict(
            "os.environ", {"PIPELINE_RUN_TOKEN": "tok-pf"}, clear=False
        ):
            cfg = PipelineEventsConfig.from_env()
        self.assertEqual(cfg.run_token, "tok-pf")

    def test_from_env_defaults_the_token_to_empty(self) -> None:
        with mock.patch.dict("os.environ", {}, clear=True):
            cfg = PipelineEventsConfig.from_env()
        self.assertEqual(cfg.run_token, "")

    def test_from_env_defaults_the_api_url(self) -> None:
        with mock.patch.dict("os.environ", {}, clear=True):
            cfg = PipelineEventsConfig.from_env()
        self.assertEqual(cfg.api_url, "http://lakehouse-api:8080")


class HeadersTests(unittest.TestCase):
    def test_unset_token_sends_no_headers(self) -> None:
        self.assertEqual(_headers(_cfg(run_token="")), {})

    def test_set_token_sends_both_bearer_and_run_token_headers(self) -> None:
        headers = _headers(_cfg(run_token="tok-pf"))
        self.assertEqual(headers["Authorization"], "Bearer tok-pf")
        self.assertEqual(headers["x-run-token"], "tok-pf")


class PostRunFailedTests(unittest.TestCase):
    def test_posts_the_run_id_and_job_name_to_the_run_failed_endpoint(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        with mock.patch.object(
            pipeline_events.requests, "post", return_value=response
        ) as mocked_post:
            post_run_failed(_cfg(), "run-deadbeef", "authored__pl_orders")
        mocked_post.assert_called_once_with(
            "http://lakehouse-api.invalid:8080/api/pipelines/events/run-failed",
            json={"runId": "run-deadbeef", "jobName": "authored__pl_orders"},
            headers={
                "Authorization": "Bearer tok-pf",
                "x-run-token": "tok-pf",
            },
            timeout=30,
        )

    def test_unset_token_posts_nothing(self) -> None:
        with mock.patch.object(
            pipeline_events.requests, "post"
        ) as mocked_post:
            post_run_failed(_cfg(run_token=""), "run-1", "authored__pl_x")
        mocked_post.assert_not_called()


class PostRunFinishedTests(unittest.TestCase):
    """Plan 1f: the SUCCESS sensor mirrors the failure sensor. Same
    posture, different endpoint."""

    def test_posts_the_run_id_and_job_name_to_the_run_finished_endpoint(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        with mock.patch.object(
            pipeline_events.requests, "post", return_value=response
        ) as mocked_post:
            post_run_finished(_cfg(), "run-cafebabe", "authored__pl_orders")
        mocked_post.assert_called_once_with(
            "http://lakehouse-api.invalid:8080/api/pipelines/events/run-finished",
            json={"runId": "run-cafebabe", "jobName": "authored__pl_orders"},
            headers={
                "Authorization": "Bearer tok-pf",
                "x-run-token": "tok-pf",
            },
            timeout=30,
        )

    def test_unset_token_posts_nothing(self) -> None:
        with mock.patch.object(
            pipeline_events.requests, "post"
        ) as mocked_post:
            post_run_finished(_cfg(run_token=""), "run-1", "authored__pl_x")
        mocked_post.assert_not_called()


class SensorEvaluationTests(unittest.TestCase):
    def test_a_failed_run_posts_its_run_id_and_job_name(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        context = _FakeContext("run-1", "authored__pl_orders")
        with mock.patch.dict(
            "os.environ", {"PIPELINE_RUN_TOKEN": "tok-pf"}, clear=False
        ):
            with mock.patch.object(
                pipeline_events.requests, "post", return_value=response
            ) as mocked_post:
                evaluate_failed_run(context)
        mocked_post.assert_called_once()
        args, kwargs = mocked_post.call_args
        self.assertEqual(
            args[0],
            "http://lakehouse-api:8080/api/pipelines/events/run-failed",
        )
        self.assertEqual(
            kwargs["json"], {"runId": "run-1", "jobName": "authored__pl_orders"}
        )
        self.assertEqual(kwargs["timeout"], 30)

    def test_an_unset_token_logs_and_posts_nothing(self) -> None:
        context = _FakeContext("run-1", "authored__pl_orders")
        with mock.patch.dict("os.environ", {}, clear=True):
            with mock.patch.object(
                pipeline_events.requests, "post"
            ) as mocked_post:
                evaluate_failed_run(context)
        mocked_post.assert_not_called()
        context.log.info.assert_called_once()
        info_message = context.log.info.call_args.args[0]
        self.assertIn("PIPELINE_RUN_TOKEN", info_message)

    def test_a_post_failure_is_logged_but_does_not_propagate(self) -> None:
        context = _FakeContext("run-1", "authored__pl_orders")
        with mock.patch.dict(
            "os.environ", {"PIPELINE_RUN_TOKEN": "tok-pf"}, clear=False
        ):
            with mock.patch.object(
                pipeline_events.requests,
                "post",
                side_effect=requests.ConnectionError("network down"),
            ):
                # Must not raise — Dagster will retry the tick, and the
                # API's `(run_id, kind)` dedupe makes the retry safe.
                evaluate_failed_run(context)
        context.log.warning.assert_called_once()
        # Dagster's logger takes the format string and the values
        # separately; checking the args is the cleanest assertion.
        args = context.log.warning.call_args.args
        self.assertIn("run-1", args)
        self.assertIn("authored__pl_orders", args)


class FinishedSensorEvaluationTests(unittest.TestCase):
    """Plan 1f: the SUCCESS sensor's evaluation body has the same
    posture as the failure sensor's, so the test set mirrors
    [`SensorEvaluationTests`] one for one."""

    def test_a_successful_run_posts_its_run_id_and_job_name(self) -> None:
        response = mock.Mock(spec=requests.Response)
        response.raise_for_status.return_value = None
        context = _FakeContext("run-2", "authored__pl_orders")
        with mock.patch.dict(
            "os.environ", {"PIPELINE_RUN_TOKEN": "tok-pf"}, clear=False
        ):
            with mock.patch.object(
                pipeline_events.requests, "post", return_value=response
            ) as mocked_post:
                evaluate_finished_run(context)
        mocked_post.assert_called_once()
        args, kwargs = mocked_post.call_args
        self.assertEqual(
            args[0],
            "http://lakehouse-api:8080/api/pipelines/events/run-finished",
        )
        self.assertEqual(
            kwargs["json"], {"runId": "run-2", "jobName": "authored__pl_orders"}
        )
        self.assertEqual(kwargs["timeout"], 30)

    def test_an_unset_token_logs_and_posts_nothing(self) -> None:
        context = _FakeContext("run-2", "authored__pl_orders")
        with mock.patch.dict("os.environ", {}, clear=True):
            with mock.patch.object(
                pipeline_events.requests, "post"
            ) as mocked_post:
                evaluate_finished_run(context)
        mocked_post.assert_not_called()
        context.log.info.assert_called_once()
        info_message = context.log.info.call_args.args[0]
        self.assertIn("PIPELINE_RUN_TOKEN", info_message)

    def test_a_post_failure_is_logged_but_does_not_propagate(self) -> None:
        context = _FakeContext("run-2", "authored__pl_orders")
        with mock.patch.dict(
            "os.environ", {"PIPELINE_RUN_TOKEN": "tok-pf"}, clear=False
        ):
            with mock.patch.object(
                pipeline_events.requests,
                "post",
                side_effect=requests.ConnectionError("network down"),
            ):
                # Must not raise — Dagster will retry the tick, and the
                # API's `(run_id, kind)` dedupe makes the retry safe.
                evaluate_finished_run(context)
        context.log.warning.assert_called_once()
        args = context.log.warning.call_args.args
        self.assertIn("run-2", args)
        self.assertIn("authored__pl_orders", args)


if __name__ == "__main__":
    unittest.main()

