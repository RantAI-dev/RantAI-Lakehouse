"""dagster/dispar_orchestrate/adapters/kafka.py -- hand-written over
`kafka.KafkaConsumer` (`kafka-python==3.0.11`: pure Python, every
connection resolves through `socket.getaddrinfo`, unlike
`confluent-kafka`'s C-based `librdkafka` transport, which this build does
not depend on; `3.0.11` is the current maintained release -- see
`dagster/pyproject.toml`'s own note on that pin). No dlt kafka
source ships in the installed package, so this adapter is hand-written
rather than routed through `dlt_pipeline.py`.

API SHAPES VERIFIED AGAINST kafka-python==3.0.11's REAL, INSTALLED
SOURCE (`~/.cache/rantai-dagster-venv/lib/python3.14/site-packages/kafka`,
read directly, not assumed from a library's changelog):
- `KafkaConsumer(bootstrap_servers=..., group_id=..., enable_auto_commit=...,
  value_deserializer=...)` -- all four are real `DEFAULT_CONFIG` keys
  (`kafka/consumer/group.py`'s `DEFAULT_CONFIG` dict).
- `consumer.poll(timeout_ms=0, max_records=None, update_offsets=True) ->
  dict[TopicPartition, list[ConsumerRecord]]` (`kafka/consumer/group.py:778`).
  `ConsumerRecord` carries `.value`, `.offset`, `.partition` fields.
- `consumer.commit` takes `offsets={TopicPartition: OffsetAndMetadata}`
  (`kafka/consumer/group.py:648`); the commit convention is "the next
  offset your application should consume", i.e. `last_offset + 1`. This
  module never calls it -- see "STREAM HONESTY" below.
- `consumer.end_offsets(partitions, timeout_ms=None) -> {TopicPartition: int}`
  (`kafka/consumer/group.py:1253`).
- `OffsetAndMetadata` -- `kafka/structs.py`:
  `namedtuple("OffsetAndMetadata", ["offset", "metadata", "leader_epoch"],
  defaults=[None, '', -1])` -- THREE fields, `metadata` DEFAULTS TO `''`,
  not `None`. Since every field carries a default, `OffsetAndMetadata(offset
  + 1)` alone is the correct, complete call -- this module never passes
  `metadata=None` explicitly, which would diverge from the field's own
  declared default for no benefit.
- `consumer._client.cluster.brokers() -> list[MetadataResponseBroker]` --
  `KafkaConsumer.__init__` assigns `self._client = self.config['kafka_client'](...)`
  (`kafka/consumer/group.py:418`), and the client's own `cluster` attribute
  is a `ClusterMetadata` whose `.brokers()` (`kafka/cluster.py:235`) lists
  every broker the client currently knows about, each with `.host`/`.port`
  fields (among `node_id`/`rack`).

SSRF: TWO layers, not one.
1. `ssrf_guard_kafka.check_all_advertised_brokers` is an early,
   readable pre-check against the metadata `KafkaConsumer` already holds
   (`consumer._client.cluster.brokers()` -- kafka-python's own internal,
   but standard and widely used, cluster-introspection accessor; there is
   no public alternative in this client library) -- a fast rejection for
   the common case, not the only defence.
2. `ssrf_guard.checking_resolver()` wraps the ENTIRE batch --
   pre-check AND every `.poll()` call -- so a broker re-advertised and
   reconnected to mid-batch (kafka-python refreshes cluster metadata
   periodically and reconnects the same way any other connection does)
   is checked at the moment it is actually dialed, closing the window a
   one-shot pre-check alone leaves open.

STREAM HONESTY: `consume_one_batch` is a
pure, bounded read -- it returns the rows read and the LAST offset per
partition seen, but commits NOTHING (no broker-side consumer-group
commit, no `bronze_meta.ingest_offset` write). The caller
(`ingest_factory.py::run_kafka_stream_batch`) calls
`adapters.sink.load_via_sink` on the returned rows FIRST, and only on
that call's success commits `offsets_to_commit` -- both to Kafka's own
consumer-group offset and to `bronze_meta.ingest_offset` -- so a crash
between read and write never advances the recorded offset past
unwritten data. At-least-once: a crash after a successful sink write but
before the commit itself lands causes the same batch to be reconsumed
and rewritten on the next scheduled run (an Iceberg append, not
deduplicated -- a stated limitation, not hidden).
"""

from __future__ import annotations

import json
import time
from dataclasses import dataclass, field

from kafka import KafkaConsumer, TopicPartition

from dispar_orchestrate import ssrf_guard
from dispar_orchestrate.column_gate import reject_unsupported_column_types_from_sample
from dispar_orchestrate.ssrf_guard_kafka import BrokerMetadata, check_all_advertised_brokers


@dataclass(frozen=True)
class BatchResult:
    rows: list[dict] = field(default_factory=list)
    offsets_to_commit: dict[int, int] = field(default_factory=dict)


class BrokerListUnavailable(Exception):
    """The consumer's advertised-broker list could not be read, so the
    pre-check cannot run. Raised instead of proceeding: see
    [`_advertised_brokers`]."""


def _advertised_brokers(consumer) -> BrokerMetadata:
    """Read the brokers the cluster has advertised to this consumer.

    This reaches through `consumer._client.cluster` — kafka-python's own
    internal accessor, cited with file:line in this module's docstring,
    and the only way to see the advertised list. The pin in
    `dagster/pyproject.toml` is exact, so that path changing is a
    deliberate upgrade rather than a surprise; what must not happen is
    that upgrade turning into a bare `AttributeError` deep inside a batch.
    A list we cannot read is a list we cannot check, and an unchecked
    broker set is exactly what the guard exists to prevent — so this
    refuses by name, and the batch never starts.
    """
    try:
        brokers = consumer._client.cluster.brokers()  # noqa: SLF001 - see docstring
    except AttributeError as exc:
        raise BrokerListUnavailable(
            "cannot read the consumer's advertised broker list "
            "(kafka-python's internal cluster accessor changed shape); "
            "refusing to consume rather than dial unchecked brokers"
        ) from exc
    return BrokerMetadata(brokers=[(b.host, b.port) for b in brokers])


def consume_one_batch(
    consumer: KafkaConsumer,
    *,
    topic: str,
    max_seconds: int,
    resolve_checked=ssrf_guard.resolve_checked,
    checking_resolver=ssrf_guard.checking_resolver,
) -> BatchResult:
    """Bounded read of one micro-batch. Checks every advertised broker
    BEFORE the first `.poll()` (`check_all_advertised_brokers`), then
    keeps `ssrf_guard.checking_resolver()` installed for the WHOLE poll
    loop -- not just that one pre-check -- so a broker this consumer
    reconnects to mid-batch (kafka-python's own periodic metadata
    refresh) is still validated at the moment it is actually dialed.

    Returns the rows read and, per partition, the LAST (highest) offset
    seen -- never a count -- so the caller can build the exact
    `OffsetAndMetadata(offset + 1)` commit kafka-python's own convention
    requires. Commits NOTHING itself: see this module's docstring for
    why that is the caller's job, done only after a successful sink
    write.
    """
    check_all_advertised_brokers(
        _advertised_brokers(consumer), resolve_checked=resolve_checked
    )

    rows: list[dict] = []
    offsets: dict[int, int] = {}
    with checking_resolver():
        deadline = time.monotonic() + max_seconds
        while time.monotonic() < deadline:
            remaining_ms = max(0, int((deadline - time.monotonic()) * 1000))
            records_by_partition = consumer.poll(timeout_ms=min(1000, remaining_ms), max_records=500)
            for tp, records in records_by_partition.items():
                for record in records:
                    row = json.loads(record.value)
                    # R7: a Kafka message carries no declared
                    # column types either, so the batch's FIRST decoded
                    # record is the sample this gate inspects — a nested
                    # JSON array/object is refused before the batch is
                    # written, not flattened by the sink. Refusing here
                    # also means no offset is committed for a batch whose
                    # shape this pipeline cannot represent, because the
                    # commit only happens after a successful sink write.
                    if not rows:
                        reject_unsupported_column_types_from_sample(row)
                    rows.append(row)
                    offsets[tp.partition] = max(offsets.get(tp.partition, -1), record.offset)
    return BatchResult(rows=rows, offsets_to_commit=offsets)


def compute_lag(consumer: KafkaConsumer, *, topic: str, partition: int, committed_offset: int) -> int | None:
    """Real lag ONLY: the difference between the topic's live high-water
    mark (`end_offsets`, a live broker call) and the last COMMITTED
    offset (never the last CONSUMED one -- that would hide an
    uncommitted, still-at-risk batch). Returns `None` if the watermark
    call itself fails -- never a fabricated `0` (AGENTS.md principle 2,
    "never fabricate")."""
    try:
        tp = TopicPartition(topic, partition)
        high = consumer.end_offsets([tp])[tp]
    except Exception:  # noqa: BLE001 -- any broker/network failure means "not measured", not a crash here
        return None
    return max(0, high - committed_offset)
