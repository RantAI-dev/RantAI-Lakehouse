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
            timeout=pipeline_events.POST_TIMEOUT,
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
            timeout=pipeline_events.POST_TIMEOUT,
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
        self.assertEqual(kwargs["timeout"], pipeline_events.POST_TIMEOUT)

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
            with mock.patch.object(pipeline_events, "_sleep"), mock.patch.object(
                pipeline_events.requests,
                "post",
                side_effect=requests.ConnectionError("network down"),
            ):
                # Must not raise: Dagster does not retry either way (the
                # cursor moves past the run), so the sensor logs and returns
                # after the in-tick attempts.
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
        self.assertEqual(kwargs["timeout"], pipeline_events.POST_TIMEOUT)

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
            with mock.patch.object(pipeline_events, "_sleep"), mock.patch.object(
                pipeline_events.requests,
                "post",
                side_effect=requests.ConnectionError("network down"),
            ):
                # Must not raise; see the failure sensor's test.
                evaluate_finished_run(context)
        context.log.warning.assert_called_once()
        args = context.log.warning.call_args.args
        self.assertIn("run-2", args)
        self.assertIn("authored__pl_orders", args)


def _ok() -> mock.Mock:
    response = mock.Mock(spec=requests.Response)
    response.raise_for_status.return_value = None
    return response


def _http_error(status: int) -> requests.HTTPError:
    response = requests.Response()
    response.status_code = status
    return requests.HTTPError(f"{status} from the API", response=response)


class BoundedRetryTests(unittest.TestCase):
    """`SRC-7` review SHOULD-FIX 4: Dagster's run sensor moves past a run
    whether the function raised or not, so the only retry is the bounded one
    inside the tick. `requests.post` and the pause are replaced: no network,
    no real sleeping."""

    def _run(self, post_fn, side_effect):
        with mock.patch.object(pipeline_events, "_sleep") as sleep, mock.patch.object(
            pipeline_events.requests, "post", side_effect=side_effect
        ) as mocked_post:
            error = None
            try:
                post_fn(_cfg(), "run-1", "ingest_job")
            except requests.RequestException as err:
                error = err
        return mocked_post, sleep, error

    def test_a_connection_error_is_retried_and_the_second_try_succeeds(self) -> None:
        for post_fn in (post_run_failed, post_run_finished):
            mocked_post, sleep, error = self._run(
                post_fn, [requests.ConnectionError("down"), _ok()]
            )
            self.assertIsNone(error)
            self.assertEqual(mocked_post.call_count, 2)
            sleep.assert_called_once_with(pipeline_events.POST_PAUSE_SECONDS)

    def test_a_timeout_is_retried(self) -> None:
        mocked_post, _, error = self._run(
            post_run_failed, [requests.ReadTimeout("slow"), _ok()]
        )
        self.assertIsNone(error)
        self.assertEqual(mocked_post.call_count, 2)

    def test_a_5xx_answer_is_retried(self) -> None:
        failing = mock.Mock(spec=requests.Response)
        failing.raise_for_status.side_effect = _http_error(503)
        mocked_post, _, error = self._run(post_run_finished, [failing, _ok()])
        self.assertIsNone(error)
        self.assertEqual(mocked_post.call_count, 2)

    def test_a_4xx_answer_is_never_retried(self) -> None:
        for status in (400, 401, 403, 409):
            refused = mock.Mock(spec=requests.Response)
            refused.raise_for_status.side_effect = _http_error(status)
            mocked_post, sleep, error = self._run(post_run_failed, [refused, _ok()])
            self.assertIsInstance(error, requests.HTTPError)
            self.assertEqual(mocked_post.call_count, 1)
            sleep.assert_not_called()

    def test_three_failed_attempts_give_up_with_two_pauses_and_raise(self) -> None:
        mocked_post, sleep, error = self._run(
            post_run_failed, requests.ConnectionError("down")
        )
        self.assertIsInstance(error, requests.ConnectionError)
        self.assertEqual(mocked_post.call_count, pipeline_events.POST_ATTEMPTS)
        self.assertEqual(sleep.call_count, pipeline_events.POST_ATTEMPTS - 1)

    def test_the_worst_case_for_one_run_fits_inside_the_default_sensor_limit(self) -> None:
        connect, read = pipeline_events.POST_TIMEOUT
        worst = (
            pipeline_events.POST_ATTEMPTS * (connect + read)
            + (pipeline_events.POST_ATTEMPTS - 1) * pipeline_events.POST_PAUSE_SECONDS
        )
        self.assertLessEqual(worst, 60)

    def test_a_sensor_that_gave_up_logs_that_the_alert_is_not_sent(self) -> None:
        context = _FakeContext("run-9", "ingest_job")
        with mock.patch.dict("os.environ", {"PIPELINE_RUN_TOKEN": "tok-pf"}, clear=False):
            with mock.patch.object(pipeline_events, "_sleep"), mock.patch.object(
                pipeline_events.requests,
                "post",
                side_effect=requests.ConnectionError("down"),
            ) as mocked_post:
                evaluate_failed_run(context)
        self.assertEqual(mocked_post.call_count, pipeline_events.POST_ATTEMPTS)
        context.log.warning.assert_called_once()
        self.assertIn("not sent", context.log.warning.call_args.args[0])


if __name__ == "__main__":
    unittest.main()

