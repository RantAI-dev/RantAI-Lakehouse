#!/usr/bin/env python3
"""SRC-4 slice 0 harness: measures ADR 0016 (Debezium on Kafka Connect, then a Dagster
applier into Bronze) against the targets of
`docs/superpowers/plans/2026-10-29-src-4-databases-cdc.md`, slice 0.

Run on the CI runner (`.github/workflows/src4-measure.yml`) against the compose stack
of `ops/src4/docker-compose.src4.override.yml`. Nothing here is a gate of the product
and the API is not touched: the connector is registered by calling Kafka Connect's REST
API directly.

# What it does, per load mode

The two modes run on two identical source tables (`src4.orders` for "current row per
key", `src4.orders_history` for "history"), so each starts from a clean source and its
own topic and Bronze table. For each mode:

1. FRESHNESS: 30 inserts, 30 updates and 30 deletes, interleaved, one every 2 s. For each,
   the time from just BEFORE the statement is sent (so the source commit is inside the
   interval, never before it) to the first poll of ClickHouse that sees the change in
   Bronze, taken when that poll RETURNS. Both ends round against the target, never for
   it: the resolution is the poll's own duration, reported beside the numbers.
2. CORRECTNESS: after the 90, Bronze equals the source (current: same keys, same values,
   deleted keys absent; history: one row per event, in source order per key, checked
   against `src4.audit`, a trigger-written record of every change the source committed).
3. STOP AND CATCH UP: pause the connector and the mode's sensor, commit 108,000 changes
   at the source (70 % updates, 20 % inserts, 10 % deletes, counted from `src4.audit`),
   resume both, time until the last change is in Bronze, then check again that Bronze
   equals the source with no row missing and none twice (history: exactly 108,000 new
   events).
4. COST: peak memory of the broker, Kafka Connect and the code location (`docker stats`,
   sampled every 5 s over the whole run) and the applier's time per batch (from its
   `CDC_BATCH` log lines).

It prints a GATE-SUMMARY block, writes `src4-result.json`, and exits non-zero if a TARGET
is missed (p95 of a change kind over 120 s, or Bronze not equal to the source). A
measured success is never assumed: every expected status is asserted.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import shlex
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from typing import Any

import requests

SCHEMA = "src4"
CONNECTOR = "src4"
TOPIC_PREFIX = "s4"
FRESH_PER_KIND = 30
FRESH_GAP_S = 2.0
FRESH_TIMEOUT_S = 600.0
SEED_ROWS = 10_000
BULK_UPDATES = 75_600
BULK_INSERTS = 21_600
BULK_DELETES = 10_800
BULK_TOTAL = BULK_UPDATES + BULK_INSERTS + BULK_DELETES  # 108,000
P95_TARGET_S = 120.0
CATCHUP_TIMEOUT_S = 1_500.0
FIRST_COPY_TIMEOUT_S = 600.0
MODES = ("current", "history")
TABLES = {"current": "orders", "history": "orders_history"}
SENSORS = {"current": "cdc_apply_sensor_current", "history": "cdc_apply_sensor_history"}
BRONZE = {"current": "src4_orders_current", "history": "src4_orders_history"}
REPO_LOCATION = "dispar_orchestrate.definitions"
REPO_NAME = "__repository__"
JOB = "cdc_apply_job"
KINDS = ("insert", "update", "delete")
OP_OF = {"insert": "c", "update": "u", "delete": "d"}

DAGSTER_GRAPHQL = os.environ.get("DAGSTER_GRAPHQL_URL", "http://localhost:3000/graphql")
CONNECT_URL = os.environ.get("KAFKA_CONNECT_URL", "http://localhost:8083")
QUERY_DB = os.environ.get("ICEBERG_QUERY_DB", "icecat_api")
COMPOSE = shlex.split(
    os.environ.get(
        "SRC4_COMPOSE",
        "docker compose -p src4 -f docker-compose.yml -f ops/src4/docker-compose.src4.override.yml --profile dagster",
    )
)
PG_USER = os.environ.get("POSTGRES_USER", "lakehouse")
PG_DB = os.environ.get("POSTGRES_DB", "lakehouse")
CONTAINERS = {
    "broker": os.environ.get("SRC4_BROKER_CONTAINER", "src4-kafka-src4-1"),
    "connect": os.environ.get("SRC4_CONNECT_CONTAINER", "src4-connect-src4-1"),
    "applier_host": os.environ.get("SRC4_APPLIER_CONTAINER", "src4-dagster-code-location-1"),
}


class HarnessFailure(Exception):
    """A step did not do what the harness expects of it. The run is red."""


# ---------------------------------------------------------------------------
# Pure functions (unit-tested in test_measure_cdc.py, no network)
# ---------------------------------------------------------------------------


def percentile(values: list[float], p: float) -> float:
    """Nearest-rank percentile (no interpolation: an observed value, never an invented
    one). `p` in (0, 100]."""
    if not values:
        raise ValueError("percentile of nothing")
    ordered = sorted(values)
    rank = max(1, math.ceil(p / 100.0 * len(ordered)))
    return ordered[rank - 1]


def latency_stats(values: list[float]) -> dict[str, float | int]:
    return {
        "n": len(values),
        "p50_s": round(percentile(values, 50), 2),
        "p95_s": round(percentile(values, 95), 2),
        "max_s": round(max(values), 2),
    }


_MEM = re.compile(r"^\s*([0-9.]+)\s*([KMG]i?B|B)\s*/")
_UNITS = {"B": 1, "KB": 1e3, "MB": 1e6, "GB": 1e9, "KIB": 1024, "MIB": 1024**2, "GIB": 1024**3}


def parse_mem_usage(text: str) -> int:
    """The bytes in front of the slash of `docker stats`' MemUsage ("812.4MiB / 15.6GiB")."""
    found = _MEM.match(text)
    if not found:
        raise ValueError(f"cannot read a memory figure from {text!r}")
    return int(float(found.group(1)) * _UNITS[found.group(2).upper()])


@dataclass(frozen=True)
class ChangeEvent:
    kind: str  # insert | update | delete
    n: int  # 0..29 within its kind
    row_id: int
    marker: str  # the customer value that identifies this event ("" for a delete)

    def sql(self, table: str) -> str:
        if self.kind == "insert":
            return (
                f"INSERT INTO {SCHEMA}.{table} (id, customer, amount, status, updated_at) "
                f"VALUES ({self.row_id}, '{self.marker}', 5.00, 'fresh', now())"
            )
        if self.kind == "update":
            return f"UPDATE {SCHEMA}.{table} SET customer = '{self.marker}', updated_at = now() WHERE id = {self.row_id}"
        return f"DELETE FROM {SCHEMA}.{table} WHERE id = {self.row_id}"


def freshness_events(per_kind: int = FRESH_PER_KIND) -> list[ChangeEvent]:
    """Inserts, updates and deletes interleaved. Updates touch seed ids 1..n and deletes
    seed ids 9001..9000+n, so neither meets the bulk step's ids (31.. and 3001..9000)."""
    events: list[ChangeEvent] = []
    for n in range(per_kind):
        events.append(ChangeEvent("insert", n, 1_000_000 + n, f"fi{n}"))
        events.append(ChangeEvent("update", n, 1 + n, f"fu{n}"))
        events.append(ChangeEvent("delete", n, 9_001 + n, ""))
    return events


def visible_in_current(event: ChangeEvent, rows: dict[int, str]) -> bool:
    """`rows` is id -> customer of the ids asked about, as Bronze holds them now."""
    if event.kind == "delete":
        return event.row_id not in rows
    return rows.get(event.row_id) == event.marker


def visible_in_history(event: ChangeEvent, rows: set[tuple[int, str, str]]) -> bool:
    """`rows` are (id, __op, customer) of the ids asked about."""
    if event.kind == "delete":
        return any(r[0] == event.row_id and r[1] == "d" for r in rows)
    return (event.row_id, OP_OF[event.kind], event.marker) in rows


# The bulk step's key ranges (all on the seed table of 10,000 ids, after the freshness
# step has touched 1..30 and 9001..9030):
BULK_UPDATE_IDS = [*range(31, 3001), *range(9_031, 10_001)]  # 2,970 + 970 = 3,940 keys
BULK_DELETE_SEED_IDS = range(3_001, 9_001)  # 6,000 keys
BULK_INSERT_BASE = 2_000_000
BULK_DELETE_NEW = 4_800  # the first 4,800 of the bulk inserts are deleted again


def bulk_statements(table: str) -> list[str]:
    """The 108,000 changes of the catch-up step as autocommit statements, in order:
    21,600 inserts, 75,600 updates in passes over the update keys (each value unique
    per pass and key), 10,800 deletes (6,000 seed rows, 4,800 of the new rows), then one
    SENTINEL update, the very last change, whose arrival in Bronze marks the end of the
    catch-up. The sentinel is 1 more change than 108,000: it is the 108,001st and is
    counted separately, so the 108,000 are exact."""
    t = f"{SCHEMA}.{table}"
    stmts = [
        f"INSERT INTO {t} (id, customer, amount, status, updated_at) "
        f"SELECT {BULK_INSERT_BASE} + g, 'bi' || g, 7.25, 'bulk', now() FROM generate_series(1, {BULK_INSERTS}) g"
    ]
    full_passes, remainder = divmod(BULK_UPDATES, len(BULK_UPDATE_IDS))
    for p in range(full_passes):
        stmts.append(
            f"UPDATE {t} SET customer = 'bu{p}_' || id, updated_at = now() "
            f"WHERE id BETWEEN 31 AND 3000 OR id BETWEEN 9031 AND 10000"
        )
    stmts.append(
        f"UPDATE {t} SET customer = 'bur_' || id, updated_at = now() "
        f"WHERE id BETWEEN 31 AND {30 + remainder}"
    )
    stmts.append(f"DELETE FROM {t} WHERE id BETWEEN 3001 AND 9000")
    stmts.append(f"DELETE FROM {t} WHERE id BETWEEN {BULK_INSERT_BASE + 1} AND {BULK_INSERT_BASE + BULK_DELETE_NEW}")
    return stmts


SENTINEL_ID = 9_999
SENTINEL_VALUE = "SENTINEL"


def sentinel_statement(table: str) -> str:
    return f"UPDATE {SCHEMA}.{table} SET customer = '{SENTINEL_VALUE}', updated_at = now() WHERE id = {SENTINEL_ID}"


def compare_current(source: list[tuple[str, ...]], bronze: list[tuple[str, ...]]) -> dict[str, Any]:
    """Row-for-row comparison of two lists of (id, customer, amount, status). Reports
    what is missing from Bronze, what Bronze has that the source does not, and keys that
    appear more than once in Bronze. Equal only when all three are empty."""
    src = {r[0]: r for r in source}
    keys_b = [r[0] for r in bronze]
    dup = sorted({k for k in keys_b if keys_b.count(k) > 1}, key=int) if len(set(keys_b)) != len(keys_b) else []
    brz = {r[0]: r for r in bronze}
    missing = sorted(set(src) - set(brz), key=int)
    extra = sorted(set(brz) - set(src), key=int)
    different = sorted((k for k in set(src) & set(brz) if src[k] != brz[k]), key=int)
    return {
        "source_rows": len(source),
        "bronze_rows": len(bronze),
        "missing": len(missing),
        "extra": len(extra),
        "duplicated_keys": len(dup),
        "different": len(different),
        "equal": not (missing or extra or dup or different),
        "examples": {"missing": missing[:5], "extra": extra[:5], "duplicated": dup[:5], "different": different[:5]},
    }


def check_history(
    expected: list[tuple[str, str, str, str]], bronze: list[tuple[str, str, str, str, str]]
) -> dict[str, Any]:
    """`expected` are (id, op, customer, status) in source order (snapshot rows first,
    then `src4.audit` by seq). `bronze` are (id, op, customer, status, offset). Checks:
    same number of rows; the same multiset of events (so none missing and none twice);
    and for every key, the events in Kafka-offset order equal the source order."""
    from collections import Counter, defaultdict

    want = Counter(expected)
    got = Counter(r[:4] for r in bronze)
    missing = want - got
    extra = got - want
    by_key_src: dict[str, list[tuple[str, str, str]]] = defaultdict(list)
    for i, op, c, s in expected:
        by_key_src[i].append((op, c, s))
    by_key_brz: dict[str, list[tuple[str, str, str]]] = defaultdict(list)
    for i, op, c, s, _off in sorted(bronze, key=lambda r: int(r[4])):
        by_key_brz[i].append((op, c, s))
    out_of_order = sorted((k for k in by_key_src if by_key_src[k] != by_key_brz.get(k)), key=int)
    return {
        "expected_events": len(expected),
        "bronze_rows": len(bronze),
        "missing": sum(missing.values()),
        "extra_or_duplicated": sum(extra.values()),
        "keys_out_of_order": len(out_of_order),
        "equal": not (missing or extra or out_of_order),
        "examples": {"missing": list(missing)[:3], "extra": list(extra)[:3], "out_of_order": out_of_order[:5]},
    }


def connector_config(password: str) -> dict[str, str]:
    """Debezium's PostgreSQL connector as the applier expects its events: flattened by
    `ExtractNewRecordState`, deletes rewritten (the old row plus `__deleted=true`), the
    operation and source commit time added, JSON without schemas, decimals as text. The
    password is the CI stack's own throwaway one (see the override)."""
    return {
        "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
        "tasks.max": "1",
        "database.hostname": "postgres",
        "database.port": "5432",
        "database.user": PG_USER,
        "database.password": password,
        "database.dbname": PG_DB,
        "topic.prefix": TOPIC_PREFIX,
        "schema.include.list": SCHEMA,
        "table.include.list": f"{SCHEMA}.orders,{SCHEMA}.orders_history",
        "plugin.name": "pgoutput",
        "slot.name": "src4_slot",
        "publication.name": "src4_pub",
        "publication.autocreate.mode": "disabled",
        "snapshot.mode": "initial",
        "decimal.handling.mode": "string",
        "key.converter": "org.apache.kafka.connect.json.JsonConverter",
        "key.converter.schemas.enable": "false",
        "value.converter": "org.apache.kafka.connect.json.JsonConverter",
        "value.converter.schemas.enable": "false",
        "transforms": "unwrap",
        "transforms.unwrap.type": "io.debezium.transforms.ExtractNewRecordState",
        "transforms.unwrap.delete.handling.mode": "rewrite",
        "transforms.unwrap.add.fields": "op,source.ts_ms",
        "transforms.unwrap.drop.tombstones": "true",
    }


_BATCH = re.compile(r"CDC_BATCH target=(\w+) events=(\d+) seconds=([0-9.]+)")


def parse_batch_lines(messages: list[str]) -> list[dict[str, Any]]:
    out = []
    for message in messages:
        found = _BATCH.search(message)
        if found:
            out.append({"target": found.group(1), "events": int(found.group(2)), "seconds": float(found.group(3))})
    return out


def verdicts(result: dict[str, Any]) -> dict[str, Any]:
    """One verdict per target, from the measured numbers only. A target with no number is
    `not measured`, never `met`."""
    out: dict[str, Any] = {"freshness_p95_within_120s": {}, "correctness": {}, "catch_up_correctness": {}}
    for mode in MODES:
        phase = result.get("modes", {}).get(mode)
        if not phase:
            out["freshness_p95_within_120s"][mode] = "not measured"
            out["correctness"][mode] = "not measured"
            out["catch_up_correctness"][mode] = "not measured"
            continue
        fresh = phase.get("freshness", {})
        if not fresh:
            out["freshness_p95_within_120s"][mode] = "not measured"
        else:
            out["freshness_p95_within_120s"][mode] = (
                "met"
                if all(k in fresh and fresh[k].get("p95_s", math.inf) <= P95_TARGET_S and fresh[k].get("not_visible", 1) == 0 for k in KINDS)
                else "MISSED"
            )
        for name, key in (("correctness", "correctness"), ("catch_up_correctness", "catch_up")):
            check = phase.get(key, {}).get("check")
            out[name][mode] = "not measured" if check is None else ("met" if check["equal"] else "MISSED")
    return out


def all_met(v: dict[str, Any]) -> bool:
    return all(value == "met" for group in v.values() for value in group.values())


# ---------------------------------------------------------------------------
# Plumbing
# ---------------------------------------------------------------------------


def run(cmd: list[str], *, input_text: str | None = None, timeout: float = 600) -> str:
    result = subprocess.run(cmd, input=input_text, capture_output=True, text=True, timeout=timeout, check=False)
    if result.returncode != 0:
        raise HarnessFailure(f"{' '.join(cmd[:6])} failed ({result.returncode}): {result.stderr.strip()[-800:]}")
    return result.stdout


def psql(sql: str, *, timeout: float = 900) -> str:
    """One or more statements, each autocommitted, in one `psql` session."""
    cmd = [*COMPOSE, "exec", "-T", "postgres", "psql", "-U", PG_USER, "-d", PG_DB, "-v", "ON_ERROR_STOP=1", "-tqA", "-F", "\t", "-f", "-"]
    return run(cmd, input_text=sql, timeout=timeout).strip()


def ch(sql: str, *, timeout: float = 300) -> str:
    cmd = [*COMPOSE, "exec", "-T", "clickhouse", "clickhouse-client", "--multiquery"]
    return run(cmd, input_text=sql, timeout=timeout).strip()


def tsv(text: str) -> list[tuple[str, ...]]:
    return [tuple(line.split("\t")) for line in text.splitlines() if line]


def bronze_table(mode: str) -> str:
    return f"{QUERY_DB}.`bronze.{BRONZE[mode]}`"


def gql(query: str, variables: dict[str, Any] | None = None) -> dict[str, Any]:
    response = requests.post(DAGSTER_GRAPHQL, json={"query": query, "variables": variables or {}}, timeout=30)
    response.raise_for_status()
    body = response.json()
    if body.get("errors"):
        raise HarnessFailure(f"Dagster GraphQL error: {str(body['errors'])[:400]}")
    return body["data"]


def sensor_selector(name: str) -> dict[str, str]:
    return {"repositoryLocationName": REPO_LOCATION, "repositoryName": REPO_NAME, "sensorName": name}


def set_sensor(name: str, running: bool) -> None:
    data = gql(
        "query($s:SensorSelector!){ sensorOrError(sensorSelector:$s){ __typename ... on Sensor { jobOriginId sensorState { selectorId status } } ... on PythonError { message } } }",
        {"s": sensor_selector(name)},
    )["sensorOrError"]
    if data["__typename"] != "Sensor":
        raise HarnessFailure(f"sensor {name} not found: {data}")
    if running:
        res = gql(
            "mutation($s:SensorSelector!){ startSensor(sensorSelector:$s){ __typename ... on Sensor { sensorState { status } } ... on PythonError { message } } }",
            {"s": sensor_selector(name)},
        )["startSensor"]
        status = (res.get("sensorState") or {}).get("status")
    else:
        res = gql(
            "mutation($o:String!,$i:String!){ stopSensor(jobOriginId:$o, jobSelectorId:$i){ __typename ... on StopSensorMutationResult { instigationState { status } } ... on PythonError { message } } }",
            {"o": data["jobOriginId"], "i": data["sensorState"]["selectorId"]},
        )["stopSensor"]
        status = (res.get("instigationState") or {}).get("status")
    expected = "RUNNING" if running else "STOPPED"
    if status != expected:
        raise HarnessFailure(f"sensor {name}: expected {expected} after the call, got {status!r} ({res})")
    log(f"sensor {name} is {status}")


def connect(method: str, path: str, body: dict[str, Any] | None = None, expect: tuple[int, ...] = (200,)) -> Any:
    response = requests.request(method, f"{CONNECT_URL}{path}", json=body, timeout=30)
    if response.status_code not in expect:
        raise HarnessFailure(f"Kafka Connect {method} {path}: expected {expect}, got {response.status_code}: {response.text[:300]}")
    return response.json() if response.text else None


def connector_state() -> tuple[str, list[str]]:
    # 404 for a moment after the connector is created: its status is not written yet.
    status = connect("GET", f"/connectors/{CONNECTOR}/status", expect=(200, 404))
    if "connector" not in status:
        return "UNKNOWN", []
    return status["connector"]["state"], [t["state"] for t in status.get("tasks", [])]


def wait_connector(expected: str, timeout_s: float = 120) -> None:
    deadline = time.time() + timeout_s
    last: Any = None
    while time.time() < deadline:
        last = connector_state()
        if last[0] == expected and last[1] and all(t == expected for t in last[1]):
            log(f"connector {CONNECTOR} {expected}")
            return
        time.sleep(2)
    raise HarnessFailure(f"connector did not reach {expected} within {timeout_s}s; last state {last}")


def log(message: str) -> None:
    print(f"[src4] {time.strftime('%H:%M:%S')} {message}", flush=True)


class MemorySampler(threading.Thread):
    """`docker stats` of the broker, Kafka Connect and the applier's host every 5 s; keeps
    the maximum per container. A container that cannot be read is recorded as unread
    (None), never as 0."""

    def __init__(self, containers: dict[str, str], interval_s: float = 5.0) -> None:
        super().__init__(daemon=True)
        self.containers = containers
        self.interval_s = interval_s
        self.peak: dict[str, int | None] = {k: None for k in containers}
        self.samples: dict[str, int] = {k: 0 for k in containers}
        self._halt = threading.Event()

    def run(self) -> None:
        while not self._halt.is_set():
            try:
                out = run(["docker", "stats", "--no-stream", "--format", "{{.Name}}\t{{.MemUsage}}", *self.containers.values()], timeout=60)
                by_name = dict(line.split("\t") for line in out.splitlines() if "\t" in line)
                for key, name in self.containers.items():
                    if name in by_name:
                        value = parse_mem_usage(by_name[name])
                        self.samples[key] += 1
                        self.peak[key] = value if self.peak[key] is None else max(self.peak[key] or 0, value)
            except Exception as exc:  # noqa: BLE001 -- a missed sample is a missed sample, not a failure of the run
                log(f"memory sample failed: {str(exc)[:120]}")
            self._halt.wait(self.interval_s)

    def stop(self) -> None:
        self._halt.set()


# ---------------------------------------------------------------------------
# The measurement
# ---------------------------------------------------------------------------


def seed(table: str) -> None:
    """10,000 rows with the audit trigger off (the seed is the snapshot, not a change)."""
    t = f"{SCHEMA}.{table}"
    psql(
        f"ALTER TABLE {t} DISABLE TRIGGER audit;\n"
        f"INSERT INTO {t} SELECT g, 'seed_' || g, ((g % 1000) + 0.5)::numeric(12,2), 'seed', now() FROM generate_series(1, {SEED_ROWS}) g;\n"
        f"ALTER TABLE {t} ENABLE TRIGGER audit;\n"
    )
    count = int(psql(f"SELECT count(*) FROM {t};"))
    if count != SEED_ROWS:
        raise HarnessFailure(f"seed of {t}: expected {SEED_ROWS} rows, found {count}")


def bronze_count(mode: str) -> int | None:
    try:
        return int(ch(f"SELECT count() FROM {bronze_table(mode)} WHERE 1"))
    except HarnessFailure:
        return None  # the table does not exist yet


def wait_first_copy(mode: str, t_start: float) -> float:
    deadline = time.time() + FIRST_COPY_TIMEOUT_S
    while time.time() < deadline:
        if bronze_count(mode) == SEED_ROWS:
            elapsed = time.time() - t_start
            log(f"first copy of {mode}: {SEED_ROWS} rows in Bronze after {elapsed:.0f}s")
            return elapsed
        time.sleep(3)
    raise HarnessFailure(f"{mode}: the first copy ({SEED_ROWS} rows) did not reach Bronze in {FIRST_COPY_TIMEOUT_S}s; count {bronze_count(mode)}")


def source_rows(table: str) -> list[tuple[str, ...]]:
    return tsv(psql(f"SELECT id, customer, amount, status FROM {SCHEMA}.{table} ORDER BY id;"))


def bronze_current_rows(mode: str) -> list[tuple[str, ...]]:
    return tsv(ch(f"SELECT id, customer, amount, status FROM {bronze_table(mode)} WHERE 1 ORDER BY id FORMAT TSV"))


def expected_history(table: str) -> list[tuple[str, str, str, str]]:
    """Snapshot rows (op r, in the seed's form) then every audited change in commit order."""
    snapshot = [(str(g), "r", f"seed_{g}", "seed") for g in range(1, SEED_ROWS + 1)]
    audited = tsv(psql(f"SELECT id, op, coalesce(customer,''), coalesce(status,'') FROM {SCHEMA}.audit WHERE tbl = '{table}' ORDER BY seq;"))
    return snapshot + [tuple(r) for r in audited]  # type: ignore[misc]


def bronze_history_rows(mode: str) -> list[tuple[str, ...]]:
    return tsv(ch(f"SELECT id, `__op`, coalesce(customer,''), coalesce(status,''), `__offset` FROM {bronze_table(mode)} WHERE 1 FORMAT TSV"))


def verify(mode: str, table: str) -> dict[str, Any]:
    if mode == "current":
        return compare_current(source_rows(table), bronze_current_rows(mode))
    return check_history(expected_history(table), bronze_history_rows(mode))  # type: ignore[arg-type]


def poll_visible(mode: str, events: list[ChangeEvent]) -> tuple[set[int], float]:
    """Indices of the events visible in Bronze now, and the seconds the query took."""
    ids = ",".join(str(e.row_id) for e in events)
    started = time.time()
    if mode == "current":
        rows = {int(a): b for a, b in tsv(ch(f"SELECT id, customer FROM {bronze_table(mode)} WHERE id IN ({ids}) FORMAT TSV"))}
        seen = {i for i, e in enumerate(events) if visible_in_current(e, rows)}
    else:
        hist = {(int(a), b, c) for a, b, c in tsv(ch(f"SELECT id, `__op`, coalesce(customer,'') FROM {bronze_table(mode)} WHERE id IN ({ids}) FORMAT TSV"))}
        seen = {i for i, e in enumerate(events) if visible_in_history(e, hist)}
    return seen, time.time() - started


def measure_freshness(mode: str, table: str) -> dict[str, Any]:
    events = freshness_events()
    sent_at: dict[int, float] = {}
    seen_at: dict[int, float] = {}
    query_seconds: list[float] = []
    done_sending = threading.Event()
    failure: list[BaseException] = []

    def poller() -> None:
        deadline = time.time() + FRESH_PER_KIND * 3 * FRESH_GAP_S + FRESH_TIMEOUT_S
        try:
            while time.time() < deadline:
                if sent_at:
                    asked = [i for i in sorted(sent_at) if i not in seen_at]
                    if asked:
                        seen, took = poll_visible(mode, [events[i] for i in range(len(events))])
                        end = time.time()
                        query_seconds.append(took)
                        for i in seen:
                            if i in sent_at and i not in seen_at:
                                seen_at[i] = end
                if done_sending.is_set() and len(seen_at) == len(events):
                    return
                if done_sending.is_set() and time.time() > last_sent[0] + FRESH_TIMEOUT_S:
                    return
                time.sleep(0.5)
        except BaseException as exc:  # noqa: BLE001
            failure.append(exc)

    last_sent = [time.time()]
    thread = threading.Thread(target=poller, daemon=True)
    thread.start()
    for i, event in enumerate(events):
        tick = time.time()
        sent_at[i] = time.time()  # BEFORE the statement: the commit is after this instant
        psql(event.sql(table) + ";")
        last_sent[0] = time.time()
        time.sleep(max(0.0, FRESH_GAP_S - (time.time() - tick)))
    done_sending.set()
    thread.join(timeout=FRESH_TIMEOUT_S + 60)
    if failure:
        raise HarnessFailure(f"freshness poller failed: {failure[0]}")
    per_kind: dict[str, Any] = {}
    for kind in KINDS:
        idx = [i for i, e in enumerate(events) if e.kind == kind]
        seen_lat = [seen_at[i] - sent_at[i] for i in idx if i in seen_at]
        entry: dict[str, Any] = {"not_visible": len(idx) - len(seen_lat)}
        if seen_lat:
            entry.update(latency_stats(seen_lat))
        per_kind[kind] = entry
    resolution = round(percentile(query_seconds, 50), 2) if query_seconds else None
    return {**per_kind, "poll_query_median_s": resolution, "poll_queries": len(query_seconds)}


def sentinel_visible(mode: str) -> bool:
    out = ch(f"SELECT count() FROM {bronze_table(mode)} WHERE id = {SENTINEL_ID} AND customer = '{SENTINEL_VALUE}'")
    return int(out) >= 1


def measure_catch_up(mode: str, table: str) -> dict[str, Any]:
    wait_connector("RUNNING")
    set_sensor(SENSORS[mode], False)
    connect("PUT", f"/connectors/{CONNECTOR}/pause", expect=(200, 202))
    wait_connector("PAUSED")
    audit_before = int(psql(f"SELECT count(*) FROM {SCHEMA}.audit WHERE tbl = '{table}';"))
    t0 = time.time()
    psql(";\n".join(bulk_statements(table)) + ";", timeout=1800)
    wrote_s = time.time() - t0
    audited = int(psql(f"SELECT count(*) FROM {SCHEMA}.audit WHERE tbl = '{table}';")) - audit_before
    by_op = {op: int(n) for op, n in tsv(psql(f"SELECT op, count(*) FROM (SELECT op FROM {SCHEMA}.audit WHERE tbl='{table}' ORDER BY seq DESC LIMIT {audited}) a GROUP BY op;"))}
    if audited != BULK_TOTAL or by_op != {"u": BULK_UPDATES, "c": BULK_INSERTS, "d": BULK_DELETES}:
        raise HarnessFailure(f"the bulk step committed {audited} changes {by_op}, expected {BULK_TOTAL} (u {BULK_UPDATES}, c {BULK_INSERTS}, d {BULK_DELETES})")
    log(f"{mode}: {audited} changes committed at the source in {wrote_s:.0f}s; sentinel next")
    psql(sentinel_statement(table) + ";")
    resumed = time.time()
    connect("PUT", f"/connectors/{CONNECTOR}/resume", expect=(200, 202))
    wait_connector("RUNNING")
    set_sensor(SENSORS[mode], True)
    deadline = resumed + CATCH_UP_TIMEOUT_S
    caught_up: float | None = None
    while time.time() < deadline:
        if sentinel_visible(mode):
            caught_up = time.time() - resumed
            break
        time.sleep(5)
    if caught_up is None:
        raise HarnessFailure(f"{mode}: the last change did not reach Bronze within {CATCH_UP_TIMEOUT_S}s of resuming")
    log(f"{mode}: caught up {BULK_TOTAL} changes in {caught_up:.0f}s")
    return {"changes": audited, "by_op": by_op, "source_write_s": round(wrote_s, 1), "catch_up_s": round(caught_up, 1)}


CATCH_UP_TIMEOUT_S = CATCHUP_TIMEOUT_S


def applier_batches(since: float) -> list[dict[str, Any]]:
    """Batch sizes and times from the CDC_BATCH log lines of the runs started after `since`."""
    runs = gql(
        "query{ runsOrError(filter:{pipelineName:\"%s\"}, limit: 300){ __typename ... on Runs { results { runId status stats { __typename ... on RunStatsSnapshot { startTime endTime } } } } } }" % JOB
    )["runsOrError"]
    out: list[dict[str, Any]] = []
    for r in runs.get("results", []):
        start = (r.get("stats") or {}).get("startTime")
        if start is None or start < since:
            continue
        cursor = None
        while True:
            page = gql(
                "query($id:ID!,$c:String){ logsForRun(runId:$id, afterCursor:$c, limit: 500){ __typename ... on EventConnection { cursor hasMore events { ... on MessageEvent { message } } } } }",
                {"id": r["runId"], "c": cursor},
            )["logsForRun"]
            out.extend(parse_batch_lines([e.get("message", "") for e in page.get("events", [])]))
            if not page.get("hasMore"):
                break
            cursor = page["cursor"]
    return out


def batch_summary(batches: list[dict[str, Any]], target: str) -> dict[str, Any]:
    mine = [b for b in batches if b["target"] == target]
    if not mine:
        return {"batches": 0}
    secs = [b["seconds"] for b in mine]
    return {
        "batches": len(mine),
        "events": sum(b["events"] for b in mine),
        "seconds_total": round(sum(secs), 1),
        "seconds_median": round(percentile(secs, 50), 2),
        "seconds_max": round(max(secs), 2),
        "largest_batch_events": max(b["events"] for b in mine),
        "seconds_per_1000_events": round(sum(secs) / sum(b["events"] for b in mine) * 1000, 2),
    }


def run_mode(mode: str, first_copy_s: float) -> dict[str, Any]:
    table = TABLES[mode]
    phase_start = time.time()
    log(f"== {mode} mode, source table {SCHEMA}.{table}")
    out: dict[str, Any] = {"first_copy_s": round(first_copy_s, 1)}
    out["freshness"] = measure_freshness(mode, table)
    log(f"{mode} freshness: {json.dumps(out['freshness'])}")
    # All 90 must be in Bronze before the equality check; the freshness step only ends when they are or time ran out.
    check = verify(mode, table)
    out["correctness"] = {"check": check}
    log(f"{mode} correctness after the 90: {json.dumps(check)[:400]}")
    catch = measure_catch_up(mode, table)
    after = verify(mode, table)
    catch["check"] = after
    out["catch_up"] = catch
    log(f"{mode} correctness after catch-up: {json.dumps(after)[:400]}")
    out["applier"] = batch_summary(applier_batches(phase_start), mode)
    return out


def wait_stack_ready() -> dict[str, Any]:
    deadline = time.time() + 300
    last: Any = None
    while time.time() < deadline:
        try:
            info = connect("GET", "/")
            data = gql("query{ repositoriesOrError { __typename ... on RepositoryConnection { nodes { name sensors { name } } } } }")["repositoriesOrError"]
            names = {s["name"] for n in data.get("nodes", []) for s in n.get("sensors", [])}
            if set(SENSORS.values()) <= names:
                ch("SELECT 1")
                return info
            last = f"sensors seen: {sorted(names)}"
        except Exception as exc:  # noqa: BLE001
            last = str(exc)[:200]
        time.sleep(3)
    raise HarnessFailure(f"the stack was not ready in 300s: {last}")


def summary_block(result: dict[str, Any], v: dict[str, Any]) -> str:
    lines = ["GATE-SUMMARY SRC-4 slice 0", f"kafka connect: {result.get('kafka_connect')}"]
    for mode in MODES:
        phase = result.get("modes", {}).get(mode, {})
        lines.append(f"-- {mode}")
        for kind in KINDS:
            f = phase.get("freshness", {}).get(kind, {})
            lines.append(f"   {kind:7s} p50 {f.get('p50_s')}  p95 {f.get('p95_s')}  max {f.get('max_s')}  not visible {f.get('not_visible')}")
        lines.append(f"   catch-up {phase.get('catch_up', {}).get('catch_up_s')} s for {BULK_TOTAL} changes; first copy {phase.get('first_copy_s')} s")
        lines.append(f"   applier {json.dumps(phase.get('applier'))}")
    lines.append(f"memory peak (bytes): {json.dumps(result.get('memory_peak_bytes'))}")
    lines.append(f"verdicts: {json.dumps(v)}")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default="src4-result.json")
    args = parser.parse_args()
    password = os.environ.get("POSTGRES_PASSWORD", "")
    if not password:
        raise SystemExit("POSTGRES_PASSWORD must be set (the CI stack's own value)")
    result: dict[str, Any] = {"started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "modes": {}}
    sampler = MemorySampler(CONTAINERS)
    failure: str | None = None
    try:
        result["kafka_connect"] = wait_stack_ready()
        log(f"Kafka Connect: {result['kafka_connect']}")
        sampler.start()
        for mode in MODES:
            seed(TABLES[mode])
        t_register = time.time()
        connect("POST", "/connectors", {"name": CONNECTOR, "config": connector_config(password)}, expect=(201,))
        wait_connector("RUNNING")
        first = {mode: wait_first_copy(mode, t_register) for mode in MODES}
        for mode in MODES:
            result["modes"][mode] = run_mode(mode, first[mode])
    except Exception as exc:  # noqa: BLE001 -- recorded in the result, then re-raised through the exit status
        failure = f"{type(exc).__name__}: {str(exc)[:600]}"
        log(f"FAILED: {failure}")
    finally:
        sampler.stop()
        result["memory_peak_bytes"] = sampler.peak
        result["memory_samples"] = sampler.samples
    result["failure"] = failure
    v = verdicts(result)
    result["verdicts"] = v
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2, default=str)
    print(summary_block(result, v), flush=True)
    return 0 if failure is None and all_met(v) else 1


if __name__ == "__main__":
    sys.exit(main())
