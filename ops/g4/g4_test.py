#!/usr/bin/env python3
"""G4 acceptance test: PostgreSQL change capture on `Kafka Connect`
(`SRC-4` slice 2, decision S2-j; ADR 0016).

The old version of this gate drove the fixed `debezium-server` demo pipeline.
That pipeline is gone. This one drives the product path end to end, through
the API a person uses:

1. The source is a schema of the stack's own `postgres:16` with TWO identical
   tables (`orders` in "current row per key" mode, `orders_history` in "history"
   mode), 20,000 seeded rows each, `REPLICA IDENTITY FULL`, one publication.
2. The connector is registered as the console does it (`POST /api/connectors`
   with a `file:` credential, `PUT .../ingest-spec` with the `cdc` adapter and
   the two tables). That PUT provisions `Kafka Connect`; nothing else is started
   by hand.
3. Steps, in order, each of which fails the gate by name:
   - the connector's `Kafka Connect` config does not hold the source password
     (only the `${dir:...}` reference), and the password file exists on the
     memory-only volume;
   - the first copy reaches Bronze: 20,000 rows in each table (step `load`);
   - an insert, an update and a delete each become visible in Bronze, in both
     modes, within 120 seconds (step `reload`; decision D5);
   - pause through `POST .../pause`, write 20,000 changes to each table, resume
     through `POST .../resume`: Bronze equals the source, nothing missing or
     twice (current mode: same rows; history mode: the same multiset of events);
   - restart the `kafka-connect` container during a stream of changes: Bronze
     equals the source again. The runner has no Docker access (and gets none,
     see `docker-compose.yml`), so it prints the marker line
     `G4-RESTART-KAFKA-CONNECT` and waits for the file
     `/tmp/g4-kafka-connect-restarted`; the CI job (`g4-cdc` in
     `.github/workflows/ci.yml`) watches the log, restarts the container and
     creates the file with `docker exec`;
   - a source schema change is observed: a new column reaches the Bronze table
     and `GET .../schema-changes` lists it;
   - the WAL-breach alert and the OIDC round trip, unchanged from before;
   - `DELETE /api/connectors/{id}` removes the `Kafka Connect` connector, the
     replication slot, the publication and the password file.

# Why the sources of truth are the ones they are

Bronze is read through ClickHouse (`DataLakeCatalog` over Lakekeeper), the way
the product reads it; the source is read with `psql`. Neither side is asked to
describe itself: this gate compares rows with rows. In history mode the
expected events are known to the gate because it issues every statement, so the
comparison is of two multisets of (id, operation, customer), not of the pipeline
with itself.

# SRC-9: the four steps of the connector gates

This gate is also the connector gate of `PostgreSQL CDC` (`ops/gates/
coverage.json`, gate `g4`), and records its four steps through
`ops/gates/gate_summary.py`, whose table it prints at the end and the job
copies into its summary: `connect` and `discover` (the demo source is
registered and `POST .../test` and the discovery routes answer for real; a
failed one is recorded and the gate goes on), `load` (the first copy) and
`reload` (the three kinds of change within the budget).

# Why `count() WHERE id > 0`, never a bare `count()`

`docs/plans/P5-RESULT.md`'s (A) finding: ClickHouse 26.3's bare
`count()`/`count(*)`/`count(<col>)` over a merge-on-read Iceberg table with
equality deletes takes a metadata-only fast path that does NOT apply
delete filtering and overcounts. Every row-count check in this test uses a
`WHERE` predicate (forcing the correct, row-scanning path).
"""

from __future__ import annotations

import collections
import decimal
import os
import subprocess
import sys
import threading
import time
import urllib.parse

import requests

# SRC-9 plan task 6: the coverage registry and the result table, mounted at
# /opt/gates by `g4-test-runner` in docker-compose.yml (PYTHONPATH).
import gate_summary

CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
LAKEKEEPER_CATALOG_URI = os.environ.get("CH_LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog")
RUSTFS_S3_ENDPOINT = os.environ.get("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000")
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
# R1 (ADR 0011): see `ops/g3a/g3a_test.py`'s identical comment —
# `catalog_credential` needs the OAuth2 `client_id:client_secret` form,
# pointed at `ops/oidc-mock`'s `/token` endpoint. Empty on a pre-R1 or
# authz-disabled stack.
CH_OAUTH_CLIENT_ID = os.environ.get("CH_OAUTH_CLIENT_ID", "")
CH_OAUTH_SERVER_URI = os.environ.get("CH_OAUTH_SERVER_URI", "")

# Duplicated from ops/g3a/g3a_test.py's step_login/API session — g4_test.py
# has no shared package with g3a_test.py to import this from (each g*
# gate script is a standalone entrypoint), matching this file's existing
# precedent of duplicating ch_query/_wait_for from the same source.
API_URL = os.environ.get("API_URL", "http://lakehouse-api:8089")
# oidc-mock's /authorize + /token endpoints that
# step_oidc_authorization_code_round_trip drives lakehouse-api's
# /api/auth/oidc/{start,callback} against. Mirrors this file's existing
# `API_URL` env-var-with-default pattern, not a new style.
OIDC_MOCK_URL = os.environ.get("OIDC_MOCK_URL", "http://oidc-mock:8090")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "admin@example.invalid")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "changeme")
ALERTS_RUN_TOKEN = os.environ.get("ALERTS_RUN_TOKEN", "")

API = requests.Session()

def step_login() -> None:
    """Every `/api/*` route this step calls (`/api/alerts`, `/api/overview/
    alerts`, `/api/overview/alerts/{id}/silence`) is `Policy::RequiresAuth`
    or stricter (`policy.rs`, confirmed) — log in as the bootstrap admin
    and let `API` carry the session cookie, exactly as
    `ops/g3a/g3a_test.py::step_login` already does for the identical
    reason."""
    login = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
    if not login.ok:
        raise G4Failure(f"login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = API.post(f"{API_URL}/api/auth/change-password", json={"newPassword": AUTH_PASSWORD}, timeout=10)
        if not rotated.ok:
            raise G4Failure(f"forced password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
        if not relogin.ok:
            raise G4Failure(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")
    print("[g4] logged in as bootstrap admin")


WAL_ALERT_RULE_ID = "al_g4_p5cdc_wal"


def step_seed_wal_alert_rule() -> None:
    """An ordinary `AlertKind::Alert` rule against `serving.
    replication_slot_health` — WAL-slot health needs no new alert kind
    (WS5 plan review Y4); `replication_metrics.py`'s own module doc names
    this exact rule shape (`mart = replication_slot_health`, `measure =
    unhealthy`). `console.alert_rule` is a `ReplacingMergeTree` keyed on
    `id` (`lakehouse-alerts::ensure`), so re-`PUT`ing the same id every
    run is a correct, idempotent upsert, not a conditional "if missing"
    check. `PUT`, not `POST`: `POST /api/alerts` ignores a caller's `id`
    and mints a new `al_<hex>` every time (`lakehouse_alerts::save_rule`
    with `id: None`), so each run added another rule under an id this gate
    never looked for."""
    resp = API.put(
        f"{API_URL}/api/alerts",
        json={
            "id": WAL_ALERT_RULE_ID,
            "name": "g4 WAL slot health",
            "type": "alert",
            "mart": "replication_slot_health",
            "measure": "unhealthy",
            "agg": "max",
            "op": ">",
            "threshold": 0,
            "channel": "webhook",
            # A sink nothing listens on. It was `http://127.0.0.1:1/...`, which
            # SEC-10 now refuses when the rule is saved (loopback can never be
            # allow-listed, by design). 192.0.2.0/24 is TEST-NET-1, reserved
            # for documentation and never routed, so this is still a dead
            # target, but one the address check admits: the delivery fails
            # at connect, as it did before.
            "target": "http://192.0.2.1:9/g4-webhook-sink",
            "severity": "critical",
        },
        timeout=10,
    )
    if not resp.ok:
        raise G4Failure(f"failed to seed the WAL alert rule: {resp.status_code} {resp.text}")
    print("[g4] seeded WAL alert rule (idempotent upsert)")


def ch_auth_settings() -> str:
    if not CH_OAUTH_CLIENT_ID:
        return ""
    return (
        f", catalog_credential = '{CH_OAUTH_CLIENT_ID}:unused', "
        f"oauth_server_uri = '{CH_OAUTH_SERVER_URI}'"
    )


CATALOG_DB = "icecat_g4"

# The freshness budget of `SRC-4` decision D5: each kind of change visible in
# Bronze within two minutes. Measured p95 was 18 to 40 s in slice 0
# (`docs/plans/SRC-4-RESULT.md`); the sensor's 30 s interval dominates.
FRESHNESS_BUDGET_SECONDS = 120.0
# How long a backlog (20,000 changes per table after a pause, or a worker
# restart) may take to reach Bronze. Slice 0 measured 108,000 changes in about
# 100 s; this is the job's whole budget, not a target.
CATCH_UP_TIMEOUT_SECONDS = 600.0
FIRST_COPY_TIMEOUT_SECONDS = 600.0
SEED_ROWS = 20_000


class G4Failure(Exception):
    pass


# SRC-9 task 6. The registry type this gate stands for, and the recorder the
# four steps write into (created by `main`).
CDC_TYPE = "PostgreSQL CDC"
RECORDER: gate_summary.Recorder | None = None

# The demo source as the registry holds it: the database and role this stack's
# Postgres really has (`PG*` is what `psql` below uses), a schema of its own, two
# tables (one per load mode) and the slot and publication names the connector
# will use (`replication_metrics` attributes the slot to a connector by the
# registry row).
GATE_SECRETS_DIR = "/gate-secrets"
FILE_REF_PREFIX = "file:/run/secrets/connector_"
GATE_TENANT_SLUG = "meridian-group"
CDC_SLOT_NAME = "g4_slot"
CDC_PUBLICATION_NAME = "g4_pub"
CDC_SCHEMA = "g4_cdc"
MODES = ("current", "history")
TABLES = {"current": "orders", "history": "orders_history"}
OBJECTS = {mode: f"{CDC_SCHEMA}.{TABLES[mode]}" for mode in MODES}
TARGETS = {"current": "g4_orders_current", "history": "g4_orders_history"}
LOAD_MODES = {"current": "incremental_dedup", "history": "history"}
# What the discovery of the schema must list and preview.
CDC_DISCOVER_TABLE = OBJECTS["current"]
CDC_DISCOVER_COLUMNS = {"id", "customer", "amount", "status", "updated_at"}

# Kafka Connect, from inside the compose network (the runner is the only thing
# besides `lakehouse-api` that dials it, and only to read: the gate asserts that
# the connector's stored config holds no password).
CONNECT_URL = os.environ.get("KAFKA_CONNECT_URL", "http://kafka-connect:8083").rstrip("/")
# `lakehouse_cdc_secrets` mounted read-only in the runner by the gate's override
# (`ops/g4/docker-compose.g4.override.yml`): the only reason this gate can look
# at the password file. Production mounts it in `lakehouse-api` and
# `kafka-connect` only.
CDC_SECRETS_DIR = "/cdc-secrets"
# The handshake with the CI job, which alone can restart a container.
RESTART_MARKER = "G4-RESTART-KAFKA-CONNECT"
RESTART_DONE_FILE = "/tmp/g4-kafka-connect-restarted"
RESTART_WAIT_SECONDS = 300.0

# What the history table must hold at any moment: every event the gate caused,
# keyed (id, operation, customer). The snapshot rows are added when the source
# is seeded; every later statement adds what it changes.
EXPECTED_HISTORY: collections.Counter[tuple[int, str, str]] = collections.Counter()


def _record(step: str, outcome: str, detail: str = "") -> None:
    if RECORDER is None:
        raise G4Failure("internal error: the gate summary recorder was not created")
    RECORDER.record(CDC_TYPE, step, outcome, detail)
    print(f"[g4] {CDC_TYPE} {step}: {outcome}{' (' + detail + ')' if detail else ''}", flush=True)


def ch_query(sql: str) -> str:
    resp = requests.post(CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=60)
    if not resp.ok:
        raise G4Failure(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


def ch_rows(sql: str) -> list[tuple[str, ...]]:
    text = ch_query(f"{sql} SETTINGS allow_database_iceberg=1 FORMAT TSV")
    return [tuple(line.split("\t")) for line in text.splitlines() if line]


def pg_exec(sql: str) -> str:
    result = subprocess.run(
        ["psql", "-v", "ON_ERROR_STOP=1", "-tqA", "-c", sql],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise G4Failure(f"psql failed: {result.stderr}\nSQL: {sql}")
    return result.stdout.strip()


def pg_rows(sql: str) -> list[tuple[str, ...]]:
    result = subprocess.run(
        ["psql", "-v", "ON_ERROR_STOP=1", "-tqA", "-F", "\t", "-c", sql],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise G4Failure(f"psql failed: {result.stderr}\nSQL: {sql}")
    return [tuple(line.split("\t")) for line in result.stdout.splitlines() if line]


def bronze(mode: str) -> str:
    return f"{CATALOG_DB}.`bronze.{TARGETS[mode]}`"


def _wait_for(name: str, check, timeout_s: float, interval_s: float = 1.0):
    deadline = time.time() + timeout_s
    last_err = None
    while time.time() < deadline:
        try:
            value = check()
            if value is not None:
                elapsed = timeout_s - (deadline - time.time())
                print(f"[g4] {name}: done after {elapsed:.1f}s", flush=True)
                return value
        except Exception as exc:  # noqa: BLE001
            last_err = exc
        time.sleep(interval_s)
    raise G4Failure(f"timed out after {timeout_s}s waiting for {name}: {last_err}")


def _dagster_job_loaded(job: str):
    """True once a loaded Dagster repository lists `job`; None otherwise
    (`_wait_for` keeps polling on None)."""
    query = "{ repositoriesOrError { ... on RepositoryConnection { nodes { jobs { name } } } } }"
    resp = requests.post(DAGSTER_URL, json={"query": query}, timeout=5)
    resp.raise_for_status()
    nodes = (resp.json().get("data") or {}).get("repositoriesOrError", {}).get("nodes") or []
    return True if any(j.get("name") == job for n in nodes for j in n.get("jobs", [])) else None


def _dagster_run_finished(run_id: str):
    """True once the run succeeded; raises if it failed; None while it is
    still queued or running (`_wait_for` keeps polling on None)."""
    query = "query($id:ID!){ runOrError(runId:$id){ ... on Run { status } } }"
    resp = requests.post(DAGSTER_URL, json={"query": query, "variables": {"id": run_id}}, timeout=5)
    resp.raise_for_status()
    status = ((resp.json().get("data") or {}).get("runOrError") or {}).get("status")
    if status == "SUCCESS":
        return True
    if status in ("FAILURE", "CANCELED"):
        raise G4Failure(f"replication_slot_check_job run {run_id} ended {status}")
    return None



def step_wait_for_services() -> None:
    _wait_for("ClickHouse", lambda: ch_query("SELECT 1 FORMAT TabSeparated").strip() == "1" or None, 60)
    pg_exec("SELECT 1;")

    def connect_up():
        resp = requests.get(f"{CONNECT_URL}/", timeout=5)
        return True if resp.ok else None

    _wait_for("Kafka Connect's REST API", connect_up, 180, interval_s=3)
    print("[g4] ClickHouse, Postgres and Kafka Connect are reachable", flush=True)


def step_seed_source() -> None:
    """The gate's own source (it replaces `g4-source-init`'s fixed seed): a schema
    of its own, two identical tables (`REPLICA IDENTITY FULL`, so a delete event
    carries the old row), 20,000 seeded rows each, a publication scoped to those
    two tables (never `FOR ALL TABLES`), and a table outside the publication that
    the WAL-breach step writes to."""
    pg_exec(f"DROP SCHEMA IF EXISTS {CDC_SCHEMA} CASCADE; CREATE SCHEMA {CDC_SCHEMA};")
    for table in TABLES.values():
        pg_exec(
            f"CREATE TABLE {CDC_SCHEMA}.{table} (id bigint PRIMARY KEY, customer text, "
            "amount numeric(12,2), status text, updated_at timestamptz); "
            f"ALTER TABLE {CDC_SCHEMA}.{table} REPLICA IDENTITY FULL; "
            f"INSERT INTO {CDC_SCHEMA}.{table} SELECT g, 'seed_' || g, ((g % 1000) + 0.5)::numeric(12,2), "
            f"'seed', now() FROM generate_series(1, {SEED_ROWS}) g;"
        )
    pg_exec(
        f"CREATE TABLE {CDC_SCHEMA}.wal_noise (id int PRIMARY KEY, v int NOT NULL); "
        f"ALTER TABLE {CDC_SCHEMA}.wal_noise REPLICA IDENTITY FULL; "
        f"INSERT INTO {CDC_SCHEMA}.wal_noise SELECT g, 0 FROM generate_series(1, 5) g;"
    )
    pg_exec(
        f"DROP PUBLICATION IF EXISTS {CDC_PUBLICATION_NAME}; "
        f"CREATE PUBLICATION {CDC_PUBLICATION_NAME} FOR TABLE "
        + ", ".join(f"{CDC_SCHEMA}.{table}" for table in TABLES.values())
        + ";"
    )
    for g in range(1, SEED_ROWS + 1):
        EXPECTED_HISTORY[(g, "r", f"seed_{g}")] += 1
    print(f"[g4] seeded {SEED_ROWS} rows into each of {', '.join(OBJECTS.values())}", flush=True)


def step_create_catalog_database() -> None:
    ch_query(
        f"CREATE DATABASE IF NOT EXISTS {CATALOG_DB} "
        f"ENGINE = DataLakeCatalog('{LAKEKEEPER_CATALOG_URI}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{LAKEKEEPER_WAREHOUSE}', "
        f"storage_endpoint = '{RUSTFS_S3_ENDPOINT}'{ch_auth_settings()} "
        "SETTINGS allow_database_iceberg = 1"
    )


# ── The connector, as the console registers it ──────────────────────────────


def _ingest_spec_body() -> dict:
    return {
        "adapter": "cdc", "ingestMode": "cdc",
        "dial": {
            "driver": "postgres", "host": os.environ["PGHOST"], "port": int(os.environ["PGPORT"]),
            "database": os.environ["PGDATABASE"], "user": os.environ["PGUSER"],
            "slotName": CDC_SLOT_NAME, "publicationName": CDC_PUBLICATION_NAME,
        },
        "sourceObjects": [
            {"name": OBJECTS[mode], "target": TARGETS[mode], "loadMode": LOAD_MODES[mode], "primaryKey": ["id"]}
            for mode in MODES
        ],
    }


def _connect_name(connector_id: str) -> str:
    """`SRC-4` decision S2-b: the worker's name for a registry connector."""
    return f"lakehouse-{connector_id}"


def _capture_state(connector_id: str) -> str | None:
    resp = API.get(f"{API_URL}/api/connectors/{connector_id}/change-capture", timeout=15)
    if not resp.ok:
        raise G4Failure(f"GET .../change-capture answered {resp.status_code} {resp.text[:200]}")
    body = resp.json()
    if body.get("supported") is not True:
        raise G4Failure(f"change capture reported unsupported: {body.get('reason')}")
    return body.get("state")


def _wait_capture_state(connector_id: str, expected: str, timeout_s: float) -> None:
    def check():
        return True if _capture_state(connector_id) == expected else None

    _wait_for(f"the connector is {expected} in Kafka Connect", check, timeout_s, interval_s=3)


def step_connector_holds_no_password(connector_id: str) -> None:
    """`SRC-4` decision S2-a, proved on the running stack: Kafka Connect stores and returns
    the connector's config, and the source password must not be in it; the connector names
    a file instead, and that file (memory-only volume) holds the value."""
    resp = requests.get(f"{CONNECT_URL}/connectors/{_connect_name(connector_id)}/config", timeout=15)
    if not resp.ok:
        raise G4Failure(f"GET the connector's config from Kafka Connect answered {resp.status_code}")
    config_text = resp.text
    password = os.environ["PGPASSWORD"]
    if password in config_text:
        raise G4Failure("the source password is in the connector's Kafka Connect config")
    reference = "${dir:/run/cdc-secrets:" + connector_id + "}"
    if reference not in config_text:
        raise G4Failure(f"the connector's config does not name its password file ({reference}); got keys {sorted(resp.json())}")
    path = os.path.join(CDC_SECRETS_DIR, connector_id)
    try:
        with open(path, encoding="utf-8") as fh:
            stored = fh.read()
    except OSError as exc:
        raise G4Failure(f"the password file {path} cannot be read: {exc}") from exc
    if stored != password:
        raise G4Failure(f"the password file {path} does not hold the source password")
    print("[g4] the connector's Kafka Connect config holds only a file reference; the file holds the password", flush=True)


# ── Comparing Bronze with the source ────────────────────────────────────────


def _current_diff() -> str | None:
    """None when the current-mode Bronze table holds exactly the source's rows (id, customer,
    amount, status), else what differs. A bare `count()` is never used (see the module doc)."""
    def norm(row):
        return (int(row[0]), row[1], decimal.Decimal(row[2]), row[3])

    source = [norm(r) for r in pg_rows(f"SELECT id, customer, amount, status FROM {OBJECTS['current']} ORDER BY id")]
    found = [norm(r) for r in ch_rows(f"SELECT id, customer, amount, status FROM {bronze('current')} WHERE id > 0 ORDER BY id")]
    found_ids = [r[0] for r in found]
    duplicated = len(found_ids) - len(set(found_ids))
    source_ids = {r[0] for r in source}
    missing = source_ids - set(found_ids)
    extra = set(found_ids) - source_ids
    different = len({r for r in set(source) - set(found) if r[0] in set(found_ids)})
    if not (missing or extra or duplicated or different) and len(source) == len(found):
        return None
    return (
        f"current: source {len(source)} rows, Bronze {len(found)}; missing {len(missing)}, extra {len(extra)}, "
        f"duplicated {duplicated}, different {different}"
    )


def _history_diff() -> str | None:
    """None when the history table holds exactly the events the gate caused, as a multiset."""
    found: collections.Counter = collections.Counter()
    for row in ch_rows(f"SELECT id, `_op`, coalesce(customer, '') FROM {bronze('history')} WHERE id > 0"):
        found[(int(row[0]), row[1], row[2])] += 1
    missing = EXPECTED_HISTORY - found
    extra = found - EXPECTED_HISTORY
    if not missing and not extra:
        return None
    return (
        f"history: expected {sum(EXPECTED_HISTORY.values())} events, Bronze {sum(found.values())}; "
        f"missing {sum(missing.values())} (e.g. {list(missing)[:3]}), twice or unexpected {sum(extra.values())} (e.g. {list(extra)[:3]})"
    )


def _wait_equal(label: str, timeout_s: float) -> None:
    def check():
        problems = [d for d in (_current_diff(), _history_diff()) if d]
        if problems:
            raise G4Failure("; ".join(problems))
        return True

    _wait_for(f"Bronze equals the source ({label})", check, timeout_s, interval_s=10)


def step_first_copy_reached_bronze() -> None:
    """SRC-9 task 6, step `load`: the connector's first copy delivers every seeded row to the
    Bronze table of each mode. `WHERE`, never a bare `count()` (R11)."""
    def table_registered():
        text = ch_query(f"SHOW TABLES FROM {CATALOG_DB} SETTINGS allow_database_iceberg=1 FORMAT TabSeparated")
        return True if all(f"bronze.{TARGETS[m]}" in text for m in MODES) else None

    def all_seeded_rows():
        counts = {}
        for mode in MODES:
            extra = " AND `_op` = 'r'" if mode == "history" else ""
            text = ch_query(
                f"SELECT count() FROM {bronze(mode)} WHERE id > 0{extra} SETTINGS allow_database_iceberg=1 FORMAT TabSeparated"
            ).strip()
            counts[mode] = int(text) if text.isdigit() else -1
        if any(c != SEED_ROWS for c in counts.values()):
            raise G4Failure(f"first copy so far: {counts}")
        return counts

    try:
        _wait_for("both Bronze tables registered in Lakekeeper", table_registered, FIRST_COPY_TIMEOUT_SECONDS, interval_s=5)
        counts = _wait_for(f"the first copy holds the {SEED_ROWS} seeded rows of each table", all_seeded_rows, FIRST_COPY_TIMEOUT_SECONDS, interval_s=5)
    except G4Failure as exc:
        _record("load", "fail", str(exc))
        raise
    _record("load", "pass", f"{counts['current']} + {counts['history']} rows in Bronze after the first copy")


# ── Changes ─────────────────────────────────────────────────────────────────


def _history_events(ids, op: str, customer) -> None:
    for i in ids:
        EXPECTED_HISTORY[(i, op, customer(i) if callable(customer) else customer)] += 1


def _apply(sql_by_mode, history_events=None) -> None:
    """Run one statement per table (the same change on both) and record what the history table
    must come to hold."""
    for mode in MODES:
        pg_exec(sql_by_mode(f"{CDC_SCHEMA}.{TABLES[mode]}"))
    if history_events:
        history_events()


def _visible(mode: str, kind: str, row_id: int, marker: str) -> bool:
    if mode == "current":
        rows = ch_rows(f"SELECT customer FROM {bronze('current')} WHERE id = {row_id}")
        return (not rows) if kind == "delete" else any(r[0] == marker for r in rows)
    op = {"insert": "c", "update": "u", "delete": "d"}[kind]
    extra = "" if kind == "delete" else f" AND customer = '{marker}'"
    n = ch_rows(f"SELECT count() FROM {bronze('history')} WHERE id = {row_id} AND `_op` = '{op}'{extra}")
    return int(n[0][0]) > 0


def step_changes_visible_within_budget() -> None:
    """SRC-9 task 6, step `reload`, and `SRC-4` decision D5: an insert, an update and a
    delete each visible in Bronze, in both modes, within 120 seconds."""
    new_id, upd_id, del_id = 900_001, 1, SEED_ROWS
    changes = [
        ("insert", new_id, "fresh_i",
         lambda t: f"INSERT INTO {t} (id, customer, amount, status, updated_at) VALUES ({new_id}, 'fresh_i', 5.00, 'fresh', now())",
         lambda: _history_events([new_id], "c", "fresh_i")),
        ("update", upd_id, "fresh_u",
         lambda t: f"UPDATE {t} SET customer = 'fresh_u', updated_at = now() WHERE id = {upd_id}",
         lambda: _history_events([upd_id], "u", "fresh_u")),
        ("delete", del_id, "",
         lambda t: f"DELETE FROM {t} WHERE id = {del_id}",
         lambda: _history_events([del_id], "d", f"seed_{del_id}")),
    ]
    latencies = []
    try:
        for kind, row_id, marker, sql, events in changes:
            t0 = time.time()
            _apply(sql, events)
            for mode in MODES:
                _wait_for(
                    f"{kind} of id {row_id} in the {mode} table", lambda m=mode, k=kind, i=row_id, mk=marker: True if _visible(m, k, i, mk) else None,
                    FRESHNESS_BUDGET_SECONDS, interval_s=2,
                )
                latencies.append(f"{kind}/{mode} {time.time() - t0:.0f}s")
    except G4Failure as exc:
        _record("reload", "fail", str(exc))
        raise
    _record("reload", "pass", f"each kind of change visible within {FRESHNESS_BUDGET_SECONDS:.0f}s in both modes ({', '.join(latencies)})")


def _bulk_changes() -> None:
    """20,000 changes per table: 10,000 updates, 5,000 deletes, 5,000 inserts."""
    _apply(
        lambda t: f"UPDATE {t} SET customer = 'bulk_u', updated_at = now() WHERE id BETWEEN 1 AND 10000",
        lambda: _history_events(range(1, 10_001), "u", "bulk_u"),
    )
    _apply(
        lambda t: f"DELETE FROM {t} WHERE id BETWEEN 10001 AND 15000",
        lambda: _history_events(range(10_001, 15_001), "d", lambda i: f"seed_{i}"),
    )
    _apply(
        lambda t: f"INSERT INTO {t} SELECT g, 'bulk_i', 5.00, 'bulk', now() FROM generate_series(100001, 105000) g",
        lambda: _history_events(range(100_001, 105_001), "c", "bulk_i"),
    )


def step_pause_and_resume_lose_nothing(connector_id: str) -> None:
    """Pause through the new route, write 20,000 changes to each table while the connector is
    paused, resume through the route: Bronze equals the source, nothing missing or twice."""
    paused = API.post(f"{API_URL}/api/connectors/{connector_id}/pause", timeout=30)
    if not paused.ok:
        raise G4Failure(f"POST .../pause answered {paused.status_code} {paused.text[:200]}")
    _wait_capture_state(connector_id, "paused", 90)
    _bulk_changes()
    print("[g4] 20,000 changes per table written while the connector is paused", flush=True)
    resumed = API.post(f"{API_URL}/api/connectors/{connector_id}/resume", timeout=30)
    if not resumed.ok:
        raise G4Failure(f"POST .../resume answered {resumed.status_code} {resumed.text[:200]}")
    _wait_capture_state(connector_id, "running", 90)
    _wait_equal("after a pause and a resume", CATCH_UP_TIMEOUT_SECONDS)


def step_restart_kafka_connect_during_changes(connector_id: str) -> None:
    """Restart the worker while changes are being written. The runner cannot restart a
    container; the CI job does (see the module doc): the runner prints `RESTART_MARKER` and
    waits for `RESTART_DONE_FILE`. The writer keeps writing across the restart, so some
    changes are made while the worker is down."""
    if os.path.exists(RESTART_DONE_FILE):
        os.remove(RESTART_DONE_FILE)
    started = threading.Event()
    errors: list[BaseException] = []

    def writer() -> None:
        try:
            for k in range(18):
                lo = 15_001 + 250 * k
                hi = lo + 249
                _apply(
                    lambda t, lo=lo, hi=hi, k=k: f"UPDATE {t} SET customer = 'stream_{k}', updated_at = now() WHERE id BETWEEN {lo} AND {hi}",
                    lambda lo=lo, hi=hi, k=k: _history_events(range(lo, hi + 1), "u", f"stream_{k}"),
                )
                if k == 4:
                    started.set()
                time.sleep(1.0)
        except BaseException as exc:  # noqa: BLE001
            errors.append(exc)
            started.set()

    thread = threading.Thread(target=writer, daemon=True)
    thread.start()
    started.wait(120)
    print(RESTART_MARKER, flush=True)
    _wait_for(
        "the CI job restarted kafka-connect",
        lambda: True if os.path.exists(RESTART_DONE_FILE) else None,
        RESTART_WAIT_SECONDS,
        interval_s=2,
    )
    thread.join()
    if errors:
        raise G4Failure(f"the change writer failed: {errors[0]}")
    _wait_capture_state(connector_id, "running", 240)
    _wait_equal("after a restart of kafka-connect during a stream of changes", CATCH_UP_TIMEOUT_SECONDS)


def step_schema_change_is_observed(connector_id: str) -> None:
    """A new source column is observed (`SRC-8`): the applier reports it, the default policy
    applies a non-breaking change, the Bronze table gains the column and the API lists it."""
    pg_exec(f"ALTER TABLE {OBJECTS['current']} ADD COLUMN note text;")
    pg_exec(f"UPDATE {OBJECTS['current']} SET note = 'schema_note' WHERE id = 2;")

    def column_arrived():
        rows = ch_rows(f"SELECT note FROM {bronze('current')} WHERE id = 2")
        return True if rows and rows[0][0] == "schema_note" else None

    _wait_for("the new column reaches the current-mode Bronze table", column_arrived, CATCH_UP_TIMEOUT_SECONDS, interval_s=10)
    listing = API.get(f"{API_URL}/api/connectors/{connector_id}/schema-changes", timeout=15)
    if not listing.ok:
        raise G4Failure(f"GET .../schema-changes answered {listing.status_code} {listing.text[:200]}")
    body = listing.json()
    seen = [
        c for bucket in ("recent", "pending") for c in (body.get(bucket) or [])
        if c.get("kind") == "column_added" and c.get("columnName") == "note"
    ]
    if not seen:
        raise G4Failure(f"the API lists no column_added change for 'note': {str(body)[:400]}")
    print("[g4] the added source column reached Bronze and is listed as a schema change", flush=True)


def step_delete_removes_everything(connector_id: str) -> None:
    """G4's second acceptance criterion, on the product path: DELETE removes the Kafka Connect
    connector, the replication slot, the publication and the password file."""
    slot_before = pg_exec(f"SELECT count(*) FROM pg_replication_slots WHERE slot_name = '{CDC_SLOT_NAME}';")
    if slot_before != "1":
        raise G4Failure(f"expected the {CDC_SLOT_NAME} replication slot to exist before the delete, got count={slot_before}")
    retained = pg_exec(
        f"SELECT pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn) FROM pg_replication_slots WHERE slot_name = '{CDC_SLOT_NAME}';"
    )
    print(f"[g4] slot {CDC_SLOT_NAME!r} exists before the delete, wal_retained_bytes={retained}", flush=True)
    deleted = API.delete(f"{API_URL}/api/connectors/{connector_id}", timeout=120)
    if not deleted.ok:
        raise G4Failure(f"DELETE /api/connectors/{connector_id} answered {deleted.status_code} {deleted.text[:300]}")

    def gone():
        in_connect = requests.get(f"{CONNECT_URL}/connectors/{_connect_name(connector_id)}", timeout=10).status_code
        slots = pg_exec(f"SELECT count(*) FROM pg_replication_slots WHERE slot_name = '{CDC_SLOT_NAME}';")
        pubs = pg_exec(f"SELECT count(*) FROM pg_publication WHERE pubname = '{CDC_PUBLICATION_NAME}';")
        file_left = os.path.exists(os.path.join(CDC_SECRETS_DIR, connector_id))
        if in_connect == 404 and slots == "0" and pubs == "0" and not file_left:
            return True
        raise G4Failure(
            f"after DELETE: Kafka Connect answers {in_connect} for the connector, slots={slots}, "
            f"publications={pubs}, password file left={file_left}"
        )

    _wait_for("the connector, slot, publication and password file are gone", gone, 90, interval_s=3)
    print(f"[g4] slot {CDC_SLOT_NAME!r} no longer exists after the delete: WAL is no longer pinned (R5)", flush=True)


WAL_BREACH_SLOT_NAME = "g4_wal_breach_slot"


def step_wal_breach_produces_a_silenceable_alert_instance() -> None:
    """WS5 acceptance (grand plan §7): a WAL breach on the CDC slot
    produces a real alert_instance row via /api/alerts/run, and that row
    can be silenced through the API.

    C3-F2: the breach cannot be made by stopping the connector's container:
    `g4-test-runner` (`docker-compose.yml`) is `python:3.12-slim` with no Docker
    CLI and no `/var/run/docker.sock` mount, and mounting the Docker socket into
    a test container would hand this gate root-equivalent control of the host's
    Docker daemon just to grow a number. (The one container restart this gate
    needs, `kafka-connect`, is done by the CI job, see the module doc.)

    Breach mechanism: open a SECOND, deliberately unconsumed
    logical replication slot with `pg_create_logical_replication_slot`,
    using the same `pgoutput` output plugin the real CDC slot uses
    (`plugin.name=pgoutput` in the connector config `lakehouse-store` renders). A slot with no consumer
    attached is `active = false` in `pg_replication_slots` from the
    moment it is created, and `replication_metrics.py::_status_for`
    treats `not active` as `critical` independently of WAL byte volume
    ("a disconnected [consumer] still pins WAL ... exactly why this
    checks `active` independently of the byte thresholds"). WAL is still
    grown with ordinary UPDATEs against the five rows of `wal_noise`, a
    table of this gate's schema that is NOT in the connector's publication
    (so the connector and the Bronze comparison never see them) —
    REPLICA IDENTITY FULL means each UPDATE writes a full-row WAL image —
    so the slot's
    `wal_retained_bytes` is also genuinely non-trivial, not just
    `active = false` on an idle slot.

    `replication_slot_check_job` reads every logical slot on the source
    (`SELECT ... FROM pg_replication_slots WHERE slot_type = 'logical'`,
    no slot-name filter), and the seeded alert rule
    (`step_seed_wal_alert_rule`) is `max(unhealthy) > 0` over ALL slots
    in `serving.replication_slot_health` — it cannot distinguish which
    slot is unhealthy, only that at least one is. That is fine for what
    this step needs (a real, non-fabricated breach that fires the rule)
    but it means the rule would fire identically if the REAL CDC slot
    (`g4_slot`) ever went unhealthy for an unrelated reason. This step
    never touches `g4_slot` or the connector — the CDC pipeline the rest
    of this gate exercises keeps running, untouched, throughout.

    The breach slot is dropped in a `finally`: an unconsumed logical
    slot pins WAL indefinitely (the same R5 risk this whole job exists
    to catch) and would fill the disk of any environment that left it
    behind, CI runner included.
    """
    pg_exec(
        f"SELECT pg_create_logical_replication_slot('{WAL_BREACH_SLOT_NAME}', 'pgoutput');"
    )
    print(f"[g4] created deliberately unconsumed slot {WAL_BREACH_SLOT_NAME!r} (plugin=pgoutput)")
    try:
        for i in range(2000):
            pg_exec(f"UPDATE {CDC_SCHEMA}.wal_noise SET v = v + 1 WHERE id = {(i % 5) + 1};")
        # The webserver answering is not the code location having loaded:
        # compose can recreate `dagster-code-location` when this runner
        # starts, and a launch in that window fails with
        # PipelineNotFoundError (seen in CI). Wait until Dagster lists it.
        _wait_for(
            "Dagster code location (replication_slot_check_job loaded)",
            lambda: _dagster_job_loaded("replication_slot_check_job"),
            120,
            2.0,
        )
        # replication_slot_check_job writes lake.bronze_meta.replication_slot
        # on its own 15-minute schedule (replication_metrics.py:455,
        # cron_schedule="*/15 * * * *") — run it once, out of band, via
        # Dagster's launchRun mutation so this gate does not wait 15
        # minutes for a fresh row.
        launch = requests.post(
            DAGSTER_URL,
            json={
                "query": "mutation($job:String!){ launchRun(executionParams:{selector:{repositoryLocationName:\"dispar_orchestrate.definitions\",repositoryName:\"__repository__\",jobName:$job}, runConfigData:\"{}\"}){ __typename ... on LaunchRunSuccess { run { id } } ... on PythonError { message } } }",
                "variables": {"job": "replication_slot_check_job"},
            },
            timeout=30,
        )
        launch.raise_for_status()
        launched = launch.json().get("data", {}).get("launchRun", {})
        if launched.get("__typename") != "LaunchRunSuccess":
            raise G4Failure(f"failed to launch replication_slot_check_job: {launch.text}")
        # Wait for THIS run to finish while the breach slot still exists. A
        # fixed sleep let the run start after this step had already
        # evaluated and dropped the slot (seen: the run recorded only the
        # healthy CDC slot).
        run_id = launched["run"]["id"]
        _wait_for(
            f"replication_slot_check_job run {run_id} finished",
            lambda: _dagster_run_finished(run_id),
            180,
            2.0,
        )

        run_resp = API.post(
            f"{API_URL}/api/alerts/run",
            headers={"x-run-token": ALERTS_RUN_TOKEN} if ALERTS_RUN_TOKEN else {},
            timeout=30,
        )
        run_resp.raise_for_status()
        results = run_resp.json()["results"]
        fired = next((r for r in results if r.get("id") == WAL_ALERT_RULE_ID and r.get("fired")), None)
        if fired is None:
            raise G4Failure(f"expected a fired result for {WAL_ALERT_RULE_ID}, got: {results}")

        alerts_resp = API.get(f"{API_URL}/api/overview/alerts", timeout=10)
        alerts_resp.raise_for_status()
        # Match the instance produced by THIS rule specifically — not
        # "any open alert" (WS5 plan review U8), since other rules may
        # also be open in a shared CI environment.
        open_alert = next(
            (a for a in alerts_resp.json() if a["status"] == "open" and a.get("ruleId") == WAL_ALERT_RULE_ID),
            None,
        )
        if open_alert is None:
            raise G4Failure(f"no open alert_instance row for {WAL_ALERT_RULE_ID} after a fired WAL breach")
        if open_alert.get("severity") != "critical":
            raise G4Failure(
                f"expected the fired instance's severity to be copied verbatim from the seeded "
                f"rule's severity ('critical'), got: {open_alert.get('severity')!r}"
            )

        # The 15-minute dedup holds (insert_from_fired_rule's `fired_at >
        # now - 15min` check, independent of silence -- the rule is not
        # yet silenced at this point): a second /api/alerts/run call while
        # the breach is still ongoing must NOT insert a second
        # alert_instance row for this rule, even though the rule still
        # fires on every evaluation.
        dedup_run_resp = API.post(
            f"{API_URL}/api/alerts/run",
            headers={"x-run-token": ALERTS_RUN_TOKEN} if ALERTS_RUN_TOKEN else {},
            timeout=30,
        )
        dedup_run_resp.raise_for_status()
        alerts_after_dedup_run = API.get(f"{API_URL}/api/overview/alerts", timeout=10)
        alerts_after_dedup_run.raise_for_status()
        instances_for_rule_after_dedup = [
            a for a in alerts_after_dedup_run.json() if a.get("ruleId") == WAL_ALERT_RULE_ID
        ]
        if len(instances_for_rule_after_dedup) != 1:
            raise G4Failure(
                f"expected exactly one alert_instance row for {WAL_ALERT_RULE_ID} within the "
                f"15-minute dedup window, got {len(instances_for_rule_after_dedup)}: "
                f"{instances_for_rule_after_dedup}"
            )

        silence_resp = API.post(
            f"{API_URL}/api/overview/alerts/{open_alert['id']}/silence",
            json={"untilMinutes": 60},
            timeout=10,
        )
        if silence_resp.status_code != 200 or not silence_resp.json().get("silencedUntil"):
            raise G4Failure(f"silence did not return a populated silencedUntil: {silence_resp.text}")

        # Silencing suppresses the next insert (and, per
        # `deliver_unless_silenced`/`persist_fired_results`, the next
        # delivery too -- both check the same `SilenceSource`): re-running
        # /api/alerts/run now that the instance is silenced must not
        # produce a new OPEN row for this rule (the existing row moved to
        # "acknowledged" when it was silenced, and stays there). This gate
        # has no webhook receiver to inspect delivery directly (no HTTP
        # surface exists to prove a webhook was NOT sent), so delivery
        # suppression is asserted by code reference here, not measured --
        # what IS externally observable, and asserted below, is that no
        # second open instance appears.
        rerun_resp = API.post(
            f"{API_URL}/api/alerts/run",
            headers={"x-run-token": ALERTS_RUN_TOKEN} if ALERTS_RUN_TOKEN else {},
            timeout=30,
        )
        rerun_resp.raise_for_status()
        alerts_after_rerun = API.get(f"{API_URL}/api/overview/alerts", timeout=10)
        alerts_after_rerun.raise_for_status()
        instances_for_rule = [
            a for a in alerts_after_rerun.json() if a.get("ruleId") == WAL_ALERT_RULE_ID
        ]
        open_after_rerun = [a for a in instances_for_rule if a["status"] == "open"]
        if open_after_rerun:
            raise G4Failure(
                f"a re-run of /api/alerts/run after silencing produced a new open "
                f"instance for {WAL_ALERT_RULE_ID} (dedup/silence-suppression did not "
                f"hold): {open_after_rerun}"
            )
        print("[g4] WAL breach produced a fired, silenceable alert_instance row; re-run after silence stayed suppressed")
    finally:
        # Not optional: an unconsumed logical slot pins WAL on the source
        # Postgres server indefinitely (R5) until it is dropped — leaving
        # this slot behind after the gate exits would fill the disk of
        # whatever environment ran it, same failure mode this job exists
        # to catch. `ON_ERROR_STOP=1` (pg_exec) means a missing slot here
        # (e.g. creation itself failed above) raises rather than silently
        # no-op'ing, so a broken cleanup is visible in gate output instead
        # of masked.
        pg_exec(f"SELECT pg_drop_replication_slot('{WAL_BREACH_SLOT_NAME}');")
        print(f"[g4] dropped slot {WAL_BREACH_SLOT_NAME!r} — WAL is no longer pinned")


def step_oidc_authorization_code_round_trip() -> None:
    """Drive a full authorization-code + PKCE login through lakehouse-api's
    real /api/auth/oidc/{start,callback} routes against oidc-mock's
    /authorize + authorization_code /token support (ops/oidc-mock/server.py)
    — not a mocked HTTP
    layer, a real three-hop redirect chain over `requests.Session` so
    cookies persist exactly as a browser's would. A separate `Session`
    from this file's module-level `API` (used by step_login and every
    alerts step above): that one already carries a bootstrap-admin
    `lh_session` cookie, and reusing it here would make it impossible to
    tell whether `/api/auth/me` below is reporting the OIDC-minted
    session's permissions or the pre-existing admin one's.
    """
    session = requests.Session()

    # Hop 1: lakehouse-api issues the PKCE challenge/state/nonce and
    # redirects to oidc-mock's /authorize. `allow_redirects=False` at
    # every hop so this test controls and asserts on each redirect
    # individually, rather than following the whole chain blind and only
    # checking the final response.
    start = session.get(f"{API_URL}/api/auth/oidc/start", allow_redirects=False, timeout=10)
    if start.status_code != 302:
        raise G4Failure(f"/api/auth/oidc/start did not redirect: HTTP {start.status_code}")
    authorize_url = start.headers.get("Location", "")
    if not authorize_url.startswith(f"{OIDC_MOCK_URL}/authorize"):
        raise G4Failure(f"/start redirected somewhere other than oidc-mock's /authorize: {authorize_url}")
    if "code_challenge_method=S256" not in authorize_url:
        raise G4Failure("authorize URL is missing code_challenge_method=S256")
    flow_cookie = session.cookies.get("lh_oidc_flow")
    if not flow_cookie:
        raise G4Failure("lh_oidc_flow cookie was not set by /start")

    # Hop 2: oidc-mock's /authorize. A browser would GET it, be
    # served a login-screen stand-in whose hidden fields carry the OIDC
    # request parameters, and POST those back with the chosen identity —
    # so this test does the same. Posting `login_hint` alone would submit
    # empty redirect_uri/nonce/code_challenge fields (the mock's POST
    # handler reads its form body, not the URL's query string), and the
    # code it minted would then be bound to nothing, failing at hop 3 for
    # a reason that has nothing to do with lakehouse-api.
    authorize_params = {
        key: values[0]
        for key, values in urllib.parse.parse_qs(
            urllib.parse.urlparse(authorize_url).query
        ).items()
    }
    login_form = session.get(authorize_url, allow_redirects=False, timeout=10)
    if login_form.status_code != 200:
        raise G4Failure(
            f"oidc-mock /authorize did not serve its login screen: HTTP {login_form.status_code}"
        )
    authorize = session.post(
        authorize_url,
        data={
            "login_hint": "test-analyst",
            "redirect_uri": authorize_params.get("redirect_uri", ""),
            "nonce": authorize_params.get("nonce", ""),
            "code_challenge": authorize_params.get("code_challenge", ""),
            "state": authorize_params.get("state", ""),
        },
        allow_redirects=False,
        timeout=10,
    )
    if authorize.status_code != 302:
        raise G4Failure(f"oidc-mock /authorize did not redirect: HTTP {authorize.status_code}")
    callback_url = authorize.headers.get("Location", "")
    if "/api/auth/oidc/callback" not in callback_url:
        raise G4Failure(f"oidc-mock redirected somewhere other than the callback: {callback_url}")

    # Hop 3: lakehouse-api's callback exchanges the code (PKCE verifier
    # from the flow cookie, never re-sent by the client), verifies the id
    # token (signature/iss/aud/exp/nbf/nonce via
    # OidcAuthenticator::authenticate_with_nonce), and mints a
    # real session.
    callback = session.get(callback_url, allow_redirects=False, timeout=10)
    if callback.status_code != 302:
        raise G4Failure(f"/api/auth/oidc/callback did not redirect: HTTP {callback.status_code}")
    if "lh_session" not in session.cookies:
        raise G4Failure("no lh_session cookie was set after the OIDC callback")
    if "lh_oidc_flow" in session.cookies:
        raise G4Failure("lh_oidc_flow cookie was not cleared after the callback consumed it")

    # Prove the session is real and carries the role OIDC_ROLE_MAP mapped
    # test-analyst's "lakehouse-analysts" group to (g4-test-runner sets
    # OIDC_ROLE_MAP=lakehouse-analysts=Analyst — see the compose change
    # below), not merely that a cookie exists.
    me = session.get(f"{API_URL}/api/auth/me", timeout=10)
    if me.status_code != 200:
        raise G4Failure(f"GET /api/auth/me failed after OIDC login: HTTP {me.status_code}")
    permissions = me.json().get("permissions", [])
    if "query:read" not in permissions:
        raise G4Failure(f"OIDC-mapped Analyst role did not grant query:read: permissions={permissions}")
    print("[g4] OIDC authorization-code + PKCE round trip: session minted, Analyst role's query:read confirmed")



def _write_credential_file(ref_name: str, value: str) -> None:
    """Duplicated from `ops/g6/g6_ingest_matrix_test.py` (this repository's
    gates share no code beyond `gate_summary`): refuse any reference that is
    not the `file:/run/secrets/connector_*` name the server derived (ADR 0002
    Addendum 3) before writing `value` under its basename into
    `/gate-secrets`, the writable half of the volume `lakehouse-api` reads at
    `/run/secrets` (`ops/g4/docker-compose.g4.override.yml`)."""
    if not ref_name.startswith(FILE_REF_PREFIX):
        raise G4Failure(f"refusing to write a credential file for an unexpected ref shape: {ref_name!r}")
    path = os.path.join(GATE_SECRETS_DIR, ref_name.rsplit("/", 1)[-1])
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(value)
    os.chmod(path, 0o644)


def _ensure_gate_tenant() -> str:
    """Duplicated, shortened, from `ops/g6/g6_ingest_matrix_test.py::
    step_ensure_tenant` (its docstring has the why): per-connector routes
    answer only for a connector in one of the caller's tenants, and the
    bootstrap admin of a fresh database belongs to none, so the account joins
    the seed tenant first. Returns that tenant's id."""
    me = API.get(f"{API_URL}/api/auth/me", timeout=10)
    if not me.ok:
        raise G4Failure(f"GET /api/auth/me failed: {me.status_code} {me.text[:300]}")
    me = me.json()
    listed = API.get(f"{API_URL}/api/identity/tenants", timeout=10)
    if not listed.ok:
        raise G4Failure(f"list tenants failed: {listed.status_code} {listed.text[:300]}")
    tenant = next((t for t in listed.json() if t["slug"] == GATE_TENANT_SLUG), None)
    if tenant is None:
        raise G4Failure(f"tenant {GATE_TENANT_SLUG!r} does not exist (0042_tenant_provisioning.sql seeds it)")
    if tenant["id"] not in [t["id"] for t in me.get("tenants", [])]:
        added = API.put(f"{API_URL}/api/identity/users/{me['id']}/tenants/{tenant['id']}", timeout=10)
        if not added.ok:
            raise G4Failure(f"add the account to tenant {GATE_TENANT_SLUG!r} failed: {added.status_code} {added.text[:300]}")
    return tenant["id"]


def _register_demo_cdc_connector(tenant_id: str) -> str:
    """Register the demo source the way the console registers a `PostgreSQL
    CDC` connector: `POST /api/connectors` with a `file:` credential whose
    value is this stack's Postgres password, then `PUT .../ingest-spec` with
    the `cdc` adapter, the slot and publication of this gate and the two
    tables. That PUT provisions the connector in Kafka Connect (`SRC-4`
    decision S2-e). Returns the connector id."""
    created = API.post(
        f"{API_URL}/api/connectors",
        json={
            "name": "g4-demo-cdc", "type": CDC_TYPE, "direction": "source",
            "host": "postgres:5432", "credential": {"source": "file", "primary": "password"},
            "environment": "production", "tenant": "g4", "tenantId": tenant_id, "residency": "", "capabilities": [],
        },
        timeout=10,
    )
    if not created.ok:
        raise G4Failure(f"create the CDC connector failed: {created.status_code} {created.text[:300]}")
    body = created.json()
    connector_id = body["id"]
    derived = (body.get("credential") or {}).get("primary")
    if not derived:
        raise G4Failure("the create response carries no derived credential name")
    _write_credential_file(derived, os.environ["PGPASSWORD"])
    spec = API.put(f"{API_URL}/api/connectors/{connector_id}/ingest-spec", json=_ingest_spec_body(), timeout=60)
    if not spec.ok:
        raise G4Failure(f"set the CDC connector's ingest-spec failed: {spec.status_code} {spec.text[:300]}")
    return connector_id


def _expects_real(step: str) -> bool:
    """False after recording a failure when the registry does not say `real`:
    this gate runs the step for real, so a registry that says otherwise is
    stale (one list, not two)."""
    if RECORDER is not None and RECORDER.registry.expectation(CDC_TYPE, step) == "real":
        return True
    _record(step, "fail", "ops/gates/coverage.json does not say 'real' for this step but g4 runs it; update the registry")
    return False


def _check_connect(connector_id: str) -> None:
    """`connect`: `POST .../test` answers `supported: true, ok: true` for the
    demo source, as it does for its SQL twin (duplicated in miniature from
    `ops/g6`'s `_check_connect`, whose `real` branch it is)."""
    if not _expects_real("connect"):
        return
    try:
        resp = API.post(f"{API_URL}/api/connectors/{connector_id}/test", timeout=30)
        if not resp.ok:
            _record("connect", "fail", f"POST .../test answered {resp.status_code} {resp.text[:200]}")
            return
        body = resp.json()
    except (requests.RequestException, ValueError) as exc:
        _record("connect", "fail", f"POST .../test did not answer cleanly ({type(exc).__name__})")
        return
    if body.get("supported") is True and body.get("ok") is True:
        _record("connect", "pass", f"{body.get('latencyMs')} ms")
    else:
        _record(
            "connect", "fail",
            f"expected supported=true ok=true, got supported={body.get('supported')!r} "
            f"ok={body.get('ok')!r} message={str(body.get('message'))[:200]!r}",
        )


def _discovery(connector_id: str, kind: str, body: dict) -> dict:
    """`POST .../discovery/{kind}` (SRC-13). A PostgreSQL CDC source is read by the API itself, so the
    answer comes in the call (`state` `done` or `failed`), with no polling."""
    resp = API.post(f"{API_URL}/api/connectors/{connector_id}/discovery/{kind}", json=body, timeout=135)
    if not resp.ok:
        raise G4Failure(f"POST .../discovery/{kind} answered {resp.status_code} {resp.text[:200]}")
    return resp.json()


def _check_discover(connector_id: str) -> None:
    """`discover`: the schemas hold `g4_cdc`, its tables hold `g4_cdc.orders`, and the preview of
    that table shows its columns (`real` branch of `ops/g6`'s `_check_discover`, SRC-13 routes)."""
    if not _expects_real("discover"):
        return
    try:
        schemas = _discovery(connector_id, "list", {})
        names = [o.get("name") for o in schemas.get("objects") or [] if o.get("kind") == "schema"]
        if schemas.get("state") != "done" or CDC_SCHEMA not in names:
            _record("discover", "fail", f"expected the schemas to hold {CDC_SCHEMA!r}, got state={schemas.get('state')!r} {names[:20]}")
            return
        listing = _discovery(connector_id, "list", {"schema": CDC_SCHEMA})
        preview = _discovery(connector_id, "preview", {"object": CDC_DISCOVER_TABLE})
    except (requests.RequestException, ValueError, G4Failure) as exc:
        _record("discover", "fail", f"the discovery routes did not answer cleanly ({type(exc).__name__}: {str(exc)[:200]})")
        return
    listed = [o.get("name") for o in listing.get("objects") or []]
    if listing.get("state") != "done" or CDC_DISCOVER_TABLE not in listed:
        _record("discover", "fail", f"expected state=done listing {CDC_DISCOVER_TABLE!r}, got state={listing.get('state')!r} objects={listed[:20]}")
        return
    found = {c.get("name") for c in preview.get("columns") or []}
    if preview.get("state") != "done" or not CDC_DISCOVER_COLUMNS <= found:
        _record("discover", "fail", f"{CDC_DISCOVER_TABLE!r} preview: state={preview.get('state')!r}, missing columns {sorted(CDC_DISCOVER_COLUMNS - found)}, has {sorted(found)}")
        return
    _record("discover", "pass", f"{CDC_DISCOVER_TABLE} with {len(found)} columns")


def step_register_demo_connector_and_check_connect_discover() -> str | None:
    """SRC-9 task 6, steps `connect` and `discover`. A problem with the
    registration itself fails both cells, with the reason; the connector id is
    None then, and `main` stops (everything below needs the connector)."""
    try:
        connector_id = _register_demo_cdc_connector(_ensure_gate_tenant())
    except (G4Failure, requests.RequestException, KeyError, ValueError) as exc:
        detail = f"could not register the demo CDC connector: {type(exc).__name__}: {exc}"
        _record("connect", "fail", detail)
        _record("discover", "fail", detail)
        return None
    _check_connect(connector_id)
    _check_discover(connector_id)
    return connector_id


def main() -> int:
    global RECORDER
    try:
        RECORDER = gate_summary.Recorder(gate_summary.Registry.load(), "g4")
        owner = RECORDER.registry.gate_of(CDC_TYPE)
        if owner != "g4":
            raise G4Failure(f"ops/gates/coverage.json gives {CDC_TYPE!r} to gate {owner!r}, but g4 runs it")
        step_wait_for_services()
        step_login()
        step_seed_source()
        step_create_catalog_database()
        connector_id = step_register_demo_connector_and_check_connect_discover()
        if connector_id is None:
            raise G4Failure("the demo connector could not be registered, so nothing below can run")
        _wait_capture_state(connector_id, "running", 240)
        step_connector_holds_no_password(connector_id)
        step_first_copy_reached_bronze()
        step_changes_visible_within_budget()
        step_pause_and_resume_lose_nothing(connector_id)
        step_restart_kafka_connect_during_changes(connector_id)
        step_schema_change_is_observed(connector_id)
        step_seed_wal_alert_rule()
        step_wal_breach_produces_a_silenceable_alert_instance()
        step_delete_removes_everything(connector_id)
        step_oidc_authorization_code_round_trip()
        # SRC-9 task 6: connect and discover record a failure and let the gate
        # run on; the verdict comes here, once, naming each.
        failed = RECORDER.failures()
        if failed:
            raise G4Failure("connector gate steps failed: " + "; ".join(f"{t} {s}: {d}" for t, s, d in failed))
    except G4Failure as exc:
        print(f"[g4] FAILED: {exc}", file=sys.stderr, flush=True)
        return 1
    finally:
        # The table is printed on every path, so a failed run still says which
        # step broke and which never ran.
        if RECORDER is not None:
            RECORDER.emit()
    print("[g4] PASS", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
