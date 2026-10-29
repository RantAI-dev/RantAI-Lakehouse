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

- `connect`: refused, honestly, until `SRC-4` T1.1 lands. The API has no
  Oracle client; `POST .../test` answers `supported: false` (SRC-5,
  `connector_probe.rs`).
- `discover`: real (SRC-13 D5, D12). The orchestrator lists Oracle: the
  schemas the login can read hold `G6_INGEST`, that schema's tables hold
  `G6_INGEST.ORDERS`, and the preview shows its declared columns `ID`,
  `AMOUNT` and `NOTE` (types not inferred). The source object of the load
  below is that listed name, `G6_INGEST.ORDERS`, so the gate also proves that
  what the listing offers loads (SRC-13 review 1 SHOULD-FIX 1: the schema part
  and the upper-case spelling).
- `load`: real (`SRC-4` T1.2, T1.3). The table is saved with column choice
  (`NOTE` left out, spelled as the preview spells it) and the `incremental`
  load mode with the cursor `ID`. The run succeeds, Bronze holds every seeded
  row, has `id` and `amount`, and has no `note` column.
- `reload`: real (`SRC-4` T1.3). Two rows are inserted at the source and the
  run again: only those two rows are read (the run's own row count is 2, not
  the 5 of the table) and Bronze holds the five, no row twice.

# The dial, over TLS (`SRC-4` T1.4)

`driver: oracle`, `sslMode: required`, `sslServerCertDn: CN=oracle-tls-g6`,
host `oracle-tls-g6` and port 2484: the TLS front of the fixture
(`ops/g6/oracle-tls-proxy.conf`), with a certificate from a CA the gate's
`oracle-g6-cert-init` generates and the Dagster code location trusts through
`SSL_CERT_FILE`. `database` is the pluggable database's service name (the
console's field is labelled "Database (service name)",
`oracle-dial-form.tsx`). Everything the orchestrator does with this connector
(discovery, the loads) is one `tcps` dial with the chain and the DN checked.
Limit of what is proven: Oracle's own TCPS listener and wallet are not
exercised, the proxy terminates TLS in front of the plaintext listener (the
reason is in the proxy's config file). The fixture seeds over plaintext (the
gate's own login, not the product's).

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

# Where the gate's own seeding logs in: the listener, in clear text (not the product's dial).
ORACLE_HOST = "oracle-g6"
ORACLE_PORT = 1521
# Where the connector dials: the TLS front, and the DN of its certificate (CN = the host name).
ORACLE_TLS_HOST = "oracle-tls-g6"
ORACLE_TLS_PORT = 2484
ORACLE_TLS_DN = "CN=oracle-tls-g6"
# The default pluggable database of the image; the application user exists
# there, and the connector's `database` is its service name.
ORACLE_SERVICE = "FREEPDB1"
ORACLE_USER = "g6_ingest"
# Unquoted Oracle identifiers fold to upper case: the user's schema.
ORACLE_SCHEMA = "G6_INGEST"
CONNECTOR_ORACLE_PASSWORD = os.environ.get("CONNECTOR_ORACLE_PASSWORD", "")

# The name the listing offers for the fixture table, and what the load's source object is called
# (SRC-13: a listed name must load).
SOURCE_OBJECT = f"{ORACLE_SCHEMA}.ORDERS"
FIXTURE_ROWS = [(1, 10.5, "one"), (2, 20.25, "two"), (3, 30.0, "three")]
# Inserted before the second run: the only rows an incremental load may read then.
NEW_ROWS = [(4, 40.0, "four"), (5, 50.5, "five")]
# The column left out of the load, spelled as the preview spells it (Oracle's own, upper case); the
# cursor likewise. Bronze has the reflected spelling, lower case (`adapters/oracle.py`).
LEFT_OUT_COLUMN = "NOTE"
CURSOR_COLUMN = "ID"
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
    """`G6_INGEST.ORDERS(ID, AMOUNT, NOTE)` with `FIXTURE_ROWS`, as the application
    user. Idempotent: rows are replaced, not added to."""
    conn = _connect_oracle()
    try:
        with conn.cursor() as cur:
            cur.execute(
                "BEGIN EXECUTE IMMEDIATE 'CREATE TABLE orders (id NUMBER(10) PRIMARY KEY, amount NUMBER(10,2), "
                "note VARCHAR2(40))'; "
                "EXCEPTION WHEN OTHERS THEN IF SQLCODE != -955 THEN RAISE; END IF; END;"
            )
            cur.execute("DELETE FROM orders")
            cur.executemany("INSERT INTO orders (id, amount, note) VALUES (:1, :2, :3)", FIXTURE_ROWS)
        conn.commit()
    finally:
        conn.close()


def _install_oracle_into_matrix() -> None:
    """Point the matrix module's lookups at the one connector this gate runs.

    Its checks look the type, Bronze table and discovery fixture up in module
    level dicts by connector name, and `_step_reload` walks all of `TYPE_OF`
    (it would record a `skipped` for every other connector type). Rebinding
    the names, rather than adding to the dicts, leaves the matrix with only
    Oracle in it for the whole run. `DISCOVER_FIXTURES` carries what the
    discover check must find: Oracle's own upper-case spelling, and declared
    (not inferred) column types."""
    matrix.TYPE_OF = {NAME: TYPE_NAME}
    matrix.BRONZE_OF = {NAME: BRONZE_TABLE}
    matrix.DISCOVER_FIXTURES = {
        TYPE_NAME: {
            "schema": ORACLE_SCHEMA, "object": SOURCE_OBJECT, "kind": "table",
            "columns": {"ID", "AMOUNT", "NOTE"}, "inferred": False,
        }
    }


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
    try:
        columns = matrix._bronze_columns(BRONZE_TABLE)
    except matrix.G6Failure as exc:
        matrix._record_fail(TYPE_NAME, "load", str(exc))
        return None
    if LEFT_OUT_COLUMN.lower() in columns or not {"id", "amount"} <= set(columns):
        matrix._record_fail(
            TYPE_NAME, "load",
            f"expected id and amount and no {LEFT_OUT_COLUMN.lower()} in bronze.{BRONZE_TABLE}, got {columns}",
        )
        return None
    matrix._record(
        TYPE_NAME, "load", "pass",
        f"{count} rows in bronze.{BRONZE_TABLE}, {LEFT_OUT_COLUMN} left out, over tcps",
    )
    return count


def _insert_new_rows() -> None:
    """The rows the second run must pick up, as the application user."""
    conn = _connect_oracle()
    try:
        with conn.cursor() as cur:
            cur.executemany("INSERT INTO orders (id, amount, note) VALUES (:1, :2, :3)", NEW_ROWS)
        conn.commit()
    finally:
        conn.close()


def _check_reload(connector_id: str, first_count: int) -> None:
    """`reload` (`SRC-4` T1.3): new rows at the source, a second run, and only
    those rows read. What the run wrote, from `GET /api/governance/ingest-runs`,
    tells an incremental load from a `replace`, which would also leave five rows
    in Bronze but read all five."""
    want = first_count + len(NEW_ROWS)
    try:
        _insert_new_rows()
        run_id = matrix._post_ingest_run(connector_id).json().get("runId")
        if not run_id:
            matrix._record_fail(TYPE_NAME, "reload", "the second ingest/run returned no runId")
            return
        matrix.step_wait_for_run_success(run_id)
        matrix._wait_for(
            f"bronze.{BRONZE_TABLE} has {want} rows after the second run",
            lambda: matrix._bronze_row_count(BRONZE_TABLE) == want or None, 30, interval_s=3.0,
        )
        runs = matrix._src10_runs_of(connector_id, SOURCE_OBJECT)  # newest first
        latest = runs[0] if runs else None
        if latest is None or latest.get("status") != "succeeded" or latest.get("rows") != len(NEW_ROWS):
            matrix._record_fail(
                TYPE_NAME, "reload",
                f"expected the second run to read exactly the {len(NEW_ROWS)} new rows, got "
                f"{None if latest is None else {k: latest.get(k) for k in ('status', 'rows')}}",
            )
            return
        # A freshly written Iceberg table can show its previous snapshot to ClickHouse for a moment: read again.
        time.sleep(6.0)
        again = matrix._bronze_row_count(BRONZE_TABLE)
    except matrix.G6Failure as exc:
        matrix._record_fail(TYPE_NAME, "reload", str(exc))
        return
    if again != want:
        matrix._record_fail(TYPE_NAME, "reload", f"bronze.{BRONZE_TABLE} had {want} rows after the second run and {again} a few seconds later")
        return
    matrix._record(
        TYPE_NAME, "reload", "pass",
        f"{len(NEW_ROWS)} new rows read, {want} rows in bronze.{BRONZE_TABLE}: incremental brought only the new rows",
    )


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
        name=NAME, kind=TYPE_NAME, host=f"{ORACLE_TLS_HOST}:{ORACLE_TLS_PORT}",
        credential={"source": "file", "primary": "password"},
        credential_values={"primary": CONNECTOR_ORACLE_PASSWORD},
        adapter="sql",
        dial={
            "driver": "oracle", "host": ORACLE_TLS_HOST, "port": ORACLE_TLS_PORT, "database": ORACLE_SERVICE,
            "user": ORACLE_USER, "sslMode": "required", "sslServerCertDn": ORACLE_TLS_DN,
        },
        source_objects=[{
            "name": SOURCE_OBJECT, "target": BRONZE_TABLE, "loadMode": "incremental",
            "incrementalKey": CURSOR_COLUMN, "excludedColumns": [LEFT_OUT_COLUMN],
        }],
        connector_ids=connector_ids,
    )
    connector_id = connector_ids[NAME]
    # While the first run is in flight: `connect` is a quick, read-only call. `discover` is Dagster runs of its
    # own (the orchestrator lists Oracle), so it waits for the load below to end and not for a run slot.
    matrix._check_connect(NAME, connector_id)

    if run_id is None:
        matrix._record_fail(TYPE_NAME, "load", "ingest/run answered supported=false for an Oracle sql connector")
        matrix._check_discover(NAME, connector_id)
        return
    count = _check_load(run_id)
    matrix._check_discover(NAME, connector_id)
    if count is None:
        matrix._record(TYPE_NAME, "reload", "skipped", "the first load did not complete, so there is nothing to load again")
        return
    _check_reload(connector_id, count)


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
