"""dagster/dispar_orchestrate/test_adapters_mongodb.py -- no network:
every test fakes `MongoClient` (monkeypatched on the adapter module, not
the real driver) and injects `resolve_checked` -- see
`adapters/mongodb.py`'s module doc comment and `ssrf_guard_mongo.py`'s
for the two checks (SRV-shape/directConnection, then every seed host)
each test asserts run BEFORE any connection attempt.
"""
from __future__ import annotations

import datetime
import decimal

import pytest
from bson import Binary, Decimal128, ObjectId

from dispar_orchestrate.adapters.mongodb import _bson_safe, build_source
from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked
from dispar_orchestrate.ssrf_guard_mongo import MongoConfigRejected


def test_build_source_validates_before_resolving_anything(monkeypatch):
    monkeypatch.setattr("dispar_orchestrate.adapters.mongodb.MongoClient", lambda **_: pytest.fail("must not connect"))
    with pytest.raises(MongoConfigRejected):
        build_source(
            {"hosts": ["mongo-a.invalid:27017"], "database": "shop", "username": "reader", "directConnection": False},
            secrets={"password": "s3cret"}, source_objects=[],
        )


def test_build_source_checks_every_seed_host_before_connecting(monkeypatch):
    calls = []

    def _resolve(host, port):
        calls.append((host, port))
        return ResolvedAddress(ip="10.0.0.5", port=port, family=2)

    monkeypatch.setattr("dispar_orchestrate.adapters.mongodb.MongoClient", lambda **kwargs: pytest.fail("checked separately"))
    with pytest.raises(SsrfBlocked):
        build_source(
            {"hosts": ["mongo-a.invalid:27017", "169.254.169.254:27017"], "database": "shop",
             "username": "reader", "directConnection": True},
            secrets={"password": "s3cret"}, source_objects=[{"name": "orders", "target": "orders"}],
            resolve_checked=lambda h, p: (_ for _ in ()).throw(SsrfBlocked("refused")) if h == "169.254.169.254" else _resolve(h, p),
        )


def test_build_source_builds_a_dlt_resource_over_a_mongo_collection(monkeypatch):
    docs = [{"_id": 1, "name": "a"}, {"_id": 2, "name": "b"}]

    class _FakeCollection:
        def find(self, *_a, **_k):
            return iter(docs)

    class _FakeDb:
        def __getitem__(self, name):
            assert name == "orders"
            return _FakeCollection()

    class _FakeClient:
        def __init__(self, **kwargs):
            self.captured = kwargs

        def __getitem__(self, name):
            assert name == "shop"
            return _FakeDb()

    monkeypatch.setattr("dispar_orchestrate.adapters.mongodb.MongoClient", _FakeClient)
    result = build_source(
        {"hosts": ["mongo-a.invalid:27017"], "database": "shop", "username": "reader", "directConnection": True},
        secrets={"password": "s3cret"}, source_objects=[{"name": "orders", "target": "orders"}],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.5", port=p, family=2),
    )
    rows = list(result.sources["orders"])
    assert rows == docs
    assert result.resolved[0].ip == "10.0.0.5"


def test_build_source_never_interpolates_the_username_or_password_into_a_uri_string(monkeypatch):
    captured = {}

    class _FakeClient:
        def __init__(self, **kwargs):
            captured.update(kwargs)

        def __getitem__(self, name):
            class _Db:
                def __getitem__(self, name):
                    class _Coll:
                        def find(self, *_a, **_k):
                            return iter([])
                    return _Coll()
            return _Db()

    monkeypatch.setattr("dispar_orchestrate.adapters.mongodb.MongoClient", _FakeClient)
    # A password containing every URI-metacharacter that would matter if
    # this were ever (wrongly) concatenated into a connection string.
    build_source(
        {"hosts": ["mongo-a.invalid:27017"], "database": "shop", "username": "reader", "directConnection": True},
        secrets={"password": "p@ss/word:with?special&chars"}, source_objects=[],
        resolve_checked=lambda h, p: ResolvedAddress(ip="10.0.0.5", port=p, family=2),
    )
    # Passed through UNCHANGED as a keyword argument -- proof there was
    # no string-building step for it to be mangled or escape out of.
    assert captured["password"] == "p@ss/word:with?special&chars"

def test_reading_documents_runs_inside_checking_resolver_so_a_rebound_host_is_refused():
    """The seed-host check happens before `MongoClient` exists; pymongo then
    resolves those names ITSELF, lazily, when the first document is pulled.
    An answer that is safe during the pre-check and internal during the read
    must still be refused -- that second lookup is the rebinding window, and
    only `checking_resolver`, installed for the whole iteration, closes it."""
    import functools
    import socket

    from dispar_orchestrate import ssrf_guard
    from dispar_orchestrate.adapters import mongodb

    def _internal(host, port, *args, **kwargs):
        return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("169.254.169.254", port))]

    class _ResolvingCollection:
        """Stands in for pymongo's own lazy connect: the driver resolves the
        host when the first document is pulled, not when the client is made."""

        def find(self, _query):
            socket.getaddrinfo("mongo.invalid", 27017)
            yield {"never": "reached"}

    rows = mongodb._collection_rows(
        _ResolvingCollection(),
        checking_resolver=functools.partial(ssrf_guard.checking_resolver, getaddrinfo=_internal),
    )
    with pytest.raises(SsrfBlocked, match="private/internal/multicast"):
        list(rows)


def test_reading_refuses_a_document_whose_first_row_carries_a_nested_value():
    """R7: Mongo has no declared column types, so the first document of the
    read is the sample the gate inspects. A nested list/dict must be refused
    at read time — silently flattening it in the sink would lose structure
    the catalog would then describe wrongly."""
    from dispar_orchestrate import column_gate
    from dispar_orchestrate.adapters import mongodb

    class _NestedCollection:
        def find(self, _query):
            yield {"id": 1, "tags": ["a", "b"]}

    rows = mongodb._collection_rows(_NestedCollection())
    with pytest.raises(column_gate.UnsupportedColumnType, match="tags"):
        list(rows)


# ── _bson_safe ──────────────────────────────────────────────────────────


def test_bson_safe_converts_an_objectid_to_its_hex_string():
    oid = ObjectId("64b7f9e2c2a4f1a2b3c4d5e6")
    assert _bson_safe(oid) == "64b7f9e2c2a4f1a2b3c4d5e6"
    assert isinstance(_bson_safe(oid), str)


def test_bson_safe_converts_a_decimal128_to_a_python_decimal_not_a_lossy_float():
    result = _bson_safe(Decimal128("19.99"))
    assert result == decimal.Decimal("19.99")
    assert isinstance(result, decimal.Decimal)


def test_bson_safe_strips_the_binary_subclass_down_to_plain_bytes():
    result = _bson_safe(Binary(b"\x00\x01\xff"))
    assert result == b"\x00\x01\xff"
    assert type(result) is bytes  # not the Binary subclass -- see this module's docstring


def test_bson_safe_passes_a_datetime_through_unchanged():
    now = datetime.datetime(2026, 9, 18, 12, 0, 0, tzinfo=datetime.timezone.utc)
    assert _bson_safe(now) is now


def test_bson_safe_passes_ordinary_scalars_through_unchanged():
    assert _bson_safe("plain") == "plain"
    assert _bson_safe(42) == 42
    assert _bson_safe(None) is None
    assert _bson_safe(True) is True


def test_bson_safe_recurses_into_a_nested_document_and_array():
    oid = ObjectId("64b7f9e2c2a4f1a2b3c4d5e6")
    doc = {
        "owner": {"ref": oid, "balance": Decimal128("5.00")},
        "tags": [oid, {"blob": Binary(b"x")}],
    }
    result = _bson_safe(doc)
    assert result["owner"]["ref"] == "64b7f9e2c2a4f1a2b3c4d5e6"
    assert result["owner"]["balance"] == decimal.Decimal("5.00")
    assert result["tags"][0] == "64b7f9e2c2a4f1a2b3c4d5e6"
    assert result["tags"][1]["blob"] == b"x"


def test_reading_documents_converts_the_default_objectid_id_to_a_string():
    """Every real collection's default `_id` is an `ObjectId` -- this is
    the case that broke ingestion of ANY real collection before
    `_bson_safe` was wired into `_collection_rows`."""
    from dispar_orchestrate.adapters import mongodb

    oid = ObjectId("64b7f9e2c2a4f1a2b3c4d5e6")

    class _RealIdCollection:
        def find(self, _query):
            yield {"_id": oid, "name": "widget"}

    rows = list(mongodb._collection_rows(_RealIdCollection()))
    assert rows == [{"_id": "64b7f9e2c2a4f1a2b3c4d5e6", "name": "widget"}]


def test_reading_accepts_a_flat_document():
    """The gate must not refuse the ordinary case — a flat document streams
    through untouched."""
    from dispar_orchestrate.adapters import mongodb

    class _FlatCollection:
        def find(self, _query):
            yield {"id": 1, "name": "widget"}
            yield {"id": 2, "name": "gadget"}

    assert [r["id"] for r in mongodb._collection_rows(_FlatCollection())] == [1, 2]

