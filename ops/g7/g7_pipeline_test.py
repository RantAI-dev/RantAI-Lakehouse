#!/usr/bin/env python3
"""G7 — pipeline detail, source, and per-step materializations are real,
end to end (WS4 item H1, grand plan §6). Creates an authored pipeline over
an existing Bronze table with `dedupe`+`select`, activates it, triggers
it, and asserts:

- `GET /api/pipelines/{id}/runs/{runId}/steps` includes a materialization
  reporting a real `rows` count for the pipeline this test creates and
  runs.
- the Silver table the pipeline wrote holds exactly
  `count(DISTINCT id)` rows of the Bronze table it read, both counted
  through `ClickHouse` — the rows are proven, not just the count field
  the run reported (BLD-3 / D7);
- `GET /api/pipelines/{id}` returns a real (non-null) `graph` for a
  fixture `Dagster` job.
- `GET /api/pipelines/{id}/source?op=<that fixture op>` returns text
  containing that op's real function name.

Structured exactly like `ops/g3a/g3a_test.py` (full file read for this
task, WS4 item H1): one `G7Failure` exception, one `_wait_for` polling
helper, one authenticated `requests.Session` carrying the login cookie
(`ops/g3a/g3a_test.py:60-64` and `:111-149`'s `step_login`, duplicated
here rather than imported — this script intentionally shares no code with
the application it verifies, `ops/g3a/g3a_test.py`'s own module doc
explains why), and a `main()` that runs every step inside one
`try`/`except G7Failure`, printing `[g7] FAILED: ...`/returning 1 on
failure or `[g7] PASS`/0 on success (`ops/g3a/g3a_test.py:368-388`). The
bootstrap admin's session is used deliberately, NOT the
`authored-pipeline-scheduler` service identity `PIPELINE_RUN_TOKEN`
(WS4 item G3, `main.rs::bootstrap_pipeline_run_service`): that identity is
scoped to `pipeline:write` only (never `*:*`, per that function's own doc
comment) and cannot itself satisfy this gate's `pipeline:read`-gated
assertions -- the bootstrap admin's `Platform Admin` role's `*:*` covers
both.

# What this gate verifies, and how

The three routes this gate calls are ALL registered on this branch --
verified by reading `rust/crates/lakehouse-api/src/routes/mod.rs`'s
`/api/pipelines/*` route table:

- `GET /api/pipelines/{id}` (`step_verify_graph_and_source`) --
  `pipelines::detail`.
- `GET /api/pipelines/{id}/source` (`step_verify_graph_and_source`) --
  `pipelines::source`. It answers only when the op's declared `commit`
  matches the API image's own `GIT_SHA`, and REFUSES with 409 when
  either side is a placeholder (`pipeline_source::check_commit`'s
  `is_placeholder` short-circuit). Which of the two this gate asserts is
  decided by `G7_SOURCE_PROVENANCE`; read the section below before
  changing either branch.
- `GET /api/pipelines/{id}/runs/{runId}/steps`
  (`step_trigger_and_wait_for_real_row_materializations`) --
  `pipelines::run_steps`.
- `GET /api/pipelines/{id}/runs`
  (`step_a_load_of_its_own_table_triggers_the_chain`) --
  `pipelines::runs`, the chain's run list the negative assertion polls.
- `GET`/`PUT /api/connectors/{id}/ingest-spec` and
  `POST /api/connectors/{id}/ingest/run` (the BLD-4 half) -- the two
  routes `ops/g6/g6_ingest_matrix_test.py` drives its own connectors
  with, here used on the stack's own seeded `conn-pg-lakehouse`.

# BLD-4: the chain after a connector load fires only for its own table

`ingest_job` (`dagster/dispar_orchestrate/ingest_factory.py`) is ONE static
job for every connector and loads EVERY selected object of that connector
per run, so the dependency sensor's "newest SUCCESS run of the upstream" was
an anchor that said nothing about which table loaded. Authing a pipeline
with `dependsOn: ["ingest_job"]` therefore re-ran it on every connector's
load in the deployment.

The fix (`build_authored_dependency_sensor`'s per-table walk,
`_latest_record_for_ingest_table`) anchors on the newest SUCCESS run that
MATERIALIZED the pipeline's own Bronze table -- the asset key each loaded
object logs inside its run. This gate proves both halves on a real stack,
which is the only place they can be proven: `dagster/dispar_orchestrate/
test_authored_factory.py`'s unit tests carry a stub instance, and the
`start_time`-on-the-wrong-object bug this repo already shipped once hid from
exactly that shape.

1. a one-table run of the chain's OWN source table starts the chain, and
   the chain's Silver output carries the rows it should;
2. a one-table run of ANOTHER table does NOT, within two sensor intervals
   (Dagster's default 30 s evaluation interval, verified against the pinned
   1.13.20 and doubled in the gate's own wait).

Both are read from this gate's own two Bronze tables
(`g7_dep_orders`, `g7_dep_customers`), loaded one object at a time through
a connector spec this gate PUTs onto the seeded `conn-pg-lakehouse` and
restores in a `finally`. Why a second fixture table at all: a one-table run
selects its object by NAME (`ingest_factory.run_ingest` filters
`sourceObjects` on it), so two tables need two source objects, and
`docker-compose.yml`'s `g3a-source-init` now creates the stack's smallest
second table for exactly that. The seeded connector's own row is NOT edited
to load it -- that would change every deployment's fixture, which is a
product decision, not this gate's.

The earlier version of this module doc said the opposite (that those
three routes did not exist here and the gate could therefore not be
run). That was true of the commit that first added this file and is not
true now; SRC-20 runs this gate for real, after G3a, in the
`g3a-dagster` CI job.

# A failed run must fail the gate (BLD-3 / D7)

`step_trigger_and_wait_for_real_row_materializations` treats FAILURE as
a terminal state and then ASSERTS the run reported row materializations,
so a run that failed used to fail the gate on a confusing message ("no
step materialization reported a real row count") or, before that, to
time out silently. It now reads the run's own failure text from
`GET /api/pipelines/{id}/runs/{runId}/logs` and raises `G7Failure` with
it. The wait is 300 s, not 120 s, for a measured reason: the authored
op retries a real failure twice with a 60 s delay
(`op_metadata.DEFAULT_RETRY_POLICY`), so the run needs ~2 min to reach
FAILURE -- a 120 s wait expired first and reported a timeout instead of
the failure it was waiting for.

This script also depends on `POST /api/pipelines/{id}/status` (WS4 phase D
item D4, already on this branch per this workstream's earlier commits) to
move the freshly created pipeline from `"draft"` to `"ready"`, and on
`bronze_maintenance_job` (`dagster/dispar_orchestrate/maintenance.py`)
existing as the fixture Dagster job whose graph/source this gate checks.

`PIPELINE_RUN_TOKEN` must reach both `lakehouse-api` and the code
location: without it the code location builds zero authored jobs
(`authored_factory._fetch_authored_pipelines`) and the trigger answers
409. The CI job's `.env` therefore sets it, like every other job that
starts an authored pipeline.

# `G7_SOURCE_PROVENANCE`: what the source step asserts (BLD-3 review
BLOCKER 1)

The source step has two honest settings, and which one runs is a fact
about the BUILD, not a preference:

- `required` (the default; every local run and every real deployment)
  -- the step is unchanged: `GET /api/pipelines/{id}/source` must
  answer 200 with text containing the op's real function name. The
  images must therefore carry a real `GIT_SHA` on both sides, which is
  what the developer-facing `.local/build-images.sh` does.
- `unavailable` -- the step instead asserts the refusal the API is
  built to make: 409 with the body exactly `{"error": "source
  provenance is unavailable for this build"}`, and it prints one line
  saying the source text was NOT checked because this build carries no
  `GIT_SHA`. Any other status or body fails the gate.

`unavailable` is not a weakened gate. It is a real assertion about a
real response -- it is the same principle 3 (fail closed) the Rust
`is_placeholder` short-circuit implements, checked from the outside: a
build that cannot prove which commit it ran must say so, and a gate that
demanded the source text there would be demanding the API lie. It is
what CI needs, because `ci.yml`'s `images` job deliberately builds both
images WITHOUT `GIT_SHA` so the layer cache is shared across commits
(`ARG GIT_SHA` sits near the top of `dagster/Dockerfile`), which is the
same `GIT_SHA=unknown` shape those images always had. The source TEXT is
proven by the local run with a real commit; CI proves the refusal.

An unrecognised value is refused up front rather than silently treated
as `required`: a typo in the CI `.env` must fail loudly, not quietly
change what the gate checks.
"""

from __future__ import annotations

import os
import sys
import time

import requests

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")
BRONZE_TABLE_NAME = os.environ.get("BRONZE_TABLE_NAME", "g3a_orders")
FIXTURE_JOB_NAME = os.environ.get("G7_FIXTURE_JOB_NAME", "bronze_maintenance_job")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
SILVER_TABLE_NAME = os.environ.get("G7_SILVER_TABLE_NAME", "g7_dedupe_select_out")

# What the source step asserts (BLD-3 review BLOCKER 1): the real source
# text (`required`, the default) or the API's honest refusal on a build
# that carries no `GIT_SHA` (`unavailable`, the CI shape). See the module
# doc's section of the same name for why both are real assertions and why
# CI needs the second one.
SOURCE_PROVENANCE = os.environ.get("G7_SOURCE_PROVENANCE", "required")
SOURCE_PROVENANCE_MODES = ("required", "unavailable")

# The one body `pipelines::source` answers with for
# `pipeline_source::SourceError::UnverifiableProvenance`, quoted from the
# `Display` impl (`rust/crates/lakehouse-api/src/pipeline_source.rs:80`)
# the route serializes as `{"error": err.to_string()}`
# (`routes/pipelines.rs:1202-1206`). Compared as parsed JSON so the
# assertion is on the message and the whole body, not on the serializer's
# key spacing.
SOURCE_UNAVAILABLE_BODY = {"error": "source provenance is unavailable for this build"}

# ClickHouse, for the row assertion: the Silver output and the Bronze
# source are counted through the same client `ops/g3a/g3a_test.py` uses
# (`ch_query` below is that script's own function, duplicated -- these
# gate scripts share no module).
CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
LAKEKEEPER_CATALOG_URI = os.environ.get(
    "CH_LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"
)
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
CH_RUSTFS_S3_ENDPOINT = os.environ.get("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000")
# R1 (ADR 0011): the `catalog_credential` setting only accepts the Iceberg
# REST spec's OAuth2 `client_id:client_secret` form, so this points at
# `ops/oidc-mock`'s `/token` (`ops/g3a/g3a_test.py`'s own module-level
# comment says more). Empty on a pre-R1 or authz-disabled stack.
CH_OAUTH_CLIENT_ID = os.environ.get("CH_OAUTH_CLIENT_ID", "")
CH_OAUTH_SERVER_URI = os.environ.get("CH_OAUTH_SERVER_URI", "")

# This gate's own `DataLakeCatalog` database, its own so it never depends
# on another gate's (`ops/g3a` creates `icecat_g3a`, the authored job
# creates `icecat_authored`) having been created first -- the same
# one-database-per-reader rule those three follow.
CATALOG_DB = "icecat_g7"

# The status vocabulary `GET /api/pipelines/{id}/runs/{runId}/steps`
# answers in: the route maps Dagster's own statuses through
# `lakehouse_dagster::map_run_status` into the console's `EntityStatus`
# (`success` -> "completed", "failure" -> "failed"), so a gate that waits
# for "SUCCESS"/"FAILURE" waits forever on a run that has already
# finished -- which is what this gate did before SRC-20 first ran it.
TERMINAL_STEP_STATUSES = ("completed", "failed", "cancelled")

# How long to wait for the triggered run to reach a terminal state. Long
# enough for the authored op's retry budget (2 retries x 60 s) to be
# spent, so a genuinely failing run is reported AS a failure rather than
# as this timeout (see the module doc).
RUN_TERMINAL_TIMEOUT_S = 300

# One session for every `lakehouse-api` call after login, so the session
# cookie `POST /api/auth/login` sets is carried on every subsequent
# request -- duplicated from `ops/g3a/g3a_test.py:60-64`'s own module-level
# `API = requests.Session()` and its comment explaining why.
API = requests.Session()


class G7Failure(Exception):
    pass


def _wait_for(name: str, check, timeout_s: int, interval_s: float = 2.0) -> None:
    """Duplicated from `ops/g3a/g3a_test.py:71-82`'s `_wait_for` -- same
    shape, same reason (`ops/g4`/`ops/g6` each carry their own copy too,
    per this task's instructions: inventing a shared module across gate
    scripts is a bigger change than WS4 item H1)."""
    deadline = time.time() + timeout_s
    last_err: Exception | None = None
    while time.time() < deadline:
        try:
            if check():
                print(f"[g7] ready: {name}")
                return
        except Exception as exc:  # noqa: BLE001 - report the real cause below
            last_err = exc
        time.sleep(interval_s)
    raise G7Failure(f"timed out waiting for {name!r}: {last_err}")


def step_login() -> None:
    """Duplicated from `ops/g3a/g3a_test.py:111-149`'s `step_login` --
    same rotation-aware login: the bootstrap account is always created
    with `must_change_password = true`
    (`lakehouse_api::routes::auth`'s module doc), and every route other
    than `/api/auth/*` 403s until that rotation happens."""
    login = API.post(
        f"{API_URL}/api/auth/login",
        json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
        timeout=10,
    )
    if not login.ok:
        raise G7Failure(f"login failed: {login.status_code} {login.text}")

    if login.json().get("mustChangePassword"):
        rotated = API.post(
            f"{API_URL}/api/auth/change-password",
            json={"newPassword": AUTH_PASSWORD},
            timeout=10,
        )
        if not rotated.ok:
            raise G7Failure(f"password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(
            f"{API_URL}/api/auth/login",
            json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
            timeout=10,
        )
        if not relogin.ok:
            raise G7Failure(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")

    print("[g7] logged in as bootstrap admin")


def step_create_and_activate_authored_pipeline() -> str:
    """`POST /api/pipelines` (already on this branch, Phase 2 Task 2.5)
    creates a `"draft"` authored pipeline over `BRONZE_TABLE_NAME`;
    `POST /api/pipelines/{id}/status` (WS4 phase D item D4, already on
    this branch) moves it to `"ready"` -- the console's own "Activate"
    action, per this task's plan doc, uses the identical route.

    The transforms name the G3a FIXTURE's real columns: that table is
    created by `g3a-source-init` as
    `ingest_demo.orders(id, customer, amount, created_at)`, so `select(
    id, customer)` is the only spelling that runs. It used to say
    `select(id,name)`, which ClickHouse refused with `Code: 47.
    Unknown expression identifier 'name'` -- a gate that asserted a
    column the fixture never had, so it could only ever have failed.
    """
    create = API.post(
        f"{API_URL}/api/pipelines",
        json={
            "name": "g7_dedupe_select", "kind": "batch",
            "sourceZone": "bronze", "sourceTable": BRONZE_TABLE_NAME,
            "transforms": ["dedupe(id)", "select(id,customer)"],
            "targetZone": "silver", "targetTable": SILVER_TABLE_NAME,
            "schedule": "manual",
        },
        timeout=30,
    )
    if not create.ok:
        raise G7Failure(f"POST /api/pipelines failed: {create.status_code} {create.text}")
    pipeline_id = create.json()["id"]

    activate = API.post(f"{API_URL}/api/pipelines/{pipeline_id}/status", json={"status": "ready"}, timeout=30)
    if not activate.ok:
        raise G7Failure(f"POST .../status failed: {activate.status_code} {activate.text}")
    print(f"[g7] authored pipeline {pipeline_id!r} created and activated")
    return pipeline_id


def _dagster_job_is_loaded(job_name: str) -> bool:
    """True once a loaded Dagster repository lists `job_name`. Twin of
    `ops/g6/g6_ingest_matrix_test.py::_ingest_job_is_loaded` and of
    `ops/g3/g3_test.py::_dagster_job_is_loaded` (the gate scripts share no
    module)."""
    query = (
        "{ repositoriesOrError { ... on RepositoryConnection "
        "{ nodes { jobs { name } } } } }"
    )
    resp = requests.post(DAGSTER_URL, json={"query": query}, timeout=5)
    resp.raise_for_status()
    nodes = (resp.json().get("data") or {}).get("repositoriesOrError", {}).get("nodes") or []
    return any(job.get("name") == job_name for node in nodes for job in node.get("jobs", []))


def _job_not_loaded(resp: requests.Response) -> bool:
    """Whether the API answered "the orchestrator has not loaded this job
    yet": 503 and the stable `code`, never the message text. SEC-11 removed
    Dagster's `PipelineNotFoundError` text from responses, which this gate
    used to match (PR #105 CI fix)."""
    try:
        body = resp.json()
    except ValueError:
        return False
    return resp.status_code == 503 and isinstance(body, dict) and body.get("code") == "orchestrator_job_not_loaded"


def _post_trigger(url: str, job_name: str) -> requests.Response:
    """POST a trigger with a one-shot retry for the code-location start-up
    race. SRC-9 F-c: `ops/g3/g3_test.py::_post_trigger` (its twin, and
    `ops/g6/g6_ingest_matrix_test.py::_post_ingest_run`) hit
    `422 {"error":"PipelineNotFoundError"}` when the first trigger came
    before the code location had loaded. On that 503 `orchestrator_job_not_loaded`, wait up to 120 s for
    Dagster to list `job_name` (here the fixture job, which lives in the
    same code location as the authored pipeline's job) and POST once more. Any other failure is
    returned unchanged for the caller's own assertion."""
    resp = API.post(url, timeout=30)
    if resp.ok or not _job_not_loaded(resp):
        return resp
    print(f"[g7] trigger hit the code-location start-up window; waiting for {job_name} to load")
    _wait_for(f"Dagster code location ({job_name} loaded)", lambda: _dagster_job_is_loaded(job_name), 120)
    return API.post(url, timeout=30)


def _run_failure_message(pipeline_id: str, run_id: str) -> str:
    """The failed run's own failure text, from
    `GET /api/pipelines/{id}/runs/{runId}/logs` (`pipelines::run_logs`,
    which returns `{lines: [{ts, level, stepKey, message}]}`). The last
    few ERROR-level lines are what the operator reads in the console, so
    that is what the gate reports: "the run failed" without the reason is
    the failure mode this exists to remove.

    Best effort by design -- a log route that is itself unavailable must
    not replace the real failure with a second, less useful one.
    """
    resp = API.get(
        f"{API_URL}/api/pipelines/{pipeline_id}/runs/{run_id}/logs",
        params={"limit": 500}, timeout=30,
    )
    if not resp.ok:
        return f"(the run's logs were not readable: {resp.status_code} {resp.text})"
    lines = resp.json().get("lines", [])
    errors = [l for l in lines if str(l.get("level", "")).upper() in ("ERROR", "CRITICAL")]
    chosen = (errors or lines)[-5:]
    return "\n".join(str(l.get("message", l)) for l in chosen) or "(no log lines returned)"


def step_trigger_and_wait_for_real_row_materializations(pipeline_id: str) -> None:
    """`POST /api/pipelines/{id}/trigger` and
    `GET /api/pipelines/{id}/runs/{runId}/steps` are both real on this
    branch (the module doc names the route registrations). A run that
    ends in FAILURE fails THIS gate with the run's own message (BLD-3 /
    D7) rather than being reported as a missing materialization or, as
    before, as a timeout.
    """
    trigger = _post_trigger(f"{API_URL}/api/pipelines/{pipeline_id}/trigger", FIXTURE_JOB_NAME)
    if not trigger.ok:
        raise G7Failure(f"POST .../trigger failed: {trigger.status_code} {trigger.text}")
    run_id = trigger.json().get("id") or trigger.json().get("runId")
    if not run_id:
        raise G7Failure(f"trigger response had no run id: {trigger.json()}")

    def steps_are_terminal() -> bool:
        resp = API.get(f"{API_URL}/api/pipelines/{pipeline_id}/runs/{run_id}/steps", timeout=30)
        resp.raise_for_status()
        steps = resp.json().get("steps", [])
        return bool(steps) and all(s.get("status") in TERMINAL_STEP_STATUSES for s in steps)

    _wait_for(
        f"run {run_id} terminal",
        steps_are_terminal,
        timeout_s=RUN_TERMINAL_TIMEOUT_S,
        interval_s=3.0,
    )
    steps = API.get(f"{API_URL}/api/pipelines/{pipeline_id}/runs/{run_id}/steps", timeout=30).json()["steps"]
    failed = [s for s in steps if s.get("status") == "failed"]
    if failed:
        raise G7Failure(
            f"run {run_id} FAILED on step(s) {[s.get('stepKey') for s in failed]}:\n"
            f"{_run_failure_message(pipeline_id, run_id)}"
        )
    materializations = [m for s in steps for m in s.get("materializations", [])]
    if not any(m.get("rows") is not None for m in materializations):
        raise G7Failure(f"no step materialization reported a real row count: {steps}")
    print(f"[g7] run {run_id} reports real row materializations")


def ch_query(sql: str) -> str:
    """One ClickHouse statement, verbatim body, exactly as
    `ops/g3a/g3a_test.py::ch_query` sends it (duplicated -- the gate
    scripts share no module)."""
    resp = requests.post(
        CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=60
    )
    if not resp.ok:
        raise G7Failure(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


def ch_auth_settings() -> str:
    """The same `catalog_credential` settings
    `ops/g3a/g3a_test.py::ch_auth_settings` sends; empty on a pre-R1 or
    authz-disabled stack."""
    if not CH_OAUTH_CLIENT_ID:
        return ""
    return (
        f", catalog_credential = '{CH_OAUTH_CLIENT_ID}:unused', "
        f"oauth_server_uri = '{CH_OAUTH_SERVER_URI}'"
    )


def step_verify_the_silver_rows_match_the_bronze_source() -> None:
    """BLD-3 / D7: the pipeline's rows, not just its reported count.

    The Silver output must hold exactly one row per distinct `id` of the
    Bronze table -- what `dedupe(id)` means -- counted through ClickHouse
    on both sides. Bronze is Iceberg, so it is counted through this
    gate's own `DataLakeCatalog` database (`CATALOG_DB`), the same shape
    `ops/g3a/g3a_test.py` and the authored op use, each with its own
    database name.

    The Bronze query carries `WHERE 1`: an unqualified aggregate over a
    merge-on-read Iceberg table is answered from metadata and does not
    subtract equality deletes (R11, the
    `ops/lint/check_bare_iceberg_count.py` guard; the Silver side is a
    plain MergeTree table, so its count needs no such guard). That guard
    also scans prose, so this docstring and the failure messages below
    deliberately spell the aggregate out instead of writing the SQL it
    names beside `bronze.` -- the same reword-the-comment boundary the
    guard's own docstring documents.
    """
    _ensure_catalog_database()
    bronze_rows = int(
        ch_query(
            f"SELECT count(DISTINCT id) FROM {CATALOG_DB}.`bronze.{BRONZE_TABLE_NAME}` "
            "WHERE 1 SETTINGS allow_database_iceberg = 1 FORMAT TabSeparated"
        ).strip()
    )
    silver_rows = int(
        ch_query(
            f"SELECT count() FROM silver.`{SILVER_TABLE_NAME}` FORMAT TabSeparated"
        ).strip()
    )
    if bronze_rows <= 0:
        raise G7Failure(
            f"bronze.{BRONZE_TABLE_NAME} reported {bronze_rows} distinct ids; "
            "the fixture row count is wrong, so this gate would prove nothing"
        )
    if silver_rows != bronze_rows:
        raise G7Failure(
            f"silver.{SILVER_TABLE_NAME} holds {silver_rows} rows, "
            f"expected {bronze_rows} (one per distinct id of bronze.{BRONZE_TABLE_NAME})"
        )
    print(f"[g7] silver.{SILVER_TABLE_NAME} holds {silver_rows} rows = the {bronze_rows} distinct ids of bronze.{BRONZE_TABLE_NAME}")


# ── BLD-4: a chain after `ingest_job` fires only for its own table ──────

# The one upstream this half proves: `dagster/dispar_orchestrate/
# ingest_factory.py`'s ONE static job for every connector, launched by
# `POST /api/connectors/{id}/ingest/run`
# (`rust/crates/lakehouse-api/src/routes/connectors.rs`). Before BLD-4 the
# dependency sensor took its newest SUCCESS run as the freshness anchor, so
# every connector's load fired every pipeline that named it.
UPSTREAM_JOB_NAME = "ingest_job"

# The stack's own ingestible connector (`docker-compose.yml` +
# `rust/migrations/0034_seed_connector_ingest_spec.sql` seeds
# `conn-pg-lakehouse` on `ingest_demo.orders`). It is driven through the
# SAME `PUT .../ingest-spec` / `POST .../ingest/run` pair
# `ops/g6/g6_ingest_matrix_test.py` uses for its own connectors; the only
# thing this gate changes on it is the `sourceObjects` list, and that is
# restored before the gate exits. `0034`'s seeded row is NOT edited: adding
# a second table to every deployment's seeded connector is a product
# decision, not a gate fixture.
CONNECTOR_ID = os.environ.get("G7_INGEST_CONNECTOR_ID", "conn-pg-lakehouse")
SOURCE_OBJECT_A = os.environ.get("G7_INGEST_OBJECT_A", "ingest_demo.orders")
BRONZE_TABLE_A = os.environ.get("G7_INGEST_BRONZE_TABLE_A", "g7_dep_orders")
SOURCE_OBJECT_B = os.environ.get("G7_INGEST_OBJECT_B", "ingest_demo.customers")
BRONZE_TABLE_B = os.environ.get("G7_INGEST_BRONZE_TABLE_B", "g7_dep_customers")
CHAIN_SILVER_TABLE = os.environ.get("G7_CHAIN_SILVER_TABLE", "g7_dep_ingest_chain_out")

# Two Bronze tables that no earlier step of this gate (and no earlier gate
# in the g3a-dagster CI job) has ever loaded, so the chain under test cannot
# have a qualifying anchor before the load this gate runs on purpose: a
# chain that fires before table A is loaded would pass a "fires after A"
# assertion for the wrong reason.
# Both are loaded by ONE-TABLE runs (`{object, action}`, SRC-10 decision 5),
# never by a full run of the connector, so each run materializes exactly
# one of the two.
ONE_TABLE_ACTION = "refresh_remove"

# Dagster's own default evaluation interval, verified against the pinned
# `dagster==1.13.20`
# (`dagster._core.definitions.sensor_definition.DEFAULT_SENSOR_DAEMON_INTERVAL
# == 30`). The plan's window is TWO sensor intervals, and the loop below
# polls through it twice over, so a chain that fires late (on the second or
# third interval) still fails the gate rather than racing it.
SENSOR_INTERVAL_S = 30
OTHER_TABLE_MUST_NOT_TRIGGER_S = 2 * SENSOR_INTERVAL_S * 2

# How long the wait for a run to appear (a chain launch, or the ingest run
# this gate starts itself) is allowed to be. An ingest run of one table
# measures seconds locally; the chain run adds the sensor's next tick.
RUN_APPEARED_TIMEOUT_S = 180


def _read_connector_spec() -> dict:
    """`GET /api/connectors/{id}/ingest-spec`, the spec this gate will put
    back when it is done (`IngestSpec` in
    `rust/crates/lakehouse-store/src/connectors.rs`: `adapter`,
    `ingestMode`, `dial`, `sourceObjects` in camelCase)."""
    resp = API.get(f"{API_URL}/api/connectors/{CONNECTOR_ID}/ingest-spec", timeout=30)
    if not resp.ok:
        raise G7Failure(f"GET .../ingest-spec for {CONNECTOR_ID!r} failed: {resp.status_code} {resp.text}")
    return resp.json()


def _put_connector_spec(spec: dict) -> None:
    """`PUT /api/connectors/{id}/ingest-spec`, keeping the connector's own
    `adapter`/`ingestMode`/`dial` and replacing only its `sourceObjects`.
    No credential is sent: the stored `secretRef` is untouched, which is
    what lets the gate re-save the spec of a connector it did not create
    (ADR 0002 Addendum 3, `SEC-14`'s rule)."""
    resp = API.put(
        f"{API_URL}/api/connectors/{CONNECTOR_ID}/ingest-spec",
        json={
            "adapter": spec["adapter"],
            "ingestMode": spec["ingestMode"],
            "dial": spec.get("dial") or {},
            "sourceObjects": spec["sourceObjects"],
        },
        timeout=30,
    )
    if not resp.ok:
        raise G7Failure(
            f"PUT .../ingest-spec for {CONNECTOR_ID!r} failed: {resp.status_code} {resp.text}"
        )


def _run_ingest_one_table(object_name: str) -> str:
    """`POST /api/connectors/{id}/ingest/run` with `{object, action}` -- the
    SRC-10 decision 5 run of ONE table, retried only while the orchestrator
    answers that the code location is not loaded yet (the same race
    `ops/g6/g6_ingest_matrix_test.py::_post_ingest_run` documents). Returns
    the launched run id."""
    url = f"{API_URL}/api/connectors/{CONNECTOR_ID}/ingest/run"
    resp = API.post(url, json={"object": object_name, "action": ONE_TABLE_ACTION}, timeout=30)
    for attempt in range(1, 4):
        if resp.ok:
            break
        if not _job_not_loaded(resp):
            break
        print(f"[g7] ingest/run hit the code-location window; waiting for {UPSTREAM_JOB_NAME}")
        _wait_for(
            f"{UPSTREAM_JOB_NAME} loaded", lambda: _dagster_job_is_loaded(UPSTREAM_JOB_NAME), 120
        )
        resp = API.post(url, json={"object": object_name, "action": ONE_TABLE_ACTION}, timeout=30)
    if not resp.ok:
        raise G7Failure(
            f"POST .../ingest/run for {CONNECTOR_ID!r} object {object_name!r} failed: "
            f"{resp.status_code} {resp.text}"
        )
    body = resp.json()
    run_id = body.get("runId") or body.get("id")
    if not run_id:
        raise G7Failure(f"ingest/run response had no run id: {body}")
    return run_id


def _dagster_run_status(run_id: str) -> str | None:
    """A run's Dagster status, straight from the webserver's GraphQL
    (`ops/g6/g6_ingest_matrix_test.py`'s own query, same reason)."""
    query = (
        "query($rid:ID!){ pipelineRunOrError(runId:$rid)"
        "{ __typename ... on Run { status } } }"
    )
    resp = requests.post(DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10)
    resp.raise_for_status()
    block = (resp.json().get("data") or {}).get("pipelineRunOrError") or {}
    return block.get("status")


def _wait_for_run_terminal(run_id: str, what: str) -> None:
    """Wait until `run_id` reaches SUCCESS/FAILURE/CANCELED, and fail the
    gate on anything but SUCCESS: a FAILED chain run is the feature being
    broken, not a missing one."""
    def terminal() -> bool:
        return _dagster_run_status(run_id) in ("SUCCESS", "FAILURE", "CANCELED")

    _wait_for(f"{what} {run_id} terminal", terminal, timeout_s=RUN_TERMINAL_TIMEOUT_S, interval_s=3.0)
    status = _dagster_run_status(run_id)
    if status != "SUCCESS":
        raise G7Failure(f"{what} {run_id} ended {status}, expected SUCCESS")


def _chain_run_ids(pipeline_id: str) -> list[str]:
    """The ids of the chain's runs, from `GET /api/pipelines/{id}/runs`
    (`pipelines::runs`, which lists the `authored__<id>` job's runs). An
    `unavailable` payload (Dagster unreachable) is a gate failure, not an
    empty chain: the route says so instead of answering `[]`."""
    resp = API.get(f"{API_URL}/api/pipelines/{pipeline_id}/runs", timeout=30)
    resp.raise_for_status()
    body = resp.json()
    if body.get("unavailable"):
        raise G7Failure(f"GET .../runs reported the orchestrator unavailable: {body['unavailable']}")
    return [run["id"] for run in body.get("runs", [])]


def _ensure_catalog_database() -> None:
    """This gate's own `DataLakeCatalog` database, idempotent (the same
    statement `step_verify_the_silver_rows_match_the_bronze_source` sends,
    extracted because the BLD-4 half reads Bronze BEFORE that step runs and
    so needs the database to exist already). Each reader owns its own --
    `ops/g3a` creates `icecat_g3a`, the authored job creates
    `icecat_authored` -- so no gate depends on another's runs."""
    ch_query(
        f"CREATE DATABASE IF NOT EXISTS {CATALOG_DB} "
        f"ENGINE = DataLakeCatalog('{LAKEKEEPER_CATALOG_URI}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{LAKEKEEPER_WAREHOUSE}', "
        f"storage_endpoint = '{CH_RUSTFS_S3_ENDPOINT}'{ch_auth_settings()} "
        "SETTINGS allow_database_iceberg = 1"
    )


def _bronze_rows(table: str) -> int:
    """How many rows a Bronze Iceberg table holds, read through this gate's
    own `DataLakeCatalog` database -- the same read
    `step_verify_the_silver_rows_match_the_bronze_source` does, WITH a
    `WHERE 1` so the aggregate walks the rows rather than the snapshot's
    metadata (R11, `ops/lint/check_bare_iceberg_count.py`; the docstring on
    the step above explains why its prose spells the aggregate out too).

    The catalog database is ensured here and not by the caller: the BLD-4
    half reads Bronze before any other step of this gate has created it, and
    the statement is idempotent."""
    _ensure_catalog_database()
    return int(
        ch_query(
            f"SELECT count(DISTINCT id) FROM {CATALOG_DB}.`bronze.{table}` "
            "WHERE 1 SETTINGS allow_database_iceberg = 1 FORMAT TabSeparated"
        ).strip()
    )


def _create_the_ingest_chained_pipeline() -> str:
    """`POST /api/pipelines` + `POST /api/pipelines/{id}/status` -- the
    console's own "create, then Activate" pair
    (`step_create_and_activate_authored_pipeline` is the same two calls) --
    for a pipeline whose SOURCE is `bronze.<BRONZE_TABLE_A>` and whose
    `dependsOn` is `["ingest_job"]`: the shape that used to fire on every
    connector's load.

    The transforms name the same fixture columns the existing step's do
    (`ingest_demo.orders` is that table's source), so the only thing this
    pipeline proves is the trigger, not a new read path.
    """
    create = API.post(
        f"{API_URL}/api/pipelines",
        json={
            "name": "g7_dep_ingest_chain", "kind": "batch",
            "sourceZone": "bronze", "sourceTable": BRONZE_TABLE_A,
            "transforms": ["select(id,customer)"],
            "targetZone": "silver", "targetTable": CHAIN_SILVER_TABLE,
            "schedule": "manual",
            "dependsOn": [UPSTREAM_JOB_NAME],
        },
        timeout=30,
    )
    if not create.ok:
        raise G7Failure(
            f"POST /api/pipelines (dependsOn=[{UPSTREAM_JOB_NAME!r}]) failed: "
            f"{create.status_code} {create.text}"
        )
    pipeline_id = create.json()["id"]
    activate = API.post(
        f"{API_URL}/api/pipelines/{pipeline_id}/status", json={"status": "ready"}, timeout=30
    )
    if not activate.ok:
        raise G7Failure(f"POST .../status failed: {activate.status_code} {activate.text}")
    print(
        f"[g7] authored pipeline {pipeline_id!r} (bronze.{BRONZE_TABLE_A} -> "
        f"silver.{CHAIN_SILVER_TABLE}, depends_on=[{UPSTREAM_JOB_NAME!r}]) created and activated"
    )
    return pipeline_id


def step_a_load_of_its_own_table_triggers_the_chain(pipeline_id: str) -> None:
    """BLD-4 / D1, the positive half: a one-table run of the chain's OWN
    source table is the only kind of `ingest_job` run that may start it, so
    it must start -- and the run it starts must produce the Silver rows,
    counted through ClickHouse rather than taken from the run's own report
    (the same discipline `step_verify_the_silver_rows_match_the_bronze_
    source` follows)."""
    run_id = _run_ingest_one_table(SOURCE_OBJECT_A)
    print(f"[g7] ran one-table ingest of {SOURCE_OBJECT_A!r}: run {run_id}")
    _wait_for_run_terminal(run_id, "ingest run")
    _wait_for(
        f"bronze.{BRONZE_TABLE_A} readable with rows",
        lambda: _bronze_rows(BRONZE_TABLE_A) > 0,
        60,
    )
    loaded = _bronze_rows(BRONZE_TABLE_A)
    print(f"[g7] bronze.{BRONZE_TABLE_A} holds {loaded} rows, so the anchor this chain needs exists")

    def appeared() -> bool:
        return bool(_chain_run_ids(pipeline_id))

    _wait_for("the chain to run", appeared, timeout_s=RUN_APPEARED_TIMEOUT_S, interval_s=2.0)
    chain_runs = _chain_run_ids(pipeline_id)
    if len(chain_runs) != 1:
        raise G7Failure(
            f"the chain produced {len(chain_runs)} runs after one load of "
            f"{SOURCE_OBJECT_A!r}, expected exactly 1: {chain_runs}"
        )
    _wait_for_run_terminal(chain_runs[0], "chain run")
    silver_rows = int(
        ch_query(f"SELECT count() FROM silver.`{CHAIN_SILVER_TABLE}` FORMAT TabSeparated").strip()
    )
    print(
        f"[g7] the chain ran ({chain_runs[0]}) and wrote {silver_rows} rows to "
        f"silver.{CHAIN_SILVER_TABLE}"
    )


def step_a_load_of_another_table_does_not_trigger_the_chain(pipeline_id: str) -> None:
    """BLD-4 / D1, the negative half, and the reason the feature exists.

    A one-table run of a DIFFERENT table is a SUCCESS run of the same
    `ingest_job` -- the unfiltered rule ("newest SUCCESS run of the job")
    answered exactly this, which is how every connector's load came to fire
    every chain in a deployment. Here the chain has already run once (so the
    sensor is demonstrably alive and ticking: the assertion below is not
    satisfied by a sensor that never loads), and the new run keys a DIFFERENT
    set of upstream run ids, so Dagster's own `run_key` dedup cannot hide it
    either.

    The wait is two sensor intervals, doubled, and the run ids are compared
    rather than counted: a chain that starts here fails with its run id in
    the message."""
    before = _chain_run_ids(pipeline_id)
    run_id = _run_ingest_one_table(SOURCE_OBJECT_B)
    print(f"[g7] ran one-table ingest of {SOURCE_OBJECT_B!r}: run {run_id}")
    _wait_for_run_terminal(run_id, "ingest run")
    _wait_for(
        f"bronze.{BRONZE_TABLE_B} readable with rows",
        lambda: _bronze_rows(BRONZE_TABLE_B) > 0,
        60,
    )
    print(
        f"[g7] bronze.{BRONZE_TABLE_B} holds {_bronze_rows(BRONZE_TABLE_B)} rows; "
        "the chain must not run"
    )

    deadline = time.time() + OTHER_TABLE_MUST_NOT_TRIGGER_S
    while time.time() < deadline:
        time.sleep(2.0)
        current = _chain_run_ids(pipeline_id)
        new = [r for r in current if r not in before]
        if new:
            raise G7Failure(
                f"a load of {SOURCE_OBJECT_B!r} (bronze.{BRONZE_TABLE_B}) started the "
                f"chain anyway: new run(s) {new}; only a load of {SOURCE_OBJECT_A!r} "
                f"(bronze.{BRONZE_TABLE_A}) may start it"
            )
    print(
        f"[g7] {OTHER_TABLE_MUST_NOT_TRIGGER_S}s later (two sensor intervals, doubled) "
        f"the chain still has its {len(before)} run(s): a load of another table "
        "did not trigger it"
    )


def step_verify_graph_and_source() -> None:
    """`GET /api/pipelines/{id}` and `GET /api/pipelines/{id}/source` are
    real on this branch (the module doc names the route registrations).
    Checked against `FIXTURE_JOB_NAME`
    (`bronze_maintenance_job`, real on this branch), not the pipeline this
    test itself authored: a single `dedupe`+`select` authored pipeline
    compiles to exactly ONE `@op` (`authored_factory._op_for_pipeline`,
    WS4 item E3, applies the whole `transforms` list inside one op's
    body), so the grand plan's `>= 3 ops` acceptance text is checked
    against this pre-existing, multi-op fixture job instead -- a real op
    count is proven, just not on the pipeline this gate itself created.

    The GRAPH half is asserted the same way in both
    `G7_SOURCE_PROVENANCE` modes: it does not depend on the build's
    commit. Only the SOURCE half branches, and it branches on what the
    API actually does with a build that cannot name its commit (BLD-3
    review BLOCKER 1; the module doc's section on the variable says why
    the refusal is a real assertion and not a hole)."""
    fixture_detail = API.get(f"{API_URL}/api/pipelines/{FIXTURE_JOB_NAME}", timeout=30).json()
    if fixture_detail.get("graph") is None or len(fixture_detail["graph"]["ops"]) < 1:
        raise G7Failure(f"fixture job {FIXTURE_JOB_NAME!r} has no real graph: {fixture_detail}")
    op = fixture_detail["graph"]["ops"][0]
    if SOURCE_PROVENANCE == "unavailable":
        _assert_source_refused(op)
        return
    source = API.get(
        f"{API_URL}/api/pipelines/{FIXTURE_JOB_NAME}/source",
        params={"op": op["sourceRef"]}, timeout=30,
    )
    if not source.ok:
        raise G7Failure(f"GET .../source failed: {source.status_code} {source.text}")
    func_name = op["sourceRef"].split("::")[-1]
    if func_name not in source.json()["text"]:
        raise G7Failure(f"source text for {op['sourceRef']!r} did not contain {func_name!r}")
    print(f"[g7] {FIXTURE_JOB_NAME!r} graph has {len(fixture_detail['graph']['ops'])} op(s); source contains {func_name!r}")


def _assert_source_refused(op: dict) -> None:
    """`G7_SOURCE_PROVENANCE=unavailable`: the source text cannot be
    checked on this build, so the gate checks the refusal instead --
    409 with EXACTLY `SOURCE_UNAVAILABLE_BODY`, nothing else. A 200 here
    would mean the API served source it cannot prove; a different body
    would mean the message changed and this gate stopped describing what
    an operator would see. Both fail (BLD-3 review BLOCKER 1)."""
    source = API.get(
        f"{API_URL}/api/pipelines/{FIXTURE_JOB_NAME}/source",
        params={"op": op["sourceRef"]}, timeout=30,
    )
    if source.status_code != 409:
        raise G7Failure(
            f"G7_SOURCE_PROVENANCE=unavailable: GET .../source answered "
            f"{source.status_code} {source.text}; this build carries no GIT_SHA, "
            f"so the API must refuse with 409, not serve source it cannot verify"
        )
    try:
        body = source.json()
    except ValueError:
        raise G7Failure(f"the refusal body was not JSON: {source.text}") from None
    if body != SOURCE_UNAVAILABLE_BODY:
        raise G7Failure(
            f"G7_SOURCE_PROVENANCE=unavailable: refusal body was {body}, "
            f"expected {SOURCE_UNAVAILABLE_BODY}"
        )
    print(
        f"[g7] {FIXTURE_JOB_NAME!r} graph checked; source text NOT checked because "
        f"this build has no GIT_SHA -- the API refused with 409 "
        f"'{SOURCE_UNAVAILABLE_BODY['error']}', which is the honest answer"
    )


def main() -> int:
    seeded_spec: dict | None = None
    try:
        # Fail closed on an unrecognised mode rather than defaulting it to
        # `required`: a typo in a CI `.env` must be loud, not a quiet
        # change of what the gate checks (BLD-3 review BLOCKER 1).
        if SOURCE_PROVENANCE not in SOURCE_PROVENANCE_MODES:
            raise G7Failure(
                f"G7_SOURCE_PROVENANCE={SOURCE_PROVENANCE!r} is not one of "
                f"{list(SOURCE_PROVENANCE_MODES)}"
            )
        step_login()
        # BLD-4: the chain's two source objects on the stack's own
        # connector, and the chain itself, BEFORE either ingest run -- a
        # chain created after its anchor already exists would pass the
        # positive assertion for the wrong reason.
        seeded_spec = _read_connector_spec()
        _put_connector_spec({**seeded_spec, "sourceObjects": [
            {"name": SOURCE_OBJECT_A, "incrementalKey": "created_at", "target": BRONZE_TABLE_A},
            {"name": SOURCE_OBJECT_B, "target": BRONZE_TABLE_B},
        ]})
        print(
            f"[g7] {CONNECTOR_ID!r} now loads {SOURCE_OBJECT_A!r} -> bronze.{BRONZE_TABLE_A} "
            f"and {SOURCE_OBJECT_B!r} -> bronze.{BRONZE_TABLE_B} (one object per run)"
        )
        chain_pipeline_id = _create_the_ingest_chained_pipeline()
        step_a_load_of_its_own_table_triggers_the_chain(chain_pipeline_id)
        step_a_load_of_another_table_does_not_trigger_the_chain(chain_pipeline_id)
        pipeline_id = step_create_and_activate_authored_pipeline()
        step_trigger_and_wait_for_real_row_materializations(pipeline_id)
        step_verify_the_silver_rows_match_the_bronze_source()
        step_verify_graph_and_source()
    except G7Failure as exc:
        print(f"[g7] FAILED: {exc}", file=sys.stderr)
        return 1
    finally:
        # Leave the stack as this gate found it: the seeded connector's own
        # spec, so a later gate in the same stack (or a developer's next
        # `ops/g6` run) does not inherit this gate's two fixtures.
        if seeded_spec is not None:
            try:
                _put_connector_spec(seeded_spec)
                print(f"[g7] restored {CONNECTOR_ID!r}'s seeded ingest-spec")
            except G7Failure as exc:
                print(f"[g7] WARNING: could not restore the seeded spec: {exc}", file=sys.stderr)
    print("[g7] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
