"""Tests for the dial-time SSRF guard (WS3 plan judge review Z1).

Covers every address class `is_blocked_ip` blocks (mirroring
`rust/crates/lakehouse-api/src/connector_probe.rs`'s own
`is_blocked_ip_covers_every_documented_range` test) plus multicast, the
deliberate widening documented in `ssrf_guard.py`'s module docstring.

No network: `getaddrinfo` is injected via `_fake_getaddrinfo` rather than
resolving anything for real (AGENTS.md Python rule: no network in unit
tests).
"""

from __future__ import annotations

import socket

import pytest

from dispar_orchestrate.ssrf_guard import (
    ResolvedAddress,
    SsrfBlocked,
    pinned_resolution,
    resolve_checked,
)


def _fake_getaddrinfo(ip: str):
    family = socket.AF_INET6 if ":" in ip else socket.AF_INET

    def _fn(host, port, *a, **k):
        return [(family, socket.SOCK_STREAM, socket.IPPROTO_TCP, "", (ip, port))]

    return _fn


def test_resolve_checked_allows_a_public_address():
    resolved = resolve_checked(
        "example.invalid", 443, getaddrinfo=_fake_getaddrinfo("93.184.216.34")
    )
    assert resolved.ip == "93.184.216.34"


@pytest.mark.parametrize(
    "blocked_ip",
    [
        "127.0.0.1",  # IPv4 loopback
        "10.0.0.1",  # RFC1918
        "172.16.5.5",  # RFC1918
        "192.168.1.1",  # RFC1918
        "169.254.169.254",  # link-local / cloud metadata
        "0.0.0.0",  # unspecified
        "224.0.0.1",  # IPv4 multicast -- deliberate widening beyond the Rust guard
        "::1",  # IPv6 loopback
        "fc00::1",  # IPv6 unique-local
        "fe80::1",  # IPv6 link-local
        "::ffff:10.0.0.1",  # IPv4-mapped IPv6, private
        "ff02::1",  # IPv6 multicast -- deliberate widening beyond the Rust guard
    ],
)
def test_resolve_checked_refuses_every_blocked_class_before_any_connection(blocked_ip):
    with pytest.raises(SsrfBlocked):
        resolve_checked("internal.invalid", 5432, getaddrinfo=_fake_getaddrinfo(blocked_ip))


def test_resolve_checked_opt_in_admits_an_internal_address():
    resolved = resolve_checked(
        "internal.invalid",
        5432,
        allow_internal_hosts=True,
        getaddrinfo=_fake_getaddrinfo("10.0.0.5"),
    )
    assert resolved.ip == "10.0.0.5"


def test_resolve_checked_reads_the_opt_in_env_var_when_not_passed_explicitly(monkeypatch):
    monkeypatch.setenv("INGEST_ALLOW_INTERNAL_HOSTS", "true")
    resolved = resolve_checked(
        "internal.invalid", 5432, getaddrinfo=_fake_getaddrinfo("10.0.0.5")
    )
    assert resolved.ip == "10.0.0.5"


def test_resolve_checked_checks_every_resolved_address_not_just_the_first():
    """A host that returns one public and one private address must be
    refused -- accepting it on the strength of the first record alone
    would let an attacker order the records so DNS answers "public first,
    private second" and slip the private one past a first-record-only
    check."""

    def _mixed(host, port, *a, **k):
        return [
            (socket.AF_INET, socket.SOCK_STREAM, socket.IPPROTO_TCP, "", ("93.184.216.34", port)),
            (socket.AF_INET, socket.SOCK_STREAM, socket.IPPROTO_TCP, "", ("10.0.0.9", port)),
        ]

    with pytest.raises(SsrfBlocked):
        resolve_checked("mixed.invalid", 443, getaddrinfo=_mixed)


def test_pinned_resolution_forces_getaddrinfo_to_the_checked_address_and_restores_after():
    original = socket.getaddrinfo
    resolved = ResolvedAddress(ip="93.184.216.34", port=443, family=socket.AF_INET)
    with pinned_resolution("example.invalid", resolved):
        infos = socket.getaddrinfo("example.invalid", 443)
        assert infos[0][4][0] == "93.184.216.34"
    assert socket.getaddrinfo is original


def test_pinned_resolution_leaves_other_hosts_unaffected():
    resolved = ResolvedAddress(ip="93.184.216.34", port=443, family=socket.AF_INET)
    with pinned_resolution("pinned.invalid", resolved):
        with pytest.raises(socket.gaierror):
            socket.getaddrinfo("definitely-not-a-real-host.invalid", 443)
