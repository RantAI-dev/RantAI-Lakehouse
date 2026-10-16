#!/usr/bin/env python3
"""SRC-13 gate (task 8, decisions D9 and D12): listing a PostgreSQL schema of
10,000 tables completes within 2 minutes and is searchable.

It is a job of its own (`g6-discovery-scale`), so no existing gate job grows.
The gate creates the schema `scale_10k` with 10,000 tables in the stack's own
PostgreSQL (the database the seeded connector `conn-pg-lakehouse` points at),
then through the API, as the console does:

1. lists the schemas (`POST .../discovery/list`, no schema): `scale_10k` is
   among them;
2. lists the tables of `scale_10k` and prints the seconds the call took. The
   list must hold exactly 10,000 names, not truncated, and the call must take
   no more than 120 seconds (D9's limit; the listing's own limit in the API is
   the same). The gate FAILS above it;
3. searches one table by part of its name on the server (`search`) and prints
   the seconds; the answer is that table only;
4. previews that table: its two columns and its key (names first, columns when
   a table is opened: the other half of D9).

# What is measured, and what is not

The time is the HTTP call as the console makes it, from the runner container to
the API on the compose network, over a freshly created schema on a CI runner.
It is a measurement of this stack, not a promise for a customer's database: the
catalogue query is the same, but a remote server adds its own latency. The
numbers are printed on lines starting `[src13]` and copied to the job summary.

# The tables

`scale_10k.t00000` to `t09999`, each `(id integer PRIMARY KEY, name text)`.
They are created in batches of 500 inside one transaction each: a single
transaction holding 10,000 tables (and their indexes) would need more locks
than PostgreSQL's default `max_locks_per_transaction` allows ("out of shared
memory"). The schema is dropped first, so a rerun starts clean.

# Settings (environment)

`LAKEHOUSE_API_URL`, `AUTH_BOOTSTRAP_EMAIL`, `AUTH_BOOTSTRAP_PASSWORD` as
`g6_ingest_matrix_test.py` reads them (its login and tenant helpers are
imported; nothing in that module runs on import); `PGHOST`, `PGPORT`, `PGUSER`,
`PGPASSWORD`, `PGDATABASE` for the runner's own connection to PostgreSQL.

Exit code 0 and `[src13] PASS`, or `[src13] FAILED: ...` on stderr and exit
code 1.
"""
from __future__ import annotations

import os
import sys
import time

import g6_ingest_matrix_test as matrix

SCHEMA = "scale_10k"
TABLES = 10_000
BATCH = 500
# Decision D9: the listing of 10,000 tables completes within two minutes.
LIST_LIMIT_S = 120.0
# One table, picked far from the ends so a search for part of its name is not
# satisfied by a prefix or suffix of the list.
NEEDLE_INDEX = 4242
CONNECTOR_ID = "conn-pg-lakehouse"


class ScaleGateFailure(Exception):
    pass


def _log(message: str) -> None:
    print(f"[src13] {message}", flush=True)


def _table_name(i: int) -> str:
    return f"t{i:05d}"


def _create_tables() -> None:
    import psycopg2

    conn = psycopg2.connect(
        host=os.environ.get("PGHOST", "postgres"),
        port=int(os.environ.get("PGPORT", "5432")),
        user=os.environ.get("PGUSER", "lakehouse"),
        password=os.environ.get("PGPASSWORD", "lakehouse"),
        dbname=os.environ.get("PGDATABASE", "lakehouse"),
    )
    try:
        with conn.cursor() as cur:
            cur.execute(f"DROP SCHEMA IF EXISTS {SCHEMA} CASCADE")
            cur.execute(f"CREATE SCHEMA {SCHEMA}")
        conn.commit()
        started = time.time()
        for first in range(0, TABLES, BATCH):
            with conn.cursor() as cur:
                for i in range(first, min(first + BATCH, TABLES)):
                    # Constant identifiers built from an integer, never from input.
                    cur.execute(f"CREATE TABLE {SCHEMA}.{_table_name(i)} (id integer PRIMARY KEY, name text)")
            conn.commit()
        _log(f"created {TABLES} tables in schema {SCHEMA} in {time.time() - started:.1f} s")
    finally:
        conn.close()


def _discovery(kind: str, body: dict) -> tuple[dict, float]:
    """One call as the console makes it; the seconds it took. These answers are
    in the call itself (PostgreSQL is read by the API), so there is no polling."""
    url = f"{matrix.API_URL}/api/connectors/{CONNECTOR_ID}/discovery/{kind}"
    started = time.time()
    resp = matrix.API.post(url, json=body, timeout=135)
    took = time.time() - started
    if not resp.ok:
        raise ScaleGateFailure(f"POST .../discovery/{kind} {body} answered {resp.status_code} {resp.text[:200]}")
    return resp.json(), took


def run() -> None:
    matrix._wait_for("lakehouse-api", lambda: matrix.API.get(f"{matrix.API_URL}/health", timeout=5).status_code == 200, 120)
    matrix.step_login()
    matrix.step_ensure_tenant()
    _create_tables()

    schemas, took = _discovery("list", {})
    names = [o.get("name") for o in schemas.get("objects") or [] if o.get("kind") == "schema"]
    if schemas.get("state") != "done" or SCHEMA not in names:
        raise ScaleGateFailure(f"expected the schemas to hold {SCHEMA!r}, got state={schemas.get('state')!r} {names[:20]}")
    _log(f"schemas listed in {took:.2f} s ({len(names)} schemas)")

    listing, took = _discovery("list", {"schema": SCHEMA})
    objects = listing.get("objects") or []
    _log(f"{len(objects)} tables listed in {took:.2f} s (limit {LIST_LIMIT_S:.0f} s)")
    if listing.get("state") != "done":
        raise ScaleGateFailure(
            f"the listing of {SCHEMA} ended {listing.get('state')!r} errorClass={listing.get('errorClass')!r} "
            f"reason={listing.get('reason')!r}"
        )
    expected = {f"{SCHEMA}.{_table_name(i)}" for i in range(TABLES)}
    if {o.get("name") for o in objects} != expected or listing.get("truncated"):
        raise ScaleGateFailure(
            f"expected exactly {TABLES} tables not truncated, got {len(objects)} truncated={listing.get('truncated')!r}"
        )
    if took > LIST_LIMIT_S:
        raise ScaleGateFailure(f"listing {TABLES} tables took {took:.1f} s, above the {LIST_LIMIT_S:.0f} s limit")

    needle = _table_name(NEEDLE_INDEX)
    found, took = _discovery("list", {"schema": SCHEMA, "search": needle.upper()})
    hits = [o.get("name") for o in found.get("objects") or []]
    _log(f"search for {needle.upper()!r} answered in {took:.2f} s with {hits}")
    if found.get("state") != "done" or hits != [f"{SCHEMA}.{needle}"]:
        raise ScaleGateFailure(f"expected the search to find only {SCHEMA}.{needle}, got state={found.get('state')!r} {hits[:10]}")

    preview, took = _discovery("preview", {"object": f"{SCHEMA}.{needle}"})
    columns = [c.get("name") for c in preview.get("columns") or []]
    _log(f"columns of {SCHEMA}.{needle} read in {took:.2f} s: {columns}, key {preview.get('primaryKey')}")
    if preview.get("state") != "done" or columns != ["id", "name"] or preview.get("primaryKey") != ["id"]:
        raise ScaleGateFailure(f"expected columns id, name and key id, got {preview}")


def main() -> int:
    sys.stdout.reconfigure(line_buffering=True)
    sys.stderr.reconfigure(line_buffering=True)
    started = time.time()
    try:
        run()
    except (ScaleGateFailure, matrix.G6Failure) as exc:
        print(f"[src13] FAILED: {exc}", file=sys.stderr)
        return 1
    except Exception:  # noqa: BLE001 -- any other error is a failure of the gate and must be named as one
        import traceback

        print(f"[src13] FAILED: unexpected error:\n{traceback.format_exc()}", file=sys.stderr)
        return 1
    _log(f"PASS ({time.time() - started:.0f} s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
