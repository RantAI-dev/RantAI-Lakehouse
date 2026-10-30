"""Unit tests for `discover.py` -- table discovery for the types only the
orchestrator reads (`SRC-13` decisions D1, D4, D5, D6, D9, D10).

No network and no real driver connection: every driver is a fake injected through
`discover.Backends`; the SSRF check is a stub that returns an address; the POST to
the API is captured. What the tests pin is what the plan promises: bounded
answers, the closed wire shapes, a failure that is a class and never a driver's
text, and a Kafka preview that never commits.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_discover.py -q
"""

from __future__ import annotations

import json
import logging
import socket
import stat
from datetime import datetime
from decimal import Decimal
from types import SimpleNamespace

import pytest
from dagster import Failure, build_op_context

from dispar_orchestrate import discover, schema_observer, ssrf_guard
from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.adapters.kafka import BrokerListUnavailable  # noqa: F401 -- imported to prove the module loads
from dispar_orchestrate.discover import Backends, DiscoverRequest

SECRET_TEXT = "hunter2-the-password"


def _address(host: str = "203.0.113.7", port: int = 0) -> ssrf_guard.ResolvedAddress:
    return ssrf_guard.ResolvedAddress(ip=host, port=port, family=socket.AF_INET)


def _backends(**overrides) -> Backends:
    """Backends whose SSRF check approves any host and whose resolver guard does
    nothing (the real ones patch `socket.getaddrinfo`)."""
    import contextlib

    base = {
        "resolve_checked": lambda host, port: _address(port=port),
        "checking_resolver": contextlib.nullcontext,
    }
    return Backends(**{**base, **overrides})


def _request(kind: str = "list", **fields) -> DiscoverRequest:
    return DiscoverRequest(connector_id="conn-1", request_id="req-1", kind=kind, **fields)


def _driver_error(module: str, name: str = "OperationalError", **attrs):
    return type(name, (Exception,), {"__module__": module, **attrs})


@pytest.fixture(autouse=True)
def _no_secret_resolution(monkeypatch):
    """Secrets resolve through the allowlisted resolver; here every connector
    gets a fixed one, so no environment variable is read."""
    monkeypatch.setattr(
        discover.secret_resolver,
        "resolve_secrets",
        lambda adapter, auth_type, primary, secondary: {"password": SECRET_TEXT, "privateKey": SECRET_TEXT},
    )


# --- the answer's bounds and shapes ----------------------------------------------


def test_a_list_keeps_only_names_the_api_accepts_and_stops_at_the_limit() -> None:
    entries = [
        discover.listed("a", "topic"),
        discover.listed("   ", "topic"),
        discover.listed("line\nbreak", "topic"),
        discover.listed("x" * 257, "topic"),
        discover.listed("b", "topic"),
        discover.listed("c", "topic"),
    ]
    cut = discover.listing(entries, 2)
    assert [o["name"] for o in cut["objects"]] == ["a", "b"]
    assert cut["truncated"] is True

    whole = discover.listing(entries, 10)
    assert [o["name"] for o in whole["objects"]] == ["a", "b", "c"]
    assert whole["truncated"] is False


def test_a_list_never_holds_more_than_ten_thousand_names() -> None:
    entries = (discover.listed(f"t{i}", "table") for i in range(discover.MAX_NAMES + 5))
    result = discover.listing(entries, discover.MAX_NAMES + 100)
    assert len(result["objects"]) == discover.MAX_NAMES
    assert result["truncated"] is True


def test_a_list_is_cut_before_its_json_passes_the_result_cap() -> None:
    long_name = "n" * 250
    entries = (discover.listed(f"{long_name}{i:05d}", "table") for i in range(discover.MAX_NAMES))
    result = discover.listing(entries, discover.MAX_NAMES)
    body = json.dumps(result, separators=(",", ":"), ensure_ascii=False).encode()
    assert len(body) <= discover.MAX_RESULT_BYTES
    assert result["truncated"] is True
    assert 0 < len(result["objects"]) < discover.MAX_NAMES


def test_a_preview_drops_columns_the_api_would_refuse_and_a_key_that_names_a_dropped_one() -> None:
    columns = [
        discover.column("id", "integer", inferred=False),
        discover.column("", "string", inferred=True),
        discover.column("note", "string", inferred=True),
    ]
    result = discover.preview(columns, ["id", "gone"])
    assert [c["name"] for c in result["columns"]] == ["id", "note"]
    assert result["primaryKey"] == ["id"]
    assert "primaryKey" not in discover.preview(columns, ["gone"])
    assert discover.preview([]) == {"columns": []}


def test_the_post_body_is_exactly_the_documented_shape() -> None:
    request = _request("list")
    done = discover.result_body(request, result={"objects": [], "truncated": False}, error_class=None)
    assert done == {"requestId": "req-1", "kind": "list", "state": "done", "objects": [], "truncated": False}
    failed = discover.result_body(request, result=None, error_class="timeout")
    assert failed == {"requestId": "req-1", "kind": "list", "state": "failed", "errorClass": "timeout"}
    with pytest.raises(ValueError):
        discover.result_body(request, result=None, error_class="the password was wrong")


def test_the_closed_class_list_is_the_one_the_api_accepts() -> None:
    assert discover.ERROR_CLASSES == {
        "unreachable",
        "sign_in_refused",
        "address_refused",
        "no_permission",
        "timeout",
        "too_many_objects",
        "not_supported",
        "failed",
    }


# --- the request ---------------------------------------------------------------------


def test_a_request_is_read_from_the_run_config_and_its_limit_is_clamped() -> None:
    config = {"connector_id": "c", "request_id": "r", "kind": "preview", "object": "orders", "limit": 99_999}
    request = DiscoverRequest.from_op_config(config)
    assert (request.kind, request.object, request.limit) == ("preview", "orders", discover.MAX_NAMES)
    assert DiscoverRequest.from_op_config({**config, "limit": 0}).limit == 1
    with pytest.raises(ValueError):
        DiscoverRequest.from_op_config({**config, "kind": "tables"})


# --- globs and sampled types -----------------------------------------------------------


@pytest.mark.parametrize(
    ("pattern", "text", "expected"),
    [
        ("*.csv", "a.csv", True),
        ("*.csv", "a.txt", False),
        ("sales-??.csv", "sales-01.csv", True),
        ("sales-??.csv", "sales-1.csv", False),
        ("*", "dir/a.csv", False),
        ("**", "dir/a.csv", True),
        ("", "anything", True),
        ("a.b", "axb", False),
    ],
)
def test_a_glob_matches_by_the_rules_the_api_uses(pattern: str, text: str, expected: bool) -> None:
    assert discover.Glob(pattern).matches(text) is expected


def test_a_hostile_glob_finishes_at_once_and_a_bad_one_is_refused() -> None:
    assert discover.Glob("a*" * 60 + "b").matches("a" * 2000) is False
    with pytest.raises(ValueError):
        discover.Glob("x" * 257)
    with pytest.raises(ValueError):
        discover.Glob("a\nb")


def test_a_text_sample_is_typed_by_the_narrowest_type_all_values_fit() -> None:
    cases = [
        (["1", "22", ""], "integer"),
        (["1", "2.5"], "number"),
        (["true", "FALSE"], "boolean"),
        (["2026-10-15", "2026-01-02"], "date"),
        (["2026-10-15T09:00:00Z", "2026-10-15 09:00:00"], "timestamp"),
        (["1", "x"], "string"),
        (["", "  "], "string"),
        (["nan", "inf"], "string"),
    ]
    for values, expected in cases:
        assert discover.infer_text_type(values) == expected, values


def test_records_are_typed_by_the_union_of_their_keys() -> None:
    records = [
        {"_id": "1", "n": 1, "x": None, "flag": True, "when": datetime(2026, 1, 1), "price": Decimal("1.5")},
        {"_id": "2", "n": 2.5, "x": None, "nested": {"a": 1}, "tags": ["a"], "mixed": 1},
        {"mixed": "text"},
        "not an object",
    ]
    columns = {c["name"]: c for c in discover.columns_from_records(records)}
    assert list(columns)[:2] == ["_id", "n"], "first-seen order"
    assert columns["n"]["typeName"] == "number", "an integer and a float is a number"
    assert columns["x"]["typeName"] == "unknown", "only nulls says nothing"
    assert columns["flag"]["typeName"] == "boolean"
    assert columns["when"]["typeName"] == "timestamp"
    assert columns["price"]["typeName"] == "decimal"
    assert columns["nested"]["typeName"] == "nested"
    assert columns["tags"]["typeName"] == "nested"
    assert columns["mixed"]["typeName"] == "mixed"
    assert all(c["inferred"] is True for c in columns.values())


def test_a_csv_head_gives_a_typed_header_and_loses_a_partial_last_line() -> None:
    text = "id,name,,when\n1,ann,x,2026-01-02\n2,bob,y,2026-02-03\n3,par"
    columns = discover.csv_columns(text, head_is_partial=True)
    assert [(c["name"], c["typeName"]) for c in columns] == [
        ("id", "integer"),
        ("name", "string"),
        ("column_3", "string"),
        ("when", "date"),
    ]
    assert discover.csv_columns("", head_is_partial=False) == []


# --- classification --------------------------------------------------------------------


def test_a_failure_is_named_with_a_word_of_the_closed_list_by_type_and_code() -> None:
    cases = [
        (discover.NotSupported("x"), "not_supported"),
        (discover.TooManyObjects("x"), "too_many_objects"),
        (ssrf_guard.SsrfBlocked("refusing to dial 'db': it resolves to 10.0.0.5"), "address_refused"),
        (ssrf_guard.SsrfBlocked("could not resolve host 'db': [Errno -2]"), "unreachable"),
        (ConnectionRefusedError(), "unreachable"),
        (TimeoutError(), "timeout"),
        (_driver_error("pymongo", "OperationFailure", code=18)("auth"), "sign_in_refused"),
        (_driver_error("pymongo", "OperationFailure", code=13)("unauthorized"), "no_permission"),
        (_driver_error("pymongo", "ServerSelectionTimeoutError")("no servers"), "failed"),
        (_driver_error("kafka", "TopicAuthorizationFailedError")("acl"), "no_permission"),
        (_driver_error("kafka", "SaslAuthenticationFailedError")("sasl"), "sign_in_refused"),
        (_driver_error("kafka", "NoBrokersAvailable")(), "unreachable"),
        (PermissionError(13, "Permission denied"), "no_permission"),
        (_driver_error("paramiko", "AuthenticationException")("x"), "sign_in_refused"),
        (RuntimeError("nobody taught this module"), "failed"),
        (KeyError("x"), "failed"),
    ]
    for exc, expected in cases:
        assert discover.classify_discovery(exc) == expected, repr(exc)
    for word in {expected for _, expected in cases}:
        assert word in discover.ERROR_CLASSES


def test_an_oracle_privilege_error_wrapped_by_the_orm_is_no_permission_and_a_refused_login_is_not() -> None:
    import sqlalchemy.exc

    def oracle(code: str):
        arg = SimpleNamespace(full_code=code)
        return sqlalchemy.exc.DatabaseError("SELECT 1", {}, _driver_error("oracledb", "DatabaseError")(arg))

    assert discover.classify_discovery(oracle("ORA-01031")) == "no_permission"
    assert discover.classify_discovery(oracle("ORA-01017")) == "sign_in_refused"
    assert discover.classify_discovery(oracle("ORA-12541")) == "unreachable"


# --- MongoDB ---------------------------------------------------------------------------


class _Cursor:
    def __init__(self, documents):
        self._documents = documents

    def limit(self, n):
        return iter(self._documents[:n])


class _Collection:
    def __init__(self, documents):
        self._documents = documents

    def find(self, _filter):
        return _Cursor(self._documents)


class _Database:
    def __init__(self, collections):
        self._collections = collections

    def list_collection_names(self):
        return list(self._collections)

    def __getitem__(self, name):
        return _Collection(self._collections[name])


class _MongoClient:
    def __init__(self, collections):
        self.database = _Database(collections)
        self.closed = False

    def __getitem__(self, name):
        assert name == "shop"
        return self.database

    def close(self):
        self.closed = True


_MONGO = {
    "id": "conn-1",
    "adapter": "mongodb",
    "dial": {"hosts": ["m.example:27017"], "database": "shop", "username": "u", "directConnection": True},
    "secretRef": "env:CONNECTOR_MONGO_PASSWORD",
}


def test_mongodb_lists_collections_without_the_system_ones_and_closes_the_client() -> None:
    client = _MongoClient({"orders": [], "customers": [], "system.views": [], "Order_items": []})
    backends = _backends(mongo_connect=lambda dial, secrets, **kw: (client, []))
    result = discover.discover(_MONGO, _request("list", search="ORDER"), backends=backends)
    assert [o["name"] for o in result["objects"]] == ["Order_items", "orders"]
    assert all(o["kind"] == "collection" for o in result["objects"])
    assert client.closed is True


def test_mongodb_previews_a_sample_of_a_hundred_documents_and_closes_the_client() -> None:
    documents = [{"_id": str(i), "qty": i} for i in range(250)]
    client = _MongoClient({"orders": documents})
    seen = {}

    def connect(dial, secrets, **kw):
        seen["password"] = secrets["password"]
        return client, []

    result = discover.discover(
        _MONGO, _request("preview", object="orders"), backends=_backends(mongo_connect=connect)
    )
    assert [(c["name"], c["typeName"], c["inferred"]) for c in result["columns"]] == [
        ("_id", "string", True),
        ("qty", "integer", True),
    ]
    assert client.closed is True
    assert seen["password"] == SECRET_TEXT


def test_mongodb_hands_the_adapters_own_connection_code_the_checked_dial() -> None:
    calls = []

    def connect(dial, secrets, **kw):
        calls.append((dial["hosts"], kw["resolve_checked"]))
        return _MongoClient({}), []

    backends = _backends(mongo_connect=connect)
    discover.discover(_MONGO, _request("list"), backends=backends)
    assert calls == [(["m.example:27017"], backends.resolve_checked)]


# --- MongoDB change capture (SRC-4 slice 5, decision S5-f) ---------------------------------


_CDC_MONGO = {
    "id": "conn-1",
    "adapter": "cdc",
    "dial": {
        "driver": "mongodb",
        "hosts": ["m1.example:27017", "m2.example:27017"],
        "database": "shop",
        "user": "reader",
        "replicaSet": "rs0",
        "sslMode": "disable",
    },
    "secretRef": "env:CONNECTOR_MONGO_PASSWORD",
}


class _Admin:
    """`ping` answers; `hello` and `connectionStatus` answer what the test set (`answers`), or raise
    what it set in `raises`. `fail` makes every command raise (a refused login)."""

    def __init__(self, fail: Exception | None = None, answers: dict | None = None, raises: dict | None = None) -> None:
        self.commands: list[str] = []
        self.fail = fail
        self.answers = answers or {}
        self.raises = raises or {}

    def command(self, name: str, **kwargs):
        self.commands.append(name)
        if self.fail:
            raise self.fail
        if name in self.raises:
            raise self.raises[name]
        return self.answers.get(name, {"ok": 1})


class _PingClient(_MongoClient):
    def __init__(self, collections, fail: Exception | None = None, answers: dict | None = None, raises: dict | None = None):
        super().__init__(collections)
        self.admin = _Admin(fail, answers, raises)


def test_a_mongodb_cdc_connector_lists_its_databases_collections_through_the_batch_code() -> None:
    client = _MongoClient({"orders": [], "system.views": [], "items": []})
    seen = {}

    def connect(dial, secrets, **kw):
        seen.update(dial)
        seen["password"] = secrets["password"]
        return client, []

    result = discover.discover(_CDC_MONGO, _request("list"), backends=_backends(mongo_connect=connect))
    assert [o["name"] for o in result["objects"]] == ["items", "orders"]
    assert client.closed is True
    # The change-capture dial is handed over as the batch dial: one host, direct, the user as username.
    assert seen["hosts"] == ["m1.example:27017"]
    assert (seen["username"], seen["database"], seen["directConnection"]) == ("reader", "shop", True)
    assert seen["password"] == SECRET_TEXT


def test_a_mongodb_cdc_preview_names_id_as_the_key() -> None:
    client = _MongoClient({"orders": [{"_id": "a1", "qty": 3}]})
    result = discover.discover(
        _CDC_MONGO, _request("preview", object="orders"), backends=_backends(mongo_connect=lambda d, s, **kw: (client, []))
    )
    assert [c["name"] for c in result["columns"]] == ["_id", "qty"]
    assert result["primaryKey"] == ["_id"]
    # An empty collection has nothing to infer: no key is invented for it.
    empty = discover.discover(
        _CDC_MONGO,
        _request("preview", object="orders"),
        backends=_backends(mongo_connect=lambda d, s, **kw: (_MongoClient({"orders": []}), [])),
    )
    assert empty == {"columns": []}


def test_the_ssl_mode_of_a_mongodb_cdc_dial_becomes_the_clients_tls_options() -> None:
    assert discover._cdc_mongo_client_options("disable") == {}
    assert discover._cdc_mongo_client_options("require") == {"tls": True, "tlsAllowInvalidHostnames": True}
    for mode in ("verify-ca", "verify-full"):
        assert discover._cdc_mongo_client_options(mode) == {"tls": True, "tlsAllowInvalidHostnames": False}
    seen = {}

    def connect(dial, secrets, **kw):
        seen.update(dial)
        return _MongoClient({}), []

    secure = {**_CDC_MONGO, "dial": {**_CDC_MONGO["dial"], "sslMode": "verify-full"}}
    discover.discover(secure, _request("list"), backends=_backends(mongo_connect=connect))
    assert seen["clientOptions"] == {"tls": True, "tlsAllowInvalidHostnames": False}
    # A dial that does not name a mode is `require`, the default the API parses it with.
    bare = {**_CDC_MONGO, "dial": {k: v for k, v in _CDC_MONGO["dial"].items() if k != "sslMode"}}
    discover.discover(bare, _request("list"), backends=_backends(mongo_connect=connect))
    assert seen["clientOptions"]["tls"] is True


def test_a_mongodb_cdc_connection_test_connects_pings_and_reports_only_the_time() -> None:
    client = _PingClient({})
    result = discover.discover(
        _CDC_MONGO, _request("test"), backends=_backends(mongo_connect=lambda d, s, **kw: (client, []))
    )
    # A server that answers `hello` with no set name and `connectionStatus` with no privileges list: the
    # facts that could be read are posted (`none`), the one that could not (no `authInfo`) is left out.
    assert set(result) == {"latencyMs", "mongoSetup"}
    assert result["mongoSetup"] == {"replicaSet": "none"}
    assert isinstance(result["latencyMs"], int) and result["latencyMs"] >= 0
    assert client.admin.commands == ["ping", "hello", "connectionStatus"]
    assert client.closed is True
    body = discover.result_body(_request("test"), result=result, error_class=None)
    assert body == {
        "requestId": "req-1", "kind": "test", "state": "done", "latencyMs": result["latencyMs"],
        "mongoSetup": {"replicaSet": "none"},
    }


def _status(*privileges: tuple[dict, list[str]]) -> dict:
    return {"authInfo": {"authenticatedUserPrivileges": [{"resource": r, "actions": a} for r, a in privileges]}}


_READ_SHOP = ({"db": "shop", "collection": ""}, ["find", "changeStream", "listCollections"])
_DIAL_SHOP = {**_CDC_MONGO["dial"], "database": "shop", "replicaSet": "rs0"}
_SHOP = {**_CDC_MONGO, "dial": _DIAL_SHOP}


def _facts(**answers) -> dict:
    client = _PingClient({}, answers=answers.get("answers"), raises=answers.get("raises"))
    result = discover.discover(_SHOP, _request("test"), backends=_backends(mongo_connect=lambda d, s, **kw: (client, [])))
    return result.get("mongoSetup", {})


def test_the_setup_facts_are_closed_words_read_from_hello_and_connection_status() -> None:
    good = {"hello": {"setName": "rs0", "isWritablePrimary": True}, "connectionStatus": _status(_READ_SHOP)}
    assert _facts(answers=good) == {"replicaSet": "matches", "primary": True, "changeStream": "granted"}
    differs = {"hello": {"setName": "other", "isWritablePrimary": False}, "connectionStatus": _status(_READ_SHOP)}
    assert _facts(answers=differs) == {"replicaSet": "differs", "primary": False, "changeStream": "granted"}
    standalone = {"hello": {"isWritablePrimary": True}, "connectionStatus": _status()}
    assert _facts(answers=standalone) == {"replicaSet": "none", "primary": True, "changeStream": "missing"}


def test_change_stream_is_granted_only_by_privileges_that_cover_the_database_and_hold_both_actions() -> None:
    def fact(*privileges):
        answers = {"hello": {"setName": "rs0", "isWritablePrimary": True}, "connectionStatus": _status(*privileges)}
        return _facts(answers=answers)["changeStream"]

    both = ["find", "changeStream"]
    assert fact(({"db": "shop", "collection": ""}, both)) == "granted"
    assert fact(({"db": "", "collection": ""}, both)) == "granted"  # every database
    assert fact(({"anyResource": True}, both)) == "granted"
    # Union of the covering privileges: the two actions may come from two grants.
    assert fact(({"db": "shop", "collection": ""}, ["find"]), ({"db": "", "collection": ""}, ["changeStream"])) == "granted"
    # Another database, one collection only, or one action only does not count.
    assert fact(({"db": "other", "collection": ""}, both)) == "missing"
    assert fact(({"db": "shop", "collection": "orders"}, both)) == "missing"
    assert fact(({"db": "shop", "collection": ""}, ["find"])) == "missing"
    assert fact(({"cluster": True}, both)) == "missing"


def test_a_fact_whose_command_raised_is_not_posted_and_no_error_text_is_kept() -> None:
    boom = RuntimeError(f"not authorized {SECRET_TEXT}")
    hello = {"setName": "rs0", "isWritablePrimary": True}
    only_status = _facts(answers={"connectionStatus": _status(_READ_SHOP)}, raises={"hello": boom})
    assert only_status == {"changeStream": "granted"}
    only_hello = _facts(answers={"hello": hello}, raises={"connectionStatus": boom})
    assert only_hello == {"replicaSet": "matches", "primary": True}
    client = _PingClient({}, raises={"hello": boom, "connectionStatus": boom})
    result = discover.discover(_SHOP, _request("test"), backends=_backends(mongo_connect=lambda d, s, **kw: (client, [])))
    assert "mongoSetup" not in result  # no key at all when nothing was read
    assert SECRET_TEXT not in json.dumps(result)


def test_a_mongodb_cdc_test_failure_is_posted_as_its_class_and_never_its_text() -> None:
    refused = _driver_error("pymongo", "OperationFailure", code=18)(f"Authentication failed {SECRET_TEXT}")
    for exc, expected in [(refused, "sign_in_refused"), (_driver_error("pymongo", "ServerSelectionTimeoutError")("x"), "failed")]:
        client = _PingClient({}, fail=exc)
        posted: list[dict] = []
        error_class = discover.run_request(
            _request("test"),
            connector_loader=lambda cid: _CDC_MONGO,
            poster=posted.append,
            backends=_backends(mongo_connect=lambda d, s, _c=client, **kw: (_c, [])),
        )
        assert error_class == expected
        assert posted == [{"requestId": "req-1", "kind": "test", "state": "failed", "errorClass": expected}]
        assert SECRET_TEXT not in json.dumps(posted)
        assert client.closed is True


def test_a_sql_cdc_connector_is_still_not_read_by_the_orchestrator() -> None:
    sql_cdc = {**_CDC_MONGO, "dial": {"driver": "postgres", "host": "h", "port": 5432, "database": "d", "user": "u"}}
    for kind in ("list", "test"):
        posted: list[dict] = []
        assert discover.run_request(_request(kind), connector_loader=lambda cid: sql_cdc, poster=posted.append) == "not_supported"


# --- Kafka -----------------------------------------------------------------------------


class _Consumer:
    def __init__(self, topics: dict[str, list[list[bytes]]]):
        self._topics = topics
        self.positions: dict = {}
        self.commits: list = []
        self.closed = False
        brokers = [SimpleNamespace(host="kafka.example", port=9092)]
        self._client = SimpleNamespace(cluster=SimpleNamespace(brokers=lambda: brokers))

    def topics(self):
        return set(self._topics)

    def partitions_for_topic(self, topic):
        return set(range(len(self._topics[topic]))) if topic in self._topics else None

    def assign(self, partitions):
        self.assigned = list(partitions)

    def end_offsets(self, partitions):
        return {tp: len(self._topics[tp.topic][tp.partition]) for tp in partitions}

    def beginning_offsets(self, partitions):
        return {tp: 0 for tp in partitions}

    def seek(self, tp, offset):
        self.positions[tp] = offset

    def position(self, tp):
        return self.positions[tp]

    def poll(self, timeout_ms, max_records):
        out = {}
        for tp, position in list(self.positions.items()):
            messages = self._topics[tp.topic][tp.partition]
            batch = [
                SimpleNamespace(value=value, partition=tp.partition, offset=offset, timestamp=offset)
                for offset, value in enumerate(messages)
                if offset >= position
            ]
            if batch:
                out[tp] = batch
                self.positions[tp] = len(messages)
        return out

    def commit(self, *args, **kwargs):
        self.commits.append((args, kwargs))

    def close(self):
        self.closed = True


_KAFKA = {
    "id": "conn-1",
    "adapter": "kafka",
    "dial": {
        "bootstrapServers": ["kafka.example:9092"],
        "topic": "orders-v1",
        "groupId": "lakehouse-conn-1",
        "auth": {"type": "none"},
    },
    "secretRef": None,
}


def _json_messages(count: int) -> list[bytes]:
    return [json.dumps({"id": i, "name": f"n{i}"}).encode() for i in range(count)]


def test_kafka_lists_the_brokers_topics_with_the_internal_ones_hidden() -> None:
    consumer = _Consumer({"orders-v1": [[]], "__consumer_offsets": [[]], "Payments": [[]]})
    backends = _backends(kafka_consumer=lambda spec, secrets, *, group_id: consumer)
    result = discover.discover(_KAFKA, _request("list", search="p"), backends=backends)
    assert [o["name"] for o in result["objects"]] == ["Payments"]
    assert result["objects"][0]["kind"] == "topic"
    everything = discover.discover(_KAFKA, _request("list"), backends=backends)
    assert [o["name"] for o in everything["objects"]] == ["Payments", "orders-v1"]
    assert consumer.closed is True


def test_kafka_preview_reads_the_last_hundred_messages_and_never_commits() -> None:
    consumer = _Consumer({"orders-v1": [_json_messages(250)]})
    groups = []

    def factory(spec, secrets, *, group_id):
        groups.append(group_id)
        return consumer

    result = discover.discover(
        _KAFKA, _request("preview", object="orders-v1"), backends=_backends(kafka_consumer=factory)
    )
    assert [(c["name"], c["typeName"], c["inferred"]) for c in result["columns"]] == [
        ("id", "integer", True),
        ("name", "string", True),
    ]
    assert consumer.commits == [], "a preview moves no offset"
    assert groups == ["lakehouse-discover-req-1"]
    assert groups[0] != _KAFKA["dial"]["groupId"], "a group id the connector never uses"
    (start,) = consumer.positions.values()
    assert start == 250, "it read to the end"
    assert consumer.closed is True


def test_kafka_preview_seeks_to_the_last_hundred_of_each_partition() -> None:
    consumer = _Consumer({"orders-v1": [_json_messages(250), _json_messages(30)]})
    seeks = []
    original = consumer.seek

    def seek(tp, offset):
        seeks.append((tp.partition, offset))
        original(tp, offset)

    consumer.seek = seek
    discover.discover(
        _KAFKA,
        _request("preview", object="orders-v1"),
        backends=_backends(kafka_consumer=lambda spec, secrets, *, group_id: consumer),
    )
    assert sorted(seeks) == [(0, 150), (1, 0)]


def test_kafka_preview_of_an_empty_topic_or_of_non_json_messages_has_no_columns() -> None:
    for messages in ([], [b"not json", b"\xff\xfe", b"[1, 2]"]):
        consumer = _Consumer({"t": [messages]})
        result = discover.discover(
            _KAFKA,
            _request("preview", object="t"),
            backends=_backends(kafka_consumer=lambda spec, secrets, *, group_id, c=consumer: c),
        )
        assert result == {"columns": []}, messages


def test_kafka_checks_every_advertised_broker_before_reading() -> None:
    consumer = _Consumer({"t": [[]]})

    def refuse(host, port):
        raise ssrf_guard.SsrfBlocked("refusing to dial 'kafka.example'")

    backends = _backends(kafka_consumer=lambda spec, secrets, *, group_id: consumer, resolve_checked=refuse)
    with pytest.raises(ssrf_guard.SsrfBlocked):
        discover.discover(_KAFKA, _request("list"), backends=backends)
    assert consumer.positions == {} and consumer.closed is True


def test_kafka_preview_of_a_topic_the_broker_does_not_list_is_a_failure() -> None:
    consumer = _Consumer({"t": [[]]})
    backends = _backends(kafka_consumer=lambda spec, secrets, *, group_id: consumer)
    with pytest.raises(ValueError):
        discover.discover(_KAFKA, _request("preview", object="missing"), backends=backends)


# --- SFTP -------------------------------------------------------------------------------


class _Handle:
    def __init__(self, data: bytes):
        self._data = data

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def read(self, n):
        return self._data[:n]


class _Sftp:
    def __init__(self, entries, files=None):
        self._entries = entries
        self._files = files or {}
        self.listed = None

    def listdir_attr(self, path):
        self.listed = path
        return self._entries

    def open(self, path):
        return _Handle(self._files[path])


def _entry(name: str, *, regular: bool = True, size: int = 10, mtime: float | None = 1_790_000_000.0):
    mode = stat.S_IFREG | 0o644 if regular else stat.S_IFDIR | 0o755
    return SimpleNamespace(filename=name, st_mode=mode, st_size=size, st_mtime=mtime)


_SFTP = {
    "id": "conn-1",
    "adapter": "sftp",
    "dial": {
        "host": "sftp.example",
        "port": 22,
        "user": "u",
        "hostKeyFingerprint": "SHA256:x",
        "path": "/inbox/",
        "fileFormat": "csv",
        "auth": {"type": "password"},
    },
    "secretRef": "env:CONNECTOR_SFTP_PASSWORD",
}


def _sftp_backends(sftp: _Sftp) -> tuple[Backends, SimpleNamespace]:
    client = SimpleNamespace(closed=False)
    client.close = lambda: setattr(client, "closed", True)
    return _backends(sftp_session=lambda spec, secrets, *, resolve_checked: (client, sftp)), client


def test_sftp_lists_files_of_the_path_by_glob_with_size_and_date_and_closes_the_session() -> None:
    sftp = _Sftp(
        [
            _entry("b-2026.csv", size=20),
            _entry("a-2026.csv", size=10),
            _entry("notes.txt"),
            _entry("subdir", regular=False),
            _entry("nodate.csv", mtime=None),
        ]
    )
    backends, client = _sftp_backends(sftp)
    result = discover.discover(_SFTP, _request("list", glob="*-2026.csv"), backends=backends)
    assert sftp.listed == "/inbox"
    assert result["truncated"] is False
    assert [o["name"] for o in result["objects"]] == ["a-2026.csv", "b-2026.csv"]
    first = result["objects"][0]
    assert first["kind"] == "file" and first["size"] == 10
    assert first["modifiedAt"].endswith("Z") and "T" in first["modifiedAt"]
    assert client.closed is True

    everything = discover.discover(_SFTP, _request("list"), backends=backends)
    assert "nodate.csv" in [o["name"] for o in everything["objects"]]
    nodate = next(o for o in everything["objects"] if o["name"] == "nodate.csv")
    assert "modifiedAt" not in nodate, "no time is invented"
    assert "subdir" not in [o["name"] for o in everything["objects"]]


def test_sftp_refuses_a_directory_of_more_than_the_scan_limit() -> None:
    sftp = _Sftp([_entry(f"f{i}.csv") for i in range(discover.MAX_SCANNED + 1)])
    backends, client = _sftp_backends(sftp)
    with pytest.raises(discover.TooManyObjects):
        discover.discover(_SFTP, _request("list"), backends=backends)
    assert client.closed is True


def test_sftp_previews_the_head_of_a_csv_file() -> None:
    sftp = _Sftp([], files={"/inbox/sales.csv": b"id,name,joined\n1,ann,2026-01-02\n2,bob,2026-02-03\n"})
    backends, _ = _sftp_backends(sftp)
    result = discover.discover(_SFTP, _request("preview", object="sales.csv"), backends=backends)
    assert [(c["name"], c["typeName"], c["inferred"]) for c in result["columns"]] == [
        ("id", "integer", True),
        ("name", "string", True),
        ("joined", "date", True),
    ]


# --- Oracle -----------------------------------------------------------------------------


class _Connection:
    def __init__(self, answers):
        self._answers = answers
        self.statements: list[tuple[str, dict]] = []

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def execute(self, statement, params):
        sql = str(statement)
        self.statements.append((sql, dict(params)))
        for needle, rows in self._answers.items():
            if needle in sql:
                return rows
        raise AssertionError(f"unexpected statement: {sql}")


class _Engine:
    def __init__(self, answers):
        self.connection = _Connection(answers)
        self.disposed = False

    def connect(self):
        return self.connection

    def dispose(self):
        self.disposed = True


_ORACLE = {
    "id": "conn-1",
    "adapter": "sql",
    "dial": {"driver": "oracle", "host": "o.example", "port": 1521, "database": "FREEPDB1", "user": "u"},
    "secretRef": "env:CONNECTOR_ORACLE_PASSWORD",
}


def _oracle_backends(engine: _Engine) -> Backends:
    return _backends(oracle_engine=lambda spec, secrets, *, resolve_checked: engine)


def test_oracle_lists_the_schemas_the_account_can_read_then_that_schemas_base_tables() -> None:
    engine = _Engine(
        {
            "all_users": [("APP",), ("HR",)],
            "FROM all_tables WHERE owner": [("ORDERS",), ("CUSTOMERS",), ("MORE",)],
        }
    )
    schemas = discover.discover(_ORACLE, _request("list"), backends=_oracle_backends(engine))
    assert [(o["name"], o["kind"]) for o in schemas["objects"]] == [("APP", "schema"), ("HR", "schema")]

    tables = discover.discover(_ORACLE, _request("list", schema="APP", limit=2), backends=_oracle_backends(engine))
    assert [(o["name"], o["kind"]) for o in tables["objects"]] == [("APP.ORDERS", "table"), ("APP.CUSTOMERS", "table")]
    assert tables["truncated"] is True, "one more than the limit was fetched"
    sql, params = engine.connection.statements[-1]
    assert params["owner"] == "APP" and params["max_rows"] == 3
    assert ":owner" in sql and "APP" not in sql, "values are bound, not interpolated"
    assert engine.disposed is True


def test_oracle_search_text_is_bound_and_like_escaped() -> None:
    engine = _Engine({"all_users": []})
    discover.discover(_ORACLE, _request("list", search="a_b%"), backends=_oracle_backends(engine))
    _, params = engine.connection.statements[-1]
    assert params["search"] == "a\\_b\\%"
    engine = _Engine({"all_users": []})
    discover.discover(_ORACLE, _request("list"), backends=_oracle_backends(engine))
    assert engine.connection.statements[-1][1]["search"] is None


def test_oracle_previews_declared_columns_and_the_primary_key_in_key_order() -> None:
    engine = _Engine(
        {
            "all_tab_columns": [("ID", "NUMBER"), ("NOTE", "VARCHAR2")],
            "all_constraints": [("ID",)],
        }
    )
    result = discover.discover(_ORACLE, _request("preview", object="APP.ORDERS"), backends=_oracle_backends(engine))
    assert result == {
        "columns": [
            {"name": "ID", "typeName": "NUMBER", "inferred": False},
            {"name": "NOTE", "typeName": "VARCHAR2", "inferred": False},
        ],
        "primaryKey": ["ID"],
    }
    assert engine.disposed is True
    with pytest.raises(ValueError):
        discover.discover(_ORACLE, _request("preview", object="ORDERS"), backends=_oracle_backends(_Engine({})))


# --- the connection test (SRC-4 T1.1) -----------------------------------------------------------


def test_an_oracle_connection_test_connects_runs_one_statement_and_reports_only_the_time() -> None:
    engine = _Engine({"SELECT 1 FROM DUAL": [(1,)]})
    result = discover.discover(_ORACLE, _request("test"), backends=_oracle_backends(engine))
    assert list(result) == ["latencyMs"]
    assert isinstance(result["latencyMs"], int) and result["latencyMs"] >= 0
    assert [sql for sql, _ in engine.connection.statements] == ["SELECT 1 FROM DUAL"]
    assert engine.disposed is True

    body = discover.result_body(_request("test"), result=result, error_class=None)
    assert body == {"requestId": "req-1", "kind": "test", "state": "done", "latencyMs": result["latencyMs"]}


def test_a_refused_login_is_posted_as_sign_in_refused_and_an_unreachable_host_as_unreachable() -> None:
    import sqlalchemy.exc

    def refused(code: str):
        arg = SimpleNamespace(full_code=code)
        return sqlalchemy.exc.DatabaseError("connect", {}, _driver_error("oracledb", "DatabaseError")(arg))

    for exc, expected in [(refused("ORA-01017"), "sign_in_refused"), (ConnectionRefusedError(), "unreachable")]:

        def engine_that_fails(spec, secrets, *, resolve_checked, _exc=exc):
            raise _exc

        posted: list[dict] = []
        error_class = discover.run_request(
            _request("test"),
            connector_loader=lambda cid: _ORACLE,
            poster=posted.append,
            backends=_backends(oracle_engine=engine_that_fails),
        )
        assert error_class == expected
        assert posted == [{"requestId": "req-1", "kind": "test", "state": "failed", "errorClass": expected}]
        assert SECRET_TEXT not in json.dumps(posted)


def test_only_oracle_is_tested_by_the_orchestrator() -> None:
    posted: list[dict] = []
    error_class = discover.run_request(
        _request("test"), connector_loader=lambda cid: _MONGO, poster=posted.append
    )
    assert error_class == "not_supported"
    assert posted[0]["kind"] == "test" and posted[0]["state"] == "failed"
    assert discover.DiscoverRequest.from_op_config(
        {"connector_id": "c", "request_id": "r", "kind": "test"}
    ).kind == "test"
    with pytest.raises(ValueError):
        discover.DiscoverRequest.from_op_config({"connector_id": "c", "request_id": "r", "kind": "tests"})


# --- REST (the column preview) -------------------------------------------------------------


_REST = {
    "id": "conn-1",
    "adapter": "rest",
    "dial": {
        "baseUrl": "https://api.example",
        "auth": {"type": "bearer"},
        "pagination": {"type": "page", "param": "page"},
        "endpoints": [{"path": "/orders", "recordsPath": "data"}, {"path": "/customers"}],
    },
    "secretRef": "env:CONNECTOR_REST_TOKEN",
}


def test_a_rest_preview_goes_through_the_adapters_first_page_reader() -> None:
    seen = {}

    def first_page(dial, secrets, path, *, limit, resolve_checked, connector_id):
        # SRC-12 part C: the reader is also told the connector, so a rotated
        # refresh token can be reported.
        seen.update(path=path, limit=limit, connector_id=connector_id)
        return [{"id": 1, "total": 1.5}, {"id": 2}]

    result = discover.discover(
        _REST, _request("preview", object="/orders"), backends=_backends(rest_first_page=first_page)
    )
    assert seen == {"path": "/orders", "limit": 100, "connector_id": "conn-1"}
    assert [(c["name"], c["typeName"]) for c in result["columns"]] == [("id", "integer"), ("total", "number")]


def test_a_rest_list_is_the_apis_not_the_orchestrators() -> None:
    with pytest.raises(discover.NotSupported):
        discover.discover(_REST, _request("list"), backends=_backends())


def _page_get(calls: list):
    def get(url, *, headers, params):
        calls.append((url, dict(headers), dict(params)))
        return {"data": [{"id": 1}, {"id": 2}, {"id": 3}], "next": "x"}

    return get


def test_the_rest_adapters_first_page_makes_one_request_with_the_loads_own_parameters() -> None:
    calls: list = []
    records = rest_adapter.first_page_records(
        _REST["dial"],
        {"token": "t"},
        "/orders",
        limit=2,
        resolve_checked=lambda host, port: _address(port=port),
        http_get=_page_get(calls),
    )
    assert records == [{"id": 1}, {"id": 2}]
    assert calls == [("https://api.example/orders", {"Authorization": "Bearer t"}, {"page": 1})]


def test_the_rest_adapters_first_page_refuses_an_endpoint_the_connector_does_not_declare() -> None:
    with pytest.raises(ValueError):
        rest_adapter.first_page_records(
            _REST["dial"],
            {"token": "t"},
            "/secrets",
            limit=2,
            resolve_checked=lambda host, port: _address(port=port),
            http_get=_page_get([]),
        )


def test_the_rest_adapters_first_page_checks_the_address_before_any_request() -> None:
    calls: list = []

    def refuse(host, port):
        raise ssrf_guard.SsrfBlocked("refusing to dial 'api.example'")

    with pytest.raises(ssrf_guard.SsrfBlocked):
        rest_adapter.first_page_records(
            _REST["dial"], {"token": "t"}, "/orders", limit=2, resolve_checked=refuse, http_get=_page_get(calls)
        )
    assert calls == []


# --- the types the orchestrator does not read ------------------------------------------------


@pytest.mark.parametrize(
    "connector",
    [
        {"id": "c", "adapter": "files", "dial": {}},
        {"id": "c", "adapter": "sheets", "dial": {}},
        {"id": "c", "adapter": "cdc", "dial": {}},
        {"id": "c", "adapter": "sql", "dial": {"driver": "postgres"}},
    ],
)
def test_a_type_the_orchestrator_does_not_read_is_not_supported(connector) -> None:
    with pytest.raises(discover.NotSupported):
        discover.discover(connector, _request("list"), backends=_backends())


# --- the run: read, classify, post ---------------------------------------------------------------


def test_a_failed_read_is_posted_as_its_class_and_no_source_text_leaves_the_process(caplog) -> None:
    def explode(connector, request, *, backends=None):
        raise _driver_error("pymongo", "OperationFailure", code=13)(f"not authorized on shop: {SECRET_TEXT}")

    posted = []
    with caplog.at_level(logging.DEBUG):
        original = discover.discover
        discover.discover = explode
        try:
            error_class = discover.run_request(
                _request("list"), connector_loader=lambda cid: _MONGO, poster=posted.append
            )
        finally:
            discover.discover = original
    assert error_class == "no_permission"
    assert posted == [{"requestId": "req-1", "kind": "list", "state": "failed", "errorClass": "no_permission"}]
    assert SECRET_TEXT not in json.dumps(posted)
    assert SECRET_TEXT not in caplog.text
    assert "OperationFailure" in caplog.text, "the type name is logged"


def test_a_connector_that_cannot_be_fetched_is_posted_as_a_failure_too() -> None:
    def loader(connector_id):
        raise discover.NotSupported("the connector has no ingest spec")

    posted = []
    assert discover.run_request(_request("list"), connector_loader=loader, poster=posted.append) == "not_supported"
    assert posted[0]["state"] == "failed"


def test_a_successful_read_is_posted_with_its_result() -> None:
    client = _MongoClient({"orders": []})
    posted = []
    error_class = discover.run_request(
        _request("list"),
        connector_loader=lambda cid: _MONGO,
        poster=posted.append,
        backends=_backends(mongo_connect=lambda dial, secrets, **kw: (client, [])),
    )
    assert error_class is None
    assert posted == [
        {
            "requestId": "req-1",
            "kind": "list",
            "state": "done",
            "objects": [{"name": "orders", "kind": "collection"}],
            "truncated": False,
        }
    ]


def test_a_post_that_fails_fails_the_run_instead_of_hiding_it() -> None:
    def poster(body):
        raise schema_observer.ObservationUnreachable("the API could not be reached (ConnectionError)")

    with pytest.raises(schema_observer.ObservationUnreachable):
        discover.run_request(
            _request("list"),
            connector_loader=lambda cid: _MONGO,
            poster=poster,
            backends=_backends(mongo_connect=lambda dial, secrets, **kw: (_MongoClient({}), [])),
        )


def test_the_result_is_posted_to_the_discovery_results_route_with_the_service_identity(monkeypatch) -> None:
    sent = {}

    def post_json(cfg, path, payload, what):
        sent.update(cfg=cfg, path=path, payload=payload, what=what)
        return {"recorded": True}

    monkeypatch.setattr(schema_observer, "_post_json", post_json)
    cfg = schema_observer.ObserverConfig(api_url="http://api", service_token="t")
    discover.post_result(cfg, "conn-9", {"requestId": "r"})
    assert sent["path"] == "/api/connectors/conn-9/discovery-results"
    assert sent["cfg"] is cfg and sent["payload"] == {"requestId": "r"}


def _context(**config):
    return build_op_context(
        op_config={"connector_id": "conn-1", "request_id": "req-1", "kind": "list", **config}
    )


def test_the_op_fails_without_a_retry_when_the_api_refuses_the_result(monkeypatch) -> None:
    monkeypatch.setattr(discover, "_fetch_connector", lambda cid: _MONGO)
    monkeypatch.setattr(
        discover, "discover", lambda connector, request, *, backends=None: {"objects": [], "truncated": False}
    )

    def refuse(cfg, connector_id, body):
        raise schema_observer.ObservationRefused("the API refused the discovery result (HTTP 400)")

    monkeypatch.setattr(discover, "post_result", refuse)
    with pytest.raises(Failure) as caught:
        discover.run_discover(_context())
    assert caught.value.allow_retries is False


def test_the_op_posts_the_answer_for_its_own_request(monkeypatch) -> None:
    monkeypatch.setattr(discover, "_fetch_connector", lambda cid: _MONGO)
    monkeypatch.setattr(
        discover, "discover", lambda connector, request, *, backends=None: {"objects": [], "truncated": False}
    )
    posted = []
    monkeypatch.setattr(discover, "post_result", lambda cfg, connector_id, body: posted.append((connector_id, body)))
    discover.run_discover(_context())
    assert posted == [
        ("conn-1", {"requestId": "req-1", "kind": "list", "state": "done", "objects": [], "truncated": False})
    ]


def test_the_job_is_registered_and_its_op_declares_its_source_and_what_it_writes() -> None:
    from dispar_orchestrate.definitions import defs

    job_def = defs.resolve_job_def("discover_job")
    (node,) = job_def.graph.node_dict.values()
    assert node.definition.name == "run_discover"
    assert node.definition.tags["source_ref"] == "dispar_orchestrate/discover.py::run_discover"
    assert "discovery-results" in node.definition.tags["writes"]


def test_the_real_kafka_consumer_is_built_only_from_settings_kafka_python_knows(monkeypatch) -> None:
    # SRC-13 review 3 BLOCKER 1: the consumer was given `api_version_auto_timeout_ms`, which kafka-python
    # 3.0.11 rejects at construction ("Unrecognized configs"), so every Kafka listing failed in CI while the
    # loader's consumer worked. The fake refuses what the real class refuses: a key outside its DEFAULT_CONFIG.
    from kafka import KafkaConsumer as RealConsumer
    from kafka.errors import KafkaConfigurationError

    seen: dict = {}

    class _Strict:
        def __init__(self, **configs) -> None:
            unknown = set(configs) - set(RealConsumer.DEFAULT_CONFIG)
            if unknown:
                raise KafkaConfigurationError(f"Unrecognized configs: {unknown}")
            seen.update(configs)

    monkeypatch.setattr(discover, "KafkaConsumer", _Strict)
    discover._real_kafka_consumer({"bootstrapServers": ["k:9092"], "auth": {"type": "none"}}, {}, group_id="g")
    assert seen["enable_auto_commit"] is False and seen["group_id"] == "g"
    assert seen["request_timeout_ms"] > 10_000  # kafka-python wants it above the session timeout


def test_an_unclassified_failure_is_logged_at_warning_with_its_type_name_and_never_its_text(caplog) -> None:
    class Odd(Exception):
        pass

    def loader(_id: str) -> dict:
        raise Odd("secret-looking upstream text")

    with caplog.at_level(logging.WARNING, logger=discover.logger.name):
        discover.run_request(_request("list"), connector_loader=loader, poster=lambda body: None)
    records = [r for r in caplog.records if r.levelno == logging.WARNING]
    assert records and "Odd" in records[0].getMessage()
    assert "secret-looking" not in caplog.text


# --- SRC-3 T1.10: business apps through the vendor module ------------------------------------


class _FakeSaasModule:
    def __init__(self, *, describes: bool = True) -> None:
        from dispar_orchestrate.adapters.saas import Field

        self._fields = [Field("id", "integer", False), Field("subject", "text")]
        if describes:
            self.describe = lambda session, parts, dial, name: self._fields

    def list_objects(self, session, parts, dial):
        from dispar_orchestrate.saas.objects import ObjectDecl

        return [
            ObjectDecl(name="tickets", primary_key=("id",), cursor="updated_at"),
            ObjectDecl(name="owners", primary_key=("id",), full_reason="the API has no last-modified filter"),
            ObjectDecl(name="Users", primary_key=("id",), cursor="updated_at"),
        ]

    def declaration(self, session, parts, dial, name):
        return next(d for d in self.list_objects(session, parts, dial) if d.name == name)


_SAAS = {
    "id": "conn-1",
    "adapter": "saas",
    "dial": {"vendor": "netsuite", "account": "1234567", "auth": {"type": "integration"}},  # the module is injected by `_saas_backends`
    "secretRef": "env:CONNECTOR_SAAS_TOKEN",
}


def _saas_backends(module) -> Backends:
    def open_session(dial, secrets, *, connector_id, resolve_checked):
        from dispar_orchestrate.saas import vendors

        return module, vendors.vendor("zendesk"), vendors.AccountParts("acme"), object()

    return _backends(saas_open=open_session)


def test_a_saas_list_names_each_object_with_how_it_loads_and_honours_search() -> None:
    result = discover.discover(_SAAS, _request("list"), backends=_saas_backends(_FakeSaasModule()))
    assert result["objects"] == [
        {"name": "Users", "kind": "object", "load": "incremental"},
        {"name": "owners", "kind": "object", "load": "re-read in full: the API has no last-modified filter"},
        {"name": "tickets", "kind": "object", "load": "incremental"},
    ]
    found = discover.discover(_SAAS, _request("list", search="USER"), backends=_saas_backends(_FakeSaasModule()))
    assert [o["name"] for o in found["objects"]] == ["Users"]


def test_a_saas_fields_list_gives_the_vendors_dimensions_and_metrics_to_build_a_report_from() -> None:
    """Slice 3 review BLOCKER 9: `schema = "fields"` lists what an object can be defined from."""

    class WithFields(_FakeSaasModule):
        def list_fields(self, session, parts, dial):
            return [{"name": "sessions", "kind": "metric"}, {"name": "country", "kind": "dimension"}, {"name": "countryId", "kind": "dimension"}]

    result = discover.discover(_SAAS, _request("list", schema="fields"), backends=_saas_backends(WithFields()))
    assert result["objects"] == [
        {"name": "country", "kind": "dimension"},
        {"name": "countryId", "kind": "dimension"},
        {"name": "sessions", "kind": "metric"},
    ]
    found = discover.discover(_SAAS, _request("list", schema="fields", search="ID"), backends=_saas_backends(WithFields()))
    assert [o["name"] for o in found["objects"]] == ["countryId"]
    with pytest.raises(discover.NotSupported):
        discover.discover(_SAAS, _request("list", schema="fields"), backends=_saas_backends(_FakeSaasModule()))


def test_a_saas_preview_gives_the_vendors_fields_and_the_declared_key() -> None:
    result = discover.discover(_SAAS, _request("preview", object="tickets"), backends=_saas_backends(_FakeSaasModule()))
    assert result["columns"] == [
        {"name": "id", "typeName": "integer", "inferred": False},
        {"name": "subject", "typeName": "text", "inferred": False},
    ]
    assert result["primaryKey"] == ["id"]


def test_a_saas_preview_of_a_vendor_that_does_not_describe_is_not_supported() -> None:
    with pytest.raises(discover.NotSupported):
        discover.discover(
            _SAAS, _request("preview", object="tickets"), backends=_saas_backends(_FakeSaasModule(describes=False))
        )


def test_the_saas_failures_have_discovery_classes() -> None:
    from dispar_orchestrate.saas import session as saas_session
    from dispar_orchestrate.saas import vendors

    assert discover.classify_discovery(saas_session.SaasSignInRefused()) == discover.SIGN_IN_REFUSED
    assert discover.classify_discovery(saas_session.SaasPermissionMissing()) == discover.NO_PERMISSION
    assert discover.classify_discovery(vendors.UnsupportedVendor()) == discover.NOT_SUPPORTED


def test_a_saas_vendor_with_no_module_is_not_supported_through_the_real_opener(monkeypatch: pytest.MonkeyPatch) -> None:
    # Slice 6 gave NetSuite, the last of the ten, a module; remove it to have the case of a vendor id added to the table first.
    from dispar_orchestrate.saas import vendors

    monkeypatch.delitem(vendors.MODULE_PATHS, "netsuite")
    monkeypatch.delitem(vendors._MODULES, "netsuite", raising=False)
    with pytest.raises(Exception) as raised:
        discover.discover(_SAAS, _request("list"), backends=_backends())
    assert discover.classify_discovery(raised.value) == discover.NOT_SUPPORTED


def test_a_saas_object_the_vendor_cannot_describe_has_no_preview() -> None:
    module = _FakeSaasModule()
    module.describe = lambda session, parts, dial, name: None  # type: ignore[method-assign]
    with pytest.raises(discover.NotSupported):
        discover.discover(_SAAS, _request("preview", object="tickets"), backends=_saas_backends(module))
