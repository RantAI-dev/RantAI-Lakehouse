#!/usr/bin/env python3
"""G3a end-to-end acceptance test — `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md`
§3: dlt, running inside Dagster, reads a real Postgres table and writes it
to Bronze Iceberg through Lakekeeper at format-version 2; the result is
visible in the console catalog, the run is audited, and the dataset is
traceable through the console's lineage surface.

Run inside the compose network (the `g3a-test-runner` service in
`docker-compose.yml`), the same reason `g1_lakekeeper.rs`'s G1 test and
this repo's dlt pipeline (`dagster/dispar_orchestrate/dlt_pipeline.py`)
both do: Lakekeeper vends/advertises compose-internal hostnames
(`rustfs`, `lakekeeper`) that only resolve from inside this network.

Every HTTP call here is plain `urllib`/`http.client`-shaped (stdlib only,
plus `requests` which the base image already carries via dlt/dagster) —
this script intentionally does not depend on the Rust or Python
application code it is verifying, so it fails honestly if either is
actually broken rather than sharing a bug with it.
"""

from __future__ import annotations

import os
import sys
import time

import requests

CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
LAKEKEEPER_CATALOG_URI = os.environ.get(
    "CH_LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"
)
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
CH_RUSTFS_S3_ENDPOINT = os.environ.get("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000")
JOB_NAME = os.environ.get("BRONZE_JOB_NAME", "bronze_ingest_job")
BRONZE_TABLE_NAME = os.environ.get("BRONZE_TABLE_NAME", "g3a_orders")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")
# R1 (ADR 0011): ClickHouse's `catalog_credential` setting only accepts the
# Iceberg REST spec's OAuth2 `client_id:client_secret` form, never a raw
# static token — `CH_OAUTH_SERVER_URI` points it at `ops/oidc-mock`'s
# `/token` endpoint instead of Lakekeeper's own (unverified) one. Empty on
# a pre-R1 or authz-disabled stack, where no auth settings are added.
CH_OAUTH_CLIENT_ID = os.environ.get("CH_OAUTH_CLIENT_ID", "")
CH_OAUTH_SERVER_URI = os.environ.get("CH_OAUTH_SERVER_URI", "")


def ch_auth_settings() -> str:
    if not CH_OAUTH_CLIENT_ID:
        return ""
    return (
        f", catalog_credential = '{CH_OAUTH_CLIENT_ID}:unused', "
        f"oauth_server_uri = '{CH_OAUTH_SERVER_URI}'"
    )

# One session for every `lakehouse-api` call after login, so the session
# cookie `POST /api/auth/login` sets is carried on every subsequent
# request — every `/api/*` route this test exercises requires an
# authenticated session (`AuthenticatedPrincipal`).
API = requests.Session()


class G3aFailure(Exception):
    pass


def _wait_for(name: str, check, timeout_s: int, interval_s: float = 2.0) -> None:
    deadline = time.time() + timeout_s
    last_err: Exception | None = None
    while time.time() < deadline:
        try:
            if check():
                print(f"[g3a] ready: {name}")
                return
        except Exception as exc:  # noqa: BLE001 - report the real cause below
            last_err = exc
        time.sleep(interval_s)
    raise G3aFailure(f"timed out waiting for {name!r}: {last_err}")


def ch_query(sql: str) -> str:
    resp = requests.post(
        CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=30
    )
    if not resp.ok:
        raise G3aFailure(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


def step_wait_for_services() -> None:
    _wait_for("ClickHouse", lambda: ch_query("SELECT 1 FORMAT TabSeparated").strip() == "1", 60)
    _wait_for(
        "lakehouse-api",
        lambda: API.get(f"{API_URL}/health", timeout=5).status_code == 200,
        60,
    )
    _wait_for(
        "Dagster webserver",
        lambda: requests.get(
            DAGSTER_URL.replace("/graphql", "/server_info"), timeout=5
        ).status_code
        == 200,
        90,
    )


def step_login() -> None:
    """Every `/api/*` route this test calls requires an authenticated
    session (`AuthenticatedPrincipal`) — log in as the bootstrap admin
    (`AUTH_BOOTSTRAP_EMAIL`/`AUTH_BOOTSTRAP_PASSWORD`, seeded at
    `lakehouse-api` startup) and let `API` (a `requests.Session`) carry the
    `Set-Cookie` session token on every subsequent call.

    The bootstrap account is always created with `must_change_password =
    true` (`lakehouse_api::routes::auth`'s module doc), and every route
    other than `/api/auth/*` 403s until that rotation happens — so this
    also rotates the password (to itself; G3a doesn't need a different
    one) and logs in again, since `change-password` revokes every session
    it was called with, including the one that just authenticated.
    """
    login = API.post(
        f"{API_URL}/api/auth/login",
        json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
        timeout=10,
    )
    if not login.ok:
        raise G3aFailure(f"login failed: {login.status_code} {login.text}")

    if login.json().get("mustChangePassword"):
        rotated = API.post(
            f"{API_URL}/api/auth/change-password",
            json={"newPassword": AUTH_PASSWORD},
            timeout=10,
        )
        if not rotated.ok:
            raise G3aFailure(f"forced password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(
            f"{API_URL}/api/auth/login",
            json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
            timeout=10,
        )
        if not relogin.ok:
            raise G3aFailure(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")

    print("[g3a] logged in as bootstrap admin")


def step_job_is_listed_via_lakehouse_api() -> None:
    """Deliverable 1: the existing `lakehouse-dagster`-backed pipeline
    routes route to the real code location, not 503."""
    resp = API.get(f"{API_URL}/api/pipelines", timeout=10)
    if not resp.ok:
        raise G3aFailure(f"GET /api/pipelines failed: {resp.status_code} {resp.text}")
    body = resp.json()
    names = [p.get("id") or p.get("name") for p in body.get("pipelines", [])]
    if JOB_NAME not in names:
        raise G3aFailure(f"{JOB_NAME!r} not in GET /api/pipelines: {names}")
    print(f"[g3a] {JOB_NAME!r} is visible via GET /api/pipelines")


def step_trigger_run_via_lakehouse_api() -> str:
    resp = API.post(f"{API_URL}/api/pipelines/{JOB_NAME}/trigger", timeout=10)
    if not resp.ok:
        raise G3aFailure(f"POST trigger failed: {resp.status_code} {resp.text}")
    body = resp.json()
    run_id = body.get("id") or body.get("runId") or body.get("run_id")
    if not run_id:
        raise G3aFailure(f"trigger response had no runId: {body}")
    print(f"[g3a] launched run {run_id}")
    return run_id


def step_wait_for_run_success(run_id: str) -> None:
    query = (
        "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename "
        "... on Run { status } } }"
    )

    def check() -> bool:
        resp = requests.post(
            DAGSTER_URL,
            json={"query": query, "variables": {"rid": run_id}},
            timeout=10,
        )
        resp.raise_for_status()
        data = resp.json()
        run = data.get("data", {}).get("pipelineRunOrError", {})
        status = run.get("status")
        print(f"[g3a] run {run_id} status={status}")
        if status == "FAILURE":
            raise G3aFailure(f"run {run_id} FAILED")
        return status == "SUCCESS"

    _wait_for(f"run {run_id} SUCCESS", check, 180, interval_s=3.0)


def step_verify_rows_in_clickhouse() -> int:
    """The real proof the Bronze path works end to end: rows readable
    through ClickHouse's `DataLakeCatalog`, per the task brief."""
    ch_query(
        "CREATE DATABASE IF NOT EXISTS icecat_g3a "
        f"ENGINE = DataLakeCatalog('{LAKEKEEPER_CATALOG_URI}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{LAKEKEEPER_WAREHOUSE}', "
        f"storage_endpoint = '{CH_RUSTFS_S3_ENDPOINT}'{ch_auth_settings()} "
        "SETTINGS allow_database_iceberg = 1"
    )
    # Never a bare, unqualified row-count query here: ClickHouse 26.3
    # takes a metadata-only fast path against a merge-on-read Iceberg
    # table that overcounts on equality deletes (P5-RESULT.md; R11 in the
    # risk register, enforced by the ops/lint R11 guard script). This
    # table has no deletes today, but the WHERE-qualified form is the
    # only sanctioned shape for any Bronze Iceberg row count, regardless
    # of whether the bug is currently observable for this particular
    # table.
    count_text = ch_query(
        f"SELECT count() FROM icecat_g3a.`bronze.{BRONZE_TABLE_NAME}` "
        "WHERE 1 SETTINGS allow_database_iceberg = 1 FORMAT TabSeparated"
    )
    count = int(count_text.strip())
    if count <= 0:
        raise G3aFailure(f"expected rows in bronze.{BRONZE_TABLE_NAME}, got {count}")
    print(f"[g3a] ClickHouse DataLakeCatalog sees {count} rows in bronze.{BRONZE_TABLE_NAME}")
    return count


def step_verify_catalog_visibility() -> None:
    resp = API.get(f"{API_URL}/api/catalog", timeout=10)
    if not resp.ok:
        raise G3aFailure(f"GET /api/catalog failed: {resp.status_code} {resp.text}")
    body = resp.json()
    slug = BRONZE_TABLE_NAME.replace("_", "-")
    ids = [a.get("id") for a in body.get("assets", [])]
    if slug not in ids:
        raise G3aFailure(f"{slug!r} not visible in GET /api/catalog assets: {ids}")
    print(f"[g3a] {slug!r} is visible in GET /api/catalog")


def _mint_lakekeeper_read_token() -> str | None:
    """Mint a short-lived bearer token from `ops/oidc-mock`'s
    client-credentials endpoint (see that module's own doc: it exists so
    ClickHouse's `oauth_server_uri` has something to call, but any
    `TOKEN_ENDPOINT_PRINCIPAL` client_id works the same way) so this test
    can call Lakekeeper's REST catalog directly, the same identity
    ClickHouse's read path already authenticates as. Returns `None` on a
    pre-R1 or authz-disabled stack (`CH_OAUTH_CLIENT_ID` unset there too),
    where `step_verify_format_version_2` degrades to a skip rather than a
    failure."""
    if not CH_OAUTH_CLIENT_ID:
        return None
    resp = requests.post(
        CH_OAUTH_SERVER_URI, data={"client_id": CH_OAUTH_CLIENT_ID}, timeout=10
    )
    if not resp.ok:
        raise G3aFailure(f"minting a Lakekeeper read token failed: {resp.status_code} {resp.text}")
    token = resp.json().get("access_token")
    if not token:
        raise G3aFailure(f"token endpoint returned no access_token: {resp.json()}")
    return token


def step_verify_format_version_2() -> None:
    """PR #29 review: the body claimed format-version 2 was "confirmed"
    while `dlt_pipeline.py` never set it and nothing asserted it. This
    reads the table's OWN metadata straight from Lakekeeper's Iceberg REST
    catalog (`GET /v1/{prefix}/namespaces/bronze/tables/{table}`, the
    exact spec surface `iceberg-rust`'s `table.metadata().format_version()`
    reads from on the Rust side — see `lakehouse-iceberg/tests/
    g1_lakekeeper.rs`) — never through ClickHouse and never by trusting
    what dlt's own `load_info` reports back, since dlt reporting success
    says nothing about what property value actually landed in the
    catalog's metadata document."""
    token = _mint_lakekeeper_read_token()
    if not token:
        print(
            "[g3a] no CH_OAUTH_CLIENT_ID set (pre-R1/authz-disabled stack) — "
            "skipping the direct REST format-version check"
        )
        return
    headers = {"Authorization": f"Bearer {token}"}
    config_resp = requests.get(
        f"{LAKEKEEPER_CATALOG_URI}/v1/config",
        params={"warehouse": LAKEKEEPER_WAREHOUSE},
        headers=headers,
        timeout=10,
    )
    if not config_resp.ok:
        raise G3aFailure(f"catalog /v1/config failed: {config_resp.status_code} {config_resp.text}")
    config_body = config_resp.json()
    prefix = {**config_body.get("defaults", {}), **config_body.get("overrides", {})}.get("prefix")
    if not prefix:
        raise G3aFailure(f"catalog /v1/config returned no warehouse prefix: {config_body}")

    table_resp = requests.get(
        f"{LAKEKEEPER_CATALOG_URI}/v1/{prefix}/namespaces/bronze/tables/{BRONZE_TABLE_NAME}",
        headers=headers,
        timeout=10,
    )
    if not table_resp.ok:
        raise G3aFailure(
            f"catalog table-metadata GET failed for bronze.{BRONZE_TABLE_NAME}: "
            f"{table_resp.status_code} {table_resp.text}"
        )
    format_version = table_resp.json().get("metadata", {}).get("format-version")
    if format_version != 2:
        raise G3aFailure(
            f"expected format-version 2 from the catalog's own REST metadata "
            f"for bronze.{BRONZE_TABLE_NAME}, got {format_version!r}"
        )
    print(
        f"[g3a] catalog REST metadata confirms format-version={format_version} "
        f"for bronze.{BRONZE_TABLE_NAME}"
    )


def step_verify_run_audited() -> None:
    """The run is recorded via the governance/audit surface (Dagster run
    history) — a run-list check, distinct from lineage (see
    `step_verify_lineage_recorded` below). PR #29 review: this step used
    to be labeled "lineage recorded" while reading `/api/governance/audit`
    (a run list), never `/api/governance/lineage` — that claim is corrected
    here by naming this step for what it actually checks, and adding a
    real lineage check alongside it rather than in place of it, since this
    audit trail is still a real, useful assertion on its own ("do not
    invent a parallel mechanism" — this reuses the existing audit surface,
    exactly as before)."""
    resp = API.get(f"{API_URL}/api/governance/audit", timeout=10)
    if not resp.ok:
        raise G3aFailure(f"GET /api/governance/audit failed: {resp.status_code} {resp.text}")
    body = resp.json()
    resources = [a.get("resource") for a in body.get("audit", [])]
    if JOB_NAME not in resources:
        raise G3aFailure(f"no audit entry for job {JOB_NAME!r}: {resources}")
    print("[g3a] run recorded in GET /api/governance/audit")


def step_verify_lineage_recorded() -> None:
    """The REAL lineage surface (PR #29 review): `GET
    /api/governance/lineage?focus=<slug>` — `rust/crates/lakehouse-api/src/
    routes/governance.rs::lineage_body` builds a source -> Bronze -> Silver
    -> Gold graph from `lake.bronze_meta.dataset_catalog`, which
    `dagster/dispar_orchestrate/bronze_catalog.py::register_bronze_table`
    populates as part of this same G3a run (`assets.py`'s
    `bronze_ingest_job` calls it after the dlt load succeeds). An empty
    lineage graph for this slug means the dataset never actually became
    traceable, whatever the audit run-list above says."""
    slug = BRONZE_TABLE_NAME.replace("_", "-")
    resp = API.get(f"{API_URL}/api/governance/lineage", params={"focus": slug}, timeout=10)
    if not resp.ok:
        raise G3aFailure(f"GET /api/governance/lineage failed: {resp.status_code} {resp.text}")
    body = resp.json()
    nodes = body.get("nodes", [])
    if not nodes:
        raise G3aFailure(f"GET /api/governance/lineage?focus={slug} returned an empty graph: {body}")
    node_ids = [n.get("id") for n in nodes]
    bronze_node = f"bronze.{BRONZE_TABLE_NAME}"
    if bronze_node not in node_ids:
        raise G3aFailure(
            f"GET /api/governance/lineage?focus={slug} nodes {node_ids} do not "
            f"include the Bronze node {bronze_node!r}"
        )
    print(f"[g3a] GET /api/governance/lineage?focus={slug} shows nodes: {node_ids}")


def main() -> int:
    try:
        step_wait_for_services()
        step_login()
        step_job_is_listed_via_lakehouse_api()
        run_id = step_trigger_run_via_lakehouse_api()
        step_wait_for_run_success(run_id)
        step_verify_rows_in_clickhouse()
        step_verify_format_version_2()
        step_verify_catalog_visibility()
        step_verify_run_audited()
        step_verify_lineage_recorded()
    except G3aFailure as exc:
        print(f"[g3a] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g3a] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
