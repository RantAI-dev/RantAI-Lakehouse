# More databases, with live change capture

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-4` |
| Kind | Feature |
| Spec | `docs/core/specs/src-4.md`. This page follows it row by row; three of its "Today" cells are out of date and are corrected under Problem. Its one *(proposed)* number, the 15-minute lag alert, is decision 7 |
| Status | Planned. Decisions below are the planner's defaults, awaiting the product owner's signature. Nothing blocks it any more: `SEC-14` part B merged in #192 on 2026-10-28 (decision 10) |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Plan | Written after the decisions are signed: `docs/superpowers/plans/2026-10-27-src-4-databases-cdc.md` |
| Started | 2026-10-27 (planning) |
| Shipped | Not yet |
| Evidence | The pull request, with the first run of each new gate |

## Problem

Read in the code on 2026-10-27 (`main` at `e497b118`). The spec was written
at `f3a3196` and three of its "Today" cells have moved since:

- **Oracle batch loads are gate-tested** (`g6-oracle`, `ops/g6/g6_oracle_test.py`:
  discover, load, reload). What Oracle still lacks: a connection test in the
  console (`connector_probe.rs` answers "not testable here"), column choice
  (`supports_column_choice` leaves Oracle out), an incremental load tested
  end to end (the gate reloads with `replace` only), and an encrypted dial
  (the gate dials in plain text).
- **MariaDB is supported and gate-tested** in `g6-ingest`, dialled with the
  MySQL driver. Nothing to add beyond what MySQL gets.
- **Teradata does not exist**: no type, driver or code.

Live change capture:

- **One change-capture pipeline exists, and it is fixed in the deployment.**
  A single Debezium Server (`docker-compose.yml`, service `debezium-server`)
  reads one hand-written PostgreSQL table (`p5_cdc.orders`) and writes it to
  Iceberg. A user who creates a "PostgreSQL CDC" connector in the console
  gets a row in the registry and a rendered settings file, but nothing
  starts reading their database: ADR 0007 chose "no dynamic per-connector
  provisioning". Debezium Server runs one source per process.
- **MySQL CDC and SQL Server CDC are listed as not supported** since
  `SRC-9` decision 8 (migration `0066`): their settings could not run.
  Oracle CDC is refused by the API. MongoDB CDC has no type; the MongoDB
  test database is a single server, and change streams need a replica set.
- **Deletes are kept as rows marked `__deleted`**, and the `SRC-10` load
  modes (current row per key, or history) do not apply to change capture.
- **No lag in time is measured.** A job measures the PostgreSQL slot's
  retained bytes, for the deployment's own database only; `health.rs` says
  plainly that bytes cannot be turned into seconds honestly.
- **The connection test only runs `SELECT 1`.** No setting a change-capture
  source needs (log level, replication user, retention) is checked.

Checked outside the repo on 2026-10-27:

- **Teradata has no container image.** Its free test database, Vantage
  Express, is a virtual machine behind a sign-in; the other free option is
  Teradata's hosted trial (ClearScape), an outside service. A GitHub runner
  can run neither as an ordinary service container.
- **Debezium's Oracle connector does not ship Oracle's JDBC driver**
  (licence); the image has to add it. A reported issue (DBZ-9681) has the
  connector failing to read the version of the `gvenzl/oracle-free` image
  our gate uses; to be checked in the first Oracle slice.
- **Debezium's MongoDB connector needs a replica set** (change streams).

## What the user can do when this is done

| Spec row | Spec target | What they can do |
| --- | --- | --- |
| Full and incremental loads | Oracle tested end to end, MariaDB, Teradata | Test an Oracle connection in the console, choose its columns, load it incrementally over an encrypted connection. MariaDB as today. Teradata: decision 2 |
| Live change capture | PostgreSQL, MySQL, SQL Server, Oracle, MongoDB, each gated | Create a change-capture connector for any of the five in the console and have it start reading, with no step by an operator (decision 1) |
| Initial snapshot then changes | A consistent first copy, then changes without gaps or duplicates | The first run copies each chosen table, then follows the source's log |
| Deletes and updates | Applied, with the `SRC-10` load modes | Choose per table: current row per key (a delete removes the row) or history (every change kept) (decision 4) |
| Lag | Per table on the connector page, alert past a limit, default 15 min | See each table's lag in seconds; get an alert past the limit (decisions 6, 7) |
| Database setup help | Each required source setting checked, a fixed message per missing one | The connection test names each missing setting and how to set it (decision 9) |
| Each connector | `SRC-11`, `SRC-8`, `SRC-7`, `SRC-9`, `SEC-14` | Column choice, schema change handling, failure alerts, a gate, credentials as `SEC-14` requires |

## Not included

- Cloud warehouses and enterprise databases (product owner, 2026-10-07).
- Querying a source in place (`QRY-1`); business apps (`SRC-3`).
- Change capture for MariaDB: not in the spec's target list. Its binlog is
  close to MySQL's, so it is a follow-up, not a promise (decision 3).
- Moving an existing batch connector to change capture in place: a
  change-capture connector is created as its own connector.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed
to the AI team in `AI-16`.

## Decisions

| # | Decision | Default *(planner's proposal)* | Signed |
| --- | --- | --- | --- |
| 1 | How a change-capture connector runs | **Kafka Connect, started by the API.** One Kafka Connect service in the deployment holds every change-capture connector; the API adds, pauses and removes a connector through Kafka Connect's own API when the user creates, pauses or deletes it. Changes go to Kafka, and a Dagster job applies them to Bronze through the same writer batch loads use. This replaces ADR 0007's fixed single connector (a new ADR, decision 11). Rejected: one Debezium Server per connector started by an operator (not self-serve, unlike every competitor); a service that starts containers (needs the Docker socket, a host takeover risk) | |
| 2 | Teradata | **Not built in `SRC-4`; a backlog item of its own.** There is no Teradata database a CI runner can start, and a supported connector needs a gate (`SRC-9`). Building it untested would be "works, approximately". The new item names what unblocks it: a test database the gate can reach (Teradata's hosted trial with a CI secret, or a self-hosted runner with Vantage Express). This is a spec target not met, reported here rather than lowered quietly | |
| 3 | Which change-capture sources | The spec's five: PostgreSQL, MySQL, SQL Server, Oracle (LogMiner), MongoDB (change streams, replica set). MariaDB change capture is a follow-up backlog item | |
| 4 | How changes land in a table | The `SRC-10` load modes, chosen per table: **current row per key** (default: an update replaces the row, a delete removes it) or **history** (every change kept, with its operation and source time). The `__deleted` marker of today's pipeline goes away. A table with no primary key can only use history | |
| 5 | How fresh | Changes are applied in small batches; target: a change visible in Bronze **within 2 minutes** at the source's normal rate *(proposed; the spec has no number; today's single pipeline does it within 20 s, but it writes straight to Iceberg and skips the load modes)*. Measured in the first slice and written into a result file | |
| 6 | What lag means | Per table: now minus the source commit time of the last change applied to Bronze, in seconds; while a table has no new changes, its lag is "up to date" rather than a growing number (Kafka's backlog for that table is checked) | |
| 7 | Lag alert | Per connector, default **15 minutes** (the spec's proposal), changeable on the connector; sent through the `SRC-7` alert channels as a new kind `connector_lag`, once when crossed and once when back under | |
| 8 | Oracle batch | Add a connection test, column choice, incremental loads and an encrypted dial (native network encryption or TLS, as the source offers), each in the `g6-oracle` gate. The Oracle JDBC driver is fetched into the image at build time, never committed | |
| 9 | Setup checks in the connection test | Per database, fixed sentences: PostgreSQL `wal_level=logical`, replication permission, publication; MySQL `binlog_format=ROW`, `binlog_row_image=FULL`, replication grants, binlog retention; SQL Server CDC enabled on the database and the table, SQL Server Agent running; Oracle `ARCHIVELOG`, supplemental logging, LogMiner grants; MongoDB replica set and change-stream permission. A failed check names the setting and the command that sets it; the connector can still be saved | |
| 10 | Building on `SEC-14` part B (#192, merged 2026-10-28) | The connection test and the setup checks build on what #192 left: PostgreSQL is tested by the console's own minimal client (`pg_probe.rs`: TLS required, `SCRAM-SHA-256` only, one fixed `SELECT 1`), so its setup checks extend that client with a few more fixed statements rather than going back to `sqlx`; SQL Server keeps encryption required; MySQL keeps `Preferred` (#192's D9). The Oracle connection test needs an Oracle client in the API, or runs in the orchestrator as discovery does today: decided in the plan | |
| 11 | ADR | A new ADR replaces ADR 0007's "no per-connector provisioning" with decision 1, written before the first code slice | |
| 12 | Gates | One CI job per change-capture source (each needs its own database and Kafka Connect, and the slowest connector job must stay under 20 minutes, `SRC-9` decision 1). Today's `g4-cdc` moves to the new runtime. Each gate proves: first copy, insert, update, delete, a one-hour-equivalent stop and catch-up without gaps or duplicates, lag shown, setup check failing on a missing setting | |

## Limits to tell a customer

To be filled from the signed decisions and the measurements (decision 5).

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `SRC-4-AC1` | For each of the five databases, create a change-capture connector, choose a table, then insert, update and delete a row at the source | The first copy appears, then each change, within the decision 5 target | Not run |
| `SRC-4-AC2` | Pause one connector for an hour while changing rows, then resume it | It catches up with no row missing and none twice | Not run |
| `SRC-4-AC3` | Test a connection to a source missing a required setting | The test names that setting and how to set it | Not run |
| `SRC-4-AC4` | Look at the CI run of the pull request | Each database has a passing gate | Not run |
| `SRC-4-AC5` | As a user without `connector:manage`; and break a source while it runs | Refused; the failure shows a fixed, honest message | Not run |
| `SRC-4-AC6` | Load an Oracle table incrementally with chosen columns, over an encrypted connection | Only new rows and the chosen columns arrive | Not run |
| `SRC-4-AC7` | Stop a source long enough to pass the lag limit | The lag shows on the connector page and one alert arrives | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added (Teradata, MariaDB change capture)
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo
- [ ] `CHANGELOG.md` entry a customer can read
