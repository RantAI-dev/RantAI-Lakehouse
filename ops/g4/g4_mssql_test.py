#!/usr/bin/env python3
"""G4 acceptance test for SQL Server change capture (`SRC-4` slice 3, plan task T3.8; ADR 0016).

The steps are the shared ones of `cdc_gate.py` (see its module doc). This file holds what only
SQL Server has.

# The source

`mssql-g6` (the SQL Server 2022 fixture of the g6 gates) with SQL Server Agent switched on by the
gate's override (`MSSQL_AGENT_ENABLED`: CDC's capture job runs in the Agent; without it nothing is
captured). A database `g4_cdc` of its own, CDC enabled on it (`sys.sp_cdc_enable_db`) and on each
of the two identical tables (`sys.sp_cdc_enable_table`) by `sa`, an administrator, as the
documentation says. The connector dials as a LEAST-PRIVILEGE login, not `sa` (decision S3-e):
`SELECT` on the two tables and on the `cdc` schema, plus `VIEW DATABASE STATE`. If Debezium 3.8
turns out to need more, the gate fails and the plan says: stop and report what and why.

# A schema change needs a second capture instance

SQL Server does not pick up a new column by itself: a capture instance has the columns the table
had when it was enabled. The gate follows Debezium's "online schema update" procedure: add the
column, enable a SECOND capture instance (`dbo_orders_v2`) on the table, then write a row;
Debezium switches to the new instance, and the column reaches Bronze. The old instance is left in
place (a table may have two); an operator drops it once Debezium has switched. "Limits" in the
feature page gives the steps.

# TLS

`require` (the connector encrypts and takes the certificate as presented): the fixture's private CA
is not in Kafka Connect's trust store. The verify modes are covered by the renderer's unit tests.
"""

from __future__ import annotations

import os

import cdc_gate

DATABASE = "g4_cdc"
HOST = "mssql-g6"
PORT = 1433
CDC_USER = "g4_cdc"
SCHEMA = "dbo"


def _connect(database: str = "master", user: str = "sa", password: str | None = None):
    import pyodbc

    pwd = password if password is not None else os.environ["MSSQL_SA_PASSWORD"]
    return pyodbc.connect(
        "DRIVER={ODBC Driver 18 for SQL Server};"
        f"SERVER=tcp:{HOST},{PORT};UID={user};PWD={{{pwd}}};DATABASE={database};"
        "Encrypt=yes;TrustServerCertificate=yes;",
        autocommit=True,
        timeout=10,
    )


class MsSqlSource(cdc_gate.Source):
    cdc_type = "SQL Server CDC"
    gate = "g4-mssql"
    catalog_db = "icecat_g4_mssql"
    schema = SCHEMA
    host_for_registry = f"{HOST}:{PORT}"
    now = "SYSDATETIME()"
    needs_schema_history = True

    def __init__(self) -> None:
        # The connector login's password: a throwaway of this gate's stack that meets SQL Server's
        # complexity rule (`G4_MSSQL_CDC_PASSWORD` in the job's `.env`).
        self.password = os.environ["G4_MSSQL_CDC_PASSWORD"]

    def ping(self) -> None:
        _connect().close()

    def _run(self, statements: list[str], database: str = DATABASE) -> None:
        conn = _connect(database)
        try:
            cur = conn.cursor()
            for statement in statements:
                cur.execute(statement)
        finally:
            conn.close()

    def execute(self, sql: str) -> None:
        # One connection per statement: the change writer runs in its own thread.
        self._run([sql])

    def rows(self, sql: str) -> list[tuple[str, ...]]:
        conn = _connect(DATABASE)
        try:
            cur = conn.cursor()
            cur.execute(sql)
            return [tuple("" if v is None else str(v) for v in row) for row in cur.fetchall()]
        finally:
            conn.close()

    def qualified(self, table: str) -> str:
        return f"{SCHEMA}.{table}"

    def numbers(self, lo: int, hi: int) -> str:
        return f"SELECT g FROM {SCHEMA}.g4_numbers WHERE g BETWEEN {lo} AND {hi}"

    def dial(self) -> dict:
        return {
            "driver": "mssql", "host": HOST, "port": PORT, "database": DATABASE,
            "user": CDC_USER, "sslMode": "require",
        }

    def _agent_is_running(self) -> None:
        rows = self.rows("SELECT status_desc FROM sys.dm_server_services WHERE servicename LIKE 'SQL Server Agent%'")
        if not rows or not rows[0][0].lower().startswith("running"):
            raise cdc_gate.G4Failure(f"SQL Server Agent is not running (MSSQL_AGENT_ENABLED): {rows}")

    def setup(self) -> None:
        self._run(
            [
                f"IF DB_ID('{DATABASE}') IS NOT NULL BEGIN ALTER DATABASE {DATABASE} SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE {DATABASE}; END",
                f"CREATE DATABASE {DATABASE}",
            ],
            database="master",
        )
        conn = _connect("master")
        try:
            self._agent_is_running()
        finally:
            conn.close()
        self._run(["EXEC sys.sp_cdc_enable_db"])
        self._run([f"CREATE TABLE {SCHEMA}.g4_numbers (g INT PRIMARY KEY)"])
        self._run(
            [
                f"WITH n AS (SELECT TOP 110000 CAST(ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS INT) AS g "
                "FROM sys.all_objects a CROSS JOIN sys.all_objects b) "
                f"INSERT INTO {SCHEMA}.g4_numbers SELECT g FROM n"
            ]
        )
        for table in cdc_gate.TABLES.values():
            self._run(
                [
                    f"CREATE TABLE {SCHEMA}.{table} (id BIGINT PRIMARY KEY, customer NVARCHAR(100), "
                    "amount DECIMAL(12,2), status NVARCHAR(50), updated_at DATETIME2(6))",
                    f"INSERT INTO {SCHEMA}.{table} SELECT g, CONCAT('seed_', g), "
                    "CAST(((g % 1000) + 0.5) AS DECIMAL(12,2)), 'seed', SYSDATETIME() "
                    f"FROM {SCHEMA}.g4_numbers WHERE g BETWEEN 1 AND {cdc_gate.SEED_ROWS}",
                    f"EXEC sys.sp_cdc_enable_table @source_schema = '{SCHEMA}', @source_name = '{table}', "
                    "@role_name = NULL, @supports_net_changes = 0",
                ]
            )
        # The least-privilege login the connector dials (decision S3-e).
        pwd = self.password.replace("'", "''")
        self._run(
            [
                f"IF EXISTS (SELECT 1 FROM sys.server_principals WHERE name = '{CDC_USER}') DROP LOGIN {CDC_USER}",
                f"CREATE LOGIN {CDC_USER} WITH PASSWORD = '{pwd}', CHECK_POLICY = OFF",
            ],
            database="master",
        )
        self._run(
            [
                f"CREATE USER {CDC_USER} FOR LOGIN {CDC_USER}",
                *[f"GRANT SELECT ON {SCHEMA}.{table} TO {CDC_USER}" for table in cdc_gate.TABLES.values()],
                f"GRANT SELECT ON SCHEMA::cdc TO {CDC_USER}",
                f"GRANT VIEW DATABASE STATE TO {CDC_USER}",
            ]
        )

    def add_column_and_observe(self) -> None:
        """Debezium's online schema update: the column, a second capture instance that has it,
        then a change to a row (see the module doc)."""
        self._run(
            [
                f"ALTER TABLE {SCHEMA}.orders ADD note NVARCHAR(100) NULL",
                f"EXEC sys.sp_cdc_enable_table @source_schema = '{SCHEMA}', @source_name = 'orders', "
                "@role_name = NULL, @capture_instance = 'dbo_orders_v2', @supports_net_changes = 0",
            ]
        )
        self._run([f"UPDATE {SCHEMA}.orders SET note = 'schema_note' WHERE id = 2"])


def main() -> int:
    return cdc_gate.run(MsSqlSource())


if __name__ == "__main__":
    raise SystemExit(main())
