#!/usr/bin/env python3
"""G8 acceptance test (WS7 plan, Hard Requirement 10; item H1): proves that
an authored masking policy actually does something now that it is wired
into the read path.

Until WS7 Phase C/D, `sql_rewrite`/`policy_engine` existed but were never
called from `routes::query::run` (or the dashboard/embed/Gold-export read
sites), so a masking gate would have exercised only dead code — the
assertions below would have passed against a build with the entire
enforcement module deleted. Phase C wired `routes::query::run` and Phase D
routed dashboards/embeds/Gold export through the same
`policy_engine::rewrite_sql_for_roles` (`rust/crates/lakehouse-api/src/
routes/query.rs::rewrite_sql_for_principal_inner`), so this gate now proves
something real:

1. A `status = 'ready'` policy naming role `Analyst` and masking a column
   makes `POST /api/query/run` return `"***"` for that column when an
   Analyst session runs it, and the real value for a session with no
   matching policy (the bootstrap admin, role `Platform Admin`, `*:*`).
2. Three syntactic bypass attempts against the SAME governed table (a bare
   `SELECT *`, a `SELECT *` wrapped in a CTE) either come back masked or
   are refused outright (`sql_rewrite`'s documented "never a silent
   pass-through" rule) — never leak the raw value.

# Why this gate owns its own governed table, not `serving.mart_gci_readiness`

WS6/WS7 planning docs use `serving.mart_gci_readiness` as their worked
example, and it is a real name — `rust/crates/lakehouse-bi/specs/
builtin-default.json` ships a built-in dashboard tile against it. But
nothing in this repository's own provisioning (`dagster/dispar_orchestrate/
*.py`, `rust/migrations/*.sql`) creates or populates that table: it exists
today only inside a specific customer demo deployment's ClickHouse
(`deploy/187/`, `demo/`), not in a fresh `docker compose up`. A gate that
queried it would pass locally against that demo stack and fail — for a
reason having nothing to do with governance — against the CI job's clean
Postgres/ClickHouse volumes this file's own `g8-test-runner` service (WS7
item H2) brings up. So this gate creates and owns a small ClickHouse table
under `serving` (`g8_gate_subjects`) with a fixed, synthetic row: an
`email` column to mask and an unmasked `region` column alongside it, so the
masked-Analyst assertion also proves the OTHER column in the same row
comes back in clear text — a bug that masked the whole row (not just the
named column) would still fail this gate.

# What this gate cannot prove

This script only ever observes `lakehouse-api`'s HTTP responses — it
cannot inspect the SQL ClickHouse actually executed, or `policy_engine`'s
internal decision. A masking implementation that happened to hardcode the
literal string `"***"` for any column named `email` (rather than genuinely
consulting the authored policy) would also pass every assertion below;
this gate substantially narrows that risk by also asserting the SAME
column comes back in clear text for a session with NO obligation (the
admin), and by asserting the untouched sibling column is never masked, but
it is not a substitute for reading `policy_engine.rs`/`sql_rewrite.rs`
directly. That reading was done as part of authoring this file (WS7 item
H1) — see the module doc comments this file cites above — not re-verified
by anything this script runs.

This gate also has NEVER BEEN EXECUTED as of this commit: it needs a live
compose stack (`ops/g8`'s implementer was explicitly instructed not to run
`docker compose up` or start any container while writing it). Only
`python3 -m py_compile` and the repo's import-boundary lint have been run
against it — see the commit body for the exact commands.
"""

from __future__ import annotations

import os
import subprocess
import sys

import requests
from argon2 import PasswordHasher  # argon2-cffi — gate-only dependency, installed by g8-test-runner's command (WS7 item H2)

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")

AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")

# Gate-owned fixture identity — never a real person, `.invalid` is the RFC
# 2606 reserved TLD, matching this repo's existing convention for
# synthetic gate accounts.
ANALYST_EMAIL = "g8-analyst@lakehouse.invalid"
ANALYST_PASSWORD = "g8-analyst-password-not-real-456"  # noqa: S105 -- gate fixture, not a real secret

# The table this gate seeds and governs, and the fixed row it authors a
# masking policy against. `region` is never named by the policy's `mask`
# list below, so it doubles as the "only the named column is masked, not
# the whole row" check.
GATE_TABLE = "serving.g8_gate_subjects"
GATE_EMAIL_VALUE = "subject@g8.invalid"
GATE_REGION_VALUE = "g8-fixture-region"

POLICY_NAME = "g8-mask-email"


class G8Failure(Exception):
    """Raised for any acceptance-criterion violation or infrastructure
    failure this gate cannot proceed past."""


def psql(sql: str) -> str:
    """Runs `sql` against the CONSOLE's own Postgres (`app_user`/
    `auth_identity`/`role` — identity tables, not lake data) via the
    `psql` client, reading connection parameters from the standard
    `PGHOST`/`PGPORT`/`PGUSER`/`PGPASSWORD`/`PGDATABASE` environment
    variables `g8-test-runner`'s compose service sets — the same pattern
    `ops/g4/g4_test.py::pg_exec` already establishes for its own,
    unrelated CDC-source psql access."""
    result = subprocess.run(
        ["psql", "-v", "ON_ERROR_STOP=1", "-tqA", "-c", sql],
        capture_output=True,
        text=True,
        check=False,
        timeout=30,
    )
    if result.returncode != 0:
        raise G8Failure(f"psql failed: {result.stderr}\nSQL: {sql}")
    return result.stdout.strip()


def ch_query(sql: str) -> str:
    resp = requests.post(CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=30)
    if not resp.ok:
        raise G8Failure(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


def step_seed_gate_table() -> None:
    """Creates and populates this gate's OWN governed table, rather than
    depending on a customer demo dataset — see the module docstring's
    "why this gate owns its own governed table" section. `CREATE ...  IF
    NOT EXISTS` plus an unconditional `INSERT` is idempotent enough for
    this gate's purpose: a repeat local run against a persisted volume
    accumulates extra identical rows, which is harmless because every
    assertion below reads `LIMIT 1` and checks only the single column
    values it cares about, never a row count. A real CI run always starts
    from a fresh, just-created ClickHouse volume (`docker compose down
    -v` at the end of every prior run — WS7 item H2's CI job step), so
    this case never actually arises there.
    """
    ch_query("CREATE DATABASE IF NOT EXISTS serving")
    ch_query(
        f"CREATE TABLE IF NOT EXISTS {GATE_TABLE} "
        "(email String, region String) ENGINE = MergeTree ORDER BY email"
    )
    ch_query(
        f"INSERT INTO {GATE_TABLE} (email, region) VALUES "
        f"('{GATE_EMAIL_VALUE}', '{GATE_REGION_VALUE}')"
    )
    print(f"[g8] seeded {GATE_TABLE}")


def step_seed_analyst_login() -> None:
    """Inserts one `app_user` + `auth_identity` + `app_user_role` row
    directly, bypassing `POST /api/identity/users` (which creates no
    password credential today), the same "gate reaches Postgres directly"
    pattern `ops/g4/g4_test.py::pg_exec` already establishes for its own
    CDC source rows. Idempotent: `ON CONFLICT` on every insert, so a
    repeat run against a persisted volume upserts the same row rather
    than failing on a unique-constraint violation.

    Schema verified against `rust/migrations/0001_*.sql` (`app_user`,
    `app_user_role`) and `rust/migrations/0019_auth.sql`
    (`auth_identity`): `external_subject` for a `local` identity is the
    owning user's own id as text (`lakehouse_auth::password::
    create_local_identity`, `rust/crates/lakehouse-auth/src/
    password.rs`, binds `app_user_id.to_string()` as `external_subject`);
    the natural key is `(provider, external_subject)`, not `(user_id,
    provider)`.
    """
    user_id = psql(
        "INSERT INTO app_user (id, name, email, status) "
        f"VALUES (gen_random_uuid(), 'G8 Analyst', '{ANALYST_EMAIL}', 'active') "
        "ON CONFLICT (email) DO UPDATE SET status = 'active' RETURNING id;"
    )
    hasher = PasswordHasher()
    phc_hash = hasher.hash(ANALYST_PASSWORD)  # a real, standard Argon2id PHC string
    psql(
        "INSERT INTO auth_identity "
        "(provider, external_subject, app_user_id, password_hash, must_change_password) "
        f"VALUES ('local', '{user_id}', '{user_id}', '{phc_hash}', false) "
        "ON CONFLICT (provider, external_subject) DO UPDATE SET password_hash = EXCLUDED.password_hash;"
    )
    psql(
        "INSERT INTO app_user_role (user_id, role_id) "
        f"SELECT '{user_id}', r.id FROM role r WHERE r.name = 'Analyst' "
        "ON CONFLICT DO NOTHING;"
    )
    print("[g8] seeded Analyst login (idempotent upsert)")


def step_login_as_bootstrap_admin() -> requests.Session:
    """Logs in as the bootstrap admin, handling the forced first-login
    password rotation exactly as `ops/g3a/g3a_test.py::step_login` and
    `ops/g4/g4_test.py::step_login` already do (`must_change_password` is
    always `true` for a freshly bootstrapped credential —
    `main::bootstrap_admin`'s own doc comment). Every call below checks
    `.ok` explicitly rather than assuming success, so an auth failure
    (401) here is never mistaken for a later, unrelated failure."""
    session = requests.Session()
    login = session.post(
        f"{API_URL}/api/auth/login",
        json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
        timeout=10,
    )
    if not login.ok:
        raise G8Failure(f"admin login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = session.post(
            f"{API_URL}/api/auth/change-password",
            json={"newPassword": AUTH_PASSWORD},
            timeout=10,
        )
        if not rotated.ok:
            raise G8Failure(f"forced password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = session.post(
            f"{API_URL}/api/auth/login",
            json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
            timeout=10,
        )
        if not relogin.ok:
            raise G8Failure(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")
    print("[g8] logged in as bootstrap admin")
    return session


def step_login_as_analyst() -> requests.Session:
    session = requests.Session()
    login = session.post(
        f"{API_URL}/api/auth/login",
        json={"email": ANALYST_EMAIL, "password": ANALYST_PASSWORD},
        timeout=10,
    )
    if not login.ok:
        raise G8Failure(f"analyst login failed: {login.status_code} {login.text}")
    print("[g8] logged in as g8 analyst")
    return session


def step_author_masking_policy(admin_session: requests.Session) -> None:
    """Authors a `status = 'ready'` policy masking `email` on `GATE_TABLE`
    for role `Analyst`, via the real `POST /api/governance/policies`
    route (`rust/crates/lakehouse-api/src/routes/governance.rs::
    create_policy`) — never inserted directly into Postgres, so this gate
    also exercises `create_policy_body`'s own validation
    (`crate::policy_engine::PolicyCondition::parse`) of the `conditions`
    shape it authors.

    Idempotent by checking `GET /api/governance/policies` first and
    skipping the `POST` if a policy named `POLICY_NAME` already exists —
    `lakehouse_store::governance::create_policy` enforces a unique name
    and returns 409 on a repeat, so re-POSTing unconditionally would not
    be idempotent (`policy:name_unique`, `rust/crates/lakehouse-store/
    src/governance.rs::create_policy`'s doc comment). A read-then-create
    is safe here (no concurrent writer contends for this fixed name) and
    is chosen over swallowing a 409, so a genuine, unrelated 409 (e.g. a
    name collision from a different cause) still fails the gate instead
    of being silently treated as success.
    """
    conditions = (
        '{"roles":["Analyst"],"table":"' + GATE_TABLE + '","mask":["email"]}'
    )
    existing = admin_session.get(f"{API_URL}/api/governance/policies", timeout=10)
    if not existing.ok:
        raise G8Failure(f"failed to list policies: {existing.status_code} {existing.text}")
    if any(p.get("name") == POLICY_NAME for p in existing.json()):
        print(f"[g8] policy {POLICY_NAME!r} already authored; skipping POST")
        return
    resp = admin_session.post(
        f"{API_URL}/api/governance/policies",
        json={
            "name": POLICY_NAME,
            "kind": "Row filter",
            "subjects": "Analyst (g8 gate)",
            "resources": GATE_TABLE,
            "effect": "Permit with obligation",
            "conditions": conditions,
            "activate": True,
        },
        timeout=10,
    )
    if not resp.ok:
        raise G8Failure(f"failed to author masking policy: {resp.status_code} {resp.text}")
    print(f"[g8] authored policy {POLICY_NAME!r}")


def _run_query(session: requests.Session, sql: str) -> requests.Response:
    return session.post(f"{API_URL}/api/query/run", json={"sql": sql}, timeout=15)


def step_analyst_sees_masked_column(analyst_session: requests.Session) -> None:
    """The load-bearing assertion: if enforcement regressed to a
    no-op (Phase C/D's wiring reverted, or the policy's `conditions`
    stopped being read), `email` comes back as `GATE_EMAIL_VALUE` and
    this fails. `region` (never named by the policy) must stay in clear
    text — a masking bug that redacted the entire row, not just the
    named column, also fails here."""
    resp = _run_query(analyst_session, f"SELECT email, region FROM {GATE_TABLE} LIMIT 1")
    if not resp.ok:
        raise G8Failure(f"Analyst query failed: {resp.status_code} {resp.text}")
    rows = resp.json().get("rows", [])
    if not rows:
        raise G8Failure("Analyst query returned no rows — cannot prove masking from an empty result")
    row = rows[0]
    if row.get("email") != "***":
        raise G8Failure(f"expected a masked email for Analyst, got: {row}")
    if row.get("region") != GATE_REGION_VALUE:
        raise G8Failure(f"unmasked sibling column was altered too — masking is not column-scoped: {row}")


def step_admin_sees_clear_text(admin_session: requests.Session) -> None:
    """No authored policy names role `Platform Admin`, so the bootstrap
    admin's own read of the SAME table/column must come back in clear
    text. Fails if a regression over-masks (applies the Analyst
    obligation to every principal) as loudly as `step_analyst_sees_
    masked_column` fails an under-masking regression."""
    resp = _run_query(admin_session, f"SELECT email FROM {GATE_TABLE} LIMIT 1")
    if not resp.ok:
        raise G8Failure(f"admin query failed: {resp.status_code} {resp.text}")
    rows = resp.json().get("rows", [])
    if not rows:
        raise G8Failure("admin query returned no rows — cannot prove clear text from an empty result")
    if rows[0].get("email") != GATE_EMAIL_VALUE:
        raise G8Failure(f"admin's own read must not be masked (no policy names Platform Admin): {rows[0]}")


def step_bypass_attempts_still_masked(analyst_session: requests.Session) -> None:
    """Three syntactic shapes that `sql_rewrite`'s own test suite
    (`rust/crates/lakehouse-api/src/sql_rewrite.rs`) names as substitution
    targets, run as the Analyst: a `SELECT *`, and that same `SELECT *`
    read through a CTE. Each must either come back masked (`"***"`) or be
    refused outright with `422 Unprocessable`
    (`policy_engine::PolicyEngineError`/`RewriteError` ->
    `ApiError::Unprocessable`, `routes/query.rs::
    rewrite_sql_for_principal_inner`) — `sql_rewrite`'s documented
    "never a silent pass-through" rule.

    A `401`/`403` here is NEVER treated as an acceptable refusal — that
    would be the session losing its own authentication mid-gate, not the
    governance engine refusing a bypass, and conflating the two is
    exactly the "an unauthenticated call passes for free" failure mode
    this gate must not have.
    """
    for sql in [
        f"SELECT * FROM {GATE_TABLE} LIMIT 1",
        f"WITH x AS (SELECT * FROM {GATE_TABLE}) SELECT email FROM x LIMIT 1",
    ]:
        resp = _run_query(analyst_session, sql)
        if resp.status_code in (401, 403):
            raise G8Failure(
                f"bypass check itself failed to authenticate for {sql!r}: "
                f"{resp.status_code} {resp.text}"
            )
        if resp.status_code == 422:
            # A refusal is an acceptable outcome — never an unmasked leak
            # — see `sql_rewrite`'s "never a silent pass-through" rule
            # this docstring cites above.
            print(f"[g8] bypass {sql!r} was refused (422) — acceptable")
            continue
        if not resp.ok:
            raise G8Failure(f"unexpected failure evaluating bypass {sql!r}: {resp.status_code} {resp.text}")
        rows = resp.json().get("rows", [])
        if not rows:
            raise G8Failure(f"bypass {sql!r} returned no rows — cannot prove it was masked")
        if rows[0].get("email") != "***":
            raise G8Failure(f"bypass via {sql!r} leaked an unmasked value: {rows[0]}")
        print(f"[g8] bypass {sql!r} was masked — acceptable")


def main() -> int:
    try:
        step_seed_gate_table()
        step_seed_analyst_login()
        admin = step_login_as_bootstrap_admin()
        step_author_masking_policy(admin)
        analyst = step_login_as_analyst()
        step_analyst_sees_masked_column(analyst)
        step_admin_sees_clear_text(admin)
        step_bypass_attempts_still_masked(analyst)
    except G8Failure as exc:
        print(f"[g8] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g8] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
