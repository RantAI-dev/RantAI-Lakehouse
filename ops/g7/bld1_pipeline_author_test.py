#!/usr/bin/env python3
"""BLD-1 acceptance: a pipeline reads as its author, end to end.

Before this feature an authored pipeline's `SELECT` was built by the
orchestrator and run as the orchestrator's own ClickHouse user, so a masked
column reached a Silver table in clear (`GOV-4`). This gate proves the fix
where it matters -- in the rows a pipeline actually writes:

1. Seeds a ready policy that masks `customer` on the Bronze fixture for
   role `Analyst` (`POST /api/governance/policies`), the same shape
   `ops/g8/g8_governance_test.py` and `ops/g8/g8_time_travel_test.py`
   each seed for Query Studio.
2. Authors a pipeline over that Bronze table, assigns an ANALYST as its
   reader (`PUT /api/pipelines/{id}/run-as`), runs it, and asserts the
   Silver output holds the mask and never the clear value.
3. Runs a second pipeline with an IDENTICAL definition authored by an
   ADMIN, whom no policy governs, and asserts the same column arrives
   clear. Without this half the gate could be passing because every
   pipeline is masked, rather than because the author decides.
4. Asserts D2's refusal on a pipeline with NO author: its run fails with
   the API's fixed message and writes nothing.

Stands alone like every gate script here: it imports nothing from
`ops/g7/g7_pipeline_test.py` or `ops/g8/*`, so a bug shared with them
cannot hide in both. The helpers below (`ch_query`, `_wait_for`, the login
dance) are the same copies those scripts carry, for the reason
`ops/g3a/g3a_test.py`'s module doc gives.

# Step 4's fixture, and why it is passed in rather than made here

A pipeline with no author is what a SERVICE identity's create produces --
`pipeline_definition.run_as` is left NULL, because a service identity has
no `app_user` row to name. The gate's only credential is the bootstrap
admin's session, so it cannot create one that way, and this runner image
(`python:3.12-slim`) has no `psql` to write the column directly. So the
fixture is created by the caller and its id arrives in
`BLD1_NO_AUTHOR_PIPELINE_ID`: create the pipeline through the API as
usual, then

    docker compose -p <project> exec -T postgres \\
      psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -c \\
      "UPDATE pipeline_definition SET run_as = NULL WHERE id = '<id>'"

which is the same value the migration's backfill leaves on every
pre-existing pipeline, and the state an operator meets on an unassigned
pipeline. With the variable unset this gate prints an explicit SKIP line
saying so and continues; it does not silently pass a case it did not run.

# Which tenant the author is picked from (`SEC-50`)

The shared Bronze table this gate's pipelines read is gated as the tenant
that OWNS the shared catalog -- `CATALOG_TENANT_ID` -- because a table no
upload claimed belongs to the shared catalog, not to the tenant of the
pipeline that reads it. So the author has to be a member of that tenant
and the pipeline has to be scoped to it, or the run is refused with the
gate's own 404 ("table not available to your organisation").

The harness therefore reads the same setting the API reads (`the
CATALOG_TENANT_ID` environment variable, passed into this container by
`docker-compose.yml`'s `bld1-test-runner` service) and picks an author of
that tenant. When the setting is unset -- a deployment with a single
tenant, where the gate changes nothing -- the harness keeps the behaviour
it had: any holder of the role that belongs to some tenant.

# What this gate cannot prove

It cannot prove anything about a DAGSTER-NATIVE job: those have no
`run_as` and no enforce call. It cannot prove the rewrite for a policy
authored through the console UI, only that the engine applies what is
stored. And the `***` it looks for is the rewriter's own mask expression
as ClickHouse evaluates it; what it proves is that the CLEAR value never
reaches Silver, which is the property that matters.
"""

from __future__ import annotations

import json
import os
import sys
import time

import requests

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")
BRONZE_TABLE_NAME = os.environ.get("BRONZE_TABLE_NAME", "g3a_orders")
CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")

# `SEC-50`: the tenant that owns this deployment's shared catalog. The
# shared Bronze table is gated as that tenant, so the author picked below
# has to belong to it. Unset in a deployment that names no catalog tenant
# (a single-tenant deployment needs no owner): the gate then behaves as it
# did before the shared catalog had one.
CATALOG_TENANT_ID = os.environ.get("CATALOG_TENANT_ID", "").strip()

# The catalog tenant, resolved once per run (see `_catalog_tenant`).
_UNRESOLVED = object()
_CATALOG_TENANT = _UNRESOLVED

# The pipeline created with no author, for step 4. See the module doc.
NO_AUTHOR_PIPELINE_ID = os.environ.get("BLD1_NO_AUTHOR_PIPELINE_ID", "")
# `"create"` makes this run build that fixture and print its id, then stop:
# clearing `run_as` needs `psql`, which this image has not got, so the
# caller does that one statement and runs the gate again with the id.
CREATE_NO_AUTHOR_FIXTURE = os.environ.get("BLD1_NO_AUTHOR_FIXTURE", "") == "create"

# The mask target, the role the policy governs, and the role it does not.
MASKED_COLUMN = "customer"
MASKED_ROLE = "Analyst"
UNMASKED_ROLE = "Platform Admin"

# The exact string `routes::authored_pipelines::NO_AUTHOR_REASON` answers
# 409 with, which the orchestrator puts in the run's failure text. Quoted
# so a changed message -- a changed operator instruction -- fails loudly
# instead of being accepted as any 409.
NO_AUTHOR_REASON = (
    "this pipeline has no author to read as; an administrator must assign one"
)

# Every pipeline name and every Silver target carries this suffix. Two
# reasons, both learned by re-running this gate against a stack an earlier
# run had already touched: a pipeline NAME is unique per tenant (the second
# create answers 409), and -- worse, and silently -- a Silver table left by
# a previous run would be read as this run's output, which would let the
# gate "prove" masking on rows nothing wrote since. A gate that cannot be
# re-run against a used stack is a gate that gets run once.
RUN = str(int(time.time()))[-7:]

TERMINAL_STEP_STATUSES = ("completed", "failed", "cancelled")
RUN_TERMINAL_TIMEOUT_S = 300

API = requests.Session()


class Bld1Failure(Exception):
    pass


def _wait_for(name: str, check, timeout_s: int, interval_s: float = 2.0) -> None:
    deadline = time.time() + timeout_s
    last_err: Exception | None = None
    while time.time() < deadline:
        try:
            if check():
                print(f"[bld-1] ready: {name}")
                return
        except Exception as exc:  # noqa: BLE001 - the real cause is reported below
            last_err = exc
        time.sleep(interval_s)
    raise Bld1Failure(f"timed out waiting for {name!r}: {last_err}")


def step_login() -> None:
    login = API.post(
        f"{API_URL}/api/auth/login",
        json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
        timeout=10,
    )
    if not login.ok:
        raise Bld1Failure(f"login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = API.post(
            f"{API_URL}/api/auth/change-password",
            json={"newPassword": AUTH_PASSWORD},
            timeout=10,
        )
        if not rotated.ok:
            raise Bld1Failure(f"password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(
            f"{API_URL}/api/auth/login",
            json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
            timeout=10,
        )
        if not relogin.ok:
            raise Bld1Failure(f"re-login failed: {relogin.status_code} {relogin.text}")
    print("[bld-1] logged in as bootstrap admin")


def ch_query(sql: str) -> str:
    resp = requests.post(
        CH_URL,
        params={"user": CH_USER, "password": CH_PASSWORD},
        data=sql.encode("utf-8"),
        timeout=60,
    )
    if resp.status_code != 200:
        raise Bld1Failure(f"ClickHouse refused a statement: {resp.status_code} {resp.text}")
    return resp.text


def step_seed_the_masking_policy() -> None:
    """`conditions` is a STRING holding JSON, not a JSON object: the
    column stores authored conditions as text and the route deserializes
    it as `Option<String>`. Passing an object is a 400, which is what this
    did before the gate said so."""
    conditions = json.dumps(
        {
            "roles": [MASKED_ROLE],
            "table": f"bronze.{BRONZE_TABLE_NAME}",
            "mask": [MASKED_COLUMN],
        }
    )
    policy = API.post(
        f"{API_URL}/api/governance/policies",
        json={
            "name": "bld1_mask_customer_for_analyst",
            "kind": "Row filter",
            "subjects": MASKED_ROLE,
            "resources": f"bronze.{BRONZE_TABLE_NAME}",
            "effect": "Permit with obligation",
            "conditions": conditions,
            "activate": True,
        },
        timeout=30,
    )
    if not policy.ok:
        # A re-run on a stack this gate already seeded answers 409 for the
        # NAME, which is the policy this gate wanted -- so it is accepted
        # and said out loud. Any other 409 is a different collision and
        # still fails.
        if policy.status_code == 409 and "already exists" in policy.text:
            print("[bld-1] masking policy was already seeded by an earlier run")
        else:
            raise Bld1Failure(
                f"POST /api/governance/policies failed: {policy.status_code} {policy.text}"
            )
        return
    print(f"[bld-1] masking policy seeded for role {MASKED_ROLE} on bronze.{BRONZE_TABLE_NAME}")


def _seeded_tenants() -> list[dict]:
    """Every tenant of this deployment, as the API lists them.

    Through the API's own tenant list rather than a database read: the
    fixture should go through the surface a customer uses.
    """
    resp = API.get(f"{API_URL}/api/identity/tenants", timeout=30)
    if not resp.ok:
        raise Bld1Failure(f"GET /api/identity/tenants failed: {resp.status_code} {resp.text}")
    body = resp.json()
    rows = body if isinstance(body, list) else body.get("tenants", [])
    return [row for row in rows if isinstance(row, dict)]


def _catalog_tenant() -> dict | None:
    """The tenant that owns the shared catalog, as the deployment names it.

    `CATALOG_TENANT_ID` is a UUID and `User.tenants` carries tenant NAMES,
    so the id is resolved through the tenant list -- the same lookup the
    console's own tenant picker does. `None` when the deployment names no
    catalog tenant: nothing to prefer, nothing to fail on.

    The answer is resolved once per run: the tenant list does not change
    under the gate, and the gate asks for it on every author choice.
    """
    global _CATALOG_TENANT
    if _CATALOG_TENANT is not _UNRESOLVED:
        return _CATALOG_TENANT
    if not CATALOG_TENANT_ID:
        _CATALOG_TENANT = None
        return None
    for tenant in _seeded_tenants():
        if tenant.get("id") == CATALOG_TENANT_ID:
            _CATALOG_TENANT = tenant
            return tenant
    raise Bld1Failure(
        f"CATALOG_TENANT_ID={CATALOG_TENANT_ID} names no tenant of this deployment, so the "
        "gate cannot pick an author that may read the shared catalog"
    )


def _user_by_role(role_name: str) -> dict:
    """A user holding `role_name` that can actually be an author.

    Through the API's own user list rather than a database read: the
    fixture should go through the surface a customer uses, and the role
    NAME is the thing the policy is written against.

    Tenant membership is part of the choice, not a detail: `PUT .../run-as`
    requires the author to be a member of the pipeline's tenant, and the
    bootstrap admin (`ci@example.com`, which holds Platform Admin) is
    created by the API with no memberships at all -- so the FIRST user with
    the role is often one the assignment would refuse. Every holder is
    considered and the first one that could be assigned wins.

    `SEC-50`: the pipeline reads the SHARED Bronze table, which is gated
    as the tenant that owns the shared catalog. So the author has to
    belong to THAT tenant (and the pipeline is scoped to it, in
    `_tenant_shared_with`): a holder of the role outside that tenant
    cannot be an author here. The gate says so plainly instead of picking
    one and letting the API refuse the run.
    """
    resp = API.get(f"{API_URL}/api/identity/users", timeout=30)
    if not resp.ok:
        raise Bld1Failure(f"GET /api/identity/users failed: {resp.status_code} {resp.text}")
    body = resp.json()
    rows = body if isinstance(body, list) else body.get("users", [])
    holders = [user for user in rows if role_name in user.get("roles", [])]
    if not holders:
        raise Bld1Failure(f"no seeded user holds role {role_name!r}")
    catalog = _catalog_tenant()
    if catalog is not None:
        catalog_name = catalog.get("name")
        for user in holders:
            if catalog_name in (user.get("tenants") or []):
                return user
        raise Bld1Failure(
            f"no user holding {role_name!r} belongs to the catalog tenant {catalog_name!r} "
            f"({CATALOG_TENANT_ID}), so none of them may read the shared catalog"
        )
    for user in holders:
        if user.get("tenants"):
            return user
    raise Bld1Failure(
        f"every user holding {role_name!r} belongs to no tenant, so none can be an author"
    )


def _tenant_shared_with(user: dict) -> str:
    """The UUID of a tenant `user` belongs to.

    Not "the first seeded tenant": `PUT .../run-as` refuses an author who
    is not a member of the pipeline's OWN tenant (D2), so the pipeline has
    to be scoped to a tenant the chosen author is actually in. That
    refusal is correct -- it is the one the console hits too -- so the
    gate picks the tenant to fit the author rather than working around it.

    `User.tenants` carries tenant NAMES and `PUT .../tenant` takes a
    UUID, so the name is resolved through the tenant list -- the same
    lookup the console's own tenant picker does.

    `SEC-50`: when the deployment names a catalog tenant, that is the
    tenant the pipeline is scoped to -- the shared Bronze table it reads
    belongs to the owner of the shared catalog, and `_user_by_role` has
    already picked an author of that tenant.
    """
    names = user.get("tenants") or []
    if not names:
        raise Bld1Failure(f"user {user.get('email')} belongs to no tenant")
    catalog = _catalog_tenant()
    if catalog is not None:
        catalog_name = catalog.get("name")
        if catalog_name not in names:
            raise Bld1Failure(
                f"user {user.get('email')} does not belong to the catalog tenant "
                f"{catalog_name!r} ({CATALOG_TENANT_ID}), so its pipeline cannot read the "
                "shared catalog"
            )
        return catalog["id"]
    for tenant in _seeded_tenants():
        if tenant.get("name") in names:
            return tenant["id"]
    raise Bld1Failure(f"no seeded tenant matches any of {sorted(names)}")


def _create_pipeline(name: str, target_table: str, tenant_id: str) -> str:
    """One authored pipeline over the Bronze fixture, identical in every
    field except its name and target, so the two runs differ ONLY in who
    reads as. That is the whole comparison: same source, same transform,
    same masking policy, different author."""
    created = API.post(
        f"{API_URL}/api/pipelines",
        json={
            "name": name,
            "kind": "batch",
            "sourceZone": "bronze",
            "sourceTable": BRONZE_TABLE_NAME,
            "transforms": ["dedupe(id)", f"select(id,{MASKED_COLUMN})"],
            "targetZone": "silver",
            "targetTable": target_table,
            "schedule": "manual",
        },
        timeout=30,
    )
    if not created.ok:
        raise Bld1Failure(f"POST /api/pipelines failed: {created.status_code} {created.text}")
    pipeline_id = created.json()["id"]
    scoped = API.put(
        f"{API_URL}/api/pipelines/{pipeline_id}/tenant", json={"tenantId": tenant_id}, timeout=30
    )
    if scoped.status_code != 204:
        raise Bld1Failure(
            f"PUT .../tenant failed: {scoped.status_code} {scoped.text} -- without a tenant "
            "the author assignment below would be refused, correctly"
        )
    activated = API.post(
        f"{API_URL}/api/pipelines/{pipeline_id}/status", json={"status": "ready"}, timeout=30
    )
    if not activated.ok:
        raise Bld1Failure(f"POST .../status failed: {activated.status_code} {activated.text}")
    return pipeline_id


def _assign_author(pipeline_id: str, user_id: str) -> None:
    assigned = API.put(
        f"{API_URL}/api/pipelines/{pipeline_id}/run-as", json={"userId": user_id}, timeout=30
    )
    if assigned.status_code != 204:
        raise Bld1Failure(f"PUT .../run-as failed: {assigned.status_code} {assigned.text}")


def _trigger_and_wait(pipeline_id: str) -> tuple[str, list[dict]]:
    trigger = API.post(f"{API_URL}/api/pipelines/{pipeline_id}/trigger", json={}, timeout=60)
    if not trigger.ok:
        raise Bld1Failure(f"POST .../trigger failed: {trigger.status_code} {trigger.text}")
    body = trigger.json()
    run_id = body.get("id") or body.get("runId")
    if not run_id:
        raise Bld1Failure(f"trigger response had no run id: {body}")

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
    steps = API.get(
        f"{API_URL}/api/pipelines/{pipeline_id}/runs/{run_id}/steps", timeout=30
    ).json()["steps"]
    return run_id, steps


def _run_record(pipeline_id: str, run_id: str) -> dict:
    resp = API.get(f"{API_URL}/api/pipelines/{pipeline_id}/runs", timeout=30)
    if not resp.ok:
        return {}
    for run in resp.json().get("runs", []):
        if run.get("id") == run_id or run.get("runId") == run_id:
            return run
    return {}


def _silver_column_values(table: str) -> list[str]:
    resp = requests.post(
        CH_URL,
        params={"user": CH_USER, "password": CH_PASSWORD},
        data=f"SELECT DISTINCT {MASKED_COLUMN} FROM silver.`{table}` FORMAT TabSeparatedWithNames".encode("utf-8"),
        timeout=60,
    )
    if resp.status_code != 200:
        # A pipeline refused before staging wrote no Silver table at all --
        # that is 0 rows, which is what a refusal wants. Any other ClickHouse
        # error is still an error.
        if "UNKNOWN_TABLE" in resp.text or "doesn't exist" in resp.text:
            return []
        raise Bld1Failure(f"ClickHouse refused a statement: {resp.status_code} {resp.text}")
    out = resp.text.splitlines()
    return [line for line in out[1:] if line]


def step_masked_author_writes_the_masked_column() -> None:
    author = _user_by_role(MASKED_ROLE)
    pipeline_id = _create_pipeline(
        f"bld1_masked_{RUN}", f"bld1_masked_out_{RUN}", _tenant_shared_with(author)
    )
    _assign_author(pipeline_id, author["id"])
    run_id, steps = _trigger_and_wait(pipeline_id)
    failed = [s for s in steps if s.get("status") == "failed"]
    if failed:
        raise Bld1Failure(
            f"the masked-author run FAILED instead of writing masked rows: {json.dumps(steps)} "
            f"\nrun: {json.dumps(_run_record(pipeline_id, run_id))}"
        )
    values = _silver_column_values(f"bld1_masked_out_{RUN}")
    if not values:
        raise Bld1Failure("the masked pipeline wrote no rows, so nothing was proven")
    clear = [value for value in values if "***" not in value]
    if clear:
        raise Bld1Failure(
            f"silver masked output holds the UNMASKED {MASKED_COLUMN} values {clear} "
            "for an author the policy governs -- this is exactly GOV-4"
        )
    print(f"[bld-1] masked author wrote masked values only: {values}")


def step_unmasked_author_writes_the_column_clear() -> None:
    author = _user_by_role(UNMASKED_ROLE)
    pipeline_id = _create_pipeline(
        f"bld1_unmasked_{RUN}", f"bld1_unmasked_out_{RUN}", _tenant_shared_with(author)
    )
    _assign_author(pipeline_id, author["id"])
    run_id, steps = _trigger_and_wait(pipeline_id)
    failed = [s for s in steps if s.get("status") == "failed"]
    if failed:
        raise Bld1Failure(
            f"the unmasked-author run FAILED: {json.dumps(steps)} "
            f"\nrun: {json.dumps(_run_record(pipeline_id, run_id))}"
        )
    values = _silver_column_values(f"bld1_unmasked_out_{RUN}")
    if not values:
        raise Bld1Failure("the unmasked pipeline wrote no rows, so the comparison is empty")
    if any("***" in value for value in values):
        raise Bld1Failure(
            f"silver unmasked output holds masked values {values} for an author no policy "
            "governs -- the rewrite is being applied to everyone, not to the author"
        )
    print(f"[bld-1] unmasked author wrote clear values: {values}")


def step_a_pipeline_with_no_author_is_refused_with_d2s_message() -> None:
    if not NO_AUTHOR_PIPELINE_ID:
        print(
            "[bld-1] SKIPPED: the no-author case needs BLD1_NO_AUTHOR_PIPELINE_ID "
            "(see this file's module doc for the fixture). It is covered by "
            "crates/lakehouse-api/tests/pipeline_enforce.rs, which proves the same 409 "
            "for a NULL run_as and for a deactivated author."
        )
        return
    pipeline_id = NO_AUTHOR_PIPELINE_ID

    # 1. Direct enforcement check: the API refuses with D2's fixed 409 message
    enforce_resp = API.post(
        f"{API_URL}/api/pipelines/runnable/{pipeline_id}/enforce",
        json={"sql": f"SELECT {MASKED_COLUMN} FROM bronze.{BRONZE_TABLE_NAME}"},
        timeout=30,
    )
    if enforce_resp.status_code != 409:
        raise Bld1Failure(
            f"expected 409 from enforce for no-author pipeline, got {enforce_resp.status_code}: {enforce_resp.text}"
        )
    enforce_err = enforce_resp.json().get("error", "")
    if NO_AUTHOR_REASON not in enforce_err:
        raise Bld1Failure(
            f"enforce error missing D2 reason; expected {NO_AUTHOR_REASON!r}, got: {enforce_err!r}"
        )

    # 2. End-to-end orchestrator run: the run fails and writes nothing
    run_id, steps = _trigger_and_wait(pipeline_id)
    failed = [s for s in steps if s.get("status") == "failed"]
    if not failed:
        raise Bld1Failure(
            f"a pipeline with no author RAN to completion: {json.dumps(steps)}"
        )
    values = _silver_column_values("bld1_noauthor_out")
    if values:
        raise Bld1Failure(
            f"the refused pipeline still wrote rows: {values} -- a refusal must write nothing"
        )
    print(f"[bld-1] no author: API answered 409 {NO_AUTHOR_REASON!r}, run refused, and wrote nothing")


def step_create_the_no_author_fixture() -> None:
    """Builds step 4's pipeline and prints its id for the caller to clear.

    It is created WITH an author first -- through the same route an
    operator uses -- and the caller then runs the one statement that
    removes it. Clearing the column from here is not an option: the runner
    is `python:3.12-slim`, with no database driver and no `psql`, and
    inventing an API to unset a field that has no business being unset
    through the UI would be a worse change than a fixture step.
    """
    author = _user_by_role(MASKED_ROLE)
    pipeline_id = _create_pipeline(
        f"bld1_noauthor_{RUN}", "bld1_noauthor_out", _tenant_shared_with(author)
    )
    _assign_author(pipeline_id, author["id"])
    print(f"BLD1_FIXTURE_PIPELINE_ID={pipeline_id}")


def main() -> int:
    try:
        step_login()
        _wait_for(
            "lakehouse-api reachable",
            lambda: API.get(f"{API_URL}/api/auth/me", timeout=5).status_code in (200, 401),
            timeout_s=180,
        )
        if CREATE_NO_AUTHOR_FIXTURE:
            step_create_the_no_author_fixture()
            print("[bld-1] fixture created; clear its run_as, then re-run with the id")
            return 0
        step_seed_the_masking_policy()
        step_masked_author_writes_the_masked_column()
        step_unmasked_author_writes_the_column_clear()
        step_a_pipeline_with_no_author_is_refused_with_d2s_message()
    except Bld1Failure as exc:
        print(f"[bld-1] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[bld-1] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())