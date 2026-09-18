"""dagster/dispar_orchestrate/ssrf_guard_sftp.py -- WS9 plan hard
requirement 1: paramiko's `SSHClient.connect()` needs a
`MissingHostKeyPolicy` to decide what happens when it has no cached
entry for a host. `paramiko.AutoAddPolicy` accepts and silently trusts
WHATEVER key the server presents on first connect -- exactly the
"TOFU with no verification" failure mode this build's own connector_probe.rs
module comment calls out for other protocols (never trust
response-supplied data). `PinnedHostKeyPolicy` instead requires the
operator to have captured the real host's key fingerprint out of band
(e.g. `ssh-keyscan sftp.example.com | ssh-keygen -lf - -E sha256`) and
stored it in `dial.hostKeyFingerprint` (SftpDial, Task A2, a REQUIRED
field -- there is no code path that connects without one). Every
connection attempt computes the presented key's SHA256 fingerprint and
refuses outright on any mismatch -- never falls back to trusting it."""
from __future__ import annotations

import base64
import hashlib

import paramiko


class HostKeyMismatch(Exception):
    """The SFTP server's presented host key does not match
    `dial.hostKeyFingerprint`. Raised, never silently accepted."""


def _sha256_fingerprint(key_bytes: bytes) -> str:
    digest = hashlib.sha256(key_bytes).digest()
    return "SHA256:" + base64.b64encode(digest).decode().rstrip("=")


class PinnedHostKeyPolicy(paramiko.MissingHostKeyPolicy):
    """Refuses every host key except the one exact fingerprint pinned at
    connector-registration time. Never adds a key to the client's known
    hosts -- there is no persistent known_hosts state at all; every
    connection re-validates against `dial.hostKeyFingerprint` fresh. See
    the module docstring for why the trust-on-first-use policy this
    class replaces is not acceptable here."""

    def __init__(self, expected_fingerprint: str) -> None:
        self._expected = expected_fingerprint

    def missing_host_key(self, client, hostname, key) -> None:
        actual = _sha256_fingerprint(key.asbytes())
        if actual != self._expected:
            raise HostKeyMismatch(
                f"host key for {hostname!r} does not match the pinned fingerprint "
                "(expected vs. presented fingerprints differ) -- refusing to connect"
            )
