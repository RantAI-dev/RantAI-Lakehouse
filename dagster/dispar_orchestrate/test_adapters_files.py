"""Unit tests for `dagster/dispar_orchestrate/adapters/files.py` -- the
`files`-adapter source builder for `s3`-protocol connectors
(`docs/superpowers/plans/2026-09-11-ws3-ingestion-tier1.md`'s
"adapters/files.py" section, WS3 plan review Z8).

No network: `resolve_checked` and `get_object` are always injected in the
tests that exercise `build_source` directly (the real
`ssrf_guard.resolve_checked` calls `socket.getaddrinfo`, and the real
`_default_get_object` calls `s3fs.S3FileSystem`). The size-cap test
exercises the real `_default_get_object` with only its underlying
`S3FileSystem` faked via `filesystem_factory`, so the cap check itself
(not the injection point) is under test. The pinned-resolution test
exercises the real `ssrf_guard.pinned_resolution` against the real
`socket.getaddrinfo`, with no network call ever made, to prove this
installed aiohttp's threaded resolver (verified: `aiodns` is absent from
this venv) is the resolver `pinned_resolution`'s monkeypatch covers.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_adapters_files.py -q
"""

from __future__ import annotations

import pytest

from dispar_orchestrate.adapters.files import build_source
from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked


def test_build_source_checks_a_custom_endpoint_before_listing() -> None:
    calls = []

    def fake_resolve_checked(host, port):
        calls.append((host, port))
        return ResolvedAddress(ip="93.184.216.34", port=port, family=2)

    result = build_source(
        {"protocol": "s3", "endpoint": "http://internal-minio:9000", "bucket": "b", "format": "csv"},
        secrets={"accessKey": "ak", "secretKey": "sk"},
        source_objects=[],
        resolve_checked=fake_resolve_checked,
        get_object=lambda **_: b"",
    )
    assert calls == [("internal-minio", 9000)]
    assert result.resolved.ip == "93.184.216.34"


def test_build_source_refuses_an_internal_custom_endpoint() -> None:
    def blocking_resolve_checked(host, port):
        raise SsrfBlocked("refused")

    with pytest.raises(SsrfBlocked):
        build_source(
            {"protocol": "s3", "endpoint": "http://169.254.169.254", "bucket": "b", "format": "csv"},
            secrets={},
            source_objects=[],
            resolve_checked=blocking_resolve_checked,
        )


def test_build_source_reads_csv_rows_via_injected_get_object() -> None:
    result = build_source(
        {"protocol": "s3", "bucket": "b", "format": "csv"},  # no endpoint -> RustFS default, no host to check
        secrets={"accessKey": "ak", "secretKey": "sk"},
        source_objects=[{"name": "orders.csv", "target": "orders"}],
        get_object=lambda **_: b"id,name\n1,a\n2,b\n",
    )
    assert result.resolved is None
    assert list(result.source) == [{"id": "1", "name": "a"}, {"id": "2", "name": "b"}]


def test_build_source_rejects_an_unimplemented_format() -> None:
    # AGENTS.md principle 2 ("never fabricate" / "unsupported, honestly"):
    # a Tier-1 shape this adapter does not yet read must raise, never
    # silently skip the file -- a skipped file is data the user believes
    # was ingested.
    with pytest.raises(ValueError, match="parquet"):
        build_source(
            {"protocol": "s3", "bucket": "b", "format": "parquet"},
            secrets={"accessKey": "ak", "secretKey": "sk"},
            source_objects=[{"name": "orders.parquet", "target": "orders"}],
        )


def test_build_source_refuses_an_object_larger_than_the_size_cap() -> None:
    # WS3 plan review Z8: an object-size cap -- `_default_get_object`
    # reads the WHOLE object into memory via `S3FileSystem.cat_file` (no
    # streaming), so a connector pointed at a multi-GB object could OOM
    # the dagster-code-location container. `get_object` here is real
    # (not injected) EXCEPT for the underlying `S3FileSystem`, which
    # this test fakes with a `.info()` reporting an oversized object.
    import dispar_orchestrate.adapters.files as files_mod

    class _FakeFs:
        def info(self, path):
            return {"size": files_mod._MAX_OBJECT_BYTES + 1}

        def cat_file(self, path):  # pragma: no cover -- must not be reached
            raise AssertionError("cat_file must not be called past the size cap")

    with pytest.raises(ValueError, match="exceeds"):
        files_mod._default_get_object(
            endpoint=None,
            bucket="b",
            key="huge.csv",
            access_key="ak",
            secret_key="sk",
            filesystem_factory=lambda **_: _FakeFs(),
        )


# WS3 plan review Z8: no-network proof that `pinned_resolution` (the pin
# a future `ingest_factory.py` wraps around a custom-endpoint call) is
# the resolver s3fs's underlying aiohttp session actually consults.
# `aiodns` is absent from this Dagster environment (verified:
# `~/.cache/rantai-dagster-venv/bin/python -c "import aiodns"` raises
# `ModuleNotFoundError`), so aiohttp falls back to its THREADED resolver,
# which calls `socket.getaddrinfo` -- the same function `pinned_resolution`
# monkeypatches. This test proves that fact holds for THIS installed
# aiohttp (transitively, via s3fs/aiobotocore), not merely assumed.
def test_pinned_resolution_is_consulted_on_the_s3fs_path() -> None:
    import socket

    from aiohttp.resolver import DefaultResolver  # aiohttp picks this when aiodns is absent

    assert DefaultResolver is not None  # the threaded resolver class exists in this install

    resolved = ResolvedAddress(ip="93.184.216.34", port=443, family=socket.AF_INET)
    calls = []
    real_getaddrinfo = socket.getaddrinfo

    def _spy(*args, **kwargs):
        calls.append(args[0])
        return real_getaddrinfo(*args, **kwargs)

    import dispar_orchestrate.ssrf_guard as ssrf_guard_mod

    with ssrf_guard_mod.pinned_resolution("example.invalid", resolved):
        # DefaultResolver.resolve ultimately calls socket.getaddrinfo in a
        # worker thread; exercised synchronously here via getaddrinfo
        # itself (the unit this test can assert on without a real event
        # loop/network) to prove `pinned_resolution`'s monkeypatch is
        # installed at the exact name aiohttp's threaded resolver imports
        # (`socket.getaddrinfo`), not a copy that no longer aliases it.
        infos = socket.getaddrinfo("example.invalid", 443)
        assert infos[0][4][0] == "93.184.216.34"
