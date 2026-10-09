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
# The tenant every connector of this run is created in; set by
# `step_ensure_tenant` right after login.
TENANT_ID = ""
# The tenant that owns the seeded connectors this gate drives
# (`conn-pg-lakehouse`; `rust/migrations/0042_tenant_provisioning.sql`
# backfills the seeded connectors to the seed tenant `meridian-group`,
# `0002_seed_identity.sql`). Per-connector routes answer only for a
# connector in one of the caller's tenants, so the account must be a member
# of this one, and the gate's own connectors are created in it too.
GATE_TENANT_SLUG = "meridian-group"


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
    # (seen in CI). The recreate can ALSO land AFTER this wait -- compose
    # recreates the container when THIS runner starts, so the first launch
    # POST races the recovered gRPC server by milliseconds (seen twice in
    # CI: PR #57 and the R3 merge to main). The launch sites guard against
    # that race directly via `_post_ingest_run`, which waits again on a
    # 422 `PipelineNotFoundError` and retries once. Wait here only until
    # Dagster itself lists `ingest_job`.
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


def _me_tenant_ids() -> list[str]:
    me = API.get(f"{API_URL}/api/auth/me", timeout=10)
    if not me.ok:
        raise G6Failure(f"GET /api/auth/me failed: {me.status_code} {me.text[:300]}")
    return [t["id"] for t in me.json().get("tenants", [])]


def step_ensure_tenant() -> None:
    """Make sure the logged-in account belongs to `GATE_TENANT_SLUG`, and
    remember that tenant's id for creating the gate's connectors.

    The bootstrap admin of a fresh database belongs to no tenant, and a
    connector created by an account with no tenant is stored with no tenant
    id, so every `/api/connectors/{id}/*` route answers 404 for it (per-
    connector routes are tenant-scoped, `ensure_connector_in_tenants`: the
    connector's tenant must be one of the caller's). The seeded
    `conn-pg-lakehouse` the matrix also runs belongs to `meridian-group`
    (0042), so that is the tenant the account joins, even when it already
    has others: with `PUT /api/identity/users/{id}/tenants/{tenant_id}`
    (the unrestricted admin may add itself to any tenant), then
    `/api/auth/me` is read again to confirm. Membership is read per
    request, so no new login. An account already in it changes nothing.

    Duplicated, not shared, in ops/g6/g6_linklocal_test.py: the two gates
    share no module (see the module docstring)."""
    global TENANT_ID
    me = API.get(f"{API_URL}/api/auth/me", timeout=10)
    if not me.ok:
        raise G6Failure(f"GET /api/auth/me failed: {me.status_code} {me.text[:300]}")
    me = me.json()
    listed = API.get(f"{API_URL}/api/identity/tenants", timeout=10)
    if not listed.ok:
        raise G6Failure(f"list tenants failed: {listed.status_code} {listed.text[:300]}")
    gate_tenant = next((t for t in listed.json() if t["slug"] == GATE_TENANT_SLUG), None)
    if gate_tenant is None:
        raise G6Failure(
            f"tenant {GATE_TENANT_SLUG!r} does not exist: it owns the seeded connector "
            "conn-pg-lakehouse (0042_tenant_provisioning.sql) this gate runs"
        )
    if gate_tenant["id"] not in _me_tenant_ids():
        added = API.put(f"{API_URL}/api/identity/users/{me['id']}/tenants/{gate_tenant['id']}", timeout=10)
        if not added.ok:
            raise G6Failure(f"add the account to tenant {GATE_TENANT_SLUG!r} failed: {added.status_code} {added.text[:300]}")
        if gate_tenant["id"] not in _me_tenant_ids():
            raise G6Failure(f"the account was added to tenant {GATE_TENANT_SLUG!r} but /api/auth/me does not list it")
        print(f"[g6] added the account to tenant {GATE_TENANT_SLUG!r}")
    TENANT_ID = gate_tenant["id"]


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


def _produce_kafka_fixture(topic: str, *, run_id: str, interval_s: float = 1.5, max_s: float = 240.0) -> None:
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
    # Keep producing until THIS run has finished, not for a fixed count: in
    # CI the kafka run sat queued behind the other adapters' runs, first
    # polled after a fixed 45 s burst had ended, consumed nothing under
    # "latest", and so never created its Bronze table.
    query = "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename ... on Run { status } } }"
    deadline = time.time() + max_s
    i = 0
    try:
        while time.time() < deadline:
            row = {"id": i, "amount": 10.0 + i}
            producer.send(topic, json.dumps(row).encode("utf-8"))
            producer.flush()
            i += 1
            time.sleep(interval_s)
            resp = requests.post(DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10)
            resp.raise_for_status()
            status = ((resp.json().get("data") or {}).get("pipelineRunOrError") or {}).get("status")
            if status in ("SUCCESS", "FAILURE", "CANCELED"):
                break
    finally:
        producer.close()
    print(f"[g6] produced {i} kafka messages while run {run_id} was live")


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
            "environment": "production", "tenant": "g6", "tenantId": TENANT_ID, "residency": "", "capabilities": [],
        },
        timeout=10,
    )
    if not created.ok:
        raise G6Failure(f"create connector {name!r} failed: {created.status_code} {created.text}")
    return created.json()


def _post_ingest_run(connector_id: str) -> requests.Response:
    """POST /api/connectors/{id}/ingest/run with a one-shot retry that
    closes compose's dagster-code-location reload window. The reload
    can land AFTER step_wait_for_services() already observed a loaded
    location -- compose recreates the container when THIS runner
    starts, so the first launch POST races the recovered gRPC server
    by milliseconds and the API returns 422 `PipelineNotFoundError`
    (seen twice in CI: PR #57 and the R3 merge to main). On a 422
    whose body names `PipelineNotFoundError`, wait up to 120 s for
    `_ingest_job_is_loaded` and POST once more. Any other failure
    raises immediately. The retry's own failure raises with an
    explicit `post-reload retry` note so the gate's failure log names
    which side of the race it lost.
    """
    url = f"{API_URL}/api/connectors/{connector_id}/ingest/run"
    resp = API.post(url, timeout=10)
    if resp.ok:
        return resp
    if resp.status_code != 422 or "PipelineNotFoundError" not in resp.text:
        raise G6Failure(f"ingest/run for {connector_id!r} failed: {resp.status_code} {resp.text}")
    print(
        f"[g6] ingest/run for {connector_id!r} hit the dagster-code-location "
        "reload window; waiting for ingest_job to reload"
    )
    _wait_for(
        "Dagster code location (ingest_job loaded) post-reload",
        _ingest_job_is_loaded,
        120,
    )
    retry = API.post(url, timeout=10)
    if retry.ok:
        return retry
    raise G6Failure(
        f"ingest/run for {connector_id!r} failed (post-reload retry): "
        f"{retry.status_code} {retry.text}"
    )


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
    connector_ids: dict[str, str] | None = None,
) -> str | None:
    """POST /api/connectors (writing the derived `file:` credential's
    real value, if any, from `credential_values`), PUT .../ingest-spec,
    POST .../ingest/run. Returns the launched runId, or None for an
    honest supported:false response (CDC/Sheets)."""
    body = _create_connector(name=name, kind=kind, host=host, credential=credential)
    connector_id = body["id"]
    if connector_ids is not None:
        connector_ids[name] = connector_id
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

    run_resp = _post_ingest_run(connector_id)
    result = run_resp.json()
    if result.get("supported") is False:
        print(f"[g6] {name!r}: supported=false ({result.get('reason')})")
        return None
    run_id = result.get("runId")
    if not run_id:
        raise G6Failure(f"ingest/run for {name!r} returned no runId: {result}")
    print(f"[g6] launched run {run_id} for {name!r}")
    return run_id


def _check_object_storage_test_and_key_change(connector_id: str) -> None:
    """SRC-6 B3 (F1, F2): the object-storage connector made the way the
    wizard makes it (`host` is not the endpoint, the endpoint is in `dial`)
    is tested at its dial endpoint, and a wrong secret key is refused
    without being stored.

    Not covered here: `PUT .../credential` with the RIGHT pair answering
    `verified: true`. That request writes a `connector_managed_*` file, and
    in this gate `/run/secrets` is the read-only `g6_secrets` volume in
    `lakehouse-api` (ops/g6/docker-compose.g6.override.yml), so the API
    cannot write it (503), and a file it did write would not reach
    `dagster-code-location` either. Making that step honest needs a
    writable credential volume shared by the API and Dagster in this
    override; it is reported to the planner, not faked here (AGENTS.md
    principle 2). The 422 below never writes: the probe runs on an
    in-memory candidate before `replace_credentials`.
    """
    test_url = f"{API_URL}/api/connectors/{connector_id}/test"
    first = API.post(test_url, timeout=30)
    if not first.ok:
        raise G6Failure(f"POST .../test for g6-files failed: {first.status_code} {first.text}")
    first_body = first.json()
    if first_body.get("supported") is not True or first_body.get("ok") is not True:
        raise G6Failure(
            "expected the g6-files connection test to report supported=true, ok=true "
            f"(tested at the dial endpoint, SRC-6 F1), got {first_body}"
        )
    print(f"[g6] g6-files connection test: ok, {first_body.get('latencyMs')} ms")

    wrong = API.put(
        f"{API_URL}/api/connectors/{connector_id}/credential",
        json={
            "primary": {"kind": "access_key", "value": RUSTFS_ACCESS_KEY},
            "secondary": {"kind": "secret_key", "value": RUSTFS_SECRET_KEY + "-wrong"},
        },
        timeout=30,
    )
    if wrong.status_code != 422:
        raise G6Failure(
            "expected PUT .../credential with a wrong secret key to be refused with 422 "
            f"(SRC-6 F2), got {wrong.status_code} {wrong.text}"
        )
    print("[g6] g6-files wrong secret key: refused with 422")

    again = API.post(test_url, timeout=30)
    again_body = again.json() if again.ok else {}
    if again_body.get("supported") is not True or again_body.get("ok") is not True:
        raise G6Failure(
            "expected the g6-files connection test to still pass after the refused key change, "
            f"got {again.status_code} {again.text}"
        )
    print("[g6] g6-files connection test after the refused key change: still ok")


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
    pg_run = _post_ingest_run("conn-pg-lakehouse")
    run_ids["conn-pg-lakehouse"] = pg_run.json()["runId"]

    # files: the fixture ops/g6/seed_files_fixture.py wrote to
    # landing/orders.csv, registered against a NEW test connector -- never
    # conn-s3-warehouse, which rust/migrations/0033_connector_ingest_spec.sql
    # deliberately seeds with an empty source_objects.
    connector_ids: dict[str, str] = {}
    files_run_id = _register_and_run(
        connector_ids=connector_ids,
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
    # SRC-6 B3: after the ingest spec is saved (inside _register_and_run).
    # Read-only against the connector, so the run just launched is unaffected.
    _check_object_storage_test_and_key_change(connector_ids["g6-files"])

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
        _produce_kafka_fixture(kafka_topic, run_id=kafka_run_id)

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


def step_wait_for_run_failure(run_id: str) -> float:
    """Poll Dagster until the run is FAILURE and return its `endTime` (epoch
    seconds, the orchestrator's own clock on the same host as this runner).
    The mirror image of `step_wait_for_run_success`: here SUCCESS is the
    failure of the gate. `endTime` is the start of the interval SRC-7 is
    measured over; if Dagster has none, the time this gate first saw the
    FAILURE stands in for it and the printed line says so."""
    query = "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename ... on Run { status endTime } } }"
    seen: dict = {}

    def check() -> bool:
        resp = requests.post(DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10)
        resp.raise_for_status()
        run = resp.json().get("data", {}).get("pipelineRunOrError", {})
        status = run.get("status")
        print(f"[g6] run {run_id} status={status}")
        if status == "SUCCESS":
            raise G6Failure(f"run {run_id} SUCCEEDED but its credential was wrong: the run must fail")
        if status == "FAILURE":
            seen["end_time"] = run.get("endTime")
            seen["seen_at"] = time.time()
            return True
        return False

    # PR #101 CI: a wrong password is a refused credential, which
    # `ingest_factory.ingest_source_object` fails at once with
    # `allow_retries=False` (`_is_authentication_refusal`), so the run ends
    # in about 15 s (two steps, a few seconds each to start). Before that fix
    # `DEFAULT_RETRY_POLICY` retried it twice (30 s, then ~60 s, each with
    # jitter) and the run took 2m23s, which the old 150 s window missed by
    # one second. 90 s is generous for the fast path and shorter than the
    # retried run that was measured, so a return of the retries is meant to
    # fail here rather than pass by waiting them out (the jitter makes a
    # retried run's length vary, so this is a tripwire, not a proof).
    _wait_for(f"run {run_id} FAILURE", check, 90, interval_s=3.0)
    if seen["end_time"] is None:
        print("[g6] SRC-7 note: Dagster reported no endTime; measuring from when the gate first saw FAILURE")
        return seen["seen_at"]
    return float(seen["end_time"])


# SRC-7 spec: a failed load must be reflected "within 5 minutes" of the run
# ending (decision D1). The step fails if the connector's row is not there by
# then, and prints the measured seconds either way.
SRC7_WITHIN_SECONDS = 300


def step_failed_run_updates_connector_health() -> None:
    """SRC-7 tasks 4 and 10: a real failed run reaches the connector's row
    without anyone pressing Test.

    A SQL connector whose password is wrong (the `g6-mysql` fixture with a
    credential value that is not its password) is registered, given its ingest
    spec, and run. Once Dagster says the run FAILED, the orchestrator's
    `pipeline_run_failed_sensor` posts it to `POST /api/pipelines/events/
    run-failed` (needs `PIPELINE_RUN_TOKEN` on `lakehouse-api` and
    `dagster-code-location`, written to the g6 job's CI-only `.env`), and the
    API stamps the connector. This polls `GET /api/connectors/{id}` until it
    shows `failureStreak == 1`, `health == "degraded"` and a `lastRunFailureAt`,
    and prints how many seconds after the run ended that took (the planner
    copies the line into `docs/plans/SRC-7-RESULT.md`).

    NOT asserted here, on purpose: that the failure is listed by
    `GET /api/overview/alerts` and that a webhook arrives. That needs a
    `connector_failure` rule whose webhook target the gate can receive, and
    `ops/g6/rest_stub.py` answers `GET` only (`BaseHTTPRequestHandler` has no
    `do_POST`, so the webhook POST would get a 501); giving it a POST handler
    is a change outside this test file. The alert rule path is covered by the
    API's own route tests. This step proves the half the CI can prove today:
    the sensor -> API -> connector row path with a real Dagster run."""
    wrong_password = "g6-wrong-" + secrets.token_hex(4)
    body = _create_connector(
        name="g6-mysql-wrong-password", kind="MySQL", host="mysql-g6:3306",
        credential={"source": "file", "primary": "password"},
    )
    connector_id = body["id"]
    derived = (body.get("credential") or {}).get("primary")
    if not derived:
        raise G6Failure(f"create response for g6-mysql-wrong-password carried no derived credential name: {body}")
    _write_credential_file(derived, wrong_password)
    spec_resp = API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={
            "adapter": "sql", "ingestMode": "batch",
            "dial": {"driver": "mysql", "host": "mysql-g6", "port": 3306, "database": "g6_ingest", "user": "g6_reader"},
            "sourceObjects": [{"name": "orders", "target": "g6_mysql_wrong_password_orders"}],
        },
        timeout=10,
    )
    if not spec_resp.ok:
        raise G6Failure(f"set ingest-spec for g6-mysql-wrong-password failed: {spec_resp.status_code} {spec_resp.text}")

    run_id = _post_ingest_run(connector_id).json().get("runId")
    if not run_id:
        raise G6Failure("ingest/run for g6-mysql-wrong-password returned no runId")
    print(f"[g6] launched run {run_id} for 'g6-mysql-wrong-password' (wrong credential; it must fail)")
    ended_at = step_wait_for_run_failure(run_id)

    last: dict = {}
    deadline = ended_at + SRC7_WITHIN_SECONDS
    while time.time() < deadline:
        resp = API.get(f"{API_URL}/api/connectors/{connector_id}", timeout=10)
        if not resp.ok:
            raise G6Failure(f"GET /api/connectors/{connector_id} failed: {resp.status_code} {resp.text[:300]}")
        last = resp.json()
        if (
            last.get("failureStreak") == 1
            and last.get("health") == "degraded"
            and last.get("lastRunFailureAt") is not None
        ):
            seconds = time.time() - ended_at
            print(
                f"[g6] SRC-7 failed run {run_id} -> connector failureStreak=1 health=degraded "
                f"lastRunFailureAt={last['lastRunFailureAt']} {seconds:.1f}s after the run ended "
                "(polled every 3s)"
            )
            return
        time.sleep(3.0)
    raise G6Failure(
        f"SRC-7: {SRC7_WITHIN_SECONDS}s after run {run_id} ended, the connector shows "
        f"failureStreak={last.get('failureStreak')!r} health={last.get('health')!r} "
        f"lastRunFailureAt={last.get('lastRunFailureAt')!r}; expected 1, 'degraded' and a time. "
        "Is PIPELINE_RUN_TOKEN set on lakehouse-api and dagster-code-location?"
    )


# SRC-8: a table of its own and a connector of its own, so the matrix steps
# above never see a source whose shape changes under them.
SRC8_TABLE = "src8_drift"
SRC8_BRONZE = "g6_mysql_schema_drift"


def _src8_mysql(*statements: str) -> None:
    """Run DDL/DML on the MySQL fixture as root (the connector's own user,
    `g6_reader`, only reads)."""
    import pymysql
    conn = pymysql.connect(host="mysql-g6", user="root", password=MYSQL_ROOT_PASSWORD, database="g6_ingest")
    try:
        with conn.cursor() as cur:
            for statement in statements:
                cur.execute(statement)
        conn.commit()
    finally:
        conn.close()


def _bronze_columns(bronze_table: str) -> list[str]:
    """The column names ClickHouse sees on `bronze.<table>`, read the way
    `connector_catalog.register_loaded_table` reads them (`DESCRIBE TABLE`).
    `_bronze_row_count` runs first only to create the catalog database."""
    _bronze_row_count(bronze_table)
    text = ch_query(f"DESCRIBE TABLE {CATALOG_DB}.`bronze.{bronze_table}` FORMAT TabSeparated")
    return [line.split("\t", 1)[0] for line in text.splitlines() if line]


def _schema_changes(connector_id: str) -> dict:
    resp = API.get(f"{API_URL}/api/connectors/{connector_id}/schema-changes", timeout=10)
    if not resp.ok:
        raise G6Failure(f"GET schema-changes for {connector_id!r} failed: {resp.status_code} {resp.text[:300]}")
    return resp.json()


def _src8_run(connector_id: str, label: str) -> None:
    run_id = _post_ingest_run(connector_id).json().get("runId")
    if not run_id:
        raise G6Failure(f"SRC-8 {label}: ingest/run returned no runId")
    print(f"[g6] SRC-8 {label}: launched run {run_id}")
    step_wait_for_run_success(run_id)


def _src8_changes_of(listing: dict, bucket: str, kind: str, column: str) -> list[dict]:
    return [
        c for c in listing[bucket]
        if c["objectName"] == SRC8_TABLE and c["kind"] == kind and c["columnName"] == column
    ]


def step_source_schema_changes() -> None:
    """SRC-8 task 12: a MySQL table that changes shape between runs, on a
    connector of its own (policy: the default, apply non-breaking).

    1. `id, name, qty` is loaded; no change is listed (a first observation
       is the baseline).
    2. `ADD COLUMN note`: the run succeeds, Bronze gains `note`, and
       `column_added` is in `recent` (applied).
    3. `DROP COLUMN qty` (and a new source row): the run SUCCEEDS (a waiting
       table is not a failure, decision 7), the table is reported as
       waiting (a pending `column_removed` with `canApprove`), and its Bronze
       row count is the one from before; nothing was loaded.
    4. Approve, run: the new row arrives, `qty` is still a Bronze column,
       and `inactiveColumns` lists it.

    NOT asserted here: the "schema changed" alert (needs a webhook target
    this gate cannot receive, as for SRC-7); the other policies, a type
    change, a primary-key change (route and store tests cover them; the
    gate has one MySQL fixture and a type change that cannot be loaded
    needs a table re-added under a new target).

    Step 4 also reads the Bronze table's catalog detail the way the console's
    Schema tab does (`GET /api/catalog/{slug}`, slug = target with `_` as `-`)
    and asserts the mark of task 11: `qty` carries a non-null `inactiveSince`
    and no other column does. The gate's session is the bootstrap admin, which
    the catalog's tenant gate never refuses."""
    _src8_mysql(
        f"DROP TABLE IF EXISTS {SRC8_TABLE}",
        f"CREATE TABLE {SRC8_TABLE} (id INT PRIMARY KEY, name VARCHAR(50), qty INT)",
        f"INSERT INTO {SRC8_TABLE} (id, name, qty) VALUES (1, 'a', 10), (2, 'b', 20), (3, 'c', 30)",
    )
    body = _create_connector(
        name="g6-mysql-schema-drift", kind="MySQL", host="mysql-g6:3306",
        credential={"source": "file", "primary": "password"},
    )
    connector_id = body["id"]
    derived = (body.get("credential") or {}).get("primary")
    if not derived:
        raise G6Failure(f"create response for g6-mysql-schema-drift carried no derived credential name: {body}")
    _write_credential_file(derived, CONNECTOR_MYSQL_PASSWORD)
    # `replace`, so a loaded run leaves exactly the source's rows in Bronze and
    # "the count did not change" means "nothing was loaded".
    spec_resp = API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={
            "adapter": "sql", "ingestMode": "batch",
            "dial": {"driver": "mysql", "host": "mysql-g6", "port": 3306, "database": "g6_ingest", "user": "g6_reader"},
            "sourceObjects": [{"name": SRC8_TABLE, "target": SRC8_BRONZE, "loadMode": "replace"}],
        },
        timeout=10,
    )
    if not spec_resp.ok:
        raise G6Failure(f"set ingest-spec for g6-mysql-schema-drift failed: {spec_resp.status_code} {spec_resp.text}")

    # 1. baseline
    _src8_run(connector_id, "step 1 (baseline)")
    rows = _wait_for(f"bronze.{SRC8_BRONZE} rows visible", lambda: _bronze_row_count(SRC8_BRONZE) or None, 30, interval_s=3.0)
    listing = _schema_changes(connector_id)
    if rows != 3 or listing["pending"] or listing["recent"]:
        raise G6Failure(
            f"SRC-8 step 1: expected 3 rows and no change listed, got {rows} rows, "
            f"pending={listing['pending']!r} recent={listing['recent']!r}"
        )
    print(f"[g6] SRC-8 step 1: {rows} rows in bronze.{SRC8_BRONZE}, columns {_bronze_columns(SRC8_BRONZE)}, no change listed")

    # 2. a column is added
    _src8_mysql(
        f"ALTER TABLE {SRC8_TABLE} ADD COLUMN note VARCHAR(50)",
        f"INSERT INTO {SRC8_TABLE} (id, name, qty, note) VALUES (4, 'd', 40, 'added')",
    )
    _src8_run(connector_id, "step 2 (column added)")
    columns = _wait_for(
        f"bronze.{SRC8_BRONZE} has column note",
        lambda: (lambda cols: cols if "note" in cols else None)(_bronze_columns(SRC8_BRONZE)),
        45, interval_s=3.0,
    )
    listing = _schema_changes(connector_id)
    added = _src8_changes_of(listing, "recent", "column_added", "note")
    if not added or added[0]["status"] != "applied" or listing["pending"]:
        raise G6Failure(
            f"SRC-8 step 2: expected an applied column_added for note in recent and nothing pending, "
            f"got recent={listing['recent']!r} pending={listing['pending']!r}"
        )
    print(
        f"[g6] SRC-8 step 2: run succeeded, bronze columns {columns}, "
        f"recent has column_added note status={added[0]['status']}"
    )

    # 3. a column is dropped: the table waits, the run does not fail
    rows_before = _bronze_row_count(SRC8_BRONZE)
    _src8_mysql(
        f"ALTER TABLE {SRC8_TABLE} DROP COLUMN qty",
        f"INSERT INTO {SRC8_TABLE} (id, name, note) VALUES (5, 'e', 'after the drop')",
    )
    _src8_run(connector_id, "step 3 (column dropped)")  # raises if the run FAILED
    listing = _schema_changes(connector_id)
    removed = _src8_changes_of(listing, "pending", "column_removed", "qty")
    if not removed or removed[0]["canApprove"] is not True:
        raise G6Failure(
            f"SRC-8 step 3: expected a pending column_removed for qty with canApprove true, got pending={listing['pending']!r}"
        )
    waiting_rows = [
        r for r in API.get(
            f"{API_URL}/api/governance/ingest-runs?connectorId={connector_id}", timeout=10
        ).json()
        if r["object"] == SRC8_TABLE and r["status"] == "waiting"
    ]
    if not waiting_rows:
        raise G6Failure("SRC-8 step 3: the run succeeded but no ingest-runs row for the table says 'waiting'")
    rows_after = _bronze_row_count(SRC8_BRONZE)
    if rows_after != rows_before:
        raise G6Failure(
            f"SRC-8 step 3: the table was loaded while it waited: {rows_before} rows before, {rows_after} after"
        )
    print(
        f"[g6] SRC-8 step 3: run succeeded, pending column_removed qty canApprove={removed[0]['canApprove']}, "
        f"ingest-runs row status=waiting, bronze rows {rows_before} -> {rows_after} (not loaded)"
    )

    # 4. approve: it loads, and the dropped column is kept, marked inactive
    approved = API.post(
        f"{API_URL}/api/connectors/{connector_id}/schema-changes/approve", json={"object": SRC8_TABLE}, timeout=10
    )
    if not approved.ok:
        raise G6Failure(f"SRC-8 step 4: approve failed: {approved.status_code} {approved.text[:300]}")
    _src8_run(connector_id, "step 4 (approved)")
    rows_loaded = _wait_for(
        f"bronze.{SRC8_BRONZE} loads the row added after the drop",
        lambda: (lambda n: n if n == rows_before + 1 else None)(_bronze_row_count(SRC8_BRONZE)),
        45, interval_s=3.0,
    )
    columns = _bronze_columns(SRC8_BRONZE)
    listing = _schema_changes(connector_id)
    inactive = [i for i in listing["inactiveColumns"] if i["objectName"] == SRC8_TABLE and i["columnName"] == "qty"]
    if "qty" not in columns or not inactive or listing["pending"]:
        raise G6Failure(
            f"SRC-8 step 4: expected qty still in bronze columns {columns!r}, listed inactive, nothing pending; "
            f"got inactiveColumns={listing['inactiveColumns']!r} pending={listing['pending']!r}"
        )
    print(
        f"[g6] SRC-8 step 4: approved, run loaded ({rows_before} -> {rows_loaded} rows), "
        f"bronze columns {columns} (qty kept), inactiveColumns lists qty since {inactive[0]['inactiveSince']}"
    )

    # SRC-8 task 11: the Schema tab's mark, from the catalog detail. The
    # registry entry is written by the run itself (best effort,
    # `ingest_factory._register_in_catalog`), so poll for it.
    slug = SRC8_BRONZE.replace("_", "-")

    def _catalog_columns() -> list[dict] | None:
        resp = API.get(f"{API_URL}/api/catalog/{slug}", timeout=10)
        if not resp.ok:
            return None
        detail = resp.json()
        schema = detail.get("schema") if isinstance(detail, dict) else None
        return schema if schema and any(c.get("name") == "qty" for c in schema) else None

    try:
        schema = _wait_for(f"catalog detail of {slug} lists qty", _catalog_columns, 45, interval_s=3.0)
    except G6Failure as exc:
        raise G6Failure(
            f"SRC-8 step 4: the catalog detail of {slug!r} never listed qty ({exc}); is the table registered "
            "in the catalog (look for 'could not be registered in the catalog' in the code location's log)?"
        ) from exc
    # Judged on EVERY entry, not on a dict keyed by name: a column listed twice
    # (the registry is an unmerged ReplacingMergeTree, `PR #101 CI run 2`) must
    # not hide an unmarked copy behind a marked one, nor the reverse. The
    # failure text carries the raw list for the same reason.
    names = [c.get("name") for c in schema]
    qty_marks = [c.get("inactiveSince") for c in schema if c.get("name") == "qty"]
    others = [c for c in schema if c.get("name") != "qty" and c.get("inactiveSince")]
    if not qty_marks or not all(qty_marks) or others or len(set(names)) != len(names):
        raise G6Failure(
            "SRC-8 step 4: expected one entry per column, inactiveSince on qty and on no other column of the "
            f"catalog detail, got {[(c.get('name'), c.get('inactiveSince')) for c in schema]!r}"
        )
    print(f"[g6] SRC-8 step 4: catalog detail of {slug} marks qty inactiveSince {qty_marks[0]}, no other column")


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
    run_resp = _post_ingest_run(connector_id)
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
    run_resp = _post_ingest_run(connector_id)
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
    # Line-buffered, so the progress lines and the failure line below reach the
    # job log in order and a crash cannot take buffered lines with it: before
    # this, every "[g6]" line of a failed run carried the timestamp of the
    # process exit (`PR #101 CI run 2`), which also made the phase timings
    # unreadable.
    sys.stdout.reconfigure(line_buffering=True)
    sys.stderr.reconfigure(line_buffering=True)
    try:
        step_wait_for_services()
        step_login()
        step_ensure_tenant()
        step_ingest_matrix()
        step_failed_run_updates_connector_health()
        step_source_schema_changes()
        step_column_gate_rejects_an_unsupported_column()
        step_cdc_reports_unsupported_not_a_launch()
        step_cdc_mongo_debezium_properties()
        step_sheets_reports_unsupported()
    except G6Failure as exc:
        print(f"[g6] FAILED: {exc}", file=sys.stderr)
        return 1
    except Exception:  # noqa: BLE001 -- any other error is a failure of the gate and must be named as one
        import traceback
        print(f"[g6] FAILED: unexpected error, not a G6Failure:\n{traceback.format_exc()}", file=sys.stderr)
        return 1
    print("[g6] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
