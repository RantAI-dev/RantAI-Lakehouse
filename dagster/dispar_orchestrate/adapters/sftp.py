"""dagster/dispar_orchestrate/adapters/sftp.py -- hand-written over
`paramiko` (no dlt filesystem source speaks SFTP; WS3's `files.py` is
s3fs/S3-only -- WS9 plan Correction 5). Host-key verification uses
`ssrf_guard_sftp.PinnedHostKeyPolicy` -- never `paramiko.AutoAddPolicy`.
Auth is `password` or `public_key` (secret_map.py's ("sftp","*") rows,
Task C4); a passphrase-protected private key is out of scope this
workstream (Open Questions).

SSRF (WS9 plan hard requirement 1): `resolve_checked` validates
`dial.host` before anything connects, but that alone is only a
name-shaped pre-check -- it proves nothing about the address the
driver actually dials, because `paramiko.SSHClient.connect` resolves
`hostname` itself, independently, when the TCP socket is opened
(`SSHClient._families_and_addresses` calls `socket.getaddrinfo`
directly -- verified against the installed `paramiko==3.5.0` source).
Checking a name and then handing that same name to a driver that
re-resolves it is exactly the check-then-connect gap `ssrf_guard`'s
module doc calls out (a DNS rebind between the check and the dial
would connect the checked name to a different, unchecked address).
`build_source` therefore wraps `client.connect(...)` in
`ssrf_guard.pinned_resolution(host, resolved)`, which monkeypatches
`socket.getaddrinfo` for the duration of that call so paramiko's own
resolution is forced to return the SAME address `resolve_checked`
already validated -- see `pinned_resolution`'s own doc comment for why
this is safe under this build's one-connector-per-run concurrency
model. Unlike Kafka/MongoDB (Tasks C1/C2), SFTP makes exactly one
connection per run, so a single `pinned_resolution` scope around
`connect()` covers the whole guarded surface; there is no reconnect-to-
a-different-host case here for `checking_resolver`'s wider net to
close.

Host-key trust: `paramiko.SSHClient()` is constructed WITHOUT calling
`load_system_host_keys()`/`load_host_keys()`. Either call would seed
the client's host-key cache from `~/.ssh/known_hosts` (or the system
file) -- and `PinnedHostKeyPolicy.missing_host_key` only ever runs when
the client has NO cached entry for the host (see
`ssrf_guard_sftp.py`'s module doc). A stale or attacker-influenced
`known_hosts` entry would let `SSHClient.connect` authenticate the
host key itself, silently, before the pinned policy is ever consulted
-- so this module must never load either file, and does not.
"""
from __future__ import annotations

import csv
import io
from dataclasses import dataclass
from typing import Any, Iterator

import paramiko

from dispar_orchestrate import ssrf_guard
from dispar_orchestrate.adapters.sql import _validate_hostname
from dispar_orchestrate.ssrf_guard_sftp import PinnedHostKeyPolicy

_SUPPORTED_FILE_FORMATS = ("csv",)


@dataclass(frozen=True)
class AdapterBuildResult:
    sources: dict[str, Iterator[dict]]
    resolved: ssrf_guard.ResolvedAddress


def _rows_from_csv(raw: bytes) -> Iterator[dict]:
    yield from csv.DictReader(io.StringIO(raw.decode("utf-8")))


def _connect_kwargs(spec: dict, secrets: dict[str, str]) -> dict[str, Any]:
    """Build paramiko's `connect()` keyword arguments for `spec['auth']['type']`.

    Refuses an unrecognised auth type outright (fail closed, AGENTS.md
    principle 3) rather than falling through to the `public_key` branch
    for anything that is not literally `"password"` -- a typo'd or
    unexpected auth type must not silently be treated as a private-key
    login. Neither branch ever puts a secret into an f-string that could
    end up in a log line or an exception message; both hand the raw
    secret value to paramiko as a plain keyword argument.
    """
    auth_type = spec["auth"]["type"]
    kwargs: dict[str, Any] = {
        "hostname": spec["host"],
        "port": spec["port"],
        "username": spec.get("user", ""),
    }
    if auth_type == "password":
        kwargs["password"] = secrets["password"]
    elif auth_type == "public_key":
        kwargs["pkey"] = paramiko.RSAKey.from_private_key(io.StringIO(secrets["privateKey"]))
    else:
        raise ValueError(f"sftp adapter does not support auth type {auth_type!r} (expected 'password' or 'public_key')")
    return kwargs


def build_source(
    spec: dict,
    secrets: dict[str, str],
    source_objects: list[dict],
    *,
    resolve_checked=ssrf_guard.resolve_checked,
) -> AdapterBuildResult:
    """Build one dlt-shaped source per `source_objects` entry over an
    SFTP directory.

    `_validate_hostname` and `resolve_checked` both run BEFORE any
    connection attempt (WS3 Task F2/A3 (Z13)'s canonical hostname-shape
    check, imported rather than re-implemented -- see kafka.py/mongodb.py
    for the same pattern) -- a rejected or blocked dial never constructs
    a `paramiko.SSHClient` at all (see this module's
    `test_build_source_checks_the_host_before_connecting`). The actual
    connect call is pinned to that checked address -- see the module
    docstring's SSRF section for why a name-only check is not enough for
    a driver that resolves its own hostname.
    """
    _validate_hostname("host", spec["host"])
    resolved = resolve_checked(spec["host"], spec["port"])

    file_format = spec.get("fileFormat", "csv")
    if file_format not in _SUPPORTED_FILE_FORMATS:
        raise ValueError(f"sftp adapter does not support fileFormat {file_format!r} (supported: {_SUPPORTED_FILE_FORMATS})")

    connect_kwargs = _connect_kwargs(spec, secrets)

    client: Any = paramiko.SSHClient()
    client.set_missing_host_key_policy(PinnedHostKeyPolicy(spec["hostKeyFingerprint"]))
    # Pin the connect call to the ALREADY-CHECKED address -- see the
    # module docstring's SSRF section. `host` itself is left unchanged
    # (paramiko still connects "to `spec['host']`" as far as its own
    # logging/host-key bookkeeping is concerned), only the address
    # `socket.getaddrinfo` hands back for that name is forced.
    with ssrf_guard.pinned_resolution(spec["host"], resolved):
        client.connect(**connect_kwargs)
    sftp = client.open_sftp()

    sources: dict[str, Iterator[dict]] = {}
    base_path = spec["path"].rstrip("/")
    for obj in source_objects:
        raw = sftp.open(f"{base_path}/{obj['name']}").read()
        sources[obj["target"]] = _rows_from_csv(raw)
    return AdapterBuildResult(sources=sources, resolved=resolved)
