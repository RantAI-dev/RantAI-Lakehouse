# More databases, with live change capture

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-4` |
| Kind | Feature |
| Spec | `docs/core/specs/src-4.md`. This page follows it row by row; three of its "Today" cells are out of date and are corrected under Problem. Its one *(proposed)* number, the 15-minute lag alert, is decision 7; decision 5's 2-minute target is the planner's, signed and measured in slice 0 |
| Status | Building (#264). All thirteen decisions signed by the product owner on 2026-10-29, as the planner recommended ("saya setuju, jalankan sesuai dengan rekomendasi anda"). Nothing blocks it: `SEC-14` part B merged in #192 on 2026-10-28 (decision 10) |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Plan | `docs/superpowers/plans/2026-10-29-src-4-databases-cdc.md`; ADR `docs/adr/0016-change-capture-per-connector-on-kafka-connect.md` |
| Started | 2026-10-27 (planning), 2026-10-29 (decisions signed); also in `BACKLOG.md` Dates |
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

- **PostgreSQL change capture runs per connector** (`SRC-4` slice 2, ADR 0016).
  Saving a "PostgreSQL CDC" connector's ingest spec creates a Debezium connector
  in one shared Kafka Connect worker; a Dagster job (`cdc_apply_job`, started by
  a 30-second sensor) writes its topics to Bronze. It needs the `cdc` profile and
  `KAFKA_CONNECT_URL` and `CDC_KAFKA_BOOTSTRAP` to be set; without them the
  connector answers "not supported on this deployment". The two services cost
  about 1.2 GiB. Freshness measured in slice 0: p95 18 to 40 s, 108,000 changes
  caught up in 60 to 105 s (`docs/plans/SRC-4-RESULT.md`).
- **Load modes for change capture are two**: "Current row per key" (the default)
  and "Keep history"; both need a primary key. A deleted row is removed in the
  first and kept as an event (`_op` = `d`) in the second.
- **Topics are not removed when a connector is deleted**; the broker's 7-day
  retention removes them. A change older than 7 days that the applier has not
  read is lost.
- **`date` and `timestamp` (without time zone) columns arrive as integers**
  (Debezium's temporal types) and their Bronze column is an integer.
- **MySQL and SQL Server change capture run the same way** (`SRC-4` slice 3):
  the same worker, the same applier, the same two load modes, and the types
  "MySQL CDC" and "SQL Server CDC" are supported again (migration `0121`). Each
  has its own gate in CI (`g4-mysql`, `g4-mssql`), which creates the connector
  through the API and reads the result back. What each source needs, all proven
  by the gates with a user that is not `root` or `sa`:
  - **MySQL**: `log_bin` on, `binlog_format=ROW`, `binlog_row_image=FULL` (MySQL
    8's defaults; the gate asserts them). The user needs `SELECT` on the
    database, and the global `RELOAD`, `SHOW DATABASES`, `REPLICATION SLAVE` and
    `REPLICATION CLIENT`. The connector needs a `serverId` (a number no other
    replica of that server uses, 1 to 4294967295), which the console asks for and
    never fills in.
  - **SQL Server**: SQL Server Agent running; CDC enabled on the database
    (`sys.sp_cdc_enable_db`) and on each table (`sys.sp_cdc_enable_table`) by an
    administrator; a login that is not `sa` with `SELECT` on the tables and on the
    `cdc` schema, and `VIEW DATABASE STATE`. Debezium also calls
    `sys.sp_cdc_help_jobs` to read the capture job's polling interval; that
    procedure needs `db_owner` (Microsoft's documentation), so with the grants
    above Debezium logs a warning and goes on, and the gate passes
    (`docs/superpowers/plans/2026-10-29-src-4-databases-cdc.md`, Slice 3 handoff).
  - **A new column in SQL Server is not picked up by itself.** A capture instance
    has the columns the table had when it was enabled. To add a column: add it to
    the table; enable a second capture instance on the table
    (`sys.sp_cdc_enable_table @capture_instance = '<schema>_<table>_v2', ...`);
    change a row; once the column has reached Bronze, drop the old instance
    (`sys.sp_cdc_disable_table @capture_instance = '<old name>'`). A table may
    have two instances at a time. The gate runs the first three steps.
  - **TLS to the source.** `require` encrypts and takes the certificate as
    presented. `verify-ca` and `verify-full` use Kafka Connect's own trust store,
    so the source's CA must be in the worker's image or mounted into it; the
    gates run `require`, and the renderer's unit tests cover the other modes. For
    SQL Server `verify-ca` is stricter than its name, never looser: the driver
    cannot check the chain without the host name.
  - **The schema-history topic is kept.** `MySQL` and `SQL Server` connectors keep
    their table definitions in a Kafka topic, `cdc.<connector id>.schema-history`,
    which needs the broker address (`CDC_KAFKA_BOOTSTRAP` on the API as well as
    on the code location; unset, the connector answers "not supported on this
    deployment"). Debezium creates it with retention `Long.MAX_VALUE` (measured in
    CI), so the broker's 7-day retention does not apply to it and a connector can
    restart after any time. It is never deleted with the connector (Kafka
    Connect has no topic API); it is a few kilobytes.
- **Oracle and MongoDB capture are added by their own slices.** (Written before slices 5 and 6: MongoDB CDC
  and Oracle CDC are types now, see their sections below.)
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
  (licence); the image has to add it. *Corrected 2026-10-30 (slice 6):* the pinned image already
  holds it. A reported issue (DBZ-9681) has the connector failing to read the version of the
  `gvenzl/oracle-free` image our gate uses; it is fixed in Debezium 3.2.7, 3.3.3 and 3.4.0 and the
  image holds 3.7.0.Final, and the `g4-oracle` gate is what proves it.
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

| # | Decision | Decided | Signed |
| --- | --- | --- | --- |
| 1 | How a change-capture connector runs | **Kafka Connect, started by the API.** One Kafka Connect service in the deployment holds every change-capture connector; the API adds, pauses and removes a connector through Kafka Connect's own API when the user creates, pauses or deletes it. Changes go to Kafka, and a Dagster job applies them to Bronze through the same writer batch loads use. This replaces ADR 0007's fixed single connector (a new ADR, decision 11). Rejected: one Debezium Server per connector started by an operator (not self-serve, unlike every competitor); a service that starts containers (needs the Docker socket, a host takeover risk) | 2026-10-29 |
| 2 | Teradata | **Not built in `SRC-4`; a backlog item of its own.** There is no Teradata database a CI runner can start, and a supported connector needs a gate (`SRC-9`). Building it untested would be "works, approximately". The new item names what unblocks it: a test database the gate can reach (Teradata's hosted trial with a CI secret, or a self-hosted runner with Vantage Express). This is a spec target not met, reported here rather than lowered quietly | 2026-10-29 |
| 3 | Which change-capture sources | The spec's five: PostgreSQL, MySQL, SQL Server, Oracle (LogMiner), MongoDB (change streams, replica set). MariaDB change capture is a follow-up backlog item | 2026-10-29 |
| 4 | How changes land in a table | The `SRC-10` load modes, chosen per table: **current row per key** (default: an update replaces the row, a delete removes it) or **history** (every change kept, with its operation and source time). The `__deleted` marker of today's pipeline goes away. A table with no primary key can only use history | 2026-10-29 |
| 5 | How fresh | Changes are applied in small batches; target: a change visible in Bronze **within 2 minutes** at the source's normal rate *(proposed; the spec has no number; today's single pipeline does it within 20 s, but it writes straight to Iceberg and skips the load modes)*. Measured in the first slice and written into a result file | 2026-10-29 |
| 6 | What lag means | Per table: now minus the source commit time of the last change applied to Bronze, in seconds; while a table has no new changes, its lag is "up to date" rather than a growing number (Kafka's backlog for that table is checked) | 2026-10-29 |
| 7 | Lag alert | Per connector, default **15 minutes** (the spec's proposal), changeable on the connector; sent through the `SRC-7` alert channels as a new kind `connector_lag`, once when crossed and once when back under | 2026-10-29 |
| 8 | Oracle batch | Add a connection test, column choice, incremental loads and an encrypted dial (native network encryption or TLS, as the source offers), each in the `g6-oracle` gate. Correction 2026-10-31 (slice 6): the Oracle JDBC driver already ships in the pinned Kafka Connect image (`ojdbc11` with `orai18n`, shipped with the connector since DBZ-7364), so nothing is fetched at build time and nothing is committed | 2026-10-29 |
| 9 | Setup checks in the connection test | Per database, fixed sentences: PostgreSQL `wal_level=logical`, replication permission, publication; MySQL `binlog_format=ROW`, `binlog_row_image=FULL`, replication grants, binlog retention; SQL Server CDC enabled on the database and the table, SQL Server Agent running; Oracle `ARCHIVELOG`, supplemental logging, all-column logging on each selected table, LogMiner grants (read over an unencrypted, dictionary-only session: the exception signed on 2026-10-31, see Limits); MongoDB replica set and change-stream permission. A failed check names the setting and the command that sets it; the connector can still be saved | 2026-10-29 |
| 10 | Building on `SEC-14` part B (#192, merged 2026-10-28) | The connection test and the setup checks build on what #192 left: PostgreSQL is tested by the console's own minimal client (`pg_probe.rs`: TLS required, `SCRAM-SHA-256` only, one fixed `SELECT 1`), so its setup checks extend that client with a few more fixed statements rather than going back to `sqlx`; SQL Server keeps encryption required; MySQL keeps `Preferred` (#192's D9). The Oracle connection test needs an Oracle client in the API, or runs in the orchestrator as discovery does today: decided in the plan | 2026-10-29 |
| 11 | ADR | A new ADR replaces ADR 0007's "no per-connector provisioning" with decision 1, written before the first code slice | 2026-10-29 |
| 12 | Gates | One CI job per change-capture source (each needs its own database and Kafka Connect, and the slowest connector job must stay under 20 minutes, `SRC-9` decision 1). Today's `g4-cdc` moves to the new runtime. Each gate proves: first copy, insert, update, delete, a one-hour-equivalent stop and catch-up without gaps or duplicates, lag shown, setup check failing on a missing setting | 2026-10-29 |
| 13 | Order of the work | A measurement first (slice 0): Kafka Connect and the applier on PostgreSQL alone, on a branch never merged, measuring decision 5's target, the load modes on updates and deletes, and an hour's stop and catch-up; written into `docs/plans/SRC-4-RESULT.md`. If it misses, decision 1 is revisited before any product code. Then: Oracle batch (decision 8); the change-capture runtime with PostgreSQL moved onto it and the fixed pipeline removed; MySQL, then SQL Server; lag, the alert and the setup checks; MongoDB; Oracle change capture last (the riskiest: the driver Debezium does not ship and DBZ-9681). Oracle change capture can be split out without holding the other four | 2026-10-29 |

## Limits to tell a customer

To be filled from the signed decisions and the measurements (decision 5). Slice 4 (lag,
the lag alert, the setup checks) adds these; each is a statement the product makes, not a
promise it keeps better than it says:

- **What lag means.** Per table, the time since the later of two moments: the source commit
  time of the last change applied to Bronze, and the last moment the table was known to be up
  to date. Taking the later of the two keeps a table that was quiet for a day, then got one
  change, from reading a day behind. The number is never larger than the true lag. A table is
  "up to date" only when nothing waits on its topic **and** Kafka Connect says the connector
  runs: an empty topic proves nothing while Kafka Connect is stopped. Just after Kafka Connect
  starts again, a table can read "up to date" for a moment before its first events reach the
  topic.
- **If the orchestrator is down, no alert is sent.** The lag comes from two reports of the
  orchestrator (the backlog every 30 seconds, the last applied change after each batch). When
  they stop, the connector page shows "Not known since <time>" after two minutes, and no
  `connector_lag` alert can fire, because nothing evaluates. Watch the orchestrator itself.
- **The alert is sent once when the lag passes the limit and once when it is back under.** The
  Alerts list keeps one row per rule within 15 minutes, so the second alert is delivered but
  may not appear as a second row if both happen within 15 minutes. A connector a person paused
  is not evaluated and shows "Paused", not a number.
- **A pause is not lag.** After a resume the lag counts from the resume; changes made during the
  pause show as backlog until they are applied. A source change time in the future is stored as
  now. Kafka Connect not answering counts as "not running": a table is then never "up to date".
- **Rare ordering.** If delivering an alert fails and the flag cannot be put back, that alert is
  lost; a concurrent post that closes an alert whose delivery then fails can leave the two out of
  order. Both need two faults at once.
- **Setup checks never block saving.** They are read after a connection test passes, with
  fixed statements; a setting the login cannot read is "Not checked", never a pass. MySQL's
  binlog retention must be at least **7 days** (or never expire), the broker's own retention:
  the window in which a stopped connector can still catch up (the number is the planner's). The check also reads the legacy `expire_logs_days`: logs never
  expire only when it and `binlog_expire_logs_seconds` are both 0.
  SQL Server Agent can only be read with `VIEW SERVER STATE`, which the documented login does
  not have, so that check always reads "Not checked" with that reason; the login is not
  widened. A SQL Server table the login cannot see is reported as missing CDC.
- **Not in this slice.** MongoDB's setup check comes with MongoDB (slice 5); Oracle's with its
  change capture (slice 6, below).

### MongoDB change capture (slice 5)

What `MongoDB CDC` needs and what it does not do. Proven by the `g4-mongo` gate unless a line says otherwise.

- **A replica set is required.** Change streams exist only on a replica set; a standalone server has none, and
  the connector cannot start against one. The replica set's name is a required field of the connector
  ("the name in the server's replica set configuration"): there is no default.
- **Sharded clusters are out of scope.** The connector is not built or tested against a sharded cluster.
- **Atlas and `mongodb+srv` are not offered.** The connector takes one explicit `host:port` and no connection
  string, as the batch `MongoDB` connector takes explicit seeds.
- **The user.** The connector dials as a user with the built-in `read` role on the one database, nothing more.
  The gate proves that role is enough: the connector captures only that database (`capture.scope=database`);
  Debezium's default scope, the whole deployment, would need the right to read every database. `root` is not
  needed. Debezium's documentation names the `hello` command, which any user may run.
- **One host.** The connector takes exactly one `host:port`, which must be a member of the replica set that
  change streams can be read from, and the connection string always carries `directConnection=true`, so the
  worker dials only that host and never the member addresses a replica set reports (the API checks only the
  written host against the installation's address rules). A dial with another number of hosts is refused. The
  connection test and the list of collections dial the same host. What happens when that host stops being
  primary is *not verified*: the gate does not stop it.
- **TLS.** `disable`, `require`, `verify-ca` and `verify-full` are all rendered and unit-tested; the gate runs
  `disable` (its fixture has no certificate). Debezium has no way to take a MongoDB certificate "as
  presented": in every TLS mode the chain is checked against the worker's trust store, so the source's CA must
  be in it; `require` differs from `verify-full` only in not checking the host name. The console form does not
  yet offer a TLS choice for any change-capture connector, so a connector made there takes `require`.
- **Documents become rows as the batch connector makes them.** `_id` is the key; an `ObjectId` is stored as its
  24-digit hex text. A nested document or an array in a document **stops the table's load** with a fixed message
  (the batch connector refuses it too): flatten such fields at the source or leave them out.
- **A delete carries the key only.** MongoDB keeps no old document, so in "History" mode a delete row has `_id`
  and empty values; in "Current row per key" mode the row is removed.
- **A new field in a document is a schema change** and goes through the same decision as a new column in a
  relational table (`SRC-8`).
- **Setup is checked by the connection test**, in fixed sentences (`SRC-4` Amendment 8): the replica set's name
  against the connector's, that the host is the primary, and that the login may read change streams on the
  database (with the command that grants the `read` role). The orchestrator reads three facts from the server
  after the test; one it cannot read is shown as "Not checked", never as passing. The checks never change
  whether the test passed.

### Oracle change capture (slice 6)

What `Oracle CDC` needs and what it does not do. Proven by the `g4-oracle` gate unless a line says otherwise.

- **Native network encryption, not TLS.** Oracle's JDBC driver does TLS only over TCPS with a Java trust
  store, and the fixture could not bring Oracle Free's own TCPS listener up (`ORA-28759`, slice 1). So
  "Require" means Oracle's native network encryption is *required* (AES256 with a SHA256 checksum): the
  session is encrypted, the server is **not** authenticated. "Disable" sends none of it. "Verify CA" and
  "Verify CA and host name" are refused (`SRC-37`, certificate checking over TCPS, is on the backlog). The
  gate reads the server's own session banner to prove the encryption is on.
- **The connection test goes through Kafka Connect.** `python-oracledb` thin mode (2.5.1, and 26.1.0) has no
  native network encryption, so a test from the orchestrator would not use the encryption the connector
  uses. The API asks Kafka Connect to validate the connector's configuration with the connector's own driver
  (`SRC-4` Amendment 10). Debezium 3.7.0 answers "Unable to connect (primary): Failed
  to resolve Oracle database version" for a refused login, an unreachable server or service and a refused
  encryption alike (the Oracle code is only in the cause, which the answer does not carry), so the test says
  one fixed sentence for all of them: the connection or the login was refused, check the host, the port, the
  service name or PDB name, the login and its password, and that the server allows Oracle native network
  encryption (ask the database administrator if it does not). A validation timeout has its own sentence.
- **Setup checks are read over unencrypted connections (owner-signed exception to `SEC-14`, 2026-10-31).**
  `SEC-14` says a stored credential is not dialled in plain text unless the person chose "Disable". After the
  Kafka Connect test passes, the orchestrator reads Oracle's settings with thin mode, **not encrypted**:
  `ARCHIVELOG`, minimal and all-column supplemental logging, whether each selected table exists and has
  all-column logging (read in a second session on the pluggable database's own service, because a common user sees only
  the root in the container-wide views unless `CONTAINER_DATA` is widened), and the session's privileges and roles. Fixed statements over the data dictionary only,
  never a table row; every value is compared in Python and none is put in a statement. The password is not sent
  in clear text (Oracle's own logon protocol), but the session is not encrypted, and the test result says so
  in a fixed line. A server that refuses an unencrypted session (`ORA-12660`) makes every check "Not checked"
  with "The server requires encryption, which the setup check cannot use: check these settings by hand."
- **Tables are typed by hand.** Listing them would need the same unencrypted session, so an Oracle
  change-capture connector lists nothing: type each table as `SCHEMA.TABLE` in Oracle's own spelling (upper
  case; the console upper-cases what you type, the API refuses anything else). The test tells a name that does
  not exist ("not found"), and a table that exists without all-column logging, with the command.
- **ARCHIVELOG needs a restart.** There is no one-line command for it; the check names the steps. Archived logs
  must be kept for as long as a stopped connector may need to catch up: if they are deleted first, the connector
  cannot resume where it stopped.
- **All-column logging.** Without `ALTER TABLE ... ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS` Debezium 3.7.0
  only logs a warning and keeps running: an update then reaches Bronze with its unchanged columns missing, and a
  delete carries the key only. A *failed* connector can say why only for the two settings Debezium throws for
  (`ARCHIVELOG` off, minimal supplemental logging off); the table case is visible in the test, not in a failure.
- **The documented grants.** The connector's login is a common user (`c##...`) with Debezium's documented
  LogMiner grants, no `DBA`: `CREATE SESSION`, `SET CONTAINER`, `LOGMINING`, `SELECT ANY TRANSACTION`,
  `FLASHBACK ANY TABLE` (or `FLASHBACK` on each table), `SELECT ANY TABLE` (or `SELECT` on each table),
  `EXECUTE_CATALOG_ROLE`, `SELECT_CATALOG_ROLE`, `CREATE TABLE`, `LOCK ANY TABLE`, `CREATE SEQUENCE`, and the
  object grants on the `V_$` views the gate script lists, each `CONTAINER=ALL`. A per-table `SELECT` or
  `FLASHBACK` cannot be seen by the test: it reads "Not checked".
- **Debezium leaves a table behind.** Its `LOG_MINING_FLUSH` table stays in the login's schema after the
  connector is deleted; drop it by hand if you do not want it.
- **`source.ts_ms` is the change time of the redo row**, not the commit time, so the lag stays an upper bound.
- **Oracle NUMBER columns, keys included, arrive in Bronze as text, so that no digits are lost; cast them in Silver.**
- **A schema change made by `SYS` or `SYSTEM` is never seen.** Debezium skips DDL run by the users in
  `log.mining.schema_changes.username.exclude.list`, whose default is `SYS,SYSTEM`
  (`AbstractLogMinerStreamingChangeEventSource.java:1406-1412`, `OracleConnectorConfig.java:636-642`); the first change
  that then uses the new column fails the task (observed in `g4-oracle`, runs 41235139798 and 41238887534, where the
  gate ran the `ALTER` as `SYS`). Change a captured table as its owner or another ordinary user.
- **A new column** added to a captured table by an ordinary user (its owner) while the connector runs reaches Bronze
  and is listed as a schema change; the connector keeps running. Observed in `g4-oracle` (run 41244187205) with
  `log.mining.strategy=hybrid`: the column was in Bronze 50.7 s after the change. The same change on a second
  connector with Oracle's default `online_catalog` also left its task `RUNNING` 120 s later (the probe reads the task's
  state, not the column). Not tested: a dropped or renamed column, a changed column type, a renamed table. The
  Debezium source does not say which of these `hybrid` follows.

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
