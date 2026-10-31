"""Unit tests for `cdc_apply.py`'s MongoDB case (SRC-4 slice 5, decisions S5-d and S5-e). No
network: the consumer is a fake, the Bronze table of the transaction test is a local `pyiceberg`
SQL catalog in a temp directory.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_cdc_apply_mongodb.py -q
"""

from __future__ import annotations

import json
from datetime import datetime, timezone
from types import SimpleNamespace
from typing import Any

import pyarrow as pa
import pytest
from bson import Binary, Decimal128, ObjectId
from kafka import TopicPartition
from pyiceberg.catalog.sql import SqlCatalog

from dispar_orchestrate import cdc_apply
from dispar_orchestrate.cdc_apply import (
    ApplyTarget,
    CdcEventError,
    CdcSetupError,
    apply_current,
    document_to_columns,
    event_to_row,
    read_batch,
    split_current,
    targets_of,
)

HEX = "65f0a1b2c3d4e5f607182930"
DIAL = {"driver": "mongodb", "database": "shop", "user": "reader", "replicaSet": "rs0"}
TP = TopicPartition("cdc.conn-m.shop.orders", 0)


def _event(op: str, id_: Any = HEX, deleted: str | None = None, **fields: Any) -> bytes:
    body: dict[str, Any] = {"_id": id_, **fields, "__op": op, "__source_ts_ms": 5_000}
    body["__deleted"] = deleted if deleted is not None else ("true" if op == "d" else "false")
    return json.dumps(body).encode()


def _row(raw: bytes, offset: int = 0) -> dict[str, Any]:
    row = event_to_row(raw, offset, document=True)
    assert row is not None
    return row


# ── the topic ──────────────────────────────────────────────────────────────────────────


def test_a_mongodb_collection_topic_is_the_prefix_the_database_and_the_collection() -> None:
    connector = {
        "id": "conn-m",
        "adapter": "cdc",
        "dial": DIAL,
        "sourceObjects": [{"name": "orders", "target": "orders", "primaryKey": ["_id"]}],
    }
    (target,) = targets_of(connector)
    assert target.topic == "cdc.conn-m.shop.orders"
    assert target.key == ("_id",)
    assert target.mode == "current"  # the default load mode, as for the other drivers
    assert target.document is True


def test_only_a_mongodb_dial_marks_its_targets_as_documents() -> None:
    for dial in ({"driver": "postgres", "database": "d"}, {"driver": "mysql", "database": "d"}, None):
        connector = {
            "id": "c",
            "dial": dial,
            "sourceObjects": [{"name": "public.t", "target": "t", "primaryKey": ["id"]}],
        }
        assert [t.document for t in targets_of(connector)] == [False]


def test_a_mongodb_dial_without_a_database_is_a_setup_error_not_a_guessed_topic() -> None:
    connector = {
        "id": "c",
        "dial": {"driver": "mongodb"},
        "sourceObjects": [{"name": "orders", "target": "orders", "primaryKey": ["_id"]}],
    }
    with pytest.raises(CdcSetupError):
        targets_of(connector)


def test_history_is_a_load_mode_for_a_mongodb_collection_and_no_other_mode_is() -> None:
    def connector(mode: str) -> dict[str, Any]:
        return {
            "id": "c",
            "dial": DIAL,
            "sourceObjects": [{"name": "o", "target": "o", "primaryKey": ["_id"], "loadMode": mode}],
        }

    assert [t.mode for t in targets_of(connector("history"))] == ["history"]
    assert [t.mode for t in targets_of(connector("incremental_dedup"))] == ["current"]
    with pytest.raises(CdcSetupError):
        targets_of(connector("replace"))


# ── a document into a row ──────────────────────────────────────────────────────────────


def test_an_insert_event_becomes_a_live_row_keyed_by_the_id_text() -> None:
    row = _row(_event("c", customer="ann", amount=12, vip=True))
    assert row["_id"] == HEX
    assert (row["customer"], row["amount"], row["vip"]) == ("ann", 12, True)
    assert (row["_op"], row["_deleted"], row["_source_ts_ms"], row["_offset"]) == ("c", False, 5_000, 0)


def test_a_snapshot_read_is_a_live_row() -> None:
    row = _row(_event("r", customer="bob"))
    assert (row["_op"], row["_deleted"], row["customer"]) == ("r", False, "bob")


def test_an_update_event_carries_the_whole_document() -> None:
    row = _row(_event("u", customer="ann", amount=99), offset=7)
    assert (row["_op"], row["amount"], row["_offset"]) == ("u", 99, 7)


def _delete_without_id() -> bytes:
    # Seen in CI (`g4-mongo`, Debezium 3.8): a delete's value has no document and no `_id`, only these.
    return json.dumps({"__op": "d", "__source_ts_ms": 5_000, "__deleted": "true"}).encode()


def test_a_delete_event_takes_its_id_from_the_message_key_because_its_value_has_none() -> None:
    row = event_to_row(_delete_without_id(), 3, document=True, key=json.dumps({"id": HEX}).encode())
    assert row is not None
    assert row["_id"] == HEX
    assert (row["_op"], row["_deleted"], row["_offset"]) == ("d", True, 3)
    assert "customer" not in row


def test_a_delete_whose_value_does_carry_the_id_keeps_it() -> None:
    row = _row(_event("d"))
    assert row["_id"] == HEX and row["_deleted"] is True


def test_the_key_id_of_an_object_id_is_its_hex_text_and_other_ids_are_parsed() -> None:
    def key(text: Any) -> bytes:
        return json.dumps({"id": text}).encode()

    assert cdc_apply.key_id(key(HEX)) == HEX
    assert cdc_apply.key_id(key(HEX.upper())) == HEX  # the same ObjectId, one spelling
    # 24 digits are an ObjectId, not the number they would read as.
    assert cdc_apply.key_id(key("123456789012345678901234")) == "123456789012345678901234"
    assert cdc_apply.key_id(key("1234")) == 1234  # a numeric `_id`
    assert cdc_apply.key_id(key('"abc"')) == "abc"  # a string `_id`: extended JSON text, quotes included
    assert cdc_apply.key_id(key('{"$oid": "' + HEX + '"}')) == HEX
    assert cdc_apply.key_id(key("not json, not hex")) == "not json, not hex"
    for nothing in (None, b"not json", b"[1]", json.dumps({"other": 1}).encode()):
        assert cdc_apply.key_id(nothing) is None


def test_a_delete_without_a_key_has_no_id_and_is_still_a_row_not_a_crash() -> None:
    row = event_to_row(_delete_without_id(), 0, document=True)
    assert row is not None and "_id" not in row


def test_an_object_id_in_extended_json_becomes_its_hex_string_whichever_way_it_arrives() -> None:
    as_object = _row(_event("c", id_={"$oid": HEX}))
    as_text = _row(_event("c", id_='{"$oid": "' + HEX + '"}'))
    assert as_object["_id"] == as_text["_id"] == HEX
    # Another key or value is left alone.
    assert document_to_columns({"_id": 5, "__op": "c"})["_id"] == 5
    assert document_to_columns({"_id": '{"x": 1}', "__op": "c"})["_id"] == '{"x": 1}'


def test_the_values_are_converted_by_the_batch_adapters_own_helper() -> None:
    # The same function, not a copy: what the batch adapter stores for an ObjectId, a
    # Decimal128 and binary is what a captured document stores.
    from dispar_orchestrate.adapters import mongodb

    assert mongodb.bson_safe(ObjectId(HEX)) == HEX
    assert document_to_columns({"_id": ObjectId(HEX), "__op": "c"})["_id"] == HEX
    assert document_to_columns({"_id": 1, "p": Decimal128("1.5"), "__op": "c"})["p"] == mongodb.bson_safe(
        Decimal128("1.5")
    )
    assert document_to_columns({"_id": 1, "b": Binary(b"ab"), "__op": "c"})["b"] == b"ab"


def test_a_nested_document_or_array_is_refused_without_quoting_it() -> None:
    for nested in ({"city": "secret-city"}, ["secret-item"]):
        with pytest.raises(CdcEventError) as caught:
            _row(_event("c", customer="x", address=nested))
        assert "secret" not in str(caught.value)
    # The refusal is the batch adapter's rule, so a flat document passes.
    assert _row(_event("c", customer="x"))["customer"] == "x"


def test_a_message_that_is_not_an_event_is_refused_for_a_document_as_for_a_row() -> None:
    for bad in (b"not json", b"[1]", json.dumps({"_id": HEX}).encode()):
        with pytest.raises(CdcEventError):
            event_to_row(bad, 0, document=True)
    assert event_to_row(None, 0, document=True) is None


def test_a_relational_event_is_still_read_without_the_document_rules() -> None:
    # A nested value in a relational event is a JSON column: it is not this module's to refuse.
    raw = json.dumps({"id": 1, "payload": {"a": 1}, "__op": "c", "__source_ts_ms": 1, "__deleted": "false"})
    assert event_to_row(raw, 0)["payload"] == {"a": 1}
    with pytest.raises(CdcEventError):
        event_to_row(raw, 0, document=True)


def test_the_id_is_the_key_and_only_the_last_event_per_key_counts() -> None:
    rows = [
        _row(_event("c", customer="a"), 0),
        _row(_event("u", customer="b"), 1),
        _row(_event("c", id_="65f0a1b2c3d4e5f607182931", customer="x"), 2),
        _row(_event("d", id_="65f0a1b2c3d4e5f607182931"), 3),
    ]
    survivors, touched = split_current(rows, ("_id",))
    assert [r["customer"] for r in survivors] == ["b"]
    assert touched == [(HEX,), ("65f0a1b2c3d4e5f607182931",)]


# ── a batch of documents is read as documents, a batch of rows is not ───────────────────


class _Consumer:
    def __init__(self, values: list[bytes | None]) -> None:
        self._batches = [[SimpleNamespace(offset=i, value=v, key=None) for i, v in enumerate(values)]]

    def poll(self, timeout_ms: int, max_records: int):
        return {TP: self._batches.pop(0)} if self._batches else {}


def test_read_batch_reads_documents_when_told_to() -> None:
    batch = read_batch(
        _Consumer([_event("c", customer="a"), None, _event("d")]), poll_ms=0, document=True
    )
    assert [r["_op"] for r in batch.rows] == ["c", "d"]  # the tombstone carries nothing
    assert batch.next_offsets == {TP: 3}


# ── the current-row transaction on a document table ─────────────────────────────────────


UTC = pa.timestamp("us", tz="UTC")
SCHEMA = pa.schema(
    [
        pa.field("_id", pa.string()),
        pa.field("customer", pa.string()),
        pa.field("_source_ts_ms", pa.int64()),
        pa.field("_ingested_at", UTC),
        pa.field("_dlt_load_id", pa.string(), nullable=False),
        pa.field("_dlt_id", pa.string(), nullable=False),
    ]
)
NOW = lambda: datetime(2026, 10, 30, 12, 0, tzinfo=timezone.utc)  # noqa: E731


@pytest.fixture
def table(tmp_path):
    catalog = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/wh")
    catalog.create_namespace("bronze")
    return catalog.create_table("bronze.orders", schema=SCHEMA)


def _state(table) -> dict[str, str]:
    return {r["_id"]: r["customer"] for r in table.scan().to_arrow().to_pylist()}


def _apply(table, *events: bytes) -> None:
    rows = [_row(e, i) for i, e in enumerate(events)]
    apply_current(table.refresh(), rows, ("_id",), table="orders", now=NOW)


def test_a_batch_inserts_updates_and_deletes_documents_by_their_id(table) -> None:
    other = "65f0a1b2c3d4e5f607182931"
    _apply(table, _event("c", customer="a"), _event("c", id_=other, customer="z"))
    assert _state(table) == {HEX: "a", other: "z"}
    _apply(table, _event("u", customer="b"), _event("d", id_=other))
    assert _state(table) == {HEX: "b"}


def test_replaying_a_document_batch_gives_the_same_table(table) -> None:
    events = (_event("c", customer="a"), _event("u", customer="b"))
    _apply(table, *events)
    first = _state(table)
    _apply(table, *events)
    assert _state(table) == first == {HEX: "b"}


def test_the_id_column_name_is_the_name_bronze_has() -> None:
    # `dlt` keeps `_id`, so the key of the source object and the column of the row agree.
    assert cdc_apply._loaded_name("_id") == "_id"
    assert ApplyTarget("c", "orders", "t", "orders", ("_id",), "current", "g", True).document


def test_read_batch_hands_each_records_key_to_the_document_reader() -> None:
    class _Keyed:
        def poll(self, timeout_ms: int, max_records: int):
            if getattr(self, "done", False):
                return {}
            self.done = True
            record = SimpleNamespace(offset=0, value=_delete_without_id(), key=json.dumps({"id": HEX}).encode())
            return {TP: [record]}

    batch = read_batch(_Keyed(), poll_ms=0, document=True)
    assert [(r["_id"], r["_op"], r["_deleted"]) for r in batch.rows] == [(HEX, "d", True)]
