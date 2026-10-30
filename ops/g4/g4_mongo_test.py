#!/usr/bin/env python3
"""G4 acceptance test for MongoDB change capture (`SRC-4` slice 5, plan task T5.7; ADR 0016).

It proves the same things as the other change-capture gates (`cdc_gate.py`): the connector is
created through the API, 20,000 documents per collection are copied, each kind of change is
visible within 120 s in both load modes, a pause and a restart of `kafka-connect` lose nothing, a
new field is observed as a schema change, and `DELETE` removes the connector and its password
file. It reuses the shared module's low-level parts (the API session, the sign-in, waiting,
ClickHouse, the credential file, the discovery poll, the first copy, the restart handshake, the
password check, the schema change and the delete) and has its own orchestration and comparison,
because MongoDB differs where those parts are SQL-shaped:

- A source object is a bare collection name, not `schema.table`, and its key is `_id`.
- The documents carry `_id` = an `ObjectId` made from the number `n` (`ObjectId(f"{n:024x}")`)
  AND an integer field `id` = `n`. The `ObjectId` proves the real conversion (Debezium's key and
  value to the hex text the batch adapter stores); `id` lets the shared first-copy step and the
  freshness checks read Bronze as they read every other source.
- A MongoDB delete event carries the key only, no old document (the SQL sources send the old
  row). So the history table's delete rows have `_id` and nothing else, and the expected events of
  a delete are `(id, 'd', '')`, found by decoding `_id`.
- The connection test runs in the orchestrator (the API has no MongoDB client): `POST .../test`
  answers pending and the gate polls `GET .../discovery/test`.

# The source

`mongo-cdc-g4` (`ops/g4/docker-compose.g4-mongo.override.yml`): `mongo:7` as a one-member replica
set with authentication, started by the override; `mongo-cdc-g4-init` runs `rs.initiate` and
waits until the member is primary. The database `g4_cdc` has the collections `orders` and
`orders_history`. The connector dials as a LEAST-PRIVILEGE user: the built-in `read` role on
`g4_cdc` and nothing else (decision S5-h; the renderer sets `capture.scope=database`, whose
documented requirement is read access to that database). TLS is `disable`: the fixture has no
certificate, and the four modes are covered by the renderer's unit tests.
"""

from __future__ import annotations

import collections
import datetime
import decimal
import os
import sys
import threading
import time

import requests

import cdc_gate as g
import gate_summary
from cdc_gate import G4Failure

DATABASE = "g4_cdc"
HOST = "mongo-cdc-g4"
PORT = 27017
REPLICA_SET = "rs0"
CDC_USER = "g4_cdc"
DISCOVER_FIELDS = {"_id", "id", "customer", "amount", "status", "updated_at"}


def oid(n: int):
    """The `ObjectId` of document number `n`: 24 hex digits, the number in the low ones."""
    from bson import ObjectId

    return ObjectId(f"{n:024x}")


def number_of(hex_id: str) -> int:
    """The inverse of `oid`, for a Bronze `_id` (the hex text)."""
    return int(hex_id, 16)


def document(n: int, customer: str, status: str, amount: float) -> dict:
    return {
        "_id": oid(n), "id": n, "customer": customer, "amount": amount, "status": status,
        "updated_at": datetime.datetime.now(datetime.timezone.utc),
    }


def _client():
    from pymongo import MongoClient

    return MongoClient(
        host=HOST, port=PORT, username=os.environ["MONGO_CDC_G4_ROOT_USER"],
        password=os.environ["MONGO_CDC_G4_ROOT_PASSWORD"], authSource="admin",
        serverSelectionTimeoutMS=10_000,
    )


class MongoSource(g.Source):
    cdc_type = "MongoDB CDC"
    gate = "g4-mongo"
    catalog_db = "icecat_g4_mongo"
    schema = DATABASE
    host_for_registry = f"{HOST}:{PORT}"

    def __init__(self) -> None:
        self.password = os.environ["CONNECTOR_MONGO_CDC_PASSWORD"]
        self.client = _client()

    def coll(self, table: str):
        return self.client[DATABASE][table]

    def ping(self) -> None:
        # The member must be PRIMARY, not just up: the init service may still be initiating.
        hello = self.client.admin.command("hello")
        if not hello.get("isWritablePrimary"):
            raise G4Failure("the replica set has no primary yet")

    def dial(self) -> dict:
        return {
            "driver": "mongodb", "hosts": [f"{HOST}:{PORT}"], "database": DATABASE,
            "user": CDC_USER, "replicaSet": REPLICA_SET, "sslMode": "disable",
        }

    def object_name(self, mode: str) -> str:
        return g.TABLES[mode]  # the bare collection name, as discovery lists it

    def setup(self) -> None:
        self.client.drop_database(DATABASE)
        for table in g.TABLES.values():
            collection = self.coll(table)
            for lo in range(1, g.SEED_ROWS + 1, 5000):
                collection.insert_many(
                    [document(n, f"seed_{n}", "seed", (n % 1000) + 0.5) for n in range(lo, min(lo + 5000, g.SEED_ROWS + 1))]
                )
        # The user lives in the captured database itself: the connector's `mongodb.authsource` and the
        # orchestrator's `authSource` are both the dial's database (decision S5-c). It is dropped with
        # the database above, so a rerun starts clean.
        users = self.client[DATABASE]
        if users.command("usersInfo", CDC_USER)["users"]:
            users.command("dropUser", CDC_USER)
        # The least-privilege user (decision S5-h): the built-in `read` role on the one database.
        users.command("createUser", CDC_USER, pwd=self.password, roles=[{"role": "read", "db": DATABASE}])

    def add_column_and_observe(self) -> None:
        # A new field in one document is a new column: MongoDB has no schema to alter.
        self.coll("orders").update_one({"id": 2}, {"$set": {"note": "schema_note"}})

    # ── source changes: the same change on both collections ─────────────────────────────
    def each(self, action) -> None:
        for table in g.TABLES.values():
            action(self.coll(table))


CONNECTOR_ID = ""

# What the history collection must hold: (id, op, customer). A delete has customer '' (no old document).
EXPECTED: collections.Counter = collections.Counter()


def _expect(ids, op: str, customer) -> None:
    for i in ids:
        EXPECTED[(i, op, customer(i) if callable(customer) else customer)] += 1


def src() -> MongoSource:
    return g.SOURCE  # type: ignore[return-value]


# ── comparison ──────────────────────────────────────────────────────────────────────────


def _current_diff() -> str | None:
    source = sorted(
        (d["id"], d["customer"], decimal.Decimal(str(d["amount"])), d["status"])
        for d in src().coll(g.TABLES["current"]).find({}, {"id": 1, "customer": 1, "amount": 1, "status": 1})
    )
    found = sorted(
        (int(r[0]), r[1], decimal.Decimal(r[2]), r[3])
        for r in g.ch_rows(f"SELECT id, customer, amount, status FROM {g.bronze('current')} WHERE id > 0")
    )
    ids = [r[0] for r in found]
    duplicated = len(ids) - len(set(ids))
    missing = {r[0] for r in source} - set(ids)
    extra = set(ids) - {r[0] for r in source}
    different = len({r for r in set(source) - set(found) if r[0] in set(ids)})
    if not (missing or extra or duplicated or different) and len(source) == len(found):
        return None
    return (
        f"current: source {len(source)} documents, Bronze {len(found)}; missing {len(missing)}, extra {len(extra)}, "
        f"duplicated {duplicated}, different {different}"
    )


def _history_found() -> collections.Counter:
    found: collections.Counter = collections.Counter()
    for _id, op, customer in g.ch_rows(
        f"SELECT `_id`, `_op`, coalesce(customer, '') FROM {g.bronze('history')} WHERE `_op` != ''"
    ):
        found[(number_of(_id), op, customer)] += 1
    return found


def _history_diff() -> str | None:
    found = _history_found()
    missing, extra = EXPECTED - found, found - EXPECTED
    if not missing and not extra:
        return None
    return (
        f"history: expected {sum(EXPECTED.values())} events, Bronze {sum(found.values())}; "
        f"missing {sum(missing.values())} (e.g. {list(missing)[:3]}), twice or unexpected {sum(extra.values())} (e.g. {list(extra)[:3]})"
    )


def _wait_equal(label: str, timeout_s: float) -> None:
    def check():
        problems = [d for d in (_current_diff(), _history_diff()) if d]
        if problems:
            raise G4Failure("; ".join(problems))
        return True

    g._wait_for(f"Bronze equals the source ({label})", check, timeout_s, interval_s=10)


def step_ids_are_the_object_id_text() -> None:
    """The real conversion, end to end: every Bronze `_id` is the lower-case 24-digit hex text of
    the `ObjectId` the source holds (here made from `id`), in both collections."""
    for mode in g.MODES:
        wrong = g.ch_rows(
            f"SELECT `_id`, id FROM {g.bronze(mode)} WHERE id > 0 "
            "AND `_id` != lower(leftPad(hex(id), 24, '0')) LIMIT 5"
        )
        if wrong:
            raise G4Failure(f"the {mode} table holds _id values that are not the ObjectId hex text: {wrong}")
    print("[g4] every Bronze _id is the ObjectId's hex text", flush=True)


# ── registration, connect, discover ─────────────────────────────────────────────────────


def _register(tenant_id: str, *, suffix: str = "", replica_set: str | None = None, tolerate_refusal: bool = False) -> str:
    """Register the connector the way the console does. `replica_set` and `tolerate_refusal` are for the
    setup-check step's second connector: a wrong replica set name makes Kafka Connect refuse the config
    (the API saves the settings first and answers 422 "did not start"), which is the case under test."""
    dial = {**src().dial(), **({"replicaSet": replica_set} if replica_set is not None else {})}
    created = g.API.post(
        f"{g.API_URL}/api/connectors",
        json={
            "name": f"g4-mongo-demo-cdc{suffix}", "type": src().cdc_type, "direction": "source",
            "host": src().host_for_registry, "credential": {"source": "file", "primary": "password"},
            "environment": "production", "tenant": "g4", "tenantId": tenant_id, "residency": "", "capabilities": [],
        },
        timeout=10,
    )
    if not created.ok:
        raise G4Failure(f"create the MongoDB CDC connector failed: {created.status_code} {created.text[:300]}")
    body = created.json()
    connector_id = body["id"]
    derived = (body.get("credential") or {}).get("primary")
    if not derived:
        raise G4Failure("the create response carries no derived credential name")
    g._write_credential_file(derived, src().password)
    spec = g.API.put(
        f"{g.API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={
            "adapter": "cdc", "ingestMode": "cdc", "dial": dial,
            "sourceObjects": [
                {"name": g.TABLES[m], "target": g.TARGETS[m] if not suffix else f"g4_wrongrs_{m}", "loadMode": g.LOAD_MODES[m], "primaryKey": ["_id"]}
                for m in g.MODES
            ],
        },
        timeout=60,
    )
    refused_on_purpose = tolerate_refusal and spec.status_code == 422 and "did not start" in spec.text
    if not spec.ok and not refused_on_purpose:
        raise G4Failure(f"set the connector's ingest-spec failed: {spec.status_code} {spec.text[:300]}{_explain_refusal()}")
    return connector_id


def _explain_refusal() -> str:
    """When Kafka Connect refuses the connector, the API answers a fixed sentence (principle 4: no
    upstream text in a response). The gate may look: it asks Kafka Connect to validate the same keys
    (with the gate's own throwaway password in place of the file reference, which validation never
    stores) and returns the fields it objects to, so a refusal in CI says why. Field names and
    Kafka Connect's messages only; never the password."""
    config = {
        "connector.class": "io.debezium.connector.mongodb.MongoDbConnector", "tasks.max": "1",
        "mongodb.connection.string": f"mongodb://{HOST}:{PORT}/?replicaSet={REPLICA_SET}&directConnection=true",
        "mongodb.user": CDC_USER, "mongodb.password": src().password, "mongodb.authsource": DATABASE,
        "mongodb.ssl.enabled": "false", "topic.prefix": "cdc.diagnose",
        "collection.include.list": ",".join(f"{DATABASE}.{t}" for t in g.TABLES.values()),
        "capture.mode": "change_streams_update_full", "capture.scope": "database", "capture.target": DATABASE,
        "snapshot.mode": "initial",
    }
    try:
        resp = requests.put(
            f"{g.CONNECT_URL}/connector-plugins/io.debezium.connector.mongodb.MongoDbConnector/config/validate",
            json=config, timeout=60,
        )
        body = resp.json()
    except (requests.RequestException, ValueError) as exc:
        return f" [could not ask Kafka Connect to validate: {type(exc).__name__}]"
    problems = [
        f"{c['value']['name']}: {'; '.join(c['value']['errors'])}"
        for c in body.get("configs", []) if c.get("value", {}).get("errors")
    ]
    return " [Kafka Connect validation: " + (" | ".join(problems) if problems else f"no field is refused, error_count={body.get('error_count')}") + "]"


def _run_test(connector_id: str) -> dict:
    """`POST .../test` (answered pending: the orchestrator tests) and the final answer of
    `GET .../discovery/test`, polled until it is no longer running. Raises `G4Failure` otherwise."""
    started = g.API.post(f"{g.API_URL}/api/connectors/{connector_id}/test", timeout=30)
    if not started.ok or started.json().get("pending") is not True:
        raise G4Failure(f"POST .../test answered {started.status_code} {started.text[:200]}")
    deadline = time.time() + 240
    answer: dict = {"state": "running"}
    while answer.get("state") == "running" and time.time() < deadline:
        time.sleep(2)
        polled = g.API.get(f"{g.API_URL}/api/connectors/{connector_id}/discovery/test", timeout=30)
        if polled.ok:
            answer = polled.json()
    return answer


def _check_connect(connector_id: str) -> None:
    """`connect`: `POST .../test` is answered by the orchestrator (pending), and its job reports a
    successful connection for the least-privilege user."""
    if not g._expects_real("connect"):
        return
    try:
        answer = _run_test(connector_id)
    except (requests.RequestException, ValueError, G4Failure) as exc:
        g._record("connect", "fail", f"the connection test did not answer cleanly ({type(exc).__name__}: {str(exc)[:200]})")
        return
    if answer.get("state") == "done" and isinstance(answer.get("latencyMs"), int):
        g._record("connect", "pass", f"{answer['latencyMs']} ms, tested in the orchestrator")
    else:
        g._record("connect", "fail", f"expected state=done with a latency, got state={answer.get('state')!r} errorClass={answer.get('errorClass')!r}")


# The fixed words of `cdc_setup_check::mongo_checks` for a wrong replica set name.
REPLICA_SET_DIFFERS = "The server's replica set has another name. Use the name in `rs.status().set`."
SETUP_SETTINGS = ["changeStream", "primary", "replicaSet"]


def step_setup_checks(connector_id: str) -> None:
    """`SRC-4` Amendment 8, S8-f: the finished test lists three setup checks, all `ok` for the gate's
    connector (its user has `read` on the database and the one host is the primary); a second
    connector with a wrong `replicaSet` has that check `missing` with the fixed sentence and no
    command, and the other two unaffected. The checks never change the test's outcome."""
    try:
        good = _run_test(connector_id)
        checks = good.get("setupChecks") or []
        if sorted(c["setting"] for c in checks) != SETUP_SETTINGS or any(c["status"] != "ok" for c in checks):
            raise G4Failure(f"expected {SETUP_SETTINGS} all ok, got {checks}")
        other = _register(g._ensure_gate_tenant(), suffix="-wrong-rs", replica_set="not-the-replica-set", tolerate_refusal=True)
        try:
            answer = _run_test(other)
            found = {c["setting"]: c for c in answer.get("setupChecks") or []}
            if sorted(found) != SETUP_SETTINGS:
                raise G4Failure(f"the wrong-replica-set connector lists {sorted(found)}, expected {SETUP_SETTINGS}")
            wrong = found["replicaSet"]
            if wrong["status"] != "missing" or wrong.get("message") != REPLICA_SET_DIFFERS or wrong.get("command"):
                raise G4Failure(f"expected replicaSet missing with its fixed sentence and no command, got {wrong}")
            others = [found[k]["status"] for k in ("primary", "changeStream")]
            if others != ["ok", "ok"]:
                raise G4Failure(f"the other two checks should be ok, got {others}")
            if answer.get("state") != "done":
                raise G4Failure(f"the test of the wrong-replica-set connector should still finish, got {answer.get('state')!r}")
        finally:
            deleted = g.API.delete(f"{g.API_URL}/api/connectors/{other}", timeout=120)
            if not deleted.ok:
                print(f"[g4] cleanup: DELETE of the wrong-replica-set connector answered {deleted.status_code}", flush=True)
    except (G4Failure, requests.RequestException, KeyError, ValueError) as exc:
        g._say("setup checks", "fail", str(exc))
        raise G4Failure(f"setup checks: {exc}") from exc
    g._say("setup checks", "pass", "gate connector: replicaSet, primary, changeStream all ok; wrong replicaSet: missing alone, no command")


def _check_discover(connector_id: str) -> None:
    """`discover`: the database's collections are listed and the preview of one names its fields
    and `_id` as the key."""
    if not g._expects_real("discover"):
        return
    try:
        listing = g._discovery(connector_id, "list", {})
        preview = g._discovery(connector_id, "preview", {"object": g.TABLES["current"]})
    except (requests.RequestException, ValueError, G4Failure) as exc:
        g._record("discover", "fail", f"the discovery routes did not answer cleanly ({type(exc).__name__}: {str(exc)[:200]})")
        return
    listed = {o.get("name") for o in listing.get("objects") or []}
    if listing.get("state") != "done" or not set(g.TABLES.values()) <= listed:
        g._record("discover", "fail", f"expected both collections listed, got state={listing.get('state')!r} reason={listing.get('reason')!r} {sorted(listed)[:20]}")
        return
    found = {c.get("name") for c in preview.get("columns") or []}
    if preview.get("state") != "done" or not DISCOVER_FIELDS <= found or preview.get("primaryKey") != ["_id"]:
        g._record("discover", "fail", f"preview: state={preview.get('state')!r}, missing {sorted(DISCOVER_FIELDS - found)}, primaryKey={preview.get('primaryKey')!r}")
        return
    g._record("discover", "pass", f"{len(listed)} collections; {g.TABLES['current']} with {len(found)} fields, key _id")


# ── changes ─────────────────────────────────────────────────────────────────────────────


def _visible(mode: str, kind: str, row_id: int, marker: str) -> bool:
    if mode == "current":
        rows = g.ch_rows(f"SELECT customer FROM {g.bronze('current')} WHERE id = {row_id}")
        return (not rows) if kind == "delete" else any(r[0] == marker for r in rows)
    op = {"insert": "c", "update": "u", "delete": "d"}[kind]
    if kind == "delete":
        # The delete event has `_id` only.
        n = g.ch_rows(f"SELECT count() FROM {g.bronze('history')} WHERE `_id` = '{oid(row_id)}' AND `_op` = 'd'")
    else:
        n = g.ch_rows(f"SELECT count() FROM {g.bronze('history')} WHERE id = {row_id} AND `_op` = '{op}' AND customer = '{marker}'")
    return int(n[0][0]) > 0


def step_changes_visible_within_budget() -> None:
    new_id, upd_id, del_id = 900_001, 1, g.SEED_ROWS
    changes = [
        ("insert", new_id, "fresh_i", lambda c: c.insert_one(document(new_id, "fresh_i", "fresh", 5.0)), lambda: _expect([new_id], "c", "fresh_i")),
        ("update", upd_id, "fresh_u", lambda c: c.update_one({"id": upd_id}, {"$set": {"customer": "fresh_u", "updated_at": _now()}}), lambda: _expect([upd_id], "u", "fresh_u")),
        ("delete", del_id, "", lambda c: c.delete_one({"id": del_id}), lambda: _expect([del_id], "d", "")),
    ]
    latencies = []
    try:
        for kind, row_id, marker, action, events in changes:
            t0 = time.time()
            src().each(action)
            events()
            for mode in g.MODES:
                g._wait_for(
                    f"{kind} of id {row_id} in the {mode} collection",
                    lambda m=mode, k=kind, i=row_id, mk=marker: True if _visible(m, k, i, mk) else None,
                    g.FRESHNESS_BUDGET_SECONDS, interval_s=2,
                )
                latencies.append(f"{kind}/{mode} {time.time() - t0:.0f}s")
    except G4Failure as exc:
        _dump_topic_tail()
        g._record("reload", "fail", str(exc))
        raise
    g._record("reload", "pass", f"each kind of change visible within {g.FRESHNESS_BUDGET_SECONDS:.0f}s in both modes ({', '.join(latencies)})")


def _dump_topic_tail(count: int = 4) -> None:
    """On a failed wait, print the last messages of the current-mode topic and what the applier's own
    reader makes of each: the key names, the operation, and either the row's keys or the fixed
    refusal. Values are printed for the keys the gate's own documents carry only (`id`, `customer`,
    `_op`): the gate's data, not a customer's, and no credential is ever in a topic."""
    try:
        import json

        from kafka import KafkaConsumer, TopicPartition

        from dispar_orchestrate import cdc_apply

        topic = f"cdc.{CONNECTOR_ID}.{DATABASE}.{g.TABLES['current']}"
        consumer = KafkaConsumer(bootstrap_servers=[g.KAFKA_BOOTSTRAP], enable_auto_commit=False, consumer_timeout_ms=5000)
        try:
            partitions = [TopicPartition(topic, p) for p in sorted(consumer.partitions_for_topic(topic) or [])]
            consumer.assign(partitions)
            for tp in partitions:
                end = consumer.end_offsets([tp])[tp]
                consumer.seek(tp, max(0, end - count))
            for record in consumer:
                try:
                    shown = json.loads(record.value) if record.value else None
                    keys = sorted(shown) if isinstance(shown, dict) else shown
                    ops = shown.get("__op") if isinstance(shown, dict) else None
                    verdict = cdc_apply.event_to_row(record.value, record.offset, document=True)
                    verdict = sorted(verdict) if verdict else verdict
                except Exception as exc:  # noqa: BLE001
                    verdict = f"{type(exc).__name__}: {exc}"
                print(f"[g4] topic tail {topic}@{record.offset}: key={record.key!r} op={ops!r} value-keys={keys} -> {verdict}", flush=True)
        finally:
            consumer.close()
    except Exception as exc:  # noqa: BLE001
        print(f"[g4] could not read the topic tail ({type(exc).__name__}: {exc})", flush=True)


def _now():
    return datetime.datetime.now(datetime.timezone.utc)


def _bulk_changes() -> None:
    """20,000 changes per collection: 10,000 updates, 5,000 deletes, 5,000 inserts."""
    src().each(lambda c: c.update_many({"id": {"$gte": 1, "$lte": 10_000}}, {"$set": {"customer": "bulk_u", "updated_at": _now()}}))
    _expect(range(1, 10_001), "u", "bulk_u")
    src().each(lambda c: c.delete_many({"id": {"$gte": 10_001, "$lte": 15_000}}))
    _expect(range(10_001, 15_001), "d", "")
    src().each(lambda c: c.insert_many([document(n, "bulk_i", "bulk", 5.0) for n in range(100_001, 105_001)]))
    _expect(range(100_001, 105_001), "c", "bulk_i")


def step_lag_is_measured(connector_id: str) -> None:
    """`SRC-4` Amendment 8, S8-b: after the changes of the freshness step, `GET .../change-capture`
    carries a measured lag for both collections (slice 4's route; the sensor reads the topics through
    `topic_of`, which names `cdc.<id>.<database>.<collection>`). No number is asserted: each table
    reports a status, and a seconds value, when there is one, is not negative."""
    try:
        words = g._wait_up_to_date(connector_id, g.UP_TO_DATE_TIMEOUT_SECONDS)
        tables = g._capture(connector_id)["lag"]["tables"]
        names = {t.get("table") for t in tables}
        if names != set(g.TABLES.values()):
            raise G4Failure(f"the lag names the tables {sorted(map(str, names))}, expected {sorted(g.TABLES.values())}")
        for t in tables:
            if t.get("status") is None or (t.get("seconds") is not None and t["seconds"] < 0):
                raise G4Failure(f"a table's lag is not a measured value: {t}")
    except G4Failure as exc:
        g._say("lag", "fail", str(exc))
        raise
    g._say("lag", "pass", f"after the changes: {words}")


def step_pause_and_resume_lose_nothing(connector_id: str) -> None:
    paused = g.API.post(f"{g.API_URL}/api/connectors/{connector_id}/pause", timeout=30)
    if not paused.ok:
        raise G4Failure(f"POST .../pause answered {paused.status_code} {paused.text[:200]}")
    g._wait_capture_state(connector_id, "paused", 90)
    _bulk_changes()
    print("[g4] 20,000 changes per collection written while the connector is paused", flush=True)
    resumed = g.API.post(f"{g.API_URL}/api/connectors/{connector_id}/resume", timeout=30)
    if not resumed.ok:
        raise G4Failure(f"POST .../resume answered {resumed.status_code} {resumed.text[:200]}")
    g._wait_capture_state(connector_id, "running", 90)
    _wait_equal("after a pause and a resume", g.CATCH_UP_TIMEOUT_SECONDS)


def step_restart_kafka_connect_during_changes(connector_id: str) -> None:
    """As `cdc_gate.step_restart_kafka_connect_during_changes`: the CI job restarts the worker
    when the marker line is printed, and the writer keeps writing across the restart."""
    if os.path.exists(g.RESTART_DONE_FILE):
        os.remove(g.RESTART_DONE_FILE)
    started = threading.Event()
    errors: list[BaseException] = []

    def writer() -> None:
        try:
            for k in range(18):
                lo = 15_001 + 250 * k
                hi = lo + 249
                src().each(lambda c, lo=lo, hi=hi, k=k: c.update_many({"id": {"$gte": lo, "$lte": hi}}, {"$set": {"customer": f"stream_{k}", "updated_at": _now()}}))
                _expect(range(lo, hi + 1), "u", f"stream_{k}")
                if k == 4:
                    started.set()
                time.sleep(1.0)
        except BaseException as exc:  # noqa: BLE001
            errors.append(exc)
            started.set()

    thread = threading.Thread(target=writer, daemon=True)
    thread.start()
    started.wait(120)
    print(g.RESTART_MARKER, flush=True)
    g._wait_for("the CI job restarted kafka-connect", lambda: True if os.path.exists(g.RESTART_DONE_FILE) else None, g.RESTART_WAIT_SECONDS, interval_s=2)
    thread.join()
    if errors:
        raise G4Failure(f"the change writer failed: {errors[0]}")
    g._wait_capture_state(connector_id, "running", 240)
    _wait_equal("after a restart of kafka-connect during a stream of changes", g.CATCH_UP_TIMEOUT_SECONDS)


def main() -> int:
    source = MongoSource()
    g.SOURCE = source
    try:
        g.RECORDER = gate_summary.Recorder(gate_summary.Registry.load(), source.gate)
        owner = g.RECORDER.registry.gate_of(source.cdc_type)
        if owner != source.gate:
            raise G4Failure(f"ops/gates/coverage.json gives {source.cdc_type!r} to gate {owner!r}, but {source.gate} runs it")
        g.step_wait_for_services()
        g.step_login()
        source.setup()
        for n in range(1, g.SEED_ROWS + 1):
            EXPECTED[(n, "r", f"seed_{n}")] += 1
        print(f"[g4] seeded {g.SEED_ROWS} documents into each of {', '.join(g.TABLES.values())}", flush=True)
        g.step_create_catalog_database()
        try:
            connector_id = _register(g._ensure_gate_tenant())
        except (G4Failure, requests.RequestException, KeyError, ValueError) as exc:
            detail = f"could not register the demo CDC connector: {type(exc).__name__}: {exc}"
            g._record("connect", "fail", detail)
            g._record("discover", "fail", detail)
            raise G4Failure("the demo connector could not be registered, so nothing below can run") from exc
        global CONNECTOR_ID
        CONNECTOR_ID = connector_id
        _check_connect(connector_id)
        _check_discover(connector_id)
        g._wait_capture_state(connector_id, "running", 240)
        g.step_connector_holds_no_password(connector_id)
        g.step_first_copy_reached_bronze()
        step_ids_are_the_object_id_text()
        step_setup_checks(connector_id)
        step_changes_visible_within_budget()
        step_lag_is_measured(connector_id)
        step_pause_and_resume_lose_nothing(connector_id)
        step_restart_kafka_connect_during_changes(connector_id)
        g.step_schema_change_is_observed(connector_id)
        g.step_delete_removes_everything(connector_id)
        failed = g.RECORDER.failures()
        if failed:
            raise G4Failure("connector gate steps failed: " + "; ".join(f"{t} {s}: {d}" for t, s, d in failed))
    except G4Failure as exc:
        print(f"[g4] FAILED: {exc}", file=sys.stderr, flush=True)
        return 1
    finally:
        if g.RECORDER is not None:
            g.RECORDER.emit()
    print("[g4] PASS", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
