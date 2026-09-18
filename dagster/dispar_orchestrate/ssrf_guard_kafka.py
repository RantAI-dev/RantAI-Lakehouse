"""dagster/dispar_orchestrate/ssrf_guard_kafka.py -- WS9 plan hard
requirement 1: Kafka's bootstrap host is not the only host the client
ever dials. After connecting to ANY bootstrap broker, `kafka-python`
(like every Kafka client) asks for cluster metadata and then dials
whatever "advertised listener" addresses that metadata names for each
partition's leader/replicas -- addresses the BROKER chooses, not the
operator who configured `dial.bootstrapServers`. A cluster reachable at
a public bootstrap address can advertise an internal address for its
real data brokers; `ssrf_guard.resolve_checked` on the bootstrap host
alone never inspects those. This module checks EVERY broker address a
metadata response names, via the SAME resolve_checked rule, before
`adapters/kafka.py` is allowed to call `.poll()` even once -- an early,
readable pre-check, NOT the only defence: `ssrf_guard.checking_resolver`
(WS9 plan Task C1, `ssrf_guard.py`) covers the mid-batch reconnect window
this one-shot check alone cannot (WS9 judge review K1).

Fail-closed: ANY single advertised broker failing the check aborts the
WHOLE batch (no partial consumption from "the brokers that happened to
be safe") -- a cluster that advertises even one internal address is
refused outright, since there is no way to know in advance which
partition's data the caller's topic actually needs without already
trusting the (unsafe) metadata.
"""

from __future__ import annotations

from dataclasses import dataclass

from dispar_orchestrate.adapters.sql import _validate_hostname
from dispar_orchestrate.ssrf_guard import ResolvedAddress, resolve_checked as _default_resolve_checked


@dataclass(frozen=True)
class BrokerMetadata:
    """The subset of a Kafka `Metadata` response this guard needs:
    every broker address the cluster currently advertises. Built by
    `adapters/kafka.py` from `kafka-python`'s `KafkaConsumer._client.
    cluster.brokers()` -- kept as its own small, independently-testable
    type so this module never imports `kafka-python` itself (no
    network-capable dependency in a pure-logic test module)."""

    brokers: list[tuple[str, int]]


def check_all_advertised_brokers(
    metadata: BrokerMetadata, *, resolve_checked=_default_resolve_checked
) -> list[ResolvedAddress]:
    """Resolve-and-check EVERY broker `metadata` names. Raises
    `SsrfBlocked` (propagated from `resolve_checked`) on the first
    blocked address found -- the caller (`adapters/kafka.py`) never
    calls `.poll()` unless this returns successfully for the FULL,
    current broker list, re-checked on every micro-batch run (not
    cached across runs, since a cluster's advertised addresses can
    change between runs -- DNS/config drift, not just DNS TTL).

    `_validate_hostname` (WS3 Task F2/A3, Z13's Python port, imported
    from `adapters.sql` rather than re-implemented here -- ONE canonical
    hostname-shape rule for this whole workstream, per Task A3's own
    stated reason) runs BEFORE `resolve_checked` for every advertised
    host: an advertised address is server-supplied text, and this
    module's own `dial.bootstrapServers` entries already pass the SAME
    check at `Dial::parse` time (Task A2) -- an advertised host that is
    not even hostname-shaped (e.g. carries `;`) is refused here for the
    identical reason, before a DNS lookup is even attempted."""
    for host, _port in metadata.brokers:
        _validate_hostname("advertised broker host", host)
    return [resolve_checked(host, port) for host, port in metadata.brokers]
