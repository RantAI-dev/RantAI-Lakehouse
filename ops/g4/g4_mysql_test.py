#!/usr/bin/env python3
"""G4 acceptance test for MySQL change capture (`SRC-4` slice 3, plan task T3.7; ADR 0016).

The steps are the shared ones of `cdc_gate.py` (see its module doc): the connector is created
through the API, 20,000 rows per table are copied, each kind of change is visible within 120 s
in both load modes, a pause and a restart of `kafka-connect` lose nothing, a schema change is
observed, `DELETE` removes the connector and its password file, and the schema-history topic
`cdc.<id>.schema-history` has unlimited retention (decision S3-d). This file holds what only
MySQL has.

# The source

`mysql-g6` (the `mysql:8` fixture of the g6 gates), a database `g4_cdc` of its own with two
identical tables and a numbers table (`MySQL` has no `generate_series`). The connector dials as a
LEAST-PRIVILEGE user, not `root`, so the grants the documentation lists are proven to be enough
(decision S3-e): `SELECT` on `g4_cdc.*` and the global `RELOAD`, `SHOW DATABASES`,
`REPLICATION SLAVE` and `REPLICATION CLIENT`. The gate asserts, and does not assume, the three
server settings Debezium needs: `log_bin` on, `binlog_format=ROW`, `binlog_row_image=FULL` (MySQL
8's defaults). TLS is `require` (MySQL 8's own self-signed certificate); the verify modes are
covered by the renderer's unit tests.
"""

from __future__ import annotations

import os

import cdc_gate
from cdc_gate import G4Failure

DATABASE = "g4_cdc"
HOST = "mysql-g6"
PORT = 3306
CDC_USER = "g4_cdc"
# The server id the connector presents as a replica: any number no other replica of this server
# uses (the fixture's own `server_id` is 1).
SERVER_ID = 5400


def _connect(database: str | None = None):
    import pymysql

    return pymysql.connect(
        host=HOST, port=PORT, user="root", password=os.environ["MYSQL_ROOT_PASSWORD"],
        database=database, autocommit=True, connect_timeout=10,
    )


class MySqlSource(cdc_gate.Source):
    cdc_type = "MySQL CDC"
    gate = "g4-mysql"
    catalog_db = "icecat_g4_mysql"
    schema = DATABASE
    host_for_registry = f"{HOST}:{PORT}"
    now = "NOW(6)"
    needs_schema_history = True

    def __init__(self) -> None:
        # The connector user's password: this stack's own throwaway fixture value.
        self.password = os.environ["CONNECTOR_MYSQL_PASSWORD"]

    def ping(self) -> None:
        _connect().close()

    def execute(self, sql: str) -> None:
        # One connection per statement: the change writer runs in its own thread.
        conn = _connect()
        try:
            with conn.cursor() as cur:
                cur.execute(sql)
        finally:
            conn.close()

    def rows(self, sql: str) -> list[tuple[str, ...]]:
        conn = _connect()
        try:
            with conn.cursor() as cur:
                cur.execute(sql)
                return [tuple("" if v is None else str(v) for v in row) for row in cur.fetchall()]
        finally:
            conn.close()

    def qualified(self, table: str) -> str:
        return f"{DATABASE}.{table}"

    def numbers(self, lo: int, hi: int) -> str:
        return f"SELECT g FROM {DATABASE}.g4_numbers WHERE g BETWEEN {lo} AND {hi}"

    def dial(self) -> dict:
        return {
            "driver": "mysql", "host": HOST, "port": PORT, "database": DATABASE,
            "user": CDC_USER, "serverId": SERVER_ID, "sslMode": "require",
        }

    def _assert_binlog_settings(self) -> None:
        """The settings are asserted, never assumed (decision S3-e)."""
        wanted = {"log_bin": "ON", "binlog_format": "ROW", "binlog_row_image": "FULL"}
        found = {}
        for name in wanted:
            rows = self.rows(f"SHOW VARIABLES LIKE '{name}'")
            found[name] = rows[0][1].upper() if rows else ""
        if found != wanted:
            raise G4Failure(f"the MySQL fixture does not have the binlog settings Debezium needs: {found}, wanted {wanted}")

    def setup(self) -> None:
        self._assert_binlog_settings()
        self.execute(f"DROP DATABASE IF EXISTS {DATABASE}")
        self.execute(f"CREATE DATABASE {DATABASE}")
        self.execute(f"CREATE TABLE {DATABASE}.g4_numbers (g INT PRIMARY KEY)")
        conn = _connect()
        try:
            with conn.cursor() as cur:
                # `cte_max_recursion_depth` defaults to 1000; the bulk step needs numbers to 105,000.
                cur.execute("SET SESSION cte_max_recursion_depth = 200000")
                cur.execute(
                    f"INSERT INTO {DATABASE}.g4_numbers WITH RECURSIVE n(g) AS "
                    "(SELECT 1 UNION ALL SELECT g + 1 FROM n WHERE g < 110000) SELECT g FROM n"
                )
        finally:
            conn.close()
        for table in cdc_gate.TABLES.values():
            self.execute(
                f"CREATE TABLE {DATABASE}.{table} (id BIGINT PRIMARY KEY, customer VARCHAR(100), "
                "amount DECIMAL(12,2), status VARCHAR(50), updated_at DATETIME(6))"
            )
            self.execute(
                f"INSERT INTO {DATABASE}.{table} SELECT g, CONCAT('seed_', g), "
                "CAST(((g % 1000) + 0.5) AS DECIMAL(12,2)), 'seed', NOW(6) "
                f"FROM {DATABASE}.g4_numbers WHERE g BETWEEN 1 AND {cdc_gate.SEED_ROWS}"
            )
        # The least-privilege user the connector dials (decision S3-e).
        conn = _connect()
        try:
            with conn.cursor() as cur:
                cur.execute(f"DROP USER IF EXISTS '{CDC_USER}'@'%'")
                # `%%`: the statement has a parameter, so pymysql applies Python's % formatting to it.
                cur.execute(f"CREATE USER '{CDC_USER}'@'%%' IDENTIFIED BY %s", (self.password,))
                cur.execute(f"GRANT SELECT ON {DATABASE}.* TO '{CDC_USER}'@'%'")
                cur.execute(
                    f"GRANT RELOAD, SHOW DATABASES, REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO '{CDC_USER}'@'%'"
                )
        finally:
            conn.close()

    def add_column_and_observe(self) -> None:
        self.execute(f"ALTER TABLE {DATABASE}.orders ADD COLUMN note VARCHAR(100)")
        self.execute(f"UPDATE {DATABASE}.orders SET note = 'schema_note' WHERE id = 2")


def main() -> int:
    return cdc_gate.run(MySqlSource())


if __name__ == "__main__":
    raise SystemExit(main())
