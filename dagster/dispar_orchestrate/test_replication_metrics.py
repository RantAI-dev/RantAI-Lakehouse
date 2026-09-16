"""Unit tests for `dagster/dispar_orchestrate/replication_metrics.py`'s
registry-driven connector-id attribution (WS3 plan review X4).

Before this task, `check_replication_slots` attributed a live
`pg_replication_slots` row to a connector by stripping a `_slot` suffix off
the slot name (`slot_name[: -len("_slot")]`), unconditionally — a guessed
attribution the module doc now calls a fabricated metric (AGENTS.md rule
2). Once `ops/debezium/render_compose.py` assigns a CDC connector's slot
and publication names from its registry-owned `dial` (rather than deriving
them from the connector id), a slot name need not resemble its connector
id at all, so attribution must come from `GET /api/connectors/ingestible`
instead.

No real network: `requests.get` and `psycopg2.connect` are both
monkeypatched, the same style `test_agent_runs.py` uses for
`_fetch_schedulable_employees`. `psycopg2.connect` is stubbed with a fake
connection/cursor pair so these tests never require a live Postgres
server — the plan's fixture names (`cfg_with_two_unnamed_slots`,
`api_unreachable`) are reproduced here as small local helpers.

Run with:
  ~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_replication_metrics.py -q
"""

from __future__ import annotations

import unittest
from unittest import mock

import requests

from dispar_orchestrate import replication_metrics
from dispar_orchestrate.bronze_catalog import ClickHouseTarget


def _cfg(ingest_service_token: str = "a-real-token") -> replication_metrics.ReplicationConfig:
    return replication_metrics.ReplicationConfig(
        ch=ClickHouseTarget(url="http://clickhouse.invalid:8123", user="u", password="p"),
        source_db_host="source-db.invalid",
        source_db_port="5432",
        source_db_user="lakehouse",
        source_db_password="lakehouse",
        source_db_name="lakehouse",
        api_url="http://lakehouse-api.invalid:8080",
        ingest_service_token=ingest_service_token,
    )


class _FakeCursor:
    """A `psycopg2` cursor stand-in that returns a fixed set of
    `pg_replication_slots` rows and no-ops on `execute`, so
    `check_replication_slots`'s SQL never actually runs against a real
    server — only the shape of the returned tuples matters here."""

    def __init__(self, rows: list[tuple]) -> None:
        self._rows = rows

    def __enter__(self) -> "_FakeCursor":
        return self

    def __exit__(self, *exc_info: object) -> None:
        return None

    def execute(self, *args: object, **kwargs: object) -> None:
        return None

    def fetchall(self) -> list[tuple]:
        return self._rows


class _FakeConnection:
    def __init__(self, rows: list[tuple]) -> None:
        self._rows = rows

    def set_session(self, *args: object, **kwargs: object) -> None:
        return None

    def cursor(self) -> _FakeCursor:
        return _FakeCursor(self._rows)

    def close(self) -> None:
        return None


def _slot_row(slot_name: str, *, active: bool = True) -> tuple:
    """One `pg_replication_slots` row shaped exactly as
    `check_replication_slots`'s `SELECT` unpacks it: `(slot_name, active,
    wal_retained_bytes, confirmed_flush_lag_bytes)`. `wal_retained_bytes=0`
    keeps `_status_for` at `"ok"` so these tests aren't also asserting on
    the unrelated WAL-threshold logic."""
    return (slot_name, active, 0, 0)


def _patched_connect(rows: list[tuple]):
    return mock.patch.object(
        replication_metrics.psycopg2, "connect", return_value=_FakeConnection(rows)
    )


def _ingestible_response(connectors: list[dict]) -> mock.Mock:
    response = mock.Mock(spec=requests.Response)
    response.raise_for_status.return_value = None
    response.json.return_value = connectors
    return response


class CheckReplicationSlotsRegistryAttributionTest(unittest.TestCase):
    def test_resolves_connector_id_via_registry_dial_not_slot_name_shape(self) -> None:
        """The decisive proof that attribution is registry-driven, not
        string-derived: the live slot is named `custom_slot_v2`, which does
        NOT have the old `<connector_slug>_slot` shape at all, yet still
        resolves — because a `dial.slotName` in the registry names it
        exactly, and nothing about the connector's `id` needs to match."""
        connectors = [
            {"id": "conn-mysql-1", "adapter": "mysql", "dial": {"slotName": "custom_slot_v2"}},
            {"id": "conn-cdc-orders", "adapter": "cdc", "dial": {"slotName": "custom_slot_v2"}},
        ]
        with mock.patch.object(
            replication_metrics.requests, "get", return_value=_ingestible_response(connectors)
        ), _patched_connect([_slot_row("custom_slot_v2")]):
            slots = replication_metrics.check_replication_slots(_cfg())

        self.assertEqual(len(slots), 1)
        self.assertEqual(slots[0]["connector_id"], "conn-cdc-orders")

    def test_two_unresolved_slots_in_one_run_stay_two_rows(self) -> None:
        """WS3 plan review Z3: `lake.bronze_meta.replication_slot` is a
        `ReplacingMergeTree ORDER BY (connector_id, checked_at)` and
        `slot_name` is not in that key. Two unresolved slots checked in the
        SAME run would share one `checked_at` — if both were attributed to
        the same bare `""` sentinel they would share the full sorting key
        and a later ClickHouse merge would keep only one, silently
        discarding the other slot's WAL metrics. Asserting on the rows
        `check_replication_slots` returns (before any ClickHouse write)
        pins the row-construction step, since a real merge only happens on
        ClickHouse's own background schedule."""
        with mock.patch.object(
            replication_metrics.requests, "get", return_value=_ingestible_response([])
        ), _patched_connect([_slot_row("slot_a"), _slot_row("slot_b")]):
            slots = replication_metrics.check_replication_slots(_cfg())

        connector_ids = {row["connector_id"] for row in slots}
        self.assertEqual(connector_ids, {"unresolved:slot_a", "unresolved:slot_b"})

    def test_attributes_unresolved_slot_with_a_reason_when_registry_is_unreachable(self) -> None:
        """`requests.get` raising must degrade to the `unresolved:<slot>`
        sentinel (never the old slug-stripped guess, never a bare `""`)
        and must log a warning naming the slot and the exact reason
        `"registry unreachable"` — this module's `print`-based warning
        style, the same one `agent_runs.py::_fetch_schedulable_employees`
        uses."""
        with mock.patch.object(
            replication_metrics.requests,
            "get",
            side_effect=requests.ConnectionError("connection refused"),
        ), _patched_connect([_slot_row("orders_slot")]), mock.patch("builtins.print") as mocked_print:
            slots = replication_metrics.check_replication_slots(_cfg())

        self.assertEqual(len(slots), 1)
        self.assertEqual(slots[0]["connector_id"], "unresolved:orders_slot")
        printed = " ".join(str(call.args[0]) for call in mocked_print.call_args_list)
        self.assertIn("orders_slot", printed)
        self.assertIn("registry unreachable", printed)

    def test_attributes_unresolved_slot_with_a_reason_when_no_cdc_connector_matches(self) -> None:
        """The registry answers successfully but no `adapter == "cdc"`
        connector's `dial.slotName` matches — a DIFFERENT reason than
        "registry unreachable", and the warning must say so."""
        connectors = [{"id": "conn-other", "adapter": "cdc", "dial": {"slotName": "other_slot"}}]
        with mock.patch.object(
            replication_metrics.requests, "get", return_value=_ingestible_response(connectors)
        ), _patched_connect([_slot_row("orders_slot")]), mock.patch("builtins.print") as mocked_print:
            slots = replication_metrics.check_replication_slots(_cfg())

        self.assertEqual(slots[0]["connector_id"], "unresolved:orders_slot")
        printed = " ".join(str(call.args[0]) for call in mocked_print.call_args_list)
        self.assertIn("orders_slot", printed)
        self.assertIn("no adapter=cdc connector's dial.slotName matches this slot", printed)
        self.assertNotIn("registry unreachable", printed)

    def test_unresolved_sentinel_is_never_a_bare_empty_string_or_the_old_guess(self) -> None:
        """Regression pin for the exact fabricated-attribution shape this
        task removes: a slot literally named `orphan_slot` (no matching
        connector) must never end up with `connector_id == "orphan"` (the
        old `_slot`-suffix-stripped guess) and never `connector_id == ""`."""
        with mock.patch.object(
            replication_metrics.requests, "get", return_value=_ingestible_response([])
        ), _patched_connect([_slot_row("orphan_slot")]):
            slots = replication_metrics.check_replication_slots(_cfg())

        self.assertEqual(slots[0]["connector_id"], "unresolved:orphan_slot")
        self.assertNotEqual(slots[0]["connector_id"], "orphan")
        self.assertNotEqual(slots[0]["connector_id"], "")

    def test_unset_ingest_service_token_resolves_every_slot_unresolved_with_no_http_call(self) -> None:
        with mock.patch.object(replication_metrics.requests, "get") as mocked_get, _patched_connect(
            [_slot_row("orders_slot")]
        ):
            slots = replication_metrics.check_replication_slots(_cfg(ingest_service_token=""))

        mocked_get.assert_not_called()
        self.assertEqual(slots[0]["connector_id"], "unresolved:orders_slot")


if __name__ == "__main__":
    unittest.main()
