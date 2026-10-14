#!/usr/bin/env python3
"""G6 Oracle gate (SRC-9 plan task 5): one Oracle connector, registered the
way the console registers it, taken through the four steps of the connector
gates -- connect, discover, load, reload -- against Oracle Database Free
(`oracle-g6` in `ops/g6/docker-compose.g6-oracle.override.yml`).

It is a job of its own (`g6-oracle`) and a gate of its own, but not a second
copy of the matrix: login, tenant, registration, the connect and discover
checks, the run wait, the Bronze counts and the reload step are the
functions of `g6_ingest_matrix_test.py`, imported (that module's `main` is
guarded, and it reads its settings from the environment without side effects).
Only what is Oracle's own lives here: the fixture and the dial.

# What each step must show (`ops/gates/coverage.json`, type `Oracle`)

- `connect`: refused, honestly. The API has no Oracle client; `POST .../test`
  answers `supported: false` (SRC-5, `connector_probe.rs`).
- `discover`: refused, honestly. `POST .../discover` answers
  `supported: false` for Oracle (SRC-13; SRC-9 finding G2 changed it from an
  HTTP 422). `?schema=` is sent: without it every sql connector gets a
  `supported: false` for a different reason ("the schema parameter is
  required") and the check would pass for the wrong cause.
- `load`: real. The run succeeds and Bronze holds every seeded row.
- `reload`: real. The connector has no `loadMode`, so the default `replace`
  applies and a second run leaves the same rows.

# The dial

`driver: oracle`, host and port of the container, `database` = the pluggable
database's service name (the console's field is labelled "Database (service
name)", `oracle-dial-form.tsx`), a literal user, and no `sslMode`. The Oracle
adapter refuses TLS without an operator-typed `sslServerCertDn`
(`adapters/oracle.py`, `OracleTlsConfigError`), and a fixture container has no
certificate whose DN means anything, so this gate uses the plaintext mode the
dial allows. That is a limit of what is proven: TLS to Oracle is not
exercised here.

# Settings (environment)

`CONNECTOR_ORACLE_PASSWORD` (required: no default credential): the password of
the container's application user, which the connector dials and the fixture
is seeded as. The rest is `g6_ingest_matrix_test.py`'s (`LAKEHOUSE_API_URL`,
`DAGSTER_URL`, `CH_*`, `AUTH_BOOTSTRAP_*`).

Exit code 0 and `[g6-oracle] PASS`, or `[g6-oracle] FAILED: ...` on stderr and
exit code 1. The coverage table is printed on every path.
"""
from __future__ import annotations

import os
import sys
import time

import g6_ingest_matrix_test as matrix
import gate_summary

GATE = "g6-oracle"
NAME = "g6-oracle"
TYPE_NAME = "Oracle"
BRONZE_TABLE = "g6_oracle_orders"

ORACLE_HOST = "oracle-g6"
ORACLE_PORT = 1521
# The default pluggable database of the image; the application user exists
# there, and the connector's `database` is its service name.
ORACLE_SERVICE = "FREEPDB1"
ORACLE_USER = "g6_ingest"
# Unquoted Oracle identifiers fold to upper case: the user's schema.
ORACLE_SCHEMA = "G6_INGEST"
CONNECTOR_ORACLE_PASSWORD = os.environ.get("CONNECTOR_ORACLE_PASSWORD", "")

FIXTURE_ROWS = [(1, 10.5), (2, 20.25), (3, 30.0)]
SIGN_IN_TIMEOUT_S = 300


def _connect_oracle():
    """A thin-mode login as the application user: the same driver and mode the
    orchestrator's adapter uses (`oracledb`, no Instant Client)."""
    import oracledb
    return oracledb.connect(
        user=ORACLE_USER, password=CONNECTOR_ORACLE_PASSWORD,
        dsn=f"{ORACLE_HOST}:{ORACLE_PORT}/{ORACLE_SERVICE}",
    )


def _wait_for_sign_in() -> None:
    """`healthcheck.sh` is true when the pluggable database is open; the
    application user is created while it is initialising, so the gate also
    waits for a real login before it seeds anything."""
    def can_sign_in() -> bool:
        conn = _connect_oracle()
        conn.close()
        return True

    matrix._wait_for("Oracle application user can sign in", can_sign_in, SIGN_IN_TIMEOUT_S, interval_s=5.0)


def _seed_oracle_fixture() -> None:
    """`G6_INGEST.ORDERS(ID, AMOUNT)` with `FIXTURE_ROWS`, as the application
    user. Idempotent: rows are replaced, not added to."""
    conn = _connect_oracle()
    try:
        with conn.cursor() as cur:
            cur.execute(
                "BEGIN EXECUTE IMMEDIATE 'CREATE TABLE orders (id NUMBER(10) PRIMARY KEY, amount NUMBER(10,2))'; "
                "EXCEPTION WHEN OTHERS THEN IF SQLCODE != -955 THEN RAISE; END IF; END;"
            )
            cur.execute("DELETE FROM orders")
            cur.executemany("INSERT INTO orders (id, amount) VALUES (:1, :2)", FIXTURE_ROWS)
        conn.commit()
    finally:
        conn.close()


def _install_oracle_into_matrix() -> None:
    """Point the matrix module's lookups at the one connector this gate runs.

    Its checks look the type, Bronze table and discovery fixture up in module
    level dicts by connector name, and `_step_reload` walks all of `TYPE_OF`
    (it would record a `skipped` for every other connector type). Rebinding
    the names, rather than adding to the dicts, leaves the matrix with only
    Oracle in it for the whole run. `DISCOVER_FIXTURES` carries the schema the
    discover check sends; its table and columns are unused for a refusal."""
    matrix.TYPE_OF = {NAME: TYPE_NAME}
    matrix.BRONZE_OF = {NAME: BRONZE_TABLE}
    matrix.DISCOVER_FIXTURES = {TYPE_NAME: (ORACLE_SCHEMA, f"{ORACLE_SCHEMA}.ORDERS", set())}


def _check_load(run_id: str) -> int | None:
    """`load`: the run succeeds and Bronze holds every seeded row, each with
    `_ingested_at`. Returns the row count, or None after recording a failure."""
    try:
        matrix.step_wait_for_run_success(run_id)
        count = matrix._wait_for(
            f"bronze.{BRONZE_TABLE} rows visible", lambda: matrix._bronze_row_count(BRONZE_TABLE) or None, 30, interval_s=3.0
        )
        null_ingested_at = matrix._bronze_ingested_at_null_count(BRONZE_TABLE)
    except matrix.G6Failure as exc:
        matrix._record_fail(TYPE_NAME, "load", str(exc))
        return None
    if count != len(FIXTURE_ROWS):
        matrix._record_fail(TYPE_NAME, "load", f"expected {len(FIXTURE_ROWS)} rows in bronze.{BRONZE_TABLE}, got {count}")
        return None
    if null_ingested_at != 0:
        matrix._record_fail(
            TYPE_NAME, "load",
            f"expected _ingested_at on every row of bronze.{BRONZE_TABLE}, {null_ingested_at} of {count} rows lack it",
        )
        return None
    matrix._record(TYPE_NAME, "load", "pass", f"{count} rows in bronze.{BRONZE_TABLE}")
    return count


def run() -> None:
    registry = gate_summary.Registry.load()
    owner = registry.gate_of(TYPE_NAME)
    if owner != GATE:
        raise matrix.G6Failure(f"ops/gates/coverage.json gives {TYPE_NAME!r} to gate {owner!r}, not {GATE!r}")
    matrix.RECORDER = gate_summary.Recorder(registry, GATE)
    _install_oracle_into_matrix()
    if not CONNECTOR_ORACLE_PASSWORD:
        raise matrix.G6Failure("set CONNECTOR_ORACLE_PASSWORD: the gate has no default credential")

    _wait_for_sign_in()
    matrix.step_wait_for_services()
    matrix.step_login()
    matrix.step_ensure_tenant()
    _seed_oracle_fixture()

    connector_ids: dict[str, str] = {}
    run_id = matrix._register_and_run(
        name=NAME, kind=TYPE_NAME, host=f"{ORACLE_HOST}:{ORACLE_PORT}",
        credential={"source": "file", "primary": "password"},
        credential_values={"primary": CONNECTOR_ORACLE_PASSWORD},
        adapter="sql",
        dial={"driver": "oracle", "host": ORACLE_HOST, "port": ORACLE_PORT, "database": ORACLE_SERVICE, "user": ORACLE_USER},
        source_objects=[{"name": "orders", "target": BRONZE_TABLE}],
        connector_ids=connector_ids,
    )
    connector_id = connector_ids[NAME]
    # While the first run is in flight: both are quick, read-only calls.
    matrix._check_connect(NAME, connector_id)
    matrix._check_discover(NAME, connector_id)

    if run_id is None:
        matrix._record_fail(TYPE_NAME, "load", "ingest/run answered supported=false for an Oracle sql connector")
        return
    count = _check_load(run_id)
    if count is None:
        matrix._record(TYPE_NAME, "reload", "skipped", "the first load did not complete, so there is nothing to load again")
        return
    matrix._step_reload({NAME: connector_id}, {NAME: count})


def main() -> int:
    sys.stdout.reconfigure(line_buffering=True)
    sys.stderr.reconfigure(line_buffering=True)
    started = time.time()
    try:
        run()
        failed = matrix._recorder().failures() if matrix.RECORDER is not None else []
        if failed:
            raise matrix.G6Failure(
                "connector gate steps failed: " + "; ".join(f"{t} {s}: {d}" for t, s, d in failed)
            )
    except matrix.G6Failure as exc:
        print(f"[g6-oracle] FAILED: {exc}", file=sys.stderr)
        return 1
    except Exception:  # noqa: BLE001 -- any other error is a failure of the gate and must be named as one
        import traceback
        print(f"[g6-oracle] FAILED: unexpected error, not a G6Failure:\n{traceback.format_exc()}", file=sys.stderr)
        return 1
    finally:
        # The table is printed on every path, as the matrix gate does.
        if matrix.RECORDER is not None:
            matrix.RECORDER.emit()
    print(f"[g6-oracle] PASS ({time.time() - started:.0f}s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
