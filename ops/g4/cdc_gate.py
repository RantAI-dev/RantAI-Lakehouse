"""The change-capture gate, shared by `g4-cdc` (PostgreSQL), `g4-mysql` and `g4-mssql`
(`SRC-4` slice 3, plan task T3.6; ADR 0016).

Each gate is a thin script (`g4_test.py`, `g4_mysql_test.py`, `g4_mssql_test.py`) that
builds a `Source` (what is specific to one database: how to run SQL against it, how its
two tables are made, the connector's dial, what a schema change is) and calls `run`. This
module holds everything the three have in common, and it drives the product path through
the API a person uses:

1. The source has TWO identical tables (`orders` in "current row per key" mode,
   `orders_history` in "history" mode), 20,000 seeded rows each.
2. The connector is registered as the console does it (`POST /api/connectors` with a
   `file:` credential, `PUT .../ingest-spec` with the `cdc` adapter and the two tables).
   That PUT provisions `Kafka Connect`; nothing else is started by hand.
3. Steps, in order, each of which fails the gate by name:
   - the connector's `Kafka Connect` config does not hold the source password (only the
     `${dir:...}` reference), and the password file exists on the memory-only volume;
   - for `MySQL` and `SQL Server`: the schema-history topic exists and never expires
     (decision S3-d), checked on the broker;
   - the first copy reaches Bronze: 20,000 rows in each table (step `load`);
   - an insert, an update and a delete each become visible in Bronze, in both modes,
     within 120 seconds (step `reload`; decision D5);
   - pause through `POST .../pause`, write 20,000 changes to each table, resume through
     `POST .../resume`: Bronze equals the source, nothing missing or twice;
   - restart the `kafka-connect` container during a stream of changes: Bronze equals the
     source again. The runner has no Docker access (and gets none), so it prints the
     marker line `G4-RESTART-KAFKA-CONNECT` and waits for the file
     `/tmp/g4-kafka-connect-restarted`; the CI job restarts the container and creates the
     file with `docker exec`;
   - lag (`SRC-4` slice 4, decision S4-h): after the first copy `GET .../change-capture` says
     "up to date" for both tables; the connection test lists the setup checks, all `ok` for the
     gate's source (the SQL Server Agent one is `unchecked`: the documented login cannot read it);
     and a second connector with ONE setting missing (a publication that does not exist, a
     login without `REPLICATION CLIENT`, a table without CDC) gets exactly that check `missing`
     with its fixed sentence and a command, the rest `ok`;
   - (`g4-cdc` only, AC7) the lag alert: with the limit at 1 minute and `kafka-connect` stopped by
     the CI job (marker `G4-STOP-KAFKA-CONNECT`, file `/tmp/g4-kafka-connect-stopped`) while
     changes are written, the capture route reports a lag above 60 seconds and ONE `connector_lag`
     alert instance exists; started again (`G4-START-KAFKA-CONNECT`,
     `/tmp/g4-kafka-connect-started`) the tables come back to "up to date" and the alert closes;
   - a source schema change is observed: a new column reaches the Bronze table and
     `GET .../schema-changes` lists it;
   - (PostgreSQL only, in `g4_test.py`) the WAL-breach alert and the OIDC round trip;
   - `DELETE /api/connectors/{id}` removes the `Kafka Connect` connector and the password
     file (and, for PostgreSQL, the replication slot and publication).

# Why the sources of truth are the ones they are

Bronze is read through ClickHouse (`DataLakeCatalog` over Lakekeeper), the way the product
reads it; the source is read with its own client. Neither side is asked to describe itself:
this gate compares rows with rows. In history mode the expected events are known to the gate
because it issues every statement, so the comparison is of two multisets of (id, operation,
customer), not of the pipeline with itself.

# SRC-9: the four steps of the connector gates

Each gate is also the connector gate of its type (`ops/gates/coverage.json`) and records the
four steps through `ops/gates/gate_summary.py`, whose table it prints at the end and the job
copies into its summary: `connect` and `discover` (the source is registered and `POST
.../test` and the discovery routes answer for real; a failed one is recorded and the gate
goes on), `load` (the first copy) and `reload` (the three kinds of change within the budget).

# Why `count() WHERE id > 0`, never a bare `count()`

`docs/plans/P5-RESULT.md`'s (A) finding: ClickHouse 26.3's bare `count()` over a merge-on-read
Iceberg table with equality deletes takes a metadata-only fast path that does NOT apply delete
filtering and overcounts. Every row-count check uses a `WHERE` predicate.
"""

from __future__ import annotations

import collections
import dataclasses
import decimal
import os
import sys
import threading
import time

import requests

# SRC-9 plan task 6: the coverage registry and the result table, mounted at /opt/gates by
# the gate's runner (PYTHONPATH).
import gate_summary

CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
LAKEKEEPER_CATALOG_URI = os.environ.get("CH_LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog")
RUSTFS_S3_ENDPOINT = os.environ.get("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000")
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
# R1 (ADR 0011): see `ops/g3a/g3a_test.py`'s identical comment: `catalog_credential` needs the
# OAuth2 `client_id:client_secret` form, pointed at `ops/oidc-mock`'s `/token` endpoint.
CH_OAUTH_CLIENT_ID = os.environ.get("CH_OAUTH_CLIENT_ID", "")
CH_OAUTH_SERVER_URI = os.environ.get("CH_OAUTH_SERVER_URI", "")

API_URL = os.environ.get("API_URL", "http://lakehouse-api:8089")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "admin@example.invalid")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "changeme")

API = requests.Session()

# The freshness budget of `SRC-4` decision D5: each kind of change visible in Bronze within
# two minutes. Measured p95 was 18 to 40 s in slice 0 (`docs/plans/SRC-4-RESULT.md`); the
# sensor's 30 s interval dominates.
FRESHNESS_BUDGET_SECONDS = 120.0
# How long a backlog (20,000 changes per table after a pause, or a worker restart) may take to
# reach Bronze. Slice 0 measured 108,000 changes in about 100 s; this is the job's whole budget.
CATCH_UP_TIMEOUT_SECONDS = 600.0
FIRST_COPY_TIMEOUT_SECONDS = 600.0
SEED_ROWS = 20_000

GATE_SECRETS_DIR = "/gate-secrets"
FILE_REF_PREFIX = "file:/run/secrets/connector_"
GATE_TENANT_SLUG = "meridian-group"
MODES = ("current", "history")
TABLES = {"current": "orders", "history": "orders_history"}
TARGETS = {"current": "g4_orders_current", "history": "g4_orders_history"}
LOAD_MODES = {"current": "incremental_dedup", "history": "history"}
# What the discovery of the schema must list and preview.
DISCOVER_COLUMNS = {"id", "customer", "amount", "status", "updated_at"}

# Kafka Connect, from inside the compose network (the runner is the only thing besides
# `lakehouse-api` that dials it, and only to read: the gate asserts that the connector's stored
# config holds no password).
CONNECT_URL = os.environ.get("KAFKA_CONNECT_URL", "http://kafka-connect:8083").rstrip("/")
# `lakehouse_cdc_secrets` mounted read-only in the runner by the gate's override: the only reason
# this gate can look at the password file. Production mounts that volume in `lakehouse-api` and
# `kafka-connect` only.
CDC_SECRETS_DIR = "/cdc-secrets"
# The handshake with the CI job, which alone can restart a container.
RESTART_MARKER = "G4-RESTART-KAFKA-CONNECT"
RESTART_DONE_FILE = "/tmp/g4-kafka-connect-restarted"
RESTART_WAIT_SECONDS = 300.0
# The lag alert step (`g4-cdc` only) asks the CI job to stop and to start `kafka-connect`, the way
# the restart step asks for a restart: a marker line, then a file the job creates with `docker exec`.
STOP_MARKER = "G4-STOP-KAFKA-CONNECT"
STOP_DONE_FILE = "/tmp/g4-kafka-connect-stopped"
START_MARKER = "G4-START-KAFKA-CONNECT"
START_DONE_FILE = "/tmp/g4-kafka-connect-started"
# The lag limit the alert step sets (`SRC-4` AC7), and a webhook nothing listens on: 192.0.2.0/24 is
# TEST-NET-1, which the address check of `SEC-10` admits and nothing routes (as the WAL rule's sink).
LAG_LIMIT_MINUTES = 1
LAG_ALERT_SINK = "http://192.0.2.1:9/g4-lag-sink"
UP_TO_DATE_TIMEOUT_SECONDS = 300.0
# The broker the schema-history topics live on (`MySQL` and `SQL Server`).
KAFKA_BOOTSTRAP = os.environ.get("CDC_KAFKA_BOOTSTRAP", "kafka:9092")


class G4Failure(Exception):
    pass


@dataclasses.dataclass(frozen=True)
class MissingCase:
    """The second connector of the setup-check step: its dial and tables, the password of its
    login, and the one check that must come out `missing` with its fixed sentence (the text of
    `lakehouse-api`'s `cdc_setup_check.rs`)."""

    dial: dict
    source_objects: list
    password: str
    setting: str
    message: str


class Source:
    """What is specific to one source database. A gate subclasses this; `run` does the rest.

    Every SQL string the shared steps build goes through `now`, `qualified` and `numbers`, so
    the dialect differences (`now()` against `NOW()` against `SYSDATETIME()`, `generate_series`
    against a numbers table) live in one place per source."""

    cdc_type = ""  # the registry type, e.g. "PostgreSQL CDC"
    gate = ""  # the recorder's gate name, e.g. "g4"
    catalog_db = ""  # the ClickHouse database over the Lakekeeper catalog
    schema = ""  # the first part of an object name, as discovery lists it
    host_for_registry = ""  # `host` of `POST /api/connectors`
    now = "now()"
    needs_schema_history = False  # `MySQL` and `SQL Server` keep their history in a Kafka topic
    password = ""  # the source password the connector's credential file holds

    def ping(self) -> None:
        """Raise if the source cannot be reached."""
        raise NotImplementedError

    def setup(self) -> None:
        """Create the schema, the two tables, a user the connector dials, and seed `SEED_ROWS`
        rows into each table, then switch change capture on for the two tables."""
        raise NotImplementedError

    def execute(self, sql: str) -> None:
        raise NotImplementedError

    def rows(self, sql: str) -> list[tuple[str, ...]]:
        """Rows as tuples of strings (a NULL is the empty string)."""
        raise NotImplementedError

    def qualified(self, table: str) -> str:
        """`table` as a statement of this source names it."""
        raise NotImplementedError

    def numbers(self, lo: int, hi: int) -> str:
        """A `SELECT` of one column `g`, the integers `lo..=hi`."""
        raise NotImplementedError

    def dial(self) -> dict:
        raise NotImplementedError

    def add_column_and_observe(self) -> None:
        """Add a nullable text column `note` to the current-mode table and set it on row 2, the
        way this source's documentation says a schema change is made."""
        raise NotImplementedError

    def after_delete(self, connector_id: str) -> None:
        """Source-side checks after `DELETE` (PostgreSQL: slot and publication gone)."""

    def before_delete(self, connector_id: str) -> None:
        """Source-side checks before `DELETE` (PostgreSQL: the slot exists)."""

    # Whether this gate also runs the lag alert step (AC7): `g4-cdc` only.
    lag_alert_gate = False

    # The setup checks the connection test lists for this source's own connector (SHOULD-FIX 10c).
    expected_settings: tuple = ()

    def missing_case(self) -> "MissingCase":
        """A connector whose source lacks exactly one setting (`SRC-4` S4-h). May change the source
        (a second login, a table) first."""
        raise NotImplementedError

    def unchecked_is_expected(self, check: dict) -> bool:
        """Whether `check` may be `unchecked` for this source's documented login (SQL Server
        Agent: reading it needs `VIEW SERVER STATE`, which the login does not have)."""
        return False

    def object_name(self, mode: str) -> str:
        return f"{self.schema}.{TABLES[mode]}"

    @property
    def discover_table(self) -> str:
        return self.object_name("current")


SOURCE: Source | None = None
RECORDER: gate_summary.Recorder | None = None

# What the history table must hold at any moment: every event the gate caused, keyed (id,
# operation, customer). The snapshot rows are added when the source is seeded; every later
# statement adds what it changes.
EXPECTED_HISTORY: collections.Counter[tuple[int, str, str]] = collections.Counter()


def src() -> Source:
    if SOURCE is None:
        raise G4Failure("internal error: no source was set")
    return SOURCE


def _record(step: str, outcome: str, detail: str = "") -> None:
    if RECORDER is None:
        raise G4Failure("internal error: the gate summary recorder was not created")
    RECORDER.record(src().cdc_type, step, outcome, detail)
    print(f"[g4] {src().cdc_type} {step}: {outcome}{' (' + detail + ')' if detail else ''}", flush=True)


def ch_auth_settings() -> str:
    if not CH_OAUTH_CLIENT_ID:
        return ""
    return f", catalog_credential = '{CH_OAUTH_CLIENT_ID}:unused', oauth_server_uri = '{CH_OAUTH_SERVER_URI}'"


def ch_query(sql: str) -> str:
    resp = requests.post(CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=60)
    if not resp.ok:
        raise G4Failure(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


def ch_rows(sql: str) -> list[tuple[str, ...]]:
    text = ch_query(f"{sql} SETTINGS allow_database_iceberg=1 FORMAT TSV")
    return [tuple(line.split("\t")) for line in text.splitlines() if line]


def bronze(mode: str) -> str:
    return f"{src().catalog_db}.`bronze.{TARGETS[mode]}`"


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


def step_login() -> None:
    """Every `/api/*` route this gate calls is `Policy::RequiresAuth` or stricter: log in as the
    bootstrap admin and let `API` carry the session cookie, as `ops/g3a/g3a_test.py::step_login`
    does for the same reason."""
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
    print("[g4] logged in as bootstrap admin", flush=True)


def step_wait_for_services() -> None:
    _wait_for("ClickHouse", lambda: ch_query("SELECT 1 FORMAT TabSeparated").strip() == "1" or None, 60)
    _wait_for("the source", lambda: (src().ping() or True), 120, interval_s=3)

    def connect_up():
        resp = requests.get(f"{CONNECT_URL}/", timeout=5)
        return True if resp.ok else None

    _wait_for("Kafka Connect's REST API", connect_up, 180, interval_s=3)
    print("[g4] ClickHouse, the source and Kafka Connect are reachable", flush=True)


def step_seed_source() -> None:
    src().setup()
    for g in range(1, SEED_ROWS + 1):
        EXPECTED_HISTORY[(g, "r", f"seed_{g}")] += 1
    print(f"[g4] seeded {SEED_ROWS} rows into each of {', '.join(src().object_name(m) for m in MODES)}", flush=True)


def step_create_catalog_database() -> None:
    ch_query(
        f"CREATE DATABASE IF NOT EXISTS {src().catalog_db} "
        f"ENGINE = DataLakeCatalog('{LAKEKEEPER_CATALOG_URI}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{LAKEKEEPER_WAREHOUSE}', "
        f"storage_endpoint = '{RUSTFS_S3_ENDPOINT}'{ch_auth_settings()} "
        "SETTINGS allow_database_iceberg = 1"
    )


# ── The connector, as the console registers it ──────────────────────────────


def _write_credential_file(ref_name: str, value: str) -> None:
    """Duplicated from `ops/g6/g6_ingest_matrix_test.py` (this repository's gates share no code
    beyond `gate_summary`): refuse any reference that is not the `file:/run/secrets/connector_*`
    name the server derived (ADR 0002 Addendum 3) before writing `value` under its basename into
    `/gate-secrets`, the writable half of the volume `lakehouse-api` reads at `/run/secrets`."""
    if not ref_name.startswith(FILE_REF_PREFIX):
        raise G4Failure(f"refusing to write a credential file for an unexpected ref shape: {ref_name!r}")
    path = os.path.join(GATE_SECRETS_DIR, ref_name.rsplit("/", 1)[-1])
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(value)
    os.chmod(path, 0o644)


def _ensure_gate_tenant() -> str:
    """Duplicated, shortened, from `ops/g6/g6_ingest_matrix_test.py::step_ensure_tenant`: per-
    connector routes answer only for a connector in one of the caller's tenants, and the
    bootstrap admin of a fresh database belongs to none, so the account joins the seed tenant
    first. Returns that tenant's id."""
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


def _ingest_spec_body() -> dict:
    return {
        "adapter": "cdc", "ingestMode": "cdc",
        "dial": src().dial(),
        "sourceObjects": [
            {"name": src().object_name(mode), "target": TARGETS[mode], "loadMode": LOAD_MODES[mode], "primaryKey": ["id"]}
            for mode in MODES
        ],
    }


def _register_demo_cdc_connector(tenant_id: str, *, suffix: str = "", spec: dict | None = None, password: str | None = None) -> str:
    """Register the source the way the console registers a change-capture connector: `POST
    /api/connectors` with a `file:` credential whose value is the source user's password, then
    `PUT .../ingest-spec` with the `cdc` adapter, the dial and the two tables. That PUT
    provisions the connector in Kafka Connect (`SRC-4` decision S2-e). Returns the connector id.
    `suffix`, `spec` and `password` register another connector (the setup-check step's)."""
    created = API.post(
        f"{API_URL}/api/connectors",
        json={
            "name": f"{src().gate}-demo-cdc{suffix}", "type": src().cdc_type, "direction": "source",
            "host": src().host_for_registry, "credential": {"source": "file", "primary": "password"},
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
    _write_credential_file(derived, password if password is not None else src().password)
    answer = API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec", json=spec if spec is not None else _ingest_spec_body(), timeout=60
    )
    if not answer.ok:
        raise G4Failure(f"set the CDC connector's ingest-spec failed: {answer.status_code} {answer.text[:300]}")
    return connector_id


def _expects_real(step: str) -> bool:
    """False after recording a failure when the registry does not say `real`: this gate runs the
    step for real, so a registry that says otherwise is stale (one list, not two)."""
    if RECORDER is not None and RECORDER.registry.expectation(src().cdc_type, step) == "real":
        return True
    _record(step, "fail", f"ops/gates/coverage.json does not say 'real' for this step but {src().gate} runs it; update the registry")
    return False


def _check_connect(connector_id: str) -> None:
    """`connect`: `POST .../test` answers `supported: true, ok: true` for the source, as it does
    for its SQL twin."""
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
    """`POST .../discovery/{kind}` (SRC-13). A source read by the API itself answers in the call
    (`state` `done` or `failed`); one read by the orchestrator answers `running` and is polled."""
    resp = API.post(f"{API_URL}/api/connectors/{connector_id}/discovery/{kind}", json=body, timeout=135)
    if not resp.ok:
        raise G4Failure(f"POST .../discovery/{kind} answered {resp.status_code} {resp.text[:200]}")
    answer = resp.json()
    deadline = time.time() + 240
    while answer.get("state") == "running" and time.time() < deadline:
        time.sleep(2)
        polled = API.get(f"{API_URL}/api/connectors/{connector_id}/discovery/{kind}", timeout=30)
        if not polled.ok:
            raise G4Failure(f"GET .../discovery/{kind} answered {polled.status_code} {polled.text[:200]}")
        answer = polled.json()
    return answer


def _check_discover(connector_id: str) -> None:
    """`discover`: the schemas hold the source's schema, its tables hold the current-mode table,
    and the preview of that table shows its columns (SRC-13 routes)."""
    if not _expects_real("discover"):
        return
    table = src().discover_table
    try:
        schemas = _discovery(connector_id, "list", {})
        names = [o.get("name") for o in schemas.get("objects") or [] if o.get("kind") == "schema"]
        if schemas.get("state") != "done" or src().schema not in names:
            _record("discover", "fail", f"expected the schemas to hold {src().schema!r}, got state={schemas.get('state')!r} reason={schemas.get('reason')!r} {names[:20]}")
            return
        listing = _discovery(connector_id, "list", {"schema": src().schema})
        preview = _discovery(connector_id, "preview", {"object": table})
    except (requests.RequestException, ValueError, G4Failure) as exc:
        _record("discover", "fail", f"the discovery routes did not answer cleanly ({type(exc).__name__}: {str(exc)[:200]})")
        return
    listed = [o.get("name") for o in listing.get("objects") or []]
    if listing.get("state") != "done" or table not in listed:
        _record("discover", "fail", f"expected state=done listing {table!r}, got state={listing.get('state')!r} reason={listing.get('reason')!r} objects={listed[:20]}")
        return
    found = {c.get("name") for c in preview.get("columns") or []}
    if preview.get("state") != "done" or not DISCOVER_COLUMNS <= found:
        _record("discover", "fail", f"{table!r} preview: state={preview.get('state')!r}, missing columns {sorted(DISCOVER_COLUMNS - found)}, has {sorted(found)}")
        return
    _record("discover", "pass", f"{table} with {len(found)} columns")


def step_register_demo_connector_and_check_connect_discover() -> str | None:
    """SRC-9 task 6, steps `connect` and `discover`. A problem with the registration itself fails
    both cells, with the reason; the connector id is None then, and the gate stops (everything
    after needs the connector)."""
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
    """`SRC-4` decision S2-a, proved on the running stack: Kafka Connect stores and returns the
    connector's config, and the source password must not be in it; the connector names a file
    instead, and that file (memory-only volume) holds the value."""
    resp = requests.get(f"{CONNECT_URL}/connectors/{_connect_name(connector_id)}/config", timeout=15)
    if not resp.ok:
        raise G4Failure(f"GET the connector's config from Kafka Connect answered {resp.status_code}")
    config_text = resp.text
    password = src().password
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


def topic_config_value(answer: dict, topic: str, key: str) -> str | None:
    """`key`'s value in the answer of kafka-python 3.0.11's `KafkaAdminClient.describe_configs` (the
    version the code location pins; its `admin/_configs.py`): a dict
    `{"topic": {<topic name>: {<config key>: {"value": ..., "read_only": ..., ...}}}}`. With
    `config_filter="all"` every key is present, with its effective value, so a key that is missing
    is an unexpected answer, not a default. Returns None for a key that is not in the answer."""
    entry = ((answer.get("topic") or {}).get(topic) or {}).get(key)
    return None if entry is None else entry.get("value")


# What Debezium 3.8 really sets (seen in CI, run 40997876457): `KafkaSchemaHistory` creates the topic
# with `retention.ms` = Java's `Long.MAX_VALUE`, not -1. It is Debezium's own spelling of "keep
# forever" (about 292 million years); only a value that expires is a failure. The broker default,
# 7 days = 604800000 ms, is what the topic would inherit if Debezium had not set it.
UNLIMITED_RETENTION_MS = 9_223_372_036_854_775_807


def retention_is_unlimited(value) -> bool:
    """True for -1 (Kafka's "no limit") and for Debezium's `Long.MAX_VALUE` retention.ms."""
    try:
        number = int(str(value))
    except ValueError:
        return False
    return number == -1 or number == UNLIMITED_RETENTION_MS


def step_schema_history_never_expires(connector_id: str) -> None:
    """`SRC-4` decision S3-d: a `MySQL` or `SQL Server` connector keeps its schema history in the
    topic `cdc.<id>.schema-history`. Debezium creates it with unlimited retention; if the broker's
    7-day default applied instead, the connector could not restart after a week. Asked of the
    broker (kafka-python's admin client): `retention.ms` and `retention.bytes` must both be unlimited
    (-1, or Debezium's `Long.MAX_VALUE` for retention.ms: see `retention_is_unlimited`).
    If they are not, this gate fails: the answer is never to raise the broker default."""
    from kafka.admin import ConfigResource, ConfigResourceType, KafkaAdminClient

    topic = f"cdc.{connector_id}.schema-history"
    admin = KafkaAdminClient(bootstrap_servers=[KAFKA_BOOTSTRAP], request_timeout_ms=30_000)
    try:
        def check():
            if topic not in admin.list_topics():
                raise G4Failure(f"the schema-history topic {topic} does not exist yet")
            # `config_filter="all"`: the effective value of every key, not only the modified ones.
            answer = admin.describe_configs([ConfigResource(ConfigResourceType.TOPIC, topic)], config_filter="all")
            values = {key: topic_config_value(answer, topic, key) for key in ("retention.ms", "retention.bytes")}
            if any(v is None for v in values.values()):
                # The topic exists but the answer does not hold what was asked: say so, with the
                # whole answer (a topic's config is not secret).
                raise G4Failure(f"the topic {topic} exists but its config answer has no retention keys: {answer!r}"[:1500])
            return values

        entries = _wait_for(f"the schema-history topic {topic} exists", check, 120, interval_s=5)
    finally:
        admin.close()
    for key in ("retention.ms", "retention.bytes"):
        if not retention_is_unlimited(entries.get(key)):
            raise G4Failure(f"the schema-history topic {topic} has {key}={entries.get(key)!r}, which expires (the broker default is 7 days)")
    print(f"[g4] the schema-history topic {topic} has unlimited retention (retention.ms={entries['retention.ms']}, retention.bytes={entries['retention.bytes']})", flush=True)


# ── Comparing Bronze with the source ────────────────────────────────────────


def _norm(row):
    return (int(row[0]), row[1], decimal.Decimal(row[2]), row[3])


def _current_diff() -> str | None:
    """None when the current-mode Bronze table holds exactly the source's rows (id, customer,
    amount, status), else what differs. A bare `count()` is never used (see the module doc)."""
    table = src().qualified(TABLES["current"])
    source = [_norm(r) for r in src().rows(f"SELECT id, customer, amount, status FROM {table} ORDER BY id")]
    found = [_norm(r) for r in ch_rows(f"SELECT id, customer, amount, status FROM {bronze('current')} WHERE id > 0 ORDER BY id")]
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
        text = ch_query(f"SHOW TABLES FROM {src().catalog_db} SETTINGS allow_database_iceberg=1 FORMAT TabSeparated")
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


# ── Lag and setup checks (SRC-4 slice 4, decision S4-h) ─────────────────────


def _say(step: str, outcome: str, detail: str = "") -> None:
    """Print a step's result like `_record` does. These steps are not columns of the connector
    gate table (`ops/gates/coverage.json` gates the four steps of SRC-9); they fail the gate by
    name and print their numbers here, for the job log."""
    print(f"[g4] {src().cdc_type} {step}: {outcome}{' (' + detail + ')' if detail else ''}", flush=True)


def _capture(connector_id: str) -> dict:
    resp = API.get(f"{API_URL}/api/connectors/{connector_id}/change-capture", timeout=15)
    if not resp.ok:
        raise G4Failure(f"GET .../change-capture answered {resp.status_code} {resp.text[:200]}")
    body = resp.json()
    if body.get("supported") is not True or not isinstance(body.get("lag"), dict):
        raise G4Failure(f"the capture route carries no lag: {str(body)[:300]}")
    return body


def _lag_words(capture: dict) -> str:
    return ", ".join(
        f"{t['table']} {t['status']}" + (f" {t['seconds']}s" if t.get("seconds") is not None else "")
        for t in capture["lag"]["tables"]
    )


def _wait_up_to_date(connector_id: str, timeout_s: float) -> str:
    """Both tables read "up to date" on the capture route: nothing waits to be applied and the
    capture runs. Returns the lag line."""
    def check():
        capture = _capture(connector_id)
        tables = capture["lag"]["tables"]
        if len(tables) == len(MODES) and all(t["status"] == "up_to_date" for t in tables):
            return _lag_words(capture)
        raise G4Failure(f"not up to date yet: {_lag_words(capture)} (state {capture.get('state')})")

    return _wait_for("both tables read up to date on the capture route", check, timeout_s, interval_s=5)


def step_lag_is_up_to_date_after_the_first_copy(connector_id: str) -> None:
    """AC7, first half: after the first copy the capture route says "up to date" for both tables."""
    try:
        words = _wait_up_to_date(connector_id, UP_TO_DATE_TIMEOUT_SECONDS)
    except G4Failure as exc:
        _say("lag", "fail", str(exc))
        raise
    _say("lag", "pass", f"after the first copy: {words}")


def _test_setup_checks(connector_id: str) -> list[dict]:
    resp = API.post(f"{API_URL}/api/connectors/{connector_id}/test", timeout=60)
    if not resp.ok:
        raise G4Failure(f"POST .../test answered {resp.status_code} {resp.text[:200]}")
    body = resp.json()
    if body.get("supported") is not True or body.get("ok") is not True:
        raise G4Failure(f"the connection test did not pass, so no setup check was read: {str(body)[:300]}")
    checks = body.get("setupChecks")
    if not isinstance(checks, list) or not checks:
        raise G4Failure(f"the test answer carries no setupChecks: {str(body)[:300]}")
    return checks


def _describe(checks: list[dict]) -> str:
    return "; ".join(f"{c['setting']}={c['status']}" for c in checks)


def _expect_rest_ok(checks: list[dict], except_setting: str | None) -> None:
    """Every check but `except_setting` is `ok`, or `unchecked` where the source's documented
    login cannot read it."""
    for check in checks:
        if check["setting"] == except_setting or check["status"] == "ok":
            continue
        if check["status"] == "unchecked" and src().unchecked_is_expected(check):
            continue
        raise G4Failure(f"setup check {check['setting']!r} is {check['status']!r}: {check.get('message')!r} ({_describe(checks)})")


def step_setup_checks(connector_id: str) -> None:
    """AC3: the connection test lists the setup checks. The gate's own source passes all of
    them; a second connector whose source lacks ONE setting gets that check `missing` with the
    fixed sentence and a command, and the rest `ok` (or `unchecked` where the documented login
    cannot read the setting)."""
    try:
        good = _test_setup_checks(connector_id)
        # Slice 4 review SHOULD-FIX 10c: the settings listed are exactly this source's, so a
        # dropped check fails the gate.
        listed = sorted(c["setting"] for c in good)
        if not src().expected_settings or listed != sorted(src().expected_settings):
            raise G4Failure(f"the setup checks list {listed}, expected {sorted(src().expected_settings)}")
        _expect_rest_ok(good, None)
        case = src().missing_case()
        spec = {"adapter": "cdc", "ingestMode": "cdc", "dial": case.dial, "sourceObjects": case.source_objects}
        other = _register_demo_cdc_connector(_ensure_gate_tenant(), suffix="-missing", spec=spec, password=case.password)
        try:
            checks = _test_setup_checks(other)
            found = [c for c in checks if c["status"] == "missing"]
            if len(found) != 1 or found[0]["setting"] != case.setting:
                raise G4Failure(f"expected exactly {case.setting!r} to be missing, got {_describe(checks)}")
            if found[0].get("message") != case.message:
                raise G4Failure(f"the missing check says {found[0].get('message')!r}, expected {case.message!r}")
            if not found[0].get("command"):
                raise G4Failure(f"the missing check {case.setting!r} names no command")
            _expect_rest_ok(checks, case.setting)
        finally:
            deleted = API.delete(f"{API_URL}/api/connectors/{other}", timeout=120)
            if not deleted.ok:
                print(f"[g4] cleanup: DELETE of the setup-check connector answered {deleted.status_code}", flush=True)
    except G4Failure as exc:
        _say("setup checks", "fail", str(exc))
        raise
    _say("setup checks", "pass", f"gate source: {_describe(good)}; other connector: {case.setting!r} missing alone, the rest ok")


def _alert_instances(rule_id: str) -> list[dict]:
    resp = API.get(f"{API_URL}/api/overview/alerts", timeout=15)
    if not resp.ok:
        raise G4Failure(f"GET /api/overview/alerts answered {resp.status_code} {resp.text[:200]}")
    return [a for a in resp.json() if a.get("ruleId") == rule_id]


def _ask_ci_job(marker: str, done_file: str, what: str) -> None:
    """Print `marker` and wait for the CI job to do `what` and create `done_file` (the runner has
    no Docker access; see the module doc)."""
    if os.path.exists(done_file):
        os.remove(done_file)
    print(marker, flush=True)
    _wait_for(f"the CI job {what}", lambda: True if os.path.exists(done_file) else None, RESTART_WAIT_SECONDS, interval_s=2)


def _wait_lag_over_the_limit(connector_id: str) -> int:
    def check():
        capture = _capture(connector_id)
        worst = capture["lag"].get("worst")
        if worst and worst["seconds"] > LAG_LIMIT_MINUTES * 60:
            return int(worst["seconds"])
        raise G4Failure(f"lag not over the limit yet: {_lag_words(capture)} (state {capture.get('state')})")

    return _wait_for(f"the lag passes {LAG_LIMIT_MINUTES * 60}s on the capture route", check, 300, interval_s=5)


def step_lag_alert_fires_once_and_closes(connector_id: str) -> None:
    """`g4-cdc` only (AC7, S4-h). Limit 1 minute; the CI job stops `kafka-connect`; changes are
    written meanwhile. The capture route reports a lag above 60 seconds and ONE `connector_lag`
    instance exists naming the connector, a table and the limit. The CI job starts `kafka-connect` again; the
    tables come back to "up to date" and the alert closes (`alertOpen` false).

    Two limits of what a gate can see, stated rather than hidden: the Alerts list dedupes a rule's
    instances for 15 minutes (`overview::insert_from_fired_rule`), so the "back under" alert is
    delivered but not listed as a second row, and the gate sees it as `alertOpen` going false; and
    the lag outlasts the outage by design (it counts from the last moment the table was up to
    date)."""
    created = API.post(
        f"{API_URL}/api/alerts",
        json={
            "name": "g4 connector lag", "type": "connector_lag", "connector": connector_id,
            "channel": "webhook", "target": LAG_ALERT_SINK, "severity": "high",
        },
        timeout=15,
    )
    if not created.ok:
        raise G4Failure(f"create the connector_lag rule answered {created.status_code} {created.text[:300]}")
    rule_id = (created.json().get("rule") or {}).get("id")
    if not rule_id:
        raise G4Failure(f"the connector_lag rule answer carries no id: {created.text[:300]}")
    try:
        limit = API.put(f"{API_URL}/api/connectors/{connector_id}/lag-limit", json={"minutes": LAG_LIMIT_MINUTES}, timeout=15)
        if not limit.ok or limit.json().get("minutes") != LAG_LIMIT_MINUTES:
            raise G4Failure(f"PUT .../lag-limit answered {limit.status_code} {limit.text[:200]}")
        _wait_up_to_date(connector_id, UP_TO_DATE_TIMEOUT_SECONDS)
        if _alert_instances(rule_id):
            raise G4Failure("an alert instance exists before any lag")

        _ask_ci_job(STOP_MARKER, STOP_DONE_FILE, "stopped kafka-connect")
        now = src().now
        _apply(
            lambda t: f"UPDATE {t} SET customer = 'lag_u', updated_at = {now} WHERE id BETWEEN 16001 AND 16500",
            lambda: _history_events(range(16_001, 16_501), "u", "lag_u"),
        )
        over = _wait_lag_over_the_limit(connector_id)
        instances = _wait_for("one connector_lag alert instance", lambda: _alert_instances(rule_id) or None, 240, interval_s=5)
        if len(instances) != 1:
            raise G4Failure(f"expected one connector_lag instance for one crossing, got {len(instances)}")
        if _capture(connector_id)["lag"].get("alertOpen") is not True:
            raise G4Failure("the lag passed the limit and an alert was listed, but alertOpen is not true")
        # Slice 4 review SHOULD-FIX 10a: what is listed names the connector, a selected table and the
        # limit, in the API's fixed words. ("Once per crossing" rests on the compare-and-set flag and
        # its tests: the Alerts list dedupes for 15 minutes, so a count here could not fail.)
        detail = str(instances[0].get("detail", ""))
        tables = [src().object_name(mode) for mode in MODES]
        if connector_id not in detail or not any(t in detail for t in tables) or f"over the limit of {LAG_LIMIT_MINUTES} min" not in detail:
            raise G4Failure(f"the listed alert does not name the connector, a table and the limit: {detail!r}")

        _ask_ci_job(START_MARKER, START_DONE_FILE, "started kafka-connect")
        _wait_capture_state(connector_id, "running", 240)
        _wait_equal("after kafka-connect was stopped and started during changes", CATCH_UP_TIMEOUT_SECONDS)
        words = _wait_up_to_date(connector_id, UP_TO_DATE_TIMEOUT_SECONDS)

        def closed():
            return True if _capture(connector_id)["lag"].get("alertOpen") is False else None

        _wait_for("the lag alert closes (back under the limit)", closed, 120, interval_s=5)
        final = _alert_instances(rule_id)
    except G4Failure as exc:
        _say("lag alert", "fail", str(exc))
        raise
    finally:
        API.delete(f"{API_URL}/api/alerts", params={"id": rule_id}, timeout=15)
    _say(
        "lag alert", "pass",
        f"limit {LAG_LIMIT_MINUTES} min, kafka-connect stopped: lag {over}s, {len(instances)} instance ({detail[:120]!r}); "
        f"started again: {words}, alert closed, {len(final)} instance listed (15-minute dedupe)",
    )


# ── Changes ─────────────────────────────────────────────────────────────────


def _history_events(ids, op: str, customer) -> None:
    for i in ids:
        EXPECTED_HISTORY[(i, op, customer(i) if callable(customer) else customer)] += 1


def _apply(sql_by_table, history_events=None) -> None:
    """Run one statement per table (the same change on both) and record what the history table
    must come to hold."""
    for mode in MODES:
        src().execute(sql_by_table(src().qualified(TABLES[mode])))
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
    """SRC-9 task 6, step `reload`, and `SRC-4` decision D5: an insert, an update and a delete
    each visible in Bronze, in both modes, within 120 seconds."""
    now = src().now
    new_id, upd_id, del_id = 900_001, 1, SEED_ROWS
    changes = [
        ("insert", new_id, "fresh_i",
         lambda t: f"INSERT INTO {t} (id, customer, amount, status, updated_at) VALUES ({new_id}, 'fresh_i', 5.00, 'fresh', {now})",
         lambda: _history_events([new_id], "c", "fresh_i")),
        ("update", upd_id, "fresh_u",
         lambda t: f"UPDATE {t} SET customer = 'fresh_u', updated_at = {now} WHERE id = {upd_id}",
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
    now = src().now
    _apply(
        lambda t: f"UPDATE {t} SET customer = 'bulk_u', updated_at = {now} WHERE id BETWEEN 1 AND 10000",
        lambda: _history_events(range(1, 10_001), "u", "bulk_u"),
    )
    _apply(
        lambda t: f"DELETE FROM {t} WHERE id BETWEEN 10001 AND 15000",
        lambda: _history_events(range(10_001, 15_001), "d", lambda i: f"seed_{i}"),
    )
    _apply(
        lambda t: f"INSERT INTO {t} (id, customer, amount, status, updated_at) SELECT g, 'bulk_i', 5.00, 'bulk', {now} FROM ({src().numbers(100_001, 105_000)}) AS n",
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
    """Restart the worker while changes are being written. The runner cannot restart a container;
    the CI job does (see the module doc): the runner prints `RESTART_MARKER` and waits for
    `RESTART_DONE_FILE`. The writer keeps writing across the restart, so some changes are made
    while the worker is down."""
    if os.path.exists(RESTART_DONE_FILE):
        os.remove(RESTART_DONE_FILE)
    started = threading.Event()
    errors: list[BaseException] = []
    now = src().now

    def writer() -> None:
        try:
            for k in range(18):
                lo = 15_001 + 250 * k
                hi = lo + 249
                _apply(
                    lambda t, lo=lo, hi=hi, k=k: f"UPDATE {t} SET customer = 'stream_{k}', updated_at = {now} WHERE id BETWEEN {lo} AND {hi}",
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
    src().add_column_and_observe()

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
    connector and the password file (and what the source keeps for it: see `Source`)."""
    src().before_delete(connector_id)
    deleted = API.delete(f"{API_URL}/api/connectors/{connector_id}", timeout=120)
    if not deleted.ok:
        raise G4Failure(f"DELETE /api/connectors/{connector_id} answered {deleted.status_code} {deleted.text[:300]}")

    def gone():
        in_connect = requests.get(f"{CONNECT_URL}/connectors/{_connect_name(connector_id)}", timeout=10).status_code
        file_left = os.path.exists(os.path.join(CDC_SECRETS_DIR, connector_id))
        if in_connect == 404 and not file_left:
            return True
        raise G4Failure(f"after DELETE: Kafka Connect answers {in_connect} for the connector, password file left={file_left}")

    _wait_for("the connector and its password file are gone", gone, 90, interval_s=3)
    src().after_delete(connector_id)


def run(source: Source, before_delete=None, after_delete=None) -> int:
    """Run the gate for `source`. `before_delete(connector_id)` runs after the schema change and
    before the delete (PostgreSQL's WAL alert); `after_delete()` runs after it (PostgreSQL's OIDC
    round trip, which is independent of the connector)."""
    global SOURCE, RECORDER
    SOURCE = source
    try:
        RECORDER = gate_summary.Recorder(gate_summary.Registry.load(), source.gate)
        owner = RECORDER.registry.gate_of(source.cdc_type)
        if owner != source.gate:
            raise G4Failure(f"ops/gates/coverage.json gives {source.cdc_type!r} to gate {owner!r}, but {source.gate} runs it")
        step_wait_for_services()
        step_login()
        step_seed_source()
        step_create_catalog_database()
        connector_id = step_register_demo_connector_and_check_connect_discover()
        if connector_id is None:
            raise G4Failure("the demo connector could not be registered, so nothing below can run")
        _wait_capture_state(connector_id, "running", 240)
        step_connector_holds_no_password(connector_id)
        if source.needs_schema_history:
            step_schema_history_never_expires(connector_id)
        step_first_copy_reached_bronze()
        step_lag_is_up_to_date_after_the_first_copy(connector_id)
        step_setup_checks(connector_id)
        step_changes_visible_within_budget()
        step_pause_and_resume_lose_nothing(connector_id)
        step_restart_kafka_connect_during_changes(connector_id)
        if source.lag_alert_gate:
            step_lag_alert_fires_once_and_closes(connector_id)
        step_schema_change_is_observed(connector_id)
        if before_delete is not None:
            before_delete(connector_id)
        step_delete_removes_everything(connector_id)
        if after_delete is not None:
            after_delete()
        # SRC-9 task 6: connect and discover record a failure and let the gate run on; the
        # verdict comes here, once, naming each.
        failed = RECORDER.failures()
        if failed:
            raise G4Failure("connector gate steps failed: " + "; ".join(f"{t} {s}: {d}" for t, s, d in failed))
    except G4Failure as exc:
        print(f"[g4] FAILED: {exc}", file=sys.stderr, flush=True)
        return 1
    finally:
        # The table is printed on every path, so a failed run still says which step broke and
        # which never ran.
        if RECORDER is not None:
            RECORDER.emit()
    print("[g4] PASS", flush=True)
    return 0
