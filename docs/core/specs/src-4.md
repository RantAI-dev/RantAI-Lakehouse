# `SRC-4` More databases, with live change capture

| | |
| --- | --- |
| Backlog | `SRC-4` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| When | Next |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Full and incremental loads are proven for PostgreSQL, MySQL, SQL Server and MongoDB; Oracle exists but has never been tested end to end. Live change capture is proven only for PostgreSQL; MySQL and SQL Server change capture exist but have never been gate-tested. All three competitors cover more.

## What users get

Load from the common operational databases, and keep each one in sync with live change capture.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Full and incremental loads | PostgreSQL, MySQL, SQL Server, MongoDB proven (gate g6); Oracle untested | Add Oracle (tested end to end), MariaDB and Teradata: Databricks' query-based list |
| Live change capture | PostgreSQL proven (gate g4); MySQL and SQL Server exist, never gate-tested | PostgreSQL, MySQL, SQL Server, Oracle and MongoDB, each with a passing gate (Snowflake Openflow's GA list plus MongoDB, as Airbyte) |
| Initial snapshot then changes | Not checked beyond PostgreSQL | Every change-capture source takes a consistent first copy, then streams changes without gaps or duplicates |
| Deletes and updates | Not checked beyond PostgreSQL | Applied in the table, with the load modes of SRC-10 (current row per key, or history) |
| Lag | Not measured | Lag per table shown on the connector page; an alert when it passes a limit set per connector, default 15 minutes *(proposed)* |
| Database setup help | Not checked | Per database, the exact source settings needed (log retention, replication user, permissions), checked by the connection test with a fixed message for each missing one |
| Each connector | n/a | Table and column choice (SRC-11), schema change handling (SRC-8), failure alerts (SRC-7), a gate test (SRC-9), credentials handled as SEC-14 requires |

## Benchmark

Databricks Lakeflow Connect: change capture for SQL Server (GA), MySQL and PostgreSQL (preview), Oracle (beta); query-based connectors for Oracle, Teradata, SQL Server, MySQL, MariaDB and PostgreSQL. Snowflake Openflow: change capture for PostgreSQL, MySQL, SQL Server and Oracle (GA), MongoDB (preview). Airbyte: certified change capture for PostgreSQL, MySQL, SQL Server and MongoDB.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SRC-4-AC1` For each database in the Target table, a table loads and a later insert, update and delete at the source show up in the lakehouse
- `SRC-4-AC2` Stopping change capture for an hour and restarting it catches up without gaps or duplicates
- `SRC-4-AC3` A source missing a required setting fails the connection test with a message naming that setting
- `SRC-4-AC4` Each database has a passing gate in CI
- `SRC-4-AC5` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Cloud warehouses as sources (Snowflake, BigQuery, Redshift): excluded by the product owner, 2026-10-07
- Enterprise databases (SAP HANA, Db2): excluded by the product owner, 2026-10-07
- Querying other systems in place without loading: QRY-1
- Business apps: SRC-3

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
