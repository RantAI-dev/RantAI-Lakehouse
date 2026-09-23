"""Tests for `ssrf_guard_sftp.PinnedHostKeyPolicy`.

Covers the exact refusal this module exists for: a `paramiko.MissingHostKeyPolicy`
that verifies against an operator-pinned fingerprint and never falls back to
`AutoAddPolicy`'s trust-on-first-use behaviour.

No network, no real SSH server: paramiko's key object is faked (a `.asbytes()`
stand-in) rather than connecting to anything (AGENTS.md Python rule: no
network in unit tests).
"""

from __future__ import annotations

import base64
import hashlib
import inspect

import pytest

from dispar_orchestrate.ssrf_guard_sftp import HostKeyMismatch, PinnedHostKeyPolicy


class _FakeKey:
    """Stands in for `paramiko.PKey` -- only `.asbytes()` is exercised by
    `PinnedHostKeyPolicy.missing_host_key`."""

    def __init__(self, blob: bytes):
        self._blob = blob

    def asbytes(self):  # matches paramiko's PKey.asbytes() signature
        return self._blob


def _fingerprint(blob: bytes) -> str:
    digest = hashlib.sha256(blob).digest()
    return "SHA256:" + base64.b64encode(digest).decode().rstrip("=")


def test_pinned_host_key_policy_accepts_a_matching_fingerprint():
    key = _FakeKey(b"the-real-host-key-bytes")
    policy = PinnedHostKeyPolicy(expected_fingerprint=_fingerprint(b"the-real-host-key-bytes"))
    policy.missing_host_key(client=None, hostname="sftp.invalid", key=key)  # must not raise


def test_pinned_host_key_policy_refuses_a_mismatched_fingerprint():
    key = _FakeKey(b"an-attacker-substituted-key")
    policy = PinnedHostKeyPolicy(expected_fingerprint=_fingerprint(b"the-real-host-key-bytes"))
    with pytest.raises(HostKeyMismatch):
        policy.missing_host_key(client=None, hostname="sftp.invalid", key=key)


def test_pinned_host_key_policy_refuses_when_no_fingerprint_was_pinned():
    # A blank/empty expected fingerprint must never be treated as "accept
    # anything" -- SftpDial.hostKeyFingerprint is a REQUIRED field
    # precisely so there is no code path that connects without one; this
    # guard must fail closed even if that invariant were ever violated
    # upstream, not warn-and-continue.
    key = _FakeKey(b"some-host-key-bytes")
    policy = PinnedHostKeyPolicy(expected_fingerprint="")
    with pytest.raises(HostKeyMismatch):
        policy.missing_host_key(client=None, hostname="sftp.invalid", key=key)


def test_pinned_host_key_policy_error_message_names_the_hostname_without_leaking_key_bytes():
    # Secrets never logged (AGENTS.md): the raised message must identify
    # which host failed, but must not embed the raw presented key bytes.
    key = _FakeKey(b"an-attacker-substituted-key")
    policy = PinnedHostKeyPolicy(expected_fingerprint=_fingerprint(b"the-real-host-key-bytes"))
    with pytest.raises(HostKeyMismatch) as excinfo:
        policy.missing_host_key(client=None, hostname="sftp.invalid", key=key)
    message = str(excinfo.value)
    assert "sftp.invalid" in message
    assert "an-attacker-substituted-key" not in message


def test_pinned_host_key_policy_is_not_an_auto_add_policy():
    # There is no code path in PinnedHostKeyPolicy that ever calls
    # `client.get_host_keys().add(...)` -- the exact behaviour
    # `paramiko.AutoAddPolicy` has and this class must never replicate.
    src = inspect.getsource(PinnedHostKeyPolicy)
    assert "get_host_keys().add" not in src
    assert "AutoAddPolicy" not in src


def test_pinned_host_key_policy_never_mutates_the_client_argument():
    # A real "reject, do not silently trust" policy has no reason to
    # touch `client` at all -- AutoAddPolicy's whole failure mode is
    # writing into the client's known-hosts state. Passing None as
    # `client` (as paramiko itself does when no client is wired) and
    # having the call succeed proves this policy never dereferences it.
    key = _FakeKey(b"the-real-host-key-bytes")
    policy = PinnedHostKeyPolicy(expected_fingerprint=_fingerprint(b"the-real-host-key-bytes"))
    policy.missing_host_key(client=None, hostname="sftp.invalid", key=key)
