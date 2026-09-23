"""Tests for `adapters.sftp.build_source`.

No network, no real SSH server: `paramiko.SSHClient` is faked at the
class level (`monkeypatch.setattr(".../paramiko.SSHClient", ...)`) so
every test runs with zero sockets opened (AGENTS.md Python rule: no
network in unit tests). Each test asserts one specific behaviour, never
just "did not crash".
"""

from __future__ import annotations

import inspect
import io

import paramiko
import pytest

from dispar_orchestrate.adapters.sftp import build_source
from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked
from dispar_orchestrate.ssrf_guard_sftp import PinnedHostKeyPolicy


def test_build_source_checks_the_host_before_connecting(monkeypatch):
    calls = []

    def _resolve(host, port):
        calls.append((host, port))
        raise SsrfBlocked("refused")

    monkeypatch.setattr(
        "dispar_orchestrate.adapters.sftp.paramiko.SSHClient",
        lambda: pytest.fail("must not connect"),
    )
    with pytest.raises(SsrfBlocked):
        build_source(
            {
                "host": "169.254.169.254",
                "port": 22,
                "path": "/export",
                "auth": {"type": "password"},
                "hostKeyFingerprint": "SHA256:abc",
            },
            secrets={"password": "x"},
            source_objects=[{"name": "orders.csv", "target": "orders"}],
            resolve_checked=_resolve,
        )
    assert calls == [("169.254.169.254", 22)]


def test_build_source_uses_the_pinned_host_key_policy(monkeypatch):
    captured = {}

    class _FakeSftp:
        def open(self, path, mode="r"):
            return io.BytesIO(b"id,name\n1,a\n")

    class _FakeClient:
        def set_missing_host_key_policy(self, policy):
            captured["policy"] = policy

        def connect(self, **kwargs):
            captured["connect_kwargs"] = kwargs

        def open_sftp(self):
            return _FakeSftp()

    monkeypatch.setattr("dispar_orchestrate.adapters.sftp.paramiko.SSHClient", _FakeClient)
    result = build_source(
        {
            "host": "sftp.invalid",
            "port": 22,
            "path": "/export",
            "auth": {"type": "password"},
            "hostKeyFingerprint": "SHA256:abc",
            "fileFormat": "csv",
        },
        secrets={"password": "s3cret"},
        source_objects=[{"name": "orders.csv", "target": "orders"}],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
    )
    assert isinstance(captured["policy"], PinnedHostKeyPolicy)
    assert captured["policy"]._expected == "SHA256:abc"  # pinned to THIS spec's fingerprint, not a default
    assert list(result.sources["orders"]) == [{"id": "1", "name": "a"}]


def test_build_source_pins_the_connect_call_to_the_resolved_address(monkeypatch):
    """`paramiko.SSHClient.connect` resolves `hostname` itself, via
    `socket.getaddrinfo`, when it opens the TCP socket (verified against
    the installed paramiko source -- see this module's docstring). This
    proves `build_source` actually forces THAT resolution to the checked
    address (via `ssrf_guard.pinned_resolution`), not merely that a
    name-shaped pre-check ran earlier and was then ignored by the real
    connect call."""
    captured = {}

    class _FakeSftp:
        def open(self, path, mode="r"):
            return io.BytesIO(b"id\n1\n")

    class _FakeClient:
        def set_missing_host_key_policy(self, policy):
            pass

        def connect(self, **kwargs):
            import socket

            # Mirrors what paramiko's own SSHClient.connect does internally:
            # resolve `hostname` via socket.getaddrinfo right here, at
            # connect time -- not the resolve_checked pre-check's result.
            captured["addrinfo"] = socket.getaddrinfo(kwargs["hostname"], kwargs["port"], 0, socket.SOCK_STREAM)

        def open_sftp(self):
            return _FakeSftp()

    monkeypatch.setattr("dispar_orchestrate.adapters.sftp.paramiko.SSHClient", _FakeClient)
    build_source(
        {
            "host": "sftp.invalid",
            "port": 22,
            "path": "/export",
            "auth": {"type": "password"},
            "hostKeyFingerprint": "SHA256:abc",
        },
        secrets={"password": "s3cret"},
        source_objects=[],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
    )
    # The address paramiko's own internal resolution returned during
    # connect() is the CHECKED address, not a second, independent lookup
    # of "sftp.invalid" that a DNS rebind could have pointed elsewhere.
    assert captured["addrinfo"][0][4][0] == "10.0.0.9"


def test_build_source_refuses_an_unrecognised_auth_type(monkeypatch):
    monkeypatch.setattr(
        "dispar_orchestrate.adapters.sftp.paramiko.SSHClient",
        lambda: pytest.fail("must not connect for a rejected auth type"),
    )
    with pytest.raises(ValueError, match="auth type"):
        build_source(
            {
                "host": "sftp.invalid",
                "port": 22,
                "path": "/export",
                "auth": {"type": "kerberos"},
                "hostKeyFingerprint": "SHA256:abc",
            },
            secrets={},
            source_objects=[],
            resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
        )


def test_build_source_refuses_an_unsupported_file_format(monkeypatch):
    monkeypatch.setattr(
        "dispar_orchestrate.adapters.sftp.paramiko.SSHClient",
        lambda: pytest.fail("must not connect for an unsupported file format"),
    )
    with pytest.raises(ValueError, match="fileFormat"):
        build_source(
            {
                "host": "sftp.invalid",
                "port": 22,
                "path": "/export",
                "auth": {"type": "password"},
                "hostKeyFingerprint": "SHA256:abc",
                "fileFormat": "parquet",
            },
            secrets={"password": "x"},
            source_objects=[],
            resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
        )


def test_build_source_uses_the_private_key_secret_for_public_key_auth(monkeypatch):
    key = paramiko.RSAKey.generate(1024)
    buf = io.StringIO()
    key.write_private_key(buf)
    private_key_pem = buf.getvalue()

    captured = {}

    class _FakeSftp:
        def open(self, path, mode="r"):
            return io.BytesIO(b"id\n1\n")

    class _FakeClient:
        def set_missing_host_key_policy(self, policy):
            pass

        def connect(self, **kwargs):
            captured["connect_kwargs"] = kwargs

        def open_sftp(self):
            return _FakeSftp()

    monkeypatch.setattr("dispar_orchestrate.adapters.sftp.paramiko.SSHClient", _FakeClient)
    build_source(
        {
            "host": "sftp.invalid",
            "port": 22,
            "path": "/export",
            "auth": {"type": "public_key"},
            "hostKeyFingerprint": "SHA256:abc",
        },
        secrets={"privateKey": private_key_pem},
        source_objects=[{"name": "orders.csv", "target": "orders"}],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
    )
    assert isinstance(captured["connect_kwargs"]["pkey"], paramiko.RSAKey)
    assert "password" not in captured["connect_kwargs"]


def test_build_source_never_puts_the_password_secret_into_a_raised_error_message(monkeypatch):
    # A bad fileFormat is refused AFTER secrets is available to
    # build_source -- prove the error text names the format, not the
    # password that was passed alongside it.
    monkeypatch.setattr(
        "dispar_orchestrate.adapters.sftp.paramiko.SSHClient",
        lambda: pytest.fail("must not connect"),
    )
    with pytest.raises(ValueError) as excinfo:
        build_source(
            {
                "host": "sftp.invalid",
                "port": 22,
                "path": "/export",
                "auth": {"type": "password"},
                "hostKeyFingerprint": "SHA256:abc",
                "fileFormat": "xml",
            },
            secrets={"password": "s3cret-value"},
            source_objects=[],
            resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.9", port=p, family=2),
        )
    assert "s3cret-value" not in str(excinfo.value)


def test_build_source_never_loads_system_or_local_host_keys():
    # PinnedHostKeyPolicy only runs when paramiko has NO cached entry for
    # the host (ssrf_guard_sftp.py's module doc): a stale
    # ~/.ssh/known_hosts entry would authenticate the connection itself,
    # silently, before the pinned policy is ever consulted. build_source
    # must never seed that cache.
    from dispar_orchestrate.adapters import sftp

    src = inspect.getsource(sftp.build_source)
    assert "load_system_host_keys" not in src
    assert "load_host_keys" not in src
