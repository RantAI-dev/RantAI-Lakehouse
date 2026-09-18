"""Tests for `ssrf_guard_mongo.validate_mongo_dial`/`resolve_all_seed_hosts`
(WS9 plan Task C2). Covers the two MongoDB host-selection mechanisms
hard requirement 1 refuses outright: an SRV-shaped host string, and
`directConnection=False` (replica-set member discovery).

No network: `resolve_checked` is faked per-test rather than resolving
anything for real (AGENTS.md Python rule: no network in unit tests).
"""

from __future__ import annotations

import pytest

from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked
from dispar_orchestrate.ssrf_guard_mongo import (
    MongoConfigRejected,
    resolve_all_seed_hosts,
    validate_mongo_dial,
)


def test_validate_mongo_dial_refuses_a_srv_style_host():
    # There is no `srvUri` field on MongoDial (ingest_spec.rs, Task A2)
    # at the SCHEMA level -- this test proves the ADAPTER-level check
    # also refuses a host string that is itself SRV-shaped (defence in
    # depth against a caller who bypasses schema validation, e.g. a
    # future direct-DB edit).
    with pytest.raises(MongoConfigRejected):
        validate_mongo_dial({"hosts": ["cluster0.abcde.mongodb.net"], "directConnection": True})


def test_validate_mongo_dial_requires_direct_connection_true():
    # Hard requirement 1: replica-set discovery is REFUSED, not
    # partially checked -- directConnection=False would let pymongo
    # dial members the operator never listed. Refusing this at
    # validate time (before any network call) is stricter and cheaper
    # than trying to check every discovered host live.
    with pytest.raises(MongoConfigRejected):
        validate_mongo_dial({"hosts": ["mongo-a.invalid:27017"], "directConnection": False})


def test_validate_mongo_dial_accepts_an_explicit_direct_host_list():
    validate_mongo_dial({"hosts": ["mongo-a.invalid:27017", "mongo-b.invalid:27017"], "directConnection": True})


def test_resolve_all_seed_hosts_checks_every_explicit_host_before_any_connection():
    calls = []

    def _resolve(host, port):
        calls.append((host, port))
        return ResolvedAddress(ip="93.184.216.34", port=port, family=2)

    resolved = resolve_all_seed_hosts(["mongo-a.invalid:27017", "mongo-b.invalid:27018"], resolve_checked=_resolve)
    assert calls == [("mongo-a.invalid", 27017), ("mongo-b.invalid", 27018)]
    assert len(resolved) == 2


def test_resolve_all_seed_hosts_refuses_if_any_explicit_host_is_internal():
    def _resolve(host, port):
        if host == "internal.invalid":
            raise SsrfBlocked("refused")
        return ResolvedAddress(ip="93.184.216.34", port=port, family=2)

    with pytest.raises(SsrfBlocked):
        resolve_all_seed_hosts(["mongo-a.invalid:27017", "internal.invalid:27017"], resolve_checked=_resolve)


def test_resolve_all_seed_hosts_defaults_to_port_27017_when_no_port_is_given():
    # dial.hosts entries need not carry an explicit port -- the driver's
    # own default (27017) applies, and this guard must check the SAME
    # port the adapter will actually dial, not an unrelated default.
    calls = []

    def _resolve(host, port):
        calls.append((host, port))
        return ResolvedAddress(ip="93.184.216.34", port=port, family=2)

    resolve_all_seed_hosts(["mongo-a.invalid"], resolve_checked=_resolve)
    assert calls == [("mongo-a.invalid", 27017)]


def test_resolve_all_seed_hosts_refuses_a_seed_host_shaped_for_injection():
    # WS3 Task A3/F2 (Z13)'s _validate_hostname rule, reused here: an
    # explicit seed host re-read at dial time (rather than trusted from
    # a prior Dial::parse save-time check) must still be hostname-shaped
    # before it is even resolved.
    with pytest.raises(ValueError):
        resolve_all_seed_hosts(["mongo-a.invalid;evil=1:27017"])
