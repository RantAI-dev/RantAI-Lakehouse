# Table discovery for every source type

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-13` |
| Kind | Feature |
| Spec | `docs/core/specs/src-13.md`. This page follows it row by row; anything the spec does not say is under "Beyond the spec". One *(proposed)* number, decision 9 |
| Status | Planned. Decisions not signed |
| Plan | Not written; it waits for the signatures below |

## Problem

From the spec: discovery covers SQL and CDC sources only. Checked in the
code on 2026-10-14 (`main` at `112cd8b` with `SRC-10`), row by row:

- **Discovery coverage.** "Find tables" works for PostgreSQL, MySQL, MariaDB,
  SQL Server and PostgreSQL CDC, one schema at a time, and the schema name
  must be typed. Every other type is refused:
  - Object storage and REST: "not implemented in this build".
  - MongoDB, Kafka and SFTP: refused with the wrong text, `unknown adapter
    "mongodb"` (or `"kafka"`, `"sftp"`). The gates check only that a refusal
    comes back, not its words.
  - Oracle: the refusal says "Oracle tables are listed by the orchestrator".
    Nothing lists Oracle tables, and the console gives an Oracle connector
    no field to type a table name either, so a table can be added only
    through the API.
  - For the refused types the user types each object's name. A Kafka
    connector reads the one topic typed into its form. A REST connector
    loads every declared endpoint into every table it is given.
- **Column preview.** None for any type. SQL discovery fetches column names
  and types, and the console shows only a count ("12 cols"). Files, REST,
  MongoDB, Kafka and SFTP learn their columns only after a load.
- **Large sources.** Not measured. Discovery reads every column of every
  table in the schema in one go, inside a 5-second limit, and the console
  draws every row. A schema of 10,000 tables would most likely time out
  (inferred from the code, not run).

Where the work can run: the API has drivers for PostgreSQL, MySQL, MariaDB,
SQL Server and S3-compatible storage. The orchestrator, which runs every
load, also has MongoDB, Kafka, SFTP and Oracle clients.

## What the user can do when this is done

| Spec row | Spec target | What they can do |
| --- | --- | --- |
| Discovery coverage | Every connector type: files and buckets (by glob), REST (declared endpoints), MongoDB (collections), Kafka (topics), SFTP (paths), Oracle (schemas and tables) | 1. For every connector type in the picker, press "Find tables" and pick from what exists at the source, instead of typing names. 2. For object storage and SFTP, narrow the list with a glob such as `sales/2026-*.csv`. 3. For a SQL source, pick the schema from a list. |
| Column preview | Inferred columns and types shown before choosing | 4. Open any listed table and see its columns and their types before ticking it. |
| Large sources | Discovery of 10,000 tables completes within 2 minutes and is searchable *(proposed)* | 5. List a source of 10,000 tables within 2 minutes and find one by typing part of its name. |

## Not included

From the spec: anything not in the Target table. In particular:

- Loading many files matched by one glob into one table. A listed file is
  picked as its own table, as today.
- More than one topic per Kafka connector.
- MySQL CDC and SQL Server CDC: not supported since `SRC-9` (`SRC-4`).
- Google Sheets: not supported since `SRC-6`.
- A connection test for MongoDB, Kafka, SFTP and Oracle. Discovery shows
  whether the orchestrator can reach the source, but "Test" stays as it is.

## Asking the assistant

From the spec: not part of the dashboards assistant (`AI-16`).

## Decisions

Not signed. Rows 1 to 6 and 9 settle how a spec row is met; the others are
beyond the spec. The planner's recommendation is the Default.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Spec "every connector type": where the listing runs for the types the API cannot reach. | Object storage and REST are listed by the API, which already reaches them for the connection test. MongoDB, Kafka, SFTP and Oracle are listed by the orchestrator, which already holds their drivers and credentials for loads: "Find tables" starts a short listing job and the page shows its progress, then the result. No new database driver goes into the API. | |
| 2 | Spec "files and buckets (by glob)". | The user types a glob (default: everything under the connector's prefix); the list shows each matching file with its size and last change. Each ticked file becomes a table, as a typed name does today. Up to 10,000 files per listing; more is cut with a message saying so. | |
| 3 | Spec "REST (declared endpoints)". | The list is the endpoints declared on the connector. Picking an endpoint loads that endpoint only. Today every table of a REST connector loads every endpoint; that is fixed here, because a list to pick from means nothing otherwise. | |
| 4 | Spec "Kafka (topics)". | The list is the broker's topics, internal ones hidden. Picking one sets the connector's topic; one topic per connector, as today. | |
| 5 | Spec "MongoDB (collections)", "SFTP (paths)", "Oracle (schemas and tables)". | MongoDB: the collections of the connector's database. SFTP: files under the connector's path, narrowed by a glob as in decision 2. Oracle: the schemas the user can read, then that schema's tables with columns and primary keys, as for the other SQL sources. Oracle also gets the "add by name" field the other types have. | |
| 6 | Spec "inferred columns and types shown before choosing". | SQL sources and Oracle: the columns and types the source declares. Files: from the header and the first 1,000 rows (CSV), or the file's own schema (Parquet). MongoDB: from a sample of 100 documents. Kafka: from the last 100 messages, read without moving the connector's position. REST: from the first page of the endpoint. An inferred type is marked "inferred from a sample" and is what the first load will create. | |
| 7 | Beyond the spec: the schema of a SQL source. | Listed for every SQL type; the user picks it instead of typing it. Typing stays possible. | |
| 8 | Beyond the spec: views. | Base tables only, as the loader lists them today. Today's API discovery also returns views, which may not load; that is corrected here. | |
| 9 | Spec *(proposed)*: 10,000 tables within 2 minutes, searchable. | Accept. The list arrives with names first and each table's columns only when it is opened, so a large schema does not wait on every column. The console shows a search box and draws only the rows in view. Measured by a gate on a PostgreSQL schema of 10,000 tables and written into `docs/plans/SRC-13-RESULT.md`. | |
| 10 | Beyond the spec: errors. | Every refusal and failure has its own fixed text: unreachable, sign-in refused, no permission at the source, timed out, too many objects. Never the driver's text (principle 4). The wrong "unknown adapter" text goes. | |
| 11 | Beyond the spec: who may list. | A user with `connector:manage` on a connector of their own tenant, as today. | |
| 12 | Beyond the spec: gates. | The "discover" step of each type's gate changes from "refused" to a real listing with a known object and its columns, and the coverage table follows. The 10,000-table measurement runs in a job of its own, so no existing job grows. | |

## Limits to tell a customer

- Columns of files, MongoDB, Kafka and REST are inferred from a sample; a
  column that appears only later in the data is found by the first load
  (`SRC-8`).
- For MongoDB, Kafka, SFTP and Oracle the listing runs as a short job and
  takes some seconds longer than for the other sources.
- A file listing stops at 10,000 files; a narrower glob finds the rest.

## Acceptance checklist

Run on a real deployment by the product owner. Mark each Pass, Fail, or Not
run with the reason. Rows 1 and 2 are the spec's checklist, in its order.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Spec `SRC-13-AC1`: for each connector type in the picker, press "Find tables" | The picker lists what exists at the source | |
| 2 | Spec `SRC-13-AC2`: as a user without `connector:manage`, list tables; and list from a source that is down | Refused; the failure shows an honest message with no source error text | |
| 3 | Open a listed table of each type | Its columns and types, marked "inferred from a sample" where they are | |
| 4 | List a bucket with a glob that matches some files | Only those files, with size and date | |
| 5 | Pick one endpoint of a REST connector with two, and run | Only that endpoint is loaded | |
| 6 | Pick a Kafka topic | The connector reads that topic at its next run | |
| 7 | List a PostgreSQL schema of 10,000 tables and search for one | The list arrives within 2 minutes; the search finds it | |
| 8 | Add an Oracle table by name | Saved, and loaded at the next run | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Released; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
