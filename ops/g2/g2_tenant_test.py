#!/usr/bin/env python3
"""G2 acceptance test — tenant provisioning and `X-Tenant` scoping
(`AGENTS.md`'s vocabulary; see `rust/crates/lakehouse-api/src/routes/
identity.rs` and `rust/crates/lakehouse-api/src/tenant_scope.rs`):

1. `POST /api/identity/tenants` (logged in as the bootstrap admin) drives a
   fresh tenant's checkpointed provisioning state machine
   (`identity::create_tenant`/`provision_tenant`) and the response reports
   the REAL terminal state that code reaches. Today that is
   `"grants_ready"`, never `"complete"`: `provision_tenant`'s own doc
   comment says why — the per-tenant Iceberg namespace call
   (`namespace_ready`/`complete`) has no caller yet, so claiming either
   status would be a fabricated completion in the database. This test
   asserts the actual value the route returns, not a status the code
   cannot produce — the alternative would be a "green" gate that stops
   detecting the day someone finishes namespace provisioning and forgets to
   update this file.
2. The warehouse that step actually created is visible through
   Lakekeeper's own admin surface (`GET management/v1/warehouse`,
   bearer-authenticated with the `admin` principal's pre-minted token) —
   proving `LakekeeperAdminClient::ensure_warehouse` really called
   Lakekeeper, not just that Postgres's `tenant` row was updated to claim
   it did.
3. `tenant_scope::resolve` fails closed the way its own module doc
   promises: an `X-Tenant` header naming a tenant the caller does not
   belong to is `404`, never `403` — a `403` would confirm to the caller
   that the tenant id exists at all, which `tenant_scope.rs`'s doc comment
   calls out by name as the leak this route must not have.
4. A SECOND tenant, provisioned into the SAME shared `TENANT_WAREHOUSE_S3_
   BUCKET` as the first (`routes::identity::provision_tenant`: `let prefix
   = format!("{}/{}/", config.lakehouse_warehouse_bucket, tenant.slug)`),
   also reaches `grants_ready` — even when its slug is the first tenant's
   slug PLUS a suffix (`<first>-eu`), a string-prefix relationship chosen
   deliberately: two Lakekeeper `key-prefix`es only avoid `Create
   WarehouseStorageProfileOverlap` if neither is an actual PATH prefix of
   the other, and `format!("{prefix}/")`'s trailing `/` is what turns
   "`g2-x`" (a string prefix of "`g2-x-eu`") into "`g2-x/`" (NOT a path
   prefix of "`g2-x-eu/`") — this step proves that trailing slash is
   doing its job, not merely that two unrelated slugs coexist. Both
   warehouses, AND the shared `default` warehouse (created at its storage
   profile's bucket root by `lakekeeper-warehouse-init`, in a DIFFERENT
   bucket from the tenant warehouses per `TENANT_WAREHOUSE_S3_BUCKET` —
   see `Config::tenant_warehouse_s3_bucket`'s doc comment), must all list
   from Lakekeeper's own `management/v1/warehouse` in the same call — this
   gate runs against the real `lakekeeper-warehouse-init`/`lakekeeper-
   authz-init` init chain, never a stack with `default` skipped.

Run inside the compose network (the `g2-test-runner` service in
`docker-compose.yml`, copying `g4-test-runner`'s shape) — Lakekeeper's
management API and the shared `lakehouse_oidc_tokens` volume both only
resolve/mount from inside this network.
"""

from __future__ import annotations

import os
import sys
import uuid

import requests

API_URL = os.environ.get("API_URL", "http://lakehouse-api:8080")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "admin@example.invalid")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "changeme")
# Same env var name `Config::lakekeeper_base_url` reads
# (`docker-compose.yml`'s `LAKEKEEPER_BASE_URI` — the P5-RESULT.md trap:
# Lakekeeper's own default self-reports `localhost`, which does not
# resolve from inside this container).
LAKEKEEPER_BASE_URI = os.environ.get("LAKEKEEPER_BASE_URI", "http://lakekeeper:8181")
# Same default path `Config::lakekeeper_admin_token_file` uses
# (`docker-compose.yml`'s `LAKEKEEPER_ADMIN_TOKEN_FILE`) — this runner
# mounts the identical volume subpath read-only, never prints its content.
ADMIN_TOKEN_FILE = os.environ.get("LAKEKEEPER_ADMIN_TOKEN_FILE", "/tokens/admin.jwt")

# The provisioning status `provision_tenant` (identity.rs) actually leaves
# a fresh tenant at today — see this file's module docstring, point 1.
# Asserted by name, not "not complete", so this gate breaks loudly (not
# silently) the day the code's real terminal state changes.
EXPECTED_TERMINAL_STATUS = "grants_ready"

API = requests.Session()


class G2Failure(Exception):
    """Raised for any gate-logic failure — never a bare `assert`, so a
    failure always carries a message naming what was expected and what was
    observed (AGENTS.md principle 2: gaps are written down where the
    reader looks)."""


def step_login() -> None:
    """`POST /api/identity/tenants` is `Policy::RequiresPermission
    ("identity:write")` (`policy.rs`) — log in as the bootstrap admin and
    let `API` carry the session cookie, the same pattern
    `ops/g4/g4_test.py::step_login` uses for the identical reason."""
    login = API.post(
        f"{API_URL}/api/auth/login",
        json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
        timeout=10,
    )
    if not login.ok:
        raise G2Failure(f"login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = API.post(
            f"{API_URL}/api/auth/change-password",
            json={"newPassword": AUTH_PASSWORD},
            timeout=10,
        )
        if not rotated.ok:
            raise G2Failure(f"forced password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(
            f"{API_URL}/api/auth/login",
            json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
            timeout=10,
        )
        if not relogin.ok:
            raise G2Failure(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")
    print("[g2] logged in as bootstrap admin")


def step_provision_tenant(slug: str, name: str) -> dict:
    """`POST /api/identity/tenants` for `slug` and assert the response
    reports the state `provision_tenant` actually reaches."""
    resp = API.post(
        f"{API_URL}/api/identity/tenants",
        json={"name": name, "slug": slug, "plan": "Enterprise", "residency": "g2-gate"},
        timeout=30,
    )
    if resp.status_code != 201:
        raise G2Failure(f"tenant create did not return 201: {resp.status_code} {resp.text}")
    tenant = resp.json()
    if tenant.get("provisioningStatus") != EXPECTED_TERMINAL_STATUS:
        raise G2Failure(
            "tenant provisioning did not reach the expected terminal state: "
            f"expected {EXPECTED_TERMINAL_STATUS!r}, got {tenant.get('provisioningStatus')!r} "
            f"(full body: {tenant})"
        )
    if not tenant.get("warehouseId"):
        raise G2Failure(f"tenant reached {EXPECTED_TERMINAL_STATUS!r} with no warehouseId: {tenant}")
    print(
        f"[g2] tenant {slug!r} provisioned to {EXPECTED_TERMINAL_STATUS!r}, "
        f"warehouseId={tenant['warehouseId']!r}"
    )
    return tenant


def _list_lakekeeper_warehouses() -> list[dict]:
    """`GET management/v1/warehouse`, bearer-authenticated with the
    `admin` principal's pre-minted token — shared by every step below that
    needs to see what Lakekeeper itself thinks exists, never printed."""
    try:
        with open(ADMIN_TOKEN_FILE, encoding="utf-8") as handle:
            admin_token = handle.read().strip()
    except OSError as exc:
        raise G2Failure(f"could not read the admin token file at {ADMIN_TOKEN_FILE}: {exc}") from exc
    if not admin_token:
        raise G2Failure(f"admin token file at {ADMIN_TOKEN_FILE} is empty")

    resp = requests.get(
        f"{LAKEKEEPER_BASE_URI}/management/v1/warehouse",
        headers={"Authorization": f"Bearer {admin_token}"},
        timeout=10,
    )
    if not resp.ok:
        raise G2Failure(f"Lakekeeper warehouse listing failed: {resp.status_code} {resp.text}")
    return resp.json().get("warehouses", [])


def step_verify_warehouse_listed(tenant: dict) -> None:
    """Lakekeeper's own `management/v1/warehouse` listing must show the
    warehouse `ensure_warehouse` (`lakehouse-auth::openfga`) actually
    created for this tenant — proving the provisioning call reached
    Lakekeeper for real, not only that Postgres's checkpoint was written."""
    warehouses = _list_lakekeeper_warehouses()
    warehouse_id = tenant["warehouseId"]
    match = next((w for w in warehouses if w.get("id") == warehouse_id), None)
    if match is None:
        listed_ids = [w.get("id") for w in warehouses]
        raise G2Failure(
            f"warehouse {warehouse_id!r} that provisioning reported is not in Lakekeeper's own "
            f"management/v1/warehouse listing: {listed_ids}"
        )
    print(f"[g2] Lakekeeper management API confirms warehouse {warehouse_id!r} exists")


def step_sibling_tenant_prefix_does_not_overlap(first_tenant: dict, first_slug: str) -> None:
    """Provision a SECOND tenant whose slug is `first_slug` plus a suffix
    (so `first_slug` is a literal string prefix of the second tenant's
    slug) and assert it also reaches `grants_ready` — proving
    `provision_tenant`'s per-tenant `key-prefix` (`{bucket}/{slug}/`, note
    the trailing slash) does not collide with a sibling tenant whose slug
    merely starts with the same characters. Then re-list Lakekeeper's
    warehouses once more and assert BOTH tenant warehouses AND the shared
    `default` warehouse (created by the real `lakekeeper-warehouse-init`/
    `lakekeeper-authz-init` chain this gate runs against) are all present
    — the `default` check is what proves this run did not silently skip
    that chain the way an earlier, reverted version of this gate did."""
    second_slug = f"{first_slug}-eu"
    second_tenant = step_provision_tenant(second_slug, "G2 gate tenant (sibling prefix)")

    warehouses = _list_lakekeeper_warehouses()
    listed_by_id = {w.get("id"): w for w in warehouses}
    listed_names = {w.get("name") for w in warehouses}

    for label, tenant in (("first", first_tenant), ("second", second_tenant)):
        warehouse_id = tenant["warehouseId"]
        if warehouse_id not in listed_by_id:
            raise G2Failure(
                f"{label} tenant's warehouse {warehouse_id!r} is missing from Lakekeeper's "
                f"management/v1/warehouse listing after provisioning the sibling: "
                f"{sorted(listed_by_id)}"
            )
    if "default" not in listed_names:
        raise G2Failure(
            "the shared 'default' warehouse is missing from Lakekeeper's management/v1/warehouse "
            f"listing — this gate must run against the real lakekeeper-warehouse-init/"
            f"lakekeeper-authz-init chain, not one that skips it: {sorted(listed_names)}"
        )
    print(
        f"[g2] sibling tenant {second_slug!r} (prefix of first slug {first_slug!r}) also reached "
        f"{EXPECTED_TERMINAL_STATUS!r}; Lakekeeper lists both tenant warehouses and 'default' "
        "with no overlap"
    )


def step_x_tenant_for_a_foreign_tenant_is_404_not_403() -> None:
    """`GET /api/connectors` (`Policy::RequiresPermission("connector:
    manage")`, which the bootstrap admin has) runs `tenant_scope::resolve`
    first — send an `X-Tenant` naming a tenant this principal is not a
    member of (a random, never-provisioned UUID) and assert `404`, per
    `tenant_scope.rs`'s own contract: a `403` here would leak that the
    named tenant id exists."""
    foreign_tenant_id = str(uuid.uuid4())
    resp = API.get(
        f"{API_URL}/api/connectors",
        headers={"X-Tenant": foreign_tenant_id},
        timeout=10,
    )
    if resp.status_code != 404:
        raise G2Failure(
            f"X-Tenant naming a foreign tenant returned {resp.status_code}, expected 404 "
            f"(never 403 — see tenant_scope.rs's module doc): body={resp.text}"
        )
    print(f"[g2] X-Tenant={foreign_tenant_id!r} (not a member) correctly 404s, never 403")


def main() -> int:
    try:
        step_login()
        first_slug = f"g2-{uuid.uuid4().hex[:12]}"
        tenant = step_provision_tenant(first_slug, "G2 gate tenant")
        step_verify_warehouse_listed(tenant)
        step_x_tenant_for_a_foreign_tenant_is_404_not_403()
        step_sibling_tenant_prefix_does_not_overlap(tenant, first_slug)
    except G2Failure as exc:
        print(f"[g2] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g2] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
