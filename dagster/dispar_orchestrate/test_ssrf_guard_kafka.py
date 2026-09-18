"""Tests for `ssrf_guard_kafka.check_all_advertised_brokers` (WS9 plan
Task C1). Covers the exact scenario the task brief names: a fake
metadata response advertising an internal address for one of the
cluster's brokers, distinct from the (public) bootstrap host.

No network: `resolve_checked` is faked per-test rather than resolving
anything for real (AGENTS.md Python rule: no network in unit tests).
"""

from __future__ import annotations

import pytest

from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked
from dispar_orchestrate.ssrf_guard_kafka import BrokerMetadata, check_all_advertised_brokers


def _resolver(mapping: dict[str, str]):
    def _fn(host, port):
        if host not in mapping:
            raise SsrfBlocked(f"unexpected host {host!r} in test resolver")
        return ResolvedAddress(ip=mapping[host], port=port, family=2)

    return _fn


def test_check_all_advertised_brokers_allows_every_broker_when_all_public():
    metadata = BrokerMetadata(brokers=[("broker-a.invalid", 9092), ("broker-b.invalid", 9092)])
    resolved = check_all_advertised_brokers(
        metadata,
        resolve_checked=_resolver(
            {"broker-a.invalid": "93.184.216.34", "broker-b.invalid": "93.184.216.50"}
        ),
    )
    assert len(resolved) == 2


def test_check_all_advertised_brokers_refuses_a_metadata_response_advertising_an_internal_address():
    # The exact trap: bootstrap already passed resolve_checked (it named
    # a public host), but the broker's OWN metadata response advertises
    # an internal address for the topic's actual leader -- this must be
    # caught here, not assumed safe because bootstrap was public.
    metadata = BrokerMetadata(brokers=[("broker-a.invalid", 9092), ("internal-leader.invalid", 9092)])

    def _resolve(host, port):
        if host == "broker-a.invalid":
            return ResolvedAddress(ip="93.184.216.34", port=port, family=2)
        raise SsrfBlocked(f"refusing to dial {host!r}: internal address")

    with pytest.raises(SsrfBlocked):
        check_all_advertised_brokers(metadata, resolve_checked=_resolve)


def test_check_all_advertised_brokers_refuses_before_any_fetch_is_attempted():
    calls = []

    def _resolve(host, port):
        calls.append(host)
        if host == "internal-leader.invalid":
            raise SsrfBlocked("refused")
        return ResolvedAddress(ip="93.184.216.34", port=port, family=2)

    metadata = BrokerMetadata(brokers=[("broker-a.invalid", 9092), ("internal-leader.invalid", 9092)])
    with pytest.raises(SsrfBlocked):
        check_all_advertised_brokers(metadata, resolve_checked=_resolve)
    # Both were checked (order-independent set membership), neither was
    # ever handed to a fetch call -- this function has no fetch call at
    # all, by construction: adapters/kafka.py never calls the consumer's
    # .poll() until this function has returned successfully for every
    # broker the LATEST metadata names.
    assert set(calls) == {"broker-a.invalid", "internal-leader.invalid"}


def test_check_all_advertised_brokers_refuses_an_advertised_host_shaped_for_injection():
    # WS3 Task A3/F2 (Z13)'s _validate_hostname rule, reused here: an
    # advertised address is server-supplied text and must be
    # hostname-shaped before it is even resolved.
    metadata = BrokerMetadata(brokers=[("broker;evil=1", 9092)])
    with pytest.raises(ValueError):
        check_all_advertised_brokers(
            metadata, resolve_checked=lambda h, p: ResolvedAddress(ip="93.184.216.34", port=p, family=2)
        )
