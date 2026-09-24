#!/usr/bin/env python3
"""G6 acceptance test (Tier 1 + Tier 2 ingestion): one connector
per adapter (sql x mysql, sql x mssql, sql x postgres, files x csv, rest,
mongodb, kafka) is registered, given an ingest-spec, run via
POST .../ingest/run, and its rows verified in Bronze through ClickHouse --
the real, end-to-end proof every adapter this workstream ships actually
loads data, not just that its unit tests pass in isolation.

Login/session shape duplicated from ops/g3a/g3a_test.py's step_login/API
(:60-149) -- every /api/connectors/* and /api/connectors/{id}/ingest/run
route is Policy::RequiresAuth or stricter. G6Failure/_wait_for duplicated
from ops/g4/g4_test.py's G4Failure/_wait_for shape. Both duplications
follow this repository's existing precedent of NOT sharing code between
standalone gate scripts (ch_query/_wait_for are already duplicated,
near-verbatim, between g3a_test.py and g4_test.py) --
docs/superpowers/plans/2026-09-11-ws5-platform-signals.md's plan for
extending ops/g4/g4_test.py duplicates the SAME g3a_test.py login block
into g4_test.py for the identical reason.

# Credential shape (ADR 0002 Addendum 3)

`POST /api/connectors` no longer accepts a client-chosen `secretRef` --
`docs/adr/0002-secretref-resolution.md`'s third addendum. The body now
carries `credential: {source, primary, secondary?}`; the server derives
the connector's actual reference name from its OWN generated id and hands
it back once, in the create response's `credential` field. This gate
requests `source: "file"` for every connector whose credential must
actually RESOLVE (everything the ingest matrix runs for real), and writes
the resolved value under the derived name's basename into
`/gate-secrets` -- the writable half of the `g6_secrets` volume
(`ops/g6/docker-compose.g6.override.yml`), whose OTHER, read-only half is
mounted at `/run/secrets` in both `lakehouse-api` and
`dagster-code-location` (the service that actually EXECUTES a launched
ingest run -- see the run-launcher comment above `dagster-code-location`
in docker-compose.yml). `_write_credential_file` never builds a path from
anything but a name the server itself returned, and refuses anything not
shaped `file:/run/secrets/connector_*`.

For connectors whose ingest/run NEVER resolves a secret at all (the
`kafka`, `auth.type = "none"` PLAINTEXT fixture; the CDC-template-only
`g6-mongo-cdc` connector below, whose route only checks the ref's
`env:`-scheme SHAPE, never its value -- `routes::connectors::
debezium_properties`, read in full), this gate still supplies a
syntactically valid `credential` (the field is required on
`POST /api/connectors`) but never provisions a real value for it -- that
would be pretending a credential exists where the code path never reads
one (AGENTS.md principle 2: never fabricate).
"""
from __future__ import annotations

import json
import os
import secrets
import subprocess
import sys
import time

import requests

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
LAKEKEEPER_CATALOG_URI = os.environ.get("CH_LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog")
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
CH_RUSTFS_S3_ENDPOINT = os.environ.get("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000")
# R1 (ADR 0011): ClickHouse's `catalog_credential` setting only accepts the
# Iceberg REST spec's OAuth2 `client_id:client_secret` form, never a raw
# static token -- `CH_OAUTH_SERVER_URI` points it at `ops/oidc-mock`'s
# `/token` endpoint instead of Lakekeeper's own (unverified) one. Empty on
# a pre-R1 or authz-disabled stack, where no auth settings are added.
# Duplicated from ops/g3a/g3a_test.py's identical `CH_OAUTH_CLIENT_ID`/
# `CH_OAUTH_SERVER_URI`/`ch_auth_settings` -- this repository's existing
# precedent of not sharing code between standalone gate scripts.
CH_OAUTH_CLIENT_ID = os.environ.get("CH_OAUTH_CLIENT_ID", "")
CH_OAUTH_SERVER_URI = os.environ.get("CH_OAUTH_SERVER_URI", "")


def ch_auth_settings() -> str:
    if not CH_OAUTH_CLIENT_ID:
        return ""
    return (
        f", catalog_credential = '{CH_OAUTH_CLIENT_ID}:unused', "
        f"oauth_server_uri = '{CH_OAUTH_SERVER_URI}'"
    )
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")
MYSQL_ROOT_PASSWORD = os.environ.get("MYSQL_ROOT_PASSWORD", "")
CONNECTOR_MYSQL_PASSWORD = os.environ.get("CONNECTOR_MYSQL_PASSWORD", "")
MSSQL_SA_PASSWORD = os.environ.get("MSSQL_SA_PASSWORD", "")
MONGO_G6_ROOT_USER = os.environ.get("MONGO_G6_ROOT_USER", "")
MONGO_G6_ROOT_PASSWORD = os.environ.get("MONGO_G6_ROOT_PASSWORD", "")
RUSTFS_ACCESS_KEY = os.environ.get("RUSTFS_ACCESS_KEY", "rustfsadmin")
RUSTFS_SECRET_KEY = os.environ.get("RUSTFS_SECRET_KEY", "rustfsadmin")
CATALOG_DB = "icecat_g6"

GATE_SECRETS_DIR = "/gate-secrets"
FILE_REF_PREFIX = "file:/run/secrets/connector_"

API = requests.Session()


class G6Failure(Exception):
    pass


def _wait_for(name: str, check, timeout_s: int, interval_s: float = 2.0):
    deadline = time.time() + timeout_s
    last_err = None
    while time.time() < deadline:
        try:
            value = check()
            if value:
                print(f"[g6] ready: {name}")
                return value
        except Exception as exc:  # noqa: BLE001
            last_err = exc
        time.sleep(interval_s)
    raise G6Failure(f"timed out waiting for {name!r}: {last_err}")


def ch_query(sql: str) -> str:
    resp = requests.post(CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=30)
    if not resp.ok:
        raise G6Failure(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


def step_wait_for_services() -> None:
    _wait_for("ClickHouse", lambda: ch_query("SELECT 1 FORMAT TabSeparated").strip() == "1", 60)
    _wait_for("lakehouse-api", lambda: API.get(f"{API_URL}/health", timeout=5).status_code == 200, 60)
    _wait_for(
        "Dagster webserver",
        lambda: requests.get(DAGSTER_URL.replace("/graphql", "/server_info"), timeout=5).status_code == 200,
        90,
    )
    # The webserver answering is not the code location having loaded:
    # compose can recreate `dagster-code-location` when this runner starts,
    # and an `ingest/run` in that window fails with PipelineNotFoundError
    # (seen in CI). Wait until Dagster itself lists `ingest_job`.
    _wait_for("Dagster code location (ingest_job loaded)", _ingest_job_is_loaded, 120)


def _ingest_job_is_loaded() -> bool:
    query = (
        "{ repositoriesOrError { ... on RepositoryConnection "
        "{ nodes { jobs { name } } } } }"
    )
    resp = requests.post(DAGSTER_URL, json={"query": query}, timeout=5)
    resp.raise_for_status()
    nodes = (resp.json().get("data") or {}).get("repositoriesOrError", {}).get("nodes") or []
    return any(job.get("name") == "ingest_job" for node in nodes for job in node.get("jobs", []))


def step_login() -> None:
    """Duplicated from ops/g3a/g3a_test.py::step_login -- every route
    this gate calls requires an authenticated session."""
    login = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
    if not login.ok:
        raise G6Failure(f"login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = API.post(f"{API_URL}/api/auth/change-password", json={"newPassword": AUTH_PASSWORD}, timeout=10)
        if not rotated.ok:
            raise G6Failure(f"forced password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
        if not relogin.ok:
            raise G6Failure(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")
    print("[g6] logged in as bootstrap admin")


def _seed_mysql_fixture() -> None:
    import pymysql
    conn = pymysql.connect(host="mysql-g6", user="root", password=MYSQL_ROOT_PASSWORD, database="g6_ingest")
    try:
        with conn.cursor() as cur:
            cur.execute("CREATE TABLE IF NOT EXISTS orders (id INT PRIMARY KEY, amount DECIMAL(10,2))")
            cur.execute("DELETE FROM orders")
            cur.executemany("INSERT INTO orders (id, amount) VALUES (%s, %s)", [(1, 10.5), (2, 20.25)])
        conn.commit()
    finally:
        conn.close()


def _seed_mssql_fixture() -> None:
    import pyodbc
    conn_str = (
        "DRIVER={ODBC Driver 18 for SQL Server};SERVER=tcp:mssql-g6,1433;"
        f"UID=sa;PWD={{{MSSQL_SA_PASSWORD}}};DATABASE=master;Encrypt=yes;TrustServerCertificate=yes;"
    )
    conn = pyodbc.connect(conn_str, autocommit=True)
    try:
        cur = conn.cursor()
        cur.execute(
            "IF NOT EXISTS (SELECT * FROM sys.databases WHERE name = 'g6_ingest') CREATE DATABASE g6_ingest"
        )
        cur.execute("USE g6_ingest")
        cur.execute(
            "IF OBJECT_ID('orders') IS NULL CREATE TABLE orders (id INT PRIMARY KEY, amount DECIMAL(10,2))"
        )
        cur.execute("DELETE FROM orders")
        cur.execute("INSERT INTO orders (id, amount) VALUES (1, 10.5), (2, 20.25)")
    finally:
        conn.close()


def _seed_files_fixture() -> None:
    """Calls `ops/g6/seed_files_fixture.py::seed` -- verification
    correction: that module's `seed()` function existed but was never
    actually CALLED anywhere in this repository (confirmed:
    `grep -rn seed_files_fixture docker-compose.yml ops/g6/*.py` found
    only this comment's own predecessor, never an invocation), so
    `landing/orders.csv` was never written and the `g6-files` connector's
    run always failed extracting a nonexistent object. Imported, not
    duplicated -- `ops/g6:/work` is this container's `working_dir`, so
    the sibling module is on `sys.path` directly."""
    import seed_files_fixture
    seed_files_fixture.seed(
        endpoint="http://rustfs:9000", bucket="lakehouse-warehouse",
        access_key=RUSTFS_ACCESS_KEY, secret_key=RUSTFS_SECRET_KEY,
    )


def _seed_mongo_fixture() -> str:
    """Connects as the root user mongo-g6's override provisions
    (MONGO_G6_ROOT_USER/PASSWORD, admin-authenticated), seeds
    `g6_ingest.orders`, and creates a DATABASE-scoped reader user.

    Verification correction: an earlier revision had the `g6-mongo`
    connector dial with the root user directly and
    `authSource=g6_ingest` (`MongoDial.database` doubles as the auth
    source by design -- `rust/crates/lakehouse-store/src/ingest_spec.rs`'s
    `MongoDial` doc comment: "pymongo also takes this as the auth
    source"). The root user is only provisioned in the `admin` database
    (`MONGO_INITDB_ROOT_USERNAME`'s standard semantics), so authenticating
    it against `g6_ingest` fails outright
    (`{'codeName': 'AuthenticationFailed'}`, reproduced against this
    stack). A reader user created ON `g6_ingest` (this function, below)
    is required for the dial's own documented auth-source shape to work
    at all -- returns the generated reader password, written into the
    connector's credential file by the caller."""
    import secrets as _secrets
    from pymongo import MongoClient
    reader_password = _secrets.token_hex(16)
    client = MongoClient(
        host=["mongo-g6:27017"],
        username=MONGO_G6_ROOT_USER,
        password=MONGO_G6_ROOT_PASSWORD,
        authSource="admin",
        directConnection=True,
    )
    try:
        db = client["g6_ingest"]
        coll = db["orders"]
        coll.delete_many({})
        # Default (auto-generated) ObjectId `_id`s -- the real shape
        # every actual Mongo collection has. `adapters/mongodb.py::
        # _collection_rows` now converts BSON-specific values (ObjectId,
        # Decimal128, Binary) to JSON-safe ones before a row reaches the
        # sink (`2017536`), so this fixture no longer needs to dodge the
        # default `_id` shape the way an earlier revision did.
        coll.insert_many([{"id": 1, "amount": 10.5}, {"id": 2, "amount": 20.25}])
        db.command("dropUser", "g6_reader") if "g6_reader" in [
            u["user"] for u in db.command("usersInfo")["users"]
        ] else None
        db.command("createUser", "g6_reader", pwd=reader_password, roles=[{"role": "readWrite", "db": "g6_ingest"}])
    finally:
        client.close()
    return reader_password


def _produce_kafka_fixture(topic: str, *, count: int = 30, interval_s: float = 1.5) -> None:
    """Produces `count` JSON messages onto `kafka-g6`, one per
    `interval_s`, spread out AFTER the caller has already launched the
    kafka connector's run -- never before. `adapters/kafka.py`'s
    `KafkaConsumer` is constructed with no `auto_offset_reset` override,
    so kafka-python's own default (`"latest"`) applies: a brand-new
    consumer group has no committed offset and starts consuming from
    whatever is produced from the moment it first polls onward, NOT from
    messages already sitting in the topic. Spreading production across
    the run's `microBatchSeconds` window (set generously below) is what
    makes at least one message land inside the consumer's actual poll
    loop, rather than requiring exact ordering this gate cannot control
    (the consumer is constructed inside an async-launched Dagster run,
    not synchronously from this script)."""
    from kafka import KafkaProducer
    producer = KafkaProducer(bootstrap_servers=["kafka-g6:9092"], security_protocol="PLAINTEXT")
    try:
        for i in range(count):
            row = {"id": i, "amount": 10.0 + i}
            producer.send(topic, json.dumps(row).encode("utf-8"))
            producer.flush()
            time.sleep(interval_s)
    finally:
        producer.close()


def _write_credential_file(ref_name: str, value: str) -> None:
    """Validate `ref_name` is a `file:/run/secrets/connector_*` derived
    reference (ADR 0002 Addendum 3) BEFORE writing `value` under its
    basename into `/gate-secrets` -- never build a path from anything but
    this validated prefix."""
    if not ref_name.startswith(FILE_REF_PREFIX):
        raise G6Failure(f"refusing to write a credential file for an unexpected ref shape: {ref_name!r}")
    basename = ref_name.rsplit("/", 1)[-1]
    path = os.path.join(GATE_SECRETS_DIR, basename)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(value)
    os.chmod(path, 0o644)


def _create_connector(*, name: str, kind: str, host: str, credential: dict) -> dict:
    created = API.post(
        f"{API_URL}/api/connectors",
        json={
            "name": name, "type": kind, "direction": "source", "host": host, "credential": credential,
            "environment": "production", "tenant": "g6", "residency": "", "capabilities": [],
        },
        timeout=10,
    )
    if not created.ok:
        raise G6Failure(f"create connector {name!r} failed: {created.status_code} {created.text}")
    return created.json()


def _register_and_run(
    *,
    name: str,
    kind: str,
    host: str,
    credential: dict,
    credential_values: dict | None,
    adapter: str,
    dial: dict,
    source_objects: list,
    ingest_mode: str = "batch",
) -> str | None:
    """POST /api/connectors (writing the derived `file:` credential's
    real value, if any, from `credential_values`), PUT .../ingest-spec,
    POST .../ingest/run. Returns the launched runId, or None for an
    honest supported:false response (CDC/Sheets)."""
    body = _create_connector(name=name, kind=kind, host=host, credential=credential)
    connector_id = body["id"]
    derived = body.get("credential") or {}
    if credential.get("source") == "file" and credential_values:
        if credential_values.get("primary") and derived.get("primary"):
            _write_credential_file(derived["primary"], credential_values["primary"])
        if credential_values.get("secondary") and derived.get("secondary"):
            _write_credential_file(derived["secondary"], credential_values["secondary"])

    spec_resp = API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={"adapter": adapter, "ingestMode": ingest_mode, "dial": dial, "sourceObjects": source_objects},
        timeout=10,
    )
    if not spec_resp.ok:
        raise G6Failure(f"set ingest-spec for {name!r} failed: {spec_resp.status_code} {spec_resp.text}")

    run_resp = API.post(f"{API_URL}/api/connectors/{connector_id}/ingest/run", timeout=10)
    if not run_resp.ok:
        raise G6Failure(f"ingest/run for {name!r} failed: {run_resp.status_code} {run_resp.text}")
    result = run_resp.json()
    if result.get("supported") is False:
        print(f"[g6] {name!r}: supported=false ({result.get('reason')})")
        return None
    run_id = result.get("runId")
    if not run_id:
        raise G6Failure(f"ingest/run for {name!r} returned no runId: {result}")
    print(f"[g6] launched run {run_id} for {name!r}")
    return run_id


def step_wait_for_run_success(run_id: str) -> None:
    """Duplicated from ops/g3a/g3a_test.py::step_wait_for_run_success
    (:177-198) -- identical Dagster GraphQL run-status polling."""
    query = "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename ... on Run { status } } }"

    def check() -> bool:
        resp = requests.post(DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10)
        resp.raise_for_status()
        run = resp.json().get("data", {}).get("pipelineRunOrError", {})
        status = run.get("status")
        print(f"[g6] run {run_id} status={status}")
        if status == "FAILURE":
            raise G6Failure(f"run {run_id} FAILED")
        return status == "SUCCESS"

    _wait_for(f"run {run_id} SUCCESS", check, 150, interval_s=3.0)


def _bronze_row_count(bronze_table: str) -> int:
    """Never a bare count() -- see g3a_test.py/g4_test.py's identical
    docs/plans/P5-RESULT.md-cited comment (R11): a merge-on-read Iceberg
    table's metadata-only fast path overcounts on equality deletes."""
    ch_query(
        f"CREATE DATABASE IF NOT EXISTS {CATALOG_DB} ENGINE = DataLakeCatalog('{LAKEKEEPER_CATALOG_URI}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{LAKEKEEPER_WAREHOUSE}', "
        f"storage_endpoint = '{CH_RUSTFS_S3_ENDPOINT}'{ch_auth_settings()} "
        "SETTINGS allow_database_iceberg = 1"
    )
    text = ch_query(
        f"SELECT count() FROM {CATALOG_DB}.`bronze.{bronze_table}` WHERE 1 "
        "SETTINGS allow_database_iceberg = 1 FORMAT TabSeparated"
    )
    return int(text.strip())


def _bronze_ingested_at_null_count(bronze_table: str) -> int:
    """Proves `3fb396d`'s fix (`adapters/sink.py::load_via_sink` now
    stamps `_ingested_at` on every row it writes, for every adapter, not
    only the Postgres-only `run_bronze_ingest` path) actually landed the
    column for THIS gate's own tables -- not just that the run returned
    SUCCESS, which it already did before that fix for every non-SQL
    adapter while silently omitting the ADR 0004 system column
    (`3fb396d`'s own commit message: "succeeded ... writing Bronze tables
    without the ADR 0004 system column"). `countIf(...)`, not a bare
    `count()`, but still carries an explicit `WHERE` (R11 -- this
    repository's `check_bare_iceberg_count.py` lint) for the same
    equality-delete-overcounting reason `_bronze_row_count` does."""
    text = ch_query(
        f"SELECT countIf(_ingested_at IS NULL) FROM {CATALOG_DB}.`bronze.{bronze_table}` WHERE 1 "
        "SETTINGS allow_database_iceberg = 1 FORMAT TabSeparated"
    )
    return int(text.strip())


def step_ingest_matrix() -> None:
    _seed_mysql_fixture()
    _seed_mssql_fixture()
    mongo_reader_password = _seed_mongo_fixture()
    _seed_files_fixture()

    rest_api_key = secrets.token_hex(8)

    connectors = [
        ("g6-mysql", "MySQL", "mysql-g6:3306",
         {"source": "file", "primary": "password"}, {"primary": CONNECTOR_MYSQL_PASSWORD},
         "sql", {"driver": "mysql", "host": "mysql-g6", "port": 3306, "database": "g6_ingest", "user": "g6_reader"},
         [{"name": "orders", "target": "g6_mysql_orders"}]),
        ("g6-mssql", "SQL Server", "mssql-g6:1433",
         {"source": "file", "primary": "password"}, {"primary": MSSQL_SA_PASSWORD},
         "sql", {"driver": "mssql", "host": "mssql-g6", "port": 1433, "database": "g6_ingest", "user": "sa"},
         [{"name": "orders", "target": "g6_mssql_orders"}]),
        # "page" pagination, recordsPath="items" -- the real contract
        # shape (ec25a5d fixed adapters/rest.py to read `recordsPath`,
        # not the never-accepted `dataPath` key, and removed the dead
        # "offset" branch `RestPagination` never admitted). See
        # ops/g6/rest_stub.py's module docstring for the fixture this
        # exercises against.
        # sourceObjects has ONE entry -- verification correction: the
        # original script set this to `[]`. `ingest_factory.py::run_ingest`
        # does `for obj in connector.get("sourceObjects", []):
        # _run_one_object(connector, obj)` -- an EMPTY list means that
        # loop body never runs at all, so the REST adapter's own
        # `build_source` (which ignores `source_objects` -- see
        # `adapters/rest.py`'s own comment on why, it reads `dial.endpoints`
        # instead) is never even CALLED, and the run "succeeds" as a
        # complete no-op that never dials `rest-stub-g6` (reproduced
        # against this stack: a run with `sourceObjects: []` returns
        # SUCCESS in under 3 seconds with no STEP_WORKER/extract activity
        # logged at all). The entry's content is irrelevant to the REST
        # adapter itself (it ignores `source_objects`); `target` is what
        # matters, since `_run_one_object`'s generic sink-write path uses
        # `obj["target"]` as the Bronze table name regardless of adapter.
        ("g6-rest", "REST API", "rest-stub-g6:8080",
         {"source": "file", "primary": "api_key"}, {"primary": rest_api_key},
         "rest",
         {"baseUrl": "http://rest-stub-g6:8080", "auth": {"type": "api_key", "header": "X-Api-Key"},
          "pagination": {"type": "page", "param": "page"},
          "endpoints": [{"path": "/items", "recordsPath": "items"}]},
         [{"name": "items", "target": "g6_rest_items"}]),
        # username="g6_reader" (database-scoped, created by
        # _seed_mongo_fixture), never the admin-scoped root user -- see
        # that function's docstring for why authSource=database (this
        # dial's `database` field, by MongoDial's own documented design)
        # rules out authenticating as root here.
        ("g6-mongo", "MongoDB", "mongo-g6:27017",
         {"source": "file", "primary": "password"}, {"primary": mongo_reader_password},
         "mongodb",
         {"hosts": ["mongo-g6:27017"], "database": "g6_ingest", "username": "g6_reader",
          "directConnection": True},
         [{"name": "orders", "target": "g6_mongo_orders"}]),
    ]
    run_ids: dict[str, str] = {}
    for name, kind, host, credential, credential_values, adapter, dial, objs in connectors:
        run_id = _register_and_run(
            name=name, kind=kind, host=host, credential=credential, credential_values=credential_values,
            adapter=adapter, dial=dial, source_objects=objs,
        )
        if run_id:
            run_ids[name] = run_id

    # Postgres batch: the existing seeded conn-pg-lakehouse (0033),
    # already adapter=sql -- run it directly, no new connector needed.
    pg_run = API.post(f"{API_URL}/api/connectors/conn-pg-lakehouse/ingest/run", timeout=10)
    if not pg_run.ok:
        raise G6Failure(f"ingest/run for conn-pg-lakehouse failed: {pg_run.status_code} {pg_run.text}")
    run_ids["conn-pg-lakehouse"] = pg_run.json()["runId"]

    # files: the fixture ops/g6/seed_files_fixture.py wrote to
    # landing/orders.csv, registered against a NEW test connector -- never
    # conn-s3-warehouse, which rust/migrations/0033_connector_ingest_spec.sql
    # deliberately seeds with an empty source_objects.
    files_run_id = _register_and_run(
        name="g6-files", kind="Object storage", host="rustfs:9000",
        credential={"source": "file", "primary": "access_key", "secondary": "secret_key"},
        credential_values={"primary": RUSTFS_ACCESS_KEY, "secondary": RUSTFS_SECRET_KEY},
        adapter="files",
        # "endpoint" set explicitly -- a real, `FilesDial`-validated
        # optional field (`rust/crates/lakehouse-store/src/ingest_spec.rs`)
        # this gate had simply left out, producing a bogus-credential 403
        # against real AWS S3 (`_default_get_object` has no RustFS
        # fallback when `endpoint` is unset; `adapters/files.py`'s
        # docstring was corrected to say so in `ec25a5d`, not "an absent
        # endpoint dials RustFS", which was never true).
        dial={"protocol": "s3", "endpoint": "http://rustfs:9000", "bucket": "lakehouse-warehouse", "format": "csv"},
        source_objects=[{"name": "landing/orders.csv", "target": "g6_files_orders"}],
    )
    if files_run_id:
        run_ids["g6-files"] = files_run_id

    # kafka: ingestMode="stream" (widened by 0043_ingest_tier2_adapters.sql
    # -- `ingest_factory.py::run_ingest` dispatches "stream" to
    # `_run_stream_connector`, a single bounded consumer.poll() loop over
    # the whole topic, never the per-sourceObjects loop the other
    # adapters above use). auth.type="none" (PLAINTEXT broker) needs NO
    # secret at all (`secret_map.SECRET_FIELD_NAMES[("kafka","none")] ==
    # ()`) -- credential is still supplied (the field is required on
    # create) but no value is ever written for it; see this module's
    # docstring.
    kafka_topic = "g6-events"
    kafka_run_id = _register_and_run(
        name="g6-kafka", kind="Kafka", host="kafka-g6:9092",
        credential={"source": "file", "primary": "token"}, credential_values=None,
        adapter="kafka",
        dial={"bootstrapServers": ["kafka-g6:9092"], "topic": kafka_topic, "auth": {"type": "none"},
              "groupId": "g6-gate-kafka", "microBatchSeconds": 60},
        source_objects=[{"name": kafka_topic, "target": "g6_kafka_events"}],
        ingest_mode="stream",
    )
    if kafka_run_id:
        run_ids["g6-kafka"] = kafka_run_id
        # Produced AFTER the run launches (see _produce_kafka_fixture's
        # own docstring for why "before" would race kafka-python's
        # default "latest" auto_offset_reset).
        _produce_kafka_fixture(kafka_topic)

    for name, run_id in run_ids.items():
        step_wait_for_run_success(run_id)

    for name, bronze_table in [
        ("g6-mysql", "g6_mysql_orders"), ("g6-mssql", "g6_mssql_orders"),
        ("conn-pg-lakehouse", "orders"), ("g6-files", "g6_files_orders"),
        ("g6-mongo", "g6_mongo_orders"), ("g6-kafka", "g6_kafka_events"),
        # g6-rest was missing from this list entirely -- verification
        # correction: without it, a REST run that did nothing (see the
        # `sourceObjects` fix above) and a REST run that genuinely loaded
        # rows were indistinguishable to this gate.
        ("g6-rest", "g6_rest_items"),
    ]:
        # Polled, not a single immediate check: a freshly-created Iceberg
        # table can take a beat to become visible through ClickHouse's
        # DataLakeCatalog engine after the writing run itself already
        # reported SUCCESS (reproduced against this stack for the
        # LAST-completing run in the matrix specifically) -- this is
        # catalog-visibility latency, not a reason to fail a table that
        # really was written.
        count = _wait_for(f"bronze.{bronze_table} rows visible", lambda t=bronze_table: _bronze_row_count(t) or None, 30, interval_s=3.0)
        null_ingested_at = _bronze_ingested_at_null_count(bronze_table)
        if null_ingested_at != 0:
            raise G6Failure(
                f"expected _ingested_at present and non-null on every row of bronze.{bronze_table} "
                f"for {name!r}, got {null_ingested_at} of {count} rows with a null/missing value"
            )
        print(f"[g6] bronze.{bronze_table}: {count} rows, _ingested_at null count: {null_ingested_at}")


def step_column_gate_rejects_an_unsupported_column() -> None:
    """A real, asserted FAILURE -- calls column_gate.reject_unsupported_column_types
    inside the REAL shipped image (not a mocked unit test), asserting it
    exits non-zero for a nested-array-typed column and zero for an
    ordinary one.

    Verification correction: this used to shell out to `docker compose
    ... exec -T dagster-code-location ...` from INSIDE this gate's own
    `g6-test-runner` container -- which has neither a `docker` binary nor
    the host's Docker socket mounted (`FileNotFoundError: [Errno 2] No
    such file or directory: 'docker'`, reproduced against this stack: the
    first time this step was ever actually reached, since every earlier
    step in this gate was broken before now). `g6-test-runner` builds
    from the SAME `dagster/Dockerfile` as `dagster-code-location`
    (docker-compose.yml, both `context: ./dagster`), so it already has
    `dispar_orchestrate` installed locally -- a plain, no-docker-needed
    `python -c` subprocess in THIS container is just as real an assertion
    against "the shipped image" as an exec into the other one would have
    been, without granting this container any Docker socket access."""
    good = subprocess.run(
        ["python", "-c",
         "from dispar_orchestrate.column_gate import reject_unsupported_column_types as f; f([('id','bigint')])"],
        capture_output=True, text=True, check=False,
    )
    if good.returncode != 0:
        raise G6Failure(f"reject_unsupported_column_types wrongly rejected a plain 'bigint' column: {good.stderr}")

    bad = subprocess.run(
        ["python", "-c",
         "from dispar_orchestrate.column_gate import reject_unsupported_column_types as f; f([('bad','text[]')])"],
        capture_output=True, text=True, check=False,
    )
    if bad.returncode == 0:
        raise G6Failure("reject_unsupported_column_types did not reject an array-typed ('text[]') column")
    print("[g6] column_gate.reject_unsupported_column_types: real, asserted pass/fail against the shipped image")


def step_cdc_reports_unsupported_not_a_launch() -> None:
    body = _create_connector(
        name="g6-cdc", kind="PostgreSQL CDC", host="postgres:5432",
        credential={"source": "env", "primary": "password"},
    )
    connector_id = body["id"]
    API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={
            "adapter": "cdc", "ingestMode": "cdc",
            "dial": {"driver": "postgres", "host": "postgres", "port": 5432, "database": "lakehouse",
                     "user": "lakehouse", "slotName": "g6_cdc_slot", "publicationName": "g6_cdc_pub"},
            "sourceObjects": [],
        },
        timeout=10,
    )
    run_resp = API.post(f"{API_URL}/api/connectors/{connector_id}/ingest/run", timeout=10)
    run_body = run_resp.json()
    if run_body.get("supported") is not False or "runId" in run_body:
        raise G6Failure(f"expected a supported:false, no-runId response for a cdc connector, got: {run_body}")
    print(f"[g6] cdc ingest/run: honest supported:false ({run_body.get('reason')})")


def step_cdc_mongo_debezium_properties() -> None:
    """`GET .../debezium-properties` for a `mongodb`-adapter connector --
    NOT `dial.driver = "mongo"` (there is no such `SqlDriver` variant;
    read `rust/crates/lakehouse-api/src/routes/connectors.rs`'s
    `resolve_debezium_source_target` before trusting a plan's snippet on
    this route). The `mongodb` branch there dispatches on the `adapter`
    STRING itself and never resolves the connector's `secretRef` value
    (it only requires the `env:` SCHEME, to interpolate `${...}` into the
    rendered template -- see that function's own doc comment) -- so
    `credential: {source: "env", ...}` is supplied for its required
    shape but never needs a real, provisioned value."""
    body = _create_connector(
        name="g6-mongo-cdc", kind="MongoDB", host="mongo-g6:27017",
        credential={"source": "env", "primary": "password"},
    )
    connector_id = body["id"]
    spec_resp = API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={
            "adapter": "mongodb", "ingestMode": "batch",
            "dial": {"hosts": ["mongo-g6:27017"], "database": "g6_ingest", "username": "g6_reader",
                     "directConnection": True},
            "sourceObjects": [],
        },
        timeout=10,
    )
    if not spec_resp.ok:
        raise G6Failure(f"set ingest-spec for g6-mongo-cdc failed: {spec_resp.status_code} {spec_resp.text}")

    props_resp = API.get(
        f"{API_URL}/api/connectors/{connector_id}/debezium-properties", params={"table": "g6_ingest.orders"},
        timeout=10,
    )
    if not props_resp.ok:
        raise G6Failure(f"debezium-properties for g6-mongo-cdc failed: {props_resp.status_code} {props_resp.text}")
    properties = props_resp.json().get("properties", "")
    if "io.debezium.connector.mongodb.MongoDbConnector" not in properties:
        raise G6Failure(f"expected the MongoDB Debezium connector class in the rendered template, got: {properties}")
    print("[g6] debezium-properties for a mongodb-adapter connector: MongoDbConnector class present")


def step_sheets_reports_unsupported() -> None:
    """`google-auth` is not installed in this image, so the `sheets`
    adapter's own `build_source` reports `supported: false` -- but,
    verification correction, NOT synchronously from
    `POST .../ingest/run` the way `cdc` does (that adapter has no
    Dagster job at all, "no separate trigger" -- see
    `step_cdc_reports_unsupported_not_a_launch`). `sheets` DOES have a
    real `ingest_job`; `POST .../ingest/run` always returns a real
    `runId` for it, and the Dagster run itself reports SUCCESS (the op
    caught the unsupported case internally and returned normally,
    `ingest_factory.py::_run_one_object`'s `elif adapter_name ==
    "sheets": ... if not result.supported: _record(status="unsupported",
    ...); return`) -- the honest `supported: false` this step actually
    proves out lives in the recorded `ingest_run` governance row, not the
    launch response.

    `sourceObjects` has ONE entry, not `[]` -- the SAME bug class as the
    `g6-rest`/linklocal fixes above: an empty list means
    `_run_one_object` (and therefore the sheets-unsupported check inside
    it) never runs at all, so this step's assertion would previously
    have found no governance row whatsoever (reproduced against this
    stack: an empty-`sourceObjects` sheets connector's run reports
    Dagster SUCCESS with zero `ingest_run` rows recorded for it, an
    entirely different, uninformative kind of "success" than the one
    this step means to prove).

    `credential.primary` is chosen as `"token"` here (the closest of the
    six fixed `CredentialKind` suffixes to "a service account key", none
    of which is an exact match) -- and, verification correction, source
    `"file"` with a REAL written value, not `"env"` left unresolved:
    `secret_map.SECRET_FIELD_NAMES[("sheets", None)] == ("serviceAccountJson",)`,
    so `_resolve_object_secrets` (which runs BEFORE the adapter's own
    `build_source`/unsupported check, the same ordering the REST
    link-local test's docstring already documents) requires a resolvable
    credential regardless of the adapter's own eventual
    `supported: false` -- an unresolvable one fails with
    `SecretRefRejected` before ever reaching the check this step means to
    prove (reproduced against this stack). The written value is never
    actually READ as a service-account JSON (the import failure happens
    first), so its content does not need to be valid JSON."""
    body = _create_connector(
        name="g6-sheets", kind="Google Sheets", host="sheets.googleapis.com",
        credential={"source": "file", "primary": "token"},
    )
    connector_id = body["id"]
    derived = (body.get("credential") or {}).get("primary")
    if not derived:
        raise G6Failure(f"create response for g6-sheets carried no derived credential name: {body}")
    _write_credential_file(derived, "not-a-real-service-account-json")
    API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={"adapter": "sheets", "ingestMode": "batch",
              "dial": {"spreadsheetId": "s1", "ranges": ["A1:B2"]},
              "sourceObjects": [{"name": "A1:B2", "target": "g6_sheets_placeholder"}]},
        timeout=10,
    )
    run_resp = API.post(f"{API_URL}/api/connectors/{connector_id}/ingest/run", timeout=10)
    run_body = run_resp.json()
    run_id = run_body.get("runId")
    if not run_id:
        raise G6Failure(f"expected a real runId for sheets (it has a real ingest_job), got: {run_body}")
    step_wait_for_run_success(run_id)

    runs_resp = API.get(f"{API_URL}/api/governance/ingest-runs", params={"connectorId": connector_id}, timeout=30)
    if not runs_resp.ok:
        raise G6Failure(f"ingest-runs query for sheets failed: {runs_resp.status_code} {runs_resp.text}")
    rows = runs_resp.json()
    unsupported = [r for r in rows if r.get("status") == "unsupported"]
    if not unsupported:
        raise G6Failure(f"expected an unsupported ingest_run row for sheets (google-auth not installed), got: {rows}")
    print(f"[g6] sheets ingest/run: honest 'unsupported' governance row ({unsupported[0].get('error')})")


def main() -> int:
    try:
        step_wait_for_services()
        step_login()
        step_ingest_matrix()
        step_column_gate_rejects_an_unsupported_column()
        step_cdc_reports_unsupported_not_a_launch()
        step_cdc_mongo_debezium_properties()
        step_sheets_reports_unsupported()
    except G6Failure as exc:
        print(f"[g6] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g6] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
