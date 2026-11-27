"""Unit tests for `iceberg_sync.py` (`DATA-66` T5): the job, the sensor's
decision body and the authored op's never-raising ask.

No network and no Dagster instance: `requests` is patched, and the job's
graph is read from the definition itself. The decision rules themselves
live in Rust (`lakehouse_api::iceberg_sync`), so these tests only check
that this side turns the API's answer into the right `RunRequest`s — and
refuses a malformed one instead of silently starting nothing.
"""

from __future__ import annotations

import unittest
from unittest import mock

import requests

from dagster import (
    DefaultSensorStatus,
    Failure,
    RunRequest,
    SkipReason,
    build_op_context,
)

from dispar_orchestrate import iceberg_sync
from dispar_orchestrate.gold_export import GoldExportConfig


def _cfg(run_token: str = "a-real-token") -> GoldExportConfig:
    return GoldExportConfig(
        ch=None,  # type: ignore[arg-type]
        api_url="http://lakehouse-api.invalid:8080",
        marts=[],
        run_token=run_token,
        rustfs_access_key="test-access-key",
        rustfs_secret_key="test-secret-key",
    )


class _FakeContext:
    """Only `log.info`/`log.warning` are exercised by the ops here."""

    def __init__(self) -> None:
        self.log = mock.MagicMock()


def _answer(body: object, status: int = 200) -> mock.Mock:
    response = mock.Mock(spec=requests.Response)
    response.raise_for_status.return_value = None
    response.ok = 200 <= status < 300
    response.status_code = status
    response.json.return_value = body
    return response


class IcebergSyncSensorTests(unittest.TestCase):
    """`evaluate_sync_due` is the sensor's whole decision: the API says
    what is due, this side turns each item into one `RunRequest` with the
    API's own key and both D6 tags."""

    def test_two_due_items_become_two_run_requests_with_keys_config_and_tags(
        self,
    ) -> None:
        body = {
            "due": [
                {
                    "zone": "gold",
                    "mart": "orders",
                    "trigger": "change",
                    "runKey": "change:gold:orders:1795773540",
                },
                {
                    "zone": "silver",
                    "mart": "events",
                    "trigger": "schedule",
                    "runKey": "schedule:silver:events:1795773600",
                },
            ]
        }
        with mock.patch.object(
            iceberg_sync.requests, "get", return_value=_answer(body)
        ) as mocked_get:
            results = list(iceberg_sync.evaluate_sync_due(_cfg()))

        mocked_get.assert_called_once_with(
            "http://lakehouse-api.invalid:8080/api/gold/sync-due",
            headers={
                "x-run-token": "a-real-token",
                "Authorization": "Bearer a-real-token",
            },
            timeout=30,
        )
        self.assertEqual(len(results), 2)
        first, second = results
        self.assertIsInstance(first, RunRequest)
        self.assertEqual(first.run_key, "change:gold:orders:1795773540")
        self.assertEqual(
            first.run_config,
            {
                "ops": {
                    "iceberg_sync_target": {
                        "config": {
                            "zone": "gold",
                            "mart": "orders",
                            "trigger": "change",
                        }
                    }
                }
            },
        )
        self.assertEqual(
            first.tags,
            {
                "lakehouse/iceberg-sync-target": "gold.orders",
                "lakehouse/iceberg-sync-trigger": "change",
            },
        )
        self.assertEqual(second.run_key, "schedule:silver:events:1795773600")
        self.assertEqual(
            second.tags["lakehouse/iceberg-sync-target"], "silver.events"
        )

    def test_an_empty_due_list_is_one_skip_reason(self) -> None:
        with mock.patch.object(
            iceberg_sync.requests, "get", return_value=_answer({"due": []})
        ):
            results = list(iceberg_sync.evaluate_sync_due(_cfg()))
        self.assertEqual(len(results), 1)
        self.assertIsInstance(results[0], SkipReason)
        self.assertIn("no publication is due", results[0].skip_message)

    def test_a_body_without_a_due_list_is_a_non_retryable_failure(self) -> None:
        with mock.patch.object(
            iceberg_sync.requests, "get", return_value=_answer({"unexpected": True})
        ):
            with self.assertRaises(Failure) as ctx:
                list(iceberg_sync.evaluate_sync_due(_cfg()))
        self.assertFalse(ctx.exception.allow_retries)

    def test_a_due_item_without_its_key_or_with_an_unknown_zone_is_a_failure(
        self,
    ) -> None:
        for item in (
            {"zone": "gold", "mart": "orders", "trigger": "change"},
            {
                "zone": "lake",
                "mart": "orders",
                "trigger": "change",
                "runKey": "change:lake:orders:1",
            },
            {
                "zone": "gold",
                "mart": "orders",
                "trigger": "later",
                "runKey": "later:gold:orders:1",
            },
        ):
            with mock.patch.object(
                iceberg_sync.requests, "get", return_value=_answer({"due": [item]})
            ):
                with self.assertRaises(Failure) as ctx:
                    list(iceberg_sync.evaluate_sync_due(_cfg()))
            self.assertFalse(ctx.exception.allow_retries, item)

    def test_no_run_token_is_one_skip_reason_and_no_request(self) -> None:
        with mock.patch.object(iceberg_sync.requests, "get") as mocked_get:
            results = list(iceberg_sync.evaluate_sync_due(_cfg(run_token="")))
        mocked_get.assert_not_called()
        self.assertEqual(len(results), 1)
        self.assertIsInstance(results[0], SkipReason)
        self.assertIn("GOLD_EXPORT_RUN_TOKEN", results[0].skip_message)

    def test_the_sensor_is_registered_running_every_fifteen_seconds(self) -> None:
        self.assertEqual(iceberg_sync.iceberg_sync_sensor.minimum_interval_seconds, 15)
        self.assertIs(
            iceberg_sync.iceberg_sync_sensor.default_status,
            DefaultSensorStatus.RUNNING,
        )
        self.assertEqual(iceberg_sync.iceberg_sync_sensor.job_name, "iceberg_sync_job")

    def test_definitions_registers_the_job_and_the_sensor(self) -> None:
        from dispar_orchestrate.definitions import defs

        sensor_names = {sensor.name for sensor in defs.sensors}
        self.assertIn("iceberg_sync_sensor", sensor_names)
        job_names = {job.name for job in defs.resolve_all_job_defs()}
        self.assertIn("iceberg_sync_job", job_names)


class IcebergSyncJobTests(unittest.TestCase):
    def test_the_job_is_the_export_and_trim_for_one_table(self) -> None:
        node_names = set(iceberg_sync.iceberg_sync_job.graph.node_dict)
        self.assertEqual(
            node_names,
            {"iceberg_sync_target", "export_gold_mart", "trim_gold_mart"},
        )

    def test_an_unknown_zone_is_a_non_retryable_failure(self) -> None:
        context = build_op_context(
            op_config={"zone": "lake", "mart": "orders", "trigger": "change"}
        )
        with self.assertRaises(Failure) as ctx:
            iceberg_sync.iceberg_sync_target(context)
        self.assertFalse(ctx.exception.allow_retries)

    def test_a_known_zone_becomes_the_one_export_target(self) -> None:
        context = build_op_context(
            op_config={"zone": "silver", "mart": "orders", "trigger": "retry"}
        )
        target = iceberg_sync.iceberg_sync_target(context)
        self.assertEqual(target.mart, "orders")
        self.assertEqual(target.zone, "silver")
        self.assertEqual(target.iceberg_table, "silver.orders")
        self.assertTrue(target.require_enabled)

    def test_require_enabled_export_sends_the_query_guard(self) -> None:
        from dispar_orchestrate import gold_export

        with mock.patch.object(
            gold_export.requests,
            "post",
            return_value=_answer({"skipped": True, "reason": "publishing_off"}),
        ) as mocked_post:
            body = gold_export.export_one_mart(
                _cfg(), "orders", "gold", require_enabled=True
            )
        self.assertEqual(body, {"skipped": True, "reason": "publishing_off"})
        self.assertEqual(
            mocked_post.call_args.kwargs["params"],
            {"ifChanged": "true", "requireEnabled": "true"},
        )


class RequestIcebergSyncTests(unittest.TestCase):
    """`DATA-66` T6/D3: the authored op's ask never fails the write."""

    def test_a_gold_target_asks_with_zone_serving(self) -> None:
        context = _FakeContext()
        with mock.patch.object(
            iceberg_sync.GoldExportConfig, "from_env", return_value=_cfg()
        ), mock.patch.object(
            iceberg_sync.requests, "post", return_value=_answer({"launched": True})
        ) as mocked_post:
            iceberg_sync.request_iceberg_sync("serving", "orders_clean", context.log)

        mocked_post.assert_called_once_with(
            "http://lakehouse-api.invalid:8080/api/gold/export/orders_clean/after-write",
            params={"zone": "serving"},
            headers={
                "x-run-token": "a-real-token",
                "Authorization": "Bearer a-real-token",
            },
            timeout=10,
        )

    def test_a_silver_target_asks_with_zone_silver(self) -> None:
        context = _FakeContext()
        with mock.patch.object(
            iceberg_sync.GoldExportConfig, "from_env", return_value=_cfg()
        ), mock.patch.object(
            iceberg_sync.requests, "post", return_value=_answer({"launched": True})
        ) as mocked_post:
            iceberg_sync.request_iceberg_sync("silver", "orders", context.log)
        self.assertEqual(mocked_post.call_args.kwargs["params"], {"zone": "silver"})

    def test_an_unset_token_is_an_info_line_and_no_request(self) -> None:
        context = _FakeContext()
        with mock.patch.object(
            iceberg_sync.GoldExportConfig, "from_env", return_value=_cfg(run_token="")
        ), mock.patch.object(iceberg_sync.requests, "post") as mocked_post:
            iceberg_sync.request_iceberg_sync("serving", "orders_clean", context.log)
        mocked_post.assert_not_called()
        context.log.info.assert_called_once()
        context.log.warning.assert_not_called()

    def test_a_connection_error_never_raises_and_warns(self) -> None:
        context = _FakeContext()
        with mock.patch.object(
            iceberg_sync.GoldExportConfig, "from_env", return_value=_cfg()
        ), mock.patch.object(
            iceberg_sync.requests,
            "post",
            side_effect=requests.ConnectionError("connection refused"),
        ):
            iceberg_sync.request_iceberg_sync("serving", "orders_clean", context.log)
        context.log.warning.assert_called_once()

    def test_a_server_error_never_raises_and_names_the_fixed_reason(self) -> None:
        context = _FakeContext()
        with mock.patch.object(
            iceberg_sync.GoldExportConfig, "from_env", return_value=_cfg()
        ), mock.patch.object(
            iceberg_sync.requests,
            "post",
            return_value=_answer({"error": "upstream text"}, status=500),
        ):
            iceberg_sync.request_iceberg_sync("serving", "orders_clean", context.log)
        warning = context.log.warning.call_args.args[0]
        self.assertIn("status 500", warning)
        self.assertNotIn("upstream text", warning)

    def test_a_timeout_never_raises(self) -> None:
        context = _FakeContext()
        with mock.patch.object(
            iceberg_sync.GoldExportConfig, "from_env", return_value=_cfg()
        ), mock.patch.object(
            iceberg_sync.requests, "post", side_effect=requests.Timeout("timed out")
        ):
            iceberg_sync.request_iceberg_sync("serving", "orders_clean", context.log)
        context.log.warning.assert_called_once()

    def test_a_missing_env_never_raises(self) -> None:
        context = _FakeContext()
        with mock.patch.object(
            iceberg_sync.GoldExportConfig,
            "from_env",
            side_effect=Failure("CONNECTOR_S3_ACCESS_KEY must be set"),
        ), mock.patch.object(iceberg_sync.requests, "post") as mocked_post:
            iceberg_sync.request_iceberg_sync("serving", "orders_clean", context.log)
        mocked_post.assert_not_called()
        context.log.warning.assert_called_once()
