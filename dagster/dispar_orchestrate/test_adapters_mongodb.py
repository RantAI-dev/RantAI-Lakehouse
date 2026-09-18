"""dagster/dispar_orchestrate/test_adapters_mongodb.py -- no network:
every test fakes `MongoClient` (monkeypatched on the adapter module, not
the real driver) and injects `resolve_checked` -- see
`adapters/mongodb.py`'s module doc comment and `ssrf_guard_mongo.py`'s
for the two checks (SRV-shape/directConnection, then every seed host)
each test asserts run BEFORE any connection attempt (WS9 plan Task D1).
"""
from __future__ import annotations

import pytest

from dispar_orchestrate.adapters.mongodb import build_source
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
