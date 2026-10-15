# `SRC-13` result: table discovery for every source type

Measured on pull request #144, commit `f2632d21`, CI run 38692942055, on
2026-10-15. One run on GitHub-hosted `ubuntu-latest` runners.

## Coverage (spec row "Discovery coverage")

Every connector type in the picker lists what exists at the source, and
each listing is checked by a gate with a known object and its columns:

| Type | Lists | Gate result |
| --- | --- | --- |
| PostgreSQL | schemas, then base tables | `ingest_demo.orders`, 4 columns |
| MySQL | schemas, then base tables | `g6_ingest.orders`, 2 columns |
| MariaDB | schemas, then base tables | `g6_ingest.orders`, 2 columns |
| SQL Server | schemas, then base tables | `dbo.orders`, 2 columns |
| Oracle | schemas the user can read, then tables, with keys | `G6_INGEST.ORDERS`, 2 columns, and that listed name then loads |
| PostgreSQL CDC | as PostgreSQL | gate `g4` |
| Object storage | files matching a glob, with size and date | `landing/orders.csv`, 3 columns |
| SFTP | files matching a glob | `orders.csv`, 3 columns |
| REST | the declared endpoints | `/items`, 2 columns (first page) |
| MongoDB | the database's collections | `orders`, 3 columns (sample) |
| Kafka | the broker's topics | `g6-events`, 2 columns (last messages, no offset committed) |

The API lists SQL sources, object storage and REST itself; MongoDB, Kafka,
SFTP, Oracle and the REST preview are listed by a short orchestrator job
(feature-page decision 1).

## Large sources (spec row, target 2 minutes for 10,000 tables)

Job "Rust · SRC-13 discovery of 10,000 tables", PostgreSQL:

| Step | Seconds |
| --- | --- |
| Create 10,000 tables (fixture, not product time) | 17.3 |
| List the schemas | 0.02 |
| List the 10,000 tables (names first) | 0.06 |
| Search for one table by part of its name | 0.03 |
| Read that table's columns and key | 0.02 |

The listing call's own limit is 120 seconds; the gate fails above it and if
the list is not exactly the 10,000 names. The console draws only the rows
in view and searches on the server above 500 names.

## Not measured

- 10,000 objects for the other types (only PostgreSQL is measured), and a
  listing through a slow network.
- The time of an orchestrator-run listing in production; in the gate stack
  each took a few seconds, mostly the job's start.
- The console in a browser.
- More than one run.
