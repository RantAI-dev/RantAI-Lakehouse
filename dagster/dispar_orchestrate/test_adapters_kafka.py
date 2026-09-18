"""Unit tests for `dagster/dispar_orchestrate/adapters/kafka.py`.

No real network: `KafkaConsumer` is never constructed -- every test drives
a fake consumer exposing only the surface `consume_one_batch` uses
(`_client.cluster.brokers()`, `.poll(...)`), matching
`test_adapters_mongodb.py`'s fake-driver style. `resolve_checked`/
`checking_resolver` are injected, never the real network-touching
defaults, so `SsrfBlocked` behaviour is exercised with no DNS/socket call.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_adapters_kafka.py -q
"""

from __future__ import annotations

import contextlib

import pytest

from dispar_orchestrate.adapters.kafka import BatchResult, consume_one_batch
from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked


class _FakeBroker:
    def __init__(self, host, port):
        self.host, self.port = host, port


class _FakeCluster:
    def __init__(self, brokers):
        self._brokers = brokers

    def brokers(self):
        return self._brokers


class _FakeClient:
    def __init__(self, brokers):
        self.cluster = _FakeCluster(brokers)


class _FakeRecord:
    def __init__(self, value, partition, offset):
        self.value, self.partition, self.offset = value, partition, offset


class _FakeConsumer:
    """Mirrors kafka-python's `KafkaConsumer` surface this module uses:
    `_client.cluster.brokers()` (metadata pre-check), `.poll(...)`
    (returns `{TopicPartition: [records]}`, kafka-python's real shape --
    not one record at a time), `.commit(...)`."""

    def __init__(self, brokers, batches):
        self._client = _FakeClient(brokers)
        self._batches = list(batches)  # list of {TopicPartition: [record, ...]}

    def poll(self, timeout_ms=1000, max_records=500):
        return self._batches.pop(0) if self._batches else {}

    def close(self):
        pass


def test_consume_one_batch_refuses_if_any_advertised_broker_is_internal():
    consumer = _FakeConsumer(brokers=[_FakeBroker("internal.invalid", 9092)], batches=[])

    def _resolve(host, port):
        raise SsrfBlocked("refused")

    with pytest.raises(SsrfBlocked):
        consume_one_batch(consumer, topic="orders", max_seconds=1, resolve_checked=_resolve)


def test_consume_one_batch_returns_messages_and_max_offset_per_partition():
    from kafka import TopicPartition

    tp = TopicPartition("orders", 0)
    consumer = _FakeConsumer(
        brokers=[_FakeBroker("broker-a.invalid", 9092)],
        batches=[{tp: [_FakeRecord(b'{"id":1}', 0, 10), _FakeRecord(b'{"id":2}', 0, 11)]}],
    )
    result = consume_one_batch(
        consumer,
        topic="orders",
        max_seconds=1,
        resolve_checked=lambda h, p: ResolvedAddress(ip="93.184.216.34", port=p, family=2),
        checking_resolver=lambda **_: contextlib.nullcontext(),
    )
    assert isinstance(result, BatchResult)
    assert [r["id"] for r in result.rows] == [1, 2]
    assert result.offsets_to_commit == {0: 11}  # last offset seen, not count


def test_consume_one_batch_wraps_the_whole_poll_loop_in_checking_resolver():
    # K1: not just a pre-check -- the WHOLE batch (pre-check and every
    # poll iteration) runs inside checking_resolver, so a broker
    # re-advertised and reconnected to MID-BATCH is still checked.
    entered = []

    class _FakeCtx:
        def __enter__(self):
            entered.append(True)

        def __exit__(self, *a):
            return False

    consumer = _FakeConsumer(brokers=[_FakeBroker("broker-a.invalid", 9092)], batches=[])
    consume_one_batch(
        consumer,
        topic="orders",
        max_seconds=0,
        resolve_checked=lambda h, p: ResolvedAddress(ip="93.184.216.34", port=p, family=2),
        checking_resolver=lambda **_: _FakeCtx(),
    )
    assert entered == [True]


def test_consume_one_batch_never_advances_the_offset_before_the_caller_confirms_the_write():
    # The offset commit itself is the CALLER's job (ingest_factory.py's
    # run_kafka_stream_batch) -- consume_one_batch itself never writes to
    # bronze_meta.ingest_offset or calls consumer.commit, only RETURNS
    # what the caller should commit once the sink write succeeds. Proven
    # by absence: no import of bronze_catalog here, and no `.commit(`
    # call anywhere in the module's own source.
    import inspect

    import dispar_orchestrate.adapters.kafka as kafka_mod

    source = inspect.getsource(kafka_mod)
    assert "bronze_catalog" not in source
    assert ".commit(" not in source


def test_offset_and_metadata_is_built_with_one_positional_arg_not_a_none_metadata():
    # K5's own warning, verified executably rather than only in a doc
    # comment: kafka-python 3.0.11's OffsetAndMetadata field list/defaults
    # changed after 2.0 -- a `leader_epoch` field was added and `metadata`
    # defaults to `''`, not `None`.
    from kafka.structs import OffsetAndMetadata

    built = OffsetAndMetadata(6)
    assert built.offset == 6
    assert built.metadata == ""  # the real default (kafka/structs.py), never None
    assert built.leader_epoch == -1
